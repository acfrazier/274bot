use super::thieving_core::{ChatEvidence, Decision, Failure, Observation, ThieveCore};
use super::{ActionContext, ActionError, NativeMachine, WalkEnd, WalkRequest};
use crate::shim::InteractReq;
use api::quest_progress::EvidenceStamp;
use api::snapshot::{NpcView, SnapshotView, StatView};
use api::WorldTile;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

const PICKPOCKET: &str = "Pickpocket";
const STEAL_FROM: &str = "Steal-from";

#[derive(Clone)]
pub(crate) struct Target {
    pub id: i32,
    pub display: Arc<str>,
    pub action: Arc<str>,
    pub required_level: i32,
}

pub(crate) struct ThieveActionArgs {
    pub targets: Arc<[Target]>,
    pub item_id: i32,
    pub goal_qty: i32,
    pub anchor: Option<WorldTile>,
    pub radius: u8,
    pub deadline: Duration,
}

#[derive(Clone, Copy)]
struct Candidate<'a> {
    npc: &'a NpcView,
    target: &'a Target,
}

pub(crate) struct Thieve {
    args: ThieveActionArgs,
    core: ThieveCore,
    area_anchor: Option<WorldTile>,
    walk: Option<PendingWalk>,
    pending_action: Option<&'static str>,
}

struct PendingWalk {
    request_id: u64,
    required_after: EvidenceStamp,
}

impl NativeMachine for Thieve {
    type Args = ThieveActionArgs;
    type Output = i32;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        if args.targets.is_empty()
            || args.item_id <= 0
            || args.goal_qty < 0
            || !(1..=32).contains(&args.radius)
            || args.deadline.is_zero()
            || args.targets.iter().any(|target| {
                target.id < 0
                    || target.display.is_empty()
                    || target.required_level < 1
                    || !is_thieving_action(&target.action)
            })
        {
            return Err(failed("invalid thieving action configuration"));
        }
        let snapshot = cx.snapshot();
        let now = cx.active_now();
        let area_anchor = args
            .anchor
            .or_else(|| snapshot.here().map(|here| here.value));
        let core = ThieveCore::new(
            now,
            args.deadline,
            Some(args.goal_qty),
            None,
            true,
            last_chat_sequence(snapshot),
        )
        .map_err(|_| failed("invalid thieving action configuration"))?;
        Ok(Self {
            args,
            core,
            area_anchor,
            walk: None,
            pending_action: None,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        let now = cx.active_now();
        let walk_pending = match self.poll_walk(cx) {
            Ok(pending) => pending,
            Err(error) => return Poll::Ready(Err(error)),
        };
        if let Some(request_id) = self.core.pending_request_id() {
            if let Some(receipt) = cx.interaction_receipt(request_id).copied() {
                self.core
                    .receipt(receipt.request_id, receipt.accepted, receipt.chat_since);
            }
        }

        let snapshot = cx.snapshot();
        let skills = snapshot.stats();
        let skill = skills.and_then(|stats| thieving_stat(stats.value));
        let inventory = snapshot.inventory();
        let target_count = inventory.map(|items| item_count(items.value, self.args.item_id));
        let inventory_used = inventory.map(|items| items.value.len() as i32);
        let here = snapshot.here().map(|tile| tile.value);
        if self.area_anchor.is_none() {
            self.area_anchor = here;
        }
        let candidate = snapshot.npcs().and_then(|npcs| {
            select_target(
                npcs.value,
                &self.args.targets,
                self.area_anchor,
                self.args.radius,
                skill.map(|(effective, _)| effective),
            )
        });
        let required_level = candidate.map(|candidate| candidate.target.required_level);
        let pending_action = if self.core.pending_request_id().is_some() {
            self.pending_action.unwrap_or(PICKPOCKET)
        } else {
            candidate
                .map(|candidate| action_name(&candidate.target.action))
                .unwrap_or(PICKPOCKET)
        };
        let skip_chat = self.core.waiting_for_receipt();
        let chat = if skip_chat {
            ChatEvidence::default()
        } else {
            observe_chat(snapshot, self.core.chat_since(), pending_action)
        };
        let (chat_dialog_open, chat_dialog_fingerprint) = chat_dialog(snapshot);
        let observation = Observation {
            effective_thieving: skill.map(|(effective, _)| effective),
            required_level,
            experience: skill.map(|(_, xp)| xp),
            inventory_used,
            target_count,
            inventory_ready: inventory.is_some(),
            chat,
            chat_dialog_open,
            chat_dialog_fingerprint,
        };
        let mut core_observation = observation;
        if walk_pending {
            core_observation.inventory_ready = false;
        }
        let decision = self.core.poll(now, core_observation);
        if !walk_pending
            && self.core.pending_request_id().is_none()
            && skill.is_some()
            && inventory.is_some()
            && !chat_dialog_open
            && matches!(decision, Decision::Wait | Decision::Dispatch)
            && self.area_anchor.is_some_and(|center| {
                here.is_none_or(|here| {
                    tile_distance(here, center)
                        .is_none_or(|distance| distance > u32::from(self.args.radius))
                })
            })
        {
            let center = self.area_anchor.expect("approach anchor was checked");
            return match self.begin_walk(center, u16::from(self.args.radius), cx) {
                Ok(()) => Poll::Pending,
                Err(error) => Poll::Ready(Err(error)),
            };
        }
        match decision {
            Decision::Wait => Poll::Pending,
            Decision::AttemptResolved
            | Decision::AttemptFailed
            | Decision::AttemptTimedOut
            | Decision::Stunned
            | Decision::Dialogue => {
                self.pending_action = None;
                Poll::Pending
            }
            Decision::Complete { held } => {
                self.cancel_walk(cx);
                Poll::Ready(Ok(held))
            }
            Decision::Dispatch => {
                if walk_pending {
                    return Poll::Pending;
                }
                let Some(candidate) = candidate else {
                    return Poll::Pending;
                };
                if candidate.npc.distance > 1 {
                    return match self.begin_walk(candidate.npc.tile, 1, cx) {
                        Ok(()) => Poll::Pending,
                        Err(error) => Poll::Ready(Err(error)),
                    };
                }
                let Some(request) = interaction_request(candidate) else {
                    return Poll::Pending;
                };
                let request_id = match cx.emit(request) {
                    Ok(request_id) => request_id,
                    Err(error) => return Poll::Ready(Err(error)),
                };
                if self
                    .core
                    .start_attempt(now, Some(request_id), &observation, self.core.chat_since())
                    .is_err()
                {
                    cx.cancel_request(request_id);
                    return Poll::Ready(Err(failed("invalid thieving attempt state")));
                }
                self.pending_action = Some(action_name(&candidate.target.action));
                Poll::Pending
            }
            Decision::Failed(reason) => {
                self.cancel_walk(cx);
                Poll::Ready(Err(action_failure(reason)))
            }
        }
    }

    fn cancel(&mut self) {}
}

impl Thieve {
    fn begin_walk(
        &mut self,
        target: WorldTile,
        radius: u16,
        cx: &mut ActionContext<'_>,
    ) -> Result<(), ActionError> {
        let required_after = cx.evidence();
        let request_id = cx.walk(walk_request(target, radius, required_after))?;
        self.walk = Some(PendingWalk {
            request_id,
            required_after,
        });
        Ok(())
    }

    fn poll_walk(&mut self, cx: &mut ActionContext<'_>) -> Result<bool, ActionError> {
        let Some(walk) = self.walk.as_ref() else {
            return Ok(false);
        };
        let request_id = walk.request_id;
        let required_after = walk.required_after;
        if let Some(receipt) = cx.walk_receipt(request_id).cloned() {
            if receipt.end == WalkEnd::UserInput {
                self.walk = None;
                return Err(ActionError::UserInput);
            }
            if receipt.evidence.meets(required_after) {
                self.walk = None;
                receipt.into_arrival()?;
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn cancel_walk(&mut self, cx: &mut ActionContext<'_>) {
        if let Some(walk) = self.walk.take() {
            cx.cancel_request(walk.request_id);
        }
    }
}

fn walk_request(target: WorldTile, radius: u16, required_after: EvidenceStamp) -> WalkRequest {
    WalkRequest {
        target,
        loc_id: None,
        radius,
        arrival: nav::arrival::ArrivalKind::Reach,
        options: super::WalkOptions::default(),
        required_after,
        evidence: None,
        cross: Vec::new().into_boxed_slice(),
        protect: false,
        allow: Default::default(),
    }
}

fn failed(message: &'static str) -> ActionError {
    ActionError::Failed(Arc::from(message))
}

fn action_failure(failure: Failure) -> ActionError {
    failed(match failure {
        Failure::InvalidConfiguration => "invalid thieving action configuration",
        Failure::Level => "effective Thieving is below the selected NPC requirement",
        Failure::Deadline => "thieving overall deadline elapsed",
        Failure::Attempts => "thieving attempt limit reached",
        Failure::DispatchRejected => "thieving interaction was rejected by the host",
        Failure::ReceiptTimeout => "thieving interaction receipt was not observed",
    })
}

fn is_thieving_action(action: &str) -> bool {
    action.eq_ignore_ascii_case(PICKPOCKET) || action.eq_ignore_ascii_case(STEAL_FROM)
}

fn action_name(action: &str) -> &'static str {
    if action.eq_ignore_ascii_case(STEAL_FROM) {
        STEAL_FROM
    } else {
        PICKPOCKET
    }
}

fn thieving_stat(stats: &[StatView]) -> Option<(i32, i32)> {
    stats
        .iter()
        .find(|stat| stat.name.eq_ignore_ascii_case("thieving"))
        .map(|stat| (stat.effective, stat.xp))
}

fn last_chat_sequence(snapshot: SnapshotView<'_>) -> i32 {
    snapshot
        .chat_lines(0)
        .and_then(|lines| lines.value.iter().map(|line| line.sequence).max())
        .unwrap_or_default()
}

fn observe_chat(snapshot: SnapshotView<'_>, since: i32, action: &str) -> ChatEvidence {
    let mut evidence = ChatEvidence::default();
    if let Some(lines) = snapshot.chat_lines(since) {
        for line in lines.value.iter() {
            evidence.observe(action, &line.text, line.sequence);
        }
    }
    evidence
}

fn chat_dialog(snapshot: SnapshotView<'_>) -> (bool, Option<u64>) {
    let Some(modal) = snapshot.chat_modal() else {
        return (false, None);
    };
    if modal.value.root < 0 {
        return (false, None);
    }
    let fingerprint = api::snapshot::chat_page_fingerprint(
        modal.value.texts,
        modal
            .value
            .options
            .iter()
            .map(|option| (option.component_id, option.text.as_str())),
    );
    (true, Some(fingerprint))
}

fn item_count(items: &[api::snapshot::ItemView], id: i32) -> i32 {
    items
        .iter()
        .filter(|item| item.def.id == id)
        .fold(0i32, |count, item| count.saturating_add(item.count.max(0)))
}

fn tile_distance(a: WorldTile, b: WorldTile) -> Option<u32> {
    (a.level == b.level).then(|| a.x.abs_diff(b.x).max(a.z.abs_diff(b.z)))
}

fn nearer(a: Candidate<'_>, b: Candidate<'_>) -> bool {
    let a_distance = a.npc.distance.max(0);
    let b_distance = b.npc.distance.max(0);
    (a_distance > 1, a_distance, a.npc.index) < (b_distance > 1, b_distance, b.npc.index)
}

fn select_target<'a>(
    npcs: &'a [NpcView],
    targets: &'a [Target],
    center: Option<WorldTile>,
    radius: u8,
    effective_level: Option<i32>,
) -> Option<Candidate<'a>> {
    let center = center?;
    let mut nearest: Option<Candidate<'a>> = None;
    let mut eligible: Option<Candidate<'a>> = None;
    for npc in npcs {
        let Some(npc_id) = npc.r#type.and_then(|id| i32::try_from(id).ok()) else {
            continue;
        };
        let Some(name) = npc.name.as_deref() else {
            continue;
        };
        let Some(index) = i32::try_from(npc.index).ok() else {
            continue;
        };
        if index < 0
            || tile_distance(center, npc.tile).is_none_or(|distance| distance > u32::from(radius))
        {
            continue;
        }
        for target in targets.iter().filter(|target| target.id == npc_id) {
            if !name.eq_ignore_ascii_case(target.display.as_ref())
                || !npc
                    .actions
                    .iter()
                    .flatten()
                    .any(|action| action.eq_ignore_ascii_case(target.action.as_ref()))
            {
                continue;
            }
            let candidate = Candidate { npc, target };
            if nearest.is_none_or(|best| nearer(candidate, best)) {
                nearest = Some(candidate);
            }
            if effective_level.is_some_and(|level| level >= target.required_level)
                && eligible.is_none_or(|best| nearer(candidate, best))
            {
                eligible = Some(candidate);
            }
        }
    }
    eligible.or(nearest)
}

fn interaction_request(candidate: Candidate<'_>) -> Option<InteractReq> {
    let index = i32::try_from(candidate.npc.index).ok()?;
    let name = candidate.npc.name.as_ref()?.clone();
    Some(InteractReq::Npc {
        name,
        action: candidate.target.action.to_string(),
        index: Some(index),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::thieving_core::DEFAULT_ACTION_DEADLINE;
    use crate::quester::families::tests::{
        def, local_player, post_user_input_walk_receipt, with_tick,
    };
    use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView};
    fn target(id: i32, required_level: i32) -> Target {
        Target {
            id,
            display: Arc::from("Man"),
            action: Arc::from(PICKPOCKET),
            required_level,
        }
    }

    fn npc(index: usize, id: usize, distance: i32) -> NpcView {
        NpcView {
            index,
            r#type: Some(id),
            name: Some("Man".into()),
            actions: vec![Some(PICKPOCKET.into())],
            tile: WorldTile {
                x: 3,
                z: 4,
                level: 0,
            },
            distance,
            animation: -1,
            animation_frame: -1,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
            health: 0,
            total_health: 0,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 0,
            size: 1,
            network: WorldTile {
                x: 3,
                z: 4,
                level: 0,
            },
            x: 3,
            z: 4,
            yaw: 0,
        }
    }

    fn args(anchor: WorldTile, radius: u8) -> ThieveActionArgs {
        ThieveActionArgs {
            targets: Arc::from([target(1, 1)]),
            item_id: 995,
            goal_qty: 1,
            anchor: Some(anchor),
            radius,
            deadline: Duration::from_secs(60),
        }
    }

    fn held(id: i32, count: i32, slot: i32) -> ItemView {
        ItemView {
            def: def(id, "fixture"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot,
            count,
            actions: vec![],
            component_id: 3214,
        }
    }

    fn game_snapshot(
        here: WorldTile,
        npcs: Vec<NpcView>,
        inventory: Vec<ItemView>,
    ) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(inventory, 28);
        snapshot.seed_stats(vec![StatView {
            index: 17,
            name: "thieving".into(),
            effective: 30,
            base: 30,
            xp: 1_000,
            used: true,
        }]);
        snapshot.seed_local_player(local_player(here));
        snapshot.seed_npcs(npcs);
        snapshot
    }

    #[test]
    fn effective_level_not_base_controls_the_selected_requirement() {
        let stats = [StatView {
            index: 17,
            name: "thieving".into(),
            effective: 29,
            base: 31,
            xp: 1_850,
            used: true,
        }];
        let (effective, xp) = thieving_stat(&stats).unwrap();
        assert_eq!((effective, xp), (29, 1_850));
        let mut core = ThieveCore::new(
            Duration::ZERO,
            DEFAULT_ACTION_DEADLINE,
            Some(1),
            None,
            true,
            0,
        )
        .unwrap();
        let observation = Observation {
            effective_thieving: Some(effective),
            required_level: Some(30),
            experience: Some(xp),
            inventory_used: Some(1),
            target_count: Some(0),
            inventory_ready: true,
            chat: ChatEvidence::default(),
            chat_dialog_open: false,
            chat_dialog_fingerprint: None,
        };
        assert_eq!(
            core.poll(Duration::ZERO, observation),
            Decision::Failed(Failure::Level)
        );
    }

    #[test]
    fn interaction_keeps_the_selected_npc_instance_index() {
        let targets = [target(1, 1)];
        let npcs = [npc(4, 1, 3), npc(19, 1, 1)];
        let candidate = select_target(
            &npcs,
            &targets,
            Some(WorldTile {
                x: 3,
                z: 4,
                level: 0,
            }),
            12,
            Some(1),
        )
        .unwrap();
        match interaction_request(candidate).unwrap() {
            InteractReq::Npc { index, action, .. } => {
                assert_eq!(index, Some(19));
                assert_eq!(action, PICKPOCKET);
            }
            _ => panic!("thieving emits an NPC interaction"),
        }
    }

    #[test]
    fn target_without_a_thieving_action_is_not_dispatchable() {
        let targets = [target(1, 1)];
        let mut npc = npc(19, 1, 1);
        npc.actions = vec![Some("Talk-to".into())];
        assert!(select_target(
            &[npc],
            &targets,
            Some(WorldTile {
                x: 3,
                z: 4,
                level: 0,
            }),
            12,
            Some(30),
        )
        .is_none());
    }

    #[test]
    fn explicit_anchor_walk_is_cancelled_by_manual_input_before_inventory_goal() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let snapshot = game_snapshot(
            WorldTile {
                x: 30,
                z: 40,
                level: 0,
            },
            vec![npc(19, 1, 1)],
            vec![],
        );
        let mut ledger = None;
        let run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<Thieve>(args(anchor, 2), &mut tick.cx)
                .unwrap()
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        let action = ledger.as_ref().unwrap().outbox.last().unwrap();
        let crate::native::HostEffect::Walk(request) = &action.effect else {
            panic!("thieving must approach its fixed anchor before interacting");
        };
        assert_eq!(request.target, anchor);
        assert_eq!(request.radius, 2);

        post_user_input_walk_receipt(&mut ledger, 3);
        let mut snapshot = snapshot;
        snapshot.seed_inventory(vec![held(995, 1, 0)], 28);
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 3, |tick| {
                tick.actions.poll(&run, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::UserInput))
        ));
        assert!(!ledger.as_ref().is_some_and(|ledger| {
            ledger
                .outbox
                .iter()
                .any(|action| matches!(&action.effect, crate::native::HostEffect::Interaction(_)))
        }));
    }

    #[test]
    fn native_action_walks_to_a_remote_target_inside_its_fixed_area() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let target_tile = WorldTile {
            x: 10,
            z: 4,
            level: 0,
        };
        let mut target = npc(19, 1, 7);
        target.tile = target_tile;
        target.network = target_tile;
        let snapshot = game_snapshot(anchor, vec![target], vec![]);
        let mut ledger = None;
        let run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<Thieve>(args(anchor, 12), &mut tick.cx)
                .unwrap()
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        let action = ledger.as_ref().unwrap().outbox.last().unwrap();
        let crate::native::HostEffect::Walk(request) = &action.effect else {
            panic!("thieving must approach a remote target before interacting");
        };
        assert_eq!(request.target, target_tile);
        assert_eq!(request.radius, 1);
        assert!(!ledger.as_ref().is_some_and(|ledger| {
            ledger
                .outbox
                .iter()
                .any(|action| matches!(&action.effect, crate::native::HostEffect::Interaction(_)))
        }));
    }
}
