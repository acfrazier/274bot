use super::thieving_core::{
    level_ready, ChatEvidence, Decision, Failure, Observation, ThieveCore, PICKPOCKET, STEAL_FROM,
};
use super::walk::Walk;
use super::{ActionContext, ActionError, HoldResume, InteractionDispatch, NativeMachine};
use crate::gatherer::oneop::{OneOp, OneOpArgs};
use crate::quester::families::reach::{self, Reach, ReachArgs, ReachKind};
use api::snapshot::{NpcView, SnapshotView, StatView};
use api::WorldTile;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

#[derive(Clone)]
pub(crate) struct Target {
    pub id: i32,
    pub display: Arc<str>,
    pub action: Arc<str>,
    pub required_level: i32,
    /// The selected NPC config's `respawnrate`, in game ticks, from
    /// `api::game_data::SelectedGameData::thieving_npc_respawn_ticks`.
    pub respawn_ticks: u32,
}

pub(crate) struct ThieveActionArgs {
    pub targets: Arc<[Target]>,
    pub item_id: i32,
    pub goal_qty: i32,
    pub anchor: Option<WorldTile>,
    pub radius: u8,
    pub deadline: Duration,
    /// Observed game ticks to wait for one of the step's selectable spawns
    /// once the player is inside the anchor. The compile step sets this to
    /// the largest `Target::respawn_ticks` plus one observed tick; the
    /// compiled target refuses when any selected id has no respawn fact.
    pub no_target_wait_ticks: u64,
}

pub(crate) struct Thieve {
    args: ThieveActionArgs,
    core: ThieveCore,
    area_anchor: Option<WorldTile>,
    walk: Option<Walk>,
    walk_retry: super::WalkRetryState,
    reach: Option<Reach>,
    reach_request_id: Option<u64>,
    reach_target: Option<(i32, &'static str)>,
    modal: Option<OneOp>,
    pending_action: Option<&'static str>,
    pending_target_id: Option<i32>,
    /// First observed tick with the player inside the anchor and no target.
    no_target_since: Option<u64>,
    /// Set when the empty-anchor wait ends by dispatching the spawn it saw.
    ///
    /// That attempt began on the tick the content spawned the target, which
    /// can be after the action's wall-clock deadline. The deadline must not
    /// kill it; it still ends the approach walk and later attempts.
    anchor_spawn_attempt: bool,
}

/// Named terminal failure for the empty-anchor wait.
const NO_TARGET_FAILURE: &str = "no thieving target appeared at the anchor";

impl NativeMachine for Thieve {
    type Args = ThieveActionArgs;
    type Output = i32;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        if args.targets.is_empty()
            || args.item_id <= 0
            || args.goal_qty < 0
            || !(1..=32).contains(&args.radius)
            || args.deadline.is_zero()
            || args.no_target_wait_ticks == 0
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
            walk_retry: super::WalkRetryState::default(),
            reach: None,
            reach_request_id: None,
            reach_target: None,
            modal: None,
            pending_action: None,
            pending_target_id: None,
            no_target_since: None,
            anchor_spawn_attempt: false,
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
                // Reach::WaitWalk can click again on arrival. Discard it
                // immediately rather than polling it while a modal is pending.
                self.discard_reach();
            }
            self.modal = Some(std::task::ready!(super::defer_budget(OneOp::begin(
                args, cx
            )))?);
            return Poll::Pending;
        }

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
        let picked = select_target(
            &*cx,
            &self.args.targets,
            self.area_anchor,
            self.args.radius,
            skill.map(|(effective, _)| effective),
        );
        // The wall-clock deadline stays out of the empty-anchor wait and the
        // one attempt that wait produces: the wait is a content wait, bounded
        // by the selected spawns' own `respawn_ticks`
        // (`self.args.no_target_wait_ticks`).
        let entered_waiting = self.no_target_since.is_some();
        if picked.is_some() {
            self.no_target_since = None;
        }
        // The compiled target already carries the selected content's pickpocket
        // level (`api::game_data::SelectedGameData::required_thieving_npc`,
        // `api/src/game_data.rs:2357-2366`, resolved by
        // `quester/families/thieve.rs:122-125`). With no live match, the least
        // demanding configured type is the gate: any configured spawn the
        // account can attempt keeps the approach valid, and a visible NPC still
        // gates on its own selected level below. A below-level account therefore
        // refuses before the walk and before any click.
        let required_level = picked
            .map(|target| target.required_level)
            .or_else(|| static_required_level(&self.args.targets));
        let pending_action = if self.core.pending_request_id().is_some() {
            self.pending_action.unwrap_or(PICKPOCKET)
        } else {
            picked
                .map(|target| action_name(&target.action))
                .unwrap_or(PICKPOCKET)
        };
        // Own the dispatch target before any mutable borrow.
        let dispatch = picked.map(|target| {
            (
                target.id,
                Arc::clone(&target.display),
                Arc::clone(&target.action),
            )
        });
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
        if !walk_pending
            && self.area_anchor.is_some_and(|center| {
                here.is_some_and(|here| reach::within(here, center, i32::from(self.args.radius)))
            })
        {
            self.walk_retry.reset();
        }
        let suspend_deadline =
            entered_waiting || (self.anchor_spawn_attempt && self.core.has_pending_attempt());
        let decision = if suspend_deadline {
            self.core.poll_without_deadline(now, core_observation)
        } else {
            self.core.poll(now, core_observation)
        };
        if self.anchor_spawn_attempt && !self.core.has_pending_attempt() {
            self.anchor_spawn_attempt = false;
        }

        if decision == Decision::Dispatch
            && !walk_pending
            && self.reach.is_none()
            && self.area_anchor.is_some_and(|center| {
                here.is_none_or(|here| !reach::within(here, center, i32::from(self.args.radius)))
            })
        {
            let center = self.area_anchor.expect("approach anchor was checked");
            let tick = cx.evidence().tick;
            if self.walk_retry.waiting_for_next_tick(tick) {
                return Poll::Pending;
            }
            match super::defer_budget(self.begin_walk(center, u16::from(self.args.radius), cx)) {
                Poll::Pending => {
                    self.walk_retry.defer_until_next_tick(tick);
                    return Poll::Pending;
                }
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(())) => self.walk_retry.action_submitted(),
            }
            return Poll::Pending;
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
                let Some((target_id, display, op)) = dispatch else {
                    // The approach walk ran above whenever the player was
                    // outside the anchor. Inside it, wait one full content
                    // respawn window for a spawn before failing by name.
                    let inside = self.area_anchor.is_some_and(|center| {
                        here.is_some_and(|here| {
                            reach::within(here, center, i32::from(self.args.radius))
                        })
                    });
                    if !inside {
                        self.no_target_since = None;
                        return Poll::Pending;
                    }
                    let since = *self.no_target_since.get_or_insert(cx.evidence().tick);
                    if cx.evidence().tick.saturating_sub(since) >= self.args.no_target_wait_ticks {
                        return Poll::Ready(Err(failed(NO_TARGET_FAILURE)));
                    }
                    return Poll::Pending;
                };
                // The attempt this wait produced began after the action's
                // deadline may have elapsed; keep it alive to its resolution.
                self.anchor_spawn_attempt = entered_waiting;
                let action = action_name(&op);
                let reach = std::task::ready!(super::defer_budget(Reach::begin(
                    ReachArgs {
                        kind: ReachKind::Npc {
                            id: target_id,
                            name: display,
                        },
                        op,
                        anchor: self.area_anchor,
                        radius: i32::from(self.args.radius),
                        wait_if_missing: true,
                        target_tile: None,
                        reachable_only: false,
                    },
                    cx,
                )))?;
                self.reach_target = Some((target_id, action));
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

    const RESUMES_AFTER_HOLD: bool = true;

    /// The core already holds the right rule for an attempt the server took:
    /// it does not dispatch again while the attempt is pending, resolves it
    /// from the experience, item or chat it caused, or times it out and only
    /// then retries (`thieving_core.rs`, `ThieveCore::poll`), and it checks the
    /// item count against the goal before any dispatch. A guardian hold adds
    /// one case: the host dropped the attempt's request unsent, so no receipt
    /// will come and the core would end the action at its receipt deadline.
    /// That attempt did not happen: forget it, so the poll after this dispatches
    /// once, unless the goal count is already held.
    fn after_hold(&mut self, cx: &mut ActionContext<'_>) -> Result<HoldResume, ActionError> {
        if let Some(walk) = self.walk.as_mut() {
            return walk.after_hold(cx);
        }
        let Some(request_id) = self.core.pending_request_id() else {
            return Ok(HoldResume::Kept("no attempt is pending"));
        };
        let (unsent, queued) = match cx.interaction_dispatch(request_id) {
            InteractionDispatch::Unsent => (true, false),
            InteractionDispatch::Queued => (false, true),
            InteractionDispatch::Settled(_) => (false, false),
        };
        if unsent {
            cx.cancel_request(request_id);
            self.discard_reach();
            self.core.abandon_attempt(cx.active_now());
            self.pending_action = None;
            self.pending_target_id = None;
        }
        let goal_met = cx
            .snapshot()
            .stock()
            .held(self.args.item_id)
            .is_some_and(|held| held >= self.args.goal_qty);
        Ok(if goal_met {
            HoldResume::Kept("the item count already meets the goal")
        } else if unsent {
            HoldResume::Reissued("thieving attempt")
        } else if queued {
            HoldResume::Kept("the host still holds the attempt's request")
        } else {
            HoldResume::Kept(
                "the server took the attempt; its own window resolves or times it out before any retry",
            )
        })
    }
}

impl Thieve {
    fn poll_core_during_modal(&mut self, now: Duration) -> Result<(), ActionError> {
        // Keep action and receipt deadlines live without advancing the attempt
        // observation while OneOp owns the modal. The one attempt the
        // empty-anchor wait produced keeps its deadline suspension.
        let observation = Observation {
            effective_thieving: None,
            required_level: None,
            experience: None,
            inventory_used: None,
            target_count: None,
            inventory_ready: false,
            chat: ChatEvidence::default(),
            chat_dialog_open: false,
            chat_dialog_fingerprint: None,
        };
        let decision = if self.anchor_spawn_attempt && self.core.has_pending_attempt() {
            self.core.poll_without_deadline(now, observation)
        } else {
            self.core.poll(now, observation)
        };
        if self.anchor_spawn_attempt && !self.core.has_pending_attempt() {
            self.anchor_spawn_attempt = false;
        }
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
        let mut request = reach::walk_request(target, radius, None, cx.evidence());
        // The action's own gate is the Chebyshev radius (`reach::within`):
        // any standable tile inside it is enough to look for a workman, so
        // the anchor walk must not demand a path onto the anchor tile. A
        // visible workman still uses the reach machine's `Reach` arrival.
        request.arrival = nav_planner::contract::ArrivalKind::Area;
        // An area route ends in a plain host `RouteEnded`, so the receipt
        // must be settled by the shared walk adapter: it accepts a walkable
        // tile inside the area and waits one tick on a scene entry. Reading
        // the raw receipt here failed the live Digsite run on `[3347,3395,0]`,
        // twelve tiles from the anchor, with "walk route ended before
        // arrival".
        self.walk = Some(Walk::begin(request, cx)?);
        Ok(())
    }

    fn poll_walk(&mut self, cx: &mut ActionContext<'_>) -> Result<bool, ActionError> {
        let Some(walk) = self.walk.as_mut() else {
            return Ok(false);
        };
        let polled = walk.poll(cx);
        let receipt = match polled {
            Poll::Pending => return Ok(true),
            Poll::Ready(Ok(receipt)) => receipt,
            Poll::Ready(Err(error)) => {
                self.walk = None;
                return Err(error);
            }
        };
        self.walk = None;
        // Manual takeover, revocation and pending evidence remain terminal.
        if self
            .walk_retry
            .observe_approach_failure(&receipt, cx.evidence().tick)
        {
            return Ok(false);
        }
        match receipt.into_arrival() {
            Ok(_) => {
                self.walk_retry.reset();
                Ok(false)
            }
            // Non-retryable and exhausted failures retain this receipt's detail.
            Err(error) => Err(error),
        }
    }

    fn cancel_walk(&mut self, cx: &mut ActionContext<'_>) {
        if let Some(walk) = self.walk.take() {
            cx.cancel_request(walk.request_id());
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

fn matches_target(npc: &NpcView, target: &Target) -> bool {
    i32::try_from(npc.index).is_ok()
        && npc.r#type.and_then(|id| i32::try_from(id).ok()) == Some(target.id)
        && npc
            .name
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case(target.display.as_ref()))
        && npc
            .actions
            .iter()
            .flatten()
            .any(|action| action.eq_ignore_ascii_case(target.action.as_ref()))
}

/// Least demanding configured target type. The action cannot know which
/// configured spawn will appear, so any of them the account can attempt keeps
/// the approach valid; a visible NPC still gates on its own selected level in
/// `select_target` and in the core.
fn static_required_level(targets: &[Target]) -> Option<i32> {
    targets.iter().map(|target| target.required_level).min()
}

fn select_target<'a>(
    cx: &ActionContext<'_>,
    targets: &'a [Target],
    anchor: Option<WorldTile>,
    radius: u8,
    effective_level: Option<i32>,
) -> Option<&'a Target> {
    let area = reach::Area::new(anchor, i32::from(radius));
    let avoid = reach::Avoid::default().at(cx.evidence().tick);
    // The thieving type, level gate and radius are chooser inputs: the
    // shared chooser ranks eligible matches by walking distance inside the
    // same anchor area. Fall back to any match so a level refusal still
    // surfaces through the core's required-level gate.
    if let Some(npc) = reach::choose_npc_matching(cx, area, &avoid, |npc| {
        targets.iter().any(|target| {
            matches_target(npc, target)
                && level_ready(effective_level, target.required_level) == Some(true)
        })
    })
    .any()
    {
        return targets.iter().find(|target| {
            matches_target(npc, target)
                && level_ready(effective_level, target.required_level) == Some(true)
        });
    }
    let npc = reach::choose_npc_matching(cx, area, &avoid, |npc| {
        targets.iter().any(|target| matches_target(npc, target))
    })
    .any()?;
    targets.iter().find(|target| matches_target(npc, target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::thieving_core::DEFAULT_ACTION_DEADLINE;
    use crate::native::WalkEnd;
    use crate::quester::families::tests::{
        def, local_player, post_user_input_walk_receipt, post_walk_receipt, with_tick,
        with_tick_reach,
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
            respawn_ticks: 100,
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
            no_target_wait_ticks: 101,
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
        game_snapshot_with_level(here, npcs, inventory, 30)
    }

    fn game_snapshot_with_level(
        here: WorldTile,
        npcs: Vec<NpcView>,
        inventory: Vec<ItemView>,
        thieving: i32,
    ) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(inventory, 28);
        snapshot.seed_stats(vec![StatView {
            index: 17,
            name: "thieving".into(),
            effective: thieving,
            base: thieving,
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

    fn walk_request_count(ledger: &Option<Box<crate::native::ledger::Ledger>>) -> usize {
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .filter(|action| matches!(&action.effect, crate::native::HostEffect::Walk(_)))
            .count()
    }

    /// Publish a terminal receipt for the newest queued walk.
    fn post_walk_end(
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
        tick: u64,
        end: WalkEnd,
        detail: Option<&str>,
    ) {
        let (request_id, run) = {
            let ledger = ledger.as_ref().expect("walk request must be queued");
            let action = ledger
                .outbox
                .iter()
                .rev()
                .find(|action| matches!(&action.effect, crate::native::HostEffect::Walk(_)))
                .expect("walk request must be queued");
            (action.request_id.get(), action.run())
        };
        ledger.as_mut().expect("ledger must remain present").walk =
            Some(crate::native::WalkReceipt {
                goal: None,
                request_id,
                evidence: api::quest_progress::EvidenceStamp {
                    run,
                    tick,
                    sequence: tick,
                },
                end,
                blocked: None,
                detail: detail.map(Arc::from),
                refusal: None,
                assessment: None,
                escape: None,
            });
    }

    fn args_with_level(
        anchor: WorldTile,
        radius: u8,
        required_level: i32,
        deadline: Duration,
    ) -> ThieveActionArgs {
        ThieveActionArgs {
            targets: Arc::from([target(1, required_level)]),
            item_id: 995,
            goal_qty: 1,
            anchor: Some(anchor),
            radius,
            deadline,
            no_target_wait_ticks: 101,
        }
    }

    /// A step compiled for one target whose selected NPC config declares
    /// `respawn_ticks`; the empty-anchor bound is that rate plus one tick.
    fn anchor_wait_args(
        anchor: WorldTile,
        required_level: i32,
        respawn_ticks: u32,
        deadline: Duration,
    ) -> ThieveActionArgs {
        ThieveActionArgs {
            targets: Arc::from([Target {
                respawn_ticks,
                ..target(1, required_level)
            }]),
            item_id: 995,
            goal_qty: 1,
            anchor: Some(anchor),
            radius: 12,
            deadline,
            no_target_wait_ticks: u64::from(respawn_ticks) + 1,
        }
    }

    #[test]
    fn same_tick_thieve_level_up_defers_modal_and_next_dispatch() {
        use crate::quester::families::tests::with_tick_snapshots;
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let snapshot = game_snapshot(anchor, vec![npc(19, 1, 1)], vec![]);
        let mut later = game_snapshot(anchor, vec![npc(19, 1, 1)], vec![]);
        later.seed_chat_modal(100, vec!["Congratulations, you advanced Thieving.".into()]);
        later.seed_chat_options(Vec::new(), 99);
        let mut ledger = None;
        let run = with_tick_snapshots(&snapshot, &later, &mut ledger, 1, |tick, later| {
            let run = tick
                .actions
                .begin::<Thieve>(args(anchor, 12), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&run, &mut tick.cx).is_pending());
            tick.cx.snapshot = SnapshotView::new(Some(later), tick.cx.evidence());
            for _ in 0..3 {
                assert!(tick.actions.poll(&run, &mut tick.cx).is_pending());
            }
            run
        });
        assert_eq!(npc_click_count(&ledger), 1);
        assert_eq!(continue_dialog_count(&ledger), 0);
        assert!(with_tick(&later, &mut ledger, 2, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(continue_dialog_count(&ledger), 1);
        drop(run);

        let mut ledger = None;
        let run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            let run = tick
                .actions
                .begin::<Thieve>(args(anchor, 12), &mut tick.cx)
                .unwrap();
            tick.cx.emit(crate::shim::InteractReq::CloseModal).unwrap();
            for _ in 0..3 {
                assert!(tick.actions.poll(&run, &mut tick.cx).is_pending());
            }
            run
        });
        assert_eq!(npc_click_count(&ledger), 0);
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(npc_click_count(&ledger), 1);
    }

    #[test]
    fn same_tick_thieve_anchor_walk_retries_after_spent_event() {
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
            let run = tick
                .actions
                .begin::<Thieve>(args(anchor, 2), &mut tick.cx)
                .unwrap();
            tick.cx.emit(crate::shim::InteractReq::CloseModal).unwrap();
            for _ in 0..3 {
                assert!(tick.actions.poll(&run, &mut tick.cx).is_pending());
                assert_eq!(tick.cx.ledger.as_ref().unwrap().outbox.len(), 1);
            }
            run
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert!(matches!(
            &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
            crate::native::HostEffect::Walk(request)
                if request.target == anchor && request.radius == 2
        ));
    }

    #[test]
    fn outside_radius_without_a_visible_target_walks_to_the_anchor_on_the_first_poll() {
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
            vec![],
            vec![],
        );
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            let run = tick
                .actions
                .begin::<Thieve>(args(anchor, 12), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&run, &mut tick.cx).is_pending());
        });
        assert_eq!(walk_request_count(&ledger), 1);
        assert!(
            !ledger
                .as_ref()
                .unwrap()
                .outbox
                .iter()
                .any(|action| matches!(&action.effect, crate::native::HostEffect::Interaction(_))),
            "no click may be sent before the anchor is reached"
        );
        assert!(matches!(
            &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
            crate::native::HostEffect::Walk(request)
                if request.target == anchor && request.radius == 12
        ));
    }

    /// The anchor gate is Chebyshev `within`, so any standable tile inside
    /// the radius satisfies the action. The walk it starts must therefore
    /// ask for an area arrival, not a path onto the anchor tile.
    #[test]
    fn anchor_walk_asks_for_area_arrival_at_the_authored_radius() {
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
            vec![],
            vec![],
        );
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            let run = tick
                .actions
                .begin::<Thieve>(args(anchor, 12), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&run, &mut tick.cx).is_pending());
        });
        let crate::native::HostEffect::Walk(request) =
            &ledger.as_ref().unwrap().outbox.last().unwrap().effect
        else {
            panic!("the anchor approach must emit a native walk");
        };
        assert_eq!(request.target, anchor);
        assert_eq!(request.radius, 12);
        assert_eq!(request.loc_id, None);
        assert_eq!(
            request.arrival,
            nav_planner::contract::ArrivalKind::Area,
            "the Chebyshev anchor gate accepts any standable tile in the radius"
        );
    }

    /// The live failure ended `RouteEnded` on a standable tile ten tiles from
    /// the anchor, with the anchor reachable only a long way round. An area
    /// arrival must count that tile as arrival, so the action can look for a
    /// workman; reach fails it `walk route ended before arrival`.
    #[test]
    fn area_anchor_walk_accepts_a_standable_tile_without_a_path_onto_the_anchor() {
        let anchor = WorldTile {
            x: 20,
            z: 20,
            level: 0,
        };
        let far = WorldTile {
            x: 0,
            z: 0,
            level: 0,
        };
        // Ten tiles east of the anchor by Chebyshev distance: inside radius 12.
        let inside = WorldTile {
            x: 10,
            z: 20,
            level: 0,
        };
        let mark = WorldTile {
            x: 11,
            z: 20,
            level: 0,
        };
        let base = WorldTile {
            x: -8,
            z: -8,
            level: 0,
        };
        // A closed pen around the anchor: no walkable path onto its tile.
        let walls = pen(anchor);
        let far_view = flood_view(base, 32, &walls, far);
        let inside_view = flood_view(base, 32, &walls, inside);
        let outside = game_snapshot(far, vec![], vec![]);
        let arrived = game_snapshot(inside, vec![placed_npc(19, mark, inside)], vec![]);
        let mut ledger = None;
        // The action emits its anchor approach from outside the radius, then
        // gives up the foreground so the walk adapter can replay that exact
        // contract (`NativeActions::begin` admits one live owner).
        let (target, radius, loc_id, arrival) =
            with_tick_reach(&outside, &far_view, &mut ledger, 1, |tick| {
                let run = tick
                    .actions
                    .begin::<Thieve>(args(anchor, 12), &mut tick.cx)
                    .unwrap();
                assert!(tick.actions.poll(&run, &mut tick.cx).is_pending());
                let crate::native::HostEffect::Walk(request) = &tick
                    .cx
                    .ledger
                    .as_ref()
                    .unwrap()
                    .outbox
                    .last()
                    .unwrap()
                    .effect
                else {
                    panic!("the anchor approach must emit a native walk");
                };
                (
                    request.target,
                    request.radius,
                    request.loc_id,
                    request.arrival,
                )
            });
        assert_eq!((target, radius), (anchor, 12));

        // The host walk ends `RouteEnded` on the standable in-radius tile.
        // The adapter's arrival predicate decides whether that is arrival.
        let walk = with_tick_reach(&arrived, &inside_view, &mut ledger, 2, |tick| {
            let mut replay = reach::walk_request(target, radius, loc_id, tick.cx.evidence());
            replay.arrival = arrival;
            tick.actions
                .begin::<crate::native::walk::Walk>(replay, &mut tick.cx)
                .unwrap()
        });
        post_walk_end(&mut ledger, 3, WalkEnd::RouteEnded, None);
        let receipt = with_tick_reach(&arrived, &inside_view, &mut ledger, 3, |tick| {
            match tick.actions.poll(&walk, &mut tick.cx) {
                Poll::Ready(Ok(receipt)) => receipt,
                Poll::Ready(Err(error)) => panic!("the walk adapter failed: {error:?}"),
                Poll::Pending => {
                    panic!("the walk adapter must settle on the standable in-radius tile")
                }
            }
        });
        drop(walk);
        if let Err(error) = receipt.into_arrival() {
            panic!(
                "a standable in-radius tile must end the walk in the action's favour: {error:?}"
            );
        }

        // That arrival lets the action look for the workman from the tile.
        let run = with_tick_reach(&outside, &far_view, &mut ledger, 4, |tick| {
            let run = tick
                .actions
                .begin::<Thieve>(args(anchor, 12), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&run, &mut tick.cx).is_pending());
            run
        });
        post_walk_end(&mut ledger, 5, WalkEnd::Arrived, None);
        let outcome = with_tick_reach(&arrived, &inside_view, &mut ledger, 5, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        });
        assert!(
            outcome.is_pending(),
            "the thieve action must keep running after the anchor approach"
        );
        assert_eq!(
            npc_click_count(&ledger),
            1,
            "after an area arrival the action looks for the workman"
        );
    }

    /// The live Digsite approach failed on its second route end. Request 131
    /// stopped thirteen tiles from the anchor `[3359,3401,0]`, which spent
    /// the first retry. Request 132 stopped on `[3347,3395,0]`, exactly twelve
    /// tiles away at radius 12. The action asks for an area arrival, so the
    /// shared walk adapter must settle that second receipt as arrival and the
    /// action must go on to look for a workman. Reading the raw receipt
    /// instead ends the step with "walk route ended before arrival".
    #[test]
    fn route_ended_at_the_radius_after_a_retry_is_an_area_arrival() {
        let anchor = WorldTile {
            x: 3359,
            z: 3401,
            level: 0,
        };
        // The live start, fifty-seven tiles north of the anchor.
        let start = WorldTile {
            x: 3357,
            z: 3344,
            level: 0,
        };
        // The live request 131 stop: thirteen tiles away.
        let too_short = WorldTile {
            x: 3346,
            z: 3394,
            level: 0,
        };
        // The live request 132 stop: twelve west, six south of the anchor.
        let inside = WorldTile {
            x: 3347,
            z: 3395,
            level: 0,
        };
        let workman_at = WorldTile {
            x: 3348,
            z: 3395,
            level: 0,
        };
        let view = flood_view(
            WorldTile {
                x: 3330,
                z: 3330,
                level: 0,
            },
            96,
            &[],
            inside,
        );
        assert!(
            !reach::within(start, anchor, 12),
            "the approach starts outside"
        );
        assert!(
            !reach::within(too_short, anchor, 12),
            "request 131 stopped outside the radius"
        );
        assert!(
            reach::within(inside, anchor, 12) && !reach::within(inside, anchor, 11),
            "the retry tile sits exactly on the authored radius"
        );
        assert!(
            view.walkable(inside),
            "the retry tile is standable, so only the receipt settlement decides"
        );

        let outside = game_snapshot(start, vec![], vec![]);
        let short = game_snapshot(too_short, vec![], vec![]);
        let arrived = game_snapshot(inside, vec![placed_npc(19, workman_at, inside)], vec![]);
        let mut ledger = None;
        let run = with_tick_reach(&outside, &view, &mut ledger, 1, |tick| {
            let run = tick
                .actions
                .begin::<Thieve>(args(anchor, 12), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&run, &mut tick.cx).is_pending());
            run
        });
        assert_eq!(walk_request_count(&ledger), 1, "one area approach walk");

        // Request 131: the host stops the area route outside the radius, so
        // the action defers its fresh plan until the next observed tick.
        post_walk_end(&mut ledger, 2, WalkEnd::RouteEnded, None);
        assert!(
            with_tick_reach(&short, &view, &mut ledger, 2, |tick| {
                tick.actions.poll(&run, &mut tick.cx)
            })
            .is_pending(),
            "the action defers after the first route end"
        );
        assert_eq!(
            walk_request_count(&ledger),
            1,
            "the next walk must wait for a new tick"
        );
        assert!(with_tick_reach(&short, &view, &mut ledger, 3, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(walk_request_count(&ledger), 2, "the retry walk is submitted");

        // Request 132: the host stops on a standable tile at the radius.
        post_walk_end(&mut ledger, 4, WalkEnd::RouteEnded, None);
        let outcome = with_tick_reach(&arrived, &view, &mut ledger, 4, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        });
        assert!(
            outcome.is_pending(),
            "the second route end must be an area arrival, not a failure: {outcome:?}"
        );
        assert_eq!(
            walk_request_count(&ledger),
            2,
            "an arrival inside the radius must not replan"
        );
        assert_eq!(
            npc_click_count(&ledger),
            1,
            "after the area arrival the action looks for the workman"
        );
    }

    /// A route end outside the radius is still a walk failure: the shared
    /// adapter keeps `RouteEnded`, and the action spends a bounded retry
    /// from the observed position. The live request 131 ended thirteen tiles
    /// from the anchor on `[3346,3394,0]`.
    #[test]
    fn route_ended_outside_the_radius_is_a_walk_failure_with_one_replan() {
        let anchor = WorldTile {
            x: 3359,
            z: 3401,
            level: 0,
        };
        let start = WorldTile {
            x: 3357,
            z: 3344,
            level: 0,
        };
        let short = WorldTile {
            x: 3346,
            z: 3394,
            level: 0,
        };
        let view = flood_view(
            WorldTile {
                x: 3330,
                z: 3330,
                level: 0,
            },
            96,
            &[],
            short,
        );
        assert!(
            !reach::within(short, anchor, 12),
            "the stop tile is outside the authored radius"
        );
        assert!(
            view.walkable(short),
            "the tile is standable, so only the radius can reject the arrival"
        );

        let outside = game_snapshot(start, vec![], vec![]);
        let stopped_short = game_snapshot(short, vec![], vec![]);
        let mut ledger = None;
        let run = with_tick_reach(&outside, &view, &mut ledger, 1, |tick| {
            let run = tick
                .actions
                .begin::<Thieve>(args(anchor, 12), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&run, &mut tick.cx).is_pending());
            run
        });
        assert_eq!(walk_request_count(&ledger), 1);

        post_walk_end(&mut ledger, 2, WalkEnd::RouteEnded, None);
        assert!(
            with_tick_reach(&stopped_short, &view, &mut ledger, 2, |tick| {
                tick.actions.poll(&run, &mut tick.cx)
            })
            .is_pending(),
            "the action retries instead of ending on the first walk failure"
        );
        assert_eq!(
            walk_request_count(&ledger),
            1,
            "the retry waits until the next observed tick"
        );
        assert!(with_tick_reach(&stopped_short, &view, &mut ledger, 3, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(
            walk_request_count(&ledger),
            2,
            "a route end outside the radius spends one bounded retry"
        );
    }

    #[test]
    fn failed_approach_walk_replans_once_then_pickpockets() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let outside = game_snapshot(
            WorldTile {
                x: 30,
                z: 40,
                level: 0,
            },
            vec![],
            vec![],
        );
        let inside = game_snapshot(anchor, vec![npc(19, 1, 1)], vec![]);
        let mut ledger = None;
        let run = with_tick(&outside, &mut ledger, 1, |tick| {
            let run = tick
                .actions
                .begin::<Thieve>(args(anchor, 12), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&run, &mut tick.cx).is_pending());
            run
        });
        assert_eq!(walk_request_count(&ledger), 1);

        post_walk_end(
            &mut ledger,
            2,
            WalkEnd::Failed,
            Some("route fixture failure 1"),
        );
        assert!(with_tick(&outside, &mut ledger, 2, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(walk_request_count(&ledger), 1, "retry waits for the next tick");
        assert!(with_tick(&outside, &mut ledger, 3, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(walk_request_count(&ledger), 2, "one fresh plan is submitted");

        post_walk_end(&mut ledger, 4, WalkEnd::Arrived, None);
        assert!(with_tick(&inside, &mut ledger, 4, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(npc_click_count(&ledger), 1);
    }

    #[test]
    fn repeated_approach_walk_failure_ends_after_the_bound_with_the_last_detail() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let outside = game_snapshot(
            WorldTile {
                x: 30,
                z: 40,
                level: 0,
            },
            vec![],
            vec![],
        );
        let mut ledger = None;
        let run = with_tick(&outside, &mut ledger, 1, |tick| {
            let run = tick
                .actions
                .begin::<Thieve>(args(anchor, 12), &mut tick.cx)
                .unwrap();
            assert!(tick.actions.poll(&run, &mut tick.cx).is_pending());
            run
        });

        post_walk_end(
            &mut ledger,
            2,
            WalkEnd::Failed,
            Some("route fixture failure 1"),
        );
        assert!(with_tick(&outside, &mut ledger, 2, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(walk_request_count(&ledger), 1, "retry waits for the next tick");
        assert!(with_tick(&outside, &mut ledger, 3, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(walk_request_count(&ledger), 2);

        post_walk_end(
            &mut ledger,
            4,
            WalkEnd::Failed,
            Some("route fixture failure 2"),
        );
        assert!(with_tick(&outside, &mut ledger, 4, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(walk_request_count(&ledger), 2, "second retry waits for the next tick");
        assert!(with_tick(&outside, &mut ledger, 5, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(walk_request_count(&ledger), 3);

        post_walk_end(
            &mut ledger,
            6,
            WalkEnd::Failed,
            Some("route fixture failure 3"),
        );
        assert!(matches!(
            with_tick(&outside, &mut ledger, 6, |tick| {
                tick.actions.poll(&run, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::Blocked(detail)))
                if detail.as_ref() == "route fixture failure 3"
        ));
        assert_eq!(
            walk_request_count(&ledger),
            3,
            "the shared budget includes the initial route and two replans"
        );
    }

    #[test]
    fn empty_anchor_fails_by_name_at_the_respawn_bound() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let snapshot = game_snapshot(anchor, vec![], vec![]);
        let mut ledger = None;
        let run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            let run = tick
                .actions
                .begin::<Thieve>(
                    args_with_level(anchor, 12, 1, Duration::from_secs(600)),
                    &mut tick.cx,
                )
                .unwrap();
            assert!(tick.actions.poll(&run, &mut tick.cx).is_pending());
            run
        });
        for tick in 2..=101 {
            assert!(
                with_tick(&snapshot, &mut ledger, tick, |tick| {
                    tick.actions.poll(&run, &mut tick.cx)
                })
                .is_pending(),
                "tick {tick} is inside the respawn window"
            );
        }
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 102, |tick| {
                tick.actions.poll(&run, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::Failed(detail)))
                if detail.as_ref() == "no thieving target appeared at the anchor"
        ));
        assert_eq!(walk_request_count(&ledger), 0);
        assert_eq!(npc_click_count(&ledger), 0);
    }

    #[test]
    fn anchor_wait_follows_the_selected_spawns_own_respawn_window() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let snapshot = game_snapshot(anchor, vec![], vec![]);
        let mut ledger = None;
        let run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            let run = tick
                .actions
                .begin::<Thieve>(
                    anchor_wait_args(anchor, 1, 120, Duration::from_secs(600)),
                    &mut tick.cx,
                )
                .unwrap();
            assert!(tick.actions.poll(&run, &mut tick.cx).is_pending());
            run
        });
        for tick in 2..=121 {
            assert!(
                with_tick(&snapshot, &mut ledger, tick, |tick| {
                    tick.actions.poll(&run, &mut tick.cx)
                })
                .is_pending(),
                "tick {tick} is still inside the 120-tick respawn window"
            );
        }
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 122, |tick| {
                tick.actions.poll(&run, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::Failed(detail)))
                if detail.as_ref() == "no thieving target appeared at the anchor"
        ));
        assert_eq!(walk_request_count(&ledger), 0);
        assert_eq!(npc_click_count(&ledger), 0);
    }

    #[test]
    fn anchor_spawn_at_the_deadline_is_clicked_and_its_first_attempt_survives() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let empty = game_snapshot(anchor, vec![], vec![]);
        let spawned = game_snapshot(anchor, vec![npc(19, 1, 1)], vec![]);
        let mut ledger = None;
        let run = with_tick(&empty, &mut ledger, 1, |tick| {
            let run = tick
                .actions
                .begin::<Thieve>(
                    anchor_wait_args(anchor, 1, 100, Duration::from_secs(60)),
                    &mut tick.cx,
                )
                .unwrap();
            assert!(tick.actions.poll(&run, &mut tick.cx).is_pending());
            run
        });
        for tick in 2..100 {
            assert!(
                with_tick(&empty, &mut ledger, tick, |tick| {
                    tick.actions.poll(&run, &mut tick.cx)
                })
                .is_pending(),
                "tick {tick} is before the deadline"
            );
        }
        // 600 ms ticks put tick 100 exactly on the 60 s deadline. The target
        // appears on that tick, inside the respawn bound.
        assert!(with_tick(&spawned, &mut ledger, 100, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(
            npc_click_count(&ledger),
            1,
            "a spawn that appears on the deadline tick is still clicked"
        );
        for tick in 101..=102 {
            assert!(
                with_tick(&spawned, &mut ledger, tick, |tick| {
                    tick.actions.poll(&run, &mut tick.cx)
                })
                .is_pending(),
                "tick {tick}: the first anchor attempt must not be killed by the deadline"
            );
        }
    }

    #[test]
    fn anchor_wait_does_not_end_on_the_deadline() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let snapshot = game_snapshot(anchor, vec![], vec![]);
        let mut ledger = None;
        let run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            let run = tick
                .actions
                .begin::<Thieve>(
                    anchor_wait_args(anchor, 1, 100, Duration::from_secs(60)),
                    &mut tick.cx,
                )
                .unwrap();
            assert!(tick.actions.poll(&run, &mut tick.cx).is_pending());
            run
        });
        for tick in 2..=101 {
            assert!(
                with_tick(&snapshot, &mut ledger, tick, |tick| {
                    tick.actions.poll(&run, &mut tick.cx)
                })
                .is_pending(),
                "tick {tick} is inside the respawn bound, past the 60 s deadline"
            );
        }
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 102, |tick| {
                tick.actions.poll(&run, &mut tick.cx)
            }),
            Poll::Ready(Err(ActionError::Failed(detail)))
                if detail.as_ref() == "no thieving target appeared at the anchor"
        ));
    }

    #[test]
    fn below_the_static_requirement_refuses_before_any_walk_or_click() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let lowered = game_snapshot_with_level(
            WorldTile {
                x: 30,
                z: 40,
                level: 0,
            },
            vec![],
            vec![],
            24,
        );
        let mut ledger = None;
        with_tick(&lowered, &mut ledger, 1, |tick| {
            let run = tick
                .actions
                .begin::<Thieve>(
                    args_with_level(anchor, 12, 25, Duration::from_secs(60)),
                    &mut tick.cx,
                )
                .unwrap();
            assert!(matches!(
                tick.actions.poll(&run, &mut tick.cx),
                Poll::Ready(Err(ActionError::Failed(detail)))
                    if detail.as_ref()
                        == "effective Thieving is below the selected NPC requirement"
            ));
        });
        assert_eq!(walk_request_count(&ledger), 0);
        assert_eq!(npc_click_count(&ledger), 0);
    }

    #[test]
    fn visible_target_from_the_start_keeps_the_existing_pickpocket_path() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let snapshot = game_snapshot(anchor, vec![npc(19, 1, 1)], vec![]);
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
        assert_eq!(npc_click_count(&ledger), 1);
        assert_eq!(walk_request_count(&ledger), 0);
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
    fn thieve_anchor_walk_retries_after_failure_from_the_observed_tile() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let start = WorldTile {
            x: 30,
            z: 40,
            level: 0,
        };
        let snapshot = game_snapshot(start, vec![npc(19, 1, 1)], vec![]);
        let mut ledger = None;
        let run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            let run = tick
                .actions
                .begin::<Thieve>(args(anchor, 2), &mut tick.cx)
                .unwrap();
            tick.cx.emit(crate::shim::InteractReq::CloseModal).unwrap();
            for _ in 0..3 {
                assert!(tick.actions.poll(&run, &mut tick.cx).is_pending());
            }
            run
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        let first_request = post_walk_receipt(
            &mut ledger,
            3,
            WalkEnd::NoPath,
            Some("first thieving route failed"),
        );
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(walk_request_count(&ledger), 1);

        let moved = WorldTile {
            x: 29,
            z: 39,
            level: 0,
        };
        let moved_snapshot = game_snapshot(moved, vec![npc(19, 1, 1)], vec![]);
        assert!(with_tick(&moved_snapshot, &mut ledger, 4, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(walk_request_count(&ledger), 2);
        let retry = ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .rev()
            .find(|action| matches!(&action.effect, crate::native::HostEffect::Walk(_)))
            .unwrap();
        let crate::native::HostEffect::Walk(request) = &retry.effect else {
            unreachable!();
        };
        assert_eq!(request.target, anchor);
        assert_eq!(request.required_after.tick, 4);

        let second_request = post_walk_receipt(&mut ledger, 5, WalkEnd::Arrived, None);
        assert_ne!(second_request, first_request);
        let arrived = game_snapshot(anchor, vec![npc(19, 1, 1)], vec![]);
        assert!(with_tick(&arrived, &mut ledger, 5, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(walk_request_count(&ledger), 2);
    }

    #[test]
    fn thieve_area_entry_resets_retry_before_a_separate_approach() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let outside = game_snapshot(
            WorldTile {
                x: 30,
                z: 40,
                level: 0,
            },
            vec![npc(19, 1, 1)],
            vec![],
        );
        let mut ledger = None;
        let mut action_id = 0;
        let run = with_tick(&outside, &mut ledger, 1, |tick| {
            let run = tick
                .actions
                .begin::<Thieve>(args(anchor, 2), &mut tick.cx)
                .unwrap();
            action_id = tick.cx.action_id;
            tick.cx.emit(crate::shim::InteractReq::CloseModal).unwrap();
            for _ in 0..3 {
                assert!(tick.actions.poll(&run, &mut tick.cx).is_pending());
            }
            run
        });
        assert!(with_tick(&outside, &mut ledger, 2, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        post_walk_receipt(
            &mut ledger,
            3,
            WalkEnd::NoPath,
            Some("first approach failed"),
        );
        assert!(with_tick(&outside, &mut ledger, 3, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert!(run.machine.borrow().as_ref().unwrap().walk_retry.pending());
        assert!(with_tick(&outside, &mut ledger, 4, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(walk_request_count(&ledger), 2, "the first retry is submitted");
        post_walk_receipt(&mut ledger, 5, WalkEnd::NoPath, Some("first retry failed"));
        assert!(with_tick(&outside, &mut ledger, 5, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert!(run.machine.borrow().as_ref().unwrap().walk_retry.pending());

        let inside = game_snapshot(anchor, vec![npc(19, 1, 1)], vec![]);
        assert!(with_tick(&inside, &mut ledger, 6, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert!(!run.machine.borrow().as_ref().unwrap().walk_retry.pending());

        with_tick(&outside, &mut ledger, 7, |tick| {
            tick.cx.action_id = action_id;
            let mut machine = run.machine.borrow_mut();
            let machine = machine.as_mut().unwrap();
            machine.begin_walk(anchor, 2, &mut tick.cx).unwrap();
            machine.walk_retry.action_submitted();
        });
        post_walk_receipt(
            &mut ledger,
            8,
            WalkEnd::NoPath,
            Some("separate approach failed"),
        );
        assert!(with_tick(&outside, &mut ledger, 8, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert!(
            run.machine.borrow().as_ref().unwrap().walk_retry.pending(),
            "a separate approach receives a fresh retry allowance"
        );
        assert_eq!(walk_request_count(&ledger), 3);
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
        let mut bad = npc(19, 1, 1);
        bad.actions = vec![Some("Talk-to".into())];
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let snapshot = game_snapshot(anchor, vec![bad], vec![]);
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            assert!(select_target(&tick.cx, &targets, Some(anchor), 12, Some(30)).is_none());
        });
    }
    fn flood_view(
        base: WorldTile,
        size: i32,
        flags: &[(WorldTile, i32)],
        player: WorldTile,
    ) -> api::query::ReachQueryView {
        let mut scene = api::snapshot::SceneView {
            available: true,
            base_x: base.x,
            base_z: base.z,
            level: 0,
            width: size,
            height: size,
            collision_flags: vec![0; (size * size) as usize],
        };
        for (tile, flag) in flags {
            let index = ((tile.x - base.x) * size + tile.z - base.z) as usize;
            scene.collision_flags[index] |= flag;
        }
        let flood = api::query::SceneQuery::new(&scene, Some(player)).flood_reach();
        api::query::pack_reach_query(&scene, flood.as_ref())
    }

    fn pen(center: WorldTile) -> Vec<(WorldTile, i32)> {
        (-1..=1)
            .flat_map(|dx| (-1..=1).map(move |dz| (dx, dz)))
            .filter(|&(dx, dz)| dx != 0 || dz != 0)
            .map(|(dx, dz)| {
                (
                    WorldTile {
                        x: center.x + dx,
                        z: center.z + dz,
                        level: 0,
                    },
                    client::dash3d::CollisionFlag::SQ_BLOCKED,
                )
            })
            .collect()
    }

    fn placed_npc(index: usize, at: WorldTile, from: WorldTile) -> NpcView {
        let mut npc = npc(index, 1, (at.x - from.x).abs().max((at.z - from.z).abs()));
        npc.tile = at;
        npc.network = at;
        npc.x = at.x;
        npc.z = at.z;
        npc
    }

    #[test]
    fn thieving_pick_prefers_a_reachable_npc_over_a_nearer_one_behind_a_wall() {
        let player = WorldTile {
            x: 0,
            z: 0,
            level: 0,
        };
        // The nearer mark sits inside a closed pen two tiles away; the
        // reachable mark is eleven tiles away, past the shared probe but
        // inside the authored twelve-tile radius.
        let near_at = WorldTile {
            x: 2,
            z: 0,
            level: 0,
        };
        let far_at = WorldTile {
            x: 11,
            z: 0,
            level: 0,
        };
        let snapshot = game_snapshot(
            player,
            vec![
                placed_npc(11, near_at, player),
                placed_npc(22, far_at, player),
            ],
            vec![],
        );
        let view = flood_view(
            WorldTile {
                x: -8,
                z: -8,
                level: 0,
            },
            32,
            &pen(near_at),
            player,
        );
        let mut ledger = None;
        let run = with_tick_reach(&snapshot, &view, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<Thieve>(args(player, 12), &mut tick.cx)
                .unwrap()
        });
        assert!(with_tick_reach(&snapshot, &view, &mut ledger, 2, |tick| {
            tick.actions.poll(&run, &mut tick.cx)
        })
        .is_pending());
        assert!(
            matches!(
                &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
                crate::native::HostEffect::Interaction(crate::shim::InteractReq::Npc {
                    index: Some(22),
                    ..
                })
            ),
            "thieving must dispatch the reachable mark past the probe",
        );
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

    // --- QUESTER-READ-KEEPS-STEP-R2: a guardian hold interrupts an attempt ---

    use crate::native::ledger::TickBudget;
    use crate::quester::families::tests::{
        host_accepts, host_drops_queued_work, host_takes_queued_work, with_tick_budget,
    };

    fn begin_thieve_at_anchor(
        snapshot: &GameSnapshot,
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
        budget: &mut TickBudget,
    ) -> crate::native::ActionHandle<Thieve> {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let handle = with_tick_budget(snapshot, ledger, budget, 1, |tick| {
            tick.actions
                .begin::<Thieve>(args(anchor, 12), &mut tick.cx)
                .unwrap()
        });
        assert!(with_tick_budget(snapshot, ledger, budget, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        assert_eq!(npc_click_count(ledger), 1, "the first attempt is queued");
        handle
    }

    fn poll_thieve(
        handle: &crate::native::ActionHandle<Thieve>,
        snapshot: &GameSnapshot,
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
        budget: &mut TickBudget,
        tick: u64,
    ) -> Poll<Result<i32, ActionError>> {
        with_tick_budget(snapshot, ledger, budget, tick, |tick| {
            tick.actions.poll(handle, &mut tick.cx)
        })
    }

    /// R1, Thieve, request dropped unsent: the attempt never reached the
    /// server. Left alone the core ends the action at its receipt deadline
    /// (`ReceiptTimeout`), 2.5 s on the active clock; instead the attempt is
    /// forgotten and dispatched once more after the settle tick.
    #[test]
    fn a_pickpocket_dropped_unsent_by_a_guardian_hold_is_attempted_again_once() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let snapshot = game_snapshot(anchor, vec![npc(19, 1, 1)], vec![]);
        let (mut ledger, mut budget) = (None, TickBudget::default());
        let handle = begin_thieve_at_anchor(&snapshot, &mut ledger, &mut budget);
        let first = ledger.as_ref().unwrap().outbox[0].request_id;
        budget.hold_begin();
        assert_eq!(host_drops_queued_work(&mut ledger), 1);
        budget.hold_end();

        assert!(poll_thieve(&handle, &snapshot, &mut ledger, &mut budget, 9).is_pending());
        assert_eq!(npc_click_count(&ledger), 0, "the settle tick sends nothing");
        assert!(poll_thieve(&handle, &snapshot, &mut ledger, &mut budget, 10).is_pending());
        assert_eq!(
            npc_click_count(&ledger),
            1,
            "one attempt is dispatched again"
        );
        assert_ne!(ledger.as_ref().unwrap().outbox[0].request_id, first);
        assert!(poll_thieve(&handle, &snapshot, &mut ledger, &mut budget, 11).is_pending());
        assert_eq!(npc_click_count(&ledger), 1, "and no second one");
        let log = crate::native::take_resume_log();
        assert!(
            log.iter()
                .any(|line| line.contains("re-issued its request once: thieving attempt")),
            "{log:?}"
        );
    }

    #[test]
    fn an_inline_anchor_walk_dropped_unsent_resumes_once_without_spending_a_retry() {
        let anchor = WorldTile { x: 3, z: 4, level: 0 };
        let here = WorldTile { x: 30, z: 40, level: 0 };
        let snapshot = game_snapshot(here, Vec::new(), Vec::new());
        let (mut ledger, mut budget) = (None, TickBudget::default());
        let handle = with_tick_budget(&snapshot, &mut ledger, &mut budget, 1, |tick| {
            tick.actions.begin::<Thieve>(args(anchor, 12), &mut tick.cx).unwrap()
        });
        assert!(poll_thieve(&handle, &snapshot, &mut ledger, &mut budget, 2).is_pending());
        assert_eq!(walk_request_count(&ledger), 1);
        assert_eq!(npc_click_count(&ledger), 0);
        let first = ledger.as_ref().unwrap().outbox[0].request_id;
        budget.hold_begin();
        assert_eq!(host_drops_queued_work(&mut ledger), 1);
        budget.hold_end();

        assert!(poll_thieve(&handle, &snapshot, &mut ledger, &mut budget, 9).is_pending());
        assert_eq!(walk_request_count(&ledger), 0, "the settle tick sends nothing");
        assert!(poll_thieve(&handle, &snapshot, &mut ledger, &mut budget, 10).is_pending());
        assert_eq!(walk_request_count(&ledger), 1);
        assert_eq!(npc_click_count(&ledger), 0);
        assert_ne!(ledger.as_ref().unwrap().outbox[0].request_id, first);
        assert_eq!(
            handle.inspect(|machine| machine.walk_retry.retry_attempt_number()),
            Some(2),
            "a dropped request is not a failed approach"
        );
        assert!(poll_thieve(&handle, &snapshot, &mut ledger, &mut budget, 11).is_pending());
        assert_eq!(walk_request_count(&ledger), 1, "the walk is reissued only once");
        let log = crate::native::take_resume_log();
        assert!(
            log.iter().any(|line| line.contains("re-issued its request once: walk request")),
            "{log:?}"
        );
    }

    /// R1, no re-issue when the goal was already reached: the item the
    /// dropped attempt would have stolen is in the pack, so the action
    /// completes instead of attempting again or timing out.
    #[test]
    fn a_dropped_pickpocket_is_not_attempted_again_when_the_goal_is_held() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let before = game_snapshot(anchor, vec![npc(19, 1, 1)], vec![]);
        let after = game_snapshot(anchor, vec![npc(19, 1, 1)], vec![held(995, 1, 0)]);
        let (mut ledger, mut budget) = (None, TickBudget::default());
        let handle = begin_thieve_at_anchor(&before, &mut ledger, &mut budget);
        budget.hold_begin();
        assert_eq!(host_drops_queued_work(&mut ledger), 1);
        budget.hold_end();

        assert!(poll_thieve(&handle, &after, &mut ledger, &mut budget, 9).is_pending());
        let done = poll_thieve(&handle, &after, &mut ledger, &mut budget, 10);
        assert!(matches!(done, Poll::Ready(Ok(1))), "{done:?}");
        assert_eq!(npc_click_count(&ledger), 0, "nothing is dispatched again");
        let log = crate::native::take_resume_log();
        assert!(
            log.iter().any(
                |line| line.contains("kept its request: the item count already meets the goal")
            ),
            "{log:?}"
        );
    }

    /// R1, Thieve, accepted then interrupted: the server took the attempt and
    /// the guardian's interaction replaced it. The core already holds the right
    /// rule for that: it neither dispatches over a pending attempt nor skips
    /// its window; it times the attempt out and then dispatches once. The hold
    /// adds nothing on top of it, so no second request appears before then.
    #[test]
    fn an_accepted_pickpocket_is_retried_once_by_its_own_window_after_a_hold() {
        let anchor = WorldTile {
            x: 3,
            z: 4,
            level: 0,
        };
        let snapshot = game_snapshot(anchor, vec![npc(19, 1, 1)], vec![]);
        let (mut ledger, mut budget) = (None, TickBudget::default());
        let handle = begin_thieve_at_anchor(&snapshot, &mut ledger, &mut budget);
        let taken = host_takes_queued_work(&mut ledger);
        host_accepts(&mut ledger, &taken[0].0, 2);
        budget.hold_begin();
        budget.hold_end();

        assert!(poll_thieve(&handle, &snapshot, &mut ledger, &mut budget, 9).is_pending());
        assert_eq!(npc_click_count(&ledger), 0);
        let mut sent = 0;
        for tick in 10..16 {
            let result = poll_thieve(&handle, &snapshot, &mut ledger, &mut budget, tick);
            assert!(result.is_pending(), "tick {tick}: {result:?}");
            sent += npc_click_count(&ledger);
            host_takes_queued_work(&mut ledger);
        }
        assert_eq!(
            sent, 1,
            "the window times the attempt out and dispatches exactly one more"
        );
    }
}
