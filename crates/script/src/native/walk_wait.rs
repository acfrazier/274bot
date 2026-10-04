//! Instance-owned walk correlation, shared by host and isolate adapters.
//! A route terminal is not necessarily arrival. Adapters supply observations;
//! this core neither routes nor retains a borrowed scene.

use api::snapshot::WorldTile;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WalkKey {
    pub tile: WorldTile,
    pub radius: i32,
    pub allow_teleports: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostOutcome {
    pub seq: u64,
    pub generation: u64,
    pub request_id: u64,
    pub failed: bool,
    pub blocked: bool,
    pub key: WalkKey,
}

impl HostOutcome {
    pub const fn empty() -> Self {
        Self {
            seq: 0,
            generation: 0,
            request_id: 0,
            failed: false,
            blocked: false,
            key: WalkKey {
                tile: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                radius: 0,
                allow_teleports: false,
            },
        }
    }
}

/// Facts are borrowed by the adapter for this call only. Arrival uses the
/// adapter's scene predicate, never route-terminal success alone.
pub trait Observation {
    fn outcome(&self) -> HostOutcome;
    fn cancelled(&self) -> bool;
    fn arrived(&self, key: WalkKey) -> bool;
}

struct Wait {
    token: u64,
    key: WalkKey,
    settled: Option<bool>,
    seq_at_begin: u64,
    matched: Option<bool>,
    blocked: bool,
}

#[derive(Default)]
pub struct WalkSlot {
    wait: Option<Wait>,
}

impl WalkSlot {
    pub const fn new() -> Self {
        Self { wait: None }
    }

    pub fn reset(&mut self) {
        self.wait = None;
    }

    pub fn owns(&self, token: u64) -> bool {
        self.wait.as_ref().is_some_and(|wait| wait.token == token)
    }

    pub fn observe_outcome(&mut self, outcome: HostOutcome) {
        if let Some(wait) = self.wait.as_mut() {
            if Self::outcome_matches(outcome, wait) {
                wait.matched = Some(wait.matched.unwrap_or(true) && !outcome.failed);
                wait.blocked = wait.matched == Some(true) && outcome.blocked;
            }
        }
    }

    /// The adapter allocates a nonzero, non-reused correlation token. A new
    /// begin supersedes the prior wait without accepting its last outcome.
    pub fn begin(&mut self, token: std::num::NonZeroU64, key: WalkKey, outcome: HostOutcome) {
        self.wait = Some(Wait {
            token: token.get(),
            key,
            settled: None,
            seq_at_begin: outcome.seq,
            matched: None,
            blocked: false,
        });
    }

    fn outcome_matches(outcome: HostOutcome, wait: &Wait) -> bool {
        outcome.request_id != 0
            && outcome.request_id == wait.token
            && outcome.key == wait.key
            && outcome.seq != 0
            && outcome.seq != wait.seq_at_begin
    }

    pub fn poll(&mut self, token: u64, observation: &impl Observation) -> bool {
        let Some(wait) = self.wait.as_mut() else {
            return false;
        };
        if wait.token != token {
            return false;
        }
        if wait.settled.is_some() {
            return true;
        }
        if observation.cancelled() {
            wait.settled = Some(false);
            return true;
        }
        if observation.arrived(wait.key) {
            wait.settled = Some(true);
            return true;
        }
        // Re-read merged observations: a delta may omit an already posted
        // terminal, and a prior failed outcome remains sticky.
        if wait.matched.is_none() {
            let outcome = observation.outcome();
            if Self::outcome_matches(outcome, wait) {
                wait.matched = Some(!outcome.failed);
                wait.blocked = !outcome.failed && outcome.blocked;
            }
        }
        if let Some(value) = wait.matched {
            wait.settled = Some(value);
            return true;
        }
        false
    }

    pub fn value(&self, token: u64) -> bool {
        self.wait
            .as_ref()
            .is_some_and(|wait| wait.token == token && wait.settled == Some(true))
    }

    pub fn blocked(&self, token: u64) -> bool {
        self.wait
            .as_ref()
            .is_some_and(|wait| wait.token == token && wait.settled == Some(true) && wait.blocked)
    }
}
