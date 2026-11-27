//! Versioned account facts on the process writer's single SQLite connection.

use std::sync::Weak;

use api::local_state::{
    validate_knowledge_payload, KnowledgeKey, KnowledgeProvenance, KnowledgeRecord,
    KnowledgeRevocation, KnowledgeStatus, MAX_KNOWLEDGE_RECORDS, MAX_KNOWLEDGE_STORES,
};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

use super::sql::{self, Commit, Db, Loaded, SqlFailure, Staging, WaitCause};
use super::*;

const DOMAIN: LocalStateDomain = LocalStateDomain::Knowledge;

pub(super) const SCHEMA: &str = "
CREATE TABLE knowledge_fact (
    profile TEXT NOT NULL CHECK (length(profile) BETWEEN 1 AND 64),
    account TEXT NOT NULL CHECK (length(account) BETWEEN 1 AND 12),
    revision INTEGER NOT NULL,
    content TEXT NOT NULL CHECK (length(content) BETWEEN 1 AND 128),
    key TEXT NOT NULL CHECK (length(key) BETWEEN 1 AND 128),
    schema INTEGER NOT NULL CHECK (schema = 1),
    provenance INTEGER NOT NULL CHECK (provenance BETWEEN 0 AND 3),
    observed_stamp BLOB NOT NULL CHECK (length(observed_stamp) = 8),
    status INTEGER NOT NULL CHECK (status BETWEEN 0 AND 2),
    revocation INTEGER CHECK (revocation BETWEEN 0 AND 2),
    expected_version BLOB NOT NULL CHECK (length(expected_version) = 8),
    version BLOB NOT NULL CHECK (length(version) = 8),
    transaction_sequence BLOB NOT NULL CHECK (length(transaction_sequence) = 8),
    payload BLOB NOT NULL CHECK (length(payload) <= 4096),
    CHECK (revocation IS NULL OR (status = 2 AND length(payload) = 0)),
    PRIMARY KEY (profile, account, revision, content, key)
) STRICT, WITHOUT ROWID;
";

/// One committed fact. Its ordering comes only from the SQL write transaction.
#[derive(Clone, Debug)]
pub struct KnowledgeFact {
    record: Arc<KnowledgeRecord>,
    version: StoreVersion,
    sequence: u64,
}

impl KnowledgeFact {
    /// The exact submitted fact and its evidence metadata.
    pub fn record(&self) -> &KnowledgeRecord {
        &self.record
    }

    /// The expected version for a replacement of this fact.
    pub fn version(&self) -> StoreVersion {
        self.version
    }

    /// The root-wide monotonic sequence assigned by its committing transaction.
    pub fn transaction_sequence(&self) -> u64 {
        self.sequence
    }
}

#[derive(Debug)]
struct Rows {
    identity: DomainIdentity,
    facts: Vec<KnowledgeFact>,
}

/// An immutable cache view that pins its identity until the last clone is dropped.
#[derive(Clone, Debug)]
pub struct KnowledgeSnapshot {
    rows: Arc<Rows>,
    _pin: Arc<()>,
}

impl KnowledgeSnapshot {
    /// The account and complete content scope of this view.
    pub fn identity(&self) -> &DomainIdentity {
        &self.rows.identity
    }

    /// A stored fact, including an explicit Unknown tombstone, or no SQL fact.
    pub fn fact(&self, key: &KnowledgeKey) -> Option<&KnowledgeFact> {
        self.rows
            .facts
            .iter()
            .find(|fact| fact.record.key() == key.as_str())
    }

    /// An absent SQL fact is Unknown without creating a durable head.
    pub fn status(&self, key: &KnowledgeKey) -> KnowledgeStatus {
        self.fact(key)
            .map_or(KnowledgeStatus::Unknown, |fact| fact.record.status())
    }

    /// An absent fact starts at INITIAL. A tombstone retains its committed version.
    pub fn version(&self, key: &KnowledgeKey) -> StoreVersion {
        self.fact(key)
            .map_or(StoreVersion::INITIAL, KnowledgeFact::version)
    }

    /// All committed facts in this bounded identity, ordered by typed key.
    pub fn records(&self) -> &[KnowledgeFact] {
        &self.rows.facts
    }
}

struct Entry {
    identity: DomainIdentity,
    resident: u64,
    used: u64,
    pin: Arc<()>,
    rows: Option<Arc<Rows>>,
    retired: Weak<Rows>,
    error: Option<ErrorCode>,
    readback: Readback,
    reads: u64,
    order: u64,
    retry: Retry,
    history: VecDeque<RequestId>,
}

impl Entry {
    fn snapshot(&self) -> Result<KnowledgeSnapshot, ErrorCode> {
        if let Some(code) = self.error {
            return Err(code);
        }
        Ok(KnowledgeSnapshot {
            rows: Arc::clone(self.rows.as_ref().ok_or(not_ready())?),
            _pin: Arc::clone(&self.pin),
        })
    }

    fn can_replace(&self) -> bool {
        self.retired.strong_count() == 0
    }

    /// Adopt `rows` unless a third live generation would result: a held
    /// current generation can only retire while no older one is still held.
    /// An unheld current generation is dropped and `retired` keeps tracking
    /// the older held one. Returns whether `rows` became current.
    fn adopt(&mut self, rows: Arc<Rows>) -> bool {
        let held = self
            .rows
            .as_ref()
            .is_some_and(|current| Arc::strong_count(current) > 1);
        if held && !self.can_replace() {
            return false;
        }
        if let Some(previous) = self.rows.replace(rows) {
            if held {
                self.retired = Arc::downgrade(&previous);
            }
        }
        self.error = None;
        true
    }

    /// Whether `snapshot` views this entry's current generation.
    #[cfg(test)]
    fn is_current(&self, snapshot: &KnowledgeSnapshot) -> bool {
        self.rows
            .as_ref()
            .is_some_and(|rows| Arc::ptr_eq(rows, &snapshot.rows))
    }
}

#[derive(Default)]
pub(super) struct KnowledgeState {
    entries: Vec<Entry>,
    queue: VecDeque<Write>,
    active: Option<(u64, RequestId, bool)>,
    results: HashMap<RequestId, Option<Settled>>,
    serial: u64,
}

pub(super) struct Write {
    resident: u64,
    request: RequestId,
    record: Arc<KnowledgeRecord>,
    retry: Retry,
}

impl Write {
    fn terminal(&self) -> Terminal {
        Terminal {
            identity: self.record.identity().clone(),
            action: CommitAction::KnowledgeWrite,
            request: self.request,
        }
    }

    #[cfg(test)]
    pub(super) fn invalidating(&self) -> bool {
        self.record.revocation().is_some()
    }
}

pub(super) enum KnowledgeJob {
    Read {
        resident: u64,
        identity: DomainIdentity,
    },
    Write(Write),
}

fn not_ready() -> ErrorCode {
    ErrorCode::NotReady { domain: DOMAIN }
}

fn busy() -> ErrorCode {
    ErrorCode::Busy { domain: DOMAIN }
}

/// The root already holds facts for [`MAX_KNOWLEDGE_STORES`] identities and
/// no fact is ever reclaimed, so a new identity is refused for good: not a
/// transient `Busy` a caller would retry.
fn identities_full() -> ErrorCode {
    ErrorCode::Validation {
        domain: DOMAIN,
        reason: ValidationReason::TooManyRows,
    }
}

fn failed(reason: FailureReason) -> ErrorCode {
    ErrorCode::Failed {
        domain: DOMAIN,
        reason,
    }
}

impl KnowledgeState {
    fn admit(&mut self, identity: &DomainIdentity) -> Result<&mut Entry, ErrorCode> {
        self.serial = self.serial.checked_add(1).ok_or_else(busy)?;
        let index = if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.identity == *identity)
        {
            index
        } else {
            if self.entries.len() == MAX_KNOWLEDGE_STORES {
                let victim = self
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| {
                        Arc::strong_count(&entry.pin) == 1
                            && entry.readback == Readback::Idle
                            && !self
                                .queue
                                .iter()
                                .any(|write| write.resident == entry.resident)
                            && self.active.is_none_or(|active| active.0 != entry.resident)
                    })
                    .min_by_key(|(_, entry)| entry.used)
                    .map(|(index, _)| index)
                    .ok_or_else(busy)?;
                let old = self.entries.swap_remove(victim);
                for request in old.history {
                    self.results.remove(&request);
                }
            }
            self.entries.push(Entry {
                identity: identity.clone(),
                resident: self.serial,
                used: self.serial,
                pin: Arc::new(()),
                rows: None,
                retired: Weak::new(),
                error: None,
                readback: Readback::Idle,
                reads: 0,
                order: 0,
                retry: Retry::default(),
                history: VecDeque::new(),
            });
            self.entries.len() - 1
        };
        self.entries[index].used = self.serial;
        Ok(&mut self.entries[index])
    }

    fn entry(&mut self, resident: u64) -> Option<&mut Entry> {
        self.entries
            .iter_mut()
            .find(|entry| entry.resident == resident)
    }

    pub(super) fn settled_of(&self, request: RequestId) -> Option<Option<&Settled>> {
        self.results.get(&request).map(Option::as_ref)
    }

    pub(super) fn outstanding(&self) -> usize {
        self.queue.len() + usize::from(self.active.is_some())
    }

    fn settle(&mut self, write: &Write, settlement: WriteSettlement) -> bool {
        if self.active.is_some_and(|active| active.1 == write.request) {
            self.active = None;
        }
        let Some(result) = self.results.get_mut(&write.request) else {
            return false;
        };
        if result.is_some() {
            return false;
        }
        *result = Some(Settled {
            request: write.request,
            settlement,
            successor: None,
        });
        if let Some(entry) = self.entry(write.resident) {
            entry.history.push_back(write.request);
            if entry.history.len() > SETTLED_PER_STORE {
                let forgotten = entry.history.pop_front().expect("bounded history");
                self.results.remove(&forgotten);
            }
        }
        true
    }

    pub(super) fn take_job(
        &mut self,
        now: Instant,
        stopping: bool,
    ) -> Result<KnowledgeJob, Option<Instant>> {
        let mut next = None;
        for entry in &mut self.entries {
            if stopping && entry.readback != Readback::Idle {
                entry.readback = Readback::Idle;
                entry.reads += 1;
                entry.error = Some(not_ready());
            }
        }
        if let Some(entry) = self
            .entries
            .iter_mut()
            .filter(|entry| entry.readback == Readback::Queued && entry.retry.due(now))
            .min_by_key(|entry| entry.order)
        {
            entry.readback = Readback::InFlight;
            return Ok(KnowledgeJob::Read {
                resident: entry.resident,
                identity: entry.identity.clone(),
            });
        }
        if let Some(index) = self.queue.iter().position(|write| write.retry.due(now)) {
            let mut write = self.queue.remove(index).expect("selected write");
            write.retry.first.get_or_insert(now);
            self.active = Some((write.resident, write.request, false));
            return Ok(KnowledgeJob::Write(write));
        }
        for at in self
            .entries
            .iter()
            .filter(|entry| entry.readback == Readback::Queued)
            .filter_map(|entry| entry.retry.at)
            .chain(self.queue.iter().filter_map(|write| write.retry.at))
        {
            next = Some(next.map_or(at, |earlier: Instant| earlier.min(at)));
        }
        Err(next)
    }

    pub(super) fn requeue(
        &mut self,
        job: KnowledgeJob,
        now: Instant,
        abandoned: bool,
    ) -> Option<(Terminal, WriteSettlement)> {
        match job {
            KnowledgeJob::Read { resident, .. } => {
                if let Some(entry) = self.entry(resident) {
                    if abandoned || entry.retry.expired(now) {
                        entry.error = Some(busy());
                        entry.readback = Readback::Idle;
                        entry.reads += 1;
                    } else {
                        entry.retry.first.get_or_insert(now);
                        entry.retry.delay(now);
                        entry.readback = Readback::Queued;
                    }
                }
            }
            KnowledgeJob::Write(mut write) => {
                self.active = None;
                if abandoned || write.retry.expired(now) {
                    let settlement = if abandoned {
                        Settlement::Cancelled(Cancellation::Shutdown)
                    } else {
                        Settlement::Failed(busy())
                    };
                    if self.settle(&write, settlement.clone()) {
                        return Some((write.terminal(), settlement));
                    }
                } else {
                    write.retry.delay(now);
                    self.queue.push_front(write);
                }
            }
        }
        None
    }

    pub(super) fn contain(&mut self, job: &KnowledgeJob) -> Option<(Terminal, WriteSettlement)> {
        match job {
            KnowledgeJob::Read { resident, .. } => {
                if let Some(entry) = self.entry(*resident) {
                    entry.error = Some(failed(FailureReason::Io));
                    entry.readback = Readback::Idle;
                    entry.reads += 1;
                }
            }
            KnowledgeJob::Write(write) => {
                let settlement = Settlement::Failed(failed(FailureReason::Io));
                if self.settle(write, settlement.clone()) {
                    return Some((write.terminal(), settlement));
                }
            }
        }
        None
    }

    pub(super) fn abandon(&mut self) -> (Vec<Terminal>, usize) {
        let mut terminals = Vec::new();
        while let Some(write) = self.queue.pop_front() {
            if self.settle(&write, Settlement::Cancelled(Cancellation::Shutdown)) {
                terminals.push(write.terminal());
            }
        }
        let mut publishing = 0;
        if let Some((resident, request, begun)) = self.active {
            if begun {
                publishing = 1;
            } else if let Some(result) = self.results.get_mut(&request) {
                if result.is_none() {
                    *result = Some(Settled {
                        request,
                        settlement: Settlement::Cancelled(Cancellation::Shutdown),
                        successor: None,
                    });
                    if let Some(entry) = self.entry(resident) {
                        terminals.push(Terminal {
                            identity: entry.identity.clone(),
                            action: CommitAction::KnowledgeWrite,
                            request,
                        });
                    }
                }
            }
        }
        (terminals, publishing)
    }
}

impl LocalStateService {
    fn check_knowledge(&self, identity: &DomainIdentity) -> Result<(), ErrorCode> {
        if identity.domain() != DOMAIN {
            return Err(ErrorCode::Validation {
                domain: DOMAIN,
                reason: ValidationReason::Identity,
            });
        }
        if !self.is_open(DOMAIN) {
            return Err(not_ready());
        }
        Ok(())
    }

    /// Queue a SQL read without blocking the caller or creating an absent durable head.
    pub fn start_knowledge_readback(
        &self,
        identity: &DomainIdentity,
    ) -> Result<ReadbackTicket, ErrorCode> {
        self.check_knowledge(identity)?;
        let mut state = self.shared.state.lock();
        if state.shutdown {
            return Err(not_ready());
        }
        if !self.shared.start_writer(&mut state) {
            return Err(not_ready());
        }
        let entry = state.knowledge.admit(identity)?;
        if !entry.can_replace() {
            return Err(busy());
        }
        let target = match entry.readback {
            Readback::Idle => {
                entry.readback = Readback::Queued;
                entry.order = entry.used;
                entry.reads + 1
            }
            Readback::Queued => entry.reads + 1,
            _ => {
                entry.readback = Readback::InFlightAgain;
                entry.reads + 2
            }
        };
        entry.retry.first.get_or_insert(Instant::now());
        let ticket = ReadbackTicket {
            resident: entry.resident,
            target,
        };
        self.shared.wake.notify_one();
        Ok(ticket)
    }

    /// Poll this read ticket. The returned snapshot pins its cache residence.
    pub fn poll_knowledge_readback(
        &self,
        identity: &DomainIdentity,
        ticket: ReadbackTicket,
    ) -> Option<Result<KnowledgeSnapshot, ErrorCode>> {
        self.finish_knowledge_readback(identity, ticket, None)
    }

    /// Wait on a control thread only. Slot and frame callers must poll instead.
    pub fn wait_knowledge_readback(
        &self,
        identity: &DomainIdentity,
        ticket: ReadbackTicket,
        timeout: Duration,
    ) -> Result<KnowledgeSnapshot, ErrorCode> {
        self.finish_knowledge_readback(identity, ticket, Some(Instant::now() + timeout))
            .unwrap_or(Err(not_ready()))
    }

    fn finish_knowledge_readback(
        &self,
        identity: &DomainIdentity,
        ticket: ReadbackTicket,
        deadline: Option<Instant>,
    ) -> Option<Result<KnowledgeSnapshot, ErrorCode>> {
        if let Err(code) = self.check_knowledge(identity) {
            return Some(Err(code));
        }
        let mut state = self.shared.state.lock();
        loop {
            let Some(entry) = state
                .knowledge
                .entries
                .iter()
                .find(|entry| entry.resident == ticket.resident && entry.identity == *identity)
            else {
                return Some(Err(not_ready()));
            };
            if entry.reads >= ticket.target {
                return Some(entry.snapshot());
            }
            if self
                .shared
                .settled
                .wait_until(&mut state, deadline?)
                .timed_out()
            {
                return None;
            }
        }
    }

    /// Read only the current resident snapshot. This method never accesses the disk.
    pub fn knowledge_snapshot(
        &self,
        identity: &DomainIdentity,
    ) -> Result<KnowledgeSnapshot, ErrorCode> {
        self.check_knowledge(identity)?;
        let mut state = self.shared.state.lock();
        if !state
            .knowledge
            .entries
            .iter()
            .any(|entry| entry.identity == *identity)
        {
            return Err(not_ready());
        }
        state.knowledge.admit(identity)?.snapshot()
    }

    /// The slot port's knowledge read ([`StatePort::start_knowledge_read`]).
    pub(super) fn port_start_knowledge_read(
        &self,
        identity: &DomainIdentity,
    ) -> Result<KnowledgeTicket, ErrorCode> {
        let ticket = self.start_knowledge_readback(identity)?;
        Ok(KnowledgeTicket::new(ticket.resident, ticket.target))
    }

    /// The slot port's read result ([`StatePort::poll_knowledge_read`]): one
    /// fact shared from the cache, never a copy, and no snapshot pin kept.
    pub(super) fn port_poll_knowledge_read(
        &self,
        identity: &DomainIdentity,
        ticket: KnowledgeTicket,
        key: &KnowledgeKey,
    ) -> Option<Result<Option<StoredKnowledge>, ErrorCode>> {
        let (resident, target) = ticket.parts();
        let snapshot =
            self.poll_knowledge_readback(identity, ReadbackTicket { resident, target })?;
        Some(snapshot.map(|snapshot| {
            snapshot
                .fact(key)
                .map(|fact| StoredKnowledge::new(Arc::clone(&fact.record), fact.version))
        }))
    }

    /// A Knowledge lease from the resident cache: every fact of the identity
    /// at the newest fact version. A cold identity queues one SQL read and is
    /// `NotReady`; a read already queued or running is not queued again.
    pub(super) fn knowledge_lease(
        &self,
        identity: &DomainIdentity,
    ) -> Result<DomainLease, ErrorCode> {
        self.check_knowledge(identity)?;
        let mut state = self.shared.state.lock();
        if state.shutdown {
            return Err(not_ready());
        }
        let entry = state.knowledge.admit(identity)?;
        if let Some(code) = entry.error {
            return Err(code);
        }
        if let Some(rows) = &entry.rows {
            let version = rows
                .facts
                .iter()
                .map(|fact| fact.version)
                .max()
                .unwrap_or(StoreVersion::INITIAL);
            let at = rows
                .facts
                .iter()
                .map(|fact| fact.record.observed_at())
                .max_by_key(|at| at.unix_seconds())
                .unwrap_or(DurableTimestamp::from_unix_seconds(0));
            let records = rows
                .facts
                .iter()
                .map(|fact| KnowledgeRecord::clone(&fact.record))
                .collect();
            return DomainLease::try_new(
                identity.clone(),
                version,
                at,
                LeaseSnapshot::knowledge(records)?,
            );
        }
        if entry.readback == Readback::Idle {
            entry.readback = Readback::Queued;
            entry.order = entry.used;
            entry.retry.first.get_or_insert(Instant::now());
            if !self.shared.start_writer(&mut state) {
                return Err(not_ready());
            }
            self.shared.wake.notify_one();
        }
        Err(not_ready())
    }

    /// Publish a fact with mandatory per-fact CAS. Acceptance is not durability.
    pub fn publish_knowledge(&self, record: KnowledgeRecord) -> SubmitReceipt {
        if let Err(code) = self.check_knowledge(record.identity()) {
            return SubmitReceipt::Refused(code);
        }
        let mut state = self.shared.state.lock();
        if state.shutdown {
            return SubmitReceipt::Refused(not_ready());
        }
        if state.knowledge.queue.len() >= OPERATOR_QUEUE_CAP {
            return SubmitReceipt::Refused(busy());
        }
        let entry = match state.knowledge.admit(record.identity()) {
            Ok(entry) => entry,
            Err(code) => return SubmitReceipt::Refused(code),
        };
        if !entry.can_replace() {
            return SubmitReceipt::Refused(busy());
        }
        let resident = entry.resident;
        if !self.shared.start_writer(&mut state) {
            return SubmitReceipt::Refused(not_ready());
        }
        let request = match self.shared.minter.mint() {
            Ok(request) => request,
            Err(_) => return SubmitReceipt::Refused(busy()),
        };
        state.knowledge.results.insert(request, None);
        state.knowledge.queue.push_back(Write {
            resident,
            request,
            record: Arc::new(record),
            retry: Retry::default(),
        });
        self.shared.wake.notify_one();
        SubmitReceipt::Accepted { request }
    }

    /// Publish an Unknown tombstone. A stale positive retry keeps its old expected version.
    /// `provenance` is the revoked fact's source: an `Operator` revocation
    /// records `Operator`, every other reason keeps `provenance`
    /// ([`KnowledgeRecord::invalidated_with`]).
    pub fn invalidate_knowledge(
        &self,
        identity: &DomainIdentity,
        key: KnowledgeKey,
        expected: StoreVersion,
        stamp: DurableTimestamp,
        provenance: KnowledgeProvenance,
        revocation: KnowledgeRevocation,
    ) -> SubmitReceipt {
        if let Err(code) = self.check_knowledge(identity) {
            return SubmitReceipt::Refused(code);
        }
        let payload = validate_knowledge_payload(1, &[]).expect("empty current-schema payload");
        let record = KnowledgeRecord::try_new(
            identity.account().clone(),
            identity.scope().clone(),
            expected,
            key.as_str(),
            provenance,
            KnowledgeStatus::Unknown,
            payload,
            stamp,
        )
        .expect("validated identity and typed key")
        .invalidated_with(expected, stamp, revocation);
        self.publish_knowledge(record)
    }

    /// Whether `snapshot` is the current cached generation of its identity.
    #[cfg(test)]
    pub(crate) fn knowledge_is_current(&self, snapshot: &KnowledgeSnapshot) -> bool {
        self.shared
            .state
            .lock()
            .knowledge
            .entries
            .iter()
            .any(|entry| entry.identity == *snapshot.identity() && entry.is_current(snapshot))
    }
}

fn db_error(error: rusqlite::Error, what: &str) -> SqlFailure {
    sql::wait_of(&error).map_or_else(|| sql::refused_by(error, what), SqlFailure::Wait)
}

fn damaged() -> SqlFailure {
    SqlFailure::refused(
        FailureReason::Schema,
        "The stored knowledge fact is invalid. Restore the paired state backup.",
    )
}

fn decode_stamp(raw: &[u8]) -> Result<u64, SqlFailure> {
    sql::decode_u64(raw).ok_or_else(damaged)
}

fn provenance(value: KnowledgeProvenance) -> i64 {
    match value {
        KnowledgeProvenance::QuestList => 0,
        KnowledgeProvenance::Journal => 1,
        KnowledgeProvenance::Operator => 2,
        KnowledgeProvenance::Observed => 3,
    }
}

fn status(value: KnowledgeStatus) -> i64 {
    match value {
        KnowledgeStatus::Known => 0,
        KnowledgeStatus::Partial => 1,
        KnowledgeStatus::Unknown => 2,
    }
}

fn revocation(value: KnowledgeRevocation) -> i64 {
    match value {
        KnowledgeRevocation::Operator => 0,
        KnowledgeRevocation::ObservationRefused => 1,
        KnowledgeRevocation::ContentChanged => 2,
    }
}

fn load(conn: &Connection, identity: &DomainIdentity) -> Result<Loaded<Arc<Rows>>, SqlFailure> {
    let mut statement = conn.prepare("SELECT key,schema,provenance,observed_stamp,status,revocation,expected_version,version,transaction_sequence,payload
        FROM knowledge_fact WHERE profile=?1 AND account=?2 AND revision=?3 AND content=?4 ORDER BY key")
        .map_err(|error| db_error(error, "preparing a knowledge read"))?;
    let mut rows = statement
        .query(params![
            identity.account().profile(),
            identity.account().account(),
            identity.scope().revision().as_i32(),
            identity.scope().content()
        ])
        .map_err(|error| db_error(error, "reading knowledge"))?;
    let mut facts = Vec::new();
    let mut head = StoreVersion::INITIAL;
    while let Some(row) = rows
        .next()
        .map_err(|error| db_error(error, "reading a knowledge fact"))?
    {
        if facts.len() == MAX_KNOWLEDGE_RECORDS {
            return Err(damaged());
        }
        let get_error = |error| db_error(error, "decoding a knowledge fact");
        let key: String = row.get(0).map_err(get_error)?;
        let schema: u32 = row.get(1).map_err(get_error)?;
        let source = match row.get::<_, i64>(2).map_err(get_error)? {
            0 => KnowledgeProvenance::QuestList,
            1 => KnowledgeProvenance::Journal,
            2 => KnowledgeProvenance::Operator,
            3 => KnowledgeProvenance::Observed,
            _ => return Err(damaged()),
        };
        let stamp = decode_stamp(
            row.get_ref(3)
                .map_err(get_error)?
                .as_blob()
                .map_err(|_| damaged())?,
        )?;
        let state = match row.get::<_, i64>(4).map_err(get_error)? {
            0 => KnowledgeStatus::Known,
            1 => KnowledgeStatus::Partial,
            2 => KnowledgeStatus::Unknown,
            _ => return Err(damaged()),
        };
        let revoked = match row.get::<_, Option<i64>>(5).map_err(get_error)? {
            None => None,
            Some(0) => Some(KnowledgeRevocation::Operator),
            Some(1) => Some(KnowledgeRevocation::ObservationRefused),
            Some(2) => Some(KnowledgeRevocation::ContentChanged),
            _ => return Err(damaged()),
        };
        let expected = StoreVersion::from_durable(decode_stamp(
            row.get_ref(6)
                .map_err(get_error)?
                .as_blob()
                .map_err(|_| damaged())?,
        )?);
        let version = StoreVersion::from_durable(decode_stamp(
            row.get_ref(7)
                .map_err(get_error)?
                .as_blob()
                .map_err(|_| damaged())?,
        )?);
        let sequence = decode_stamp(
            row.get_ref(8)
                .map_err(get_error)?
                .as_blob()
                .map_err(|_| damaged())?,
        )?;
        if version == StoreVersion::INITIAL || sequence != version.get() {
            return Err(damaged());
        }
        let payload = row
            .get_ref(9)
            .map_err(get_error)?
            .as_blob()
            .map_err(|_| damaged())?;
        if revoked.is_some() && (state != KnowledgeStatus::Unknown || !payload.is_empty()) {
            return Err(damaged());
        }
        let validated = validate_knowledge_payload(schema, payload).map_err(|_| damaged())?;
        let at = DurableTimestamp::from_unix_seconds(stamp);
        let mut record = KnowledgeRecord::try_new(
            identity.account().clone(),
            identity.scope().clone(),
            expected,
            &key,
            source,
            state,
            validated,
            at,
        )
        .map_err(|_| damaged())?;
        if let Some(reason) = revoked {
            record = record.invalidated_with(expected, at, reason);
        }
        head = head.max(version);
        facts.push(KnowledgeFact {
            record: Arc::new(record),
            version,
            sequence,
        });
    }
    Ok(Loaded {
        head,
        observed: Some(Arc::new(Rows {
            identity: identity.clone(),
            facts,
        })),
    })
}

struct StagedFact {
    rows: Arc<Rows>,
    index: usize,
    previous: StoreVersion,
}

impl Staging for StagedFact {
    type Durable = Arc<Rows>;
    const DOMAIN: LocalStateDomain = DOMAIN;
    const MAX_PAGES: u64 = 32;
    const HEAD: &'static str = "knowledge";
    const ACTION: &'static str = "knowledge-write";
    fn unchanged(&self) -> bool {
        false
    }
    fn admit(&self, tx: &Connection) -> Result<(), (ErrorCode, &'static str)> {
        if self.rows.facts.len() != 1 || self.previous != StoreVersion::INITIAL {
            return Ok(());
        }
        let identities: i64 = tx.query_row("SELECT count(*) FROM (SELECT 1 FROM knowledge_fact GROUP BY profile,account,revision,content)", [], |row| row.get(0))
            .map_err(|_| (failed(FailureReason::Io), "Could not count stored knowledge identities."))?;
        if identities >= MAX_KNOWLEDGE_STORES as i64 {
            return Err((
                identities_full(),
                "The durable knowledge identity limit is full. No fact was evicted, and the new identity is refused until facts are reclaimed.",
            ));
        }
        Ok(())
    }
    fn digest(&self) -> ContentDigest {
        let record = &self.rows.facts[self.index].record;
        let mut hash = Sha256::new();
        for part in [
            record.identity().account().profile(),
            record.identity().account().account(),
            record.identity().scope().content(),
            record.key(),
        ] {
            hash.update((part.len() as u64).to_be_bytes());
            hash.update(part.as_bytes());
        }
        hash.update(record.identity().scope().revision().as_i32().to_be_bytes());
        hash.update(record.schema_version().to_be_bytes());
        hash.update(provenance(record.provenance()).to_be_bytes());
        hash.update(status(record.status()).to_be_bytes());
        hash.update(record.observed_at().unix_seconds().to_be_bytes());
        hash.update(record.expected().get().to_be_bytes());
        hash.update(record.revocation().map_or(-1, revocation).to_be_bytes());
        hash.update(record.payload());
        hash.finalize().into()
    }
    fn set_version(&mut self, version: StoreVersion) {
        let fact = &mut Arc::get_mut(&mut self.rows)
            .expect("unpublished staged rows")
            .facts[self.index];
        fact.version = version;
        fact.sequence = version.get();
    }
    fn write(&self, tx: &Connection, version: StoreVersion) -> rusqlite::Result<()> {
        let record = &self.rows.facts[self.index].record;
        let identity = record.identity();
        tx.execute("INSERT INTO knowledge_fact
            (profile,account,revision,content,key,schema,provenance,observed_stamp,status,revocation,expected_version,version,transaction_sequence,payload)
            VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?12,?13)
            ON CONFLICT(profile,account,revision,content,key) DO UPDATE SET schema=excluded.schema,
            provenance=excluded.provenance,observed_stamp=excluded.observed_stamp,status=excluded.status,
            revocation=excluded.revocation,expected_version=excluded.expected_version,version=excluded.version,
            transaction_sequence=excluded.transaction_sequence,payload=excluded.payload",
            params![identity.account().profile(),identity.account().account(),identity.scope().revision().as_i32(),identity.scope().content(),
                record.key(),record.schema_version(),provenance(record.provenance()),sql::be(record.observed_at().unix_seconds()),
                status(record.status()),record.revocation().map(revocation),sql::be(record.expected().get()),sql::be(version.get()),record.payload()])?;
        Ok(())
    }
    fn durable(&self, _version: StoreVersion, _digest: ContentDigest) -> Arc<Rows> {
        Arc::clone(&self.rows)
    }
}

impl Shared {
    pub(super) fn run_knowledge_job(&self, db: &mut Db, job: &KnowledgeJob) -> Option<WaitCause> {
        if let Err(error) = db.open_for_domain() {
            return self.knowledge_failure(job, error);
        }
        match job {
            KnowledgeJob::Read { resident, identity } => {
                self.stall_read();
                let mut result = (|| {
                    let conn = db.conn.as_mut().ok_or_else(sql::closed)?;
                    let tx = conn
                        .transaction()
                        .map_err(|error| db_error(error, "starting a knowledge read"))?;
                    let loaded = load(&tx, identity)?;
                    tx.commit()
                        .map_err(|error| db_error(error, "ending a knowledge read"))?;
                    Ok(loaded
                        .observed
                        .expect("knowledge read always projects absence"))
                })();
                db.after_failure(&mut result);
                match result {
                    Ok(rows) => {
                        let mut state = self.state.lock();
                        if let Some(entry) = state.knowledge.entry(*resident) {
                            // A write may have retired a held generation since this
                            // read was admitted: refuse it as admission would, so no
                            // third generation is ever live.
                            if !entry.adopt(rows) {
                                entry.error = Some(busy());
                            }
                            entry.reads += 1;
                            entry.readback = if entry.readback == Readback::InFlightAgain {
                                Readback::Queued
                            } else {
                                Readback::Idle
                            };
                            entry.retry = Retry::default();
                        }
                        None
                    }
                    Err(error) => self.knowledge_failure(job, error),
                }
            }
            KnowledgeJob::Write(write) => {
                {
                    let mut state = self.state.lock();
                    if !state
                        .knowledge
                        .entry(write.resident)
                        .is_some_and(|entry| entry.can_replace())
                    {
                        drop(state);
                        self.finish_knowledge(write, Settlement::Failed(busy()));
                        return None;
                    }
                }
                let committed = db.commit(
                    write.request,
                    None,
                    None,
                    || {
                        let mut state = self.state.lock();
                        if state.abandoned {
                            return false;
                        }
                        if let Some(active) = &mut state.knowledge.active {
                            active.2 = true;
                        }
                        true
                    },
                    |conn| load(conn, write.record.identity()),
                    |current: &Arc<Rows>| {
                        let index = current
                            .facts
                            .binary_search_by(|fact| fact.record.key().cmp(write.record.key()));
                        let previous = index
                            .as_ref()
                            .ok()
                            .map_or(StoreVersion::INITIAL, |index| current.facts[*index].version);
                        if write.record.expected() != previous {
                            return Err(ErrorCode::Conflict {
                                expected: write.record.expected(),
                                found: previous,
                            });
                        }
                        if index.is_err() && current.facts.len() >= MAX_KNOWLEDGE_RECORDS {
                            return Err(ErrorCode::Validation {
                                domain: DOMAIN,
                                reason: ValidationReason::TooManyRows,
                            });
                        }
                        let mut facts = current.facts.clone();
                        let fact = KnowledgeFact {
                            record: Arc::clone(&write.record),
                            version: StoreVersion::INITIAL,
                            sequence: 0,
                        };
                        let index = match index {
                            Ok(index) => {
                                facts[index] = fact;
                                index
                            }
                            Err(index) => {
                                facts.insert(index, fact);
                                index
                            }
                        };
                        Ok(StagedFact {
                            rows: Arc::new(Rows {
                                identity: current.identity.clone(),
                                facts,
                            }),
                            index,
                            previous,
                        })
                    },
                );
                match committed {
                    Err(error) => self.knowledge_failure(job, error),
                    Ok(Commit::Committed {
                        staged,
                        durable,
                        truncate_due,
                        ..
                    }) => {
                        let fact = &durable.facts[staged.index];
                        let version = fact.version;
                        let at = DurableTimestamp::now();
                        {
                            let mut state = self.state.lock();
                            state.truncate.due |= truncate_due;
                            if let Some(entry) = state.knowledge.entry(write.resident) {
                                // The job checked `can_replace` and no reader can
                                // re-pin an unheld retired generation since.
                                let adopted = entry.adopt(durable);
                                debug_assert!(adopted, "a checked write adopts its commit");
                            }
                        }
                        self.audit(
                            AuditCommit::after_commit(
                                write.record.identity().clone(),
                                CommitAction::KnowledgeWrite,
                                staged.previous,
                                version,
                                AuditOrigin::Submitted {
                                    request: write.request,
                                },
                                at,
                            ),
                            &AuditKind::Plain,
                        );
                        self.finish_knowledge(
                            write,
                            Settlement::Completed(DurableReceipt::new(
                                write.record.identity().clone(),
                                version,
                                at,
                                Some(write.request),
                            )),
                        );
                        None
                    }
                    Ok(Commit::Declined {
                        code,
                        observed,
                        detail,
                    }) => {
                        if let Some(detail) = detail {
                            self.notice(Level::Warn, format_args!("{detail}"));
                        }
                        if let Some(rows) = observed {
                            if let Some(entry) = self.state.lock().knowledge.entry(write.resident) {
                                entry.adopt(rows);
                            }
                        }
                        self.finish_knowledge(write, Settlement::Failed(code));
                        None
                    }
                    Ok(Commit::Unresolved { detail, .. }) => {
                        self.notice(Level::Warn, format_args!("{detail}"));
                        if let Some(entry) = self.state.lock().knowledge.entry(write.resident) {
                            entry.error = Some(failed(FailureReason::Io));
                        }
                        self.finish_knowledge(write, Settlement::Failed(failed(FailureReason::Io)));
                        None
                    }
                    Ok(Commit::Abandoned) => {
                        self.finish_knowledge(write, Settlement::Cancelled(Cancellation::Shutdown));
                        None
                    }
                    Ok(Commit::Unchanged { .. }) => unreachable!(
                        "knowledge publications always advance the transaction sequence"
                    ),
                }
            }
        }
    }

    fn knowledge_failure(&self, job: &KnowledgeJob, error: SqlFailure) -> Option<WaitCause> {
        match error {
            SqlFailure::Wait(cause) => Some(cause),
            SqlFailure::Refused { reason, detail } => {
                self.notice(Level::Warn, format_args!("{detail}"));
                match job {
                    KnowledgeJob::Read { resident, .. } => {
                        if let Some(entry) = self.state.lock().knowledge.entry(*resident) {
                            entry.error = Some(failed(reason));
                            entry.readback = Readback::Idle;
                            entry.reads += 1;
                        }
                    }
                    KnowledgeJob::Write(write) => {
                        self.finish_knowledge(write, Settlement::Failed(failed(reason)))
                    }
                }
                None
            }
        }
    }

    fn finish_knowledge(&self, write: &Write, settlement: WriteSettlement) {
        if self
            .state
            .lock()
            .knowledge
            .settle(write, settlement.clone())
        {
            self.terminal(
                &Terminal {
                    identity: write.record.identity().clone(),
                    request: write.request,
                    action: CommitAction::KnowledgeWrite,
                },
                &settlement,
                None,
            );
        }
    }
}

/// Exact SQL values retained during an explicit preferences-only reset.
pub(super) struct Carry(Vec<[rusqlite::types::Value; 14]>);

impl Carry {
    pub(super) fn read(conn: &Connection) -> Result<Self, SqlFailure> {
        let mut statement = conn.prepare(
            "SELECT profile,account,revision,content,key,schema,provenance,observed_stamp,
             status,revocation,expected_version,version,transaction_sequence,payload FROM knowledge_fact"
        ).map_err(|error| db_error(error, "reading knowledge for recovery"))?;
        let mut query = statement
            .query([])
            .map_err(|error| db_error(error, "reading knowledge for recovery"))?;
        let mut facts = Vec::new();
        while let Some(row) = query
            .next()
            .map_err(|error| db_error(error, "reading knowledge for recovery"))?
        {
            if facts.len() == MAX_KNOWLEDGE_STORES * MAX_KNOWLEDGE_RECORDS {
                return Err(damaged());
            }
            facts.push(
                (|| -> rusqlite::Result<_> {
                    Ok([
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                        row.get(10)?,
                        row.get(11)?,
                        row.get(12)?,
                        row.get(13)?,
                    ])
                })()
                .map_err(|error| db_error(error, "decoding knowledge for recovery"))?,
            );
        }
        Ok(Self(facts))
    }

    pub(super) fn restore(
        &self,
        conn: &mut Connection,
        root: &StateRoot,
        guard: &files::WriteLock,
    ) -> Result<(), SqlFailure> {
        for batch in self.0.chunks(64) {
            let tx = sql::begin_write(conn, root, guard, "restoring knowledge")?;
            {
                let mut statement = tx.prepare(
                    "INSERT INTO knowledge_fact (profile,account,revision,content,key,schema,provenance,
                     observed_stamp,status,revocation,expected_version,version,transaction_sequence,payload)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)"
                ).map_err(|error| db_error(error, "restoring knowledge"))?;
                for row in batch {
                    statement
                        .execute(rusqlite::params_from_iter(row))
                        .map_err(|error| db_error(error, "restoring a knowledge fact"))?;
                }
            }
            tx.commit()
                .map_err(|error| db_error(error, "committing restored knowledge"))?;
        }
        Ok(())
    }
}
