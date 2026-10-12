//! Specification state reconstructed from boundary observations, not row leases.
use super::{Bounds, Class, Id, Origin, Outcome, Refusal, Step, Trigger, Violation};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Request {
    parent: Option<Id>,
    class: Class,
    group: u8,
    local_parent: Option<Id>,
}
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
struct Record {
    request: Option<Request>,
    admitted: bool,
    cancelled: bool,
    revoked: bool,
    outcome: Option<Outcome>,
    terminal_parent: Option<Id>,
    expected_terminal: Option<Outcome>,
}
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Live {
    id: Id,
    parent: Option<Id>,
    class: Class,
    group: u8,
    origin: Origin,
}
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Mirror {
    id: Id,
    ancestor: Option<Id>,
    parent: Option<Id>,
}
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Row {
    owner: Option<Id>,
    ancestors: Vec<Id>,
    session: Option<Id>,
    blocked: bool,
    pending: bool,
    release: bool,
}
#[derive(Clone, Copy)]
pub(super) struct Admission {
    credit: bool,
    eligible: bool,
    expired_parent: bool,
    victim: Option<Id>,
}
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(super) struct Monitor {
    records: Vec<Record>,
    live: Vec<Live>,
    mirrors: Vec<Mirror>,
    retained: Vec<Id>,
    history: Vec<Id>,
    floor: Id,
    rows: Vec<Option<Row>>,
    held: bool,
    host_action: Option<u8>,
    failure: Option<Violation>,
}
impl Monitor {
    fn fail(&mut self, name: &'static str, detail: String) {
        if self.failure.is_none() {
            self.failure = Some(Violation { name, detail });
        }
    }
    fn record(&self, id: Id) -> &Record {
        &self.records[usize::from(id - 1)]
    }
    fn record_mut(&mut self, id: Id) -> &mut Record {
        &mut self.records[usize::from(id - 1)]
    }
    fn entry(&self, id: Id) -> Option<&Live> {
        self.live.iter().find(|e| e.id == id)
    }
    fn chain(&self, mut id: Id) -> Vec<Id> {
        let mut chain = Vec::new();
        while let Some(e) = self.entry(id) {
            chain.push(id);
            if let Some(parent) = e.parent {
                id = parent;
            } else {
                break;
            }
        }
        chain
    }
    fn session(&self) -> Option<Id> {
        self.live
            .iter()
            .find(|e| e.class.is_session())
            .map(|e| e.id)
    }
    fn foreground(&self) -> Option<Id> {
        self.live
            .iter()
            .find(|e| e.class.owns_foreground())
            .map(|e| e.id)
    }
    pub(super) fn session_held(&self) -> bool {
        self.held
    }
    pub(super) fn minted(&mut self) {
        self.records.push(Record::default());
    }
    pub(super) fn start(
        &mut self,
        id: Id,
        parent: Option<Id>,
        local_parent: Option<Id>,
        class: Class,
        group: u8,
    ) {
        self.record_mut(id).request = Some(Request {
            parent,
            class,
            group,
            local_parent,
        });
    }
    pub(super) fn mirror(&mut self, id: Id, ancestor: Option<Id>, parent: Option<Id>) {
        self.mirrors.push(Mirror {
            id,
            ancestor,
            parent,
        });
    }
    pub(super) fn input(&mut self, step: &Step) {
        if self.held
            && !matches!(
                step,
                Step::HostAction { .. } | Step::SessionHeld | Step::SessionResumed
            )
        {
            self.fail(
                "guardian-hold",
                format!("seat advanced during guardian hold: {step:?}"),
            );
        }
        match *step {
            Step::Cancel { of, .. } if self.entry(of).is_none() => {
                if self.record(of).request.is_some() && self.record(of).outcome.is_none() {
                    self.record_mut(of).cancelled = true;
                }
            }
            Step::Revoke { of, trigger }
                if !matches!(
                    trigger,
                    Trigger::Pause | Trigger::GatherSignal | Trigger::Reset | Trigger::Stop
                ) =>
            {
                self.expect_revocation(of, trigger);
                if trigger == Trigger::ManualWalk {
                    self.record_mut(of).revoked = true;
                }
            }
            Step::HostAction { action } => self.host_action = Some(action),
            Step::SessionHeld => self.held = true,
            Step::SessionResumed => self.held = false,
            _ => {}
        }
        // Only observable discard boundaries can discharge row obligations.
        // The implementation does not supply a list of allegedly discarded rows.
        let discard_releases = matches!(
            step,
            Step::Stop
                | Step::HeldTick
                | Step::Reconnect { keep: false }
                | Step::Revoke {
                    trigger: Trigger::Stop | Trigger::Reset,
                    ..
                }
        );
        let discard_gameplay = discard_releases || matches!(step, Step::Reconnect { keep: true });
        if discard_gameplay {
            for row in &mut self.rows {
                if row.as_ref().is_some_and(|r| discard_releases || !r.release) {
                    *row = None;
                }
            }
            self.trim_rows();
        }
    }
    pub(super) fn host_dispatched(&mut self, action: u8) {
        if self.host_action.take() != Some(action) {
            self.fail(
                "host-action-dispatch",
                format!("host dispatch {action} differs from the next issued action"),
            );
        }
    }
    pub(super) fn finished(&mut self) {
        if let Some(action) = self.host_action.take() {
            self.fail(
                "host-action-dispatch",
                format!("host-issued action {action} was dropped or delayed"),
            );
        }
        if let Some((index, record)) = self
            .records
            .iter()
            .enumerate()
            .find(|(_, r)| r.expected_terminal.is_some())
        {
            self.fail(
                "revocation-cascade",
                format!(
                    "revocation left {} without {:?}",
                    index + 1,
                    record.expected_terminal
                ),
            );
        }
    }
    fn expect_revocation(&mut self, of: Id, trigger: Trigger) {
        let ids: Vec<_> = self
            .live
            .iter()
            .filter(|e| self.chain(e.id).contains(&of))
            .map(|e| e.id)
            .collect();
        for id in ids {
            let record = self.record_mut(id);
            record.revoked = true;
            record.expected_terminal = Some(if id == of && trigger == Trigger::ManualWalk {
                Outcome::Done
            } else {
                Outcome::Aborted(if id == of {
                    trigger
                } else {
                    Trigger::ParentAborted
                })
            });
        }
    }
    pub(super) fn prepare_admission(&mut self, before: Admission) {
        if before.eligible
            && let Some(victim) = before.victim
        {
            self.expect_revocation(victim, Trigger::Superseded);
        }
    }
    fn parent(&self, request: &Request) -> Option<Id> {
        request.parent.filter(|id| self.entry(*id).is_some())
    }
    pub(super) fn admission(&self, id: Id, bounds: Bounds) -> Admission {
        let record = self.record(id);
        let duplicate = self.entry(id).is_some() || self.retained.contains(&id);
        let credit = !duplicate && self.live.len() + self.retained.len() < bounds.credits;
        let Some(request) = record.request.as_ref() else {
            return Admission {
                credit: false,
                eligible: false,
                expired_parent: false,
                victim: None,
            };
        };
        let parent = self.parent(request);
        let parent_invalid = request.parent.is_some_and(|p| {
            self.entry(p).is_none()
                && (self.record(p).revoked
                    || self.mirrors.iter().any(|m| m.id == p)
                    || (self.record(p).request.is_some()
                        && !self.record(p).admitted
                        && self.record(p).outcome.is_none())
                    || (p < self.floor && !self.retained.contains(&p)))
        });
        let expired_parent = credit
            && !record.cancelled
            && request.parent.is_some_and(|p| {
                self.entry(p).is_none()
                    && !self.mirrors.iter().any(|m| m.id == p)
                    && !(self.record(p).request.is_some()
                        && !self.record(p).admitted
                        && self.record(p).outcome.is_none())
                    && !self.retained.contains(&p)
                    && !self.history.contains(&p)
                    && p < self.floor
            });
        let mut eligible = credit && !record.cancelled && !parent_invalid;
        let mut victim = None;
        if let Some(parent) = parent {
            eligible &= !request.class.is_session();
            eligible &= !self
                .chain(parent)
                .iter()
                .any(|p| self.entry(*p).is_some_and(|e| e.group == request.group));
            victim = self
                .live
                .iter()
                .find(|e| {
                    e.parent == Some(parent)
                        && e.class == Class::Capability
                        && e.group == request.group
                })
                .map(|e| e.id);
            eligible &= victim.is_some()
                || !self
                    .live
                    .iter()
                    .any(|e| e.parent == Some(parent) && e.origin == Origin::Started);
            eligible &= self.chain(parent).len() < bounds.depth;
            if victim.is_none() {
                victim = self
                    .live
                    .iter()
                    .find(|e| {
                        e.class == Class::Capability
                            && e.group == request.group
                            && !self.chain(e.id).contains(&parent)
                    })
                    .map(|e| e.id);
            }
        } else {
            eligible &= self.session().is_none();
            if request.class == Class::Capability {
                victim = self
                    .live
                    .iter()
                    .find(|e| e.class == Class::Capability && e.group == request.group)
                    .map(|e| e.id);
            }
            let roots = self.live.iter().filter(|e| e.parent.is_none()).count();
            eligible &= roots < bounds.roots
                || victim.is_some_and(|v| self.entry(v).is_some_and(|e| e.parent.is_none()));
        }
        let survivors: Vec<_> = self
            .live
            .iter()
            .filter(|e| victim.is_none_or(|v| !self.chain(e.id).contains(&v)))
            .collect();
        let leaves = survivors
            .iter()
            .filter(|e| parent != Some(e.id) && !survivors.iter().any(|c| c.parent == Some(e.id)))
            .count()
            + 1;
        eligible &= survivors.len() < bounds.live && leaves <= bounds.roots;
        Admission {
            credit,
            eligible,
            expired_parent,
            victim,
        }
    }
    pub(super) fn admission_finished(&mut self, id: Id, before: Admission) {
        let record = self.record(id);
        if before.expired_parent
            && record.outcome != Some(Outcome::Refused(Refusal::HistoryExpired))
        {
            self.fail(
                "history-expired-parent",
                format!("expired parent admission {id} did not refuse HistoryExpired"),
            );
        } else if before.eligible && !record.admitted {
            self.fail(
                "reusable-capacity",
                format!("independently feasible admission {id} did not admit"),
            );
        } else if before.credit && !record.admitted && !self.retained.contains(&id) {
            self.fail(
                "admission-progress",
                format!("start {id} ignored despite an available credit"),
            );
        }
    }
    pub(super) fn accepted(&mut self, id: Id) {
        let request = self
            .record(id)
            .request
            .as_ref()
            .expect("observed Start before acceptance")
            .clone();
        let parent = self.parent(&request);
        if self.record(id).cancelled {
            self.fail(
                "cancel-before-start",
                format!("cancelled unadmitted start {id} accepted"),
            );
        }
        self.record_mut(id).admitted = true;
        self.live.push(Live {
            id,
            parent,
            class: request.class,
            group: request.group,
            origin: Origin::Started,
        });
    }
    pub(super) fn published(&mut self, id: Id, outcome: Outcome, bounds: Bounds) {
        let live = self.live.iter().position(|e| e.id == id);
        if live.is_none() && self.live.len() + self.retained.len() >= bounds.credits {
            self.fail(
                "terminal-credit",
                format!("terminal {id} published without an observed credit"),
            );
        }
        if self.record(id).outcome.is_some() {
            self.fail("one-terminal-per-id", format!("second terminal for {id}"));
        }
        if self
            .record(id)
            .expected_terminal
            .is_some_and(|expected| expected != outcome)
        {
            self.fail(
                "revocation-cascade",
                format!("revoked subtree entry {id} published {outcome:?} instead of its required terminal"),
            );
        }
        self.record_mut(id).expected_terminal = None;
        let revoked = outcome.revoked() || self.record(id).revoked;
        if live.is_some() && revoked && self.live.iter().any(|child| child.parent == Some(id)) {
            self.fail(
                "revocation-cascade",
                format!("revoked entry {id} published before its live descendants settled"),
            );
        }
        if let Some(at) = live {
            let parent = self.live.remove(at).parent;
            self.record_mut(id).terminal_parent = parent;
            if !revoked {
                for child in &mut self.live {
                    if child.parent == Some(id) {
                        child.parent = parent;
                        if parent.is_some() {
                            child.origin = Origin::Adopted;
                        }
                    }
                }
            }
        }
        self.record_mut(id).outcome = Some(outcome);
        self.retained.push(id);
        if outcome.revoked() || self.record(id).revoked {
            self.record_mut(id).revoked = true;
            self.remember(id, bounds);
        }
    }
    pub(super) fn history_report(&mut self, id: Id) {
        if matches!(
            self.record(id).outcome,
            Some(Outcome::Done | Outcome::Failed)
        ) && !self.record(id).revoked
        {
            self.fail(
                "ordinary-completion-not-revocation",
                format!("ordinary terminal {id} reported revoked"),
            );
        }
    }
    fn remember(&mut self, id: Id, bounds: Bounds) {
        if id >= self.floor && !self.history.contains(&id) {
            self.history.push(id);
            self.history.sort_unstable();
            if self.history.len() > bounds.history {
                self.floor = self.history.remove(0) + 1;
            }
        }
    }
    pub(super) fn page_result(&mut self, id: Id) {
        if self.record(id).revoked {
            return;
        }
        let mut parent = self.record(id).terminal_parent;
        while let Some(id) = parent {
            if self.entry(id).is_some() {
                break;
            }
            parent = self.record(id).terminal_parent;
        }
        for mirror in &mut self.mirrors {
            if mirror.ancestor == Some(id) {
                mirror.ancestor = parent;
            }
        }
    }
    pub(super) fn settled(&mut self, id: Id, outcome: Outcome) {
        if self.record(id).outcome.is_none() {
            self.record_mut(id).outcome = Some(outcome);
        }
    }
    pub(super) fn local_end(&mut self, id: Id, revoked: bool, bounds: Bounds) {
        let Some(at) = self.mirrors.iter().position(|m| m.id == id) else {
            return;
        };
        let parent = self.mirrors.remove(at).parent;
        if revoked {
            self.record_mut(id).revoked = true;
            self.remember(id, bounds);
            let pending: Vec<_> = self
                .records
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    r.request
                        .as_ref()
                        .is_some_and(|p| p.local_parent == Some(id))
                        && r.outcome.is_none()
                })
                .map(|(i, _)| (i + 1) as Id)
                .collect();
            for child in pending {
                if !self.record(child).admitted {
                    self.record_mut(child).cancelled = true;
                }
            }
        } else {
            for mirror in &mut self.mirrors {
                if mirror.parent == Some(id) {
                    mirror.parent = parent;
                }
            }
            for record in &mut self.records {
                if let Some(request) = &mut record.request
                    && request.local_parent == Some(id)
                {
                    request.local_parent = parent;
                }
            }
        }
    }
    pub(super) fn watermark(&mut self, retired: &[Id]) {
        self.retained.retain(|id| !retired.contains(id));
    }
    pub(super) fn emitted(&mut self, owner: Option<Id>, release: bool) -> usize {
        let owner_record = owner.filter(|id| *id > 0).map(|id| self.record(id));
        let mirror = owner.and_then(|id| self.mirrors.iter().find(|m| m.id == id));
        let live = owner.is_some_and(|id| self.entry(id).is_some()) || mirror.is_some();
        let pending =
            owner_record.is_some_and(|r| r.request.is_some() && !r.admitted && r.outcome.is_none());
        let mut ancestors = if live {
            mirror.map_or_else(
                || owner.map_or_else(Vec::new, |id| self.chain(id)),
                |m| {
                    let mut ancestors = Vec::new();
                    let mut next = m.ancestor;
                    while let Some(id) = next {
                        ancestors.push(id);
                        next = self
                            .entry(id)
                            .map_or(self.record(id).terminal_parent, |e| e.parent);
                    }
                    ancestors
                },
            )
        } else {
            Vec::new()
        };
        if pending {
            let id = owner.expect("pending owner");
            ancestors.push(id);
            if let Some(parent) = self.record(id).request.as_ref().and_then(|r| r.parent) {
                ancestors.extend(self.chain(parent));
            }
        }
        let session = self
            .session()
            .filter(|s| ancestors.contains(s))
            .or_else(|| {
                owner.filter(|id| {
                    pending
                        && self
                            .record(*id)
                            .request
                            .as_ref()
                            .is_some_and(|r| r.class.is_session() && r.parent.is_none())
                })
            });
        let blocked = !live
            && !pending
            && owner.filter(|id| *id > 0).is_some_and(|id| {
                self.record(id).revoked || (id < self.floor && !self.retained.contains(&id))
            });
        let row = Row {
            owner,
            ancestors,
            session,
            blocked,
            pending,
            release,
        };
        if let Some(index) = self.rows.iter().position(Option::is_none) {
            self.rows[index] = Some(row);
            index
        } else {
            self.rows.push(Some(row));
            self.rows.len() - 1
        }
    }
    pub(super) fn overflow(&mut self, witness: usize) {
        if self.rows[witness].as_ref().is_some_and(|r| !r.release) {
            self.rows[witness] = None;
            self.trim_rows();
        }
    }
    pub(super) fn dispatched(&mut self, witness: usize, actual: bool) {
        let row = self.rows[witness]
            .take()
            .expect("dispatch of an observed row");
        let expected = row.release
            || (!row.blocked
                && !row
                    .owner
                    .filter(|id| *id > 0)
                    .into_iter()
                    .chain(row.ancestors.iter().copied())
                    .any(|id| self.record(id).revoked)
                && (!row.pending || row.owner.is_some_and(|id| self.record(id).admitted))
                && (self.foreground().is_none() || row.session == self.foreground()));
        if expected != actual {
            self.fail(
                if row.release {
                    "encoder-release"
                } else {
                    "accepted-emission-authority"
                },
                format!(
                    "owner {:?}: independently expected {expected}, observed dispatch {actual}",
                    row.owner
                ),
            );
        }
        self.trim_rows();
    }
    fn trim_rows(&mut self) {
        while self.rows.last().is_some_and(Option::is_none) {
            self.rows.pop();
        }
    }
    pub(super) fn reset(&mut self, floor: Id) {
        self.live.clear();
        self.mirrors.clear();
        self.retained.clear();
        self.history.clear();
        self.floor = floor;
        for r in &mut self.records {
            r.request = None;
        }
    }
    pub(super) fn allocated_bytes(&self) -> usize {
        self.records.capacity() * std::mem::size_of::<Record>()
            + self.live.capacity() * std::mem::size_of::<Live>()
            + self.mirrors.capacity() * std::mem::size_of::<Mirror>()
            + (self.retained.capacity() + self.history.capacity()) * std::mem::size_of::<Id>()
            + self.rows.capacity() * std::mem::size_of::<Option<Row>>()
            + self
                .rows
                .iter()
                .flatten()
                .map(|r| r.ancestors.capacity() * std::mem::size_of::<Id>())
                .sum::<usize>()
            + self.failure.as_ref().map_or(0, |f| f.detail.capacity())
    }
    pub(super) fn check(
        &self,
        bounds: Bounds,
        held: impl Iterator<Item = usize> + Clone,
    ) -> Result<(), Violation> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        let fail = |name, detail| Err(Violation { name, detail });
        if self.live.len() + self.retained.len() > bounds.credits {
            return fail(
                "credit-bound",
                "observed accept/terminal credits exceed bound".into(),
            );
        }
        let roots = self.live.iter().filter(|e| e.parent.is_none()).count();
        if roots > bounds.roots {
            return fail("root-bound", format!("observed forest has {roots} roots"));
        }
        if self.live.len() > bounds.live {
            return fail("live-bound", "observed accept count exceeds bound".into());
        }
        if self.live.iter().any(|e| {
            self.chain(e.id).len() > bounds.depth
                || e.parent.is_some_and(|p| self.entry(p).is_none())
        }) {
            return fail(
                "forest-depth-parent",
                "observed ancestry violates bounds".into(),
            );
        }
        if self.rows.iter().enumerate().any(|(index, row)| {
            row.as_ref().is_some_and(|r| r.release) && !held.clone().any(|witness| witness == index)
        }) {
            return fail(
                "encoder-release",
                "emitted release absent from held rows, dispatch, and explicit discard".into(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(monitor: &mut Monitor, id: Id) {
        monitor.minted();
        monitor.start(id, None, None, Class::Capability, id as u8);
        monitor.accepted(id);
    }

    #[test]
    fn observed_publication_requires_a_credit() {
        let bounds = Bounds {
            credits: 1,
            ..Bounds::small()
        };
        let mut monitor = Monitor::default();
        root(&mut monitor, 1);
        monitor.minted();
        monitor.published(2, Outcome::Refused(Refusal::Busy), bounds);
        assert_eq!(
            monitor.check(bounds, std::iter::empty()).unwrap_err().name,
            "terminal-credit"
        );
    }

    #[test]
    fn observed_forest_and_cancel_do_not_need_implementation_fields() {
        let bounds = Bounds {
            roots: 1,
            ..Bounds::small()
        };
        let mut monitor = Monitor::default();
        root(&mut monitor, 1);
        root(&mut monitor, 2);
        assert_eq!(
            monitor.check(bounds, std::iter::empty()).unwrap_err().name,
            "root-bound"
        );
        let mut monitor = Monitor::default();
        monitor.minted();
        monitor.start(1, None, None, Class::Capability, 0);
        monitor.input(&Step::Cancel {
            of: 1,
            throw: false,
        });
        monitor.accepted(1);
        assert_eq!(
            monitor.check(bounds, std::iter::empty()).unwrap_err().name,
            "cancel-before-start"
        );
    }

    #[test]
    fn ordinary_completion_cannot_be_reported_as_revocation() {
        let mut monitor = Monitor::default();
        root(&mut monitor, 1);
        monitor.published(1, Outcome::Done, Bounds::small());
        monitor.history_report(1);
        assert_eq!(
            monitor
                .check(Bounds::small(), std::iter::empty())
                .unwrap_err()
                .name,
            "ordinary-completion-not-revocation"
        );
    }
    #[test]
    fn feasible_admission_requires_acceptance_under_every_refusal_label() {
        for refusal in [
            Refusal::Busy,
            Refusal::ParentBusy,
            Refusal::ParentNotLive,
            Refusal::LineageDepth,
            Refusal::HistoryExpired,
        ] {
            let mut monitor = Monitor::default();
            monitor.minted();
            monitor.start(1, None, None, Class::Capability, 0);
            let before = monitor.admission(1, Bounds::small());
            monitor.published(1, Outcome::Refused(refusal), Bounds::small());
            monitor.admission_finished(1, before);
            assert_eq!(
                monitor
                    .check(Bounds::small(), std::iter::empty())
                    .unwrap_err()
                    .name,
                "reusable-capacity",
            );
        }
    }

    #[test]
    fn release_discard_requires_an_observed_discard_boundary() {
        for step in [
            Step::Reconnect { keep: true },
            Step::SessionHeld,
            Step::SessionResumed,
            Step::Dispatch,
        ] {
            let mut monitor = Monitor::default();
            monitor.emitted(None, true);
            monitor.input(&step);
            assert_eq!(
                monitor
                    .check(Bounds::small(), std::iter::empty())
                    .unwrap_err()
                    .name,
                "encoder-release",
            );
        }
        for step in [Step::Stop, Step::HeldTick, Step::Reconnect { keep: false }] {
            let mut monitor = Monitor::default();
            monitor.emitted(None, true);
            monitor.input(&step);
            monitor.check(Bounds::small(), std::iter::empty()).unwrap();
        }
    }

    #[test]
    fn revoked_publication_cannot_promote_a_live_child() {
        for trigger in [
            Trigger::HostRevoke,
            Trigger::Superseded,
            Trigger::ParentAborted,
        ] {
            let mut monitor = Monitor::default();
            root(&mut monitor, 1);
            monitor.minted();
            monitor.start(2, Some(1), None, Class::Capability, 2);
            monitor.accepted(2);
            monitor.published(1, Outcome::Aborted(trigger), Bounds::small());
            assert_eq!(
                monitor
                    .check(Bounds::small(), std::iter::empty())
                    .unwrap_err()
                    .name,
                "revocation-cascade",
            );
        }
    }

    #[test]
    fn host_action_observation_requires_immediate_ordered_dispatch() {
        for dispatched in [None, Some(1)] {
            let mut monitor = Monitor::default();
            monitor.input(&Step::SessionHeld);
            monitor.input(&Step::HostAction { action: 0 });
            if let Some(action) = dispatched {
                monitor.host_dispatched(action);
            }
            monitor.finished();
            assert_eq!(
                monitor
                    .check(Bounds::small(), std::iter::empty())
                    .unwrap_err()
                    .name,
                "host-action-dispatch",
            );
        }
    }
    #[test]
    fn expired_parent_cannot_be_reinterpreted_as_a_completed_root() {
        let bounds = Bounds {
            history: 1,
            ..Bounds::small()
        };
        let mut monitor = Monitor::default();
        root(&mut monitor, 1);
        monitor.published(1, Outcome::Done, bounds);
        monitor.watermark(&[1]);
        for id in [2, 3] {
            root(&mut monitor, id);
            monitor.published(id, Outcome::Aborted(Trigger::HostRevoke), bounds);
            monitor.watermark(&[id]);
        }
        monitor.minted();
        monitor.start(4, Some(1), None, Class::Capability, 0);
        let before = monitor.admission(4, bounds);
        monitor.accepted(4);
        monitor.admission_finished(4, before);
        assert_eq!(
            monitor.check(bounds, std::iter::empty()).unwrap_err().name,
            "history-expired-parent",
        );
    }
    #[test]
    fn pending_and_local_mirror_are_not_live_seat_parents() {
        for mirror in [false, true] {
            let mut monitor = Monitor::default();
            monitor.minted();
            if mirror {
                monitor.mirror(1, None, None);
            } else {
                monitor.start(1, None, None, Class::Capability, 0);
            }
            monitor.minted();
            monitor.start(2, Some(1), None, Class::Capability, 1);
            let before = monitor.admission(2, Bounds::small());
            monitor.published(2, Outcome::Refused(Refusal::ParentNotLive), Bounds::small());
            monitor.admission_finished(2, before);
            monitor.check(Bounds::small(), std::iter::empty()).unwrap();
        }
    }
}
