//! The slot loop's half of the Guardian Fight port (design V11D 4.2 and 6.2).
//!
//! The Guardian ticks first in a frame, with no client and no slot lock: it
//! offers, revokes and reads through the claim source. The slot observation
//! runs after it and owns the dispatch. This value is what the two share in
//! that one frame: the token of the child the Guardian published and whether
//! the child may act, and the escape walk the Guardian staged and the walk the
//! observation sent. Staging is not sending: the walk goes out only after the
//! child's upkeep guard has said that no admitted input lock blocks the tick.
use api::random::FightToken;
use host::{EscapeIntent, SentEscape};
use parking_lot::Mutex;

/// The Fight state one slot shares between the Guardian and its observation.
#[derive(Default)]
pub(crate) struct FightPort {
    gate: Mutex<Gate>,
}

#[derive(Default)]
struct Gate {
    /// The fight the Guardian published this frame and whether its child may
    /// act: the session is ready apart from the Guardian's own hold, and no
    /// manual takeover holds the account.
    open: Option<(FightToken, bool)>,
    /// The walk out the Guardian staged and the observation has not sent.
    staged: Option<EscapeIntent>,
    /// The walk out the observation sent and the Guardian has not folded.
    sent: Option<SentEscape>,
}

impl FightPort {
    /// Publish this frame's fight. A token that changed or ended drops the
    /// walk staged and the walk sent under the old one.
    pub(crate) fn publish(&self, token: Option<FightToken>, online: bool) {
        let mut gate = self.gate.lock();
        gate.open = token.map(|token| (token, online));
        if gate
            .staged
            .as_ref()
            .is_some_and(|staged| Some(staged.token()) != token)
        {
            gate.staged = None;
        }
        if gate
            .sent
            .as_ref()
            .is_some_and(|sent| Some(sent.token()) != token)
        {
            gate.sent = None;
        }
    }

    /// The fight this frame published and whether its child may act.
    pub(crate) fn open(&self) -> Option<(FightToken, bool)> {
        self.gate.lock().open
    }

    /// Stage one walk out for the observation to send.
    pub(crate) fn stage(&self, intent: EscapeIntent) {
        self.gate.lock().staged = Some(intent);
    }

    /// The staged walk of `token`, taken once.
    pub(crate) fn take_staged(&self, token: FightToken) -> Option<EscapeIntent> {
        let mut gate = self.gate.lock();
        match gate.staged.take() {
            Some(intent) if intent.token() == token => Some(intent),
            _ => None,
        }
    }

    /// Record the walk the observation sent, for the Guardian's next tick.
    pub(crate) fn record_sent(&self, sent: SentEscape) {
        self.gate.lock().sent = Some(sent);
    }

    /// The sent walk of `token`, taken once.
    pub(crate) fn take_sent(&self, token: FightToken) -> Option<SentEscape> {
        let mut gate = self.gate.lock();
        match gate.sent.take() {
            Some(sent) if sent.token() == token => Some(sent),
            _ => None,
        }
    }

    /// Forget the fight of `token`: its child ended.
    pub(crate) fn release(&self, token: FightToken) {
        let mut gate = self.gate.lock();
        if gate.open.is_some_and(|(open, _)| open == token) {
            gate.open = None;
        }
        if gate
            .staged
            .as_ref()
            .is_some_and(|staged| staged.token() == token)
        {
            gate.staged = None;
        }
        if gate.sent.as_ref().is_some_and(|sent| sent.token() == token) {
            gate.sent = None;
        }
    }
}
