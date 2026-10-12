//! Neutral local-state values (F0 item 7: contract §26 §1 item 7, D10 and D13
//! in `00-contract.md`, `21-census-ownership-dag.md` §4).
//!
//! Frozen here:
//!
//! - the [`LocalStateDomain`]s with their canonical codes (`preferences`,
//!   `bank-hints`, `loadouts-references`, `knowledge`: `23-tables-r3.md`
//!   §5.1; D10's preferences, bank hints, loadouts and quest knowledge);
//! - the durable account and content identities: [`AccountKey`] (launch
//!   profile plus the canonical validated login identity, never a display
//!   label, slot index or current run key) and [`ContentScope`] (selected
//!   engine revision and content identity);
//! - versions ([`StoreVersion`]), the durable key ([`DomainIdentity`]),
//!   immutable read leases ([`DomainLease`]), typed edits
//!   ([`PreferencesEdit`], [`LoadoutEdit`], [`BankHint`]) and durable
//!   receipts ([`DurableReceipt`]);
//! - source-proven quest facts ([`KnowledgeRecord`]);
//! - the neutral classification ([`ErrorCode`], embedding
//!   [`ProcessServiceCode`](crate::process_condition::ProcessServiceCode)
//!   for the three §26 §3.1 process-service codes) with
//!   [`recoverability`](ErrorCode::recoverability) and bounded fields;
//! - the post-commit audit record ([`AuditCommit`]).
//!
//! A durable write settles as a [`WriteSettlement`]: an
//! [`identity::Settlement`](crate::identity::Settlement) that is completed
//! with its receipt when durable, failed with its [`ErrorCode`] on conflict
//! or failure, and cancelled when the submission was withdrawn. Receipts key
//! by [`DomainIdentity`], never by a [`SlotKey`](crate::identity::SlotKey)
//! alone, and carry only [`DurableTimestamp`](crate::identity::DurableTimestamp)
//! time (§21 §4.3–§4.4; F0-1/2 interface note).
//!
//! The capability shape [`LocalStatePort`] names the neutral operations a
//! domain port offers (versioned read leases, nonblocking typed submission,
//! request and receipt observation); its functional current-file injection is
//! T5.2, and no SQLite cutover happens at F0. Behaviour (loadout selection,
//! planning, store I/O, settings merge) stays in `script`.

use std::fmt;

use crate::bank_memory::{login_identity, HintRowsError, MAX_HINT_ROWS};
use crate::identity::{DurableTimestamp, RequestId, Settlement};
use crate::loadout::Loadout;
use crate::process_condition::{ProcessServiceCode, Recoverability};

/// A local-state domain, by its canonical code (`23-tables-r3.md` §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocalStateDomain {
    /// Operator preferences.
    Preferences,
    /// One account's last-seen bank rows.
    BankHints,
    /// Eligible operator loadouts and catalogue reference values.
    LoadoutsReferences,
    /// Source-proven quest knowledge.
    Knowledge,
}

impl LocalStateDomain {
    /// Every domain, for exhaustive checks.
    pub const ALL: [LocalStateDomain; 4] = [
        LocalStateDomain::Preferences,
        LocalStateDomain::BankHints,
        LocalStateDomain::LoadoutsReferences,
        LocalStateDomain::Knowledge,
    ];

    /// The canonical code: `preferences`, `bank-hints`,
    /// `loadouts-references` or `knowledge`.
    pub const fn code(self) -> &'static str {
        match self {
            LocalStateDomain::Preferences => "preferences",
            LocalStateDomain::BankHints => "bank-hints",
            LocalStateDomain::LoadoutsReferences => "loadouts-references",
            LocalStateDomain::Knowledge => "knowledge",
        }
    }

    /// Parse a canonical code, or `None` for anything else.
    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "preferences" => Some(LocalStateDomain::Preferences),
            "bank-hints" => Some(LocalStateDomain::BankHints),
            "loadouts-references" => Some(LocalStateDomain::LoadoutsReferences),
            "knowledge" => Some(LocalStateDomain::Knowledge),
            _ => None,
        }
    }
}

impl fmt::Display for LocalStateDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// Whether `value` is one safe profile or content component: non-empty, not
/// `.` or `..`, ASCII letters, digits, `.`, `_` and `-` only. This is the
/// same rule `vault` loads profiles under
/// (`crates/vault/src/private_file.rs:36-43`); an `api` value cannot depend
/// on `vault`, so the rule is restated here rather than re-checked there.
fn is_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// Longest profile name a key carries.
pub const MAX_PROFILE_CHARS: usize = 64;

/// Longest content identity a scope carries.
pub const MAX_CONTENT_CHARS: usize = 128;

/// Why an [`AccountKey`] or [`ContentScope`] was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    /// The profile is empty, too long, or not one safe path component.
    #[error("profile is not a safe component")]
    UnsafeProfile,
    /// The name is empty, too long, carries a hostile byte, or spells no
    /// account at all.
    #[error("name is not a login identity")]
    UnsafeAccount,
    /// The content identity is empty, too long, or not one safe component.
    #[error("content is not a safe component")]
    UnsafeContent,
}

/// The durable account identity (§21 §4.1): the launch profile (server
/// namespace) plus the canonical validated login identity. Never a display
/// label, slot index or current run key: renaming a display label does not
/// invent another account.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountKey {
    profile: String,
    account: String,
}

impl AccountKey {
    /// The longest login name the login block carries, after which the
    /// server refuses the name before decoding it
    /// (`crates/script/src/bank_hints.rs:37-42`).
    pub const MAX_ACCOUNT_CHARS: usize = 12;

    /// Key `name` under `profile`. The profile must meet the rule profiles
    /// load under; the name is canonicalized by the client's own decoder
    /// ([`login_identity`]) exactly as the bank hint path spells it
    /// (`crates/script/src/bank_hints.rs:120-159`): a name with a hostile
    /// byte or more than twelve characters as typed, or one the hash spells
    /// as no account at all, is refused rather than persisted under an
    /// arbitrary identity. `Alice Smith`, `alice_smith` and ` alice smith`
    /// are one key.
    pub fn try_new(profile: &str, name: &str) -> Result<Self, IdentityError> {
        if profile.len() > MAX_PROFILE_CHARS || !is_component(profile) {
            return Err(IdentityError::UnsafeProfile);
        }
        if name.len() > Self::MAX_ACCOUNT_CHARS
            || !name
                .bytes()
                .all(|byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b' ' | b'_'))
        {
            return Err(IdentityError::UnsafeAccount);
        }
        let Some(account) = login_identity(name) else {
            return Err(IdentityError::UnsafeAccount);
        };
        Ok(Self {
            profile: profile.to_string(),
            account,
        })
    }

    /// The launch profile (server namespace).
    pub fn profile(&self) -> &str {
        &self.profile
    }

    /// The canonical validated login identity.
    pub fn account(&self) -> &str {
        &self.account
    }
}

impl fmt::Display for AccountKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.profile, self.account)
    }
}

/// The durable content identity (§21 §4.1): the selected engine revision and
/// the content identity the observations were made under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentScope {
    revision: client::io::ClientRevision,
    content: String,
}

impl ContentScope {
    /// Scope observations to `revision` and `content`.
    pub fn try_new(
        revision: client::io::ClientRevision,
        content: &str,
    ) -> Result<Self, IdentityError> {
        if content.len() > MAX_CONTENT_CHARS || !is_component(content) {
            return Err(IdentityError::UnsafeContent);
        }
        Ok(Self {
            revision,
            content: content.to_string(),
        })
    }

    /// The selected engine revision.
    pub fn revision(&self) -> client::io::ClientRevision {
        self.revision
    }

    /// The content identity.
    pub fn content(&self) -> &str {
        &self.content
    }
}

/// One domain's version: expected-version writes compare against it, and a
/// durable receipt carries the version that became durable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct StoreVersion(u64);

impl StoreVersion {
    /// The version before any write.
    pub const INITIAL: Self = Self(0);

    /// The raw version.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The next version, or `None` when the space is exhausted: versions
    /// refuse instead of wrapping or reusing a value.
    pub const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(next) => Some(Self(next)),
            None => None,
        }
    }
}

impl fmt::Display for StoreVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// The durable key of one domain's state (§21 §4.3): domain, account and
/// content scope. There is deliberately no slot field: retiring a slot
/// revokes future submission authority but never erases, renames or
/// re-keys an accepted durable write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainIdentity {
    domain: LocalStateDomain,
    account: AccountKey,
    scope: ContentScope,
}

impl DomainIdentity {
    /// Key `domain` for `account` under `scope`.
    pub fn new(domain: LocalStateDomain, account: AccountKey, scope: ContentScope) -> Self {
        Self {
            domain,
            account,
            scope,
        }
    }

    /// The domain.
    pub fn domain(&self) -> LocalStateDomain {
        self.domain
    }

    /// The account.
    pub fn account(&self) -> &AccountKey {
        &self.account
    }

    /// The content scope.
    pub fn scope(&self) -> &ContentScope {
        &self.scope
    }
}

/// Why a neutral value was refused, before admission. Every field is
/// bounded: no free text ever drives a classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum ValidationReason {
    /// A required name or key is empty.
    #[error("empty name")]
    Empty,
    /// A name, key or value exceeds its bound.
    #[error("value too long")]
    TooLong,
    /// A name or key carries a byte outside its alphabet.
    #[error("unsafe characters")]
    BadChars,
    /// A document's schema or content marker is not the frozen one.
    #[error("schema mismatch")]
    Schema,
    /// A document carries another account, profile or scope.
    #[error("identity mismatch")]
    Identity,
    /// More rows than the domain cap.
    #[error("too many rows")]
    TooManyRows,
    /// A row count that is not positive.
    #[error("non-positive count")]
    NonPositiveCount,
    /// Row ids out of order.
    #[error("ids out of order")]
    Unsorted,
    /// A duplicate row id.
    #[error("duplicate id")]
    DuplicateId,
}

impl From<HintRowsError> for ValidationReason {
    fn from(error: HintRowsError) -> Self {
        match error {
            HintRowsError::TooManyRows => ValidationReason::TooManyRows,
            HintRowsError::NonPositiveCount => ValidationReason::NonPositiveCount,
            HintRowsError::Unsorted => ValidationReason::Unsorted,
            HintRowsError::DuplicateId => ValidationReason::DuplicateId,
        }
    }
}

/// The neutral classification of a local-state refusal or failure (D13;
/// §26 §3.1–§3.3). Per-domain errors keep their typed stage and cause where
/// available; this is the neutral projection scripts and reports match on
/// instead of text. All fields are bounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum ErrorCode {
    /// The submission was refused before admission: fix the input, never
    /// restart the process for it.
    #[error("invalid {domain} value: {reason}")]
    Validation {
        /// The domain that refused the value.
        domain: LocalStateDomain,
        /// Why.
        reason: ValidationReason,
    },
    /// An expected-version write lost: another writer committed first.
    /// Retry against the found version.
    #[error("version conflict: expected {expected}, found {found}")]
    Conflict {
        /// The version the submission was written against.
        expected: StoreVersion,
        /// The version that is actually current.
        found: StoreVersion,
    },
    /// The cache is still hydrating (cold start, import or readback
    /// pending): retry on a later turn, never read the disk on a tick.
    #[error("{domain} is not ready")]
    NotReady {
        /// The domain that is still hydrating.
        domain: LocalStateDomain,
    },
    /// A bounded submission was refused for capacity: retry after a later
    /// ordinary demand turn. Acceptance is never durability.
    #[error("{domain} is busy")]
    Busy {
        /// The domain that refused the submission.
        domain: LocalStateDomain,
    },
    /// A process service is unavailable. Retries do not restore it; only a
    /// process restart does (§26 §3.1 exact text via
    /// [`ProcessServiceCode::operator_text`]).
    #[error("{0}")]
    ProcessService(ProcessServiceCode),
}

impl From<ProcessServiceCode> for ErrorCode {
    /// Every §26 §3.1 code embeds as the process-service classification.
    fn from(code: ProcessServiceCode) -> Self {
        ErrorCode::ProcessService(code)
    }
}

impl ErrorCode {
    /// How a refusal or failure recovers. Every process-service code is
    /// [`RestartRequired`](Recoverability::RestartRequired); bounded
    /// capacity and version states are
    /// [`TransientCapacity`](Recoverability::TransientCapacity); a
    /// validation refusal names bad input, which no process recovery
    /// heals, so it classifies as neither.
    pub const fn recoverability(self) -> Option<Recoverability> {
        match self {
            ErrorCode::ProcessService(_) => Some(Recoverability::RestartRequired),
            ErrorCode::Busy { .. } | ErrorCode::NotReady { .. } | ErrorCode::Conflict { .. } => {
                Some(Recoverability::TransientCapacity)
            }
            ErrorCode::Validation { .. } => None,
        }
    }

    /// The domain this code concerns, if it concerns one domain.
    pub const fn domain(self) -> Option<LocalStateDomain> {
        match self {
            ErrorCode::Validation { domain, .. }
            | ErrorCode::NotReady { domain }
            | ErrorCode::Busy { domain } => Some(domain),
            ErrorCode::Conflict { .. } | ErrorCode::ProcessService(_) => None,
        }
    }

    /// The exact §26 §3.1 operator text for a process-service code, or
    /// `None` for domain codes.
    pub const fn operator_text(self) -> Option<&'static str> {
        match self {
            ErrorCode::ProcessService(code) => Some(code.operator_text()),
            _ => None,
        }
    }
}

/// An immutable, bounded, versioned read-cache lease: what the state
/// service hands out for reads (§21 §4.2 row 1). The lease is a snapshot:
/// a newer commit never mutates a lease already handed out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainLease {
    identity: DomainIdentity,
    version: StoreVersion,
    at: DurableTimestamp,
}

impl DomainLease {
    /// Lease `identity` at `version`, observed `at`.
    pub fn new(identity: DomainIdentity, version: StoreVersion, at: DurableTimestamp) -> Self {
        Self {
            identity,
            version,
            at,
        }
    }

    /// The leased identity.
    pub fn identity(&self) -> &DomainIdentity {
        &self.identity
    }

    /// The leased version.
    pub fn version(&self) -> StoreVersion {
        self.version
    }

    /// When the leased rows were observed, in durable time.
    pub fn at(&self) -> DurableTimestamp {
        self.at
    }
}

/// A durable receipt: proof that one domain write became durable (§21
/// §4.3–§4.4). Keyed by [`DomainIdentity`]: [`accepts`](Self::accepts) is
/// exact equality over domain, account and scope, so a receipt for one
/// account never settles another's write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableReceipt {
    identity: DomainIdentity,
    version: StoreVersion,
    at: DurableTimestamp,
    request: Option<RequestId>,
}

impl DurableReceipt {
    /// Receipt `version` of `identity` durable `at`, settling `request`.
    pub fn new(
        identity: DomainIdentity,
        version: StoreVersion,
        at: DurableTimestamp,
        request: Option<RequestId>,
    ) -> Self {
        Self {
            identity,
            version,
            at,
            request,
        }
    }

    /// The identity this receipt settles.
    pub fn identity(&self) -> &DomainIdentity {
        &self.identity
    }

    /// The version that became durable.
    pub fn version(&self) -> StoreVersion {
        self.version
    }

    /// When it became durable, in durable time.
    pub fn at(&self) -> DurableTimestamp {
        self.at
    }

    /// The accepted request this receipt settles, if any.
    pub fn request(&self) -> Option<RequestId> {
        self.request
    }

    /// Whether this receipt settles `identity`: exact domain, account and
    /// scope equality. A different account, scope or domain is rejected,
    /// however the name was spelled.
    pub fn accepts(&self, identity: &DomainIdentity) -> bool {
        self.identity == *identity
    }
}

/// The terminal of one durable submission: completed with its receipt when
/// durable, failed with its [`ErrorCode`] on conflict or failure, cancelled
/// when the submission was withdrawn before it completed.
pub type WriteSettlement = Settlement<DurableReceipt, ErrorCode>;

/// A nonblocking typed submission that was accepted (with its request id;
/// acceptance is not durability) or refused with its code. Accepted
/// requests settle exactly once through [`WriteSettlement`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitReceipt {
    /// Accepted under this request id; observe it for the terminal.
    Accepted {
        /// The accepted request.
        request: RequestId,
    },
    /// Refused before admission; nothing was queued and no terminal will
    /// arrive for it.
    Refused(ErrorCode),
}

impl SubmitReceipt {
    /// The accepted request, if accepted.
    pub const fn request(self) -> Option<RequestId> {
        match self {
            SubmitReceipt::Accepted { request } => Some(request),
            SubmitReceipt::Refused(_) => None,
        }
    }

    /// The refusal code, if refused.
    pub const fn refusal(self) -> Option<ErrorCode> {
        match self {
            SubmitReceipt::Refused(code) => Some(code),
            SubmitReceipt::Accepted { .. } => None,
        }
    }
}

/// The neutral domain-port shape (D10; §21 §4.2): versioned read leases,
/// nonblocking typed submission, and request and receipt observation over
/// explicit [`AccountKey`] and [`ContentScope`] identity. Functional
/// current-file injection is T5.2's `I-state-port`; no implementation
/// lands at F0, and nothing here touches SQLite.
pub trait LocalStatePort {
    /// The current read lease for `identity`, or `None` while the domain
    /// is cold (typed `NotReady`, never a synchronous disk read on a
    /// tick and never fabricated empty data).
    fn lease(&self, identity: &DomainIdentity) -> Option<DomainLease>;

    /// Submit `edit` without blocking: [`SubmitReceipt::Accepted`] carries
    /// the request to observe, [`SubmitReceipt::Refused`] carries `Busy`
    /// or `NotReady`. Acceptance is not durability.
    fn submit(&self, edit: DomainEdit) -> SubmitReceipt;

    /// The terminal of an accepted `request`, or `None` for an unknown or
    /// not-yet-settled request.
    fn observe(&self, request: RequestId) -> Option<WriteSettlement>;
}

/// One typed edit, by domain. Each variant carries its [`DomainIdentity`]
/// and the [`StoreVersion`] it was written against (`None` writes blind;
/// `Some` writes only when current, so a lost race is a typed conflict
/// instead of a silent overwrite).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainEdit {
    /// An operator preference write.
    Preferences(PreferencesEdit),
    /// An operator loadout write.
    Loadout(LoadoutEdit),
    /// An observed bank hint.
    Bank(BankHint),
    /// A source-proven knowledge fact.
    Knowledge(KnowledgeRecord),
}

impl DomainEdit {
    /// The identity this edit writes.
    pub fn identity(&self) -> &DomainIdentity {
        match self {
            DomainEdit::Preferences(edit) => edit.identity(),
            DomainEdit::Loadout(edit) => edit.identity(),
            DomainEdit::Bank(edit) => edit.identity(),
            DomainEdit::Knowledge(edit) => edit.identity(),
        }
    }

    /// The version this edit was written against, if any.
    pub fn expected(&self) -> Option<StoreVersion> {
        match self {
            DomainEdit::Preferences(edit) => edit.expected(),
            DomainEdit::Loadout(edit) => edit.expected(),
            DomainEdit::Bank(edit) => edit.expected(),
            DomainEdit::Knowledge(edit) => edit.expected(),
        }
    }

    /// Whether this edit may write `current`: a blind write always may, a
    /// versioned write only when its expectation is current.
    pub fn applies_to(&self, current: StoreVersion) -> bool {
        match self {
            DomainEdit::Preferences(edit) => edit.applies_to(current),
            DomainEdit::Loadout(edit) => edit.applies_to(current),
            DomainEdit::Bank(edit) => edit.applies_to(current),
            DomainEdit::Knowledge(edit) => edit.applies_to(current),
        }
    }
}

/// Longest preference key.
pub const MAX_PREF_KEY_CHARS: usize = 128;

/// Longest preference value, in bytes.
pub const MAX_PREF_VALUE_BYTES: usize = 4096;

/// One operator preference write in the `preferences` domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreferencesEdit {
    identity: DomainIdentity,
    expected: Option<StoreVersion>,
    key: String,
    value: String,
}

impl PreferencesEdit {
    /// Write `key` to `value` for `identity`, expecting `expected`.
    pub fn try_new(
        identity: DomainIdentity,
        expected: Option<StoreVersion>,
        key: &str,
        value: &str,
    ) -> Result<Self, ErrorCode> {
        if identity.domain() != LocalStateDomain::Preferences {
            return Err(ErrorCode::Validation {
                domain: identity.domain(),
                reason: ValidationReason::Identity,
            });
        }
        if key.is_empty() {
            return Err(ErrorCode::Validation {
                domain: LocalStateDomain::Preferences,
                reason: ValidationReason::Empty,
            });
        }
        if key.len() > MAX_PREF_KEY_CHARS
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(ErrorCode::Validation {
                domain: LocalStateDomain::Preferences,
                reason: if key.len() > MAX_PREF_KEY_CHARS {
                    ValidationReason::TooLong
                } else {
                    ValidationReason::BadChars
                },
            });
        }
        if value.len() > MAX_PREF_VALUE_BYTES {
            return Err(ErrorCode::Validation {
                domain: LocalStateDomain::Preferences,
                reason: ValidationReason::TooLong,
            });
        }
        Ok(Self {
            identity,
            expected,
            key: key.to_string(),
            value: value.to_string(),
        })
    }

    /// The identity this edit writes.
    pub fn identity(&self) -> &DomainIdentity {
        &self.identity
    }

    /// The version this edit was written against, if any.
    pub fn expected(&self) -> Option<StoreVersion> {
        self.expected
    }

    /// The preference key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The preference value.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Whether this edit may write `current`.
    pub fn applies_to(&self, current: StoreVersion) -> bool {
        self.expected.is_none_or(|next| next == current)
    }
}

/// What a loadout write does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadoutOp {
    /// Insert or replace the row called `loadout.name`.
    Upsert(Loadout),
    /// Remove the row called `name`.
    Remove {
        /// The row to remove.
        name: String,
    },
}

/// One operator loadout write in the `loadouts-references` domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadoutEdit {
    identity: DomainIdentity,
    expected: Option<StoreVersion>,
    op: LoadoutOp,
}

impl LoadoutEdit {
    /// Insert or replace `loadout` for `identity`, expecting `expected`.
    pub fn upsert(
        identity: DomainIdentity,
        expected: Option<StoreVersion>,
        loadout: Loadout,
    ) -> Result<Self, ErrorCode> {
        if identity.domain() != LocalStateDomain::LoadoutsReferences {
            return Err(ErrorCode::Validation {
                domain: identity.domain(),
                reason: ValidationReason::Identity,
            });
        }
        Ok(Self {
            identity,
            expected,
            op: LoadoutOp::Upsert(loadout),
        })
    }

    /// Remove the row called `name` for `identity`, expecting `expected`.
    pub fn remove(
        identity: DomainIdentity,
        expected: Option<StoreVersion>,
        name: &str,
    ) -> Result<Self, ErrorCode> {
        if identity.domain() != LocalStateDomain::LoadoutsReferences {
            return Err(ErrorCode::Validation {
                domain: identity.domain(),
                reason: ValidationReason::Identity,
            });
        }
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(ErrorCode::Validation {
                domain: LocalStateDomain::LoadoutsReferences,
                reason: ValidationReason::Empty,
            });
        }
        if trimmed.len() > MAX_PREF_KEY_CHARS {
            return Err(ErrorCode::Validation {
                domain: LocalStateDomain::LoadoutsReferences,
                reason: ValidationReason::TooLong,
            });
        }
        Ok(Self {
            identity,
            expected,
            op: LoadoutOp::Remove {
                name: trimmed.to_string(),
            },
        })
    }

    /// The identity this edit writes.
    pub fn identity(&self) -> &DomainIdentity {
        &self.identity
    }

    /// The version this edit was written against, if any.
    pub fn expected(&self) -> Option<StoreVersion> {
        self.expected
    }

    /// What the write does.
    pub fn op(&self) -> &LoadoutOp {
        &self.op
    }

    /// Whether this edit may write `current`.
    pub fn applies_to(&self, current: StoreVersion) -> bool {
        self.expected.is_none_or(|next| next == current)
    }
}

/// Observed bank rows to persist in the `bank-hints` domain: `(obj id,
/// count)` sorted by id, unique, every count positive, at most
/// [`MAX_HINT_ROWS`] — the same row rules the hint file checks
/// (`api::bank_memory`), so a persisted hint is always plannable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankHint {
    identity: DomainIdentity,
    expected: Option<StoreVersion>,
    rows: Vec<(i32, i32)>,
    observed_at: DurableTimestamp,
}

impl BankHint {
    /// Persist `rows` observed `observed_at` for `identity`, expecting
    /// `expected`. The rows must already be sorted, unique and positive,
    /// or the hint is refused whole and nothing is staged.
    pub fn try_new(
        identity: DomainIdentity,
        expected: Option<StoreVersion>,
        rows: Vec<(i32, i32)>,
        observed_at: DurableTimestamp,
    ) -> Result<Self, HintRowsError> {
        if rows.len() > MAX_HINT_ROWS {
            return Err(HintRowsError::TooManyRows);
        }
        let mut previous: Option<i32> = None;
        for &(id, count) in &rows {
            if count <= 0 {
                return Err(HintRowsError::NonPositiveCount);
            }
            match previous {
                Some(prev) if id < prev => return Err(HintRowsError::Unsorted),
                Some(prev) if id == prev => return Err(HintRowsError::DuplicateId),
                _ => {}
            }
            previous = Some(id);
        }
        Ok(Self {
            identity,
            expected,
            rows,
            observed_at,
        })
    }

    /// The identity this hint writes.
    pub fn identity(&self) -> &DomainIdentity {
        &self.identity
    }

    /// The version this hint was written against, if any.
    pub fn expected(&self) -> Option<StoreVersion> {
        self.expected
    }

    /// The rows, sorted by id.
    pub fn rows(&self) -> &[(i32, i32)] {
        &self.rows
    }

    /// When the rows were observed, in durable time.
    pub fn observed_at(&self) -> DurableTimestamp {
        self.observed_at
    }

    /// Whether this hint may write `current`.
    pub fn applies_to(&self, current: StoreVersion) -> bool {
        self.expected.is_none_or(|next| next == current)
    }
}

/// Where a [`KnowledgeRecord`] fact came from. Only source-proven account
/// facts persist: observed quest-list and journal evidence, or the
/// operator's own invalidation. Attempted routes are not observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnowledgeProvenance {
    /// Typed observed quest-list output (`slot/progress.rs`).
    QuestList,
    /// An examined quest journal (`quester/progress.rs`).
    Journal,
    /// The operator invalidated the fact.
    Operator,
}

/// How well a [`KnowledgeRecord`] fact is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnowledgeStatus {
    /// Proven by its evidence.
    Known,
    /// Partially evidenced; usable with care, never as new-session action
    /// authority without revalidation.
    Partial,
    /// Unknown or invalidated: kept as a tombstone, never as authority.
    Unknown,
}

/// Longest knowledge key.
pub const MAX_KNOWLEDGE_KEY_CHARS: usize = 128;

/// Largest knowledge payload, in bytes.
pub const MAX_KNOWLEDGE_BYTES: usize = 4096;

/// One source-proven account fact in the `knowledge` domain (§21 §4.4;
/// `comparison.md` U9): a general key with a per-fact payload validated
/// against its schema version before publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeRecord {
    identity: DomainIdentity,
    expected: Option<StoreVersion>,
    key: String,
    provenance: KnowledgeProvenance,
    status: KnowledgeStatus,
    schema_version: u32,
    payload: Vec<u8>,
    observed_at: DurableTimestamp,
}

impl KnowledgeRecord {
    /// Record `key` for `account` under `scope` from `provenance` with
    /// `status`, validated against `schema_version` and observed
    /// `observed_at`. The identity domain is always
    /// [`LocalStateDomain::Knowledge`].
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        account: AccountKey,
        scope: ContentScope,
        expected: Option<StoreVersion>,
        key: &str,
        provenance: KnowledgeProvenance,
        status: KnowledgeStatus,
        schema_version: u32,
        payload: &[u8],
        observed_at: DurableTimestamp,
    ) -> Result<Self, ErrorCode> {
        if key.is_empty() {
            return Err(ErrorCode::Validation {
                domain: LocalStateDomain::Knowledge,
                reason: ValidationReason::Empty,
            });
        }
        if key.len() > MAX_KNOWLEDGE_KEY_CHARS {
            return Err(ErrorCode::Validation {
                domain: LocalStateDomain::Knowledge,
                reason: ValidationReason::TooLong,
            });
        }
        if payload.len() > MAX_KNOWLEDGE_BYTES {
            return Err(ErrorCode::Validation {
                domain: LocalStateDomain::Knowledge,
                reason: ValidationReason::TooLong,
            });
        }
        Ok(Self {
            identity: DomainIdentity::new(LocalStateDomain::Knowledge, account, scope),
            expected,
            key: key.to_string(),
            provenance,
            status,
            schema_version,
            payload: payload.to_vec(),
            observed_at,
        })
    }

    /// The identity this fact writes.
    pub fn identity(&self) -> &DomainIdentity {
        &self.identity
    }

    /// The version this fact was written against, if any.
    pub fn expected(&self) -> Option<StoreVersion> {
        self.expected
    }

    /// The fact key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Where the fact came from.
    pub fn provenance(&self) -> KnowledgeProvenance {
        self.provenance
    }

    /// How well the fact is known.
    pub fn status(&self) -> KnowledgeStatus {
        self.status
    }

    /// The schema the payload was validated against.
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// The validated payload.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// When the fact was observed, in durable time.
    pub fn observed_at(&self) -> DurableTimestamp {
        self.observed_at
    }

    /// Whether this fact may write `current`.
    pub fn applies_to(&self, current: StoreVersion) -> bool {
        self.expected.is_none_or(|next| next == current)
    }

    /// The operator invalidation of this fact: the same key and schema kept
    /// as an [`Unknown`](KnowledgeStatus::Unknown) tombstone with
    /// [`Operator`](KnowledgeProvenance::Operator) provenance, observed
    /// `at`. Stored facts are never action authority for a new session
    /// without revalidation.
    pub fn invalidated(&self, at: DurableTimestamp) -> Self {
        Self {
            identity: self.identity.clone(),
            expected: None,
            key: self.key.clone(),
            provenance: KnowledgeProvenance::Operator,
            status: KnowledgeStatus::Unknown,
            schema_version: self.schema_version,
            payload: Vec::new(),
            observed_at: at,
        }
    }
}

/// The post-commit audit record (D13): one line per durable operator-state
/// write. The composition emits it through `host_log!` once the receipt is
/// durable; its [`Display`](fmt::Display) is that one line, carrying
/// domain, profile, account, version and durable time, and no secret or
/// payload bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditCommit {
    identity: DomainIdentity,
    version: StoreVersion,
    at: DurableTimestamp,
    request: Option<RequestId>,
}

impl AuditCommit {
    /// Audit `version` of `identity` durable `at`, settling `request`.
    /// Construct only once the write it names is durable: the record is
    /// proof, never a pre-commit announcement.
    pub fn after_commit(
        identity: DomainIdentity,
        version: StoreVersion,
        at: DurableTimestamp,
        request: Option<RequestId>,
    ) -> Self {
        Self {
            identity,
            version,
            at,
            request,
        }
    }

    /// The audited identity.
    pub fn identity(&self) -> &DomainIdentity {
        &self.identity
    }

    /// The audited version.
    pub fn version(&self) -> StoreVersion {
        self.version
    }

    /// When the write became durable, in durable time.
    pub fn at(&self) -> DurableTimestamp {
        self.at
    }

    /// The settled request, if any.
    pub fn request(&self) -> Option<RequestId> {
        self.request
    }
}

impl fmt::Display for AuditCommit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "audit domain={} profile={} account={} version={} at={}",
            self.identity.domain().code(),
            self.identity.account().profile(),
            self.identity.account().account(),
            self.version.get(),
            self.at.unix_seconds(),
        )?;
        if let Some(request) = self.request {
            write!(f, " request={}", request.get())?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "local_state_tests.rs"]
mod tests;
