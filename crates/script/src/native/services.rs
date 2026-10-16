//! Native-first capability injection (§26 item 3, §27 §5, §21 §4.2).
//! This is a borrowed facade over the existing NativeActions/ActionContext,
//! not a callable fake seat. T1.3 installs it in real owner-controlled contexts.
use super::{ActionContext, NativeActions};
use api::identity::{ActiveTime, ExecutionId, Fence, FenceError, Scope};
use api::lifecycle::SessionGate;
use api::local_state::{AccountKey, ContentScope, LocalStatePort};
use api::selected::SelectedPin;
use api::snapshot::SnapshotView;

/// The mutable owner supplies the existing fence and session gate. This value
/// cannot mint request ids, replace a gate, or grant host-actor lineage.
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
}
impl<'a, 'frame> ScriptServices<'a, 'frame> {
    /// Composition passes the actual frame and capabilities, never substitutes
    /// a mock service. Production installation remains T1.3.
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
}
