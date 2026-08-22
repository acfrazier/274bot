//! Login FIFO: serializes login handshakes under the 274 server's default
//! throttle (rs2b0t coordinator math).

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Permit outcome of [`LoginQueue::request_permit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permit {
    /// Login handshake may start now.
    Grant,
    /// The queue throttled this request; retry after this long.
    Wait(Duration),
}

/// Server per-device login limit is 5 per 15 s; stay under it with 4 grants
/// then a 16 s pause measured from the latest grant.
const UID_GRANT_CAP: usize = 4;
const UID_COOLDOWN: Duration = Duration::from_secs(16);

/// FIFO queue of login permit requests.
#[derive(Debug)]
pub struct LoginQueue {
    spacing: Duration,
    ip_cap: usize,
    ip_window: Duration,
    queue: VecDeque<i32>,
    last_grant: Option<Instant>,
    window: VecDeque<Instant>,
    by_uid: HashMap<i32, UidState>,
}

#[derive(Debug)]
struct UidState {
    count: usize,
    last: Option<Instant>,
}

impl LoginQueue {
    pub fn new(spacing: Duration, ip_cap: usize, ip_window: Duration) -> Self {
        Self {
            spacing,
            ip_cap,
            ip_window,
            queue: VecDeque::new(),
            last_grant: None,
            window: VecDeque::new(),
            by_uid: HashMap::new(),
        }
    }

    /// FIFO permit request: only the head of the queue can be granted, and
    /// only when spacing, the per-IP window, and the per-uid rule all pass.
    /// A blocked caller retries after the returned wait.
    pub fn request_permit(&mut self, uid: i32, now: Instant) -> Permit {
        if !self.queue.contains(&uid) {
            self.queue.push_back(uid);
        }
        if self.queue.front() != Some(&uid) {
            let ahead = self.queue.iter().take_while(|&&u| u != uid).count() as u32 + 1;
            return Permit::Wait(self.spacing.saturating_mul(ahead));
        }
        match self.blocked_for(uid, now) {
            Some(wait) => Permit::Wait(wait),
            None => {
                self.grant(uid, now);
                Permit::Grant
            }
        }
    }

    /// Longest unmet constraint for granting `uid` at `now`.
    fn blocked_for(&mut self, uid: i32, now: Instant) -> Option<Duration> {
        let mut wait = None;

        if let Some(last) = self.last_grant {
            let since = now.saturating_duration_since(last);
            if since < self.spacing {
                wait = Some(self.spacing - since);
            }
        }

        while self
            .window
            .front()
            .is_some_and(|&t| now.saturating_duration_since(t) >= self.ip_window)
        {
            self.window.pop_front();
        }
        if self.window.len() >= self.ip_cap {
            let oldest = *self.window.front().expect("window nonempty when over cap");
            let until = (oldest + self.ip_window).saturating_duration_since(now);
            wait = Some(wait.map_or(until, |w| w.max(until)));
        }

        let state = self
            .by_uid
            .entry(uid)
            .or_insert(UidState { count: 0, last: None });
        if state.count >= UID_GRANT_CAP {
            if let Some(last) = state.last {
                if now.saturating_duration_since(last) < UID_COOLDOWN {
                    wait = Some(wait.map_or(UID_COOLDOWN, |w| w.max(UID_COOLDOWN)));
                }
            }
        }
        wait
    }

    fn grant(&mut self, uid: i32, now: Instant) {
        debug_assert_eq!(self.queue.front(), Some(&uid));
        self.queue.pop_front();
        self.last_grant = Some(now);
        self.window.push_back(now);

        let state = self
            .by_uid
            .entry(uid)
            .or_insert(UidState { count: 0, last: None });
        if state.count >= UID_GRANT_CAP {
            // Cooldown has elapsed; restart the uid window.
            state.count = 0;
        }
        state.count += 1;
        state.last = Some(now);
    }
}

impl Default for LoginQueue {
    fn default() -> Self {
        Self::new(Duration::from_millis(2500), 30, Duration::from_secs(60))
    }
}

/// Escalating delay after a response-16 (world full) rejection.
#[derive(Debug, Default)]
pub struct LoginBackoff {
    hits: u32,
}

impl LoginBackoff {
    pub fn new() -> Self {
        Self::default()
    }

    /// Delay for the next retry: 20 s + 45 s per prior hit.
    pub fn delay(&mut self) -> Duration {
        let delay = Duration::from_secs(20) + Duration::from_secs(45 * u64::from(self.hits));
        self.hits += 1;
        delay
    }

    pub fn reset(&mut self) {
        self.hits = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPACING: Duration = Duration::from_millis(2500);

    fn max_grants_in_60s(grant_times: &[Instant]) -> usize {
        grant_times
            .iter()
            .map(|&start| {
                grant_times
                    .iter()
                    .filter(|&&t| {
                        t >= start && t.saturating_duration_since(start) < Duration::from_secs(60)
                    })
                    .count()
            })
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn fifty_requests_never_exceed_30_grants_in_any_60s() {
        let base = Instant::now();
        let mut q = LoginQueue::default();
        let mut grants = Vec::new();
        for i in 0..50 {
            if let Permit::Grant = q.request_permit(i, base) {
                grants.push(base);
            }
        }
        // Only the FIFO head may grant; the rest stay queued.
        assert_eq!(grants.len(), 1);
        assert!(matches!(q.request_permit(25, base), Permit::Wait(_)));

        let mut now = base + SPACING;
        for i in 1..50 {
            assert!(matches!(q.request_permit(i, now), Permit::Grant));
            grants.push(now);
            now += SPACING;
        }
        assert_eq!(grants.len(), 50);
        assert!(max_grants_in_60s(&grants) <= 30);
    }

    #[test]
    fn ip_window_cap_holds_when_spacing_is_small() {
        let base = Instant::now();
        let mut q = LoginQueue::new(Duration::from_millis(1), 30, Duration::from_secs(60));
        let mut now = base;
        let mut grants = Vec::new();
        for i in 0..50 {
            if let Permit::Grant = q.request_permit(i, now) {
                grants.push(now);
            }
            now += Duration::from_millis(1);
        }
        // Cap binds: the 31st grant must wait out the 60 s window.
        assert_eq!(grants.len(), 30);

        now = base + Duration::from_secs(61);
        for i in 30..50 {
            assert!(matches!(q.request_permit(i, now), Permit::Grant));
            grants.push(now);
            now += Duration::from_millis(1);
        }
        assert_eq!(grants.len(), 50);
        assert!(max_grants_in_60s(&grants) <= 30);
    }

    #[test]
    fn same_uid_fifth_request_waits_at_least_16s() {
        let base = Instant::now();
        let mut q = LoginQueue::default();
        let mut now = base;
        for _ in 0..4 {
            assert!(matches!(q.request_permit(7, now), Permit::Grant));
            now += SPACING;
        }
        match q.request_permit(7, now) {
            Permit::Wait(d) => assert!(d >= Duration::from_secs(16), "fifth wait {d:?} < 16s"),
            Permit::Grant => panic!("fifth same-uid request must wait"),
        }
        // After the cooldown the uid may login again.
        now += Duration::from_secs(16);
        assert!(matches!(q.request_permit(7, now), Permit::Grant));
    }

    #[test]
    fn grants_are_spaced_at_least_2_5s_apart() {
        let base = Instant::now();
        let mut q = LoginQueue::default();
        let mut now = base;
        let mut prev = None;
        for i in 0..10 {
            assert!(matches!(q.request_permit(i, now), Permit::Grant));
            if let Some(last) = prev {
                assert!(now.saturating_duration_since(last) >= SPACING);
            }
            prev = Some(now);
            now += SPACING;
        }
    }

    #[test]
    fn login_backoff_escalates() {
        let mut b = LoginBackoff::new();
        assert_eq!(b.delay(), Duration::from_secs(20));
        assert_eq!(b.delay(), Duration::from_secs(65));
        assert_eq!(b.delay(), Duration::from_secs(110));
        b.reset();
        assert_eq!(b.delay(), Duration::from_secs(20));
    }
}
