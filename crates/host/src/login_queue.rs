//! Login FIFO: stay under Lost City's **production** login rate limits.
//!
//! Source of truth (local engine checkout, usually `$ENGINE_DIR`):
//! - `src/util/WorldConfig.ts` — `rateLimitAddressLogin: 30` (the engine
//!   rejects after incrementing to `>= 30`, so only 29 are admitted),
//!   `rateLimitDeviceLogin: 5` (`NODE_RATELIMIT_*` can override)
//! - `src/engine/World.ts` — `loginAddressAttempts` TTL **60 s**, refreshed
//!   on each attempt,
//!   `loginDeviceAttempts` TTL **15 s**; both counters run **only** when
//!   `node.production` is true. Local default is `production: false`, so
//!   a loopback engine does not apply these at all.
//!
//! The host still stays under the production numbers (so flipping
//! `NODE_PRODUCTION` does not 16 us). There is **no** inter-grant spacing
//! in the engine; 2.5 s was host-invented and is not a default.

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

/// Production `rateLimitDeviceLogin` is 5 in a 15 s TTL (`>= 5` rejects,
/// so 4 grants then wait the remaining TTL). Each grant refreshes the
/// host reservation count; cooldown is measured conservatively from the
/// latest `client.login` return while any unreturned grant prevents expiry.
const UID_GRANT_CAP: usize = 4;
const UID_COOLDOWN: Duration = Duration::from_secs(15);

/// Not-head poll. `wait_for_permit` sleeps this; do not multiply by queue
/// depth (that invented a 2.5 s × position stall the engine does not have).
const QUEUE_POLL: Duration = Duration::from_millis(20);

/// Snapshot of a queued uid's place: `position` is 1-based among `total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueuePos {
    pub position: u32,
    pub total: u32,
}

/// Login FIFO with focused-slot priority when that slot requests a permit.
///
/// FIFO identity is a slot-owner token, not a device UID: two configured
/// accounts may share the same UID and must still own two independently
/// pollable places. Rate accounting remains keyed by UID because that is the
/// server's device limit.
#[derive(Debug)]
pub struct LoginQueue {
    spacing: Duration,
    ip_cap: usize,
    ip_window: Duration,
    queue: VecDeque<QueueEntry>,
    preferred: Option<u64>,
    order_hints: HashMap<u64, u64>,
    next_hint: u64,
    last_grant: Option<Instant>,
    ip_count: usize,
    ip_pending: usize,
    ip_last: Option<Instant>,
    throttle_until: Option<Instant>,
    by_uid: HashMap<i32, UidState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueueEntry {
    owner: u64,
    uid: i32,
    order: u64,
}

fn uid_owner(uid: i32) -> u64 {
    u64::from(uid as u32)
}

#[derive(Debug)]
struct UidState {
    count: usize,
    pending: usize,
    last: Option<Instant>,
}

impl LoginQueue {
    /// Construct a queue with an explicit inter-grant spacing, address permit
    /// cap, and address idle TTL. `ip_cap` counts completed attempts plus
    /// pending reservations; a full `ip_window` starts at the latest login
    /// return only after no reservation remains in flight.
    pub fn new(spacing: Duration, ip_cap: usize, ip_window: Duration) -> Self {
        assert!(ip_cap > 0, "ip_cap must be positive");
        Self {
            spacing,
            ip_cap,
            ip_window,
            queue: VecDeque::new(),
            preferred: None,
            order_hints: HashMap::new(),
            next_hint: 0,
            last_grant: None,
            ip_count: 0,
            ip_pending: 0,
            ip_last: None,
            throttle_until: None,
            by_uid: HashMap::new(),
        }
    }

    /// Record control-thread operator order without creating membership.
    /// The owning slot thread consumes this hint when it reaches Queueing.
    pub fn hint_owner(&mut self, owner: u64) {
        if self.queue.iter().any(|entry| entry.owner == owner) {
            return;
        }
        self.order_hints.insert(owner, self.next_hint);
        self.next_hint = self.next_hint.wrapping_add(1);
    }

    /// Enter one slot owner into the FIFO exactly once at its Queueing
    /// transition. Focused priority is the only exception to hinted/FIFO
    /// order.
    pub fn enqueue_owner(&mut self, owner: u64, uid: i32) {
        if self.queue.iter().any(|entry| entry.owner == owner) {
            return;
        }
        let entry = QueueEntry {
            owner,
            uid,
            order: self.order_hints.remove(&owner).unwrap_or(u64::MAX),
        };
        if self.preferred == Some(owner) {
            self.queue.push_front(entry);
        } else if let Some(index) = self
            .queue
            .iter()
            .position(|queued| self.preferred != Some(queued.owner) && queued.order > entry.order)
        {
            self.queue.insert(index, entry);
        } else {
            self.queue.push_back(entry);
        }
    }

    /// Poll a slot-owner place already entered through
    /// [`Self::enqueue_owner`].
    pub fn poll_owner(&mut self, owner: u64, uid: i32, now: Instant) -> Permit {
        self.prune_uid(now);
        if self.queue.front().map(|entry| entry.owner) != Some(owner) {
            return Permit::Wait(QUEUE_POLL.max(self.spacing));
        }
        let front = self.queue.front().expect("owner is at the front");
        if self.preferred != Some(owner)
            && self.order_hints.values().any(|&hint| hint < front.order)
        {
            return Permit::Wait(QUEUE_POLL.max(self.spacing));
        }
        debug_assert_eq!(self.queue.front().map(|entry| entry.uid), Some(uid));
        match self.blocked_for(uid, now) {
            Some(wait) => Permit::Wait(wait),
            None => {
                self.grant(owner, uid, now);
                Permit::Grant
            }
        }
    }

    /// Where a slot owner sits in the queue. `position` is 1-based; a
    /// granted owner is popped and no longer present.
    pub fn status_owner(&self, owner: u64) -> Option<QueuePos> {
        let i = self.queue.iter().position(|entry| entry.owner == owner)?;
        Some(QueuePos {
            position: (i as u32) + 1,
            total: self.queue.len() as u32,
        })
    }

    /// Drop one slot owner's place. Other owners with the same device UID
    /// remain queued.
    pub fn leave_owner(&mut self, owner: u64) -> bool {
        self.order_hints.remove(&owner);
        let before = self.queue.len();
        self.queue.retain(|entry| entry.owner != owner);
        self.queue.len() != before
    }

    /// Give one slot owner focused priority without manufacturing membership.
    pub fn prefer_owner(&mut self, owner: u64) {
        self.set_preferred_owner(Some(owner));
    }

    /// Remember the focused slot owner for subsequent handshakes.
    pub fn set_preferred_owner(&mut self, owner: Option<u64>) {
        self.preferred = owner;
        if let Some(owner) = owner {
            if self.queue.iter().any(|entry| entry.owner == owner) {
                let entry = self
                    .queue
                    .iter()
                    .find(|entry| entry.owner == owner)
                    .copied()
                    .expect("queued owner exists");
                self.queue.retain(|entry| entry.owner != owner);
                self.queue.push_front(entry);
            }
        }
    }

    /// Backwards-compatible single-owner convenience for direct queue
    /// callers. Host slots use the explicit owner APIs above.
    pub fn enqueue(&mut self, uid: i32) {
        self.enqueue_owner(uid_owner(uid), uid);
    }

    /// Backwards-compatible direct queue poll.
    pub fn poll_permit(&mut self, uid: i32, now: Instant) -> Permit {
        self.poll_owner(uid_owner(uid), uid, now)
    }

    /// Convenience request for single-threaded callers and tests. Host slots
    /// use `enqueue_owner` at Queueing and then only `poll_owner`.
    pub fn request_permit(&mut self, uid: i32, now: Instant) -> Permit {
        self.enqueue(uid);
        self.poll_permit(uid, now)
    }

    /// Backwards-compatible direct queue position lookup.
    pub fn status(&self, uid: i32) -> Option<QueuePos> {
        self.status_owner(uid_owner(uid))
    }

    /// Backwards-compatible direct queue withdrawal.
    pub fn leave(&mut self, uid: i32) -> bool {
        self.leave_owner(uid_owner(uid))
    }

    /// Record completion of one granted `uid` login call. Success and error
    /// returns both acknowledge conservatively: the client may have sent the
    /// server attempt before either result. Duplicate-uid reservations are
    /// fungible counts, so one return consumes exactly one pending grant.
    pub fn acknowledge_login_return(&mut self, uid: i32, now: Instant) -> bool {
        let Some(state) = self.by_uid.get_mut(&uid) else {
            return false;
        };
        if state.pending == 0 {
            return false;
        }

        state.pending -= 1;
        state.last = Some(state.last.map_or(now, |last| last.max(now)));
        self.ip_pending = self
            .ip_pending
            .checked_sub(1)
            .expect("uid pending grant is also address pending");
        self.ip_last = Some(self.ip_last.map_or(now, |last| last.max(now)));
        true
    }

    /// Release one granted `uid` permit that never entered `client.login`.
    /// This removes its reservation without refreshing either server TTL.
    pub fn abandon_permit(&mut self, uid: i32) -> bool {
        let Some(state) = self.by_uid.get_mut(&uid) else {
            return false;
        };
        if state.pending == 0 {
            return false;
        }

        state.pending -= 1;
        state.count = state
            .count
            .checked_sub(1)
            .expect("pending uid grant is counted");
        self.ip_pending = self
            .ip_pending
            .checked_sub(1)
            .expect("uid pending grant is also address pending");
        self.ip_count = self
            .ip_count
            .checked_sub(1)
            .expect("pending address grant is counted");
        true
    }

    /// Backwards-compatible focused UID preference.
    pub fn prefer(&mut self, uid: i32) {
        self.prefer_owner(uid_owner(uid));
    }

    /// Backwards-compatible focused UID preference update.
    pub fn set_preferred(&mut self, uid: Option<i32>) {
        self.set_preferred_owner(uid.map(uid_owner));
    }

    /// Front-first copy of queued device UIDs (tests / panel diagnostics).
    pub fn queued_uids(&self) -> Vec<i32> {
        self.queue.iter().map(|entry| entry.uid).collect()
    }

    /// Pause all sibling attempts after the server reports an address/device
    /// login throttle. Repeated reports may extend, but never shorten, it.
    pub fn hold_for(&mut self, now: Instant, duration: Duration) {
        let deadline = now + duration;
        self.throttle_until = Some(
            self.throttle_until
                .map_or(deadline, |old| old.max(deadline)),
        );
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

        if let Some(until) = self.throttle_until {
            let left = until.saturating_duration_since(now);
            if left.is_zero() {
                self.throttle_until = None;
            } else {
                wait = Some(wait.map_or(left, |w| w.max(left)));
            }
        }
        if self.ip_pending == 0
            && self
                .ip_last
                .is_some_and(|last| now.saturating_duration_since(last) >= self.ip_window)
        {
            self.ip_count = 0;
            self.ip_last = None;
        }
        if self.ip_count >= self.ip_cap {
            let until = if self.ip_pending > 0 {
                QUEUE_POLL
            } else {
                let latest = self
                    .ip_last
                    .expect("address ack time exists when cap is reached");
                (latest + self.ip_window).saturating_duration_since(now)
            };
            wait = Some(wait.map_or(until, |w| w.max(until)));
        }

        let state = self.by_uid.entry(uid).or_insert(UidState {
            count: 0,
            pending: 0,
            last: None,
        });
        if state.count >= UID_GRANT_CAP {
            let until = if state.pending > 0 {
                Some(QUEUE_POLL)
            } else {
                state
                    .last
                    .and_then(|last| UID_COOLDOWN.checked_sub(now.saturating_duration_since(last)))
                    .filter(|left| !left.is_zero())
            };
            if let Some(until) = until {
                wait = Some(wait.map_or(until, |w| w.max(until)));
            }
        }
        wait
    }

    fn grant(&mut self, owner: u64, uid: i32, now: Instant) {
        debug_assert_eq!(
            self.queue.front().map(|entry| (entry.owner, entry.uid)),
            Some((owner, uid))
        );
        self.queue.pop_front();
        self.last_grant = Some(now);
        self.ip_count += 1;
        self.ip_pending += 1;

        let state = self.by_uid.entry(uid).or_insert(UidState {
            count: 0,
            pending: 0,
            last: None,
        });
        if state.count >= UID_GRANT_CAP {
            // Cooldown has elapsed; restart the uid window.
            debug_assert_eq!(state.pending, 0);
            state.count = 0;
            state.last = None;
        }
        state.count += 1;
        state.pending += 1;
    }

    /// Drop every uid accounting row after 15 s idle when it has no pending
    /// permit. Queue identity is stored separately, so an expired waiting uid
    /// keeps its FIFO place while its partial counter resets.
    fn prune_uid(&mut self, now: Instant) {
        self.by_uid.retain(|_, state| {
            state.pending > 0
                || state
                    .last
                    .is_some_and(|last| now.saturating_duration_since(last) < UID_COOLDOWN)
        });
    }
    #[cfg(test)]
    fn tracks(&self, uid: i32) -> bool {
        self.by_uid.contains_key(&uid)
    }
}

impl Default for LoginQueue {
    fn default() -> Self {
        // spacing 0: engine has no inter-grant delay. Its configured address
        // threshold 30 rejects the 30th attempt, hence 29 grants / 60 s idle.
        Self::new(Duration::ZERO, 29, Duration::from_secs(60))
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
    fn preferred_uid_retains_priority_after_first_login() {
        let now = Instant::now();
        let mut q = LoginQueue::new(Duration::from_secs(1), 30, Duration::from_secs(60));
        q.prefer(1);
        assert_eq!(q.request_permit(1, now), Permit::Grant);
        assert!(q.queued_uids().is_empty());
        assert!(matches!(q.request_permit(2, now), Permit::Wait(_)));
        assert!(matches!(q.request_permit(1, now), Permit::Wait(_)));
        assert_eq!(q.queued_uids(), vec![1, 2]);
        assert_eq!(
            q.request_permit(1, now + Duration::from_secs(1)),
            Permit::Grant
        );
        assert_eq!(
            q.request_permit(2, now + Duration::from_secs(2)),
            Permit::Grant
        );
    }

    #[test]
    fn duplicate_uids_keep_independent_fifo_owners() {
        let now = Instant::now();
        let mut q = LoginQueue::default();
        q.enqueue_owner(11, 7);
        q.enqueue_owner(22, 7);
        assert_eq!(q.queued_uids(), vec![7, 7]);

        assert_eq!(q.poll_owner(11, 7, now), Permit::Grant);
        assert!(q.acknowledge_login_return(7, now));
        assert_eq!(
            q.status_owner(22),
            Some(QueuePos {
                position: 1,
                total: 1,
            })
        );
        assert_eq!(q.poll_owner(22, 7, now), Permit::Grant);
        assert!(q.acknowledge_login_return(7, now));
        assert!(q.queued_uids().is_empty());
    }

    #[test]
    fn operator_hint_waits_for_earlier_owner_without_creating_membership() {
        let now = Instant::now();
        let mut q = LoginQueue::default();
        q.hint_owner(11);
        q.hint_owner(22);
        assert!(q.queued_uids().is_empty(), "hints are not membership");

        q.enqueue_owner(22, 2);
        assert_eq!(q.poll_owner(22, 2, now), Permit::Wait(QUEUE_POLL));
        q.enqueue_owner(11, 1);
        assert_eq!(q.queued_uids(), vec![1, 2]);
        assert_eq!(q.poll_owner(11, 1, now), Permit::Grant);
        assert!(q.acknowledge_login_return(1, now));
        assert_eq!(q.poll_owner(22, 2, now), Permit::Grant);
        assert!(q.acknowledge_login_return(2, now));
    }

    #[test]
    fn focus_changes_do_not_reserve_online_slots_or_bypass_limits() {
        let now = Instant::now();
        let mut q = LoginQueue::new(Duration::from_secs(1), 1, Duration::from_secs(60));
        q.set_preferred(Some(1));
        assert!(q.queued_uids().is_empty());
        assert_eq!(q.request_permit(2, now), Permit::Grant);
        assert!(matches!(q.request_permit(3, now), Permit::Wait(_)));
        assert!(matches!(q.request_permit(1, now), Permit::Wait(_)));
        q.set_preferred(Some(3));
        assert_eq!(q.queued_uids(), vec![3, 1]);
        assert!(matches!(
            q.request_permit(3, now + Duration::from_secs(1)),
            Permit::Wait(_)
        ));
        q.leave(3);
        q.leave(1);
        q.set_preferred(None);
        assert!(matches!(q.request_permit(2, now), Permit::Wait(_)));
        assert!(matches!(q.request_permit(3, now), Permit::Wait(_)));
        assert_eq!(q.queued_uids(), vec![2, 3]);
    }

    #[test]
    fn prefer_moves_uid_to_the_front() {
        // Long spacing so only the first grant lands; the rest stay queued.
        let mut q = LoginQueue::new(Duration::from_secs(60), 30, Duration::from_secs(60));
        let now = Instant::now();
        assert!(matches!(q.request_permit(2, now), Permit::Grant));
        assert!(matches!(q.request_permit(3, now), Permit::Wait(_)));
        assert!(matches!(q.request_permit(1, now), Permit::Wait(_)));
        q.prefer(1);
        assert_eq!(q.queue.front().map(|entry| entry.uid), Some(1));
        assert_eq!(q.status(1).unwrap().position, 1);
    }

    #[test]
    fn default_admits_29_and_makes_the_30th_wait() {
        let base = Instant::now();
        let mut q = LoginQueue::default();
        let mut grants = Vec::new();
        for i in 0..50 {
            if let Permit::Grant = q.request_permit(i, base) {
                grants.push(base);
            }
        }
        // The engine rejects attempt >= 30, so only 29 may enter while its
        // address key is live. Pending reservations count toward all 29, and
        // there is still no invented inter-grant delay.
        assert_eq!(grants.len(), 29);
        for i in 0..29 {
            assert!(q.acknowledge_login_return(i, base));
        }
        assert!(matches!(q.request_permit(29, base), Permit::Wait(_)));

        let now = base + Duration::from_secs(60);
        for i in 29..50 {
            assert!(matches!(q.request_permit(i, now), Permit::Grant));
            grants.push(now);
            assert!(q.acknowledge_login_return(i, now));
        }
        assert_eq!(grants.len(), 50);
        assert!(max_grants_in_60s(&grants) <= 29);
    }

    #[test]
    fn ip_idle_ttl_cap_holds_when_spacing_is_small() {
        let base = Instant::now();
        let mut q = LoginQueue::new(Duration::from_millis(1), 29, Duration::from_secs(60));
        let mut now = base;
        let mut grants = Vec::new();
        for i in 0..50 {
            if let Permit::Grant = q.request_permit(i, now) {
                grants.push(now);
                assert!(q.acknowledge_login_return(i, now));
            }
            now += Duration::from_millis(1);
        }
        // Cap binds: the 30th grant must wait out the 60 s idle TTL.
        assert_eq!(grants.len(), 29);

        now = base + Duration::from_secs(61);
        for i in 29..50 {
            assert!(matches!(q.request_permit(i, now), Permit::Grant));
            grants.push(now);
            assert!(q.acknowledge_login_return(i, now));
            now += Duration::from_millis(1);
        }
        assert_eq!(grants.len(), 50);
        assert!(max_grants_in_60s(&grants) <= 29);
    }

    #[test]
    fn address_ttl_renews_from_latest_ack_then_resets_after_idle() {
        let base = Instant::now();
        let mut q = LoginQueue::new(Duration::ZERO, 2, Duration::from_secs(60));
        assert_eq!(q.request_permit(1, base), Permit::Grant);
        assert!(q.acknowledge_login_return(1, base));
        assert_eq!(
            q.request_permit(2, base + Duration::from_secs(30)),
            Permit::Grant
        );
        assert!(q.acknowledge_login_return(2, base + Duration::from_secs(30)));

        match q.request_permit(3, base + Duration::from_secs(60)) {
            Permit::Wait(wait) => assert_eq!(wait, Duration::from_secs(30)),
            Permit::Grant => panic!("oldest + TTL must not release the refreshed address key"),
        }
        assert!(matches!(
            q.request_permit(4, base + Duration::from_secs(60)),
            Permit::Wait(_)
        ));
        assert_eq!(q.queued_uids(), vec![3, 4], "blocked callers remain FIFO");

        assert_eq!(
            q.request_permit(3, base + Duration::from_secs(90)),
            Permit::Grant
        );
        assert_eq!(
            q.request_permit(4, base + Duration::from_secs(90)),
            Permit::Grant
        );
    }

    #[test]
    fn unacked_reservation_does_not_expire_without_a_server_attempt_clock() {
        let base = Instant::now();
        let mut q = LoginQueue::new(Duration::ZERO, 1, Duration::from_secs(60));
        assert_eq!(q.request_permit(1, base), Permit::Grant);

        assert!(
            matches!(
                q.request_permit(2, base + Duration::from_secs(120)),
                Permit::Wait(_)
            ),
            "a delayed in-flight attempt can still refresh the server TTL"
        );
    }

    #[test]
    fn reordered_login_returns_keep_the_latest_monotonic_ack_clock() {
        let base = Instant::now();
        let mut q = LoginQueue::new(Duration::ZERO, 2, Duration::from_secs(60));
        assert_eq!(q.request_permit(1, base), Permit::Grant);
        assert_eq!(q.request_permit(2, base), Permit::Grant);

        assert!(q.acknowledge_login_return(2, base + Duration::from_secs(120)));
        assert!(q.acknowledge_login_return(1, base + Duration::from_secs(61)));
        match q.request_permit(3, base + Duration::from_secs(121)) {
            Permit::Wait(wait) => assert_eq!(wait, Duration::from_secs(59)),
            Permit::Grant => panic!("an older completion must not move the ack clock backward"),
        }
        assert_eq!(
            q.request_permit(3, base + Duration::from_secs(180)),
            Permit::Grant
        );
    }

    #[test]
    fn one_old_ack_cannot_reset_count_while_another_attempt_is_pending() {
        let base = Instant::now();
        let mut q = LoginQueue::new(Duration::ZERO, 2, Duration::from_secs(60));
        assert_eq!(q.request_permit(1, base), Permit::Grant);
        assert_eq!(q.request_permit(2, base), Permit::Grant);
        assert!(q.acknowledge_login_return(1, base));

        assert!(
            matches!(
                q.request_permit(3, base + Duration::from_secs(120)),
                Permit::Wait(_)
            ),
            "the pending attempt can still refresh all address accounting"
        );
        assert!(q.acknowledge_login_return(2, base + Duration::from_secs(130)));
        assert!(matches!(
            q.request_permit(3, base + Duration::from_secs(189)),
            Permit::Wait(_)
        ));
        assert_eq!(
            q.request_permit(3, base + Duration::from_secs(190)),
            Permit::Grant
        );
    }

    #[test]
    fn abandoning_unused_permit_releases_reservation_without_starting_ttl() {
        let base = Instant::now();
        let mut q = LoginQueue::new(Duration::ZERO, 1, Duration::from_secs(60));
        assert_eq!(q.request_permit(1, base), Permit::Grant);
        assert!(q.abandon_permit(1));
        assert!(!q.abandon_permit(1), "cleanup is exactly once");
        assert_eq!(q.request_permit(2, base), Permit::Grant);
    }

    #[test]
    fn duplicate_uid_reservations_are_cleaned_up_one_at_a_time() {
        let base = Instant::now();
        let mut q = LoginQueue::new(Duration::ZERO, 2, Duration::from_secs(60));
        assert_eq!(q.request_permit(7, base), Permit::Grant);
        assert_eq!(q.request_permit(7, base), Permit::Grant);

        assert!(q.acknowledge_login_return(7, base + Duration::from_secs(10)));
        assert!(q.abandon_permit(7));
        assert!(!q.abandon_permit(7), "both pending grants were resolved");
        assert_eq!(
            q.request_permit(8, base + Duration::from_secs(10)),
            Permit::Grant,
            "abandoning one duplicate must preserve only the acknowledged attempt"
        );
    }

    #[test]
    fn pending_uid_reservations_do_not_expire_before_login_returns() {
        let base = Instant::now();
        let mut q = LoginQueue::new(Duration::ZERO, 10, Duration::from_secs(60));
        for _ in 0..UID_GRANT_CAP {
            assert_eq!(q.request_permit(7, base), Permit::Grant);
        }
        assert!(matches!(
            q.request_permit(7, base + Duration::from_secs(120)),
            Permit::Wait(_)
        ));

        for _ in 0..UID_GRANT_CAP {
            assert!(q.abandon_permit(7));
        }
        assert_eq!(
            q.request_permit(7, base + Duration::from_secs(120)),
            Permit::Grant
        );
    }

    #[test]
    fn same_uid_fifth_request_waits_remaining_15s_ttl() {
        let base = Instant::now();
        let mut q = LoginQueue::default();
        for _ in 0..4 {
            assert!(matches!(q.request_permit(7, base), Permit::Grant));
            assert!(q.acknowledge_login_return(7, base));
        }
        match q.request_permit(7, base) {
            Permit::Wait(d) => assert_eq!(d, Duration::from_secs(15), "fifth wait {d:?}"),
            Permit::Grant => panic!("fifth same-uid request must wait"),
        }
        let later = base + Duration::from_secs(10);
        match q.request_permit(7, later) {
            Permit::Wait(d) => assert_eq!(d, Duration::from_secs(5), "remaining {d:?}"),
            Permit::Grant => panic!("still inside the 15 s device TTL"),
        }
        assert!(matches!(
            q.request_permit(7, base + Duration::from_secs(15)),
            Permit::Grant
        ));
    }

    /// A waiting uid keeps its FIFO place, so its cooldown row survives
    /// pruning. Its retry lands a little after the 15 s TTL (a real sleep
    /// overshoots), which must grant, not panic.
    #[test]
    fn queued_uid_retrying_after_ttl_overshoot_grants() {
        let base = Instant::now();
        let mut q = LoginQueue::default();
        for _ in 0..4 {
            assert!(matches!(q.request_permit(7, base), Permit::Grant));
            assert!(q.acknowledge_login_return(7, base));
        }
        assert!(matches!(q.request_permit(7, base), Permit::Wait(_)));
        let retry = base + UID_COOLDOWN + Duration::from_millis(3);
        assert!(matches!(q.request_permit(7, retry), Permit::Grant));
    }

    #[test]
    fn not_head_polls_20ms_not_depth_times_spacing() {
        let mut q = LoginQueue::default();
        let now = Instant::now();
        for i in 0..29 {
            assert!(matches!(q.request_permit(i, now), Permit::Grant));
            assert!(q.acknowledge_login_return(i, now));
        }
        match q.request_permit(29, now) {
            Permit::Wait(d) => assert!(
                d >= Duration::from_secs(59),
                "head waits out the address idle TTL, got {d:?}"
            ),
            other => panic!("head should wait the 60 s address idle TTL, got {other:?}"),
        }
        match q.request_permit(30, now) {
            Permit::Wait(d) => assert_eq!(
                d,
                Duration::from_millis(20),
                "not-head must poll, not sleep 2.5s × position"
            ),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn by_uid_prunes_elapsed_unqueued() {
        let mut q = LoginQueue::default();
        let now = Instant::now();
        for _ in 0..4 {
            assert!(matches!(q.request_permit(7, now), Permit::Grant));
            assert!(q.acknowledge_login_return(7, now));
        }
        assert!(q.tracks(7));
        let now = now + UID_COOLDOWN;
        assert!(matches!(q.request_permit(8, now), Permit::Grant));
        assert!(
            !q.tracks(7),
            "cooldown-elapsed uid 7 is not queued and must drop"
        );
        assert!(q.tracks(8));
    }

    #[test]
    fn status_is_k_of_n_and_grant_clears() {
        let mut q = LoginQueue::new(Duration::from_secs(60), 30, Duration::from_secs(60));
        let now = Instant::now();
        assert!(q.status(1).is_none());
        assert!(matches!(q.request_permit(1, now), Permit::Grant));
        assert!(q.status(1).is_none());
        assert!(matches!(q.request_permit(2, now), Permit::Wait(_)));
        assert!(matches!(q.request_permit(3, now), Permit::Wait(_)));
        let s2 = q.status(2).unwrap();
        let s3 = q.status(3).unwrap();
        assert_eq!((s2.position, s2.total), (1, 2));
        assert_eq!((s3.position, s3.total), (2, 2));
        q.leave(2);
        let s3 = q.status(3).unwrap();
        assert_eq!((s3.position, s3.total), (1, 1));
        assert!(q.status(2).is_none());
    }

    #[test]
    fn partial_uid_count_expires_after_idle() {
        let base = Instant::now();
        let mut q = LoginQueue::default();
        for _ in 0..3 {
            assert_eq!(q.request_permit(7, base), Permit::Grant);
            assert!(q.acknowledge_login_return(7, base));
        }
        let fresh = base + Duration::from_secs(100);
        assert_eq!(q.request_permit(7, fresh), Permit::Grant);
        assert!(q.acknowledge_login_return(7, fresh));
        assert_eq!(q.request_permit(7, fresh), Permit::Grant);
    }

    #[test]
    fn response_throttle_holds_sibling_attempts() {
        let base = Instant::now();
        let mut q = LoginQueue::default();
        q.hold_for(base, Duration::from_secs(20));
        assert!(
            matches!(q.request_permit(8, base), Permit::Wait(wait) if wait == Duration::from_secs(20))
        );
        assert_eq!(
            q.request_permit(8, base + Duration::from_secs(20)),
            Permit::Grant
        );
    }

    #[test]
    fn two_uids_enqueue_in_order() {
        let mut q = LoginQueue::new(Duration::from_secs(60), 30, Duration::from_secs(60));
        let now = Instant::now();
        let _ = q.request_permit(10, now); // grant
        let _ = q.request_permit(11, now);
        let _ = q.request_permit(12, now);
        assert_eq!(q.status(11).unwrap().position, 1);
        assert_eq!(q.status(12).unwrap().position, 2);
        assert_eq!(q.status(12).unwrap().total, 2);
    }

    #[test]
    fn enqueue_order_wins_request_lock_race() {
        let base = Instant::now();
        let mut q = LoginQueue::default();
        q.enqueue(10);
        q.enqueue(11);
        assert!(matches!(q.poll_permit(11, base), Permit::Wait(_)));
        assert_eq!(q.poll_permit(10, base), Permit::Grant);
        assert_eq!(q.poll_permit(11, base), Permit::Grant);
    }

    #[test]
    fn preferred_owner_that_is_not_waiting_cannot_block_followers() {
        let mut q = LoginQueue::new(Duration::from_secs(60), 30, Duration::from_secs(60));
        let base = Instant::now();
        q.prefer(7);
        assert!(
            q.queued_uids().is_empty(),
            "preference alone is not waiting membership"
        );
        assert_eq!(
            q.request_permit(8, base),
            Permit::Grant,
            "a terminal preferred owner cannot become a phantom head"
        );
        assert!(q.status(7).is_none());
    }

    #[test]
    #[should_panic(expected = "ip_cap must be positive")]
    fn zero_cap_is_rejected_at_construction() {
        let _ = LoginQueue::new(Duration::ZERO, 0, Duration::from_secs(60));
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
