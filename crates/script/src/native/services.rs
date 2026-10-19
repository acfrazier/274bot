//! Native-first capability injection (§26 item 3, §27 §5, §21 §4.2).
//! This is a borrowed facade over the existing NativeActions/ActionContext,
//! not a callable fake seat. T1.3 installs it in real owner-controlled contexts.
use super::abi::{FrozenPathStatus, HookCharacterization, HookPath};
use super::seat::{GameplaySeat, SeatAnswer, SeatError};
use super::{ActionContext, NativeActions};
use api::identity::{ActiveTime, ExecutionId, Fence, FenceError, RequestId, Scope};
use api::lifecycle::SessionGate;
use api::local_state::{AccountKey, ContentScope, LocalStatePort};
use api::script_contract::DecisionSequence;
use api::selected::SelectedPin;
use api::snapshot::SnapshotView;

/// Closed, source-backed callback point. A string cannot mint a point;
/// callers must name one family/member row in the frozen ABI table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromptPoint(&'static HookCharacterization);
impl PromptPoint {
    pub fn from_contract_row(row: &'static HookCharacterization) -> Option<Self> {
        (row.frozen_path() == FrozenPathStatus::MatchesFrozenCallForm
            && matches!(
                row.path(),
                HookPath::Awaited | HookPath::CallSync | HookPath::Ask
            ))
        .then_some(Self(row))
    }
    pub const fn member(self) -> &'static str {
        self.0.member()
    }
    pub const fn path(self) -> HookPath {
        self.0.path()
    }
    pub const fn families(self) -> &'static [&'static str] {
        self.0.families()
    }
}

/// Owner-borrowed prompt endpoint; no callback producer or JS scheduler lives
/// in this trait. A native family may use it only while its admitted request
/// and the owner's execution fence are both live.
pub trait SeatPromptPort {
    fn run(&self) -> api::selected::RunKey;
    fn prompt(
        &mut self,
        id: RequestId,
        point: PromptPoint,
        args: &[u8],
    ) -> Result<DecisionSequence, SeatError>;
    fn take_answer(&mut self, id: RequestId) -> Option<SeatAnswer>;
}

impl<R> SeatPromptPort for GameplaySeat<R> {
    fn run(&self) -> api::selected::RunKey {
        GameplaySeat::run(self)
    }
    fn prompt(
        &mut self,
        id: RequestId,
        point: PromptPoint,
        args: &[u8],
    ) -> Result<DecisionSequence, SeatError> {
        GameplaySeat::prompt_with_details(self, id, point, args)
    }
    fn take_answer(&mut self, id: RequestId) -> Option<SeatAnswer> {
        GameplaySeat::take_answer(self, id)
    }
}

/// The mutable owner supplies the existing fence and session gate. This value
/// cannot mint request ids, replace a gate, or grant host-actor lineage.
#[derive(Clone, Copy)]
pub struct ServiceAuthority<'a> {
    execution: ExecutionId,
    fence: &'a Fence<ExecutionId>,
    gate: &'a SessionGate,
}
impl<'a> ServiceAuthority<'a> {
    pub const fn new(
        execution: ExecutionId,
        fence: &'a Fence<ExecutionId>,
        gate: &'a SessionGate,
    ) -> Self {
        Self {
            execution,
            fence,
            gate,
        }
    }
    pub const fn execution(&self) -> ExecutionId {
        self.execution
    }
    pub const fn scope(&self) -> Scope {
        Scope::Execution(self.execution)
    }
    /// Recheck at the consuming owner, not merely at worker submission.
    pub fn check(&self) -> Result<(), ServiceError> {
        self.fence.admit(self.execution)?;
        let session = self
            .execution
            .run()
            .session_key()
            .ok_or(ServiceError::UnboundRun)?;
        if self.gate.ready() != Some(session) {
            return Err(ServiceError::SessionHeld);
        }
        Ok(())
    }
}

/// Explicit borrowed state capability. The composition root supplies the real
/// port and durable identity; no default store or fake implementation is made.
pub struct LocalStateCapability<'a> {
    port: &'a dyn LocalStatePort,
    account: &'a AccountKey,
    scope: &'a ContentScope,
}
impl<'a> LocalStateCapability<'a> {
    /// Bind a port to the account and selected content used by this script.
    pub fn new(
        port: &'a dyn LocalStatePort,
        account: &'a AccountKey,
        scope: &'a ContentScope,
    ) -> Self {
        Self {
            port,
            account,
            scope,
        }
    }
    /// Durable account identity, never a run/slot/label key.
    pub const fn account(&self) -> &AccountKey {
        self.account
    }
    /// Selected engine revision and content identity.
    pub const fn scope(&self) -> &ContentScope {
        self.scope
    }
}

/// Borrowed native script surface. It uses the same native action facility and
/// observations as compiled cards, with explicit run/session authority. No Load,
/// isolate, deno, JS value, file discovery or hidden default store is required.
pub struct ScriptServices<'a, 'frame> {
    authority: ServiceAuthority<'a>,
    actions: &'a mut NativeActions,
    context: &'a mut ActionContext<'frame>,
    state: Option<LocalStateCapability<'a>>,
    prompt_port: Option<&'a mut dyn SeatPromptPort>,
}
impl<'a, 'frame> ScriptServices<'a, 'frame> {
    /// The owner passes the existing frame and owner-controlled capabilities.
    /// Missing local state stays absent instead of selecting a default store.
    pub fn new(
        authority: ServiceAuthority<'a>,
        actions: &'a mut NativeActions,
        context: &'a mut ActionContext<'frame>,
        state: Option<LocalStateCapability<'a>>,
    ) -> Result<Self, ServiceError> {
        if context.run() != authority.execution().run() {
            return Err(ServiceError::ContextMismatch);
        }
        Ok(Self {
            authority,
            actions,
            context,
            state,
            prompt_port: None,
        })
    }
    /// Add the current owner's seat without altering the frozen four-argument
    /// constructor. A seat from a different run cannot be attached.
    pub fn with_prompt_port<'port>(
        self,
        port: &'port mut dyn SeatPromptPort,
    ) -> Result<ScriptServices<'port, 'frame>, ServiceError>
    where
        'a: 'port,
    {
        if port.run() != self.authority.execution().run() {
            return Err(ServiceError::ContextMismatch);
        }
        Ok(ScriptServices {
            authority: self.authority,
            actions: self.actions,
            context: self.context,
            state: self.state,
            prompt_port: Some(port),
        })
    }
    pub const fn authority(&self) -> &ServiceAuthority<'a> {
        &self.authority
    }
    /// Versioned selected content already resolved by the composition root.
    pub fn selected(&self) -> &SelectedPin {
        self.context.pin()
    }
    /// Borrow the real observation applied before the pass. Never copy a world.
    pub fn snapshot(&self) -> SnapshotView<'frame> {
        self.context.snapshot()
    }
    /// Shared F0 active-work time, paused by the existing owner clock.
    pub fn active_time(&self) -> ActiveTime {
        ActiveTime::ZERO.saturating_add(self.context.active_now())
    }
    /// Use the existing typed native actions after rechecking authority.
    /// Returned handles retain their existing HostAuthority dispatch checks.
    pub fn with_actions<R>(
        &mut self,
        action: impl FnOnce(&mut NativeActions, &mut ActionContext<'frame>) -> R,
    ) -> Result<R, ServiceError> {
        self.authority.check()?;
        Ok(action(self.actions, self.context))
    }
    /// Post one native-family callback prompt against a live request. Until an
    /// I-family owns a native callback source, no production family calls it.
    pub fn prompt(
        &mut self,
        id: RequestId,
        point: PromptPoint,
        args: &[u8],
    ) -> Result<DecisionSequence, ServiceError> {
        let seat = self
            .prompt_port
            .as_deref_mut()
            .ok_or(ServiceError::PromptPortUnavailable)?;
        self.authority.check()?;
        seat.prompt(id, point, args).map_err(ServiceError::Seat)
    }
    /// Consume the answer only after the native machine resumes from its
    /// prompt; stale or revoked requests cannot read it through this facade.
    pub fn take_decision(&mut self, id: RequestId) -> Result<Option<SeatAnswer>, ServiceError> {
        let seat = self
            .prompt_port
            .as_deref_mut()
            .ok_or(ServiceError::PromptPortUnavailable)?;
        self.authority.check()?;
        Ok(seat.take_answer(id))
    }
    /// Borrow the explicitly injected durable identity without allocating.
    pub fn state_identity(&self) -> Option<(&AccountKey, &ContentScope)> {
        self.state
            .as_ref()
            .map(|state| (state.account(), state.scope()))
    }
    /// Use the real nonblocking lease/submit/receipt port with explicit keys.
    /// Functional port installation is T5.2; absence is a typed refusal.
    pub fn with_state<R>(
        &self,
        operation: impl FnOnce(&dyn LocalStatePort, &AccountKey, &ContentScope) -> R,
    ) -> Result<R, ServiceError> {
        self.authority.check()?;
        let state = self.state.as_ref().ok_or(ServiceError::StateUnavailable)?;
        Ok(operation(state.port, state.account, state.scope))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("native script service authority is not installed")]
    AuthorityUnavailable,
    #[error(transparent)]
    Fence(#[from] FenceError<ExecutionId>),
    #[error("native script session is not ready or is held")]
    SessionHeld,
    #[error("native script run is unbound")]
    UnboundRun,
    #[error("native context and service authority disagree")]
    ContextMismatch,
    #[error("native script local-state capability is not injected")]
    StateUnavailable,
    #[error("native script prompt port is not injected")]
    PromptPortUnavailable,
    #[error("native script prompt refused: {0:?}")]
    Seat(SeatError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::abi::{HookPath, HOOK_CHARACTERIZATIONS};

    #[test]
    fn prompt_points_admit_only_frozen_hook_paths() {
        for row in HOOK_CHARACTERIZATIONS {
            let point = PromptPoint::from_contract_row(row);
            match (row.frozen_path(), row.path()) {
                (
                    FrozenPathStatus::MatchesFrozenCallForm,
                    HookPath::Awaited | HookPath::CallSync | HookPath::Ask,
                ) => {
                    let point = point.unwrap_or_else(|| {
                        panic!("frozen prompt row {} was refused", row.member())
                    });
                    assert_eq!(point.member(), row.member());
                    assert_eq!(point.path(), row.path());
                    assert_eq!(point.families(), row.families());
                }
                _ => assert!(
                    point.is_none(),
                    "unfrozen or unsupported row {} was admitted",
                    row.member()
                ),
            }
        }
        for path in [HookPath::Awaited, HookPath::CallSync, HookPath::Ask] {
            assert!(
                HOOK_CHARACTERIZATIONS.iter().any(|row| {
                    PromptPoint::from_contract_row(row).is_some_and(|point| point.path() == path)
                }),
                "no frozen PromptPoint row exercises {path:?}"
            );
        }
    }
}
