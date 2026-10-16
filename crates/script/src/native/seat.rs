//! Slot-owned request forest and captured gameplay authority (§28).
//! The slot calls this on its own tick; no worker, isolate, or alternate action
//! scheduler lives here. Rows keep their push-time lease until host dispatch.
use api::identity::{RequestId, RequestNamespace};
use api::script_contract::{
    dispatch_disposition, AbortReason, AdmissionWitness, AuthorityBoundary, AuthorityStatus,
    CallbackCancelReason, CapturedAuthority, ChildOrigin, DecisionSequence, DispatchDisposition,
    EmissionClass, EmissionProof, Refusal, RequestClass, RequestStamp, HOOK_CHARGE,
    MAX_ENCODED_GAMEPLAY, MAX_INTERACT_REQS, MAX_LINEAGE_DEPTH, MAX_LIVE, MAX_LIVE_LEAVES,
    MAX_RESULTS, MAX_ROOTS, MAX_WALK_RELEASES, REVOKED_RING,
};
use api::selected::RunKey;
use std::collections::VecDeque;

use super::request::NativeKind;
use super::services::PromptPoint;
use super::NativePhase;

/// A terminal's class. Family-specific values and JS thrown objects remain
/// with their real machine or isolate; the seat never copies or synthesizes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatTerminal {
    Done,
    Failed,
    Refused(Refusal),
    Aborted(AbortReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeatResult {
    pub id: RequestId,
    pub seq: u64,
    pub outcome: SeatTerminal,
    pub prior_parent: Option<RequestId>,
    /// Session captured while this entry was live. A row already emitted in
    /// the isolate can arrive after this terminal without becoming root.
    pub session: Option<RequestId>,
    pub authority_revoked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeatEntry {
    pub id: RequestId,
    pub parent: Option<RequestId>,
    pub kind: NativeKind,
    pub origin: ChildOrigin,
    pub depth: u8,
    pub decisions_consumed: u32,
    pub prompt: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Admitted,
    Refused(Refusal),
    Cancelled,
    Deferred,
    Duplicate,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatError {
    SequenceExhausted,
    Capacity,
    InvalidAuthority,
    ProtocolDefect,
    Held,
    AwaitingDecision,
    PromptTooLarge,
}

/// One current, level-triggered native callback prompt. The native family
/// owns the decision point and bounded structured-clone arguments; the seat
/// retains them until a matching Decide or terminal.
#[derive(Debug)]
pub struct SeatPrompt {
    pub id: RequestId,
    pub seq: DecisionSequence,
    pub point: PromptPoint,
    pub args: Vec<u8>,
}

/// A consumed answer waiting for the native family to resume its request.
#[derive(Debug, PartialEq, Eq)]
pub struct SeatAnswer {
    pub id: RequestId,
    pub seq: DecisionSequence,
    pub answer: Vec<u8>,
}

/// A carried decision either answered the current prompt, was a late copy,
/// or contradicted the live entry's prompt sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatDecisionStatus {
    Consumed,
    Late,
    Invalid,
    Held,
}

/// The producer's proven push-time identity. An external nonzero id with no
/// registered producer is not an ordinary completed request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmissionSource {
    Root,
    Owner(RequestId),
    Mirror {
        owner: RequestId,
        parent: Option<RequestId>,
    },
    Pending {
        owner: RequestId,
        parent: Option<RequestId>,
        kind: NativeKind,
    },
    PendingMirror {
        owner: RequestId,
        pending: RequestId,
        parent: Option<RequestId>,
        kind: NativeKind,
    },
    InvalidExternal(RequestId),
}

/// An action's immutable push-time authority, kept with its payload.
/// `None` is the ordinary root action, not a forged nonzero owner.
#[derive(Debug)]
struct SeatRow<R> {
    payload: R,
    class: EmissionClass,
    authority: Option<CapturedAuthority>,
    witness: AdmissionWitness,
    pending_owner: Option<RequestId>,
    proof: EmissionProof,
    boundary: AuthorityBoundary,
    release: Option<api::script_contract::Release>,
    reserved_policy: bool,
    walk: Option<RequestId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WalkReleaseState {
    Reserved,
    Armed,
    Queued,
}

#[derive(Clone, Copy)]
struct WalkReleaseSlot {
    id: RequestId,
    state: WalkReleaseState,
}

/// One bounded forest, terminal ledger, history and row queue. The row type is
/// the caller's owned payload; native-only builds instantiate it without V8.
/// Capacity is reserved once, on the first real script/seat activation.
pub struct GameplaySeat<R> {
    run: RunKey,
    live: Vec<SeatEntry>,
    prompts: Vec<SeatPrompt>,
    answers: Vec<SeatAnswer>,
    machine_phases: [Option<(RequestId, NativePhase)>; MAX_LIVE],
    results: Vec<SeatResult>,
    revoked: Vec<RequestId>,
    isolate_floor: u64,
    host_floor: u64,
    max_isolate: u64,
    max_host: u64,
    mirror_revoked: Vec<RequestId>,
    mirror_floor: u64,
    max_mirror: u64,
    mirror_revokes_consumed: u64,
    next_seq: u64,
    consumed_seq: u64,
    queued: VecDeque<SeatRow<R>>,
    in_flight: Vec<SeatRow<R>>,
    walk_releases: [Option<WalkReleaseSlot>; MAX_WALK_RELEASES],
    gather_release: bool,
    combat_release: bool,
    held: bool,
    stopped: bool,
    dropped: u64,
    late_controls: u64,
}

impl<R> GameplaySeat<R> {
    pub fn new(run: RunKey) -> Self {
        Self {
            run,
            live: Vec::with_capacity(MAX_LIVE),
            prompts: Vec::with_capacity(MAX_LIVE),
            answers: Vec::with_capacity(MAX_LIVE),
            machine_phases: [None; MAX_LIVE],
            results: Vec::with_capacity(MAX_RESULTS),
            revoked: Vec::with_capacity(REVOKED_RING),
            isolate_floor: RequestId::ISOLATE_FIRST,
            host_floor: RequestId::HOST_FIRST,
            max_isolate: 0,
            max_host: RequestId::HOST_FIRST - 1,
            mirror_revoked: Vec::with_capacity(MAX_ENCODED_GAMEPLAY),
            mirror_floor: RequestId::ISOLATE_FIRST,
            max_mirror: 0,
            mirror_revokes_consumed: 0,
            next_seq: 0,
            consumed_seq: 0,
            queued: VecDeque::with_capacity(MAX_INTERACT_REQS + MAX_WALK_RELEASES + 3),
            in_flight: Vec::with_capacity(MAX_ENCODED_GAMEPLAY + MAX_WALK_RELEASES + 3),
            walk_releases: [None; MAX_WALK_RELEASES],
            gather_release: false,
            combat_release: false,
            held: false,
            stopped: false,
            dropped: 0,
            late_controls: 0,
        }
    }

    pub fn run(&self) -> RunKey {
        self.run
    }

    pub fn live(&self) -> &[SeatEntry] {
        &self.live
    }

    pub fn results(&self) -> &[SeatResult] {
        &self.results
    }
    /// Adapter-only phase of a live in-isolate machine. Stored in a fixed
    /// table so a host page never guesses Waiting from stale JS state.
    pub fn machine_phase(&self, id: RequestId) -> Option<NativePhase> {
        self.machine_phases
            .iter()
            .flatten()
            .find(|(owner, _)| *owner == id)
            .map(|(_, phase)| *phase)
    }

    pub fn set_machine_phase(
        &mut self,
        id: RequestId,
        phase: NativePhase,
    ) -> Result<(), SeatError> {
        if self.entry(id).is_none() {
            return Ok(());
        }
        if let Some(slot) = self
            .machine_phases
            .iter_mut()
            .find(|slot| slot.is_some_and(|(owner, _)| owner == id))
        {
            *slot = Some((id, phase));
            return Ok(());
        }
        let slot = self
            .machine_phases
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(SeatError::Capacity)?;
        *slot = Some((id, phase));
        Ok(())
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn count_dropped(&mut self, count: usize) {
        self.dropped += count as u64;
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn reserved_releases(&self) -> usize {
        self.walk_releases.iter().flatten().count()
            + self.gather_release as usize
            + self.combat_release as usize
    }

    pub fn held(&self) -> bool {
        self.held
    }

    pub fn has_rows(&self) -> bool {
        !self.queued.is_empty() || !self.in_flight.is_empty()
    }

    /// Walks still in seat custody, before a kept reset drops their gameplay.
    /// A host that had not dispatched them must tell the owning wait to make
    /// a new request after relog, rather than let it time out.
    pub fn undelivered_walk_ids(&self) -> impl Iterator<Item = RequestId> + '_ {
        self.queued
            .iter()
            .chain(&self.in_flight)
            .filter(|row| row.class == EmissionClass::Gameplay)
            .filter_map(|row| row.walk)
    }

    pub fn late_controls(&self) -> u64 {
        self.late_controls
    }

    /// Reserve encoder custody before the corresponding physical walk drive
    /// is offered to the host. A full bank of undelivered releases paces the
    /// next drive rather than evicting an earlier release.
    pub fn reserve_walk(&mut self, id: RequestId) -> bool {
        if self.held || self.stopped {
            return false;
        }
        if self
            .walk_releases
            .iter()
            .flatten()
            .any(|slot| slot.id == id)
        {
            return true;
        }
        let Some(slot) = self.walk_releases.iter_mut().find(|slot| slot.is_none()) else {
            return false;
        };
        *slot = Some(WalkReleaseSlot {
            id,
            state: WalkReleaseState::Reserved,
        });
        true
    }

    /// A filtered row never armed its drive. Keep an already-queued release,
    /// but free an otherwise unused physical reservation for a later walk.
    pub(crate) fn discard_unarmed_walk(&mut self, id: RequestId) {
        if let Some(slot) = self.walk_releases.iter_mut().find(|slot| {
            slot.is_some_and(|reserved| {
                reserved.id == id && reserved.state == WalkReleaseState::Reserved
            })
        }) {
            *slot = None;
        }
    }

    pub fn can_reserve_walk(&self, id: RequestId) -> bool {
        self.walk_releases.iter().any(|slot| match slot {
            None => true,
            Some(armed) => armed.id == id,
        })
    }

    /// Retire an armed physical walk after its matching host terminal. A
    /// queued AbortWalk keeps custody until that release is delivered.
    pub fn walk_terminal(&mut self, id: RequestId) {
        if let Some(slot) = self.walk_releases.iter_mut().find(|slot| {
            slot.is_some_and(|reserved| {
                reserved.id == id && reserved.state == WalkReleaseState::Armed
            })
        }) {
            *slot = None;
        }
    }

    /// A native action may settle by its own arrival observation before the
    /// nav follower publishes a terminal. Once its owner no longer grants
    /// that walk, no release is owed. A queued AbortWalk remains untouched.
    pub(crate) fn retire_inactive_native_walks(&mut self, active: Option<RequestId>) {
        for slot in &mut self.walk_releases {
            if slot.is_some_and(|reserved| {
                reserved.state == WalkReleaseState::Armed
                    && reserved.id.get() > RequestId::HOST_FIRST
                    && reserved.id.get() < (1 << 63)
                    && Some(reserved.id) != active
            }) {
                *slot = None;
            }
        }
    }

    pub fn foreground(&self) -> Option<RequestId> {
        self.live
            .iter()
            .find_map(|entry| match entry.kind.policy().class() {
                RequestClass::Session(family) if family.foreground() == Some(true) => {
                    Some(entry.id)
                }
                _ => None,
            })
    }

    pub fn session(&self) -> Option<RequestId> {
        self.live.iter().find_map(|entry| {
            matches!(entry.kind.policy().class(), RequestClass::Session(_)).then_some(entry.id)
        })
    }

    pub fn lookup(&self, id: RequestId) -> AuthorityStatus {
        if self.live.iter().any(|entry| entry.id == id) {
            return AuthorityStatus::Live;
        }
        if let Some(result) = self.results.iter().find(|result| result.id == id) {
            return if result.authority_revoked {
                AuthorityStatus::Revoked
            } else {
                AuthorityStatus::Completed
            };
        }
        if self.revoked.contains(&id) {
            return AuthorityStatus::Revoked;
        }
        let floor = match id.namespace() {
            RequestNamespace::Isolate => self.isolate_floor,
            RequestNamespace::Host => self.host_floor,
        };
        if id.get() < floor {
            AuthorityStatus::Expired
        } else if self.seen_id(id) {
            // Retired ordinary completion in this generation.
            AuthorityStatus::Completed
        } else {
            AuthorityStatus::Pending
        }
    }

    fn seen_id(&self, id: RequestId) -> bool {
        match id.namespace() {
            RequestNamespace::Isolate => id.get() <= self.max_isolate,
            RequestNamespace::Host => id.get() <= self.max_host,
        }
    }

    fn note_id(&mut self, id: RequestId) {
        let max = match id.namespace() {
            RequestNamespace::Isolate => &mut self.max_isolate,
            RequestNamespace::Host => &mut self.max_host,
        };
        *max = (*max).max(id.get());
    }

    pub fn watermark(&mut self, consumed: u64) {
        if self.held || consumed < self.consumed_seq || consumed > self.next_seq {
            return;
        }
        self.consumed_seq = consumed;
        self.results.retain(|result| result.seq > consumed);
    }

    fn entry(&self, id: RequestId) -> Option<SeatEntry> {
        self.live.iter().copied().find(|entry| entry.id == id)
    }

    fn chain(&self, id: RequestId) -> ([Option<RequestId>; MAX_LINEAGE_DEPTH], usize) {
        let mut chain = [None; MAX_LINEAGE_DEPTH];
        let mut next = Some(id);
        let mut len = 0;
        while let Some(current) = next {
            if len == chain.len() {
                break;
            }
            chain[len] = Some(current);
            len += 1;
            next = self
                .entry(current)
                .and_then(|entry| entry.parent)
                .or_else(|| {
                    self.results
                        .iter()
                        .find(|result| result.id == current && !result.authority_revoked)
                        .and_then(|result| result.prior_parent)
                });
        }
        (chain, len)
    }

    fn captured_session(&self, id: RequestId) -> Option<RequestId> {
        let (chain, len) = self.chain(id);
        for member in chain[..len].iter().flatten().copied() {
            if self.entry(member).is_some_and(|entry| {
                matches!(entry.kind.policy().class(), RequestClass::Session(_))
            }) {
                return Some(member);
            }
            if let Some(session) = self
                .results
                .iter()
                .find(|result| result.id == member)
                .and_then(|result| result.session)
            {
                return Some(session);
            }
        }
        None
    }

    fn has_descendant(&self, id: RequestId, ancestor: RequestId) -> bool {
        let (chain, _) = self.chain(id);
        chain.into_iter().flatten().any(|member| member == ancestor)
    }

    fn remember_revocation(&mut self, id: RequestId) {
        let floor = match id.namespace() {
            RequestNamespace::Isolate => self.isolate_floor,
            RequestNamespace::Host => self.host_floor,
        };
        if id.get() < floor || self.revoked.contains(&id) {
            return;
        }
        if self.revoked.len() == REVOKED_RING {
            let (at, oldest) = self
                .revoked
                .iter()
                .enumerate()
                .min_by_key(|(_, item)| item.get())
                .map(|(at, id)| (at, *id))
                .expect("full history has an oldest id");
            self.revoked.remove(at);
            let next = oldest.get().saturating_add(1);
            match oldest.namespace() {
                RequestNamespace::Isolate => self.isolate_floor = next,
                RequestNamespace::Host => self.host_floor = next,
            }
        }
        self.revoked.push(id);
    }

    fn remember_mirror_revocation(&mut self, id: RequestId) {
        self.max_mirror = self.max_mirror.max(id.get());
        if id.get() < self.mirror_floor || self.mirror_revoked.contains(&id) {
            return;
        }
        if self.mirror_revoked.len() == MAX_ENCODED_GAMEPLAY {
            let (at, oldest) = self
                .mirror_revoked
                .iter()
                .enumerate()
                .min_by_key(|(_, value)| value.get())
                .map(|(at, id)| (at, *id))
                .expect("full mirror history");
            self.mirror_revoked.remove(at);
            self.mirror_floor = oldest.get().saturating_add(1);
        }
        self.mirror_revoked.push(id);
    }

    fn publish(
        &mut self,
        id: RequestId,
        outcome: SeatTerminal,
        prior_parent: Option<RequestId>,
        authority_revoked: bool,
    ) -> Result<(), SeatError> {
        if self.live.len() + self.results.len() >= MAX_RESULTS {
            return Err(SeatError::Capacity);
        }
        self.next_seq = self
            .next_seq
            .checked_add(1)
            .ok_or(SeatError::SequenceExhausted)?;
        if authority_revoked {
            self.remember_revocation(id);
        }
        self.results.push(SeatResult {
            id,
            seq: self.next_seq,
            outcome,
            prior_parent,
            session: None,
            authority_revoked,
        });
        self.note_id(id);
        Ok(())
    }

    fn publish_live(
        &mut self,
        id: RequestId,
        outcome: SeatTerminal,
        prior_parent: Option<RequestId>,
        session: Option<RequestId>,
        authority_revoked: bool,
    ) -> Result<(), SeatError> {
        // A live entry already owns the credit that its terminal converts.
        if self.results.len() >= MAX_RESULTS {
            return Err(SeatError::Capacity);
        }
        self.next_seq = self
            .next_seq
            .checked_add(1)
            .ok_or(SeatError::SequenceExhausted)?;
        if authority_revoked {
            self.remember_revocation(id);
        }
        self.results.push(SeatResult {
            id,
            seq: self.next_seq,
            outcome,
            prior_parent,
            session,
            authority_revoked,
        });
        self.note_id(id);
        Ok(())
    }

    fn revoke_leases(&mut self, id: RequestId) {
        for row in self.queued.iter_mut().chain(&mut self.in_flight) {
            if row.authority.as_ref().is_some_and(|lease| {
                lease.owner() == id || lease.ancestors().any(|ancestor| ancestor == id)
            }) {
                row.boundary = AuthorityBoundary::Revoked;
            }
        }
    }

    /// A mirror has no seat credit or terminal. Its local failure revokes
    /// only captured rows, and the adapter cascades this control to every
    /// descendant mirror owner it has already put on the wire.
    /// `false` means the control remains carried through a guardian hold.
    pub fn revoke_mirror_owner(&mut self, id: RequestId) -> bool {
        if self.held || self.stopped {
            return false;
        }
        self.remember_mirror_revocation(id);
        for row in self.queued.iter_mut().chain(&mut self.in_flight) {
            if row
                .authority
                .as_ref()
                .is_some_and(|lease| lease.owner() == id)
            {
                row.boundary = AuthorityBoundary::Revoked;
            }
        }
        true
    }

    pub fn mirror_revokes_consumed(&self) -> u64 {
        self.mirror_revokes_consumed
    }

    /// Reject a gap before touching any Watermark, control, start or action
    /// from the incoming batch. Retries at or below the acknowledged level
    /// remain idempotent and count as late when consumed.
    pub fn validate_mirror_revokes(
        &self,
        revokes: impl IntoIterator<Item = (RequestId, u64)>,
    ) -> Result<(), SeatError> {
        let mut contiguous = self.mirror_revokes_consumed;
        let mut seen = [None; MAX_ENCODED_GAMEPLAY];
        for (count, (id, seq)) in revokes.into_iter().enumerate() {
            if id.namespace() != RequestNamespace::Isolate
                || seq == 0
                || count == MAX_ENCODED_GAMEPLAY
                || seen[..count].contains(&Some(id))
            {
                return Err(SeatError::ProtocolDefect);
            }
            seen[count] = Some(id);
            if seq > contiguous {
                contiguous = contiguous
                    .checked_add(1)
                    .ok_or(SeatError::SequenceExhausted)?;
                if seq != contiguous {
                    return Err(SeatError::ProtocolDefect);
                }
            }
        }
        Ok(())
    }

    /// Apply one decoded mirror revocation in the Controls stage. A held
    /// session defers the entire carried unit without advancing the level.
    pub fn apply_mirror_revoke(&mut self, id: RequestId, seq: u64) -> Result<bool, SeatError> {
        if seq == 0 || id.namespace() != RequestNamespace::Isolate {
            return Err(SeatError::ProtocolDefect);
        }
        if self.held || self.stopped {
            return Ok(false);
        }
        if seq <= self.mirror_revokes_consumed {
            self.late_controls += 1;
            return Ok(true);
        }
        let expected = self
            .mirror_revokes_consumed
            .checked_add(1)
            .ok_or(SeatError::SequenceExhausted)?;
        if seq != expected {
            return Err(SeatError::ProtocolDefect);
        }
        self.revoke_mirror_owner(id);
        self.mirror_revokes_consumed = seq;
        Ok(true)
    }

    fn witness(&mut self, id: RequestId) {
        for row in self.queued.iter_mut().chain(&mut self.in_flight) {
            if row.pending_owner == Some(id)
                || row
                    .authority
                    .as_ref()
                    .is_some_and(|lease| lease.owner() == id)
            {
                row.witness = AdmissionWitness::Admitted;
            }
        }
    }

    fn complete_entry(
        &mut self,
        id: RequestId,
        outcome: SeatTerminal,
        revoke: bool,
    ) -> Result<(), SeatError> {
        let Some(at) = self.live.iter().position(|entry| entry.id == id) else {
            return Ok(());
        };
        let parent = self.live[at].parent;
        let kind = self.live[at].kind;
        let session = self.captured_session(id);
        if revoke {
            self.revoke_leases(id);
        }
        let descendant_depths: [bool; MAX_LIVE] = std::array::from_fn(|index| {
            self.live
                .get(index)
                .is_some_and(|entry| entry.id != id && self.has_descendant(entry.id, id))
        });
        self.live.remove(at);
        self.clear_request_state(id);
        for (index, child) in self.live.iter_mut().enumerate() {
            let before = index + usize::from(index >= at);
            if descendant_depths[before] {
                child.depth -= 1;
            }
            if child.parent == Some(id) {
                child.parent = parent;
                if parent.is_some() {
                    child.origin = ChildOrigin::Adopted;
                }
            }
        }
        self.publish_live(id, outcome, parent, session, revoke)?;
        self.retire_walk_terminal(id, kind);
        Ok(())
    }

    fn subtree_leaf_first(
        &self,
        id: RequestId,
        result: &mut [Option<RequestId>; MAX_LIVE],
        count: &mut usize,
    ) {
        for child in self.live.iter().filter(|entry| entry.parent == Some(id)) {
            self.subtree_leaf_first(child.id, result, count);
            result[*count] = Some(child.id);
            *count += 1;
        }
    }

    pub fn revoke(&mut self, id: RequestId, reason: AbortReason) -> Result<(), SeatError> {
        if self.held || self.entry(id).is_none() {
            return Ok(());
        }
        let mut ids = [None; MAX_LIVE];
        let mut count = 0;
        self.subtree_leaf_first(id, &mut ids, &mut count);
        ids[count] = Some(id);
        for child in ids.into_iter().flatten() {
            let at = self
                .live
                .iter()
                .position(|entry| entry.id == child)
                .expect("descendant still live");
            let session = self.captured_session(child);
            let kind = self.live[at].kind;
            let parent = self.live.remove(at).parent;
            self.clear_request_state(child);
            self.revoke_leases(child);
            self.publish_live(
                child,
                SeatTerminal::Aborted(if child == id {
                    reason
                } else {
                    AbortReason::ParentAborted
                }),
                parent,
                session,
                true,
            )?;
            self.retire_walk_terminal(child, kind);
        }
        Ok(())
    }

    pub fn complete(&mut self, id: RequestId, failed: bool) -> Result<(), SeatError> {
        if self.held {
            return Ok(());
        }
        self.complete_entry(
            id,
            if failed {
                SeatTerminal::Failed
            } else {
                SeatTerminal::Done
            },
            false,
        )
    }

    pub fn manual_walk(&mut self, id: RequestId) -> Result<(), SeatError> {
        if self.held || self.entry(id).is_none() {
            return Ok(());
        }
        let mut children = [None; MAX_LIVE];
        let mut count = 0;
        self.subtree_leaf_first(id, &mut children, &mut count);
        for child in children.into_iter().flatten() {
            let at = self
                .live
                .iter()
                .position(|entry| entry.id == child)
                .expect("walk descendant live");
            let kind = self.live[at].kind;
            let session = self.captured_session(child);
            let parent = self.live.remove(at).parent;
            self.clear_request_state(child);
            self.revoke_leases(child);
            self.publish_live(
                child,
                SeatTerminal::Aborted(AbortReason::ParentAborted),
                parent,
                session,
                true,
            )?;
            self.retire_walk_terminal(child, kind);
        }
        self.complete_entry(id, SeatTerminal::Done, true)
    }

    pub fn cancel(&mut self, id: RequestId, reason: CallbackCancelReason) -> Result<(), SeatError> {
        if self.held {
            return Ok(());
        }
        if self.entry(id).is_none() {
            self.late_controls += 1;
            return Ok(());
        }
        match reason {
            CallbackCancelReason::HookThrew => self.complete_entry(id, SeatTerminal::Failed, false),
            CallbackCancelReason::ParentAborted => self.revoke(id, AbortReason::ParentAborted),
        }
    }

    pub fn decide_with_answer(
        &mut self,
        id: RequestId,
        seq: DecisionSequence,
        answer: Vec<u8>,
    ) -> SeatDecisionStatus {
        if self.held {
            return SeatDecisionStatus::Held;
        }
        let Some(entry) = self.live.iter_mut().find(|entry| entry.id == id) else {
            self.late_controls += 1;
            return SeatDecisionStatus::Late;
        };
        let raw = seq.get();
        if raw <= entry.decisions_consumed {
            self.late_controls += 1;
            return SeatDecisionStatus::Late;
        }
        if entry.prompt != Some(raw)
            || entry.decisions_consumed.checked_add(1) != Some(raw)
            || answer.len() > api::script_contract::MAX_START_CHARGE
        {
            return SeatDecisionStatus::Invalid;
        }
        entry.prompt = None;
        entry.decisions_consumed = raw;
        self.prompts.retain(|prompt| prompt.id != id);
        self.answers.push(SeatAnswer { id, seq, answer });
        SeatDecisionStatus::Consumed
    }

    /// Direct model-style producer for seat unit tests. Product prompts
    /// require family-owned point/arguments through `prompt_with_details`.
    #[cfg(test)]
    pub fn decide(&mut self, id: RequestId, seq: u32) {
        let _ = self.decide_with_answer(
            id,
            DecisionSequence::from_raw(seq).expect("test decision sequence"),
            Vec::new(),
        );
    }

    /// Mint a level-triggered prompt from a real native family, retaining its
    /// bounded binary arguments until the matching decision or terminal.
    pub fn prompt_with_details(
        &mut self,
        id: RequestId,
        point: PromptPoint,
        args: &[u8],
    ) -> Result<DecisionSequence, SeatError> {
        if self.held {
            return Err(SeatError::Held);
        }
        if self.stopped {
            return Err(SeatError::InvalidAuthority);
        }
        if args.len() > HOOK_CHARGE {
            return Err(SeatError::PromptTooLarge);
        }
        if self.answers.iter().any(|answer| answer.id == id) {
            return Err(SeatError::AwaitingDecision);
        }
        let entry = self
            .live
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or(SeatError::InvalidAuthority)?;
        if entry.prompt.is_some() {
            return Err(SeatError::AwaitingDecision);
        }
        let raw = entry
            .decisions_consumed
            .checked_add(1)
            .ok_or(SeatError::SequenceExhausted)?;
        let seq = DecisionSequence::from_raw(raw).ok_or(SeatError::SequenceExhausted)?;
        entry.prompt = Some(raw);
        self.prompts.push(SeatPrompt {
            id,
            seq,
            point,
            args: args.to_vec(),
        });
        Ok(seq)
    }

    pub fn prompt_payload(&self, id: RequestId) -> Option<&SeatPrompt> {
        self.prompts.iter().find(|prompt| prompt.id == id)
    }

    pub fn take_answer(&mut self, id: RequestId) -> Option<SeatAnswer> {
        let at = self.answers.iter().position(|answer| answer.id == id)?;
        Some(self.answers.remove(at))
    }

    #[cfg(test)]
    pub fn prompt(&mut self, id: RequestId) -> Result<Option<u32>, SeatError> {
        if self.held {
            return Err(SeatError::Held);
        }
        let Some(entry) = self.live.iter_mut().find(|entry| entry.id == id) else {
            return Ok(None);
        };
        if entry.prompt.is_none() {
            entry.prompt = Some(
                entry
                    .decisions_consumed
                    .checked_add(1)
                    .ok_or(SeatError::SequenceExhausted)?,
            );
        }
        Ok(entry.prompt)
    }

    fn clear_request_state(&mut self, id: RequestId) {
        self.prompts.retain(|prompt| prompt.id != id);
        self.answers.retain(|answer| answer.id != id);
        if let Some(slot) = self
            .machine_phases
            .iter_mut()
            .find(|slot| slot.is_some_and(|(owner, _)| owner == id))
        {
            *slot = None;
        }
    }

    fn retire_walk_terminal(&mut self, id: RequestId, kind: NativeKind) {
        if kind != NativeKind::Walk {
            return;
        }
        if let Some(slot) = self.walk_releases.iter_mut().find(|slot| {
            slot.is_some_and(|reserved| {
                reserved.id == id && reserved.state != WalkReleaseState::Queued
            })
        }) {
            *slot = None;
        }
    }

    fn roots(&self) -> usize {
        self.live
            .iter()
            .filter(|entry| entry.parent.is_none())
            .count()
    }

    fn projected_leaves(&self, removed: &[Option<RequestId>], parent: Option<RequestId>) -> usize {
        self.live
            .iter()
            .filter(|entry| !removed.contains(&Some(entry.id)))
            .filter(|entry| {
                parent != Some(entry.id)
                    && !self.live.iter().any(|child| {
                        !removed.contains(&Some(child.id)) && child.parent == Some(entry.id)
                    })
            })
            .count()
            + 1
    }

    /// Consume one carried start after the watermark and controls. A credit-
    /// deferred start stays in the producer's custody and is retried in order.
    /// `cancelled` ties an unacknowledged start to ParentAborted cancellation.
    pub fn start(
        &mut self,
        stamp: RequestStamp,
        kind: NativeKind,
        cancelled: bool,
    ) -> Result<Admission, SeatError> {
        if self.held {
            return Ok(Admission::Deferred);
        }
        if self.stopped || stamp.run() != self.run {
            return Ok(Admission::Stale);
        }
        let id = stamp.id();
        if self.live.iter().any(|entry| entry.id == id)
            || self.results.iter().any(|result| result.id == id)
        {
            return Ok(Admission::Duplicate);
        }
        if self.live.len() + self.results.len() == MAX_RESULTS {
            return Ok(Admission::Deferred);
        }
        if cancelled {
            self.publish(
                id,
                SeatTerminal::Aborted(AbortReason::ParentAborted),
                None,
                true,
            )?;
            return Ok(Admission::Cancelled);
        }
        let mut parent = stamp.parent();
        if let Some(id) = parent {
            match self.lookup(id) {
                AuthorityStatus::Live if self.entry(id).is_some() => {}
                AuthorityStatus::Completed => parent = None,
                AuthorityStatus::Expired => {
                    self.publish(
                        stamp.id(),
                        SeatTerminal::Refused(Refusal::HistoryExpired),
                        None,
                        false,
                    )?;
                    return Ok(Admission::Refused(Refusal::HistoryExpired));
                }
                AuthorityStatus::Live | AuthorityStatus::Pending | AuthorityStatus::Revoked => {
                    self.publish(
                        stamp.id(),
                        SeatTerminal::Refused(Refusal::ParentNotLive),
                        None,
                        false,
                    )?;
                    return Ok(Admission::Refused(Refusal::ParentNotLive));
                }
            }
        }
        let class = kind.policy().class();
        let mut victim = None;
        let depth = if let Some(parent_id) = parent {
            if matches!(class, RequestClass::Session(_)) {
                return self.refuse(id, Refusal::Busy);
            }
            let parent_entry = self.entry(parent_id).expect("checked live parent");
            if parent_entry.depth as usize >= MAX_LINEAGE_DEPTH {
                return self.refuse(id, Refusal::LineageDepth);
            }
            let (chain, _) = self.chain(parent_id);
            if group(kind).is_some_and(|g| {
                chain.into_iter().flatten().any(|ancestor| {
                    self.entry(ancestor)
                        .is_some_and(|entry| group(entry.kind) == Some(g))
                })
            }) {
                return self.refuse(id, Refusal::Busy);
            }
            victim = self
                .live
                .iter()
                .find(|entry| {
                    entry.parent == Some(parent_id)
                        && matches!(entry.kind.policy().class(), RequestClass::Capability)
                        && group(kind).is_some_and(|g| group(entry.kind) == Some(g))
                })
                .map(|entry| entry.id);
            if victim.is_none()
                && self.live.iter().any(|entry| {
                    entry.parent == Some(parent_id) && entry.origin == ChildOrigin::Started
                })
            {
                return self.refuse(id, Refusal::ParentBusy);
            }
            if victim.is_none() {
                victim = self.live.iter().find_map(|entry| {
                    (matches!(entry.kind.policy().class(), RequestClass::Capability)
                        && group(kind).is_some_and(|g| group(entry.kind) == Some(g))
                        && !self.has_descendant(entry.id, parent_id))
                    .then_some(entry.id)
                });
            }
            parent_entry.depth + 1
        } else {
            // Admission is ordered: a Session may start beside existing
            // capability roots, but any live Session blocks a new root,
            // including a Capability under Hunt's non-filtering foreground.
            if self.session().is_some() {
                return self.refuse(id, Refusal::Busy);
            }
            if matches!(class, RequestClass::Capability) {
                victim = self.live.iter().find_map(|entry| {
                    (matches!(entry.kind.policy().class(), RequestClass::Capability)
                        && group(kind).is_some_and(|g| group(entry.kind) == Some(g)))
                    .then_some(entry.id)
                });
            }
            if self.roots() == MAX_ROOTS
                && victim
                    .is_none_or(|target| self.entry(target).is_some_and(|e| e.parent.is_some()))
            {
                return self.refuse(id, Refusal::Busy);
            }
            1
        };
        let mut removed = [None; MAX_LIVE];
        let mut count = 0;
        if let Some(victim) = victim {
            self.subtree_leaf_first(victim, &mut removed, &mut count);
            removed[count] = Some(victim);
            count += 1;
        }
        let removed = &removed[..count];
        if self.live.len() - removed.len() + 1 > MAX_LIVE
            || self.projected_leaves(removed, parent) > MAX_LIVE_LEAVES
        {
            return self.refuse(id, Refusal::Busy);
        }
        if kind == NativeKind::Walk && !self.can_reserve_walk(id) {
            return Ok(Admission::Deferred);
        }
        if let Some(victim) = victim {
            self.revoke(victim, AbortReason::Superseded)?;
        }
        if kind == NativeKind::Walk {
            assert!(self.reserve_walk(id), "walk slot preflight");
        }
        self.live.push(SeatEntry {
            id,
            parent,
            kind,
            origin: ChildOrigin::Started,
            depth,
            decisions_consumed: 0,
            prompt: None,
        });
        self.live.sort_unstable_by_key(|entry| entry.id);
        self.note_id(id);
        self.witness(id);
        Ok(Admission::Admitted)
    }

    fn refuse(&mut self, id: RequestId, reason: Refusal) -> Result<Admission, SeatError> {
        self.publish(id, SeatTerminal::Refused(reason), None, false)?;
        Ok(Admission::Refused(reason))
    }

    /// Capture an immutable row lease at the producer gate. A pending owner
    /// has conditional custody until its successful start sets the witness.
    pub fn push(
        &mut self,
        payload: R,
        class: EmissionClass,
        source: EmissionSource,
        release: Option<api::script_contract::Release>,
    ) -> Result<(), SeatError> {
        self.push_with_walk(payload, class, source, release, None, false)
    }

    /// The physical Walk request has a separate host token. Carry its
    /// release-slot claim with the action, pacing the drive at Take rather
    /// than losing a game row when all sixteen slots are occupied.
    pub fn push_walk(
        &mut self,
        payload: R,
        source: EmissionSource,
        walk: RequestId,
    ) -> Result<(), SeatError> {
        self.push_with_walk(
            payload,
            EmissionClass::Gameplay,
            source,
            None,
            Some(walk),
            false,
        )
    }

    /// The one coalesced run-policy control has encoder reservation rather
    /// than competing with the 141 gameplay/lifecycle rows.
    pub fn push_policy(&mut self, payload: R, source: EmissionSource) -> Result<(), SeatError> {
        self.push_with_walk(payload, EmissionClass::Lifecycle, source, None, None, true)
    }

    fn push_with_walk(
        &mut self,
        payload: R,
        class: EmissionClass,
        source: EmissionSource,
        release: Option<api::script_contract::Release>,
        walk: Option<RequestId>,
        reserved_policy: bool,
    ) -> Result<(), SeatError> {
        if class == EmissionClass::Host || (class == EmissionClass::Release) != release.is_some() {
            return Err(SeatError::InvalidAuthority);
        }
        if matches!(source, EmissionSource::InvalidExternal(_)) {
            self.dropped += 1;
            return Err(SeatError::ProtocolDefect);
        }
        if let EmissionSource::Mirror { owner, .. } | EmissionSource::PendingMirror { owner, .. } =
            source
        {
            if owner.namespace() != RequestNamespace::Isolate
                || self.live.iter().any(|entry| entry.id == owner)
                || self.results.iter().any(|result| result.id == owner)
            {
                self.dropped += 1;
                return Err(SeatError::ProtocolDefect);
            }
        }
        if self.stopped {
            self.dropped += 1;
            return Ok(());
        }
        if self.held && class == EmissionClass::Gameplay {
            self.dropped += 1;
            return Ok(());
        }
        if release.is_none()
            && !reserved_policy
            && self
                .queued
                .iter()
                .filter(|row| row.release.is_none() && !row.reserved_policy)
                .count()
                >= MAX_INTERACT_REQS
        {
            self.dropped += 1;
            return Ok(());
        }
        let (authority, witness, proof, boundary) = match source {
            EmissionSource::Root => (
                None,
                AdmissionWitness::Admitted,
                EmissionProof::Proven,
                AuthorityBoundary::Preserved,
            ),
            EmissionSource::InvalidExternal(_) => {
                unreachable!("invalid external lineage preflighted")
            }
            EmissionSource::Owner(owner)
                if matches!(
                    self.lookup(owner),
                    AuthorityStatus::Revoked | AuthorityStatus::Expired
                ) =>
            {
                let expired = self.lookup(owner) == AuthorityStatus::Expired;
                (
                    None,
                    AdmissionWitness::Admitted,
                    if expired {
                        EmissionProof::ExpiredLookup
                    } else {
                        EmissionProof::Proven
                    },
                    if expired {
                        AuthorityBoundary::Preserved
                    } else {
                        AuthorityBoundary::Revoked
                    },
                )
            }
            EmissionSource::Owner(owner)
                if self.results.iter().any(|result| {
                    result.id == owner
                        && matches!(result.outcome, SeatTerminal::Done | SeatTerminal::Failed)
                        && !result.authority_revoked
                }) =>
            {
                // A new post-completion push is a root action. Rows emitted
                // before a same-batch terminal pinned their lease at ingress.
                (
                    None,
                    AdmissionWitness::Admitted,
                    EmissionProof::Proven,
                    AuthorityBoundary::Preserved,
                )
            }
            EmissionSource::Owner(owner)
            | EmissionSource::Mirror { owner, .. }
            | EmissionSource::Pending { owner, .. }
            | EmissionSource::PendingMirror { owner, .. } => {
                let pending = matches!(
                    source,
                    EmissionSource::Pending { .. } | EmissionSource::PendingMirror { .. }
                );
                let (parent, owner_session) = match source {
                    EmissionSource::Owner(_) => {
                        if let Some(entry) = self.entry(owner) {
                            (entry.parent, self.captured_session(owner))
                        } else {
                            self.dropped += 1;
                            return Err(SeatError::ProtocolDefect);
                        }
                    }
                    EmissionSource::Mirror { parent, .. } => (parent, None),
                    EmissionSource::Pending { parent, kind, .. } => (
                        parent,
                        matches!(kind.policy().class(), RequestClass::Session(_)).then_some(owner),
                    ),
                    EmissionSource::PendingMirror { pending, kind, .. } => (
                        Some(pending),
                        matches!(kind.policy().class(), RequestClass::Session(_))
                            .then_some(pending),
                    ),
                    EmissionSource::Root | EmissionSource::InvalidExternal(_) => unreachable!(),
                };
                if parent.is_some_and(|id| self.lookup(id) == AuthorityStatus::Pending)
                    && !matches!(source, EmissionSource::PendingMirror { .. })
                {
                    self.dropped += 1;
                    return Err(SeatError::ProtocolDefect);
                }
                let (chain, mut len) =
                    if let EmissionSource::PendingMirror {
                        pending, parent, ..
                    } = source
                    {
                        let mut chain = [None; MAX_LINEAGE_DEPTH];
                        chain[0] = Some(pending);
                        let (ancestors, length) =
                            parent.map_or(([None; MAX_LINEAGE_DEPTH], 0), |id| self.chain(id));
                        for (to, from) in chain[1..].iter_mut().zip(ancestors[..length].iter()) {
                            *to = *from;
                        }
                        (chain, (length + 1).min(MAX_LINEAGE_DEPTH))
                    } else {
                        parent.map_or(([None; MAX_LINEAGE_DEPTH], 0), |id| self.chain(id))
                    };
                let mut ancestors = [owner; MAX_LINEAGE_DEPTH];
                for (slot, ancestor) in ancestors.iter_mut().zip(chain[..len].iter().flatten()) {
                    *slot = *ancestor;
                }
                let session = owner_session.or_else(|| {
                    chain[..len]
                        .iter()
                        .flatten()
                        .copied()
                        .find_map(|id| self.captured_session(id))
                });
                if let Some(session) = session
                    .filter(|session| *session != owner && !ancestors[..len].contains(session))
                {
                    if len == ancestors.len() {
                        return Err(SeatError::InvalidAuthority);
                    }
                    ancestors[len] = session;
                    len += 1;
                }
                let lease = CapturedAuthority::new(owner, &ancestors[..len], session)
                    .map_err(|_| SeatError::InvalidAuthority)?;
                let status = if matches!(
                    source,
                    EmissionSource::Mirror { .. } | EmissionSource::PendingMirror { .. }
                ) {
                    if owner.get() < self.mirror_floor {
                        AuthorityStatus::Expired
                    } else if self.mirror_revoked.contains(&owner) {
                        AuthorityStatus::Revoked
                    } else {
                        AuthorityStatus::Live
                    }
                } else {
                    self.lookup(owner)
                };
                let ancestor_revoked = ancestors[..len].iter().copied().any(|id| {
                    matches!(
                        self.lookup(id),
                        AuthorityStatus::Revoked | AuthorityStatus::Expired
                    )
                });
                (
                    Some(lease),
                    if pending {
                        AdmissionWitness::Pending
                    } else {
                        AdmissionWitness::Admitted
                    },
                    if status == AuthorityStatus::Expired {
                        EmissionProof::ExpiredLookup
                    } else {
                        EmissionProof::Proven
                    },
                    if status == AuthorityStatus::Revoked || ancestor_revoked {
                        AuthorityBoundary::Revoked
                    } else {
                        AuthorityBoundary::Preserved
                    },
                )
            }
        };
        if let Some(release) = release {
            if !self.reserve_release(release)? {
                return Ok(());
            }
        }
        if let EmissionSource::Mirror { owner, .. } | EmissionSource::PendingMirror { owner, .. } =
            source
        {
            self.max_mirror = self.max_mirror.max(owner.get());
        }
        if reserved_policy {
            if let Some(at) = self.queued.iter().position(|row| row.reserved_policy) {
                self.queued.remove(at).expect("found reserved policy");
                self.dropped += 1;
            }
        }
        self.queued.push_back(SeatRow {
            payload,
            class,
            authority,
            witness,
            pending_owner: match source {
                EmissionSource::Pending { owner, .. } => Some(owner),
                EmissionSource::PendingMirror { pending, .. } => Some(pending),
                _ => None,
            },
            proof,
            boundary,
            release,
            reserved_policy,
            walk,
        });
        Ok(())
    }

    fn reserve_release(
        &mut self,
        release: api::script_contract::Release,
    ) -> Result<bool, SeatError> {
        use api::script_contract::Release;
        match release {
            Release::AbortWalk(id) => {
                if let Some(slot) = self
                    .walk_releases
                    .iter_mut()
                    .flatten()
                    .find(|slot| slot.id == id)
                {
                    if slot.state == WalkReleaseState::Queued {
                        return Ok(false);
                    }
                    slot.state = WalkReleaseState::Queued;
                    return Ok(true);
                }
                let Some(slot) = self.walk_releases.iter_mut().find(|slot| slot.is_none()) else {
                    return Err(SeatError::Capacity);
                };
                *slot = Some(WalkReleaseSlot {
                    id,
                    state: WalkReleaseState::Queued,
                });
            }
            Release::GatherStop if self.gather_release => return Ok(false),
            Release::GatherStop => self.gather_release = true,
            Release::CombatStop if self.combat_release => return Ok(false),
            Release::CombatStop => self.combat_release = true,
        }
        Ok(true)
    }

    fn release_delivered(&mut self, release: api::script_contract::Release) {
        use api::script_contract::Release;
        match release {
            Release::AbortWalk(id) => {
                if let Some(slot) = self
                    .walk_releases
                    .iter_mut()
                    .find(|slot| slot.is_some_and(|armed| armed.id == id))
                {
                    *slot = None;
                }
            }
            Release::GatherStop => self.gather_release = false,
            Release::CombatStop => self.combat_release = false,
        }
    }

    /// Non-reserved rows that may join the queue without the next `take`
    /// clearing any queued row. Zero while held, or while a taken batch has
    /// not dispatched.
    pub fn take_room(&self) -> usize {
        if self.held || !self.in_flight.is_empty() {
            return 0;
        }
        MAX_ENCODED_GAMEPLAY.saturating_sub(
            self.queued
                .iter()
                .filter(|row| row.release.is_none() && !row.reserved_policy)
                .count(),
        )
    }

    /// Encoder priority: all reserved releases first, then at most 141 other
    /// rows in original push order. A prior batch must dispatch before Take.
    pub fn take(&mut self) {
        if self.held || !self.in_flight.is_empty() {
            return;
        }
        let mut index = 0;
        while index < self.queued.len() {
            if self.queued[index].release.is_some() || self.queued[index].reserved_policy {
                self.in_flight
                    .push(self.queued.remove(index).expect("reserved row at index"));
            } else {
                index += 1;
            }
        }
        let mut moved = 0;
        while moved < MAX_ENCODED_GAMEPLAY && !self.queued.is_empty() {
            if let Some(id) = self.queued[0].walk {
                if !self.reserve_walk(id) {
                    break;
                }
            }
            self.in_flight
                .push(self.queued.pop_front().expect("front row"));
            moved += 1;
        }
        if moved == MAX_ENCODED_GAMEPLAY {
            self.dropped += self.queued.len() as u64;
            self.queued.clear();
        }
    }

    /// Dispatch into a caller-owned reusable output; host-issued actions do
    /// not enter this method. Revocation already marked pinned leases in both
    /// custody locations, so terminal retirement cannot re-root accepted work.
    pub fn dispatch_into(&mut self, output: &mut Vec<R>) {
        if self.held {
            return;
        }
        let foreground = self.foreground();
        let mut releases = [None; 18];
        let mut release_count = 0;
        for row in self.in_flight.drain(..) {
            let decision = dispatch_disposition(
                row.class,
                row.authority.as_ref().and_then(CapturedAuthority::session),
                foreground,
                row.witness,
                row.proof,
                row.boundary,
                false,
            );
            if decision == DispatchDisposition::Dispatch {
                if let Some(id) = row.walk {
                    self.walk_releases
                        .iter_mut()
                        .flatten()
                        .find(|slot| slot.id == id)
                        .expect("walk release reserved before dispatch")
                        .state = WalkReleaseState::Armed;
                }
                if let Some(release) = row.release {
                    releases[release_count] = Some(release);
                    release_count += 1;
                }
                output.push(row.payload);
            } else {
                self.dropped += 1;
                if let Some(id) = row.walk {
                    if let Some(slot) = self.walk_releases.iter_mut().find(|slot| {
                        slot.is_some_and(|reserved| {
                            reserved.id == id && reserved.state == WalkReleaseState::Reserved
                        })
                    }) {
                        *slot = None;
                    }
                }
                if let Some(release) = row.release {
                    releases[release_count] = Some(release);
                    release_count += 1;
                }
            }
        }
        for release in releases.into_iter().flatten() {
            self.release_delivered(release);
        }
    }

    pub fn hold(&mut self, held: bool) {
        if held && !self.held {
            self.drop_gameplay();
        }
        self.held = held;
    }

    fn drop_gameplay(&mut self) {
        let before = self.queued.len() + self.in_flight.len();
        self.queued
            .retain(|row| row.class != EmissionClass::Gameplay);
        self.in_flight
            .retain(|row| row.class != EmissionClass::Gameplay);
        for slot in &mut self.walk_releases {
            if slot.is_some_and(|reserved| reserved.state == WalkReleaseState::Reserved) {
                *slot = None;
            }
        }
        self.dropped += (before - self.queued.len() - self.in_flight.len()) as u64;
    }

    pub fn held_tick(&mut self) {
        if self.held {
            return;
        }
        self.dropped += (self.queued.len() + self.in_flight.len()) as u64;
        self.queued.clear();
        self.in_flight.clear();
        self.walk_releases.fill(None);
        self.gather_release = false;
        self.combat_release = false;
    }

    /// Kept reconnect discards game actions, preserves results and releases.
    pub fn reconnect(&mut self, session: u64) {
        self.run.session = session;
        self.drop_gameplay();
        // Dropping gameplay clears unarmed drives; queued releases retain
        // their reservations across the kept reconnect.
    }

    /// Non-kept reset settles locally in the adapters. No seat result survives.
    pub fn reset(&mut self, run: RunKey) {
        self.live.clear();
        self.prompts.clear();
        self.answers.clear();
        self.machine_phases.fill(None);
        self.results.clear();
        self.revoked.clear();
        self.queued.clear();
        self.in_flight.clear();
        self.walk_releases.fill(None);
        self.gather_release = false;
        self.combat_release = false;
        self.isolate_floor = self.max_isolate.saturating_add(1);
        self.host_floor = self.max_host.saturating_add(1);
        self.mirror_revoked.clear();
        self.mirror_floor = self.max_mirror.saturating_add(1);
        self.mirror_revokes_consumed = 0;
        self.next_seq = 0;
        self.consumed_seq = 0;
        self.run = run;
        self.held = false;
        self.stopped = false;
    }

    /// Stop has no published terminals or JS settlement.
    pub fn stop(&mut self) {
        self.live.clear();
        self.prompts.clear();
        self.answers.clear();
        self.machine_phases.fill(None);
        self.results.clear();
        self.queued.clear();
        self.in_flight.clear();
        self.walk_releases.fill(None);
        self.gather_release = false;
        self.combat_release = false;
        self.revoked.clear();
        self.isolate_floor = RequestId::ISOLATE_FIRST;
        self.host_floor = RequestId::HOST_FIRST;
        self.max_isolate = 0;
        self.max_host = RequestId::HOST_FIRST - 1;
        self.mirror_revoked.clear();
        self.mirror_floor = RequestId::ISOLATE_FIRST;
        self.max_mirror = 0;
        self.mirror_revokes_consumed = 0;
        self.next_seq = 0;
        self.consumed_seq = 0;
        self.held = false;
        self.dropped = 0;
        self.late_controls = 0;
        self.stopped = true;
    }
}

/// Only the frozen exclusive groups supersede; independent Capabilities may
/// coexist, up to the four reserved future roots.
fn group(kind: NativeKind) -> Option<NativeKind> {
    match kind {
        NativeKind::BankPlan => Some(NativeKind::BankPlan),
        NativeKind::Dialogue
        | NativeKind::MakeX
        | NativeKind::Shop
        | NativeKind::PartnerTrade
        | NativeKind::Teleport
        | NativeKind::Reach
        | NativeKind::Prayer
        | NativeKind::Autocast
        | NativeKind::Special
        | NativeKind::Modals
        | NativeKind::Fire
        | NativeKind::Clue
        | NativeKind::Duel
        | NativeKind::CakeStall => Some(kind),
        NativeKind::Walk
        | NativeKind::Gather
        | NativeKind::Combat
        | NativeKind::Hunt
        | NativeKind::Quester
        | NativeKind::DeathRecovery
        | NativeKind::QuestProgress
        | NativeKind::BankNearest
        | NativeKind::PeriodicBank
        | NativeKind::Trade => None,
    }
}

#[cfg(test)]
#[path = "seat_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "seat_product_tests.rs"]
mod product_tests;
