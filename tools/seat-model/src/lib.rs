//! Bounded conformance oracle. No product crate depends on this crate.
//!
//! Boundary observations feed a separate specification monitor. Authority uses
//! live entries, retained results, history and bounded leases in queued rows.
//! See synthesis/28-seat-sg.md.
use std::fmt::Write;

mod explorer;
mod monitor;
pub use explorer::*;
use monitor::Monitor;

pub type Id = u16;
pub const MAX_START_CHARGE: usize = 64 * 1024;
pub const START_METADATA: usize = 128;
pub const HOOK_CHARGE: usize = 16;
pub const MAX_HOOKS: u8 = 32;
pub const CONTROL_RESERVATION: usize = 115;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Rules {
    R5,
    Chosen,
}

/// Permanent seeded regressions. Production exploration uses `None`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Mutation {
    #[default]
    None,
    Deferral,
    Replacement,
    ReleaseDisplacement,
    SessionCapture,
    ReplacementLabel,
    KeptReleaseDiscard,
    RevocationCascade(Trigger),
    ReplacementCascade,
    ExpiredParent,
    HostActionDrop,
    HostActionReorder,
    GuardianHold,
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SessionFamily {
    Hunt,
    Gather,
    Combat,
    Progress,
}
impl SessionFamily {
    pub const fn owns_foreground(self) -> bool {
        !matches!(self, Self::Hunt)
    }
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Class {
    Capability,
    Session(SessionFamily),
}
impl Class {
    pub const fn is_session(self) -> bool {
        matches!(self, Self::Session(_))
    }
    pub const fn owns_foreground(self) -> bool {
        matches!(self, Self::Session(family) if family.owns_foreground())
    }
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Origin {
    Started,
    Adopted,
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Trigger {
    Superseded,
    ParentAborted,
    HostRevoke,
    ManualWalk,
    ManualCombat,
    GatherSignal,
    Pause,
    Reset,
    Stop,
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Refusal {
    Busy,
    ParentBusy,
    ParentNotLive,
    LineageDepth,
    ArgsTooLarge,
    HistoryExpired,
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Outcome {
    Done,
    Failed,
    Aborted(Trigger),
    Refused(Refusal),
}
impl Outcome {
    fn revoked(self) -> bool {
        matches!(self, Self::Aborted(_))
    }
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Bounds {
    pub roots: usize,
    pub live: usize,
    pub depth: usize,
    pub credits: usize,
    pub pending: usize,
    pub history: usize,
    pub rows: usize,
    pub gameplay: usize,
    pub ids: Id,
    pub batches: usize,
}
impl Bounds {
    pub fn production() -> Self {
        Self {
            roots: 4,
            live: 16,
            depth: 4,
            credits: 32,
            pending: 32,
            history: 64,
            rows: 256,
            gameplay: 141,
            ids: 512,
            batches: 64,
        }
    }
    pub fn small() -> Self {
        Self {
            roots: 2,
            live: 3,
            depth: 3,
            credits: 4,
            pending: 2,
            history: 2,
            rows: 3,
            gameplay: 1,
            ids: 3,
            batches: 2,
        }
    }
    pub fn closure() -> Self {
        Self {
            roots: 1,
            live: 2,
            depth: 2,
            credits: 2,
            pending: 1,
            history: 1,
            rows: 2,
            gameplay: 1,
            ids: 1,
            batches: 1,
        }
    }
}
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Step {
    Start {
        parent: Option<Id>,
        local_parent: Option<Id>,
        class: Class,
        group: u8,
        bytes: usize,
        hooks: u8,
    },
    Mirror {
        ancestor: Option<Id>,
        parent: Option<Id>,
    },
    Admit(Id),
    AdmitStamped {
        of: Id,
        session: u16,
    },
    Complete(Id),
    Throw(Id),
    Revoke {
        of: Id,
        trigger: Trigger,
    },
    Cancel {
        of: Id,
        throw: bool,
    },
    Prompt(Id),
    Decide {
        of: Id,
        complete: bool,
    },
    Controls,
    Emit {
        of: Option<Id>,
        release: bool,
    },
    Take,
    Dispatch,
    Page,
    Watermark,
    Batch,
    Reconnect {
        keep: bool,
    },
    Stop,
    HeldTick,
    /// Host actors dispatch outside the seat encoder and lineage.
    HostAction {
        action: u8,
    },
    /// Guardian hold is not a queue-discard tick or lifecycle row.
    SessionHeld,
    SessionResumed,
}
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Entry {
    pub id: Id,
    pub parent: Option<Id>,
    pub class: Class,
    pub group: u8,
    pub origin: Origin,
    pub prompt: bool,
    pub decisions: u8,
    pub local_parent: Option<Id>,
}
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Pending {
    entry: Entry,
    local_parent: Option<Id>,
    bytes: usize,
    hooks: u8,
    deferred: bool,
    cancelled: bool,
}
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Mirror {
    id: Id,
    ancestor: Option<Id>,
    parent: Option<Id>,
}
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Terminal {
    id: Id,
    seq: u16,
    outcome: Outcome,
    old_parent: Option<Id>,
    authority_revoked: bool,
}
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum Control {
    Cancel { of: Id, throw: bool },
    Decide { of: Id, seq: u8, complete: bool },
}
impl Control {
    fn id(&self) -> Id {
        match *self {
            Self::Cancel { of, .. } | Self::Decide { of, .. } => of,
        }
    }
}
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Emission {
    owner: Option<Id>,
    lineage: Option<Id>,
    ancestors: Vec<Id>,
    session: Option<Id>,
    release: bool,
    revoked: bool,
    dropped: bool,
    take_filtered: bool,
    awaiting_admission: bool,
    witness: usize,
}
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Record {
    outcome: Option<Outcome>,
    settlements: u8,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Status {
    Live,
    Pending,
    Completed,
    Revoked,
    Expired,
}
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct State {
    pub rules: Rules,
    pub bounds: Bounds,
    entries: Vec<Entry>,
    pending: Vec<Pending>,
    mirrors: Vec<Mirror>,
    ledger: Vec<Terminal>,
    carry: Vec<Control>,
    history: Vec<Id>,
    floor: Id,
    queue: Vec<Emission>,
    wire: Vec<Emission>,
    records: Vec<Record>,
    next: Id,
    terminal_seq: u16,
    consumed_seq: u16,
    batches: usize,
    session: u16,
    dispatched: usize,
    releases: usize,
    late_controls: usize,
    dropped: usize,
    host_actions: usize,
    last_host_action: Option<u8>,
    held: bool,
    stopped: bool,
    failure: Option<Violation>,
    monitor: Monitor,
    mutation: Mutation,
}
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Violation {
    pub name: &'static str,
    pub detail: String,
}
impl State {
    pub fn new(rules: Rules, bounds: Bounds) -> Self {
        assert!(bounds.roots > 0 && bounds.depth > 0 && bounds.history > 0);
        Self {
            rules,
            bounds,
            entries: Vec::new(),
            pending: Vec::new(),
            mirrors: Vec::new(),
            ledger: Vec::new(),
            carry: Vec::new(),
            history: Vec::new(),
            floor: 0,
            queue: Vec::new(),
            wire: Vec::new(),
            records: Vec::new(),
            next: 1,
            terminal_seq: 0,
            consumed_seq: 0,
            batches: 0,
            session: 0,
            dispatched: 0,
            releases: 0,
            late_controls: 0,
            dropped: 0,
            host_actions: 0,
            last_host_action: None,
            held: false,
            stopped: false,
            failure: None,
            monitor: Monitor::default(),
            mutation: Mutation::None,
        }
    }
    pub fn with_mutation(rules: Rules, bounds: Bounds, mutation: Mutation) -> Self {
        Self {
            mutation,
            ..Self::new(rules, bounds)
        }
    }
    pub fn live(&self) -> usize {
        self.entries.len()
    }
    pub fn retained(&self) -> usize {
        self.ledger.len()
    }
    pub fn pending(&self) -> usize {
        self.pending.len()
    }
    pub fn roots(&self) -> usize {
        self.entries.iter().filter(|e| e.parent.is_none()).count()
    }
    pub fn leaves(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| !self.entries.iter().any(|c| c.parent == Some(e.id)))
            .count()
    }
    pub fn next_id(&self) -> Id {
        self.next
    }
    pub fn entry(&self, id: Id) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }
    pub fn outcome(&self, id: Id) -> Option<Outcome> {
        id.checked_sub(1)
            .and_then(|i| self.records.get(usize::from(i)))
            .and_then(|r| r.outcome)
            .or_else(|| self.ledger.iter().find(|t| t.id == id).map(|t| t.outcome))
    }
    pub fn dispatched(&self) -> usize {
        self.dispatched
    }
    pub fn release_dispatched(&self) -> usize {
        self.releases
    }
    pub fn carry_len(&self) -> usize {
        self.carry.len()
    }
    pub fn dropped(&self) -> usize {
        self.dropped
    }
    pub fn host_actions_dispatched(&self) -> usize {
        self.host_actions
    }
    pub fn last_host_action(&self) -> Option<u8> {
        self.last_host_action
    }
    pub fn session_held(&self) -> bool {
        self.held
    }
    pub fn foreground_session(&self) -> Option<Id> {
        self.entries
            .iter()
            .find(|e| e.class.owns_foreground())
            .map(|e| e.id)
    }
    pub fn encoded_rows(&self) -> Vec<(Option<Id>, bool)> {
        self.wire
            .iter()
            .map(|row| (row.owner, row.release))
            .collect()
    }
    pub fn history_floor(&self) -> Id {
        self.floor
    }
    pub fn terminal_order(&self) -> Vec<Id> {
        self.ledger.iter().map(|t| t.id).collect()
    }
    pub fn mirror_ancestor(&self, id: Id) -> Option<Option<Id>> {
        self.mirrors.iter().find(|m| m.id == id).map(|m| m.ancestor)
    }
    pub fn late_controls(&self) -> usize {
        self.late_controls
    }
    pub fn settlements(&self, id: Id) -> u8 {
        self.records[usize::from(id - 1)].settlements
    }
    fn free(&self) -> usize {
        self.bounds
            .credits
            .saturating_sub(self.live() + self.retained())
    }
    fn status(&self, id: Id) -> Status {
        if self.entry(id).is_some() || self.mirrors.iter().any(|m| m.id == id) {
            return Status::Live;
        }
        if let Some(t) = self.ledger.iter().find(|t| t.id == id) {
            if t.authority_revoked {
                return Status::Revoked;
            }
            if self.rules == Rules::Chosen {
                return Status::Completed;
            }
        }
        if self.pending.iter().any(|p| p.entry.id == id) && self.ledger.iter().all(|t| t.id != id) {
            return if self.rules == Rules::Chosen {
                Status::Pending
            } else {
                Status::Live
            };
        }
        if self.history.contains(&id) {
            return Status::Revoked;
        }
        if id < self.floor {
            return if self.rules == Rules::R5 {
                Status::Revoked
            } else {
                Status::Expired
            };
        }
        Status::Completed
    }
    fn chain(&self, mut id: Id) -> Vec<Id> {
        let mut out = Vec::new();
        while let Some(e) = self.entry(id) {
            out.push(id);
            if let Some(p) = e.parent {
                id = p;
            } else {
                break;
            }
        }
        out
    }
    fn depth(&self, id: Id) -> usize {
        self.chain(id).len()
    }
    fn session_root(&self) -> Option<Id> {
        self.entries
            .iter()
            .find(|e| e.class.is_session())
            .map(|e| e.id)
    }
    fn mint(&mut self) -> Id {
        let id = self.next;
        self.next += 1;
        self.monitor.minted();
        self.records.push(Record {
            outcome: None,
            settlements: 0,
        });
        id
    }
    fn remember(&mut self, id: Id) {
        self.monitor.history_report(id);
        if id < self.floor || self.history.contains(&id) {
            return;
        }
        self.history.push(id);
        self.history.sort_unstable();
        if self.history.len() > self.bounds.history {
            self.floor = self.history.remove(0) + 1;
        }
    }
    fn settle_local(&mut self, id: Id, outcome: Outcome) {
        self.monitor.settled(id, outcome);
        let r = &mut self.records[usize::from(id - 1)];
        r.settlements = r.settlements.saturating_add(1);
        if r.outcome.is_none() {
            r.outcome = Some(outcome);
        }
    }
    fn publish(&mut self, id: Id, outcome: Outcome, old_parent: Option<Id>) {
        self.terminal_seq += 1;
        self.monitor.published(id, outcome, self.bounds);
        self.ledger.push(Terminal {
            id,
            seq: self.terminal_seq,
            outcome,
            old_parent,
            authority_revoked: outcome.revoked(),
        });
        if outcome.revoked() {
            self.remember(id);
        }
    }
    fn retire(&mut self, id: Id) {
        self.pending.retain(|p| p.entry.id != id);
        self.carry.retain(|c| c.id() != id);
    }
    fn revoke_leases(&mut self, id: Id) {
        for row in self.queue.iter_mut().chain(&mut self.wire) {
            if row.owner == Some(id) || row.ancestors.contains(&id) {
                row.revoked = true;
            }
        }
    }
    fn complete(&mut self, id: Id, outcome: Outcome) {
        let at = self
            .entries
            .iter()
            .position(|e| e.id == id)
            .expect("live precondition");
        let parent = self.entries.remove(at).parent;
        for child in &mut self.entries {
            if child.parent == Some(id) {
                child.parent = parent;
                if parent.is_some() {
                    child.origin = Origin::Adopted;
                }
            }
        }
        self.publish(id, outcome, parent);
    }
    fn descendants(&self, id: Id, out: &mut Vec<Id>) {
        for child in self.entries.iter().filter(|e| e.parent == Some(id)) {
            self.descendants(child.id, out);
            out.push(child.id);
        }
    }
    fn cancel_pending(&mut self, id: Id) {
        if let Some(p) = self.pending.iter_mut().find(|p| p.entry.id == id) {
            p.cancelled = true;
        }
        self.carry.retain(|c| c.id() != id);
        self.carry.push(Control::Cancel {
            of: id,
            throw: false,
        });
    }
    fn revoke(&mut self, id: Id, trigger: Trigger) {
        if matches!(trigger, Trigger::GatherSignal | Trigger::Pause) {
            return;
        }
        if let Some(at) = self.mirrors.iter().position(|m| m.id == id) {
            self.monitor.local_end(id, true, self.bounds);
            self.mirrors.remove(at);
            self.remember(id);
            self.revoke_leases(id);
            self.settle_local(
                id,
                if trigger == Trigger::ManualWalk {
                    Outcome::Done
                } else {
                    Outcome::Aborted(trigger)
                },
            );
            let mut children: Vec<_> = self
                .pending
                .iter()
                .filter(|p| p.local_parent == Some(id))
                .map(|p| p.entry.id)
                .collect();
            children.extend(
                self.entries
                    .iter()
                    .filter(|e| e.local_parent == Some(id))
                    .map(|e| e.id),
            );
            children.sort_unstable();
            children.dedup();
            for child in children {
                self.cancel_pending(child);
            }
            let children: Vec<_> = self
                .mirrors
                .iter()
                .filter(|m| m.parent == Some(id))
                .map(|m| m.id)
                .collect();
            for child in children {
                self.revoke(child, Trigger::ParentAborted);
            }
            return;
        }
        let mut ids = Vec::new();
        if self.mutation != Mutation::RevocationCascade(trigger) {
            self.descendants(id, &mut ids);
        }
        ids.push(id);
        for child in ids {
            self.revoke_leases(child);
            let at = self
                .entries
                .iter()
                .position(|e| e.id == child)
                .expect("live subtree");
            let parent = self.entries.remove(at).parent;
            let outcome = if child == id && trigger == Trigger::ManualWalk {
                Outcome::Done
            } else {
                Outcome::Aborted(if child == id {
                    trigger
                } else {
                    Trigger::ParentAborted
                })
            };
            // Manual-walk Done still revokes its accepted work, unlike ordinary completion.
            if trigger == Trigger::ManualWalk {
                self.remember(child);
            }
            self.publish(child, outcome, parent);
            if trigger == Trigger::ManualWalk {
                self.ledger
                    .last_mut()
                    .expect("published takeover")
                    .authority_revoked = true;
            }
        }
    }
    fn refuse(&mut self, id: Id, reason: Refusal) {
        self.publish(id, Outcome::Refused(reason), None);
    }
    fn admit(&mut self, id: Id) {
        let before = self.monitor.admission(id, self.bounds);
        self.monitor.prepare_admission(before);
        self.admit_impl(id);
        self.monitor.admission_finished(id, before);
    }
    fn admit_impl(&mut self, id: Id) {
        let at = self
            .pending
            .iter()
            .position(|p| p.entry.id == id)
            .expect("pending precondition");
        if self.entry(id).is_some() || self.ledger.iter().any(|t| t.id == id) {
            return;
        }
        if (self.rules == Rules::R5 || self.mutation == Mutation::Deferral)
            && self.pending[at].deferred
        {
            return;
        }
        if self.free() == 0 {
            self.pending[at].deferred = true;
            return;
        }
        let mut e = self.pending[at].entry.clone();
        if self.rules == Rules::Chosen && self.pending[at].cancelled {
            self.publish(id, Outcome::Aborted(Trigger::ParentAborted), None);
            return;
        }
        if let Some(parent) = e.parent {
            match self.status(parent) {
                Status::Live if self.entry(parent).is_some() => {}
                Status::Completed => e.parent = None,
                Status::Expired if self.mutation == Mutation::ExpiredParent => e.parent = None,
                Status::Expired => {
                    self.refuse(id, Refusal::HistoryExpired);
                    return;
                }
                _ => {
                    self.refuse(id, Refusal::ParentNotLive);
                    return;
                }
            }
        }
        let mut victim = None;
        if let Some(parent) = e.parent {
            if e.class.is_session() {
                self.refuse(id, Refusal::Busy);
                return;
            }
            if self
                .chain(parent)
                .iter()
                .any(|id| self.entry(*id).is_some_and(|p| p.group == e.group))
            {
                self.refuse(id, Refusal::Busy);
                return;
            }
            victim = self
                .entries
                .iter()
                .find(|c| {
                    c.parent == Some(parent) && c.class == Class::Capability && c.group == e.group
                })
                .map(|c| c.id);
            if self.mutation == Mutation::ReplacementLabel {
                victim = None;
            }
            if victim.is_none()
                && self
                    .entries
                    .iter()
                    .any(|c| c.parent == Some(parent) && c.origin == Origin::Started)
            {
                self.refuse(id, Refusal::ParentBusy);
                return;
            }
            if self.depth(parent) + 1 > self.bounds.depth {
                self.refuse(id, Refusal::LineageDepth);
                return;
            }
            if victim.is_none() {
                victim = self
                    .entries
                    .iter()
                    .find(|c| {
                        c.class == Class::Capability
                            && c.group == e.group
                            && !self.chain(c.id).contains(&parent)
                    })
                    .map(|c| c.id);
            }
        } else {
            if self.session_root().is_some() {
                self.refuse(id, Refusal::Busy);
                return;
            }
            if e.class == Class::Capability {
                victim = self
                    .entries
                    .iter()
                    .find(|c| c.class == Class::Capability && c.group == e.group)
                    .map(|c| c.id);
            }
            if self.roots() == self.bounds.roots
                && victim.is_none_or(|v| self.entry(v).is_some_and(|c| c.parent.is_some()))
            {
                self.refuse(id, Refusal::Busy);
                return;
            }
        }
        let mut removed = Vec::new();
        if let Some(v) = victim {
            self.descendants(v, &mut removed);
            removed.push(v);
        }
        let projected_live = self.live() - removed.len() + 1;
        let projected_leaves = self
            .entries
            .iter()
            .filter(|old| !removed.contains(&old.id))
            .filter(|old| {
                !self
                    .entries
                    .iter()
                    .any(|c| !removed.contains(&c.id) && c.parent == Some(old.id))
                    && e.parent != Some(old.id)
            })
            .count()
            + 1;
        if (self.rules == Rules::R5 || self.mutation == Mutation::Replacement)
            && self.live() == self.bounds.live
        {
            self.refuse(id, Refusal::Busy);
            return;
        }
        if projected_live > self.bounds.live
            || (self.rules == Rules::Chosen && projected_leaves > self.bounds.roots)
        {
            self.refuse(id, Refusal::Busy);
            return;
        }
        if let Some(v) = victim {
            let mutation = self.mutation;
            if mutation == Mutation::ReplacementCascade {
                self.mutation = Mutation::RevocationCascade(Trigger::Superseded);
            }
            self.revoke(v, Trigger::Superseded);
            self.mutation = mutation;
        }
        e.origin = Origin::Started;
        self.monitor.accepted(id);
        for row in self.queue.iter_mut().chain(&mut self.wire) {
            if row.owner == Some(id) {
                row.awaiting_admission = false;
            }
        }
        self.entries.push(e);
        self.entries.sort_by_key(|e| e.id);
    }
    fn controls(&mut self) {
        let controls = self.carry.clone();
        // Cancellation dominates any copied Decide for the same id.
        for c in controls
            .iter()
            .filter(|c| matches!(c, Control::Cancel { .. }))
            .chain(
                controls
                    .iter()
                    .filter(|c| matches!(c, Control::Decide { .. })),
            )
        {
            match *c {
                Control::Cancel { of, throw } => {
                    if self.entry(of).is_some() {
                        if throw {
                            self.complete(of, Outcome::Failed);
                        } else {
                            self.revoke(of, Trigger::ParentAborted);
                        }
                    } else {
                        self.late_controls += 1;
                    }
                }
                Control::Decide { of, seq, complete } => {
                    if let Some(e) = self
                        .entries
                        .iter_mut()
                        .find(|e| e.id == of && e.prompt && e.decisions + 1 == seq)
                    {
                        e.prompt = false;
                        e.decisions = seq;
                        if complete {
                            self.complete(of, Outcome::Done);
                        }
                    } else {
                        self.late_controls += 1;
                    }
                }
            }
        }
    }
    fn emit(&mut self, of: Option<Id>, release: bool) {
        if release
            && self.rules == Rules::Chosen
            && self
                .queue
                .iter()
                .chain(&self.wire)
                .any(|r| r.release && r.owner == of)
        {
            return;
        }
        let gameplay_count = self.queue.iter().filter(|r| !r.release).count();
        if !release && gameplay_count >= self.bounds.rows {
            self.dropped += 1;
            return;
        }
        let status = of.map(|id| self.status(id));
        let local = of.and_then(|id| self.mirrors.iter().find(|m| m.id == id));
        let lineage = match status {
            Some(Status::Live | Status::Pending) => local.map_or(of, |m| m.ancestor),
            _ => None,
        };
        let mut ancestors = if self.rules == Rules::Chosen && local.is_some() {
            // Until Page rebases a mirror, the retained result preserves its chain.
            let mut ancestors = Vec::new();
            let mut next = lineage;
            while let Some(id) = next {
                ancestors.push(id);
                next = self.entry(id).map_or_else(
                    || {
                        self.ledger
                            .iter()
                            .find(|t| t.id == id)
                            .and_then(|t| t.old_parent)
                    },
                    |entry| entry.parent,
                );
            }
            ancestors
        } else {
            lineage.map_or_else(Vec::new, |id| self.chain(id))
        };
        let pending = of.and_then(|id| {
            self.pending.iter().find(|p| {
                p.entry.id == id
                    && self.entry(id).is_none()
                    && self.ledger.iter().all(|t| t.id != id)
            })
        });
        if let Some(p) = pending {
            ancestors.push(p.entry.id);
            if let Some(parent) = p.entry.parent {
                ancestors.extend(self.chain(parent));
            }
        }
        let session = self
            .session_root()
            .filter(|s| ancestors.contains(s))
            .or_else(|| {
                pending
                    .filter(|p| p.entry.class.is_session() && p.entry.parent.is_none())
                    .map(|p| p.entry.id)
            });
        let ancestor_revoked = self.rules == Rules::Chosen
            && local.is_some()
            && ancestors
                .iter()
                .any(|id| matches!(self.status(*id), Status::Revoked | Status::Expired));
        let awaiting_admission = pending.is_some();
        let witness = self.monitor.emitted(of, release);
        self.queue.push(Emission {
            owner: of,
            lineage,
            ancestors,
            session: if self.mutation == Mutation::SessionCapture {
                None
            } else {
                session
            },
            release,
            revoked: ancestor_revoked || matches!(status, Some(Status::Revoked)),
            dropped: ancestor_revoked || matches!(status, Some(Status::Revoked | Status::Expired)),
            take_filtered: false,
            awaiting_admission,
            witness,
        });
    }
    fn take(&mut self) {
        let cap = if self.rules == Rules::R5 && self.bounds.rows == 256 {
            191
        } else {
            self.bounds.gameplay
        };
        let mut actions = 0;
        let mut queue = std::mem::take(&mut self.queue);
        if self.rules == Rules::Chosen && self.mutation != Mutation::ReleaseDisplacement {
            self.wire.extend(queue.extract_if(.., |row| row.release));
        }
        for mut row in queue {
            if self.rules == Rules::R5
                || self.mutation == Mutation::ReleaseDisplacement
                || !row.release
            {
                actions += 1;
            }
            if actions > cap
                && (self.rules == Rules::R5
                    || self.mutation == Mutation::ReleaseDisplacement
                    || !row.release)
            {
                self.monitor.overflow(row.witness);
                self.dropped += 1;
                continue;
            }
            if !row.release
                && self.rules == Rules::R5
                && let Some(owner) = row.owner
            {
                if self.status(owner) == Status::Revoked {
                    row.take_filtered = true;
                }
                if self.status(owner) == Status::Completed
                    && self.mirrors.iter().all(|m| m.id != owner)
                {
                    row.lineage = None;
                }
            }
            self.wire.push(row);
        }
    }
    fn dispatch(&mut self) {
        let session = self.foreground_session();
        let rows = std::mem::take(&mut self.wire);
        for row in rows {
            if row.release {
                self.monitor.dispatched(row.witness, true);
                self.releases += 1;
                continue;
            }
            let actual = if self.rules == Rules::Chosen {
                !row.revoked
                    && !row.awaiting_admission
                    && !row.dropped
                    && (session.is_none() || row.session == session)
            } else {
                let live_lineage = row.lineage.and_then(|id| self.entry(id));
                let known = row.lineage.is_none()
                    || row.lineage.is_some_and(|id| {
                        self.entry(id).is_some()
                            || self
                                .ledger
                                .iter()
                                .any(|t| t.id == id && !t.outcome.revoked())
                    });
                let authority = live_lineage.map(|e| self.chain(e.id));
                !row.revoked
                    && !row.dropped
                    && !row.take_filtered
                    && known
                    && (session.is_none()
                        || authority.is_some_and(|a| session.is_some_and(|s| a.contains(&s))))
            };
            self.monitor.dispatched(row.witness, actual);
            if actual {
                self.dispatched += 1;
            } else {
                self.dropped += 1;
            }
        }
    }
    fn page(&mut self) {
        let results = self.ledger.clone();
        for t in results {
            if t.seq <= self.consumed_seq {
                continue;
            }
            self.monitor.page_result(t.id);
            if t.authority_revoked {
                let ids: Vec<_> = self
                    .mirrors
                    .iter()
                    .filter(|m| m.ancestor == Some(t.id))
                    .map(|m| m.id)
                    .collect();
                for id in ids {
                    if self.mirrors.iter().any(|m| m.id == id) {
                        self.revoke(id, Trigger::ParentAborted);
                    }
                }
            } else if self.rules == Rules::Chosen {
                // Results are processed in terminal order; follow completed parent chains.
                let mut parent = t.old_parent;
                while let Some(id) = parent {
                    if self.entry(id).is_some() {
                        break;
                    }
                    parent = self
                        .ledger
                        .iter()
                        .find(|r| r.id == id)
                        .and_then(|r| r.old_parent);
                }
                for m in &mut self.mirrors {
                    if m.ancestor == Some(t.id) {
                        m.ancestor = parent;
                    }
                }
            }
            self.settle_local(t.id, t.outcome);
            self.retire(t.id);
            self.consumed_seq = t.seq;
        }
        for e in &self.entries {
            self.pending.retain(|p| p.entry.id != e.id);
            self.carry.retain(|c| !matches!(c, Control::Decide { of, seq, .. } if *of == e.id && *seq <= e.decisions));
        }
    }
    fn watermark(&mut self) {
        let retired: Vec<_> = self
            .ledger
            .iter()
            .filter(|t| t.seq <= self.consumed_seq)
            .map(|t| t.id)
            .collect();
        self.monitor.watermark(&retired);
        self.ledger.retain(|t| t.seq > self.consumed_seq);
    }
    fn discard_rows(&mut self, keep_releases: bool) {
        for rows in [&mut self.queue, &mut self.wire] {
            for row in rows.extract_if(.., |row| !keep_releases || !row.release) {
                self.dropped += usize::from(!row.release);
            }
        }
    }
    fn clear(&mut self, stop: bool) {
        self.discard_rows(false);
        if !stop {
            for id in 1..self.next {
                if self.records[usize::from(id - 1)].settlements == 0 {
                    self.settle_local(id, Outcome::Aborted(Trigger::Reset));
                }
            }
        }
        self.entries.clear();
        self.pending.clear();
        self.mirrors.clear();
        self.ledger.clear();
        self.carry.clear();
        self.queue.clear();
        self.wire.clear();
        self.history.clear();
        self.floor = self.next;
        self.terminal_seq = 0;
        self.consumed_seq = 0;
        self.stopped = stop;
        self.monitor.reset(self.next);
    }
    /// Invalid steps leave the state unchanged. Bounds also bound the explorer's alphabet.
    pub fn apply(&mut self, step: &Step) -> Result<(), String> {
        self.enabled(step)?;
        self.monitor.input(step);
        match *step {
            Step::Start {
                parent,
                local_parent,
                class,
                group,
                bytes,
                hooks,
            } => {
                let id = self.mint();
                self.monitor.start(id, parent, local_parent, class, group);
                let charge =
                    bytes.saturating_add(START_METADATA + usize::from(hooks) * HOOK_CHARGE);
                if self.pending.len() == self.bounds.pending {
                    self.settle_local(id, Outcome::Refused(Refusal::Busy));
                } else if self.rules == Rules::Chosen
                    && (charge > MAX_START_CHARGE || hooks > MAX_HOOKS)
                {
                    self.settle_local(id, Outcome::Refused(Refusal::ArgsTooLarge));
                } else {
                    self.pending.push(Pending {
                        entry: Entry {
                            id,
                            parent,
                            class,
                            group,
                            origin: Origin::Started,
                            prompt: false,
                            decisions: 0,
                            local_parent,
                        },
                        local_parent,
                        bytes,
                        hooks,
                        deferred: false,
                        cancelled: false,
                    });
                }
            }
            Step::Mirror { ancestor, parent } => {
                let id = self.mint();
                self.monitor.mirror(id, ancestor, parent);
                self.mirrors.push(Mirror {
                    id,
                    ancestor,
                    parent,
                });
            }
            Step::Admit(id) => self.admit(id),
            Step::Complete(id) | Step::Throw(id) => {
                let outcome = if matches!(step, Step::Throw(_)) {
                    Outcome::Failed
                } else {
                    Outcome::Done
                };
                if let Some(at) = self.mirrors.iter().position(|m| m.id == id) {
                    self.monitor.local_end(id, false, self.bounds);
                    let parent = self.mirrors.remove(at).parent;
                    for m in &mut self.mirrors {
                        if m.parent == Some(id) {
                            m.parent = parent;
                        }
                    }
                    for e in &mut self.entries {
                        if e.local_parent == Some(id) {
                            e.local_parent = parent;
                        }
                    }
                    for p in &mut self.pending {
                        if p.local_parent == Some(id) {
                            p.local_parent = parent;
                            p.entry.local_parent = parent;
                        }
                    }
                    self.settle_local(id, outcome);
                } else {
                    self.complete(id, outcome);
                }
            }
            Step::Revoke { of, trigger } => {
                if trigger == Trigger::Stop {
                    self.clear(true);
                } else if trigger == Trigger::Reset {
                    self.clear(false);
                } else {
                    self.revoke(of, trigger);
                }
            }
            Step::Cancel { of, throw } => {
                if self.entry(of).is_none()
                    && self.pending.iter().all(|p| p.entry.id != of)
                    && self
                        .ledger
                        .iter()
                        .all(|t| t.id != of || t.seq <= self.consumed_seq)
                {
                    // A copied wire control after terminal receipt cannot revive adapter carry.
                    self.late_controls += 1;
                    return Ok(());
                }
                self.carry.retain(|c| c.id() != of);
                if !throw && self.pending.iter().any(|p| p.entry.id == of) {
                    self.cancel_pending(of);
                } else {
                    self.carry.push(Control::Cancel { of, throw });
                }
            }
            Step::Prompt(id) => {
                self.entries
                    .iter_mut()
                    .find(|e| e.id == id)
                    .expect("live prompt")
                    .prompt = true
            }
            Step::Decide { of, complete } => {
                if self.entry(of).is_none()
                    && self.pending.iter().all(|p| p.entry.id != of)
                    && self
                        .ledger
                        .iter()
                        .all(|t| t.id != of || t.seq <= self.consumed_seq)
                {
                    self.late_controls += 1;
                    return Ok(());
                }
                if !self
                    .carry
                    .iter()
                    .any(|c| matches!(c, Control::Cancel { of: id, .. } if *id == of))
                {
                    self.carry.retain(|c| c.id() != of);
                    let seq = self.entry(of).map_or(1, |e| e.decisions + 1);
                    self.carry.push(Control::Decide { of, seq, complete });
                }
            }
            Step::Controls => self.controls(),
            Step::Emit { of, release } => self.emit(of, release),
            Step::Take => self.take(),
            Step::Dispatch => self.dispatch(),
            Step::Page => self.page(),
            Step::Watermark => self.watermark(),
            Step::AdmitStamped { of, session } => {
                if session == self.session {
                    self.admit(of);
                }
            }
            Step::Batch => {
                self.watermark();
                self.controls();
                if let Err(failure) = self.check() {
                    self.failure = Some(failure);
                }
                let ids: Vec<_> = self.pending.iter().map(|p| p.entry.id).collect();
                for id in ids {
                    self.admit(id);
                    if let Err(failure) = self.check() {
                        self.failure = Some(failure);
                    }
                }
                self.dispatch();
                self.batches += 1;
            }
            Step::Reconnect { keep } => {
                self.session += 1;
                self.discard_rows(keep && self.mutation != Mutation::KeptReleaseDiscard);
                if !keep {
                    self.clear(false);
                }
                self.batches += 1;
            }
            Step::Stop => self.clear(true),
            Step::HeldTick => self.discard_rows(false),
            Step::HostAction { action } => {
                if self.mutation != Mutation::HostActionDrop {
                    let dispatched = if self.mutation == Mutation::HostActionReorder {
                        action ^ 1
                    } else {
                        action
                    };
                    self.monitor.host_dispatched(dispatched);
                    self.host_actions += 1;
                    self.last_host_action = Some(dispatched);
                }
            }
            Step::SessionHeld => {
                if self.mutation != Mutation::GuardianHold {
                    self.held = true;
                }
            }
            Step::SessionResumed => self.held = false,
        }
        self.monitor.finished();
        Ok(())
    }
    fn enabled(&self, step: &Step) -> Result<(), String> {
        if matches!(step, Step::HostAction { .. }) {
            return Ok(());
        }
        if self.held && !matches!(step, Step::SessionResumed | Step::SessionHeld) {
            return Err("guardian holds the seat".into());
        }
        if self.stopped {
            return Err("stopped".into());
        }
        let known = |id: Id| id > 0 && id < self.next;
        let valid = match *step {
            Step::Start {
                parent,
                local_parent,
                ..
            } => {
                self.next <= self.bounds.ids
                    && parent.is_none_or(known)
                    && local_parent.is_none_or(|id| self.mirrors.iter().any(|m| m.id == id))
            }
            Step::Mirror { ancestor, parent } => {
                self.next <= self.bounds.ids
                    && self.mirrors.len() < self.bounds.live
                    && ancestor.is_none_or(|id| self.entry(id).is_some())
                    && parent.is_none_or(|id| self.mirrors.iter().any(|m| m.id == id))
            }
            Step::Admit(id) => self.pending.iter().any(|p| p.entry.id == id),
            Step::AdmitStamped { of, .. } => self.pending.iter().any(|p| p.entry.id == of),
            Step::Complete(id) => {
                self.entry(id).is_some_and(|e| !e.prompt) || self.mirrors.iter().any(|m| m.id == id)
            }
            Step::Throw(id) | Step::Revoke { of: id, .. } => {
                self.entry(id).is_some() || self.mirrors.iter().any(|m| m.id == id)
            }
            Step::Prompt(id) => self.entry(id).is_some_and(|e| !e.prompt && e.decisions < 2),
            Step::Cancel { of, throw } => {
                known(of)
                    && (!throw
                        || self.entry(of).is_some()
                        || self.mirrors.iter().any(|m| m.id == of))
            }
            // A hook cannot decide before its request has admitted. Late copies remain legal.
            Step::Decide { of, .. } => {
                known(of)
                    && (self.entry(of).is_some()
                        || self.ledger.iter().any(|t| t.id == of)
                        || self.pending.iter().all(|p| p.entry.id != of))
            }
            Step::Emit { of, release } => {
                (of.is_none_or(known) || release && of == Some(0))
                    && (!release
                        || self.rules == Rules::R5
                        || self
                            .queue
                            .iter()
                            .chain(&self.wire)
                            .any(|r| r.release && r.owner == of)
                        || of.is_none_or(|id| id == 0)
                        || self
                            .queue
                            .iter()
                            .chain(&self.wire)
                            .filter(|r| r.release && r.owner.is_some_and(|id| id > 0))
                            .count()
                            < self.bounds.live)
            }
            Step::Take => self.wire.is_empty(),
            Step::Batch | Step::Reconnect { .. } => self.batches < self.bounds.batches,
            _ => true,
        };
        if valid {
            Ok(())
        } else {
            Err(format!("disabled: {step:?}"))
        }
    }
    pub fn check(&self) -> Result<(), Violation> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        self.monitor.check(
            self.bounds,
            self.queue.iter().chain(&self.wire).map(|row| row.witness),
        )?;
        let violation = |name, detail| Err(Violation { name, detail });
        if self.held != self.monitor.session_held() {
            return violation(
                "guardian-hold",
                "seat hold differs from observed guardian state".into(),
            );
        }
        if self.live() + self.retained() > self.bounds.credits {
            return violation("credit-bound", self.describe());
        }
        if self.roots() > self.bounds.roots {
            return violation("root-bound", self.describe());
        }
        if self.live() > self.bounds.live {
            return violation("live-bound", self.describe());
        }
        if self.rules == Rules::Chosen && self.leaves() > self.bounds.roots {
            return violation("future-root-reservation", self.describe());
        }
        for e in &self.entries {
            if e.parent.is_some_and(|p| self.entry(p).is_none())
                || self.depth(e.id) > self.bounds.depth
            {
                return violation("forest-depth-parent", self.describe());
            }
        }
        if self.entries.iter().filter(|e| e.class.is_session()).count() > 1 {
            return violation("session-exclusivity", self.describe());
        }
        for m in &self.mirrors {
            if m.ancestor.is_some_and(|id| {
                self.entry(id).is_none() && self.ledger.iter().all(|t| t.id != id)
            }) {
                return violation(
                    "hybrid-ancestry-custody",
                    format!("mirror {} ancestor {:?} unknown", m.id, m.ancestor),
                );
            }
        }
        if self.pending.len() > self.bounds.pending {
            return violation("pending-count", self.describe());
        }
        if self.pending.iter().any(|p| {
            p.bytes
                .saturating_add(START_METADATA + usize::from(p.hooks) * HOOK_CHARGE)
                > MAX_START_CHARGE
                || p.hooks > MAX_HOOKS
        }) {
            return violation("pending-byte-custody", self.describe());
        }
        if self
            .carry
            .iter()
            .filter(|c| matches!(c, Control::Cancel { .. }))
            .count()
            > self.bounds.live + self.bounds.pending
        {
            return violation("cancel-custody-bound", self.describe());
        }
        if self.ledger.windows(2).any(|w| w[0].seq >= w[1].seq) {
            return violation("terminal-sequence", self.describe());
        }
        if self
            .ledger
            .iter()
            .enumerate()
            .any(|(i, t)| self.ledger[..i].iter().any(|old| old.id == t.id))
        {
            return violation("one-terminal-per-id", self.describe());
        }
        if self
            .carry
            .iter()
            .filter(|c| matches!(c, Control::Decide { .. }))
            .count()
            > self.bounds.live
        {
            return violation("decision-custody-bound", self.describe());
        }
        if self
            .queue
            .iter()
            .chain(&self.wire)
            .filter(|r| r.release && r.owner.is_some_and(|id| id > 0))
            .count()
            > self.bounds.live
            && self.rules == Rules::Chosen
        {
            return violation("release-reservation", self.describe());
        }
        if self.queue.iter().filter(|r| !r.release).count() > self.bounds.rows
            || self.wire.iter().filter(|r| !r.release).count() > self.bounds.gameplay
                && self.rules == Rules::Chosen
        {
            return violation("encoder-row-bound", self.describe());
        }
        if self.rules == Rules::Chosen
            && self
                .wire
                .windows(2)
                .any(|rows| !rows[0].release && rows[1].release)
        {
            return violation("encoder-control-priority", self.describe());
        }
        if self.records.iter().any(|r| r.settlements > 1) {
            return violation("settles-once", self.describe());
        }
        Ok(())
    }
    pub fn describe(&self) -> String {
        format!(
            "live={} roots={} leaves={} retained={} pending={} carry={} queue={} wire={} floor={} seq={}/{} entries={:?}",
            self.live(),
            self.roots(),
            self.leaves(),
            self.retained(),
            self.pending(),
            self.carry.len(),
            self.queue.len(),
            self.wire.len(),
            self.floor,
            self.consumed_seq,
            self.terminal_seq,
            self.entries
        )
    }
}

pub struct Trace {
    pub text: String,
    pub failure: Option<Violation>,
    pub final_state: State,
}
pub fn run_trace(rules: Rules, bounds: Bounds, steps: &[Step]) -> Trace {
    let mut state = State::new(rules, bounds);
    let mut text = format!("{rules:?}\n");
    let mut failure = None;
    for (index, step) in steps.iter().enumerate() {
        if let Err(error) = state.apply(step) {
            panic!("trace step {index}: {error}");
        }
        writeln!(text, "{index}: {step:?} => {}", state.describe()).expect("string write");
        if let Err(error) = state.check() {
            writeln!(text, "FAIL {}: {}", error.name, error.detail).expect("string write");
            failure = Some(error);
            break;
        }
    }
    Trace {
        text,
        failure,
        final_state: state,
    }
}

fn alphabet(state: &State) -> Vec<Step> {
    let mut out = vec![
        Step::Controls,
        Step::Take,
        Step::Dispatch,
        Step::Page,
        Step::Watermark,
        Step::Batch,
        Step::Reconnect { keep: true },
        Step::Reconnect { keep: false },
        Step::Stop,
        Step::HeldTick,
        Step::HostAction { action: 0 },
        Step::HostAction { action: 1 },
        Step::SessionHeld,
        Step::SessionResumed,
        Step::Emit {
            of: None,
            release: false,
        },
        Step::Emit {
            of: None,
            release: true,
        },
        Step::Emit {
            of: Some(0),
            release: true,
        },
    ];
    if state.next <= state.bounds.ids {
        let parents = std::iter::once(None).chain((1..state.next).map(Some));
        for parent in parents {
            for (class, group) in [
                (Class::Capability, 0),
                (Class::Capability, 1),
                (Class::Session(SessionFamily::Gather), 2),
                (Class::Session(SessionFamily::Hunt), 2),
                (Class::Session(SessionFamily::Combat), 2),
                (Class::Session(SessionFamily::Progress), 2),
            ] {
                out.push(Step::Start {
                    parent,
                    local_parent: None,
                    class,
                    group,
                    bytes: 1,
                    hooks: 1,
                });
            }
        }
        out.push(Step::Start {
            parent: None,
            local_parent: None,
            class: Class::Capability,
            group: 0,
            bytes: MAX_START_CHARGE,
            hooks: 1,
        });
        out.push(Step::Mirror {
            ancestor: None,
            parent: None,
        });
        for e in &state.entries {
            out.push(Step::Mirror {
                ancestor: Some(e.id),
                parent: None,
            });
        }
        for m in &state.mirrors {
            out.push(Step::Mirror {
                ancestor: m.ancestor,
                parent: Some(m.id),
            });
            out.push(Step::Start {
                parent: m.ancestor,
                local_parent: Some(m.id),
                class: Class::Capability,
                group: 1,
                bytes: 1,
                hooks: 1,
            });
        }
    }
    for id in 1..state.next {
        out.extend([
            Step::Admit(id),
            Step::Complete(id),
            Step::Throw(id),
            Step::Cancel {
                of: id,
                throw: false,
            },
            Step::Cancel {
                of: id,
                throw: true,
            },
            Step::Prompt(id),
            Step::Decide {
                of: id,
                complete: false,
            },
            Step::Decide {
                of: id,
                complete: true,
            },
            Step::Emit {
                of: Some(id),
                release: false,
            },
            Step::Emit {
                of: Some(id),
                release: true,
            },
        ]);
        out.extend([
            Step::AdmitStamped {
                of: id,
                session: state.session,
            },
            Step::AdmitStamped {
                of: id,
                session: state.session + 1,
            },
        ]);
        for trigger in [
            Trigger::Superseded,
            Trigger::ParentAborted,
            Trigger::HostRevoke,
            Trigger::ManualWalk,
            Trigger::ManualCombat,
            Trigger::GatherSignal,
            Trigger::Pause,
            Trigger::Reset,
            Trigger::Stop,
        ] {
            out.push(Step::Revoke { of: id, trigger });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_stamp_is_nonallocating_and_fresh_replay_admits() {
        let mut state = State::new(Rules::Chosen, Bounds::small());
        state
            .apply(&Step::Start {
                parent: None,
                local_parent: None,
                class: Class::Capability,
                group: 0,
                bytes: 1,
                hooks: 0,
            })
            .unwrap();
        state.apply(&Step::Reconnect { keep: true }).unwrap();
        state
            .apply(&Step::AdmitStamped { of: 1, session: 0 })
            .unwrap();
        assert_eq!((state.live(), state.retained(), state.pending()), (0, 0, 1));
        state
            .apply(&Step::AdmitStamped { of: 1, session: 1 })
            .unwrap();
        assert_eq!(state.live(), 1);
        state.check().unwrap();
    }
    #[test]
    fn normative_encoder_and_byte_reservation_arithmetic() {
        let b = Bounds::production();
        assert_eq!(
            CONTROL_RESERVATION,
            b.pending + b.live + b.live + b.pending + 1 + b.live + 2
        );
        assert_eq!(b.rows - CONTROL_RESERVATION, b.gameplay);
        let bytes = MAX_START_CHARGE - START_METADATA - usize::from(MAX_HOOKS) * HOOK_CHARGE;
        let mut state = State::new(Rules::Chosen, b);
        state
            .apply(&Step::Start {
                parent: None,
                local_parent: None,
                class: Class::Capability,
                group: 0,
                bytes,
                hooks: MAX_HOOKS,
            })
            .unwrap();
        state.check().unwrap();
        assert_eq!(state.pending(), 1);
        state
            .apply(&Step::Start {
                parent: None,
                local_parent: None,
                class: Class::Capability,
                group: 1,
                bytes: bytes + 1,
                hooks: MAX_HOOKS,
            })
            .unwrap();
        state.check().unwrap();
        assert_eq!(
            state.outcome(2),
            Some(Outcome::Refused(Refusal::ArgsTooLarge))
        );
        assert_eq!(state.retained(), 0);
    }
    #[test]
    fn pending_session_own_emission_waits_for_admission_authority() {
        let steps = [
            Step::Start {
                parent: None,
                local_parent: None,
                class: Class::Session(SessionFamily::Gather),
                group: 2,
                bytes: 1,
                hooks: 1,
            },
            Step::Emit {
                of: Some(1),
                release: false,
            },
            Step::Take,
            Step::Batch,
        ];
        let trace = run_trace(Rules::Chosen, Bounds::small(), &steps);
        assert!(trace.failure.is_none(), "{}", trace.text);
        assert_eq!(trace.final_state.dispatched(), 1);
    }
    #[test]
    fn chosen_unpaged_mirror_emission_retains_ancestor_authority() {
        for revoked in [false, true] {
            let steps = [
                Step::Start {
                    parent: None,
                    local_parent: None,
                    class: if revoked {
                        Class::Capability
                    } else {
                        Class::Session(SessionFamily::Gather)
                    },
                    group: 2,
                    bytes: 1,
                    hooks: 0,
                },
                Step::Admit(1),
                Step::Start {
                    parent: Some(1),
                    local_parent: None,
                    class: Class::Capability,
                    group: 0,
                    bytes: 1,
                    hooks: 0,
                },
                Step::Admit(2),
                Step::Mirror {
                    ancestor: Some(2),
                    parent: None,
                },
                if revoked {
                    Step::Revoke {
                        of: 2,
                        trigger: Trigger::HostRevoke,
                    }
                } else {
                    Step::Complete(2)
                },
                Step::Emit {
                    of: Some(3),
                    release: false,
                },
                Step::Take,
                Step::Dispatch,
            ];
            let trace = run_trace(Rules::Chosen, Bounds::small(), &steps);
            assert!(trace.failure.is_none(), "{}", trace.text);
            assert_eq!(trace.final_state.dispatched(), usize::from(!revoked));
        }
    }
}
