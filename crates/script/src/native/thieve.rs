use super::thieving_core::{
    level_ready, ChatEvidence, Decision, Failure, Observation, ThieveCore, PICKPOCKET, STEAL_FROM,
};
use super::{ActionContext, ActionError, NativeMachine, WalkEnd};
use crate::gatherer::oneop::{OneOp, OneOpArgs};
use crate::quester::families::reach::{self, Reach, ReachArgs, ReachKind};
use api::quest_progress::EvidenceStamp;
use api::snapshot::{NpcView, SnapshotView, StatView};
use api::WorldTile;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

const REACH_MODAL_WAIT_TICKS: u8 = 1;

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
    reach: Option<Reach>,
    reach_request_id: Option<u64>,
    reach_target: Option<(i32, &'static str)>,
    modal: Option<OneOp>,
    modal_reach_ticks: u8,
    pending_action: Option<&'static str>,
    pending_target_id: Option<i32>,
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
            reach::last_chat_seq(cx),
        )
        .map_err(|_| failed("invalid thieving action configuration"))?;
        Ok(Self {
            args,
            core,
            area_anchor,
            walk: None,
            reach: None,
            reach_request_id: None,
            reach_target: None,
            modal: None,
            modal_reach_ticks: 0,
            pending_action: None,
            pending_target_id: None,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        let now = cx.active_now();
        if let Some(request_id) = self.core.pending_request_id() {
            if let Some(receipt) = cx.interaction_receipt(request_id).copied() {
                self.core
                    .receipt(receipt.request_id, receipt.accepted, receipt.chat_since);
            }
        }

        let snapshot = cx.snapshot();
        let mut core_polled_for_modal = false;
        if self.modal.is_some() {
            if let Err(error) = self.poll_core_during_modal(now) {
                self.cancel_walk(cx);
                self.discard_reach();
                self.modal = None;
                return Poll::Ready(Err(error));
            }
            core_polled_for_modal = true;
        }

        if let Some(result) = self.modal.as_mut().map(|modal| modal.poll(cx)) {
            match result {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => {
                    self.modal = None;
                    return Poll::Ready(Err(error));
                }
                Poll::Ready(Ok(true)) => self.modal = None,
                Poll::Ready(Ok(false)) => {
                    self.modal = None;
                    return Poll::Pending;
                }
            }
        }
        if let Some(args) = OneOpArgs::stray_modal(snapshot) {
            self.cancel_walk(cx);
            if !core_polled_for_modal {
                if let Err(error) = self.poll_core_during_modal(now) {
                    self.discard_reach();
                    return Poll::Ready(Err(error));
                }
            }
            if self.reach.is_some() {
                self.modal_reach_ticks = self.modal_reach_ticks.saturating_add(1);
                let receipt_absent = self
                    .reach_request_id
                    .is_none_or(|request_id| cx.interaction_receipt(request_id).is_none());
                if receipt_absent || self.modal_reach_ticks >= REACH_MODAL_WAIT_TICKS {
                    // Reach::WaitWalk can click again on arrival. Never poll Reach
                    // while a modal is pending; abandon it within one modal tick.
                    self.discard_reach();
                    self.modal_reach_ticks = 0;
                } else {
                    return Poll::Pending;
                }
            } else {
                self.modal_reach_ticks = 0;
            }
            self.modal = match OneOp::begin(args, cx) {
                Ok(modal) => Some(modal),
                Err(error) => return Poll::Ready(Err(error)),
            };
            return Poll::Pending;
        }
        self.modal_reach_ticks = 0;

        let (chat_dialog_open, chat_dialog_fingerprint) = chat_dialog(snapshot);
        if chat_dialog_open {
            self.cancel_walk(cx);
        }
        let walk_pending = if chat_dialog_open {
            false
        } else {
            match self.poll_walk(cx) {
                Ok(pending) => pending,
                Err(error) => return Poll::Ready(Err(error)),
            }
        };

        let skills = snapshot.stats();
        let skill = skills.and_then(|stats| thieving_stat(stats.value));
        let inventory = snapshot.inventory();
        let stock = snapshot.stock();
        let target_count = stock.held(self.args.item_id);
        let inventory_used = stock.occupied();
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
        let chat = if self.core.waiting_for_receipt() {
            ChatEvidence::default()
        } else {
            observe_chat(snapshot, self.core.chat_since(), pending_action)
        };
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
        core_observation.inventory_ready &=
            !walk_pending && self.reach.is_none() && !chat_dialog_open;
        let decision = self.core.poll(now, core_observation);

        if decision == Decision::Dispatch
            && !walk_pending
            && self.reach.is_none()
            && self.area_anchor.is_some_and(|center| {
                here.is_none_or(|here| !reach::within(here, center, i32::from(self.args.radius)))
            })
        {
            let center = self.area_anchor.expect("approach anchor was checked");
            return match self.begin_walk(center, u16::from(self.args.radius), cx) {
                Ok(()) => Poll::Pending,
                Err(error) => Poll::Ready(Err(error)),
            };
        }

        match decision {
            Decision::Wait => {
                if !chat_dialog_open && !self.core.stun_active(now) {
                    if let Err(error) = self.poll_reach(cx, now, Some(&observation)) {
                        return Poll::Ready(Err(error));
                    }
                }
                Poll::Pending
            }
            Decision::AttemptResolved
            | Decision::AttemptFailed
            | Decision::AttemptTimedOut
            | Decision::Dialogue => {
                if decision == Decision::AttemptFailed
                    && self.pending_target_id.is_some_and(is_troll_guard)
                {
                    self.pending_action = None;
                    self.pending_target_id = None;
                    self.cancel_walk(cx);
                    return Poll::Ready(Err(failed(
                        "troll prison guard attacked after failed pickpocket",
                    )));
                }
                self.pending_action = None;
                self.pending_target_id = None;
                if !chat_dialog_open && !self.core.stun_active(now) {
                    if let Err(error) = self.poll_reach(cx, now, Some(&observation)) {
                        return Poll::Ready(Err(error));
                    }
                }
                Poll::Pending
            }
            Decision::Stunned => {
                self.pending_action = None;
                self.pending_target_id = None;
                Poll::Pending
            }
            Decision::Complete { held } => {
                self.cancel_walk(cx);
                Poll::Ready(Ok(held))
            }
            Decision::Dispatch => {
                if walk_pending || chat_dialog_open || self.reach.is_some() {
                    return Poll::Pending;
                }
                let Some(candidate) = candidate else {
                    return Poll::Pending;
                };
                let action = action_name(&candidate.target.action);
                let reach = match Reach::begin(
                    ReachArgs {
                        kind: ReachKind::Npc {
                            id: candidate.target.id,
                            name: Arc::clone(&candidate.target.display),
                        },
                        op: Arc::clone(&candidate.target.action),
                        anchor: self.area_anchor,
                        radius: candidate.npc.distance.max(1),
                        wait_if_missing: true,
                        target_tile: None,
                        reachable_only: false,
                    },
                    cx,
                ) {
                    Ok(reach) => reach,
                    Err(error) => return Poll::Ready(Err(error)),
                };
                self.reach_target = Some((candidate.target.id, action));
                self.reach_request_id = None;
                let request_id = reach.interaction_request_id();
                self.reach = Some(reach);
                if let Some(request_id) = request_id {
                    if let Err(error) = self.track_reach_request(request_id, now, &observation) {
                        cx.cancel_request(request_id);
                        if let Some(reach) = self.reach.as_mut() {
                            reach.cancel();
                        }
                        self.reach = None;
                        self.reach_target = None;
                        self.reach_request_id = None;
                        return Poll::Ready(Err(error));
                    }
                }
                Poll::Pending
            }
            Decision::Failed(reason) => {
                self.cancel_walk(cx);
                Poll::Ready(Err(action_failure(reason)))
            }
        }
    }

    fn cancel(&mut self) {
        if let Some(reach) = self.reach.as_mut() {
            reach.cancel();
        }
        if let Some(modal) = self.modal.as_mut() {
            modal.cancel();
        }
    }
}

impl Thieve {
    fn poll_core_during_modal(&mut self, now: Duration) -> Result<(), ActionError> {
        // Keep action and receipt deadlines live without advancing the attempt
        // observation while OneOp owns the modal.
        let decision = self.core.poll(
            now,
            Observation {
                effective_thieving: None,
                required_level: None,
                experience: None,
                inventory_used: None,
                target_count: None,
                inventory_ready: false,
                chat: ChatEvidence::default(),
                chat_dialog_open: false,
                chat_dialog_fingerprint: None,
            },
        );
        match decision {
            Decision::Failed(reason) => Err(action_failure(reason)),
            Decision::AttemptFailed if self.pending_target_id.is_some_and(is_troll_guard) => Err(
                failed("troll prison guard attacked after failed pickpocket"),
            ),
            Decision::AttemptResolved
            | Decision::AttemptFailed
            | Decision::AttemptTimedOut
            | Decision::Dialogue
            | Decision::Stunned => {
                self.pending_action = None;
                self.pending_target_id = None;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn discard_reach(&mut self) {
        if let Some(mut reach) = self.reach.take() {
            reach.cancel();
        }
        self.reach_request_id = None;
        self.reach_target = None;
    }

    fn begin_walk(
        &mut self,
        target: WorldTile,
        radius: u16,
        cx: &mut ActionContext<'_>,
    ) -> Result<(), ActionError> {
        let required_after = cx.evidence();
        let request_id = cx.walk(reach::walk_request(target, radius, None, required_after))?;
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

    fn track_reach_request(
        &mut self,
        request_id: u64,
        now: Duration,
        observation: &Observation,
    ) -> Result<(), ActionError> {
        if self.reach_request_id == Some(request_id) || self.core.pending_request_id().is_some() {
            return Ok(());
        }
        let Some((target_id, action)) = self.reach_target else {
            return Err(failed("invalid thieving reach state"));
        };
        self.core
            .start_attempt(now, Some(request_id), observation, self.core.chat_since())
            .map_err(|_| failed("invalid thieving attempt state"))?;
        self.reach_request_id = Some(request_id);
        self.pending_action = Some(action);
        self.pending_target_id = Some(target_id);
        Ok(())
    }

    fn poll_reach(
        &mut self,
        cx: &mut ActionContext<'_>,
        now: Duration,
        observation: Option<&Observation>,
    ) -> Result<(), ActionError> {
        let Some(mut reach) = self.reach.take() else {
            return Ok(());
        };
        if let (Some(request_id), Some(observation)) = (reach.interaction_request_id(), observation)
        {
            if let Err(error) = self.track_reach_request(request_id, now, observation) {
                self.reach = Some(reach);
                return Err(error);
            }
        }
        let result = reach.poll(cx);
        if let (Some(request_id), Some(observation)) = (reach.interaction_request_id(), observation)
        {
            if let Err(error) = self.track_reach_request(request_id, now, observation) {
                self.reach = Some(reach);
                return Err(error);
            }
        }
        match result {
            Poll::Pending => self.reach = Some(reach),
            Poll::Ready(Ok(_)) => {
                self.reach_request_id = None;
                self.reach_target = None;
            }
            Poll::Ready(Err(error)) => {
                self.reach = Some(reach);
                return Err(error);
            }
        }
        Ok(())
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

fn is_troll_guard(target_id: i32) -> bool {
    matches!(target_id, 1_128 | 1_129)
}

fn thieving_stat(stats: &[StatView]) -> Option<(i32, i32)> {
    stats
        .iter()
        .find(|stat| stat.name.eq_ignore_ascii_case("thieving"))
        .map(|stat| (stat.effective, stat.xp))
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
        if index < 0 || !reach::within(center, npc.tile, i32::from(radius)) {
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
            if level_ready(effective_level, target.required_level) == Some(true)
                && eligible.is_none_or(|best| nearer(candidate, best))
            {
                eligible = Some(candidate);
            }
        }
    }
    eligible.or(nearest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::thieving_core::DEFAULT_ACTION_DEADLINE;
    use crate::quester::families::tests::{
        def, local_player, post_user_input_walk_receipt, with_tick,
    };
    use api::snapshot::{
        ChatLineView, GameSnapshot, ItemActionFamily, ItemContainer, ItemView, LocLayer, LocView,
    };

    fn target(id: i32, required_level: i32) -> Target {
        Target {
            id,
            display: Arc::from("Man"),
            action: Arc::from("Pickpocket"),
            required_level,
        }
    }

    fn npc(index: usize, id: usize, distance: i32) -> NpcView {
        NpcView {
            index,
            r#type: Some(id),
            name: Some("Man".into()),
            actions: vec![Some("Pickpocket".into())],
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

    fn accept_last_interaction(ledger: &mut Option<Box<crate::native::ledger::Ledger>>, tick: u64) {
        let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
        ledger.as_mut().unwrap().complete_interaction(
            &authority,
            crate::native::InteractionReceipt {
                request_id: authority.request_id().get(),
                evidence: api::quest_progress::EvidenceStamp {
                    run: authority.run(),
                    tick,
                    sequence: tick,
                },
                accepted: true,
                chat_since: 0,
            },
        );
    }

    fn seed_unreachable_door(snapshot: &mut GameSnapshot, open: bool) {
        snapshot.seed_chat_lines(vec![ChatLineView {
            sequence: 1,
            type_: 0,
            username: None,
            text: "I can't reach that!".into(),
        }]);
        snapshot.seed_locs(vec![LocView {
            id: 1516,
            name: Some("Door".into()),
            actions: vec![Some(if open { "Close" } else { "Open" }.into())],
            tile: WorldTile {
                x: 4,
                z: 4,
                level: 0,
            },
            distance: 1,
            typecode: 0,
            info: 0,
            description: None,
            layer: LocLayer::Wall,
            shape: 0,
            angle: 2,
            width: 1,
            length: 1,
            footprint_width: 1,
            footprint_length: 1,
            block_walk: true,
            block_range: false,
            active: true,
            animation: -1,
            map_function: -1,
            map_scene: -1,
            force_approach: 0,
        }]);
    }

    fn npc_click_count(ledger: &Option<Box<crate::native::ledger::Ledger>>) -> usize {
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .filter(|action| {
                matches!(
                    &action.effect,
                    crate::native::HostEffect::Interaction(crate::shim::InteractReq::Npc { .. })
                )
            })
            .count()
    }

    fn continue_dialog_count(ledger: &Option<Box<crate::native::ledger::Ledger>>) -> usize {
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .filter(|action| {
                matches!(
                    &action.effect,
                    crate::native::HostEffect::Interaction(
                        crate::shim::InteractReq::ContinueDialog { .. }
                    )
                )
            })
            .count()
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
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let snapshot = game_snapshot(anchor, vec![npc(4, 1, 3), npc(19, 1, 1)], vec![]);
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
        assert!(matches!(
            &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
            crate::native::HostEffect::Interaction(crate::shim::InteractReq::Npc {
                index: Some(19),
                action,
                ..
            }) if action == "Pickpocket"
        ));
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
    fn native_reach_interacts_with_a_remote_target_before_route_recovery() {
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
        assert!(matches!(
            &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
            crate::native::HostEffect::Interaction(crate::shim::InteractReq::Npc {
                index: Some(19),
                action,
                ..
            }) if action == "Pickpocket"
        ));
    }

    #[test]
    fn missing_reach_receipt_does_not_wedge_level_up_modal_or_thieving() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let mut snapshot = game_snapshot(anchor, vec![npc(19, 1, 1)], vec![]);
        let mut ledger = None;
        let mut action_args = args(anchor, 12);
        action_args.goal_qty = 2;
        let run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<Thieve>(action_args, &mut tick.cx)
                .unwrap()
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        accept_last_interaction(&mut ledger, 3);

        seed_unreachable_door(&mut snapshot, false);
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert!(ledger.as_ref().unwrap().walk.is_none());
        assert!(ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .any(|action| { matches!(&action.effect, crate::native::HostEffect::Walk(_)) }));

        // Reach has moved on to door recovery after ThieveCore recorded the
        // accepted NPC receipt, as happens when a later Reach emit replaces it.
        ledger.as_mut().unwrap().interaction = None;
        snapshot.seed_chat_modal(100, vec!["Congratulations, you advanced Thieving.".into()]);
        snapshot.seed_chat_options(Vec::new(), 99);
        assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(continue_dialog_count(&ledger), 1);

        accept_last_interaction(&mut ledger, 5);
        snapshot.seed_stats(vec![StatView {
            index: 17,
            name: "thieving".into(),
            effective: 30,
            base: 30,
            xp: 1_008,
            used: true,
        }]);
        snapshot.seed_inventory(vec![held(995, 1, 0)], 28);
        snapshot.seed_chat_lines(vec![
            ChatLineView {
                sequence: 1,
                type_: 0,
                username: None,
                text: "I can't reach that!".into(),
            },
            ChatLineView {
                sequence: 2,
                type_: 0,
                username: None,
                text: "You attempt to pick the man's pocket.".into(),
            },
            ChatLineView {
                sequence: 3,
                type_: 0,
                username: None,
                text: "You pick the man's pocket.".into(),
            },
        ]);
        snapshot.seed_chat_modal(-1, Vec::new());
        snapshot.seed_chat_options(Vec::new(), -1);
        assert!(with_tick(&snapshot, &mut ledger, 5, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert!(with_tick(&snapshot, &mut ledger, 6, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(npc_click_count(&ledger), 2);
    }

    #[test]
    fn pending_reach_is_bounded_and_thieve_deadline_runs_during_modal() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let mut snapshot = game_snapshot(anchor, vec![npc(19, 1, 1)], vec![]);
        let mut ledger = None;
        let mut action_args = args(anchor, 12);
        action_args.deadline = Duration::from_secs(4);
        let run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<Thieve>(action_args, &mut tick.cx)
                .unwrap()
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        accept_last_interaction(&mut ledger, 3);

        seed_unreachable_door(&mut snapshot, false);
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        snapshot.seed_chat_modal(100, vec!["Congratulations, you advanced Thieving.".into()]);
        snapshot.seed_chat_options(Vec::new(), 99);
        for tick in 4..=6 {
            assert!(with_tick(&snapshot, &mut ledger, tick, |native_tick| {
                native_tick.actions.poll(&run, &mut native_tick.cx)
            })
            .is_pending());
            assert_eq!(continue_dialog_count(&ledger), 1);
        }
        assert_eq!(continue_dialog_count(&ledger), 1);

        assert!(with_tick(&snapshot, &mut ledger, 7, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 8, |tick| {
                tick.actions.poll(&run, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::Failed(reason)))
                if reason.as_ref() == "thieving overall deadline elapsed"
        ));
    }

    #[test]
    fn pending_modal_reach_never_emits_a_second_npc_click() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let mut snapshot = game_snapshot(anchor, vec![npc(19, 1, 1)], vec![]);
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
        accept_last_interaction(&mut ledger, 3);

        seed_unreachable_door(&mut snapshot, true);
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        // The open-door recovery walk targets the NPC's current tile. Polling
        // WaitWalk on this modal tick would emit another NPC click.
        assert!(ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .any(|action| { matches!(&action.effect, crate::native::HostEffect::Walk(_)) }));
        snapshot.seed_chat_modal(100, vec!["Congratulations, you advanced Thieving.".into()]);
        snapshot.seed_chat_options(Vec::new(), 99);

        for tick in 4..=6 {
            assert!(with_tick(&snapshot, &mut ledger, tick, |native_tick| {
                native_tick.actions.poll(&run, &mut native_tick.cx)
            })
            .is_pending());
            assert_eq!(npc_click_count(&ledger), 1);
            assert_eq!(continue_dialog_count(&ledger), 1);
        }
        assert_eq!(continue_dialog_count(&ledger), 1);
        for tick in 7..=8 {
            assert!(with_tick(&snapshot, &mut ledger, tick, |native_tick| {
                native_tick.actions.poll(&run, &mut native_tick.cx)
            })
            .is_pending());
            assert_eq!(npc_click_count(&ledger), 1);
        }
    }

    #[test]
    fn level_up_modal_is_continued_and_thieving_resumes_without_duplicate_clicks() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let snapshot = game_snapshot(anchor, vec![npc(19, 1, 1)], vec![]);
        let mut ledger = None;
        let mut action_args = args(anchor, 12);
        action_args.goal_qty = 2;
        let run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<Thieve>(action_args, &mut tick.cx)
                .unwrap()
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        accept_last_interaction(&mut ledger, 3);

        let mut snapshot = snapshot;
        snapshot.seed_stats(vec![StatView {
            index: 17,
            name: "thieving".into(),
            effective: 30,
            base: 30,
            xp: 1_008,
            used: true,
        }]);
        snapshot.seed_inventory(vec![held(995, 1, 0)], 28);
        snapshot.seed_chat_lines(vec![
            ChatLineView {
                sequence: 1,
                type_: 0,
                username: None,
                text: "You attempt to pick the man's pocket.".into(),
            },
            ChatLineView {
                sequence: 2,
                type_: 0,
                username: None,
                text: "You pick the man's pocket.".into(),
            },
        ]);
        snapshot.seed_chat_modal(100, vec!["Congratulations, you advanced Thieving.".into()]);
        snapshot.seed_chat_options(Vec::new(), 99);
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert!(matches!(
            &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
            crate::native::HostEffect::Interaction(crate::shim::InteractReq::ContinueDialog {
                component_id: None
            })
        ));
        accept_last_interaction(&mut ledger, 4);

        snapshot.seed_chat_modal(-1, Vec::new());
        snapshot.seed_chat_options(Vec::new(), -1);
        assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        let npc_clicks = |ledger: &Option<Box<crate::native::ledger::Ledger>>| {
            ledger
                .as_ref()
                .unwrap()
                .outbox
                .iter()
                .filter(|action| {
                    matches!(
                        &action.effect,
                        crate::native::HostEffect::Interaction(
                            crate::shim::InteractReq::Npc { .. }
                        )
                    )
                })
                .count()
        };
        assert_eq!(npc_clicks(&ledger), 1);
        assert!(with_tick(&snapshot, &mut ledger, 5, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(npc_clicks(&ledger), 2);
    }

    #[test]
    fn troll_guard_failure_returns_a_combat_failure_instead_of_a_stun() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let mut guard_target = target(1_128, 30);
        guard_target.display = Arc::from("Troll prison guard");
        let mut guard = npc(19, 1_128, 1);
        guard.name = Some("Troll prison guard".into());
        let snapshot = game_snapshot(anchor, vec![guard], vec![]);
        let mut ledger = None;
        let mut action_args = args(anchor, 12);
        action_args.targets = Arc::from([guard_target]);
        let run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<Thieve>(action_args, &mut tick.cx)
                .unwrap()
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        accept_last_interaction(&mut ledger, 3);

        let mut snapshot = snapshot;
        snapshot.seed_chat_lines(vec![ChatLineView {
            sequence: 1,
            type_: 0,
            username: None,
            text: "You attempt to pick the guard's pocket.".into(),
        }]);
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        let mut guard = npc(19, 1_128, 1);
        guard.name = Some("Troll prison guard".into());
        guard.in_combat = true;
        snapshot.seed_npcs(vec![guard]);
        snapshot.seed_chat_lines(vec![
            ChatLineView {
                sequence: 1,
                type_: 0,
                username: None,
                text: "You attempt to pick the guard's pocket.".into(),
            },
            ChatLineView {
                sequence: 2,
                type_: 0,
                username: None,
                text: "You fail to pick the guard's pocket.".into(),
            },
        ]);
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 4, |tick| {
                tick.actions.poll(&run, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::Failed(reason)))
                if reason.as_ref() == "troll prison guard attacked after failed pickpocket"
        ));
    }

    #[test]
    fn cannot_reach_chat_reuses_reach_door_route() {
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
        let door_tile = WorldTile {
            x: 4,
            z: 4,
            level: 0,
        };
        let mut target = npc(19, 1, 7);
        target.tile = target_tile;
        target.network = target_tile;
        let mut snapshot = game_snapshot(anchor, vec![target], vec![]);
        snapshot.seed_locs(vec![LocView {
            id: 1516,
            name: Some("Door".into()),
            actions: vec![Some("Open".into())],
            tile: door_tile,
            distance: 1,
            typecode: 0,
            info: 0,
            description: None,
            layer: LocLayer::Wall,
            shape: 0,
            angle: 2,
            width: 1,
            length: 1,
            footprint_width: 1,
            footprint_length: 1,
            block_walk: true,
            block_range: false,
            active: true,
            animation: -1,
            map_function: -1,
            map_scene: -1,
            force_approach: 0,
        }]);
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
        accept_last_interaction(&mut ledger, 3);
        snapshot.seed_chat_lines(vec![ChatLineView {
            sequence: 1,
            type_: 0,
            username: None,
            text: "I can't reach that!".into(),
        }]);
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert!(ledger.as_ref().unwrap().outbox.iter().any(|action| {
            matches!(
                &action.effect,
                crate::native::HostEffect::Walk(request) if request.target == door_tile
            )
        }));
    }
}
