//! Bounded, plain-value protocol between one surface run and its host.
//!
//! The dock carries no gameplay API. The slot owns its lifecycle, and the host
//! consumes only calls that an accepted Tick dispatch committed.

use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

/// Maximum calls retained in either dispatch staging or the host queue.
pub const EXTENSION_QUEUE_CAPACITY: usize = 64;
/// Maximum nesting depth in one extension argument.
pub const EXTENSION_MAX_DEPTH: usize = 8;
/// Maximum scalar leaves in one extension argument.
pub const EXTENSION_MAX_LEAVES: usize = 256;
/// Maximum encoded size of one extension argument.
pub const EXTENSION_MAX_BYTES: usize = 64 * 1024;
/// Maximum bytes in a fault detail or namespace label.
pub const EXTENSION_MAX_TEXT_BYTES: usize = 160;

/// Maximum live request handles retained by one run.
pub const EXTENSION_MAX_HANDLES: usize = 256;
/// The identity of one admitted VM run and its private dock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RunBinding {
    /// The lifetime identity of the slot.
    pub slot_incarnation: u64,
    /// The runtime generation admitted for this Start.
    pub run_key: u64,
    /// The host-issued identity of this dock.
    pub dock_id: u64,
}

impl RunBinding {
    /// Make an immutable identity for one admitted Start.
    pub const fn new(slot_incarnation: u64, run_key: u64, dock_id: u64) -> Self {
        Self {
            slot_incarnation,
            run_key,
            dock_id,
        }
    }
}

/// A bounded, VM-free value carried by an extension call or header.
#[derive(Debug, Clone, PartialEq)]
pub enum ExtensionValue {
    /// Named fields in source order.
    Named(Vec<(String, Self)>),
    /// Positional values in source order.
    List(Vec<Self>),
    /// UTF-8 text.
    String(String),
    /// Boolean value.
    Boolean(bool),
    /// Signed integer value.
    Integer(i64),
    /// Finite floating-point value.
    Number(f64),
}
/// A host-declared extension function and its fixed call mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtensionFunction {
    /// Canonical dotted name.
    pub name: &'static str,
    /// Request, await, or terminal finish behavior.
    pub mode: ExtensionMode,
}

impl ExtensionFunction {
    /// Declare one function without a script-provided closure.
    pub const fn new(name: &'static str, mode: ExtensionMode) -> Self {
        Self { name, mode }
    }
}

/// One host-declared constant in a named extension namespace.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionConstant {
    /// Constant name within its namespace.
    pub name: &'static str,
    /// Plain, bounded constant data.
    pub value: ExtensionValue,
}

impl ExtensionConstant {
    /// Declare one immutable plain constant.
    pub fn new(name: &'static str, value: ExtensionValue) -> Self {
        Self { name, value }
    }
}

/// A host-declared namespace with plain immutable constants.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionNamespace {
    /// Canonical dotted path below `scenario`.
    pub name: &'static str,
    /// Constants exported by this namespace.
    pub constants: Vec<ExtensionConstant>,
}

impl ExtensionNamespace {
    /// Declare one namespace and its immutable constants.
    pub fn new(name: &'static str, constants: Vec<ExtensionConstant>) -> Self {
        Self { name, constants }
    }
}

/// A failed conversion from a plain extension value to JSON or bounded data.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExtensionValueError {
    /// A floating-point value was not finite.
    #[error("number must be finite")]
    NonFiniteNumber,
    /// A named value repeated one key.
    #[error("named value contains duplicate key `{0}`")]
    DuplicateKey(String),
    /// A value exceeded the safe conversion depth.
    #[error("value exceeds the conversion depth limit")]
    TooDeep,
    /// A value exceeded the scalar count.
    #[error("value exceeds the scalar-leaf limit")]
    TooManyLeaves,
    /// A value exceeded the encoded-byte bound.
    #[error("value exceeds the encoded-byte limit")]
    TooLarge,
    /// A name or label exceeded its UTF-8 byte bound.
    #[error("text exceeds the UTF-8 byte limit")]
    TextTooLong,
}

impl ExtensionValue {
    /// Convert a plain value into JSON without executing or interpreting it.
    pub fn into_json(self) -> Result<serde_json::Value, ExtensionValueError> {
        self.into_json_at(0)
    }

    /// Validate the extension argument bounds without copying or allocating.
    pub fn validate_bounded(&self) -> Result<(), ExtensionValueError> {
        ValueBudget::default().visit(self, 0)
    }

    fn into_json_at(self, depth: usize) -> Result<serde_json::Value, ExtensionValueError> {
        if depth > 16 {
            return Err(ExtensionValueError::TooDeep);
        }
        match self {
            Self::Named(fields) => {
                let mut object = serde_json::Map::new();
                for (key, value) in fields {
                    if object.contains_key(&key) {
                        return Err(ExtensionValueError::DuplicateKey(key));
                    }
                    object.insert(key, value.into_json_at(depth + 1)?);
                }
                Ok(serde_json::Value::Object(object))
            }
            Self::List(values) => values
                .into_iter()
                .map(|value| value.into_json_at(depth + 1))
                .collect::<Result<Vec<_>, _>>()
                .map(serde_json::Value::Array),
            Self::String(value) => Ok(serde_json::Value::String(value)),
            Self::Boolean(value) => Ok(serde_json::Value::Bool(value)),
            Self::Integer(value) => Ok(serde_json::Value::Number(value.into())),
            Self::Number(value) => serde_json::Number::from_f64(value)
                .map(serde_json::Value::Number)
                .ok_or(ExtensionValueError::NonFiniteNumber),
        }
    }
}

#[derive(Default)]
struct ValueBudget {
    leaves: usize,
    bytes: usize,
}

impl ValueBudget {
    fn visit(&mut self, value: &ExtensionValue, depth: usize) -> Result<(), ExtensionValueError> {
        if depth > EXTENSION_MAX_DEPTH {
            return Err(ExtensionValueError::TooDeep);
        }
        match value {
            ExtensionValue::Named(fields) => {
                self.add_bytes(2)?;
                for (index, (key, value)) in fields.iter().enumerate() {
                    if key.len() > EXTENSION_MAX_TEXT_BYTES {
                        return Err(ExtensionValueError::TextTooLong);
                    }
                    if index > 0 {
                        self.add_bytes(1)?;
                    }
                    self.add_bytes(json_string_len(key).saturating_add(1))?;
                    self.visit(value, depth + 1)?;
                }
            }
            ExtensionValue::List(values) => {
                self.add_bytes(2)?;
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        self.add_bytes(1)?;
                    }
                    self.visit(value, depth + 1)?;
                }
            }
            ExtensionValue::String(value) => self.scalar(json_string_len(value))?,
            ExtensionValue::Boolean(value) => self.scalar(if *value { 4 } else { 5 })?,
            ExtensionValue::Integer(value) => self.scalar(integer_encoded_len(*value))?,
            ExtensionValue::Number(value) => {
                if !value.is_finite() {
                    return Err(ExtensionValueError::NonFiniteNumber);
                }
                self.scalar(24)?;
            }
        }
        Ok(())
    }

    fn scalar(&mut self, bytes: usize) -> Result<(), ExtensionValueError> {
        self.leaves = self.leaves.saturating_add(1);
        if self.leaves > EXTENSION_MAX_LEAVES {
            return Err(ExtensionValueError::TooManyLeaves);
        }
        self.add_bytes(bytes)
    }

    fn add_bytes(&mut self, bytes: usize) -> Result<(), ExtensionValueError> {
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > EXTENSION_MAX_BYTES {
            return Err(ExtensionValueError::TooLarge);
        }
        Ok(())
    }
}

fn integer_encoded_len(value: i64) -> usize {
    let mut magnitude = i128::from(value).abs();
    let mut digits = 1;
    while magnitude >= 10 {
        magnitude /= 10;
        digits += 1;
    }
    digits + usize::from(value < 0)
}

fn json_string_len(value: &str) -> usize {
    let mut bytes = 2usize;
    for character in value.chars() {
        bytes = bytes.saturating_add(match character {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{08}' | '\u{0c}' => 2,
            character if character <= '\u{1f}' => 6,
            character => character.len_utf8(),
        });
    }
    bytes
}

/// The lifecycle entry that attempted an extension operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionEntry {
    /// VM setup before source loading.
    Setup,
    /// Top-level source execution.
    Load,
    /// The ordinary script Tick.
    Tick,
    /// The paint callback.
    Paint,
    /// The Stop hook.
    Hook,
}

/// How the VM and host exchange one call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionMode {
    /// Queue one request and return its sequence as a host handle.
    Request,
    /// Queue one request and suspend until its matching payload arrives.
    Await,
    /// Queue the terminal request, seal admissions, and suspend forever.
    Finish,
}

/// One committed plain-value call from a surface run.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionCall {
    /// The run that owns this request.
    pub binding: RunBinding,
    /// Monotonic request identity within the dock.
    pub sequence: u64,
    /// The VM entry that staged the request.
    pub entry: ExtensionEntry,
    /// Canonical dotted function name supplied by the host registry.
    pub name: &'static str,
    /// The declared call mode.
    pub mode: ExtensionMode,
    /// The converted arguments, with no VM handles.
    pub arguments: ExtensionValue,
}

/// The only values that can resume an awaited extension call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionPayload {
    /// The host completed the operation without a value.
    Unit,
    /// A probe returned one Boolean value.
    Boolean(bool),
}

/// The durable first-fault classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionFaultCode {
    /// A retained Start tried to claim an already-claimed dock.
    AlreadyClaimed,
    /// A caller used a different run identity.
    WrongBinding,
    /// An operation ran outside Tick.
    WrongEntry,
    /// A function name is not present in the host registry.
    UnknownFunction,
    /// An argument could not be converted to bounded plain data.
    Conversion,
    /// A bounded staging or host queue is full.
    Capacity,
    /// A call followed Finish or host sealing.
    PostSeal,
    /// A call followed revocation or close.
    Revoked,
    /// The extension protocol received an inconsistent transition.
    Protocol,
    /// Gameplay work was attempted after the host fence.
    PostFreezeGameplay,
    /// The host supplied stale or duplicate completion ownership.
    HostProtocol,
}

/// UTF-8 diagnostic stored inline at the protocol's fixed text bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundedText {
    bytes: [u8; EXTENSION_MAX_TEXT_BYTES],
    len: u8,
}

impl BoundedText {
    /// Copy the longest complete UTF-8 prefix that fits the fixed bound.
    pub fn new(value: &str) -> Self {
        let mut len = value.len().min(EXTENSION_MAX_TEXT_BYTES);
        while !value.is_char_boundary(len) {
            len -= 1;
        }
        let mut bytes = [0; EXTENSION_MAX_TEXT_BYTES];
        bytes[..len].copy_from_slice(&value.as_bytes()[..len]);
        Self {
            bytes,
            len: len as u8,
        }
    }

    /// Read the retained text without allocation.
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)])
            .expect("BoundedText only stores a UTF-8 prefix")
    }
}

impl From<&str> for BoundedText {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for BoundedText {
    fn from(value: String) -> Self {
        Self::new(&value)
    }
}

impl std::fmt::Display for BoundedText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}
/// One immutable, binding-first failure record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionFault {
    /// The dock's owning execution.
    pub binding: RunBinding,
    /// The call sequence, when a call already had one.
    pub sequence: Option<u64>,
    /// The lifecycle entry that observed the failure.
    pub entry: ExtensionEntry,
    /// The stable failure category.
    pub code: ExtensionFaultCode,
    /// Bounded diagnostic text.
    pub detail: BoundedText,
}

/// Why the slot or VM closed this dock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionEndCause {
    /// The slot completed an ordinary Stop.
    Stopped,
    /// The run completed its own terminal operation.
    Completed,
    /// The runtime or extension failed.
    Failed,
    /// The owner revoked the run.
    Cancelled,
    /// The admitted Load did not produce a run.
    LoadFailed,
}

/// A bounded failure from the ordinary Stop hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionHookFailure {
    /// Stable Luau failure code.
    pub code: BoundedText,
    /// Bounded error detail.
    pub detail: BoundedText,
}

impl ExtensionHookFailure {
    /// Store a bounded ordinary Stop-hook failure.
    pub fn new(code: &str, detail: &str) -> Self {
        Self {
            code: BoundedText::new(code),
            detail: BoundedText::new(detail),
        }
    }
}

/// Counts bounded records discarded when the VM closed its dock.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ExtensionDisposalCounts {
    /// Calls discarded from dispatch staging.
    pub staged: usize,
    /// Calls discarded from the host queue.
    pub queued: usize,
    /// Whether one completion payload was discarded.
    pub completion: bool,
    /// Request handles discarded before the script consumed or finished them.
    pub handles: usize,
}

/// Typed terminal record retained after the dock closes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionEnd {
    /// The execution whose VM ended.
    pub binding: RunBinding,
    /// The terminal cause.
    pub cause: ExtensionEndCause,
    /// Whether the VM was dropped before this record was published.
    pub vm_dropped: bool,
    /// Failure from the ordinary onStop hook, if one occurred.
    pub hook_failure: Option<ExtensionHookFailure>,
    /// The durable first extension fault, if one occurred.
    pub first_fault: Option<ExtensionFault>,
    /// A bounded setup/Load diagnostic, if setup failed.
    pub load_error: Option<BoundedText>,
    /// Queue and completion disposal counts.
    pub disposal_counts: ExtensionDisposalCounts,
}

impl ExtensionEnd {
    /// Make a terminal record for one run.
    pub fn new(binding: RunBinding, cause: ExtensionEndCause) -> Self {
        Self {
            binding,
            cause,
            vm_dropped: false,
            hook_failure: None,
            first_fault: None,
            load_error: None,
            disposal_counts: ExtensionDisposalCounts::default(),
        }
    }

    /// Test a normal terminal cause and a fault-free, fully drained closed end.
    pub fn is_clean_end(&self, binding: RunBinding, expected: ExtensionEndCause) -> bool {
        self.binding == binding
            && matches!(
                expected,
                ExtensionEndCause::Stopped | ExtensionEndCause::Completed
            )
            && self.cause == expected
            && self.vm_dropped
            && self.hook_failure.is_none()
            && self.first_fault.is_none()
            && self.load_error.is_none()
            && self.disposal_counts == ExtensionDisposalCounts::default()
    }
}

/// A rejected dock transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ExtensionError {
    /// The dock has no claimed execution.
    #[error("extension dock has no claimed execution")]
    NotClaimed,
    /// The Start already claimed this dock.
    #[error("this Start already ran")]
    AlreadyClaimed,
    /// The operation used a different execution identity.
    #[error("extension run binding does not match")]
    WrongBinding,
    /// The operation tried to use a full queue.
    #[error("extension queue is full")]
    Capacity,
    /// The operation tried to use an invalid awaited-call transition.
    #[error("extension completion does not match the pending await")]
    CompletionMismatch,
    /// The dock no longer accepts calls.
    #[error("extension dock is sealed or closed")]
    Sealed,
    /// The host revoked this run.
    #[error("extension run was revoked")]
    Revoked,
    /// The dock already retains its terminal record.
    #[error("extension dock already has an end record")]
    AlreadyClosed,
    /// Argument data exceeded a plain-value bound.
    #[error("extension argument is not valid bounded plain data")]
    InvalidValue,
    /// The dock rejected an inconsistent protocol transition.
    #[error("extension protocol transition is invalid")]
    Protocol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExtensionHandleState {
    Staged,
    Queued,
    Taken,
    Awaiting,
}

struct ExtensionHandle {
    sequence: u64,
    state: ExtensionHandleState,
}
struct DockState {
    binding: Option<RunBinding>,
    claimed: bool,
    sealed: bool,
    finished: bool,
    closed: bool,
    next_sequence: u64,
    sequence_exhausted: bool,
    staged: VecDeque<ExtensionCall>,
    queued: VecDeque<ExtensionCall>,
    handles: VecDeque<ExtensionHandle>,
    staged_awaiting: Option<u64>,
    staged_await_handle: Option<u64>,
    awaiting: Option<u64>,
    await_delivered: bool,
    completion: Option<(u64, ExtensionPayload)>,
    staged_finish: bool,
    disposed: ExtensionDisposalCounts,
    first_fault: Option<ExtensionFault>,
    end: Option<ExtensionEnd>,
}

/// One bounded bridge between a VM and its host owner.
pub struct ExtensionDock {
    state: Mutex<DockState>,
    fenced: AtomicBool,
    revoked: AtomicBool,
}

impl std::fmt::Debug for ExtensionDock {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExtensionDock")
            .field("binding", &self.binding())
            .field("fenced", &self.is_fenced())
            .field("has_fault", &self.fault().is_some())
            .field("has_end", &self.end().is_some())
            .finish()
    }
}

impl Default for ExtensionDock {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtensionDock {
    /// Create an empty dock for one future Start claim.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(DockState {
                binding: None,
                claimed: false,
                sealed: false,
                finished: false,
                closed: false,
                next_sequence: 1,
                sequence_exhausted: false,
                staged: VecDeque::with_capacity(EXTENSION_QUEUE_CAPACITY),
                queued: VecDeque::with_capacity(EXTENSION_QUEUE_CAPACITY),
                handles: VecDeque::with_capacity(EXTENSION_MAX_HANDLES),
                staged_awaiting: None,
                staged_await_handle: None,
                awaiting: None,
                await_delivered: false,
                completion: None,
                staged_finish: false,
                disposed: ExtensionDisposalCounts::default(),
                first_fault: None,
                end: None,
            }),
            fenced: AtomicBool::new(false),
            revoked: AtomicBool::new(false),
        }
    }

    /// Claim this dock for exactly one runtime Start.
    pub fn claim(&self, binding: RunBinding) -> Result<(), ExtensionError> {
        let mut state = self.state.lock();
        if state.binding.is_none() {
            state.binding = Some(binding);
        }
        if state.binding != Some(binding) {
            Self::fault_locked(
                &mut state,
                binding,
                None,
                ExtensionEntry::Setup,
                ExtensionFaultCode::WrongBinding,
                "dock claim used a different run binding",
            );
            return Err(ExtensionError::WrongBinding);
        }
        if state.claimed || state.closed {
            Self::fault_locked(
                &mut state,
                binding,
                None,
                ExtensionEntry::Setup,
                ExtensionFaultCode::AlreadyClaimed,
                "this Start already ran",
            );
            return Err(ExtensionError::AlreadyClaimed);
        }
        if self.is_revoked() || state.sealed {
            Self::fault_locked(
                &mut state,
                binding,
                None,
                ExtensionEntry::Setup,
                ExtensionFaultCode::Revoked,
                "dock was revoked before this Start claim",
            );
            return Err(ExtensionError::Revoked);
        }
        state.claimed = true;
        Ok(())
    }

    /// Read the bound identity without consuming dock state.
    pub fn binding(&self) -> Option<RunBinding> {
        self.state.lock().binding
    }

    /// Read the lock-free entry/egress fence.
    pub fn is_fenced(&self) -> bool {
        self.fenced.load(Ordering::Acquire)
    }

    /// Read whether the slot owner revoked this dock.
    pub fn is_revoked(&self) -> bool {
        self.revoked.load(Ordering::Acquire)
    }

    /// Fence VM entries and egress without revoking a successful subject.
    pub fn publish_fence(&self) {
        self.fenced.store(true, Ordering::Release);
    }

    /// Publish the owner fence without taking the dock lock or disposing work.
    pub fn publish_revocation(&self) {
        self.revoked.store(true, Ordering::Release);
        self.publish_fence();
    }

    /// Read whether the scenario staged and committed terminal Finish.
    pub fn is_finished(&self) -> bool {
        self.state.lock().finished
    }

    /// Check whether one delivered await has a payload ready for resume.
    pub fn completion_ready(&self, binding: RunBinding, sequence: u64) -> bool {
        if self.is_revoked() {
            return false;
        }
        let mut state = self.state.lock();
        Self::check_binding(&mut state, binding, ExtensionEntry::Tick)
            && state.awaiting == Some(sequence)
            && state.await_delivered
            && state
                .completion
                .is_some_and(|(completed, _)| completed == sequence)
    }
    /// Read the delivered await that owns the empty completion slot.
    pub fn pending_await(&self, binding: RunBinding) -> Option<u64> {
        if self.is_revoked() {
            return None;
        }
        let mut state = self.state.lock();
        if !Self::check_binding(&mut state, binding, ExtensionEntry::Tick) {
            return None;
        }
        let sequence = state.awaiting?;
        (state.await_delivered && state.completion.is_none()).then_some(sequence)
    }

    /// Await one request handle after the host has taken it.
    pub fn begin_await(&self, binding: RunBinding, sequence: u64) -> Result<(), ExtensionError> {
        let mut state = self.state.lock();
        if !Self::check_binding(&mut state, binding, ExtensionEntry::Tick) {
            return Err(ExtensionError::WrongBinding);
        }
        if !state.claimed {
            return Err(ExtensionError::NotClaimed);
        }
        if state.closed || self.is_revoked() {
            return Err(ExtensionError::Revoked);
        }
        if state.awaiting.is_some()
            || state.staged_awaiting.is_some()
            || state.staged_await_handle.is_some()
            || state.completion.is_some()
            || !state.handles.iter().any(|handle| {
                handle.sequence == sequence && handle.state == ExtensionHandleState::Taken
            })
        {
            Self::fault_locked(
                &mut state,
                binding,
                Some(sequence),
                ExtensionEntry::Tick,
                ExtensionFaultCode::Protocol,
                "await does not own a taken request handle",
            );
            return Err(ExtensionError::CompletionMismatch);
        }
        state.staged_await_handle = Some(sequence);
        Ok(())
    }

    /// Finish one taken request without reserving an awaited completion.
    pub fn finish_request(&self, binding: RunBinding, sequence: u64) -> Result<(), ExtensionError> {
        let mut state = self.state.lock();
        if !Self::check_binding(&mut state, binding, ExtensionEntry::Tick) {
            return Err(ExtensionError::WrongBinding);
        }
        let Some(index) = state.handles.iter().position(|handle| {
            handle.sequence == sequence && handle.state == ExtensionHandleState::Taken
        }) else {
            Self::fault_locked(
                &mut state,
                binding,
                Some(sequence),
                ExtensionEntry::Tick,
                ExtensionFaultCode::HostProtocol,
                "host finished a request it did not own",
            );
            return Err(ExtensionError::Protocol);
        };
        if state.awaiting == Some(sequence) {
            Self::fault_locked(
                &mut state,
                binding,
                Some(sequence),
                ExtensionEntry::Tick,
                ExtensionFaultCode::HostProtocol,
                "host finished a request while its await was active",
            );
            return Err(ExtensionError::Protocol);
        }
        state.handles.remove(index);
        Ok(())
    }

    fn discard_staged(state: &mut DockState) {
        state.disposed.staged = state.disposed.staged.saturating_add(state.staged.len());
        if let (Some(first), Some(last)) = (state.staged.front(), state.staged.back()) {
            let first = first.sequence;
            let last = last.sequence;
            state
                .handles
                .retain(|handle| handle.sequence < first || handle.sequence > last);
        }
        state.staged.clear();
        state.staged_awaiting = None;
        state.staged_await_handle = None;
        state.staged_finish = false;
    }

    /// Stage one bounded call for the current synchronous surface dispatch.
    ///
    /// The host cannot take the call until `finish_dispatch` accepts the Tick.
    pub fn stage(
        &self,
        binding: RunBinding,
        entry: ExtensionEntry,
        name: &'static str,
        mode: ExtensionMode,
        arguments: ExtensionValue,
    ) -> Result<u64, ExtensionError> {
        let mut state = self.state.lock();
        if !Self::check_binding(&mut state, binding, entry) {
            return Err(ExtensionError::WrongBinding);
        }
        if !state.claimed {
            Self::fault_locked(
                &mut state,
                binding,
                None,
                entry,
                ExtensionFaultCode::Protocol,
                "call arrived before the dock claim",
            );
            return Err(ExtensionError::NotClaimed);
        }
        if state.closed || self.is_revoked() {
            Self::fault_locked(
                &mut state,
                binding,
                None,
                entry,
                ExtensionFaultCode::Revoked,
                "call arrived after revocation or close",
            );
            return Err(ExtensionError::Revoked);
        }
        if state.sealed || state.staged_finish {
            Self::fault_locked(
                &mut state,
                binding,
                None,
                entry,
                ExtensionFaultCode::PostSeal,
                "call arrived after extension requests were sealed",
            );
            return Err(ExtensionError::Sealed);
        }
        if entry != ExtensionEntry::Tick {
            Self::fault_locked(
                &mut state,
                binding,
                None,
                entry,
                ExtensionFaultCode::WrongEntry,
                "extension calls are allowed only during Tick",
            );
            return Err(ExtensionError::Protocol);
        }
        if !valid_function_name(name) {
            Self::fault_locked(
                &mut state,
                binding,
                None,
                entry,
                ExtensionFaultCode::Conversion,
                "extension function name is invalid",
            );
            return Err(ExtensionError::InvalidValue);
        }
        if arguments.validate_bounded().is_err() {
            Self::fault_locked(
                &mut state,
                binding,
                None,
                entry,
                ExtensionFaultCode::Conversion,
                "extension arguments exceed plain-value bounds",
            );
            return Err(ExtensionError::InvalidValue);
        }
        if state.queued.len() + state.staged.len() >= EXTENSION_QUEUE_CAPACITY {
            Self::fault_locked(
                &mut state,
                binding,
                None,
                entry,
                ExtensionFaultCode::Capacity,
                "extension queue is full",
            );
            return Err(ExtensionError::Capacity);
        }
        if mode == ExtensionMode::Await
            && (state.awaiting.is_some()
                || state.staged_awaiting.is_some()
                || state.staged_await_handle.is_some())
        {
            Self::fault_locked(
                &mut state,
                binding,
                None,
                entry,
                ExtensionFaultCode::Protocol,
                "a second awaited extension call is already owned",
            );
            return Err(ExtensionError::Protocol);
        }
        if mode != ExtensionMode::Finish && state.handles.len() >= EXTENSION_MAX_HANDLES {
            Self::fault_locked(
                &mut state,
                binding,
                None,
                entry,
                ExtensionFaultCode::Capacity,
                "request handle capacity is full",
            );
            return Err(ExtensionError::Capacity);
        }
        if state.sequence_exhausted {
            Self::fault_locked(
                &mut state,
                binding,
                None,
                entry,
                ExtensionFaultCode::Protocol,
                "extension request sequence is exhausted",
            );
            return Err(ExtensionError::Protocol);
        }
        let sequence = state.next_sequence;
        match sequence.checked_add(1) {
            Some(next) => state.next_sequence = next,
            None => state.sequence_exhausted = true,
        }
        if mode != ExtensionMode::Finish {
            state.handles.push_back(ExtensionHandle {
                sequence,
                state: ExtensionHandleState::Staged,
            });
        }
        state.staged.push_back(ExtensionCall {
            binding,
            sequence,
            entry,
            name,
            mode,
            arguments,
        });
        if mode == ExtensionMode::Await {
            state.staged_awaiting = Some(sequence);
        }
        if mode == ExtensionMode::Finish {
            state.staged_finish = true;
        }
        Ok(sequence)
    }

    /// Commit or discard every call staged by one dispatch.
    pub fn finish_dispatch(
        &self,
        binding: RunBinding,
        accepted: bool,
    ) -> Result<(), ExtensionError> {
        let mut state = self.state.lock();
        if !Self::check_binding(&mut state, binding, ExtensionEntry::Tick) {
            return Err(ExtensionError::WrongBinding);
        }
        if !state.claimed {
            Self::fault_locked(
                &mut state,
                binding,
                None,
                ExtensionEntry::Tick,
                ExtensionFaultCode::Protocol,
                "dispatch ended before the dock claim",
            );
            return Err(ExtensionError::NotClaimed);
        }
        if !accepted {
            Self::discard_staged(&mut state);
            return Ok(());
        }
        if self.is_revoked() || state.closed {
            return Err(ExtensionError::Revoked);
        }
        let state = &mut *state;
        for call in &state.staged {
            if let Some(handle) = state
                .handles
                .iter_mut()
                .find(|handle| handle.sequence == call.sequence)
            {
                handle.state = ExtensionHandleState::Queued;
            }
        }
        state.queued.append(&mut state.staged);
        if let Some(sequence) = state.staged_awaiting.take() {
            state.awaiting = Some(sequence);
            state.await_delivered = false;
        }
        if let Some(sequence) = state.staged_await_handle.take() {
            if let Some(handle) = state
                .handles
                .iter_mut()
                .find(|handle| handle.sequence == sequence)
            {
                handle.state = ExtensionHandleState::Awaiting;
            }
            state.awaiting = Some(sequence);
            state.await_delivered = true;
        }
        if state.staged_finish {
            state.sealed = true;
            state.finished = true;
            self.fenced.store(true, Ordering::Release);
            state.staged_finish = false;
        }
        Ok(())
    }

    /// Take the next committed call for the matching run.
    pub fn take(&self, binding: RunBinding) -> Result<Option<ExtensionCall>, ExtensionError> {
        let mut state = self.state.lock();
        if !Self::check_binding(&mut state, binding, ExtensionEntry::Tick) {
            return Err(ExtensionError::WrongBinding);
        }
        if !state.claimed {
            Self::fault_locked(
                &mut state,
                binding,
                None,
                ExtensionEntry::Tick,
                ExtensionFaultCode::HostProtocol,
                "host took a call before the dock claim",
            );
            return Err(ExtensionError::NotClaimed);
        }
        if state.closed || self.is_revoked() {
            return Ok(None);
        }
        let Some(call) = state.queued.pop_front() else {
            return Ok(None);
        };
        if state.awaiting == Some(call.sequence) {
            state.await_delivered = true;
        }
        let awaiting = state.awaiting == Some(call.sequence);
        if let Some(handle) = state
            .handles
            .iter_mut()
            .find(|handle| handle.sequence == call.sequence)
        {
            handle.state = if awaiting {
                ExtensionHandleState::Awaiting
            } else {
                ExtensionHandleState::Taken
            };
        }
        Ok(Some(call))
    }

    /// Complete the one matching awaited call without overwriting a payload.
    pub fn complete(
        &self,
        binding: RunBinding,
        sequence: u64,
        payload: ExtensionPayload,
    ) -> Result<(), ExtensionError> {
        let mut state = self.state.lock();
        if !Self::check_binding(&mut state, binding, ExtensionEntry::Tick) {
            return Err(ExtensionError::WrongBinding);
        }
        if state.closed
            || self.is_revoked()
            || state.awaiting != Some(sequence)
            || !state.await_delivered
            || !state.handles.iter().any(|handle| {
                handle.sequence == sequence && handle.state == ExtensionHandleState::Awaiting
            })
            || state.completion.is_some()
        {
            Self::fault_locked(
                &mut state,
                binding,
                Some(sequence),
                ExtensionEntry::Tick,
                ExtensionFaultCode::HostProtocol,
                "host completion does not own the pending await",
            );
            return Err(ExtensionError::CompletionMismatch);
        }
        state.completion = Some((sequence, payload));
        Ok(())
    }

    /// Consume the matching completion only when the VM owner accepts a resume.
    pub fn take_completion(
        &self,
        binding: RunBinding,
        sequence: u64,
    ) -> Result<Option<ExtensionPayload>, ExtensionError> {
        let mut state = self.state.lock();
        if !Self::check_binding(&mut state, binding, ExtensionEntry::Tick) {
            return Err(ExtensionError::WrongBinding);
        }
        if state.closed || self.is_revoked() {
            return Err(ExtensionError::Revoked);
        }
        if state.awaiting != Some(sequence)
            || !state.await_delivered
            || !state.handles.iter().any(|handle| {
                handle.sequence == sequence && handle.state == ExtensionHandleState::Awaiting
            })
        {
            Self::fault_locked(
                &mut state,
                binding,
                Some(sequence),
                ExtensionEntry::Tick,
                ExtensionFaultCode::Protocol,
                "VM resume does not own the pending await",
            );
            return Err(ExtensionError::CompletionMismatch);
        }
        match state.completion.take() {
            Some((completed, payload)) if completed == sequence => {
                state.await_delivered = false;
                state.awaiting = None;
                if let Some(index) = state
                    .handles
                    .iter()
                    .position(|handle| handle.sequence == sequence)
                {
                    state.handles.remove(index);
                }
                Ok(Some(payload))
            }
            Some((completed, payload)) => {
                state.completion = Some((completed, payload));
                Self::fault_locked(
                    &mut state,
                    binding,
                    Some(sequence),
                    ExtensionEntry::Tick,
                    ExtensionFaultCode::Protocol,
                    "completion slot contains a different sequence",
                );
                Err(ExtensionError::CompletionMismatch)
            }
            None => Ok(None),
        }
    }

    /// Close new call admissions while preserving already committed host work.
    pub fn seal(&self, binding: RunBinding) -> Result<(), ExtensionError> {
        let mut state = self.state.lock();
        if !Self::check_binding(&mut state, binding, ExtensionEntry::Tick) {
            return Err(ExtensionError::WrongBinding);
        }
        if state.closed || self.is_revoked() {
            return Err(ExtensionError::Revoked);
        }
        state.sealed = true;
        Ok(())
    }

    /// Revoke admissions and dispose queued work while retaining the first fault.
    pub fn revoke(&self) {
        self.publish_revocation();
        let mut state = self.state.lock();
        state.sealed = true;
        state.disposed.staged = state.disposed.staged.saturating_add(state.staged.len());
        state.disposed.queued = state.disposed.queued.saturating_add(state.queued.len());
        state.disposed.completion |= state.completion.is_some();
        state.disposed.handles = state.disposed.handles.saturating_add(state.handles.len());
        state.staged.clear();
        state.queued.clear();
        state.handles.clear();
        state.staged_awaiting = None;
        state.staged_await_handle = None;
        state.awaiting = None;
        state.await_delivered = false;
        state.completion = None;
        state.staged_finish = false;
    }

    /// Record the first refusal before its Lua callback returns an error.
    pub fn latch_fault(
        &self,
        binding: RunBinding,
        sequence: Option<u64>,
        entry: ExtensionEntry,
        code: ExtensionFaultCode,
        detail: &str,
    ) {
        let mut state = self.state.lock();
        Self::fault_locked(&mut state, binding, sequence, entry, code, detail);
    }

    /// Read the immutable first fault, if one exists.
    pub fn fault(&self) -> Option<ExtensionFault> {
        self.state.lock().first_fault.clone()
    }

    /// Close the dock and retain its typed terminal record.
    pub fn close(&self, mut end: ExtensionEnd) {
        let mut state = self.state.lock();
        if let Some(binding) = state.binding {
            if end.binding != binding {
                Self::fault_locked(
                    &mut state,
                    binding,
                    None,
                    ExtensionEntry::Hook,
                    ExtensionFaultCode::WrongBinding,
                    "extension end used a different run binding",
                );
                end.binding = binding;
            }
        } else {
            state.binding = Some(end.binding);
        }
        if state.end.is_some() {
            Self::fault_locked(
                &mut state,
                end.binding,
                None,
                ExtensionEntry::Hook,
                ExtensionFaultCode::Protocol,
                "extension dock already has an end record",
            );
            return;
        }
        state.disposed.staged = state.disposed.staged.saturating_add(state.staged.len());
        state.disposed.queued = state.disposed.queued.saturating_add(state.queued.len());
        state.disposed.completion |= state.completion.is_some();
        state.disposed.handles = state.disposed.handles.saturating_add(state.handles.len());
        end.first_fault = state.first_fault.clone();
        end.disposal_counts = state.disposed;
        state.staged.clear();
        state.queued.clear();
        state.handles.clear();
        state.staged_awaiting = None;
        state.staged_await_handle = None;
        state.awaiting = None;
        state.await_delivered = false;
        state.completion = None;
        state.staged_finish = false;
        state.sealed = true;
        state.closed = true;
        self.fenced.store(true, Ordering::Release);
        state.end = Some(end);
    }

    /// Read the typed end after close.
    pub fn end(&self) -> Option<ExtensionEnd> {
        self.state.lock().end.clone()
    }

    fn check_binding(state: &mut DockState, binding: RunBinding, entry: ExtensionEntry) -> bool {
        match state.binding {
            Some(expected) if expected == binding => true,
            Some(expected) => {
                Self::fault_locked(
                    state,
                    expected,
                    None,
                    entry,
                    ExtensionFaultCode::WrongBinding,
                    "operation used a different run binding",
                );
                false
            }
            None => {
                Self::fault_locked(
                    state,
                    binding,
                    None,
                    entry,
                    ExtensionFaultCode::Protocol,
                    "operation arrived before a run binding was claimed",
                );
                false
            }
        }
    }

    fn fault_locked(
        state: &mut DockState,
        binding: RunBinding,
        sequence: Option<u64>,
        entry: ExtensionEntry,
        code: ExtensionFaultCode,
        detail: &str,
    ) {
        if state.first_fault.is_some() {
            return;
        }
        let owner = state.binding.unwrap_or(binding);
        state.first_fault = Some(ExtensionFault {
            binding: owner,
            sequence,
            entry,
            code,
            detail: BoundedText::new(detail),
        });
    }
}

fn valid_function_name(name: &str) -> bool {
    if name.is_empty() || name.len() > EXTENSION_MAX_TEXT_BYTES {
        return false;
    }
    name.split('.').all(|segment| {
        let mut bytes = segment.bytes();
        bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
            && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    })
}
