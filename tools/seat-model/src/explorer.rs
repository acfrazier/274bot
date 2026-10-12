//! Capped, bounded exploration of the seat transition alphabet and named compositions.
//!
//! Profiles first check a fixed composition prefix, then explore every enabled
//! action from that seed for the profile's named continuation horizon. Visited
//! states are retained once with a parent link; traces are reconstructed only
//! when a monitor fails.

use super::{
    Bounds, Class, Entry, Id, Mutation, Outcome, Refusal, Rules, SessionFamily, State, Step,
    Trigger, Violation, alphabet,
};
use std::collections::{HashMap, VecDeque, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::mem::size_of;
use std::time::{Duration, Instant};

pub const DEFAULT_STATE_CAP: usize = 1_000_000;
pub const DEFAULT_FRONTIER_CAP: usize = 1_000_000;
pub const DEFAULT_BYTE_CAP: usize = 4 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CapKind {
    States,
    Frontier,
    Bytes,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ExploreLimits {
    pub state_cap: usize,
    pub frontier_cap: usize,
    pub byte_cap: usize,
}

impl Default for ExploreLimits {
    fn default() -> Self {
        Self {
            state_cap: DEFAULT_STATE_CAP,
            frontier_cap: DEFAULT_FRONTIER_CAP,
            byte_cap: DEFAULT_BYTE_CAP,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Coverage {
    /// State visits containing an adopted entry, including prefix observations.
    pub adopted_states: usize,
    /// State visits containing a parent with at least two children.
    pub multi_child_states: usize,
    /// State visits containing a credit-deferred pending start.
    pub credit_deferred_states: usize,
    /// Number of `Complete` events following a queued Session-child gameplay emission.
    pub session_child_emission_completions: usize,
    /// State visits with a live id below the retained-history floor.
    pub live_owners_below_floor_states: usize,
    /// Number of observed states containing a live mirror with a remembered ancestor.
    pub mirror_ancestry_states: usize,
    /// Oversized starts locally refused by the byte charge rule.
    pub oversized_start_refusals: usize,
    /// Releases actually dispatched by observed `Dispatch` steps.
    pub release_dispatches: usize,
    /// Session starts by family, including the explicit composition seed.
    pub session_family_starts: [usize; 4],
    /// Continuation admissions refused because a Session root is active.
    pub capability_roots_blocked_by_session: usize,
    /// Continuation dispatches attempted with a row captured by a prior Session.
    pub session_handoff_dispatches: usize,
    /// Prior Capability gameplay dispatched after Session activation.
    pub capability_gameplay_dispatched_under_session: usize,
    /// Prior Capability gameplay filtered after Session activation.
    pub capability_gameplay_filtered_under_session: usize,
    /// Actual foreground-family classification of continuation Capability dispatch.
    pub capability_dispatched_by_family: [usize; 4],
    pub capability_filtered_by_family: [usize; 4],
    /// Prior Session gameplay allowed after a Hunt handoff.
    pub session_handoff_rows_dispatched: usize,
    /// Prior Session gameplay filtered by a later foreground-owning Session.
    pub session_handoff_rows_filtered: usize,
    /// Host actions actually dispatched while a foreground Session exists.
    pub foreground_host_actions: usize,
    /// Foreground host actions dispatched while the seat is held.
    pub foreground_host_actions_while_held: usize,
    /// Host actions dispatched while the seat is stopped.
    pub host_actions_while_stopped: usize,
    /// Host input symbols observed in continuation states, indexed by 0 or 1.
    pub host_action_values: [usize; 2],
    /// Hold edges observed while a foreground Session exists.
    pub guardian_hold_edges: usize,
    /// Resumes preserving the same foreground Session in continuation states.
    pub guardian_resumes_same_foreground: usize,
    /// Continuation admissions refused for an expired parent.
    pub expired_parent_refusals: usize,
    /// Explicit continuation revocations by trigger.
    pub revocation_triggers: [usize; 9],
    /// Internal same-group replacement that published a Superseded outcome.
    pub superseded_replacements: usize,
    pub max_live: usize,
    pub max_roots: usize,
    pub max_leaves: usize,
    pub max_history_floor: Id,
}

#[derive(Clone, Copy)]
struct TransitionContext {
    foreground: Option<Id>,
    held: bool,
    stopped: bool,
    host_actions_dispatched: usize,
    handoff_row: bool,
    capability_gameplay_row: bool,
    session_family: Option<SessionFamily>,
    capability_root_pending: bool,
    expired_parent_pending: bool,
    replacement_victim: Option<Id>,
}

impl TransitionContext {
    fn capture(state: &State, step: &Step) -> Self {
        let (capability_root_pending, expired_parent_pending, replacement_victim) = match step {
            Step::Admit(id) => {
                let pending = state.pending.iter().find(|pending| pending.entry.id == *id);
                let capability_root_pending = pending.is_some_and(|pending| {
                    pending.entry.class == Class::Capability && pending.entry.parent.is_none()
                }) && state.session_root().is_some();
                let expired_parent_pending = pending.is_some_and(|pending| {
                    pending
                        .entry
                        .parent
                        .is_some_and(|parent| parent < state.history_floor())
                });
                let replacement_victim = pending
                    .filter(|pending| {
                        pending.entry.class == Class::Capability && pending.entry.parent.is_none()
                    })
                    .and_then(|pending| {
                        state
                            .entries
                            .iter()
                            .find(|entry| {
                                entry.class == Class::Capability
                                    && entry.parent.is_none()
                                    && entry.group == pending.entry.group
                            })
                            .map(|entry| entry.id)
                    });
                (
                    capability_root_pending,
                    expired_parent_pending,
                    replacement_victim,
                )
            }
            _ => (false, false, None),
        };
        let observes_foreground = matches!(
            step,
            Step::HostAction { .. } | Step::SessionHeld | Step::SessionResumed | Step::Dispatch
        );
        let foreground = if observes_foreground {
            state.foreground_session()
        } else {
            None
        };
        let held = matches!(
            step,
            Step::HostAction { .. } | Step::SessionHeld | Step::SessionResumed
        ) && state.session_held();
        let stopped = matches!(step, Step::HostAction { .. }) && state.stopped;
        let host_actions_dispatched = if matches!(step, Step::HostAction { .. }) {
            state.host_actions_dispatched()
        } else {
            0
        };
        let handoff_row = matches!(step, Step::Dispatch)
            && state.session_root().is_some_and(|session| {
                state
                    .wire
                    .iter()
                    .any(|row| row.session.is_some() && row.session != Some(session))
            });
        let capability_gameplay_row = matches!(step, Step::Dispatch)
            && state.session_root().is_some()
            && state.wire.iter().any(|row| {
                !row.release
                    && row.session.is_none()
                    && row.owner.is_some_and(|id| {
                        state
                            .entry(id)
                            .is_some_and(|entry| !entry.class.is_session())
                    })
            });
        let session_family = if matches!(step, Step::Dispatch) {
            state
                .session_root()
                .and_then(|id| match state.entry(id)?.class {
                    Class::Session(family) => Some(family),
                    Class::Capability => None,
                })
        } else {
            None
        };
        Self {
            foreground,
            held,
            stopped,
            host_actions_dispatched,
            handoff_row,
            capability_gameplay_row,
            session_family,
            capability_root_pending,
            expired_parent_pending,
            replacement_victim,
        }
    }
}

impl Coverage {
    fn observe_state(&mut self, state: &State) {
        if state
            .entries
            .iter()
            .any(|entry| entry.origin == super::Origin::Adopted)
        {
            self.adopted_states += 1;
        }
        if state.entries.iter().any(|parent| {
            state
                .entries
                .iter()
                .filter(|child| child.parent == Some(parent.id))
                .take(2)
                .count()
                >= 2
        }) {
            self.multi_child_states += 1;
        }
        if state.pending.iter().any(|pending| pending.deferred) {
            self.credit_deferred_states += 1;
        }
        if state.entries.iter().any(|entry| entry.id < state.floor) {
            self.live_owners_below_floor_states += 1;
        }
        if state.mirrors.iter().any(|mirror| mirror.ancestor.is_some()) {
            self.mirror_ancestry_states += 1;
        }
        self.max_live = self.max_live.max(state.live());
        self.max_roots = self.max_roots.max(state.roots());
        self.max_leaves = self.max_leaves.max(state.leaves());
        self.max_history_floor = self.max_history_floor.max(state.history_floor());
    }

    fn observe_transition(
        &mut self,
        step: &Step,
        before: TransitionContext,
        after: &State,
        continuation: bool,
    ) {
        if let Step::Start {
            class: Class::Session(family),
            ..
        } = step
        {
            self.session_family_starts[session_family_index(*family)] += 1;
        }
        if let Step::Start { .. } = step {
            let id = after.next_id().saturating_sub(1);
            if after.outcome(id) == Some(Outcome::Refused(Refusal::ArgsTooLarge)) {
                self.oversized_start_refusals += 1;
            }
        }
        if matches!(step, Step::Dispatch) {
            self.release_dispatches += after.release_dispatched();
            if continuation && before.handoff_row {
                self.session_handoff_dispatches += 1;
                if after.dispatched() > 0 {
                    self.session_handoff_rows_dispatched += 1;
                } else {
                    self.session_handoff_rows_filtered += 1;
                }
            }
            if continuation && before.capability_gameplay_row {
                let family = session_family_index(before.session_family.expect("active Session"));
                if after.dispatched() > 0 {
                    self.capability_gameplay_dispatched_under_session += 1;
                    self.capability_dispatched_by_family[family] += 1;
                } else {
                    self.capability_gameplay_filtered_under_session += 1;
                    self.capability_filtered_by_family[family] += 1;
                }
            }
        }
        if !continuation {
            return;
        }
        if let Step::Admit(id) = step {
            if before.capability_root_pending
                && after.outcome(*id) == Some(Outcome::Refused(Refusal::Busy))
            {
                self.capability_roots_blocked_by_session += 1;
            }
            if before.expired_parent_pending
                && after.outcome(*id) == Some(Outcome::Refused(Refusal::HistoryExpired))
            {
                self.expired_parent_refusals += 1;
            }
            if before.replacement_victim.is_some_and(|victim| {
                after.outcome(victim) == Some(Outcome::Aborted(super::Trigger::Superseded))
            }) {
                self.superseded_replacements += 1;
            }
        }
        if let Step::HostAction { action } = step {
            if continuation
                && let Some(count) = self.host_action_values.get_mut(usize::from(*action))
            {
                *count += 1;
            }
            let dispatched = after
                .host_actions_dispatched()
                .saturating_sub(before.host_actions_dispatched);
            if before.foreground.is_some() {
                self.foreground_host_actions += dispatched;
                if before.held {
                    self.foreground_host_actions_while_held += dispatched;
                }
            }
            if before.stopped {
                self.host_actions_while_stopped += dispatched;
            }
        }
        if matches!(step, Step::SessionHeld)
            && !before.held
            && after.session_held()
            && before.foreground.is_some()
        {
            self.guardian_hold_edges += 1;
        }
        if matches!(step, Step::SessionResumed)
            && before.held
            && !after.session_held()
            && before.foreground.is_some()
            && before.foreground == after.foreground_session()
        {
            self.guardian_resumes_same_foreground += 1;
        }
        if let Step::Revoke { trigger, .. } = step {
            self.revocation_triggers[trigger_index(*trigger)] += 1;
        }
    }

    /// Adds another profile's counts and combines its observed maxima.
    pub fn merge(&mut self, other: Self) {
        self.adopted_states += other.adopted_states;
        self.multi_child_states += other.multi_child_states;
        self.credit_deferred_states += other.credit_deferred_states;
        self.session_child_emission_completions += other.session_child_emission_completions;
        self.live_owners_below_floor_states += other.live_owners_below_floor_states;
        self.mirror_ancestry_states += other.mirror_ancestry_states;
        self.oversized_start_refusals += other.oversized_start_refusals;
        self.release_dispatches += other.release_dispatches;
        for (total, count) in self
            .session_family_starts
            .iter_mut()
            .zip(other.session_family_starts)
        {
            *total += count;
        }
        self.capability_roots_blocked_by_session += other.capability_roots_blocked_by_session;
        self.session_handoff_dispatches += other.session_handoff_dispatches;
        self.capability_gameplay_dispatched_under_session +=
            other.capability_gameplay_dispatched_under_session;
        self.capability_gameplay_filtered_under_session +=
            other.capability_gameplay_filtered_under_session;
        for index in 0..4 {
            self.capability_dispatched_by_family[index] +=
                other.capability_dispatched_by_family[index];
            self.capability_filtered_by_family[index] += other.capability_filtered_by_family[index];
        }
        self.session_handoff_rows_dispatched += other.session_handoff_rows_dispatched;
        self.session_handoff_rows_filtered += other.session_handoff_rows_filtered;
        self.foreground_host_actions += other.foreground_host_actions;
        self.foreground_host_actions_while_held += other.foreground_host_actions_while_held;
        self.host_actions_while_stopped += other.host_actions_while_stopped;
        for (total, count) in self
            .host_action_values
            .iter_mut()
            .zip(other.host_action_values)
        {
            *total += count;
        }
        self.guardian_hold_edges += other.guardian_hold_edges;
        self.guardian_resumes_same_foreground += other.guardian_resumes_same_foreground;
        self.expired_parent_refusals += other.expired_parent_refusals;
        for (total, count) in self
            .revocation_triggers
            .iter_mut()
            .zip(other.revocation_triggers)
        {
            *total += count;
        }
        self.superseded_replacements += other.superseded_replacements;
        self.max_live = self.max_live.max(other.max_live);
        self.max_roots = self.max_roots.max(other.max_roots);
        self.max_leaves = self.max_leaves.max(other.max_leaves);
        self.max_history_floor = self.max_history_floor.max(other.max_history_floor);
    }
}

fn session_family_index(family: SessionFamily) -> usize {
    match family {
        SessionFamily::Hunt => 0,
        SessionFamily::Gather => 1,
        SessionFamily::Combat => 2,
        SessionFamily::Progress => 3,
    }
}

fn trigger_index(trigger: super::Trigger) -> usize {
    match trigger {
        super::Trigger::Superseded => 0,
        super::Trigger::ParentAborted => 1,
        super::Trigger::HostRevoke => 2,
        super::Trigger::ManualWalk => 3,
        super::Trigger::ManualCombat => 4,
        super::Trigger::GatherSignal => 5,
        super::Trigger::Pause => 6,
        super::Trigger::Reset => 7,
        super::Trigger::Stop => 8,
    }
}

#[derive(Clone, Debug)]
pub struct Exploration {
    pub bounds: Bounds,
    /// Continuation transitions explored after the explicit composition prefix.
    pub horizon: usize,
    pub prefix_transitions: usize,
    /// Maximum retained search states, including the composition seed.
    pub state_cap: usize,
    /// Maximum queued frontier entries.
    pub frontier_cap: usize,
    /// Maximum conservative byte estimate for retained and working memory.
    pub byte_cap: usize,
    /// Distinct continuation states retained, including the seeded composition state.
    pub states: usize,
    /// Successfully applied prefix and continuation transitions.
    pub transitions: usize,
    /// Maximum pending breadth-first queue length.
    pub max_frontier: usize,
    /// Conservative retained-plus-working-memory estimate, in bytes.
    pub estimated_peak_bytes: usize,
    pub elapsed: Duration,
    /// Set for an invariant failure or cap exhaustion; a truncated run never passes.
    pub failure: Option<(Violation, Vec<Step>)>,
    pub cap_reached: Option<CapKind>,
    /// True only when the full bounded state graph completed without a failure or cap.
    pub complete: bool,
    pub coverage: Coverage,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Profile {
    V5_01SessionChild,
    V5_02AdoptedSibling,
    V5_02BranchPromotion,
    V5_03MirrorCancel,
    V5_03PendingCancel,
    V5_03CreditDeferred,
    V5_04FullLiveReplacement,
    V5_05LiveBelowFloor,
    V5_06Bytes,
    V5_06ByteRelease,
    R3CapabilityThenSession(SessionFamily),
    R3CapabilityFirstAlphabet,
    R3SessionHandoff {
        previous: SessionFamily,
        next: SessionFamily,
    },
    R3GuardianHoldResume,
    R3ExpiredParent,
    R3HostRevokeCascade,
    R3SupersededCascade,
    R3ParentAbortedCascade,
}

pub const SESSION_FAMILIES: [SessionFamily; 4] = [
    SessionFamily::Hunt,
    SessionFamily::Gather,
    SessionFamily::Combat,
    SessionFamily::Progress,
];

pub const PROFILES: [Profile; 36] = [
    Profile::V5_01SessionChild,
    Profile::V5_02AdoptedSibling,
    Profile::V5_02BranchPromotion,
    Profile::V5_03MirrorCancel,
    Profile::V5_03PendingCancel,
    Profile::V5_03CreditDeferred,
    Profile::V5_04FullLiveReplacement,
    Profile::V5_05LiveBelowFloor,
    Profile::V5_06Bytes,
    Profile::V5_06ByteRelease,
    Profile::R3CapabilityThenSession(SessionFamily::Hunt),
    Profile::R3CapabilityThenSession(SessionFamily::Gather),
    Profile::R3CapabilityThenSession(SessionFamily::Combat),
    Profile::R3CapabilityThenSession(SessionFamily::Progress),
    Profile::R3CapabilityFirstAlphabet,
    Profile::R3SessionHandoff {
        previous: SessionFamily::Hunt,
        next: SessionFamily::Hunt,
    },
    Profile::R3SessionHandoff {
        previous: SessionFamily::Hunt,
        next: SessionFamily::Gather,
    },
    Profile::R3SessionHandoff {
        previous: SessionFamily::Hunt,
        next: SessionFamily::Combat,
    },
    Profile::R3SessionHandoff {
        previous: SessionFamily::Hunt,
        next: SessionFamily::Progress,
    },
    Profile::R3SessionHandoff {
        previous: SessionFamily::Gather,
        next: SessionFamily::Hunt,
    },
    Profile::R3SessionHandoff {
        previous: SessionFamily::Gather,
        next: SessionFamily::Gather,
    },
    Profile::R3SessionHandoff {
        previous: SessionFamily::Gather,
        next: SessionFamily::Combat,
    },
    Profile::R3SessionHandoff {
        previous: SessionFamily::Gather,
        next: SessionFamily::Progress,
    },
    Profile::R3SessionHandoff {
        previous: SessionFamily::Combat,
        next: SessionFamily::Hunt,
    },
    Profile::R3SessionHandoff {
        previous: SessionFamily::Combat,
        next: SessionFamily::Gather,
    },
    Profile::R3SessionHandoff {
        previous: SessionFamily::Combat,
        next: SessionFamily::Combat,
    },
    Profile::R3SessionHandoff {
        previous: SessionFamily::Combat,
        next: SessionFamily::Progress,
    },
    Profile::R3SessionHandoff {
        previous: SessionFamily::Progress,
        next: SessionFamily::Hunt,
    },
    Profile::R3SessionHandoff {
        previous: SessionFamily::Progress,
        next: SessionFamily::Gather,
    },
    Profile::R3SessionHandoff {
        previous: SessionFamily::Progress,
        next: SessionFamily::Combat,
    },
    Profile::R3SessionHandoff {
        previous: SessionFamily::Progress,
        next: SessionFamily::Progress,
    },
    Profile::R3GuardianHoldResume,
    Profile::R3ExpiredParent,
    Profile::R3HostRevokeCascade,
    Profile::R3SupersededCascade,
    Profile::R3ParentAbortedCascade,
];

impl Profile {
    pub const fn name(self) -> &'static str {
        match self {
            Self::V5_01SessionChild => "v5-01-session-child",
            Self::V5_02AdoptedSibling => "v5-02-adopted-sibling",
            Self::V5_02BranchPromotion => "v5-02-branch-promotion",
            Self::V5_03MirrorCancel => "v5-03-mirror-ancestry",
            Self::V5_03PendingCancel => "v5-03-pending-cancel",
            Self::V5_03CreditDeferred => "v5-03-credit-deferred",
            Self::V5_04FullLiveReplacement => "v5-04-full-live-replacement",
            Self::V5_05LiveBelowFloor => "v5-05-live-below-floor",
            Self::V5_06Bytes => "v5-06-bytes",
            Self::V5_06ByteRelease => "v5-06-release-overflow",
            Self::R3CapabilityThenSession(SessionFamily::Hunt) => "r3-capability-then-hunt",
            Self::R3CapabilityThenSession(SessionFamily::Gather) => "r3-capability-then-gather",
            Self::R3CapabilityThenSession(SessionFamily::Combat) => "r3-capability-then-combat",
            Self::R3CapabilityThenSession(SessionFamily::Progress) => "r3-capability-then-progress",
            Self::R3CapabilityFirstAlphabet => "r3-capability-first-session-alphabet",
            Self::R3SessionHandoff { previous, next } => handoff_name(previous, next),
            Self::R3GuardianHoldResume => "r3-guardian-hold-resume",
            Self::R3ExpiredParent => "r3-history-expired-parent",
            Self::R3HostRevokeCascade => "r3-revocation-host-revoke",
            Self::R3SupersededCascade => "r3-revocation-superseded",
            Self::R3ParentAbortedCascade => "r3-revocation-parent-aborted",
        }
    }

    pub const fn horizon(self) -> usize {
        match self {
            Self::V5_03MirrorCancel | Self::V5_03CreditDeferred | Self::V5_05LiveBelowFloor => 4,
            Self::R3GuardianHoldResume | Self::R3CapabilityThenSession(_) => 3,
            Self::R3SessionHandoff { .. } | Self::R3ExpiredParent => 2,
            Self::R3HostRevokeCascade
            | Self::R3SupersededCascade
            | Self::R3ParentAbortedCascade => 1,
            _ => 3,
        }
    }

    #[cfg(test)]
    fn is_r3_choice_only(self) -> bool {
        matches!(
            self,
            Self::R3CapabilityThenSession(_)
                | Self::R3CapabilityFirstAlphabet
                | Self::R3SessionHandoff { .. }
                | Self::R3GuardianHoldResume
                | Self::R3ExpiredParent
                | Self::R3HostRevokeCascade
                | Self::R3SupersededCascade
                | Self::R3ParentAbortedCascade
        )
    }

    pub fn bounds(self) -> Bounds {
        match self {
            Self::V5_01SessionChild => bounded([2, 4, 3, 8, 4, 2, 8, 4], 2, 3),
            Self::V5_02AdoptedSibling => bounded([2, 4, 3, 8, 5, 2, 8, 4], 5, 3),
            Self::V5_02BranchPromotion => bounded([2, 4, 3, 8, 4, 2, 8, 4], 4, 3),
            Self::V5_03MirrorCancel => bounded([2, 4, 3, 8, 3, 4, 8, 4], 3, 4),
            Self::V5_03PendingCancel => bounded([2, 3, 3, 6, 3, 2, 8, 4], 2, 3),
            Self::V5_03CreditDeferred => bounded([2, 2, 2, 2, 4, 2, 4, 2], 3, 4),
            Self::V5_04FullLiveReplacement => bounded([2, 4, 3, 8, 5, 4, 8, 4], 5, 3),
            Self::V5_05LiveBelowFloor => bounded([2, 3, 3, 6, 3, 1, 8, 4], 3, 4),
            Self::V5_06Bytes => bounded([2, 3, 3, 6, 3, 2, 8, 4], 1, 2),
            Self::V5_06ByteRelease => {
                let mut bounds = Bounds::production();
                // Saturate the r5 gameplay cap without opening a new-id branch.
                bounds.ids = 1;
                bounds.batches = 1;
                bounds
            }
            Self::R3CapabilityThenSession(_) => bounded([2, 3, 2, 4, 3, 2, 4, 2], 3, 2),
            Self::R3CapabilityFirstAlphabet => bounded([2, 2, 1, 4, 2, 2, 2, 1], 2, 1),
            Self::R3SessionHandoff { .. } => bounded([2, 2, 1, 4, 3, 2, 4, 2], 2, 2),
            Self::R3GuardianHoldResume => bounded([1, 1, 1, 2, 1, 1, 2, 1], 1, 1),
            Self::R3ExpiredParent => bounded([2, 2, 1, 5, 3, 1, 3, 1], 4, 2),
            Self::R3HostRevokeCascade
            | Self::R3SupersededCascade
            | Self::R3ParentAbortedCascade => bounded([2, 3, 2, 5, 3, 3, 4, 2], 3, 2),
        }
    }

    /// The reviewed composition is applied and checked before any continuation is explored.
    pub fn prefix(self) -> Vec<Step> {
        match self {
            Self::V5_01SessionChild => vec![
                start(None, None, Class::Session(SessionFamily::Gather), 1, 0, 0),
                start(Some(1), None, Class::Capability, 2, 0, 0),
                Step::Batch,
                Step::Emit {
                    of: Some(2),
                    release: false,
                },
            ],
            Self::V5_02AdoptedSibling => vec![
                start(None, None, Class::Capability, 0, 0, 0),
                Step::Admit(1),
                start(None, None, Class::Capability, 3, 0, 0),
                Step::Admit(2),
                start(Some(1), None, Class::Capability, 1, 0, 0),
                Step::Admit(3),
                start(Some(3), None, Class::Capability, 2, 0, 0),
                Step::Admit(4),
                Step::Complete(3),
                start(Some(1), None, Class::Capability, 4, 0, 0),
            ],
            Self::V5_02BranchPromotion => vec![
                start(None, None, Class::Capability, 0, 0, 0),
                Step::Admit(1),
                start(Some(1), None, Class::Capability, 1, 0, 0),
                Step::Admit(2),
                start(Some(2), None, Class::Capability, 2, 0, 0),
                Step::Admit(3),
                Step::Complete(2),
                start(Some(1), None, Class::Capability, 3, 0, 0),
            ],
            Self::V5_03MirrorCancel => vec![
                start(None, None, Class::Session(SessionFamily::Gather), 1, 0, 0),
                start(Some(1), None, Class::Capability, 2, 0, 0),
                Step::Batch,
                Step::Mirror {
                    ancestor: Some(2),
                    parent: None,
                },
            ],
            Self::V5_03PendingCancel => vec![
                Step::Mirror {
                    ancestor: None,
                    parent: None,
                },
                start(None, Some(1), Class::Capability, 1, 0, 0),
                Step::Revoke {
                    of: 1,
                    trigger: super::Trigger::HostRevoke,
                },
            ],
            Self::V5_03CreditDeferred => vec![
                start(None, None, Class::Capability, 1, 0, 0),
                Step::Admit(1),
                start(None, None, Class::Capability, 2, 0, 0),
                Step::Admit(2),
                start(None, None, Class::Capability, 3, 0, 0),
                Step::Admit(3),
            ],
            Self::V5_04FullLiveReplacement => vec![
                start(None, None, Class::Capability, 0, 0, 0),
                start(None, None, Class::Capability, 8, 0, 0),
                Step::Batch,
                start(Some(1), None, Class::Capability, 1, 0, 0),
                Step::Admit(3),
                start(Some(3), None, Class::Capability, 2, 0, 0),
                Step::Admit(4),
                start(Some(3), None, Class::Capability, 2, 0, 0),
            ],
            Self::V5_05LiveBelowFloor => vec![
                start(None, None, Class::Capability, 0, 0, 0),
                Step::Admit(1),
                start(Some(1), None, Class::Capability, 1, 0, 0),
                Step::Admit(2),
                Step::Revoke {
                    of: 2,
                    trigger: super::Trigger::HostRevoke,
                },
                start(Some(1), None, Class::Capability, 2, 0, 0),
                Step::Admit(3),
                Step::Revoke {
                    of: 3,
                    trigger: super::Trigger::HostRevoke,
                },
            ],
            Self::V5_06Bytes => Vec::new(),
            Self::V5_06ByteRelease => {
                let mut prefix = Vec::with_capacity(195);
                prefix.push(start(None, None, Class::Capability, 1, 0, 0));
                prefix.push(Step::Admit(1));
                prefix.extend((0..192).map(|_| Step::Emit {
                    of: None,
                    release: false,
                }));
                prefix.push(Step::Emit {
                    of: None,
                    release: true,
                });
                prefix
            }
            Self::R3CapabilityThenSession(family) => vec![
                start(None, None, Class::Capability, 0, 0, 0),
                Step::Admit(1),
                Step::Emit {
                    of: Some(1),
                    release: false,
                },
                Step::Take,
                start(None, None, Class::Session(family), 2, 0, 0),
            ],
            Self::R3CapabilityFirstAlphabet => vec![
                start(None, None, Class::Capability, 0, 0, 0),
                Step::Admit(1),
                Step::Emit {
                    of: Some(1),
                    release: false,
                },
                Step::Take,
            ],
            Self::R3SessionHandoff { previous, next } => vec![
                start(None, None, Class::Session(previous), 2, 0, 0),
                Step::Admit(1),
                Step::Emit {
                    of: Some(1),
                    release: false,
                },
                Step::Take,
                Step::Complete(1),
                start(None, None, Class::Session(next), 2, 0, 0),
            ],
            Self::R3GuardianHoldResume => vec![
                start(None, None, Class::Session(SessionFamily::Gather), 2, 0, 0),
                Step::Admit(1),
            ],
            Self::R3ExpiredParent => vec![
                start(None, None, Class::Capability, 0, 0, 0),
                Step::Admit(1),
                Step::Complete(1),
                Step::Page,
                Step::Watermark,
                start(None, None, Class::Capability, 0, 0, 0),
                Step::Admit(2),
                Step::Revoke {
                    of: 2,
                    trigger: Trigger::HostRevoke,
                },
                start(None, None, Class::Capability, 1, 0, 0),
                Step::Admit(3),
                Step::Revoke {
                    of: 3,
                    trigger: Trigger::HostRevoke,
                },
                Step::Page,
                Step::Watermark,
            ],
            Self::R3HostRevokeCascade
            | Self::R3SupersededCascade
            | Self::R3ParentAbortedCascade => {
                let mut prefix = vec![
                    start(None, None, Class::Capability, 0, 0, 0),
                    Step::Admit(1),
                    start(Some(1), None, Class::Capability, 1, 0, 0),
                    Step::Admit(2),
                    Step::Emit {
                        of: Some(2),
                        release: false,
                    },
                    Step::Take,
                ];
                if self == Self::R3SupersededCascade {
                    prefix.push(start(None, None, Class::Capability, 0, 0, 0));
                }
                prefix
            }
        }
    }
}

const fn handoff_name(previous: SessionFamily, next: SessionFamily) -> &'static str {
    match (previous, next) {
        (SessionFamily::Hunt, SessionFamily::Hunt) => "r3-handoff-hunt-to-hunt",
        (SessionFamily::Hunt, SessionFamily::Gather) => "r3-handoff-hunt-to-gather",
        (SessionFamily::Hunt, SessionFamily::Combat) => "r3-handoff-hunt-to-combat",
        (SessionFamily::Hunt, SessionFamily::Progress) => "r3-handoff-hunt-to-progress",
        (SessionFamily::Gather, SessionFamily::Hunt) => "r3-handoff-gather-to-hunt",
        (SessionFamily::Gather, SessionFamily::Gather) => "r3-handoff-gather-to-gather",
        (SessionFamily::Gather, SessionFamily::Combat) => "r3-handoff-gather-to-combat",
        (SessionFamily::Gather, SessionFamily::Progress) => "r3-handoff-gather-to-progress",
        (SessionFamily::Combat, SessionFamily::Hunt) => "r3-handoff-combat-to-hunt",
        (SessionFamily::Combat, SessionFamily::Gather) => "r3-handoff-combat-to-gather",
        (SessionFamily::Combat, SessionFamily::Combat) => "r3-handoff-combat-to-combat",
        (SessionFamily::Combat, SessionFamily::Progress) => "r3-handoff-combat-to-progress",
        (SessionFamily::Progress, SessionFamily::Hunt) => "r3-handoff-progress-to-hunt",
        (SessionFamily::Progress, SessionFamily::Gather) => "r3-handoff-progress-to-gather",
        (SessionFamily::Progress, SessionFamily::Combat) => "r3-handoff-progress-to-combat",
        (SessionFamily::Progress, SessionFamily::Progress) => "r3-handoff-progress-to-progress",
    }
}

pub fn profiles() -> &'static [Profile] {
    &PROFILES
}

pub fn profile(name: &str) -> Option<Profile> {
    PROFILES
        .into_iter()
        .find(|candidate| candidate.name() == name)
}

fn start(
    parent: Option<Id>,
    local_parent: Option<Id>,
    class: Class,
    group: u8,
    bytes: usize,
    hooks: u8,
) -> Step {
    Step::Start {
        parent,
        local_parent,
        class,
        group,
        bytes,
        hooks,
    }
}

fn bounded(
    [
        roots,
        live,
        depth,
        credits,
        pending,
        history,
        rows,
        gameplay,
    ]: [usize; 8],
    ids: Id,
    batches: usize,
) -> Bounds {
    Bounds {
        roots,
        live,
        depth,
        credits,
        pending,
        history,
        rows,
        gameplay,
        ids,
        batches,
    }
}

/// Backwards-compatible bounded chosen-rule small search with fail-closed default caps.
pub fn explore(bounds: Bounds, steps: usize) -> Exploration {
    explore_with_limits(
        bounds,
        steps,
        Rules::Chosen,
        Mutation::None,
        ExploreLimits::default(),
    )
}

pub fn explore_with_limits(
    bounds: Bounds,
    steps: usize,
    rules: Rules,
    mutation: Mutation,
    limits: ExploreLimits,
) -> Exploration {
    run_exploration(bounds, steps, rules, mutation, Vec::new(), limits)
}

pub fn explore_profile(profile: &Profile, rules: Rules, mutation: Mutation) -> Exploration {
    explore_profile_with_limits(profile, rules, mutation, ExploreLimits::default())
}

pub fn explore_profile_with_limits(
    profile: &Profile,
    rules: Rules,
    mutation: Mutation,
    limits: ExploreLimits,
) -> Exploration {
    run_exploration(
        profile.bounds(),
        profile.horizon(),
        rules,
        mutation,
        profile.prefix(),
        limits,
    )
}

struct Node {
    state: State,
    parent: Option<usize>,
    via: Option<Step>,
    depth: usize,
}

fn run_exploration(
    bounds: Bounds,
    horizon: usize,
    rules: Rules,
    mutation: Mutation,
    prefix: Vec<Step>,
    limits: ExploreLimits,
) -> Exploration {
    let started = Instant::now();
    let prefix_transitions = prefix.len();
    let mut transitions = 0;
    let mut coverage = Coverage::default();
    let mut state = State::with_mutation(rules, bounds, mutation);
    let prefix_bytes = prefix.capacity().saturating_mul(size_of::<Step>());
    let mut estimated_peak_bytes = size_of::<State>()
        .saturating_add(state_heap_bytes(&state).saturating_mul(2))
        .saturating_add(prefix_bytes)
        .saturating_add(4096);
    coverage.observe_state(&state);
    if let Err(failure) = state.check() {
        return failed(
            bounds,
            horizon,
            prefix_transitions,
            limits,
            0,
            transitions,
            0,
            estimated_peak_bytes,
            started.elapsed(),
            coverage,
            failure,
            prefix,
            None,
        );
    }
    if estimated_peak_bytes > limits.byte_cap {
        return capped(
            bounds,
            horizon,
            prefix_transitions,
            limits,
            0,
            transitions,
            0,
            estimated_peak_bytes,
            started.elapsed(),
            coverage,
            CapKind::Bytes,
            format!(
                "initial estimate {estimated_peak_bytes} bytes exceeds byte cap {}",
                limits.byte_cap
            ),
            prefix,
        );
    }

    for (index, step) in prefix.iter().enumerate() {
        let session_child_completion = is_session_child_completion(&state, step);
        let before = TransitionContext::capture(&state, step);
        if let Err(error) = state.apply(step) {
            let failure = Violation {
                name: "profile-prefix",
                detail: format!("prefix step {index} {step:?} is disabled: {error}"),
            };
            return failed(
                bounds,
                horizon,
                prefix_transitions,
                limits,
                0,
                transitions,
                0,
                estimated_peak_bytes,
                started.elapsed(),
                coverage,
                failure,
                prefix[..=index].to_vec(),
                None,
            );
        }
        transitions += 1;
        if session_child_completion {
            coverage.session_child_emission_completions += 1;
        }
        coverage.observe_transition(step, before, &state, false);
        coverage.observe_state(&state);
        let prefix_estimate = size_of::<State>()
            .saturating_add(state_heap_bytes(&state).saturating_mul(2))
            .saturating_add(prefix_bytes)
            .saturating_add(4096);
        estimated_peak_bytes = estimated_peak_bytes.max(prefix_estimate);
        if let Err(failure) = state.check() {
            return failed(
                bounds,
                horizon,
                prefix_transitions,
                limits,
                0,
                transitions,
                0,
                estimated_peak_bytes,
                started.elapsed(),
                coverage,
                failure,
                prefix[..=index].to_vec(),
                None,
            );
        }
        if prefix_estimate > limits.byte_cap {
            return capped(
                bounds,
                horizon,
                prefix_transitions,
                limits,
                0,
                transitions,
                0,
                estimated_peak_bytes,
                started.elapsed(),
                coverage,
                CapKind::Bytes,
                format!(
                    "prefix state estimate {prefix_estimate} bytes exceeds byte cap {}",
                    limits.byte_cap
                ),
                prefix[..=index].to_vec(),
            );
        }
        // These counters are observations, not enabled-action or authority state.
        state.dispatched = 0;
        state.releases = 0;
        state.late_controls = 0;
        state.dropped = 0;
        state.host_actions = 0;
        state.last_host_action = None;
    }

    if limits.state_cap == 0 {
        return capped(
            bounds,
            horizon,
            prefix_transitions,
            limits,
            0,
            transitions,
            0,
            estimated_peak_bytes,
            started.elapsed(),
            coverage,
            CapKind::States,
            "state cap is zero; the seed state cannot be retained",
            prefix,
        );
    }

    let root_state_bytes = state_heap_bytes(&state);
    let root_cost = retained_node_cost(root_state_bytes);
    let root_estimate = root_cost.saturating_add(prefix_bytes);
    if root_estimate > limits.byte_cap {
        return capped(
            bounds,
            horizon,
            prefix_transitions,
            limits,
            0,
            transitions,
            0,
            root_estimate,
            started.elapsed(),
            coverage,
            CapKind::Bytes,
            format!(
                "seed estimate {root_estimate} bytes exceeds byte cap {}",
                limits.byte_cap
            ),
            prefix,
        );
    }

    let mut nodes = Vec::new();
    nodes.push(Node {
        state,
        parent: None,
        via: None,
        depth: 0,
    });
    let mut frontier = VecDeque::new();
    frontier.push_back(0usize);
    let mut seen = HashMap::<u64, usize>::new();
    let root_hash = fingerprint(&nodes[0].state);
    seen.insert(root_hash, 0);
    let mut collisions = HashMap::<u64, Vec<usize>>::new();
    let mut retained_estimate = root_cost;
    estimated_peak_bytes = estimated_peak_bytes.max(root_estimate);
    let mut max_frontier = 1;

    if max_frontier > limits.frontier_cap {
        return capped(
            bounds,
            horizon,
            prefix_transitions,
            limits,
            nodes.len(),
            transitions,
            max_frontier,
            estimated_peak_bytes,
            started.elapsed(),
            coverage,
            CapKind::Frontier,
            format!(
                "seed frontier 1 exceeds frontier cap {}",
                limits.frontier_cap
            ),
            prefix,
        );
    }

    while let Some(node_index) = frontier.pop_front() {
        let depth = nodes[node_index].depth;
        if depth >= horizon {
            continue;
        }
        let state = &nodes[node_index].state;
        let scratch_estimate = alphabet_upper_bound(state)
            .saturating_mul(size_of::<Step>().saturating_mul(2))
            .saturating_add(state_heap_bytes(state).saturating_mul(2))
            .saturating_add(4096);
        let working_estimate = retained_estimate
            .saturating_add(scratch_estimate)
            .saturating_add(prefix_bytes);
        estimated_peak_bytes = estimated_peak_bytes.max(working_estimate);
        if working_estimate > limits.byte_cap {
            return capped(
                bounds,
                horizon,
                prefix_transitions,
                limits,
                nodes.len(),
                transitions,
                max_frontier,
                estimated_peak_bytes,
                started.elapsed(),
                coverage,
                CapKind::Bytes,
                format!(
                    "working estimate {working_estimate} bytes exceeds byte cap {}",
                    limits.byte_cap
                ),
                prefix,
            );
        }
        let actions = alphabet(state);
        for step in actions {
            let session_child_completion =
                is_session_child_completion(&nodes[node_index].state, &step);
            let before = TransitionContext::capture(&nodes[node_index].state, &step);
            let mut next = nodes[node_index].state.clone();
            if next.apply(&step).is_err() {
                continue;
            }
            transitions += 1;
            coverage.observe_transition(&step, before, &next, true);
            if let Err(failure) = next.check() {
                coverage.observe_state(&next);
                let path = trace_for_edge(&nodes, node_index, step, &prefix);
                return failed(
                    bounds,
                    horizon,
                    prefix_transitions,
                    limits,
                    nodes.len(),
                    transitions,
                    max_frontier,
                    estimated_peak_bytes,
                    started.elapsed(),
                    coverage,
                    failure,
                    path,
                    None,
                );
            }
            if session_child_completion {
                coverage.session_child_emission_completions += 1;
            }
            next.dispatched = 0;
            next.releases = 0;
            next.late_controls = 0;
            next.dropped = 0;
            next.host_actions = 0;
            next.last_host_action = None;

            let hash = fingerprint(&next);
            if let Some(first) = seen.get(&hash).copied() {
                if nodes[first].state == next {
                    continue;
                }
                if collisions
                    .get(&hash)
                    .is_some_and(|indices| indices.iter().any(|index| nodes[*index].state == next))
                {
                    continue;
                }
            }

            let next_state_bytes = state_heap_bytes(&next);
            let next_cost = retained_node_cost(next_state_bytes);
            let projected_states = nodes.len().saturating_add(1);
            if projected_states > limits.state_cap {
                return capped(
                    bounds,
                    horizon,
                    prefix_transitions,
                    limits,
                    nodes.len(),
                    transitions,
                    max_frontier,
                    estimated_peak_bytes,
                    started.elapsed(),
                    coverage,
                    CapKind::States,
                    format!(
                        "discovered state {projected_states} exceeds state cap {}",
                        limits.state_cap
                    ),
                    prefix,
                );
            }
            let projected_frontier = frontier.len().saturating_add(1);
            if projected_frontier > limits.frontier_cap {
                return capped(
                    bounds,
                    horizon,
                    prefix_transitions,
                    limits,
                    nodes.len(),
                    transitions,
                    max_frontier.max(projected_frontier),
                    estimated_peak_bytes,
                    started.elapsed(),
                    coverage,
                    CapKind::Frontier,
                    format!(
                        "frontier {projected_frontier} exceeds frontier cap {}",
                        limits.frontier_cap
                    ),
                    prefix,
                );
            }
            let projected_retained = retained_estimate.saturating_add(next_cost);
            let projected_bytes = projected_retained
                .saturating_add(prefix_bytes)
                .saturating_add(scratch_estimate);
            estimated_peak_bytes = estimated_peak_bytes.max(projected_bytes);
            if projected_bytes > limits.byte_cap {
                return capped(
                    bounds,
                    horizon,
                    prefix_transitions,
                    limits,
                    nodes.len(),
                    transitions,
                    max_frontier,
                    estimated_peak_bytes,
                    started.elapsed(),
                    coverage,
                    CapKind::Bytes,
                    format!(
                        "retained estimate {projected_bytes} bytes exceeds byte cap {}",
                        limits.byte_cap
                    ),
                    prefix,
                );
            }

            let next_index = nodes.len();
            if let Some(first) = seen.get(&hash).copied() {
                if let Some(indices) = collisions.get_mut(&hash) {
                    indices.push(next_index);
                } else {
                    collisions.insert(hash, vec![first, next_index]);
                }
            } else {
                seen.insert(hash, next_index);
            }
            nodes.push(Node {
                state: next,
                parent: Some(node_index),
                via: Some(step),
                depth: depth + 1,
            });
            retained_estimate = projected_retained;
            frontier.push_back(next_index);
            max_frontier = max_frontier.max(frontier.len());
            coverage.observe_state(&nodes[next_index].state);
        }
    }

    Exploration {
        bounds,
        horizon,
        prefix_transitions,
        state_cap: limits.state_cap,
        frontier_cap: limits.frontier_cap,
        byte_cap: limits.byte_cap,
        states: nodes.len(),
        transitions,
        max_frontier,
        estimated_peak_bytes,
        elapsed: started.elapsed(),
        failure: None,
        cap_reached: None,
        complete: true,
        coverage,
    }
}

#[allow(clippy::too_many_arguments)]
fn failed(
    bounds: Bounds,
    horizon: usize,
    prefix_transitions: usize,
    limits: ExploreLimits,
    states: usize,
    transitions: usize,
    max_frontier: usize,
    estimated_peak_bytes: usize,
    elapsed: Duration,
    coverage: Coverage,
    failure: Violation,
    path: Vec<Step>,
    cap_reached: Option<CapKind>,
) -> Exploration {
    Exploration {
        bounds,
        horizon,
        prefix_transitions,
        state_cap: limits.state_cap,
        frontier_cap: limits.frontier_cap,
        byte_cap: limits.byte_cap,
        states,
        transitions,
        max_frontier,
        estimated_peak_bytes,
        elapsed,
        failure: Some((failure, path)),
        cap_reached,
        complete: false,
        coverage,
    }
}

#[allow(clippy::too_many_arguments)]
fn capped(
    bounds: Bounds,
    horizon: usize,
    prefix_transitions: usize,
    limits: ExploreLimits,
    states: usize,
    transitions: usize,
    max_frontier: usize,
    estimated_peak_bytes: usize,
    elapsed: Duration,
    coverage: Coverage,
    kind: CapKind,
    detail: impl Into<String>,
    _prefix: Vec<Step>,
) -> Exploration {
    failed(
        bounds,
        horizon,
        prefix_transitions,
        limits,
        states,
        transitions,
        max_frontier,
        estimated_peak_bytes,
        elapsed,
        coverage,
        Violation {
            name: "exploration-cap",
            detail: format!("{kind:?} cap reached: {}", detail.into()),
        },
        Vec::new(),
        Some(kind),
    )
}

fn is_session_child_completion(state: &State, step: &Step) -> bool {
    let Step::Complete(id) = step else {
        return false;
    };
    if !state
        .entry(*id)
        .is_some_and(|entry| !entry.class.is_session())
    {
        return false;
    }
    state
        .queue
        .iter()
        .chain(&state.wire)
        .any(|row| row.owner == Some(*id) && row.session.is_some() && !row.release)
}

fn trace_for_edge(nodes: &[Node], parent: usize, edge: Step, prefix: &[Step]) -> Vec<Step> {
    let mut suffix = Vec::new();
    let mut cursor = Some(parent);
    while let Some(index) = cursor {
        if let Some(step) = &nodes[index].via {
            suffix.push(step.clone());
        }
        cursor = nodes[index].parent;
    }
    suffix.reverse();
    let mut trace = Vec::with_capacity(prefix.len() + suffix.len() + 1);
    trace.extend_from_slice(prefix);
    trace.extend(suffix);
    trace.push(edge);
    trace
}

fn fingerprint(state: &State) -> u64 {
    let mut hasher = DefaultHasher::new();
    state.hash(&mut hasher);
    hasher.finish()
}

fn retained_node_cost(state_heap: usize) -> usize {
    size_of::<Node>()
        .saturating_add(state_heap)
        .saturating_mul(2)
        .saturating_add(128)
}

fn state_heap_bytes(state: &State) -> usize {
    let mut bytes = 0usize;
    bytes = add_vec_bytes::<Entry>(bytes, &state.entries);
    bytes = add_vec_bytes(bytes, &state.pending);
    bytes = add_vec_bytes(bytes, &state.mirrors);
    bytes = add_vec_bytes(bytes, &state.ledger);
    bytes = add_vec_bytes(bytes, &state.carry);
    bytes = add_vec_bytes(bytes, &state.history);
    bytes = add_vec_bytes(bytes, &state.queue);
    bytes = add_vec_bytes(bytes, &state.wire);
    bytes = add_vec_bytes(bytes, &state.records);
    bytes = bytes.saturating_add(
        state
            .queue
            .iter()
            .chain(&state.wire)
            .map(|row| row.ancestors.capacity().saturating_mul(size_of::<Id>()))
            .fold(0usize, usize::saturating_add),
    );
    bytes = bytes.saturating_add(
        state
            .failure
            .as_ref()
            .map_or(0, |failure| failure.detail.capacity()),
    );
    bytes.saturating_add(state.monitor.allocated_bytes())
}

fn add_vec_bytes<T>(bytes: usize, values: &Vec<T>) -> usize {
    bytes.saturating_add(values.capacity().saturating_mul(size_of::<T>()))
}

fn alphabet_upper_bound(state: &State) -> usize {
    let known_ids = usize::from(state.next.saturating_sub(1));
    let mut count = 17usize.saturating_add(known_ids.saturating_mul(21));
    if state.next <= state.bounds.ids {
        count = count
            .saturating_add(usize::from(state.next).saturating_mul(6))
            .saturating_add(2)
            .saturating_add(state.entries.len())
            .saturating_add(state.mirrors.len().saturating_mul(2));
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_runs_fail_closed() {
        let limits = ExploreLimits {
            state_cap: 1,
            frontier_cap: 1,
            byte_cap: DEFAULT_BYTE_CAP,
        };
        let result = explore_profile_with_limits(
            &Profile::V5_01SessionChild,
            Rules::Chosen,
            Mutation::None,
            limits,
        );
        assert_eq!(result.cap_reached, Some(CapKind::States));
        assert!(!result.complete);
        assert_eq!(
            result.failure.as_ref().map(|(failure, _)| failure.name),
            Some("exploration-cap")
        );
    }

    #[test]
    fn frontier_and_byte_caps_fail_closed() {
        for (limits, expected_cap) in [
            (
                ExploreLimits {
                    frontier_cap: 0,
                    ..ExploreLimits::default()
                },
                CapKind::Frontier,
            ),
            (
                ExploreLimits {
                    byte_cap: 0,
                    ..ExploreLimits::default()
                },
                CapKind::Bytes,
            ),
        ] {
            let result = explore_profile_with_limits(
                &Profile::V5_01SessionChild,
                Rules::Chosen,
                Mutation::None,
                limits,
            );
            assert_eq!(result.cap_reached, Some(expected_cap));
            assert!(!result.complete);
            assert_eq!(
                result.failure.as_ref().map(|(failure, _)| failure.name),
                Some("exploration-cap")
            );
        }
    }

    fn assert_mutation_failure(profile: Profile, mutation: Mutation, expected: &str) {
        let result = explore_profile(&profile, Rules::Chosen, mutation);
        assert_eq!(result.cap_reached, None, "{} was capped", profile.name());
        let (failure, trace) = result
            .failure
            .as_ref()
            .unwrap_or_else(|| panic!("{} mutation did not fail", profile.name()));
        assert_eq!(
            failure.name,
            expected,
            "{} mutation did not reach its independent invariant: {:?}",
            profile.name(),
            result.failure
        );
        assert!(
            trace.len() > profile.prefix().len(),
            "{} mutation failed before its continuation edge: {trace:?}",
            profile.name()
        );
    }

    #[test]
    fn named_compositions_pass_and_cover_required_shapes() {
        let mut coverage = Coverage::default();
        let mut handoff_pairs = [[false; 4]; 4];
        for profile in PROFILES {
            let result = explore_profile(&profile, Rules::Chosen, Mutation::None);
            assert!(result.complete, "{}: {:?}", profile.name(), result.failure);
            assert_eq!(result.cap_reached, None, "{} hit a cap", profile.name());
            assert!(
                result.failure.is_none(),
                "{}: {:?}",
                profile.name(),
                result.failure
            );
            assert!(result.max_frontier > 0);
            assert!(result.transitions >= result.prefix_transitions);
            assert!(
                result.states <= result.state_cap,
                "{} exceeded its state cap",
                profile.name()
            );
            assert!(
                result.max_frontier <= result.frontier_cap,
                "{} exceeded its frontier cap",
                profile.name()
            );
            assert!(
                result.estimated_peak_bytes <= result.byte_cap,
                "{} exceeded its byte estimate cap",
                profile.name()
            );
            match profile {
                Profile::R3CapabilityThenSession(family) => {
                    assert!(
                        result.coverage.session_family_starts[session_family_index(family)] > 0,
                        "{} did not start its Session family",
                        profile.name()
                    );
                    assert!(
                        result.coverage.capability_roots_blocked_by_session > 0,
                        "{} did not test Session root exclusion",
                        profile.name()
                    );
                    if family == SessionFamily::Hunt {
                        assert!(
                            result.coverage.capability_dispatched_by_family
                                [session_family_index(family)]
                                > 0,
                            "{} incorrectly filtered Capability gameplay for Hunt",
                            profile.name()
                        );
                        assert_eq!(
                            result.coverage.capability_filtered_by_family
                                [session_family_index(family)],
                            0
                        );
                    } else {
                        assert!(
                            result.coverage.capability_filtered_by_family
                                [session_family_index(family)]
                                > 0,
                            "{} did not filter old Capability gameplay",
                            profile.name()
                        );
                        assert_eq!(
                            result.coverage.capability_dispatched_by_family
                                [session_family_index(family)],
                            0
                        );
                    }
                }
                Profile::R3CapabilityFirstAlphabet => {
                    for family in SESSION_FAMILIES {
                        assert!(
                            result.coverage.session_family_starts[session_family_index(family)] > 0
                        );
                        if family == SessionFamily::Hunt {
                            assert!(
                                result.coverage.capability_dispatched_by_family
                                    [session_family_index(family)]
                                    > 0
                            );
                        } else {
                            assert!(
                                result.coverage.capability_filtered_by_family
                                    [session_family_index(family)]
                                    > 0
                            );
                        }
                    }
                    assert!(result.coverage.capability_gameplay_dispatched_under_session > 0);
                    assert!(result.coverage.capability_gameplay_filtered_under_session > 0);
                }
                Profile::R3SessionHandoff { previous, next } => {
                    handoff_pairs[session_family_index(previous)][session_family_index(next)] =
                        true;
                    assert!(
                        result.coverage.session_handoff_dispatches > 0,
                        "{} did not dispatch a prior Session row",
                        profile.name()
                    );
                    if next == SessionFamily::Hunt {
                        assert!(result.coverage.session_handoff_rows_dispatched > 0);
                        assert_eq!(result.coverage.session_handoff_rows_filtered, 0);
                    } else {
                        assert!(result.coverage.session_handoff_rows_filtered > 0);
                        assert_eq!(result.coverage.session_handoff_rows_dispatched, 0);
                    }
                }
                Profile::R3GuardianHoldResume => {
                    assert!(result.coverage.host_action_values[0] > 0);
                    assert!(result.coverage.host_action_values[1] > 0);
                    assert!(result.coverage.foreground_host_actions > 0);
                    assert!(result.coverage.foreground_host_actions_while_held > 0);
                    assert!(result.coverage.host_actions_while_stopped > 0);
                    assert!(result.coverage.guardian_hold_edges > 0);
                    assert!(result.coverage.guardian_resumes_same_foreground > 0);
                }
                Profile::R3ExpiredParent => {
                    assert!(result.coverage.expired_parent_refusals > 0);
                }
                Profile::R3HostRevokeCascade => {
                    assert!(
                        result.coverage.revocation_triggers
                            [trigger_index(super::Trigger::HostRevoke)]
                            > 0
                    );
                }
                Profile::R3SupersededCascade => {
                    assert!(result.coverage.superseded_replacements > 0);
                }
                Profile::R3ParentAbortedCascade => {
                    assert!(
                        result.coverage.revocation_triggers
                            [trigger_index(super::Trigger::ParentAborted)]
                            > 0
                    );
                }
                _ => {}
            }
            coverage.merge(result.coverage);
        }
        for previous in SESSION_FAMILIES {
            for next in SESSION_FAMILIES {
                assert!(
                    handoff_pairs[session_family_index(previous)][session_family_index(next)],
                    "missing Session handoff {previous:?} -> {next:?}"
                );
            }
        }
        assert!(
            coverage.adopted_states > 0,
            "adoption not explored: {coverage:?}"
        );
        assert!(
            coverage.multi_child_states > 0,
            "branching not explored: {coverage:?}"
        );
        assert!(
            coverage.credit_deferred_states > 0,
            "credit deferral not explored: {coverage:?}"
        );
        assert!(
            coverage.session_child_emission_completions > 0,
            "Session-child emission/completion not explored: {coverage:?}"
        );
        assert!(
            coverage.live_owners_below_floor_states > 0,
            "live owner below floor not explored: {coverage:?}"
        );
        assert!(coverage.oversized_start_refusals > 0);
        assert!(coverage.release_dispatches > 0);
        assert!(coverage.mirror_ancestry_states > 0);
        assert!(coverage.capability_roots_blocked_by_session > 0);
        assert!(coverage.session_handoff_dispatches > 0);
        assert!(coverage.capability_gameplay_dispatched_under_session > 0);
        assert!(coverage.capability_gameplay_filtered_under_session > 0);
        assert!(coverage.session_handoff_rows_dispatched > 0);
        assert!(coverage.session_handoff_rows_filtered > 0);
        assert!(coverage.foreground_host_actions > 0);
        assert!(coverage.guardian_hold_edges > 0);
        assert!(coverage.guardian_resumes_same_foreground > 0);
        assert!(coverage.expired_parent_refusals > 0);
        assert!(coverage.superseded_replacements > 0);
    }

    #[test]
    fn seeded_mutants_fail_independent_invariants_after_the_composition() {
        for (profile, mutation, expected) in [
            (
                Profile::V5_03CreditDeferred,
                Mutation::Deferral,
                "reusable-capacity",
            ),
            (
                Profile::V5_04FullLiveReplacement,
                Mutation::Replacement,
                "reusable-capacity",
            ),
            (
                Profile::V5_04FullLiveReplacement,
                Mutation::ReplacementLabel,
                "reusable-capacity",
            ),
            (
                Profile::V5_06ByteRelease,
                Mutation::ReleaseDisplacement,
                "encoder-release",
            ),
            (
                Profile::V5_06ByteRelease,
                Mutation::KeptReleaseDiscard,
                "encoder-release",
            ),
            (
                Profile::V5_01SessionChild,
                Mutation::SessionCapture,
                "accepted-emission-authority",
            ),
            (
                Profile::R3ExpiredParent,
                Mutation::ExpiredParent,
                "history-expired-parent",
            ),
            (
                Profile::R3HostRevokeCascade,
                Mutation::RevocationCascade(super::Trigger::HostRevoke),
                "revocation-cascade",
            ),
            (
                Profile::R3SupersededCascade,
                Mutation::ReplacementCascade,
                "revocation-cascade",
            ),
            (
                Profile::R3ParentAbortedCascade,
                Mutation::RevocationCascade(super::Trigger::ParentAborted),
                "revocation-cascade",
            ),
            (
                Profile::R3GuardianHoldResume,
                Mutation::HostActionDrop,
                "host-action-dispatch",
            ),
            (
                Profile::R3GuardianHoldResume,
                Mutation::HostActionReorder,
                "host-action-dispatch",
            ),
            (
                Profile::R3GuardianHoldResume,
                Mutation::GuardianHold,
                "guardian-hold",
            ),
        ] {
            assert_mutation_failure(profile, mutation, expected);
        }
    }

    #[test]
    fn rejected_r5_compositions_fail_under_the_same_monitors() {
        for profile in PROFILES {
            if profile.is_r3_choice_only() || profile == Profile::V5_02BranchPromotion {
                continue;
            }
            let result = explore_profile(&profile, Rules::R5, Mutation::None);
            assert!(
                result.failure.is_some(),
                "{} did not discriminate r5",
                profile.name()
            );
            assert_eq!(result.cap_reached, None, "{} was capped", profile.name());
            assert_ne!(
                result.failure.as_ref().map(|(failure, _)| failure.name),
                Some("profile-prefix")
            );
        }
    }

    #[test]
    fn r5_branch_promotion_companion_fails_cancel_before_start() {
        let profile = Profile::V5_02BranchPromotion;
        let result = explore_profile(&profile, Rules::R5, Mutation::None);
        assert_eq!(result.cap_reached, None);
        let (failure, trace) = result.failure.as_ref().expect("r5 companion must fail");
        assert_eq!(failure.name, "cancel-before-start");
        assert!(trace.len() > profile.prefix().len());
    }

    #[test]
    fn profile_names_resolve_and_horizons_are_bounded() {
        for profile in PROFILES {
            assert_eq!(self::profile(profile.name()), Some(profile));
            assert!(matches!(profile.horizon(), 1..=4));
            assert!(profile.bounds().ids >= 1);
        }
        assert_eq!(self::profile("missing"), None);
        assert_eq!(profiles().len(), 36);
        assert!(Profile::V5_02AdoptedSibling.bounds().ids >= 5);
    }
    #[test]
    fn session_child_counter_excludes_the_session_root() {
        let mut state = State::new(Rules::Chosen, Bounds::small());
        state
            .apply(&start(
                None,
                None,
                Class::Session(SessionFamily::Gather),
                2,
                0,
                0,
            ))
            .unwrap();
        state.apply(&Step::Admit(1)).unwrap();
        state
            .apply(&Step::Emit {
                of: Some(1),
                release: false,
            })
            .unwrap();
        assert!(!is_session_child_completion(&state, &Step::Complete(1)));
        state
            .apply(&start(Some(1), None, Class::Capability, 0, 0, 0))
            .unwrap();
        state.apply(&Step::Admit(2)).unwrap();
        state
            .apply(&Step::Emit {
                of: Some(2),
                release: false,
            })
            .unwrap();
        assert!(is_session_child_completion(&state, &Step::Complete(2)));
    }
}
