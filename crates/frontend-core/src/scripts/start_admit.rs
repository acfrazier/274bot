//! Paced Start-all admission: the login FIFO's shape (enqueue, poll/grant,
//! leave, k-of-n status) applied to script Starts so a bulk Start cannot
//! dispatch every isolate in one operator frame.

use std::collections::VecDeque;

use crate::views::QueuePlace;

/// One Start-all (or marked bulk) Start may be dispatched per operator
/// frame ([`super::Scripts::poll`], and the click that enqueues).
///
/// Measured on this tree with a throwaway release timing of the
/// frontend-core fixture (looping JS card, `start_sel` handoff, no worker
/// threads): N=10 unpaced Start all dispatched every member in the same
/// click (~1–3 ms per Start of UI-thread `ensure_js` / bag merge /
/// sibling resolve / `script_start` handoff). Isolate V8 setup then races
/// on worker threads, so N=50 in one click is a start storm. One grant per
/// frame bounds the click-frame hitch to a single Start's cost and spreads
/// isolate construction across subsequent frames.
pub const START_ADMIT_PER_FRAME: usize = 1;

/// Same outcomes as [`host::login_queue::Permit`]: the head is granted now
/// or the caller polls again later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StartPermit {
    Grant,
    Wait,
}

#[derive(Debug, Clone)]
pub(super) struct QueuedStart {
    pub profile: String,
    pub sel: script::ScriptSel,
    /// When set, a grant is refused if the profile's saved assignment no
    /// longer names this selection (Start all / assignment-backed marked
    /// Start). Explicit-card marked Starts leave this `None` (pinned).
    pub assigned: Option<script::ScriptSel>,
    /// Logout latch at enqueue. A later latch cancels so an operator
    /// Logout while queued does not start stale work.
    pub latched: bool,
    /// Whether the slot had an arm at enqueue. Losing it is a disconnect.
    pub had_arm: bool,
}

#[derive(Debug, Default)]
pub(super) struct StartAdmit {
    queue: VecDeque<QueuedStart>,
    /// Restarts whose slot is still stopping: they join the queue (and its
    /// pacing) only once the slot is idle, so a restart never dispatches a
    /// Start onto a script that is still winding down.
    awaiting_stop: Vec<QueuedStart>,
    publish: bool,
}

impl StartAdmit {
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty() && self.awaiting_stop.is_empty()
    }

    pub fn contains(&self, profile: &str) -> bool {
        self.queue.iter().any(|entry| entry.profile == profile)
            || self
                .awaiting_stop
                .iter()
                .any(|entry| entry.profile == profile)
    }

    pub fn enqueue(&mut self, entry: QueuedStart) {
        if self.contains(&entry.profile) {
            return;
        }
        self.queue.push_back(entry);
        self.publish = true;
    }

    /// Hold `entry` until its slot has stopped; see [`Self::release_stopped`].
    pub fn hold_until_stopped(&mut self, entry: QueuedStart) {
        if self.contains(&entry.profile) {
            return;
        }
        self.awaiting_stop.push(entry);
    }

    /// Move every held restart whose slot `stopped` reports onto the paced
    /// queue, in the order they were held.
    pub fn release_stopped(&mut self, mut stopped: impl FnMut(&str) -> bool) {
        if self.awaiting_stop.is_empty() {
            return;
        }
        let (ready, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.awaiting_stop)
            .into_iter()
            .partition(|entry| stopped(&entry.profile));
        self.awaiting_stop = waiting;
        if !ready.is_empty() {
            self.queue.extend(ready);
            self.publish = true;
        }
    }

    pub fn poll(&self) -> StartPermit {
        if self.queue.is_empty() {
            StartPermit::Wait
        } else {
            StartPermit::Grant
        }
    }

    pub fn take_head(&mut self) -> Option<QueuedStart> {
        let entry = self.queue.pop_front()?;
        self.publish = true;
        Some(entry)
    }

    pub fn leave(&mut self, profile: &str) -> Option<QueuedStart> {
        if let Some(index) = self
            .awaiting_stop
            .iter()
            .position(|entry| entry.profile == profile)
        {
            return Some(self.awaiting_stop.remove(index));
        }
        let index = self
            .queue
            .iter()
            .position(|entry| entry.profile == profile)?;
        self.publish = true;
        self.queue.remove(index)
    }

    pub fn drain(&mut self) -> Vec<QueuedStart> {
        if self.is_empty() {
            return Vec::new();
        }
        self.publish = true;
        let mut all: Vec<QueuedStart> = self.queue.drain(..).collect();
        all.append(&mut self.awaiting_stop);
        all
    }

    pub fn status(&self, profile: &str) -> Option<QueuePlace> {
        let index = self
            .queue
            .iter()
            .position(|entry| entry.profile == profile)?;
        Some(QueuePlace {
            position: (index as u32) + 1,
            total: self.queue.len() as u32,
        })
    }

    /// Whether the fleet overlay is stale. Consuming the flag is the idle
    /// path: false means this poll must not walk rows.
    pub fn take_publish(&mut self) -> bool {
        std::mem::replace(&mut self.publish, false)
    }
}
