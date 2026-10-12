//! Per-request adapter custody, not a seat ledger or a fake service.
//! §25 R5-8.3 terminal receipt retires every control and the pending promise.
//! §28.4 keeps an unacknowledged start and parent cancellation as one unit.
use super::request::PendingPayload;
use api::identity::RequestId;
use api::script_contract::{CallbackCancelReason, CallbackCancellation, Decide, DecisionSequence};

/// State the adapter retains for a single request. It owns no JS values;
/// a full-build mirror keeps its actual Promise handle until `receive_terminal`
/// returns true. Native callers need no isolate to use the same semantics.
#[derive(Debug)]
pub struct CallbackCustody<A> {
    of: RequestId,
    payload: Option<PendingPayload>,
    local_parent: Option<RequestId>,
    acknowledged_ever: bool,
    last_prompt_invoked: Option<DecisionSequence>,
    last_answered: Option<DecisionSequence>,
    last_event_sequence: u32,
    decide: Option<Decide<A>>,
    cancel: Option<CallbackCancellation>,
    terminal_received: bool,
}
impl<A> CallbackCustody<A> {
    pub fn new(of: RequestId, payload: PendingPayload, local_parent: Option<RequestId>) -> Self {
        Self {
            of,
            payload: Some(payload),
            local_parent,
            acknowledged_ever: false,
            last_prompt_invoked: None,
            last_answered: None,
            last_event_sequence: 0,
            decide: None,
            cancel: None,
            terminal_received: false,
        }
    }
    pub const fn of(&self) -> RequestId {
        self.of
    }
    pub const fn payload(&self) -> Option<&PendingPayload> {
        self.payload.as_ref()
    }
    /// Payload acknowledgement must not erase hybrid ancestry custody.
    pub const fn local_parent(&self) -> Option<RequestId> {
        self.local_parent
    }
    pub const fn acknowledged_ever(&self) -> bool {
        self.acknowledged_ever
    }
    pub const fn terminal_received(&self) -> bool {
        self.terminal_received
    }
    /// Receipt of live state is sticky across a kept reconnect and lost pages.
    pub fn acknowledge_live(&mut self) {
        if !self.terminal_received {
            self.acknowledged_ever = true;
            self.payload = None;
        }
    }
    /// Reposted prompt pages do not invoke an already seen decision again.
    pub fn invoke_prompt(&mut self, sequence: DecisionSequence) -> bool {
        if self.terminal_received
            || self.cancel.is_some()
            || self
                .last_prompt_invoked
                .is_some_and(|seen| sequence <= seen)
        {
            return false;
        }
        self.last_prompt_invoked = Some(sequence);
        true
    }
    /// Consume an event only once, in ascending sequence order. Every event
    /// gets its own invocation; there is no per-tick notification coalescing.
    pub fn consume_event(&mut self, sequence: u32) -> bool {
        if self.terminal_received || sequence <= self.last_event_sequence {
            return false;
        }
        self.last_event_sequence = sequence;
        true
    }
    /// Carry the actual answer once. A pending cancellation wins any race.
    pub fn answer(&mut self, sequence: DecisionSequence, answer: A) -> bool {
        if self.terminal_received
            || self.cancel.is_some()
            || self.last_prompt_invoked != Some(sequence)
            || self.decide.is_some()
            || self
                .last_answered
                .is_some_and(|answered| sequence <= answered)
        {
            return false;
        }
        self.decide = Some(Decide::new(self.of, sequence, answer));
        self.last_answered = Some(sequence);
        true
    }
    /// Cancellation replaces Decide and is encoded before any late copy.
    /// ParentAborted retains an unacknowledged payload as one cancellation unit.
    pub fn cancel(&mut self, reason: CallbackCancelReason) -> bool {
        if self.terminal_received || self.cancel.is_some() {
            return false;
        }
        self.decide = None;
        self.cancel = Some(CallbackCancellation::new(self.of, reason));
        true
    }
    /// A live page can acknowledge a decision without retiring the request.
    pub fn decisions_consumed(&mut self, sequence: u32) {
        if self
            .decide
            .as_ref()
            .is_some_and(|decision| decision.sequence().get() <= sequence)
        {
            self.decide = None;
        }
    }
    pub const fn decide(&self) -> Option<&Decide<A>> {
        self.decide.as_ref()
    }
    pub const fn cancellation(&self) -> Option<CallbackCancellation> {
        self.cancel
    }
    /// Consume a request terminal exactly once. The caller now releases its
    /// pending Promise and settles its real wait. A late copied control never
    /// revives custody; a lost acknowledgement page needs no special fallback.
    pub fn receive_terminal(&mut self) -> bool {
        if self.terminal_received {
            return false;
        }
        self.terminal_received = true;
        self.acknowledged_ever = true;
        self.payload = None;
        self.decide = None;
        self.cancel = None;
        self.local_parent = None;
        true
    }
}
