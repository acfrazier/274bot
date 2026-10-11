//! Bounded conformance oracle. No product crate depends on this crate.
//!
//! `records` and dispatch counters are specification monitors, not authority
//! storage. Authority uses live entries, retained results, history and the
//! bounded leases embedded in queued rows. See synthesis/28-seat-sg.md.
use std::collections::{HashSet, VecDeque};
use std::fmt::Write;
use std::rc::Rc;
use std::time::{Duration, Instant};

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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Class {
    Capability,
    Session,
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
}
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Record {
    outcome: Option<Outcome>,
    settlements: u8,
    cancelled_before_admit: bool,
    revoked: bool,
    admitted: bool,
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
    stopped: bool,
    failure: Option<Violation>,
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
            stopped: false,
            failure: None,
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
            .find(|e| e.class == Class::Session)
            .map(|e| e.id)
    }
    fn mint(&mut self) -> Id {
        let id = self.next;
        self.next += 1;
        self.records.push(Record {
            outcome: None,
            settlements: 0,
            cancelled_before_admit: false,
            revoked: false,
            admitted: false,
        });
        id
    }
    fn remember(&mut self, id: Id) {
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
        let r = &mut self.records[usize::from(id - 1)];
        r.settlements = r.settlements.saturating_add(1);
        if r.outcome.is_none() {
            r.outcome = Some(outcome);
        }
    }
    fn publish(&mut self, id: Id, outcome: Outcome, old_parent: Option<Id>) {
        // Caller owns either an entry's converted credit or a fresh credit.
        if self.live() + self.retained() >= self.bounds.credits {
            self.fail("terminal-credit", format!("publish {id} without a credit"));
        }
        self.terminal_seq += 1;
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
    fn fail(&mut self, name: &'static str, detail: String) {
        if self.failure.is_none() {
            self.failure = Some(Violation { name, detail });
        }
    }
    fn retire(&mut self, id: Id) {
        self.pending.retain(|p| p.entry.id != id);
        self.carry.retain(|c| c.id() != id);
    }
    fn revoke_leases(&mut self, id: Id) {
        self.records[usize::from(id - 1)].revoked = true;
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
            if self.entry(id).is_none() {
                self.records[usize::from(id - 1)].cancelled_before_admit = true;
            }
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
        self.descendants(id, &mut ids);
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
        let at = self
            .pending
            .iter()
            .position(|p| p.entry.id == id)
            .expect("pending precondition");
        if self.entry(id).is_some() || self.ledger.iter().any(|t| t.id == id) {
            return;
        }
        if self.rules == Rules::R5 && self.pending[at].deferred {
            if self.free() > 0 {
                self.fail(
                    "admission-progress",
                    format!("deferred {id} replay ignored with a free credit"),
                );
            }
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
            if e.class == Class::Session {
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
        if self.rules == Rules::R5 && self.live() == self.bounds.live {
            if victim.is_some() && projected_live <= self.bounds.live {
                self.fail(
                    "reusable-capacity",
                    format!("replacement {id} refused at full live capacity"),
                );
            }
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
            self.revoke(v, Trigger::Superseded);
        }
        e.origin = Origin::Started;
        self.records[usize::from(id - 1)].admitted = true;
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
        let mut ancestors = lineage.map_or_else(Vec::new, |id| self.chain(id));
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
                    .filter(|p| p.entry.class == Class::Session && p.entry.parent.is_none())
                    .map(|p| p.entry.id)
            });
        self.queue.push(Emission {
            owner: of,
            lineage,
            ancestors,
            session,
            release,
            revoked: matches!(status, Some(Status::Revoked)),
            dropped: matches!(status, Some(Status::Revoked | Status::Expired)),
            take_filtered: false,
            awaiting_admission: pending.is_some(),
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
        if self.rules == Rules::Chosen {
            self.wire.extend(queue.extract_if(.., |row| row.release));
        }
        for mut row in queue {
            if self.rules == Rules::R5 || !row.release {
                actions += 1;
            }
            if actions > cap && (self.rules == Rules::R5 || !row.release) {
                if row.release {
                    self.fail(
                        "encoder-release",
                        "release displaced by gameplay overflow".into(),
                    );
                }
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
        let session = self.session_root();
        let rows = std::mem::take(&mut self.wire);
        for row in rows {
            if row.release {
                self.releases += 1;
                continue;
            }
            let monitor_revoked = row
                .owner
                .into_iter()
                .chain(row.ancestors.iter().copied())
                .any(|id| self.records[usize::from(id - 1)].revoked);
            let monitor_unadmitted = row.awaiting_admission
                && row
                    .owner
                    .is_some_and(|id| !self.records[usize::from(id - 1)].admitted);
            let expected = !monitor_revoked
                && !monitor_unadmitted
                && !row.dropped
                && (session.is_none() || row.session == session);
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
            if actual != expected {
                self.fail(
                    "accepted-emission-authority",
                    format!(
                        "owner {:?}: expected {expected}, dispatched {actual}",
                        row.owner
                    ),
                );
            }
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
    fn clear(&mut self, stop: bool) {
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
    }
    /// Invalid steps leave the state unchanged. Bounds also bound the explorer's alphabet.
    pub fn apply(&mut self, step: &Step) -> Result<(), String> {
        self.enabled(step)?;
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
            Step::Watermark => self.ledger.retain(|t| t.seq > self.consumed_seq),
            Step::AdmitStamped { of, session } => {
                if session == self.session {
                    self.admit(of);
                }
            }
            Step::Batch => {
                self.ledger.retain(|t| t.seq > self.consumed_seq);
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
                self.queue.clear();
                self.wire.clear();
                if !keep {
                    self.clear(false);
                }
                self.batches += 1;
            }
            Step::Stop => self.clear(true),
        }
        Ok(())
    }
    fn enabled(&self, step: &Step) -> Result<(), String> {
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
            Step::Cancel { of, .. } | Step::Decide { of, .. } => known(of),
            Step::Emit { of, release } => {
                of.is_none_or(known)
                    && (!release
                        || self.rules == Rules::R5
                        || self
                            .queue
                            .iter()
                            .chain(&self.wire)
                            .any(|r| r.release && r.owner == of)
                        || self
                            .queue
                            .iter()
                            .chain(&self.wire)
                            .filter(|r| r.release)
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
        let violation = |name, detail| Err(Violation { name, detail });
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
            if self.records[usize::from(e.id - 1)].cancelled_before_admit {
                return violation(
                    "cancel-before-start",
                    format!("cancelled pending {} admitted", e.id),
                );
            }
        }
        if self
            .entries
            .iter()
            .filter(|e| e.class == Class::Session)
            .count()
            > 1
        {
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
        if self.ledger.iter().any(|t| {
            matches!(t.outcome, Outcome::Done | Outcome::Failed)
                && !self.records[usize::from(t.id - 1)].revoked
                && self.history.contains(&t.id)
        }) {
            return violation("ordinary-completion-not-revocation", self.describe());
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
            .filter(|r| r.release)
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

#[derive(Clone, Debug)]
pub struct Exploration {
    pub states: usize,
    pub transitions: usize,
    pub max_frontier: usize,
    pub elapsed: Duration,
    pub failure: Option<(Violation, Vec<Step>)>,
}
/// Complete BFS of all enabled actions in this finite alphabet, through `steps`.
/// IDs, batches, payload classes and decision sequences are explicit bounds.
pub fn explore(bounds: Bounds, steps: usize) -> Exploration {
    let start = Instant::now();
    let initial = Rc::new(State::new(Rules::Chosen, bounds));
    let mut seen = HashSet::from([Rc::clone(&initial)]);
    let mut queue = VecDeque::from([(initial, Vec::<Step>::new())]);
    let mut transitions = 0;
    let mut max_frontier = 1;
    while let Some((state, path)) = queue.pop_front() {
        if path.len() == steps || state.stopped {
            continue;
        }
        for step in alphabet(&state) {
            let mut next = (*state).clone();
            if next.apply(&step).is_err() {
                continue;
            }
            transitions += 1;
            if let Err(failure) = next.check() {
                let mut trace = path.clone();
                trace.push(step);
                return Exploration {
                    states: seen.len(),
                    transitions,
                    max_frontier,
                    elapsed: start.elapsed(),
                    failure: Some((failure, trace)),
                };
            }
            // Observation counters do not affect enabled actions or authority.
            next.dispatched = 0;
            next.releases = 0;
            next.late_controls = 0;
            next.dropped = 0;
            if !seen.contains(&next) {
                let next = Rc::new(next);
                seen.insert(Rc::clone(&next));
                let mut trace = path.clone();
                trace.push(step);
                queue.push_back((next, trace));
            }
        }
        max_frontier = max_frontier.max(queue.len());
    }
    Exploration {
        states: seen.len(),
        transitions,
        max_frontier,
        elapsed: start.elapsed(),
        failure: None,
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
        Step::Emit {
            of: None,
            release: false,
        },
        Step::Emit {
            of: None,
            release: true,
        },
    ];
    if state.next <= state.bounds.ids {
        let parents = std::iter::once(None).chain((1..state.next).map(Some));
        for parent in parents {
            for (class, group) in [
                (Class::Capability, 0),
                (Class::Capability, 1),
                (Class::Session, 2),
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
    fn chosen_exhaustive_small() {
        let result = explore(Bounds::small(), 6);
        println!(
            "states={} transitions={} frontier={} elapsed={:?}",
            result.states, result.transitions, result.max_frontier, result.elapsed
        );
        assert!(result.failure.is_none(), "{:?}", result.failure);
    }
    #[test]
    fn chosen_exhaustive_protocol_closure() {
        let result = explore(Bounds::closure(), usize::MAX);
        println!(
            "closure states={} transitions={} frontier={} elapsed={:?}",
            result.states, result.transitions, result.max_frontier, result.elapsed
        );
        assert!(result.failure.is_none(), "{:?}", result.failure);
    }
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
                class: Class::Session,
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
}
