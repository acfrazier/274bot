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
            .find(|e| e.class == Class::Session)
            .map(|e| e.id)
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
        match *step {
            Step::Cancel { of, .. } if self.entry(of).is_none() => {
                if self.record(of).request.is_some() && self.record(of).outcome.is_none() {
                    self.record_mut(of).cancelled = true;
                }
            }
            Step::Revoke {
                of,
                trigger: Trigger::ManualWalk,
            } => {
                let ids: Vec<_> = self
                    .live
                    .iter()
                    .filter(|e| self.chain(e.id).contains(&of))
                    .map(|e| e.id)
                    .collect();
                for id in ids.into_iter().chain(std::iter::once(of)) {
                    self.record_mut(id).revoked = true;
                }
            }
            _ => {}
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
            };
        };
        let parent = self.parent(request);
        let parent_invalid = request.parent.is_some_and(|p| {
            self.entry(p).is_none()
                && (self.record(p).revoked || (p < self.floor && !self.retained.contains(&p)))
        });
        let mut eligible = credit && !record.cancelled && !parent_invalid;
        let mut victim = None;
        if let Some(parent) = parent {
            eligible &= request.class != Class::Session;
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
        Admission { credit, eligible }
    }
    pub(super) fn admission_finished(&mut self, id: Id, before: Admission) {
        let record = self.record(id);
        if before.eligible && record.outcome == Some(Outcome::Refused(Refusal::Busy)) {
            self.fail(
                "reusable-capacity",
                format!("Busy for independently feasible admission {id}"),
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
        if let Some(at) = live {
            let parent = self.live.remove(at).parent;
            self.record_mut(id).terminal_parent = parent;
            for child in &mut self.live {
                if child.parent == Some(id) {
                    child.parent = parent;
                    if parent.is_some() {
                        child.origin = Origin::Adopted;
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
                            .is_some_and(|r| r.class == Class::Session && r.parent.is_none())
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
                && (self.session().is_none() || row.session == self.session()));
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
    pub(super) fn discarded(&mut self, witnesses: impl Iterator<Item = usize>) {
        for witness in witnesses {
            self.rows[witness] = None;
        }
        self.trim_rows();
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
}
