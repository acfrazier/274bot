//! Process-unavailability conditions (F0 item 2: contract §26 §3; D4).
//!
//! Three closed codes name a process service that will not serve again
//! before the process restarts: [`ProcessServiceCode::ReactorUnavailable`],
//! [`ProcessServiceCode::ResolverUnavailable`] and
//! [`ProcessServiceCode::WorkAdmissionUnavailable`]. Each is published once
//! per process incarnation as a [`ProcessServiceCondition`] and stays latched
//! in the [`ProcessConditions`] ledger until the process exits; nothing a
//! consumer does clears it. A command or job that needs the failed
//! capability is refused with an [`AdmissionRefusal`] carrying the
//! [`UnavailableRefusal`] that names the condition and keeps the request's
//! own identity; an accepted command the outage makes impossible gets one
//! [`UnavailableTerminal`]. A failure report settles result custody only:
//! it is not a transport-retirement acknowledgement, so a
//! [`StrandedCustody`] keeps its charge until the worker really returns.
//!
//! The session-side projections of an outage are
//! [`crate::lifecycle::SessionEventOut::Unavailable`] (a slot's refused next
//! step, intent preserved) and
//! [`crate::lifecycle::SessionEndCause::ServiceUnavailable`] (a live session
//! that lost its authority); intent precedence (§3.2 "Logout/Stop/Remove
//! intent precedes or races the condition") is the mailbox's, which never
//! turns a Logout into a login or a Remove into a retry, while the ledger
//! refuses the retry the preserved intent would otherwise schedule.
//!
//! The neutral `ErrorCode` of F0 item 7 embeds [`ProcessServiceCode`] for
//! these three codes; no fourth process-service code exists without an
//! acknowledged contract delta (§26 §3.3).
//!
//! Today no such condition exists: a slot worker that panics publishes a
//! terminal status row (`crates/host-play/src/play_slots.rs:1868-1890`),
//! map and preparation failures are values (`MapCacheError`, `String`), and
//! nothing restarts a lost service (see
//! `274bot-evidence/F0-CORE/identity/current-representations.md` §6).

use std::fmt;

use crate::identity::{ProcessIncarnation, ProcessTime, RequestId, Scope};

/// The closed process-service availability codes (§26 §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProcessServiceCode {
    /// The game transport service: no new game connection, login, reconnect
    /// or transport-dependent acceptance; existing sessions lose usable
    /// network authority.
    ReactorUnavailable,
    /// The name-resolution service: no new resolve admission, including
    /// cache hits through that service; already-resolved addresses and
    /// established sessions need no new resolution.
    ResolverUnavailable,
    /// The shared background work service: no new preparation, route,
    /// inspection, bank-pick, bake or decode admission; unrelated services
    /// continue.
    WorkAdmissionUnavailable,
}

impl ProcessServiceCode {
    /// All three codes.
    pub const ALL: [ProcessServiceCode; 3] = [
        ProcessServiceCode::ReactorUnavailable,
        ProcessServiceCode::ResolverUnavailable,
        ProcessServiceCode::WorkAdmissionUnavailable,
    ];

    /// The stable code string carried by logs and reports.
    pub const fn code(self) -> &'static str {
        match self {
            Self::ReactorUnavailable => "ReactorUnavailable",
            Self::ResolverUnavailable => "ResolverUnavailable",
            Self::WorkAdmissionUnavailable => "WorkAdmissionUnavailable",
        }
    }

    /// The exact operator text (§26 §3.1).
    pub const fn operator_text(self) -> &'static str {
        match self {
            Self::ReactorUnavailable => {
                "Game transport is unavailable. Restart the application process. \
                 Retrying login or restarting a script will not restore transport."
            }
            Self::ResolverUnavailable => {
                "Name resolution is unavailable. Restart the application process. \
                 Retrying the request or restarting a script will not restore name resolution."
            }
            Self::WorkAdmissionUnavailable => {
                "Background work is unavailable. Restart the application process. \
                 Retrying the job or restarting a script will not restore background work."
            }
        }
    }

    /// Whether `capability` depends on the failed service (§26 §3.1
    /// "affected capability"; §3.2 consumer-event table).
    pub const fn affects(self, capability: ProcessCapability) -> bool {
        match (self, capability) {
            (
                Self::ReactorUnavailable,
                ProcessCapability::NewConnection
                | ProcessCapability::Login
                | ProcessCapability::Reconnect
                | ProcessCapability::NetworkEffect,
            ) => true,
            (Self::ReactorUnavailable, _) => false,
            (Self::ResolverUnavailable, ProcessCapability::NameResolution) => true,
            (Self::ResolverUnavailable, _) => false,
            (Self::WorkAdmissionUnavailable, ProcessCapability::BackgroundWork) => true,
            (Self::WorkAdmissionUnavailable, _) => false,
        }
    }

    /// Restart is always required for these codes: an invariant of the
    /// type, not a recovery choice.
    pub const fn restart_required(self) -> bool {
        true
    }

    /// The recoverability class: never transient (§26 §2.2 "Full means
    /// temporary capacity pressure; Unavailable means process restart").
    pub const fn recoverability(self) -> Recoverability {
        Recoverability::RestartRequired
    }
}

impl fmt::Display for ProcessServiceCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// How a refusal or failure can be recovered from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Recoverability {
    /// Temporary capacity pressure: retry after a capacity notification or
    /// a later ordinary demand turn.
    TransientCapacity,
    /// Only a process restart restores the capability; automatic retries
    /// that need it are suppressed until then.
    RestartRequired,
}

/// A capability an operation may need from a process service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProcessCapability {
    /// Opening a new game connection.
    NewConnection,
    /// A fresh login handshake.
    Login,
    /// A reconnect handshake.
    Reconnect,
    /// Sending on an established session (network effect authority).
    NetworkEffect,
    /// Resolving a host name, including through the resolver's cache.
    NameResolution,
    /// Admitting a background job.
    BackgroundWork,
    /// Reading an already-resolved address.
    ResolvedAddress,
    /// Local, non-network work on an established session or a slot.
    LocalOnly,
}

impl ProcessCapability {
    /// Every capability, for exhaustive checks.
    pub const ALL: [ProcessCapability; 8] = [
        ProcessCapability::NewConnection,
        ProcessCapability::Login,
        ProcessCapability::Reconnect,
        ProcessCapability::NetworkEffect,
        ProcessCapability::NameResolution,
        ProcessCapability::BackgroundWork,
        ProcessCapability::ResolvedAddress,
        ProcessCapability::LocalOnly,
    ];
}

/// Why the service became unavailable (§26 §3.1 `cause`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConditionCause {
    /// The service never started.
    StartFailed,
    /// The service panicked.
    Panicked,
    /// The gate determined service progress unavailable; it does not assert
    /// that an OS thread exited.
    ProgressLost,
}

impl ConditionCause {
    /// The stable cause string.
    pub const fn label(self) -> &'static str {
        match self {
            Self::StartFailed => "start_failed",
            Self::Panicked => "panicked",
            Self::ProgressLost => "progress_lost",
        }
    }
}

/// `(process_incarnation, code)`: at most one latched condition per code per
/// incarnation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConditionId {
    incarnation: ProcessIncarnation,
    code: ProcessServiceCode,
}

impl ConditionId {
    /// The id for `code` in `incarnation`.
    pub const fn new(incarnation: ProcessIncarnation, code: ProcessServiceCode) -> Self {
        Self { incarnation, code }
    }

    /// The incarnation.
    pub const fn incarnation(self) -> ProcessIncarnation {
        self.incarnation
    }

    /// The code.
    pub const fn code(self) -> ProcessServiceCode {
        self.code
    }
}

/// Upper bound of a [`Diagnostic`] in bytes.
pub const DIAGNOSTIC_MAX_BYTES: usize = 256;

/// Optional redacted diagnostic text, at most [`DIAGNOSTIC_MAX_BYTES`] of
/// UTF-8. Stable code and cause control behaviour; this text does not.
#[derive(Debug, PartialEq, Eq)]
pub struct Diagnostic(Box<str>);

impl Diagnostic {
    /// Build a diagnostic from already-redacted text, truncating on a char
    /// boundary to the bound. The caller redacts credentials and private
    /// payloads through [`crate::hostlog::redact`] before calling.
    pub fn bounded(text: &str) -> Self {
        let mut end = text.len().min(DIAGNOSTIC_MAX_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        Self(Box::from(&text[..end]))
    }

    /// The text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A latched process-service condition (§26 §3.1): accessor-based, with no
/// per-slot fan-out, Client pointer, reactor handle, cancellation token or
/// transport acknowledgement.
#[derive(Debug, PartialEq, Eq)]
pub struct ProcessServiceCondition {
    id: ConditionId,
    cause: ConditionCause,
    detected_at: ProcessTime,
    diagnostic: Option<Diagnostic>,
}

impl ProcessServiceCondition {
    /// A condition detected at `detected_at` (process time) with `cause`.
    pub const fn new(
        id: ConditionId,
        cause: ConditionCause,
        detected_at: ProcessTime,
        diagnostic: Option<Diagnostic>,
    ) -> Self {
        Self {
            id,
            cause,
            detected_at,
            diagnostic,
        }
    }

    /// `(process_incarnation, code)`.
    pub const fn id(&self) -> ConditionId {
        self.id
    }

    /// The code; it determines the service.
    pub const fn code(&self) -> ProcessServiceCode {
        self.id.code
    }

    /// Why.
    pub const fn cause(&self) -> ConditionCause {
        self.cause
    }

    /// Process time of detection.
    pub const fn detected_at(&self) -> ProcessTime {
        self.detected_at
    }

    /// Always true for these codes.
    pub const fn restart_required(&self) -> bool {
        self.id.code.restart_required()
    }

    /// The bounded, redacted diagnostic, if any.
    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_ref().map(Diagnostic::as_str)
    }

    /// The exact operator text for this condition's code.
    pub const fn operator_text(&self) -> &'static str {
        self.id.code.operator_text()
    }
}

impl fmt::Display for ProcessServiceCondition {
    /// The one process-condition log record (§26 §3.3): identity, code, cause
    /// and detection time.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "process condition code={} cause={} detected_at_ms={} incarnation={:?} restart_required=true",
            self.id.code.code(),
            self.cause.label(),
            self.detected_at.as_duration().as_millis(),
            self.id.incarnation
        )?;
        if let Some(diagnostic) = &self.diagnostic {
            write!(
                f,
                " diagnostic={diagnostic:?}",
                diagnostic = diagnostic.as_str()
            )?;
        }
        Ok(())
    }
}

/// Why a publication into the ledger was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PublishError {
    /// The condition belongs to another process incarnation; composition
    /// fences obsolete-incarnation notifications before publication.
    #[error("process condition from another incarnation; not current health")]
    StaleIncarnation,
    /// This code is already latched for this incarnation; the first
    /// publication stands.
    #[error("process condition {0} already latched for this incarnation")]
    AlreadyLatched(ProcessServiceCode),
}

/// Upper bound of entries a snapshot borrows (§26 §3.1: at most three).
pub const MAX_CONDITIONS: usize = ProcessServiceCode::ALL.len();

/// The per-incarnation ledger of latched conditions: at most one per code,
/// never cleared before process exit. T1's composition owns it; T3 borrows
/// its snapshot.
#[derive(Debug)]
pub struct ProcessConditions {
    incarnation: ProcessIncarnation,
    entries: [Option<ProcessServiceCondition>; MAX_CONDITIONS],
}

impl ProcessConditions {
    /// An empty ledger for `incarnation`.
    pub const fn new(incarnation: ProcessIncarnation) -> Self {
        Self {
            incarnation,
            entries: [None, None, None],
        }
    }

    /// The incarnation this ledger describes.
    pub const fn incarnation(&self) -> ProcessIncarnation {
        self.incarnation
    }

    const fn index(code: ProcessServiceCode) -> usize {
        match code {
            ProcessServiceCode::ReactorUnavailable => 0,
            ProcessServiceCode::ResolverUnavailable => 1,
            ProcessServiceCode::WorkAdmissionUnavailable => 2,
        }
    }

    /// Latch `condition`. Refused for another incarnation or a code already
    /// latched; a refusal changes nothing.
    pub fn publish(&mut self, condition: ProcessServiceCondition) -> Result<(), PublishError> {
        if condition.id.incarnation != self.incarnation {
            return Err(PublishError::StaleIncarnation);
        }
        let slot = &mut self.entries[Self::index(condition.code())];
        if slot.is_some() {
            return Err(PublishError::AlreadyLatched(condition.code()));
        }
        *slot = Some(condition);
        Ok(())
    }

    /// The latched condition for `code`, if any.
    pub fn latched(&self, code: ProcessServiceCode) -> Option<&ProcessServiceCondition> {
        self.entries[Self::index(code)].as_ref()
    }

    /// Whether any latched condition affects `capability`.
    pub fn blocks(&self, capability: ProcessCapability) -> Option<&ProcessServiceCondition> {
        self.entries
            .iter()
            .flatten()
            .find(|condition| condition.code().affects(capability))
    }

    /// Every latched condition, oldest code first; at most three.
    pub fn snapshot(&self) -> impl Iterator<Item = &ProcessServiceCondition> {
        self.entries.iter().flatten()
    }

    /// Decide admission of an operation needing `capability` for the request
    /// `identity`: refused with the exact condition while latched.
    pub fn admit(
        &self,
        capability: ProcessCapability,
        request: Option<RequestId>,
        scope: Scope,
    ) -> Result<(), AdmissionRefusal> {
        match self.blocks(capability) {
            Some(condition) => Err(AdmissionRefusal::Unavailable(UnavailableRefusal {
                condition: condition.id,
                capability,
                request,
                scope,
            })),
            None => Ok(()),
        }
    }

    /// Whether an automatic retry needing `capability` may be scheduled:
    /// never while the capability is blocked (§26 §3.2: suppress until
    /// process restart).
    pub fn retry_allowed(&self, capability: ProcessCapability) -> bool {
        self.blocks(capability).is_none()
    }
}

/// Why the ledger refused to admit a new command or job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionRefusal {
    /// The capability's process service is unavailable until the process
    /// restarts (§26 §3.2 table, row 1).
    #[error("{0}")]
    Unavailable(UnavailableRefusal),
}

impl AdmissionRefusal {
    /// The refusal's data.
    pub const fn refusal(&self) -> &UnavailableRefusal {
        match self {
            Self::Unavailable(refusal) => refusal,
        }
    }
}

/// A new command or job refused because its capability is unavailable
/// (§26 §3.2 table, row 1): it is not Pending, not transient `Full` and not
/// Completed; it keeps the originating identity. Reusable data: other
/// domains' refusal enums (`try_submit`'s `Unavailable`, a command report)
/// embed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnavailableRefusal {
    condition: ConditionId,
    capability: ProcessCapability,
    request: Option<RequestId>,
    scope: Scope,
}

impl UnavailableRefusal {
    /// The latched condition.
    pub const fn condition(&self) -> ConditionId {
        self.condition
    }

    /// The capability the operation needed.
    pub const fn capability(&self) -> ProcessCapability {
        self.capability
    }

    /// The refused request's id, if it had one.
    pub const fn request(&self) -> Option<RequestId> {
        self.request
    }

    /// The refused request's scope.
    pub const fn scope(&self) -> Scope {
        self.scope
    }

    /// Never transient.
    pub const fn recoverability(&self) -> Recoverability {
        self.condition.code.recoverability()
    }

    /// The exact operator text.
    pub const fn operator_text(&self) -> &'static str {
        self.condition.code.operator_text()
    }
}

impl fmt::Display for UnavailableRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({:?} refused for {:?})",
            self.condition.code.operator_text(),
            self.capability,
            self.scope
        )
    }
}

/// The one logical Failed/Unavailable terminal an accepted command or job
/// receives when the outage makes it impossible (§26 §3.2 table, row 2). It
/// settles result custody and references the condition; it acknowledges no
/// retirement and releases no worker credit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnavailableTerminal {
    condition: ConditionId,
    request: RequestId,
    scope: Scope,
}

impl UnavailableTerminal {
    /// The terminal for `request` in `scope`, referencing `condition`.
    pub const fn new(condition: ConditionId, request: RequestId, scope: Scope) -> Self {
        Self {
            condition,
            request,
            scope,
        }
    }

    /// The latched condition.
    pub const fn condition(&self) -> ConditionId {
        self.condition
    }

    /// The request this terminal settles.
    pub const fn request(&self) -> RequestId {
        self.request
    }

    /// The request's scope.
    pub const fn scope(&self) -> Scope {
        self.scope
    }

    /// The exact operator text.
    pub const fn operator_text(&self) -> &'static str {
        self.condition.code.operator_text()
    }
}

/// Custody of a worker that may still exist after its service failed
/// (§26 §3.2): reporting a logical failure settles the result cell; the
/// worker's credit stays charged until the worker actually returns, and no
/// detection deadline can stand in for that return.
#[derive(Debug, PartialEq, Eq)]
pub struct StrandedCustody {
    request: RequestId,
    scope: Scope,
    terminal: Option<UnavailableTerminal>,
    returned: bool,
}

/// Why a stranded worker's terminal could not be published or its credit
/// released.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum CustodyError {
    /// The terminal was already published; it cannot be rewritten.
    #[error("unavailable terminal already published")]
    AlreadyFailed,
    /// The worker has not returned, so its credit stays charged; the custody
    /// comes back with the refusal.
    #[error("worker for request {} still charged: no deadline or report stands in for its return", .0.request().get())]
    WorkerStillCharged(StrandedCustody),
}

impl StrandedCustody {
    /// Custody of the worker serving `request` in `scope`.
    pub const fn new(request: RequestId, scope: Scope) -> Self {
        Self {
            request,
            scope,
            terminal: None,
            returned: false,
        }
    }

    /// The request the worker serves.
    pub const fn request(&self) -> RequestId {
        self.request
    }

    /// The request's scope.
    pub const fn scope(&self) -> Scope {
        self.scope
    }

    /// Publish the one logical Failed/Unavailable terminal. The credit stays
    /// charged.
    pub fn fail_unavailable(
        &mut self,
        condition: ConditionId,
    ) -> Result<UnavailableTerminal, CustodyError> {
        if self.terminal.is_some() {
            return Err(CustodyError::AlreadyFailed);
        }
        let terminal = UnavailableTerminal::new(condition, self.request, self.scope);
        self.terminal = Some(terminal);
        Ok(terminal)
    }

    /// The published terminal, if any.
    pub const fn terminal(&self) -> Option<UnavailableTerminal> {
        self.terminal
    }

    /// Whether the worker's credit is still charged.
    pub const fn charged(&self) -> bool {
        !self.returned
    }

    /// Record the worker's actual return. Its late result cannot replace a
    /// published terminal.
    pub fn worker_returned(&mut self) {
        self.returned = true;
    }

    /// Release the credit. Refused with the custody handed back until the
    /// worker returned.
    pub fn release(self) -> Result<(), CustodyError> {
        if self.returned {
            Ok(())
        } else {
            Err(CustodyError::WorkerStillCharged(self))
        }
    }
}

#[cfg(test)]
#[path = "process_condition_tests.rs"]
mod tests;
