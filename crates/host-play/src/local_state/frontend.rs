//! What a frontend holds of the process local-state service: the
//! operator preferences (`panel-ui.json`, [`Preferences`]) and the operator
//! loadouts its form edits ([`ServiceLoadouts`]). Both are operator-wide
//! stores, one file each for every identity of their domain, so a frontend
//! reads and writes under one fixed operator identity.
//!
//! File reads and writes belong to the service's writer thread; nothing
//! here opens a file. Preference reads use the cached document; cold or
//! refused stores fail immediately, and stale reads queue a refresh.
//! `PreferencePatch` captures changed JSON members and returns a service-owned
//! result without waiting for hydration or I/O; poll `QueuedUpdate` later.
//! Loadout-form calls retain their bounded request behavior.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
#[cfg(any(test, feature = "test-support"))]
use std::time::Instant;

use api::identity::{RequestId, Settlement};
use api::local_state::{
    AccountKey, ContentScope, DomainEdit, DomainIdentity, DomainLease, DurableReceipt, ErrorCode,
    LoadoutEdit, LoadoutOp, LocalStateDomain, LocalStatePort, StoreVersion, SubmitReceipt,
    ValidationReason, MAX_PREF_EDIT_BYTES, MAX_PREF_EDIT_CHANGES, MAX_PREF_KEY_CHARS,
    MAX_PREF_VALUE_BYTES,
};
#[cfg(any(test, feature = "test-support"))]
use api::local_state::{PreferenceChange, PreferencesEdit};
#[cfg(any(test, feature = "test-support"))]
use parking_lot::Mutex;
use script::{LoadoutsBacking, LoadoutsBaseline, LoadoutsSaveError};
use serde_json::{Map, Value};

#[cfg(any(test, feature = "test-support"))]
use super::{files, ReadbackTicket};
use super::{LocalStateService, PreferenceWatchPoll, RequestState};

/// How long loadout-form operations and fixture-only preference helpers wait.
pub const FRONTEND_TIMEOUT: Duration = Duration::from_secs(5);

/// How stale the preferences a frontend reads may be: once a document is
/// held, a read older than this asks the writer thread to read the file
/// again (without waiting), and a later read sees the result.
pub const PREFERENCES_FRESHNESS: Duration = Duration::from_secs(1);

/// How many times an update reads and merges again after another writer
/// changed the file under it.
#[cfg(any(test, feature = "test-support"))]
const UPDATE_ATTEMPTS: usize = 4;

/// The identity a frontend reads and writes the operator stores under. The
/// operator stores ignore account and scope (every identity of their
/// domain is the one file), but a durable identity needs both.
fn operator_identity(domain: LocalStateDomain) -> DomainIdentity {
    DomainIdentity::new(
        domain,
        AccountKey::try_new("operator", "operator").expect("a fixed login identity"),
        ContentScope::try_new(client::io::ClientRevision::R289, "operator")
            .expect("a fixed content identity"),
    )
}
/// A typed preference failure with the affected preference path and cause.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PreferenceError {
    /// The preference service refused or failed an operation with a typed code.
    #[error("preferences at {path}: {cause}")]
    Store {
        /// The preference key or file path associated with this error.
        path: Arc<str>,
        /// The bounded diagnostic cause.
        cause: Arc<str>,
        /// The neutral service code.
        code: ErrorCode,
    },
    /// The preference service cancelled an accepted request.
    #[error("preferences at {path}: {cause}")]
    Cancelled {
        /// The preference key or file path associated with this error.
        path: Arc<str>,
        /// The bounded cancellation diagnostic.
        cause: Arc<str>,
    },
    /// The service no longer retains the request's terminal result.
    #[error("preferences at {path}: {cause}")]
    ResultNotRetained {
        /// The preference key or file path associated with this error.
        path: Arc<str>,
        /// The bounded diagnostic cause.
        cause: Arc<str>,
    },
    /// A preference failure without a service error code.
    #[error("preferences at {path}: {cause}")]
    Other {
        /// The preference key or file path associated with this error.
        path: Arc<str>,
        /// The bounded diagnostic cause.
        cause: Arc<str>,
    },
}

impl PreferenceError {
    fn from_code(path: impl Into<String>, code: ErrorCode) -> Self {
        Self::Store {
            path: Arc::from(path.into()),
            cause: Arc::from(code.to_string()),
            code,
        }
    }

    fn caused(path: impl Into<String>, cause: impl Into<String>) -> Self {
        Self::Other {
            path: Arc::from(path.into()),
            cause: Arc::from(cause.into()),
        }
    }

    fn cancelled(path: impl Into<String>, cause: impl Into<String>) -> Self {
        Self::Cancelled {
            path: Arc::from(path.into()),
            cause: Arc::from(cause.into()),
        }
    }

    fn result_not_retained(path: impl Into<String>) -> Self {
        Self::ResultNotRetained {
            path: Arc::from(path.into()),
            cause: Arc::from("the request's terminal result is no longer retained"),
        }
    }

    pub(super) fn from_cached(path: Arc<str>, cause: Arc<str>, code: Option<ErrorCode>) -> Self {
        match code {
            Some(code) => Self::Store { path, cause, code },
            None => Self::Other { path, cause },
        }
    }

    fn validation(
        path: impl Into<String>,
        reason: ValidationReason,
        cause: impl Into<String>,
    ) -> Self {
        Self::Store {
            path: Arc::from(path.into()),
            cause: Arc::from(cause.into()),
            code: ErrorCode::Validation {
                domain: LocalStateDomain::Preferences,
                reason,
            },
        }
    }

    /// The preference key or file path associated with this error.
    pub fn path(&self) -> &str {
        match self {
            Self::Store { path, .. }
            | Self::Cancelled { path, .. }
            | Self::ResultNotRetained { path, .. }
            | Self::Other { path, .. } => path,
        }
    }

    /// The bounded diagnostic cause.
    pub fn cause(&self) -> &str {
        match self {
            Self::Store { cause, .. }
            | Self::Cancelled { cause, .. }
            | Self::ResultNotRetained { cause, .. }
            | Self::Other { cause, .. } => cause,
        }
    }

    /// The neutral service code, when the failure came from the store.
    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            Self::Store { code, .. } => Some(*code),
            Self::Cancelled { .. } | Self::ResultNotRetained { .. } | Self::Other { .. } => None,
        }
    }

    /// Whether the preference document is still hydrating.
    pub fn is_not_ready(&self) -> bool {
        matches!(
            self,
            Self::Store {
                code: ErrorCode::NotReady { .. },
                ..
            }
        )
    }
}

#[cfg(test)]
mod preference_error_tests {
    use super::PreferenceError;
    use api::local_state::{ErrorCode, LocalStateDomain};
    use std::sync::Arc;

    #[test]
    fn cached_store_error_preserves_shared_diagnostics_and_code() {
        let path: Arc<str> = Arc::from("panel-ui.json");
        let cause: Arc<str> = Arc::from("preference document is not ready yet");
        let error = PreferenceError::from_cached(
            Arc::clone(&path),
            Arc::clone(&cause),
            Some(ErrorCode::NotReady {
                domain: LocalStateDomain::Preferences,
            }),
        );

        assert!(matches!(&error, PreferenceError::Store { .. }));
        assert_eq!(error.path(), "panel-ui.json");
        assert_eq!(error.cause(), "preference document is not ready yet");
        assert_eq!(
            error.code(),
            Some(ErrorCode::NotReady {
                domain: LocalStateDomain::Preferences,
            })
        );
        assert!(error.is_not_ready());
        assert_eq!(
            error.to_string(),
            "preferences at panel-ui.json: preference document is not ready yet"
        );
        let PreferenceError::Store {
            path: stored_path,
            cause: stored_cause,
            ..
        } = error
        else {
            panic!("cached error keeps the shared store diagnostics")
        };
        assert!(Arc::ptr_eq(&path, &stored_path));
        assert!(Arc::ptr_eq(&cause, &stored_cause));
    }

    #[test]
    fn cancellation_and_missing_result_have_distinct_variants() {
        let cancelled = PreferenceError::cancelled("panel-ui.json", "request cancelled (Shutdown)");
        let missing = PreferenceError::result_not_retained("panel-ui.json");

        assert!(matches!(&cancelled, PreferenceError::Cancelled { .. }));
        assert_eq!(cancelled.path(), "panel-ui.json");
        assert_eq!(cancelled.cause(), "request cancelled (Shutdown)");
        assert_eq!(cancelled.code(), None);
        assert!(!cancelled.is_not_ready());
        assert_eq!(
            cancelled.to_string(),
            "preferences at panel-ui.json: request cancelled (Shutdown)"
        );

        assert!(matches!(
            &missing,
            PreferenceError::ResultNotRetained { .. }
        ));
        assert_eq!(missing.path(), "panel-ui.json");
        assert_eq!(
            missing.cause(),
            "the request's terminal result is no longer retained"
        );
        assert_eq!(missing.code(), None);
        assert!(!missing.is_not_ready());
        assert_eq!(
            missing.to_string(),
            "preferences at panel-ui.json: the request's terminal result is no longer retained"
        );
    }
}

#[derive(Debug)]
pub(super) struct PatchMember {
    pub(super) path: Vec<String>,
    pub(super) value: Option<Value>,
}

/// One bounded set of changed JSON-member intents.
#[derive(Debug, Default)]
pub struct PreferencePatch {
    pub(super) members: Vec<PatchMember>,
    pub(super) encoded_bytes: usize,
}

struct PlannedMember<'a> {
    path: Vec<String>,
    value: Option<&'a Value>,
}

#[derive(Default)]
struct PatchPlan<'a> {
    members: Vec<PlannedMember<'a>>,
    encoded_bytes: usize,
}
struct JsonByteCounter(usize);

impl io::Write for JsonByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("serialized preference value is too large"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encoded_json_len(value: &Value) -> Result<usize, serde_json::Error> {
    let mut counter = JsonByteCounter(0);
    serde_json::to_writer(&mut counter, value)?;
    Ok(counter.0)
}

#[cfg(test)]
mod patch_encoding_tests {
    use super::{encoded_json_len, PreferencePatch};
    use serde_json::json;

    #[test]
    fn encoded_json_len_matches_serializer_output() {
        for value in [
            json!({"quoted": "quote\" newline\n tab\t"}),
            json!(["🪨", null, 5]),
            json!({}),
        ] {
            assert_eq!(
                encoded_json_len(&value).unwrap(),
                serde_json::to_vec(&value).unwrap().len()
            );
        }
    }

    #[test]
    fn patch_diff_and_set_accept_dotted_preference_keys() {
        let before = json!({"legacy.key": false});
        let after = json!({"legacy.key": true});
        assert!(PreferencePatch::between(
            before.as_object().expect("an object"),
            before.as_object().expect("an object"),
        )
        .unwrap()
        .is_empty());
        let patch = PreferencePatch::between(
            before.as_object().expect("an object"),
            after.as_object().expect("an object"),
        )
        .unwrap();
        assert!(!patch.is_empty());
        assert!(!PreferencePatch::set("legacy.key", json!(true))
            .unwrap()
            .is_empty());
    }
}

impl PreferencePatch {
    /// Create an empty patch.
    pub fn new() -> Self {
        Self::default()
    }

    /// The recursive member intent that changes `before` into `after`.
    pub fn between(
        before: &Map<String, Value>,
        after: &Map<String, Value>,
    ) -> Result<Self, PreferenceError> {
        let mut plan = PatchPlan::default();
        collect_patch_diff(before, after, &mut Vec::new(), &mut plan)?;
        Ok(Self::from_plan(plan))
    }

    /// Set one top-level member.
    pub fn set(key: &str, value: Value) -> Result<Self, PreferenceError> {
        Self::member(&[key], Some(value))
    }

    /// Create a patch with one member set or removed.
    pub fn member(path: &[&str], value: Option<Value>) -> Result<Self, PreferenceError> {
        let mut patch = Self::new();
        patch.set_member(path, value)?;
        Ok(patch)
    }

    /// Add one set or removal intent at a nested JSON-member path.
    pub fn set_member(
        &mut self,
        path: &[&str],
        value: Option<Value>,
    ) -> Result<(), PreferenceError> {
        let mut addition = Self::new();
        match value {
            Some(value) => plan_owned_value(path, value, &mut addition)?,
            None => add_owned_member(path, None, path_bytes(path)?, &mut addition)?,
        }
        self.append_patch(addition)
    }

    /// Whether this patch has no changed-member intent.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    fn from_plan(plan: PatchPlan<'_>) -> Self {
        let members = plan
            .members
            .into_iter()
            .map(|member| PatchMember {
                path: member.path,
                value: member.value.cloned(),
            })
            .collect();
        Self {
            members,
            encoded_bytes: plan.encoded_bytes,
        }
    }

    fn append_patch(&mut self, mut addition: Self) -> Result<(), PreferenceError> {
        let count = self.members.len().saturating_add(addition.members.len());
        let bytes = self.encoded_bytes.saturating_add(addition.encoded_bytes);
        if count > MAX_PREF_EDIT_CHANGES || bytes > MAX_PREF_EDIT_BYTES {
            return Err(PreferenceError::validation(
                "<document>",
                ValidationReason::TooLong,
                "the preference patch exceeds its change or byte limit",
            ));
        }
        if self.members.is_empty() {
            *self = addition;
        } else {
            self.members.append(&mut addition.members);
            self.encoded_bytes = bytes;
        }
        Ok(())
    }
}

fn plan_owned_value(
    path: &[&str],
    value: Value,
    patch: &mut PreferencePatch,
) -> Result<(), PreferenceError> {
    let key_bytes = path_bytes(path)?;
    let encoded_len = encoded_json_len(&value).map_err(|error| {
        PreferenceError::caused(
            join_path(path),
            format!("cannot encode JSON value: {error}"),
        )
    })?;
    if encoded_len <= MAX_PREF_VALUE_BYTES {
        return add_owned_member(path, Some(value), key_bytes + encoded_len, patch);
    }
    let Value::Object(members) = value else {
        return Err(PreferenceError::validation(
            join_path(path),
            ValidationReason::TooLong,
            "a preference value exceeds the per-member byte limit",
        ));
    };
    add_owned_member(path, None, key_bytes, patch)?;
    for (member, value) in members {
        let mut nested = path.to_vec();
        nested.push(&member);
        plan_owned_value(&nested, value, patch)?;
    }
    Ok(())
}

fn add_owned_member(
    path: &[&str],
    value: Option<Value>,
    encoded_bytes: usize,
    patch: &mut PreferencePatch,
) -> Result<(), PreferenceError> {
    let next_bytes = patch.encoded_bytes.saturating_add(encoded_bytes);
    if patch.members.len() >= MAX_PREF_EDIT_CHANGES || next_bytes > MAX_PREF_EDIT_BYTES {
        return Err(PreferenceError::validation(
            join_path(path),
            ValidationReason::TooLong,
            "the preference patch exceeds its change or byte limit",
        ));
    }
    patch.members.push(PatchMember {
        path: path.iter().map(|member| (*member).to_string()).collect(),
        value,
    });
    patch.encoded_bytes = next_bytes;
    Ok(())
}

fn collect_patch_diff<'a>(
    before: &'a Map<String, Value>,
    after: &'a Map<String, Value>,
    path: &mut Vec<String>,
    plan: &mut PatchPlan<'a>,
) -> Result<(), PreferenceError> {
    for key in before.keys().filter(|key| !after.contains_key(*key)) {
        path.push(key.clone());
        let refs: Vec<&str> = path.iter().map(String::as_str).collect();
        plan_add(&refs, None, path_bytes(&refs)?, plan)?;
        path.pop();
    }
    for (key, new) in after {
        let old = before.get(key);
        if old == Some(new) {
            continue;
        }
        path.push(key.clone());
        let refs: Vec<&str> = path.iter().map(String::as_str).collect();
        match (old.and_then(Value::as_object), new.as_object()) {
            (Some(old), Some(new)) => collect_patch_diff(old, new, path, plan)?,
            _ => plan_value(&refs, new, plan)?,
        }
        path.pop();
    }
    Ok(())
}

fn plan_value<'a>(
    path: &[&str],
    value: &'a Value,
    plan: &mut PatchPlan<'a>,
) -> Result<(), PreferenceError> {
    let key_bytes = path_bytes(path)?;
    let encoded_len = encoded_json_len(value).map_err(|error| {
        PreferenceError::caused(
            join_path(path),
            format!("cannot encode JSON value: {error}"),
        )
    })?;
    if encoded_len <= MAX_PREF_VALUE_BYTES {
        return plan_add(path, Some(value), key_bytes + encoded_len, plan);
    }
    let Some(members) = value.as_object() else {
        return Err(PreferenceError::validation(
            join_path(path),
            ValidationReason::TooLong,
            "a preference value exceeds the per-member byte limit",
        ));
    };
    plan_add(path, None, key_bytes, plan)?;
    for (member, value) in members {
        let mut nested = path.to_vec();
        nested.push(member);
        plan_value(&nested, value, plan)?;
    }
    Ok(())
}

fn plan_add<'a>(
    path: &[&str],
    value: Option<&'a Value>,
    encoded_bytes: usize,
    plan: &mut PatchPlan<'a>,
) -> Result<(), PreferenceError> {
    let key_bytes = path_bytes(path)?;
    let encoded_bytes = encoded_bytes.max(key_bytes);
    let next_bytes = plan.encoded_bytes.saturating_add(encoded_bytes);
    if plan.members.len() >= MAX_PREF_EDIT_CHANGES || next_bytes > MAX_PREF_EDIT_BYTES {
        return Err(PreferenceError::validation(
            join_path(path),
            ValidationReason::TooLong,
            "the preference patch exceeds its change or byte limit",
        ));
    }
    plan.members.push(PlannedMember {
        path: path.iter().map(|member| (*member).to_string()).collect(),
        value,
    });
    plan.encoded_bytes = next_bytes;
    Ok(())
}

fn path_bytes(path: &[&str]) -> Result<usize, PreferenceError> {
    if path.is_empty() {
        return Err(PreferenceError::validation(
            "<document>",
            ValidationReason::Empty,
            "a preference path must contain at least one member",
        ));
    }
    let bytes = path.iter().map(|member| member.len()).sum::<usize>() + path.len() - 1;
    if bytes > MAX_PREF_KEY_CHARS {
        return Err(PreferenceError::validation(
            join_path(path),
            ValidationReason::TooLong,
            "a preference path exceeds the key byte limit",
        ));
    }
    if path.iter().any(|member| {
        member.is_empty()
            || !member
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    }) {
        return Err(PreferenceError::validation(
            join_path(path),
            ValidationReason::BadChars,
            "preference path members must use letters, digits, '.', '_' or '-'",
        ));
    }
    Ok(bytes)
}

fn join_path(path: &[&str]) -> String {
    let mut joined = String::new();
    write_preference_path(path, &mut joined);
    joined
}

pub(super) fn write_preference_path(path: &[&str], output: &mut String) {
    for (index, member) in path.iter().enumerate() {
        if index > 0 {
            output.push('.');
        }
        for character in member.chars() {
            if matches!(character, '.' | '\\' | ';' | ':') {
                output.push('\\');
                output.push(character);
            } else if character.is_control() {
                output.extend(character.escape_default());
            } else if matches!(character, '\u{2028}' | '\u{2029}') {
                output.extend(character.escape_unicode());
            } else {
                output.push(character);
            }
        }
    }
}

/// The preference document and the durable version it was published under.
/// A clone is one reference count: the document is immutable and shared.
#[derive(Debug, Clone)]
pub struct PreferenceSnapshot {
    version: StoreVersion,
    document: Arc<Map<String, Value>>,
}

impl PreferenceSnapshot {
    /// The durable version the document was published under.
    pub fn version(&self) -> StoreVersion {
        self.version
    }

    /// The immutable document.
    pub fn document(&self) -> &Arc<Map<String, Value>> {
        &self.document
    }
}

/// The operator preferences shared by the panel, the TUI and the
/// composition (walk permissions, the quester Path folder, the session log
/// switch, ...), through the process service. One top-level member of the
/// file is one preference row; an object too long for one row is held one
/// row per member. One update is one write: every row it changes is
/// replaced in one file replacement, or none is. A clone is one reference
/// count: clones share the service handle, the identity and what was last
/// read.
#[derive(Clone)]
pub struct Preferences {
    shared: Arc<Shared>,
}

/// What every clone of one [`Preferences`] holds.
struct Shared {
    service: Arc<LocalStateService>,
    identity: DomainIdentity,
    #[cfg(any(test, feature = "test-support"))]
    held: Mutex<Held>,
}

/// A fixture-only cache for synchronous preference helpers.
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct Held {
    version: Option<StoreVersion>,
    document: Arc<Map<String, Value>>,
    read_at: Option<Instant>,
}

impl Preferences {
    pub(super) fn new(service: Arc<LocalStateService>) -> Self {
        Self {
            shared: Arc::new(Shared {
                service,
                identity: operator_identity(LocalStateDomain::Preferences),
                #[cfg(any(test, feature = "test-support"))]
                held: Mutex::default(),
            }),
        }
    }

    /// The service behind these preferences.
    pub(super) fn service(&self) -> &Arc<LocalStateService> {
        &self.shared.service
    }

    /// The operator identity these preferences read and write under.
    #[cfg(test)]
    pub(super) fn identity(&self) -> &DomainIdentity {
        &self.shared.identity
    }

    /// The file behind the store, for messages.
    pub fn path(&self) -> PathBuf {
        self.service().root().preferences_file()
    }

    /// The cached immutable document. Cold and refused stores fail immediately;
    /// a stale snapshot only queues a refresh on the writer thread.
    pub fn document(&self) -> Result<Arc<Map<String, Value>>, PreferenceError> {
        self.shared
            .service
            .preferences_document(&self.shared.identity, PREFERENCES_FRESHNESS)
    }

    /// The cached immutable document and the durable version it was
    /// published under, in one read. The version moves only when the
    /// document's bytes changed (this service's write or another writer's),
    /// so a reader applies a snapshot once per version. Same cold, refused
    /// and stale behaviour as [`Self::document`].
    pub fn snapshot(&self) -> Result<PreferenceSnapshot, PreferenceError> {
        self.shared
            .service
            .preferences_snapshot(&self.shared.identity, PREFERENCES_FRESHNESS)
            .map(|(version, document)| PreferenceSnapshot { version, document })
    }

    /// The worker-projected, cached walk permission view.
    pub fn walk_globals(&self) -> Result<crate::walk_permissions::WalkGlobals, PreferenceError> {
        self.shared
            .service
            .preferences_walk_globals(&self.shared.identity, PREFERENCES_FRESHNESS)
    }

    /// One top-level preference; `None` when it is absent or the store
    /// cannot be read.
    pub fn value(&self, key: &str) -> Option<Value> {
        self.document().ok()?.get(key).cloned()
    }

    /// Accept one changed-member intent without waiting for hydration or I/O.
    pub fn begin_patch(&self, patch: PreferencePatch) -> Result<Update, PreferenceError> {
        if patch.is_empty() {
            return Ok(Update::Durable);
        }
        let request = self
            .shared
            .service
            .enqueue_preference_patch(&self.shared.identity, patch)
            .map_err(|code| PreferenceError::from_code(self.path().display().to_string(), code))?;
        Ok(Update::Queued(QueuedUpdate::service(self.clone(), request)))
    }

    /// Accept a single-member update without waiting for hydration or I/O.
    pub fn begin_set(&self, key: &str, value: Value) -> Result<Update, PreferenceError> {
        self.begin_patch(PreferencePatch::set(key, value)?)
    }

    /// Reattach to a service-owned result that has not been projected yet.
    pub fn watch(&self, request: RequestId) -> Option<QueuedUpdate> {
        self.shared
            .service
            .has_preference_watch(request)
            .then(|| QueuedUpdate::service(self.clone(), request))
    }

    fn poll_preference(&self, request: RequestId) -> Option<Result<(), PreferenceError>> {
        self.project_watch(self.shared.service.poll_preference_watch(request))
    }

    #[cfg(any(test, feature = "test-support"))]
    fn wait_preference(
        &self,
        request: RequestId,
        wait: Duration,
    ) -> Option<Result<(), PreferenceError>> {
        self.project_watch(self.shared.service.wait_preference_watch(request, wait))
    }

    fn project_watch(&self, poll: PreferenceWatchPoll) -> Option<Result<(), PreferenceError>> {
        let path = || self.path().display().to_string();
        match poll {
            PreferenceWatchPoll::Pending => None,
            PreferenceWatchPoll::Missing => Some(Err(PreferenceError::result_not_retained(path()))),
            PreferenceWatchPoll::Settled(Settlement::Completed(_)) => Some(Ok(())),
            PreferenceWatchPoll::Settled(Settlement::Failed(code)) => {
                Some(Err(PreferenceError::from_code(path(), code)))
            }
            PreferenceWatchPoll::Settled(Settlement::Cancelled(cancellation)) => Some(Err(
                PreferenceError::cancelled(path(), format!("request cancelled ({cancellation:?})")),
            )),
        }
    }

    /// The retained request IDs whose terminal results have not been projected.
    pub fn unprojected_requests(&self) -> Vec<RequestId> {
        self.shared.service.unprojected_preference_requests()
    }

    /// Read the file again and wait. Available to test fixtures and
    /// test-support callers only.
    #[cfg(any(test, feature = "test-support"))]
    pub fn reload(&self) -> io::Result<Arc<Map<String, Value>>> {
        let shared = &*self.shared;
        let read = shared.service.readback(&shared.identity, FRONTEND_TIMEOUT);
        shared.held.lock().read_at = Some(Instant::now());
        let (lease, _) = read.map_err(|code| self.error(code))?;
        self.adopt(&lease)
    }

    /// Read the current document from the file, let `merge` change it, and
    /// write every changed top-level preference (an object too long for
    /// one row, each changed or removed member) in one edit: one durable
    /// file replacement that applies all of them or none. When another
    /// writer changed the file first, read and merge again (a few times at
    /// most). The read is a save's
    /// ([`LocalStateService::start_readback`]): a file whose content is
    /// malformed is moved aside and the update starts from fresh
    /// preferences, as the old shared writer did.
    #[cfg(any(test, feature = "test-support"))]
    pub fn update(
        &self,
        merge: impl FnMut(&mut Map<String, Value>) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut run = self.begin_run(merge)?;
        match self.drive(&mut run, Some(FRONTEND_TIMEOUT)) {
            Some(result) => result,
            None => Err(self.waited_out(&run)),
        }
    }

    /// Set one top-level preference.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set(&self, key: &str, value: Value) -> io::Result<()> {
        self.update(|document| {
            document.insert(key.to_string(), value.clone());
            Ok(())
        })
    }

    /// Queue the update's first read (a save's read: see
    /// [`LocalStateService::start_readback`]).
    #[cfg(any(test, feature = "test-support"))]
    fn begin_run<F>(&self, merge: F) -> io::Result<UpdateRun<F>> {
        let ticket = self
            .shared
            .service
            .start_readback(&self.shared.identity, true)
            .map_err(|code| self.error(code))?;
        Ok(UpdateRun {
            merge,
            stage: Stage::Reading(ticket),
            reads: 1,
        })
    }

    /// Advance `run` until it ends, each wait for the writer thread
    /// bounded by `wait` (without one, nothing waits: a frame's check).
    /// `None` while its read or its write is still queued.
    #[cfg(any(test, feature = "test-support"))]
    fn drive<F: FnMut(&mut Map<String, Value>) -> io::Result<()>>(
        &self,
        run: &mut UpdateRun<F>,
        wait: Option<Duration>,
    ) -> Option<io::Result<()>> {
        let shared = &*self.shared;
        loop {
            match &mut run.stage {
                Stage::Reading(ticket) => {
                    let read = shared.service.finish_readback(
                        &shared.identity,
                        *ticket,
                        wait.map(|wait| Instant::now() + wait),
                    )?;
                    let started = read
                        .map_err(|code| self.error(code))
                        .and_then(|(lease, _)| self.start_edits(&mut run.merge, &lease));
                    match started {
                        Err(error) => return Some(Err(error)),
                        Ok(None) => return Some(self.finished()),
                        Ok(Some(stage)) => run.stage = stage,
                    }
                }
                Stage::Writing(request) => {
                    let state = match wait {
                        Some(wait) => shared.service.wait_request(*request, wait),
                        None => shared.service.request_state(*request),
                    };
                    match state {
                        RequestState::Pending => return None,
                        RequestState::Settled(Settlement::Completed(_)) => {
                            return Some(self.finished())
                        }
                        RequestState::Settled(Settlement::Failed(
                            code @ ErrorCode::Conflict { .. },
                        )) => {
                            // Another writer changed the file first: read
                            // and merge again (a few times at most).
                            if run.reads == UPDATE_ATTEMPTS {
                                return Some(Err(self.error(code)));
                            }
                            match shared.service.start_readback(&shared.identity, true) {
                                Ok(ticket) => {
                                    run.reads += 1;
                                    run.stage = Stage::Reading(ticket);
                                }
                                Err(code) => return Some(Err(self.error(code))),
                            }
                        }
                        RequestState::Settled(Settlement::Failed(code)) => {
                            return Some(Err(self.error(code)))
                        }
                        RequestState::Settled(Settlement::Cancelled(why)) => {
                            return Some(Err(self.wrap(io::Error::new(
                                io::ErrorKind::Interrupted,
                                format!("the write was cancelled ({why:?})"),
                            ))))
                        }
                        RequestState::Forgotten => {
                            return Some(Err(self.wrap(io::Error::other(
                                "the service no longer holds the write's settlement",
                            ))))
                        }
                    }
                }
            }
        }
    }

    /// Adopt what the read found, merge, and submit the changed
    /// preferences as one edit; `None` when the merge changed nothing.
    #[cfg(any(test, feature = "test-support"))]
    fn start_edits<F: FnMut(&mut Map<String, Value>) -> io::Result<()>>(
        &self,
        merge: &mut F,
        lease: &DomainLease,
    ) -> io::Result<Option<Stage>> {
        let before = self.adopt(lease)?;
        let mut after = (*before).clone();
        merge(&mut after)?;
        let changes = preference_changes(&before, &after).map_err(|code| self.error(code))?;
        if changes.is_empty() {
            return Ok(None);
        }
        let shared = &*self.shared;
        let edit = PreferencesEdit::batch(shared.identity.clone(), Some(lease.version()), changes)
            .map_err(|code| self.error(code))?;
        match shared.service.submit(DomainEdit::Preferences(edit)) {
            SubmitReceipt::Accepted { request } => Ok(Some(Stage::Writing(request))),
            SubmitReceipt::Refused(code) => Err(self.error(code)),
        }
    }

    /// An update ended durable: what it read is fresh.
    #[cfg(any(test, feature = "test-support"))]
    fn finished(&self) -> io::Result<()> {
        self.shared.held.lock().read_at = Some(Instant::now());
        Ok(())
    }

    /// Why a bounded [`Self::update`] gave up on `run`.
    #[cfg(any(test, feature = "test-support"))]
    fn waited_out<F>(&self, run: &UpdateRun<F>) -> io::Error {
        match run.stage {
            Stage::Reading(_) => self.error(ErrorCode::NotReady {
                domain: self.shared.identity.domain(),
            }),
            Stage::Writing { .. } => self.wrap(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "the write was not durable within {FRONTEND_TIMEOUT:?}; it stays queued and \
                     settles later"
                ),
            )),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn adopt(&self, lease: &DomainLease) -> io::Result<Arc<Map<String, Value>>> {
        let rows = lease.snapshot().as_preferences().unwrap_or(&[]);
        let document = Arc::new(files::preference_document(rows).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("preferences: unreadable rows ({})", self.path().display()),
            )
        })?);
        let mut held = self.shared.held.lock();
        held.version = Some(lease.version());
        held.document = Arc::clone(&document);
        Ok(document)
    }

    #[cfg(any(test, feature = "test-support"))]
    fn error(&self, code: ErrorCode) -> io::Error {
        io::Error::new(
            error_kind(code),
            format!("preferences: {code} ({})", self.path().display()),
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    fn wrap(&self, error: io::Error) -> io::Error {
        io::Error::new(
            error.kind(),
            format!("preferences: {error} ({})", self.path().display()),
        )
    }
}

/// The changes that turn `before` into `after`: one per changed or removed
/// top-level member, or, for an object too long for one row, one per
/// changed or removed member of it (`<member>.<nested>`). A value too long
/// for one row that is not an object cannot be written.
#[cfg(any(test, feature = "test-support"))]
fn preference_changes(
    before: &Map<String, Value>,
    after: &Map<String, Value>,
) -> Result<Vec<PreferenceChange>, ErrorCode> {
    let mut changes = Vec::new();
    for key in before.keys().filter(|key| !after.contains_key(*key)) {
        changes.push(PreferenceChange::remove(key)?);
    }
    for (key, value) in after {
        let old = before.get(key);
        if old == Some(value) {
            continue;
        }
        let text = value.to_string();
        if text.len() <= MAX_PREF_VALUE_BYTES {
            changes.push(PreferenceChange::set(key, &text)?);
            continue;
        }
        let Value::Object(members) = value else {
            return Err(ErrorCode::Validation {
                domain: LocalStateDomain::Preferences,
                reason: ValidationReason::TooLong,
            });
        };
        let old = match old {
            Some(Value::Object(old)) => Some(old),
            Some(_) => {
                // Not an object before: replace it with the members.
                changes.push(PreferenceChange::remove(key)?);
                None
            }
            None => None,
        };
        if let Some(old) = old {
            for gone in old.keys().filter(|member| !members.contains_key(*member)) {
                changes.push(PreferenceChange::remove(&format!("{key}.{gone}"))?);
            }
        }
        for (member, value) in members {
            if old.and_then(|old| old.get(member)) != Some(value) {
                changes.push(PreferenceChange::set(
                    &format!("{key}.{member}"),
                    &value.to_string(),
                )?);
            }
        }
    }
    Ok(changes)
}

/// One fixture-only preferences update in progress: its read, then write.
#[cfg(any(test, feature = "test-support"))]
struct UpdateRun<F> {
    merge: F,
    stage: Stage,
    reads: usize,
}

#[cfg(any(test, feature = "test-support"))]
enum Stage {
    Reading(ReadbackTicket),
    Writing(RequestId),
}

/// Whether the request was already durable or remains service-owned.
pub enum Update {
    /// The patch was empty and required no write.
    Durable,
    /// The accepted request is owned by the process service.
    Queued(QueuedUpdate),
}

impl std::fmt::Debug for Update {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Durable => f.write_str("Durable"),
            Self::Queued(_) => f.write_str("Queued"),
        }
    }
}

/// A watch on one process-service-owned preference request. Dropping the
/// watch only unsubscribes; it does not cancel the accepted intent.
pub struct QueuedUpdate {
    preferences: Preferences,
    request: RequestId,
}

impl QueuedUpdate {
    fn service(preferences: Preferences, request: RequestId) -> Self {
        Self {
            preferences,
            request,
        }
    }

    /// The identity of this accepted service request.
    pub fn request_id(&self) -> RequestId {
        self.request
    }

    /// Check without waiting for the request's retained terminal result.
    pub fn poll(&mut self) -> Option<Result<(), PreferenceError>> {
        self.preferences.poll_preference(self.request)
    }

    /// Test-only bounded wait for a request terminal.
    #[cfg(any(test, feature = "test-support"))]
    pub fn wait(&mut self, wait: Duration) -> Option<Result<(), PreferenceError>> {
        self.preferences.wait_preference(self.request, wait)
    }
}

/// The operator loadouts a frontend's form edits ([`script::LoadoutsStore`]
/// over this backing): loads read the file again on the writer thread, and
/// saves are typed edits against the version the form loaded, each
/// durable before the next. A rename is one edit, so the row is replaced
/// in place in one write and never lost between two.
pub struct ServiceLoadouts {
    service: Arc<LocalStateService>,
    identity: DomainIdentity,
}

impl ServiceLoadouts {
    pub(super) fn new(service: Arc<LocalStateService>) -> Self {
        Self {
            service,
            identity: operator_identity(LocalStateDomain::LoadoutsReferences),
        }
    }

    fn describe(&self, what: impl std::fmt::Display) -> String {
        format!(
            "loadouts: {what} ({})",
            self.service.root().loadouts_file().display()
        )
    }
}

impl LoadoutsBacking for ServiceLoadouts {
    fn load(&self) -> Result<LoadoutsBaseline, String> {
        let (lease, _) = self
            .service
            .readback(&self.identity, FRONTEND_TIMEOUT)
            .map_err(|code| self.describe(code))?;
        Ok(baseline_of(&lease))
    }

    fn commit(
        &self,
        ops: Vec<LoadoutOp>,
        expected: StoreVersion,
    ) -> Result<LoadoutsBaseline, LoadoutsSaveError> {
        let mut expected = expected;
        for op in ops {
            let renamed_to = match &op {
                LoadoutOp::Rename { loadout, .. } => Some(loadout.name.clone()),
                LoadoutOp::Upsert(_) | LoadoutOp::Remove { .. } => None,
            };
            let edit = match op {
                LoadoutOp::Upsert(loadout) => {
                    LoadoutEdit::upsert(self.identity.clone(), Some(expected), loadout)
                }
                LoadoutOp::Remove { name } => {
                    LoadoutEdit::remove(self.identity.clone(), Some(expected), &name)
                }
                LoadoutOp::Rename { from, loadout } => {
                    LoadoutEdit::rename(self.identity.clone(), Some(expected), &from, loadout)
                }
            }
            .map_err(|code| LoadoutsSaveError::Failed(self.describe(code)))?;
            match settle(&self.service, DomainEdit::Loadout(edit))
                .map_err(|error| LoadoutsSaveError::Failed(self.describe(error)))?
            {
                Settled::Durable(receipt) => expected = receipt.version(),
                Settled::Refused(code) => {
                    // A rename onto a name another row has: the editors
                    // refuse that first, so that row is another writer's.
                    return Err(LoadoutsSaveError::Failed(match (code, renamed_to) {
                        (
                            ErrorCode::Validation {
                                reason: ValidationReason::DuplicateId,
                                ..
                            },
                            Some(name),
                        ) => format!("A loadout named \"{name}\" already exists."),
                        (code, _) => self.describe(code),
                    }));
                }
                Settled::Conflict(code) => {
                    let latest = self.load().map_err(LoadoutsSaveError::Failed)?;
                    return Err(LoadoutsSaveError::Conflict {
                        latest,
                        reason: self.describe(code),
                    });
                }
            }
        }
        let lease = self
            .service
            .lease(&self.identity)
            .map_err(|code| LoadoutsSaveError::Failed(self.describe(code)))?;
        Ok(baseline_of(&lease))
    }
}

fn baseline_of(lease: &DomainLease) -> LoadoutsBaseline {
    LoadoutsBaseline {
        rows: lease.snapshot().as_loadouts().unwrap_or(&[]).to_vec(),
        version: lease.version(),
    }
}

/// How a frontend's write ended, short of a failure to settle it.
enum Settled {
    Durable(DurableReceipt),
    /// Another writer changed the store first: retry against what it holds.
    Conflict(ErrorCode),
    /// The service refused the edit, at submission or when it decided it.
    Refused(ErrorCode),
}

/// Submit `edit` and wait, bounded, for its terminal.
fn settle(service: &LocalStateService, edit: DomainEdit) -> io::Result<Settled> {
    let request = match service.submit(edit) {
        SubmitReceipt::Accepted { request } => request,
        SubmitReceipt::Refused(code) => return Ok(Settled::Refused(code)),
    };
    match service.wait_request(request, FRONTEND_TIMEOUT) {
        RequestState::Settled(Settlement::Completed(receipt)) => Ok(Settled::Durable(receipt)),
        RequestState::Settled(Settlement::Failed(code @ ErrorCode::Conflict { .. })) => {
            Ok(Settled::Conflict(code))
        }
        RequestState::Settled(Settlement::Failed(code)) => Ok(Settled::Refused(code)),
        RequestState::Settled(Settlement::Cancelled(why)) => Err(io::Error::new(
            io::ErrorKind::Interrupted,
            format!("the write was cancelled ({why:?})"),
        )),
        RequestState::Pending => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "the write was not durable within {FRONTEND_TIMEOUT:?}; it stays queued and \
                 settles later"
            ),
        )),
        RequestState::Forgotten => Err(io::Error::other(
            "the service no longer holds the write's settlement",
        )),
    }
}

#[cfg(any(test, feature = "test-support"))]
fn error_kind(code: ErrorCode) -> io::ErrorKind {
    match code {
        ErrorCode::Validation { .. } => io::ErrorKind::InvalidInput,
        ErrorCode::Conflict { .. } => io::ErrorKind::AlreadyExists,
        ErrorCode::NotReady { .. } => io::ErrorKind::TimedOut,
        ErrorCode::Busy { .. } => io::ErrorKind::WouldBlock,
        ErrorCode::Failed { .. } => io::ErrorKind::InvalidData,
        ErrorCode::ProcessService(_) => io::ErrorKind::Other,
    }
}
