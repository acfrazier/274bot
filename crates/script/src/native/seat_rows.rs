use crate::native::request::NativeKind;
use crate::native::seat::{SeatEntry, SeatResult as NativeSeatResult, SeatTerminal};
use crate::native::NativePhase;
use api::identity::RequestId;
use api::script_contract::{
    AbortReason, CallbackCancelReason, DecisionSequence, Refusal, CONTROL_RESERVATION,
    MAX_CANCEL_CARRY, MAX_DECIDE_CARRY, MAX_ENCODED_GAMEPLAY, MAX_LIVE, MAX_PENDING_STARTS,
    MAX_RESULTS, MAX_START_CHARGE,
};

/// Seat starts retained by the adapter and encoded per batch (§28.4–28.5).
pub const MAX_SEAT_STARTS: usize = MAX_PENDING_STARTS;
/// Seat decisions carried until terminal receipt (§28.5).
pub const MAX_SEAT_DECISIONS: usize = MAX_DECIDE_CARRY;
/// Seat cancellations carried for live and pending children (§28.5).
pub const MAX_SEAT_CANCELS: usize = MAX_CANCEL_CARRY;
/// Total reserved controls and legacy releases (§28.5).
pub const MAX_SEAT_RESERVED_ROWS: usize = CONTROL_RESERVATION;
/// Remaining gameplay/lifecycle rows after reservation (§28.5).
pub const MAX_SEAT_GAMEPLAY_ROWS: usize = MAX_ENCODED_GAMEPLAY;
/// Maximum live entries on one seat page.
pub const MAX_SEAT_LIVE: usize = MAX_LIVE;
/// Maximum retained terminal results on one seat page.
pub const MAX_SEAT_RESULTS: usize = MAX_RESULTS;
/// Maximum bytes in each structured-clone wire vector.
pub const MAX_SEAT_BINARY_BYTES: usize = MAX_START_CHARGE;
/// Maximum distinct mirror revocations carried in one interact batch.
pub const MAX_SEAT_MIRROR_REVOKES: usize = 141;

/// Borrowed prompt data for one live seat request.
#[derive(Clone, Copy)]
pub struct SeatPromptInput<'a> {
    /// Nonzero sequence associated with the current prompt.
    pub sequence: DecisionSequence,
    /// Family-specific decision point.
    pub point: &'a str,
    /// Bounded binary structured-clone prompt arguments.
    pub args: &'a [u8],
}

/// The family-owned portion of one seat live-page row.
#[derive(Clone, Copy)]
pub struct SeatLiveDetails<'a> {
    /// Must identify the corresponding [`SeatEntry`] at the same index.
    pub id: RequestId,
    /// Family phase represented on the status page.
    pub phase: NativePhase,
    /// Borrowed current prompt, if the family is awaiting a decision.
    pub prompt: Option<SeatPromptInput<'a>>,
}

/// Borrowed snapshot view of the seat forest and retained terminal ledger.
///
/// `live_details` has exactly one row per `live` entry, in the same order.
/// The family machine or isolate adapter owns phase and prompt payloads;
/// this projection never fabricates them from forest state.
/// `mirror_revokes_consumed` is the highest contiguous mirror revoke applied by the host.
#[derive(Clone, Copy)]
pub struct SeatPageInput<'a> {
    live: &'a [SeatEntry],
    results: &'a [NativeSeatResult],
    result_values: &'a [SeatResultValue<'a>],
    live_details: &'a [SeatLiveDetails<'a>],
    mirror_revokes_consumed: u64,
}

/// Optional successful or failed result bytes for one terminal seat entry.
#[derive(Clone, Copy)]
pub struct SeatResultValue<'a> {
    /// Must identify exactly one terminal result on the same page.
    pub id: RequestId,
    /// Bounded structured-clone result bytes.
    pub value: &'a [u8],
}

/// A rejected native seat-page projection, before any bytes reach the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SeatPageError {
    #[error("seat live vector length {len} exceeds cap {MAX_SEAT_LIVE}")]
    LiveTooLarge { len: usize },
    #[error("seat result vector length {len} exceeds cap {MAX_SEAT_RESULTS}")]
    ResultsTooLarge { len: usize },
    #[error("seat live and result credits exceed cap {MAX_RESULTS}")]
    CreditLimit,
    #[error("seat live/detail count mismatch: {live} live, {details} details")]
    LiveDetailCountMismatch { live: usize, details: usize },
    #[error("seat live/detail id mismatch at index {index}")]
    LiveDetailIdMismatch { index: usize },
    #[error("seat live entry {id} is its own parent")]
    OwnParent { id: u64 },
    #[error("seat prompt args length {len} exceeds cap {MAX_SEAT_BINARY_BYTES}")]
    PromptArgsTooLarge { len: usize },
    #[error("seat live entry {id} has misaligned prompt sequence")]
    PromptSequenceMismatch { id: u64 },
    #[error("seat results are not strictly ordered by terminal sequence")]
    UnorderedResults,
    #[error("seat result value length {len} exceeds cap {MAX_SEAT_BINARY_BYTES}")]
    ResultValueTooLarge { len: usize },
    #[error("duplicate seat result value for {id}")]
    DuplicateResultValue { id: u64 },
    #[error("seat result value {id} has no matching terminal result")]
    MissingTerminalResult { id: u64 },
    #[error("seat result value {id} matches multiple terminal results")]
    AmbiguousTerminalResult { id: u64 },
    #[error("seat result value {id} requires a Done or Failed outcome")]
    ResultValueOutcome { id: u64 },
    #[error("duplicate live/terminal seat id")]
    DuplicateSeatId,
    #[error("local ArgsTooLarge refusal cannot be published on the seat wire")]
    LocalArgsTooLargeRefusal,
    #[error("Reset is not a seat-published terminal")]
    ResetTerminal,
    #[error("Terminated is not a seat-published terminal")]
    TerminatedTerminal,
}

impl<'a> SeatPageInput<'a> {
    /// Validate and borrow one complete seat page and its revoke acknowledgement.
    pub fn new(
        live: &'a [SeatEntry],
        results: &'a [NativeSeatResult],
        live_details: &'a [SeatLiveDetails<'a>],
        mirror_revokes_consumed: u64,
    ) -> Result<Self, SeatPageError> {
        let page = Self {
            live,
            results,
            result_values: &[],
            live_details,
            mirror_revokes_consumed,
        };
        page.validate()?;
        Ok(page)
    }

    /// Attach optional result bytes and validate the complete page.
    pub fn with_result_values(
        mut self,
        values: &'a [SeatResultValue<'a>],
    ) -> Result<Self, SeatPageError> {
        self.result_values = values;
        self.validate()?;
        Ok(self)
    }

    /// Borrow the live seat entries.
    pub fn live(&self) -> &'a [SeatEntry] {
        self.live
    }

    /// Borrow the retained terminal results.
    pub fn results(&self) -> &'a [NativeSeatResult] {
        self.results
    }

    /// Borrow the optional terminal result values.
    pub fn result_values(&self) -> &'a [SeatResultValue<'a>] {
        self.result_values
    }

    /// Borrow the family-owned details for live entries.
    pub fn live_details(&self) -> &'a [SeatLiveDetails<'a>] {
        self.live_details
    }

    /// Return the highest contiguous mirror revoke applied by the host.
    pub fn mirror_revokes_consumed(&self) -> u64 {
        self.mirror_revokes_consumed
    }

    /// Validate the complete seat page.
    pub fn validate(&self) -> Result<(), SeatPageError> {
        validate_seat_page(self)
    }
}

fn seat_terminal_publishable(terminal: SeatTerminal) -> Result<(), SeatPageError> {
    match terminal {
        SeatTerminal::Refused(Refusal::ArgsTooLarge) => {
            Err(SeatPageError::LocalArgsTooLargeRefusal)
        }
        SeatTerminal::Aborted(AbortReason::Reset) => Err(SeatPageError::ResetTerminal),
        SeatTerminal::Aborted(AbortReason::Terminated) => Err(SeatPageError::TerminatedTerminal),
        _ => Ok(()),
    }
}

fn validate_seat_page(page: &SeatPageInput<'_>) -> Result<(), SeatPageError> {
    if page.live.len() > MAX_SEAT_LIVE {
        return Err(SeatPageError::LiveTooLarge {
            len: page.live.len(),
        });
    }
    if page.results.len() > MAX_SEAT_RESULTS {
        return Err(SeatPageError::ResultsTooLarge {
            len: page.results.len(),
        });
    }
    if page.live.len() + page.results.len() > MAX_RESULTS {
        return Err(SeatPageError::CreditLimit);
    }
    let mut ids = [0u64; MAX_RESULTS];
    let mut id_count = 0usize;
    if page.live_details.len() != page.live.len() {
        return Err(SeatPageError::LiveDetailCountMismatch {
            live: page.live.len(),
            details: page.live_details.len(),
        });
    }
    for (index, (entry, details)) in page.live.iter().zip(page.live_details).enumerate() {
        if entry.id != details.id {
            return Err(SeatPageError::LiveDetailIdMismatch { index });
        }
        if entry.parent == Some(entry.id) {
            return Err(SeatPageError::OwnParent { id: entry.id.get() });
        }
        ids[id_count] = entry.id.get();
        id_count += 1;
        match details.prompt {
            Some(prompt)
                if entry.prompt == Some(prompt.sequence.get())
                    && entry.decisions_consumed.checked_add(1) == Some(prompt.sequence.get()) =>
            {
                if prompt.args.len() > MAX_SEAT_BINARY_BYTES {
                    return Err(SeatPageError::PromptArgsTooLarge {
                        len: prompt.args.len(),
                    });
                }
            }
            None if entry.prompt.is_none() => {}
            _ => return Err(SeatPageError::PromptSequenceMismatch { id: entry.id.get() }),
        }
    }
    let mut previous_seq = 0;
    for result in page.results.iter() {
        if result.seq == 0 || result.seq <= previous_seq {
            return Err(SeatPageError::UnorderedResults);
        }
        ids[id_count] = result.id.get();
        id_count += 1;
        seat_terminal_publishable(result.outcome)?;
        previous_seq = result.seq;
    }
    for (index, result_value) in page.result_values.iter().enumerate() {
        if result_value.value.len() > MAX_SEAT_BINARY_BYTES {
            return Err(SeatPageError::ResultValueTooLarge {
                len: result_value.value.len(),
            });
        }
        if page.result_values[..index]
            .iter()
            .any(|previous| previous.id == result_value.id)
        {
            return Err(SeatPageError::DuplicateResultValue {
                id: result_value.id.get(),
            });
        }
        let mut matching = page
            .results
            .iter()
            .filter(|result| result.id == result_value.id);
        let result = matching
            .next()
            .ok_or(SeatPageError::MissingTerminalResult {
                id: result_value.id.get(),
            })?;
        if matching.next().is_some() {
            return Err(SeatPageError::AmbiguousTerminalResult {
                id: result_value.id.get(),
            });
        }
        if !matches!(result.outcome, SeatTerminal::Done | SeatTerminal::Failed) {
            return Err(SeatPageError::ResultValueOutcome {
                id: result_value.id.get(),
            });
        }
    }
    ids[..id_count].sort_unstable();
    if ids[..id_count].windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(SeatPageError::DuplicateSeatId);
    }
    Ok(())
}

/// One coalesced, sequence-ordered in-isolate mirror revocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeatMirrorRevoke {
    /// Revoked mirror owner.
    pub owner: RequestId,
    /// Monotone revoke sequence, retried until acknowledged by the page.
    pub seq: u64,
}

/// One decoded start; the slot owner constructs its generation-bound stamp.
#[derive(Debug, Clone, PartialEq)]
pub struct SeatStartInput {
    /// Nonzero request identity.
    pub id: RequestId,
    /// Nearest seat ancestor; `None` denotes a proven root.
    pub parent: Option<RequestId>,
    /// Raw RunKey run stamp; slot construction binds it.
    pub run: u64,
    /// Raw RunKey session-epoch stamp; not a login-session seat entry.
    pub session: u64,
    /// Semantic family of the request.
    pub kind: NativeKind,
    /// Family arguments decoded from the wire.
    pub args: SeatStartArgs,
}

/// Owned typed arguments decoded from a native start.
#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::large_enum_variant,
    reason = "the bounded start batch owns its typed bank arguments without an additional allocation per decoded request"
)]
pub enum SeatStartArgs {
    /// No typed argument schema is active for this family.
    Empty,
    /// Decoded gather settings.
    Gather(crate::native::SettingsBag),
    /// Decoded combat-session request.
    Combat(crate::api_combat::CombatSessionRequest),
    /// Decoded quest-progress name.
    QuestProgress(String),
    /// Decoded typed banking operation.
    Bank(crate::bank::request::BankSeatArgs),
}

/// One decoded decision answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatDecisionInput {
    /// Request this decision answers.
    pub of: RequestId,
    /// Nonzero per-request prompt sequence.
    pub sequence: DecisionSequence,
    /// Owned bounded binary structured-clone answer.
    pub answer: Vec<u8>,
}

/// One decoded cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeatCancelDecoded {
    /// Request whose callback or subtree is cancelled.
    pub of: RequestId,
    /// Typed cancellation reason.
    pub reason: CallbackCancelReason,
}

/// One decoded legacy interaction row with required captured lineage.
#[derive(Debug)]
pub struct SeatInteractDecoded {
    /// Required physical wire lineage; `None` is a proven Root.
    pub lineage: Option<RequestId>,
    /// In-isolate machine mirror that owned this emission, if any.
    pub mirror_owner: Option<RequestId>,
    /// Request payload to dispatch with the captured lineage.
    pub request: crate::shim::InteractReq,
}

/// Fully decoded wire batch, including controls, mirror revocations, and rows.
#[derive(Debug)]
pub struct DecodedSeatBatch {
    /// Highest retained terminal consumed by this batch.
    pub consumed_terminal_seq: u64,
    /// Decoded pending starts.
    pub starts: Vec<SeatStartInput>,
    /// Decoded carried decisions.
    pub decisions: Vec<SeatDecisionInput>,
    /// Decoded carried cancellations.
    pub cancels: Vec<SeatCancelDecoded>,
    /// Decoded, sequence-ordered mirror revocations.
    pub mirror_revokes: Vec<SeatMirrorRevoke>,
    /// Decoded interaction rows with their captured lineage.
    pub reqs: Vec<SeatInteractDecoded>,
}

/// Wait facts that a guardian-held seat batch carries as one exact summary.
///
/// The watchdog adds one for each enqueue and subtracts one, saturating at
/// zero, for each settle, in event order (`watchdog.rs` `on_wait_enqueued`
/// and `on_wait_settled`). Any sequence of those steps maps a starting count
/// `c` to `max(c + d, m)` with `d <= m` and `m >= 0`. This type keeps that map
/// in its normal form: `settles` saturating subtractions, then `enqueues`
/// additions (`m = enqueues`, `d = enqueues - settles`). The summary has a
/// fixed size for any hold length.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HeldWaitFacts {
    /// Leading `WaitSettled` steps (`m - d`).
    pub settles: u64,
    /// Trailing `WaitEnqueued` steps (`m`).
    pub enqueues: u64,
    /// At least one wait fact was folded. Each fact stamps the scheduler.
    pub stamped: bool,
}

impl HeldWaitFacts {
    /// Fold one `WaitEnqueued` after the summarized facts.
    pub fn enqueue(&mut self) {
        self.enqueues = self.enqueues.saturating_add(1);
        self.stamped = true;
    }

    /// Fold one `WaitSettled` after the summarized facts. A settle after an
    /// enqueue cancels it exactly: `max(x + 1 - 1, 0) = x` for `x >= 0`.
    pub fn settle(&mut self) {
        if self.enqueues > 0 {
            self.enqueues -= 1;
        } else {
            self.settles = self.settles.saturating_add(1);
        }
        self.stamped = true;
    }

    /// The summary of these facts followed by `later`.
    pub fn then(self, later: Self) -> Self {
        let cancelled = self.enqueues.min(later.settles);
        Self {
            settles: self.settles.saturating_add(later.settles - cancelled),
            enqueues: (self.enqueues - cancelled).saturating_add(later.enqueues),
            stamped: self.stamped || later.stamped,
        }
    }

    /// The rows that reproduce the summarized facts on the watchdog: the
    /// settles, then the enqueues. When the facts cancel to the identity map,
    /// one enqueue and settle pair keeps the scheduler stamp and leaves the
    /// count unchanged.
    pub fn replay(self) -> impl Iterator<Item = crate::shim::InteractReq> {
        let pair = u64::from(self.stamped && self.settles == 0 && self.enqueues == 0);
        std::iter::repeat_n(
            crate::shim::InteractReq::WaitSettled,
            row_count(self.settles),
        )
        .chain(std::iter::repeat_n(
            crate::shim::InteractReq::WaitEnqueued,
            row_count(self.enqueues + pair),
        ))
        .chain(std::iter::repeat_n(
            crate::shim::InteractReq::WaitSettled,
            row_count(pair),
        ))
    }

    /// Number of rows that [`Self::replay`] yields.
    pub fn rows(&self) -> u64 {
        let pair = u64::from(self.stamped && self.settles == 0 && self.enqueues == 0);
        self.settles
            .saturating_add(self.enqueues)
            .saturating_add(2 * pair)
    }
}

fn row_count(rows: u64) -> usize {
    usize::try_from(rows).unwrap_or(usize::MAX)
}

#[derive(Debug)]
pub struct QueuedSeatBatch {
    /// The isolate session-work generation that produced this batch.
    pub generation: u64,
    /// Push-time walk outcome sequence shared by this physical batch.
    pub observed_walk_outcome_seq: u64,
    /// Decoded typed controls and gameplay rows.
    pub batch: DecodedSeatBatch,
    /// Local machine phases and terminals from this same producer turn.
    pub machine_events: Vec<SeatMachineEvent>,
    /// Wait facts folded out of `batch.reqs` while the seat was held.
    pub held_waits: HeldWaitFacts,
    /// This is the batch that a guardian hold retained. After the hold, the
    /// host hands it to the seat in pages that one take carries whole.
    pub held_fold: bool,
}

impl QueuedSeatBatch {
    /// Rows this batch carries, with each folded wait fact counted as one row.
    pub fn rows(&self) -> usize {
        self.batch
            .reqs
            .len()
            .saturating_add(usize::try_from(self.held_waits.rows()).unwrap_or(usize::MAX))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeatMachineEventKind {
    Phase(crate::native::NativePhase),
    Done,
    Failed,
    Aborted(api::script_contract::AbortReason),
    ManualWalk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeatMachineEvent {
    pub id: RequestId,
    pub kind: SeatMachineEventKind,
    pub admitted: bool,
}
