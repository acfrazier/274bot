//! Login FIFO: one login handshake at a time, paced, and under Lost City's
//! **production** login rate limits.
//!
//! Source of truth (local engine checkout, usually `$ENGINE_DIR`):
//! - `src/util/WorldConfig.ts:108-109` — `rateLimitAddressLogin: 30` (the
//!   engine rejects after incrementing to `>= 30`, so only 29 are admitted),
//!   `rateLimitDeviceLogin: 5` (`NODE_RATELIMIT_*` can override)
//! - `src/engine/World.ts:176-177` — `loginAddressAttempts` TTL **60 s**,
//!   refreshed on each attempt (`:2134-2144`), `loginDeviceAttempts` TTL
//!   **15 s** (`:2202-2212`); both counters run **only** when
//!   `node.production` is true. Local default is `production: false`, so a
//!   loopback engine does not apply these at all.
//! - `src/engine/World.ts:120,384,809-957` — the world admits new players
//!   once per 600 ms tick (`processLogins`, which sends response 2). Logins
//!   started closer together than that only queue behind each other in the
//!   same tick's batch.
//!
//! The host stays under the production numbers (so flipping
//! `NODE_PRODUCTION` does not 16 us), and it does not burst: at most one
//! handshake is in flight, and starts are at least [`LOGIN_START_SPACING`]
//! apart. The engine has no spacing rule of its own; the spacing is this
//! host's "don't hammer the server" policy, sized from the tick above.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Permit outcome of [`LoginQueue::poll_owner`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permit {
    /// Login handshake may start now.
    Grant,
    /// The queue throttled this request; retry after this long.
    Wait(Duration),
}

/// Why the queue head may not start its login yet. The deadline is a
/// monotonic instant on this host; `None` means it waits for the login
/// handshake that is in flight to return first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmitBlocker {
    /// Another bot's login handshake has not returned yet.
    Handshake,
    /// The minimum gap between two login starts.
    Spacing { until: Instant },
    /// The address's recent login attempts reached the server's limit; the
    /// next attempt waits for the server's idle period after the latest one.
    AddressBudget {
        attempts: u32,
        until: Option<Instant>,
    },
    /// This device id reached the server's per-device attempt limit.
    DeviceBudget { until: Option<Instant> },
    /// The server refused an attempt as too many (response 16).
    ServerHold { until: Instant },
}

impl AdmitBlocker {
    /// When this blocker lifts, if known.
    pub fn until(self) -> Option<Instant> {
        match self {
            Self::Handshake => None,
            Self::Spacing { until } | Self::ServerHold { until } => Some(until),
            Self::AddressBudget { until, .. } | Self::DeviceBudget { until } => until,
        }
    }
}

/// One server tick (`World.TICKRATE`, `World.ts:120`).
const SERVER_TICK: Duration = Duration::from_millis(600);

/// Default minimum gap between two login starts: two server ticks, so two
/// of our logins never land in the same tick's admission batch even with a
/// tick of timer or network jitter.
pub const LOGIN_START_SPACING: Duration = Duration::from_millis(2 * SERVER_TICK.as_millis() as u64);

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
    max_in_flight: usize,
    ip_cap: usize,
    ip_window: Duration,
    queue: VecDeque<QueueEntry>,
    preferred: Option<u64>,
    last_grant: Option<Instant>,
    /// The start before `last_grant`, restored when that grant is abandoned.
    previous_grant: Option<Instant>,
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
}

#[derive(Debug)]
struct UidState {
    count: usize,
    pending: usize,
    last: Option<Instant>,
}

impl LoginQueue {
    /// Construct a queue with an explicit start spacing, address permit
    /// cap, and address idle TTL. At most one granted login is in flight at
    /// a time. `ip_cap` counts completed attempts plus pending reservations;
    /// a full `ip_window` starts at the latest login return only after no
    /// reservation remains in flight.
    pub fn new(spacing: Duration, ip_cap: usize, ip_window: Duration) -> Self {
        assert!(ip_cap > 0, "ip_cap must be positive");
        Self {
            spacing,
            max_in_flight: 1,
            ip_cap,
            ip_window,
            queue: VecDeque::new(),
            preferred: None,
            last_grant: None,
            previous_grant: None,
            ip_count: 0,
            ip_pending: 0,
            ip_last: None,
            throttle_until: None,
            by_uid: HashMap::new(),
        }
    }

    /// Accounting tests that need several unreturned grants at once.
    #[cfg(test)]
    fn with_max_in_flight(mut self, max_in_flight: usize) -> Self {
        self.max_in_flight = max_in_flight;
        self
    }

    /// Enter one slot owner into the FIFO exactly once at its Queueing
    /// transition. Worker arrival is FIFO; the currently preferred owner is
    /// the sole exception and enters at the front.
    pub fn enqueue_owner(&mut self, owner: u64, uid: i32) {
        if self.queue.iter().any(|entry| entry.owner == owner) {
            return;
        }
        let entry = QueueEntry { owner, uid };
        if self.preferred == Some(owner) {
            self.queue.push_front(entry);
        } else {
            self.queue.push_back(entry);
        }
    }

    /// Poll a slot-owner place already entered through
    /// [`Self::enqueue_owner`].
    pub fn poll_owner(&mut self, owner: u64, uid: i32, now: Instant) -> Permit {
        self.expire(now);
        if self.queue.front().map(|entry| entry.owner) != Some(owner) {
            return Permit::Wait(QUEUE_POLL);
        }
        debug_assert_eq!(self.queue.front().map(|entry| entry.uid), Some(uid));
        match self.blocker(uid, now) {
            Some(blocker) => Permit::Wait(
                blocker
                    .until()
                    .map_or(QUEUE_POLL, |until| until.saturating_duration_since(now)),
            ),
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

    /// What keeps `owner` from starting its login at `now` while it is the
    /// queue head. `None` for any other owner (the bots ahead of it are its
    /// blocker) and for a head the next poll would grant.
    pub fn head_blocker(&self, owner: u64, now: Instant) -> Option<AdmitBlocker> {
        let head = self.queue.front().filter(|entry| entry.owner == owner)?;
        self.blocker(head.uid, now)
    }

    /// Drop one slot owner's place. Other owners with the same device UID
    /// remain queued.
    pub fn leave_owner(&mut self, owner: u64) -> bool {
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
    /// This removes its reservation without refreshing either server TTL,
    /// and the start spacing runs from the previous real start again.
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
        if self.ip_pending == 0 {
            self.last_grant = self.previous_grant;
        }
        true
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

    /// Forget limits whose time has passed: the response-16 hold, an idle
    /// address count and idle device rows.
    fn expire(&mut self, now: Instant) {
        self.prune_uid(now);
        if self.throttle_until.is_some_and(|until| until <= now) {
            self.throttle_until = None;
        }
        if self.address_idle(now) {
            self.ip_count = 0;
            self.ip_last = None;
        }
    }

    /// The address count has gone a whole idle TTL without an attempt.
    fn address_idle(&self, now: Instant) -> bool {
        self.ip_pending == 0
            && self
                .ip_last
                .is_some_and(|last| now.saturating_duration_since(last) >= self.ip_window)
    }

    /// The constraint that releases `uid` last, or `None` when it may start
    /// now. A limit that waits for the in-flight login ranks by the least it
    /// can still take (its whole TTL after that return).
    fn blocker(&self, uid: i32, now: Instant) -> Option<AdmitBlocker> {
        let mut worst: Option<(AdmitBlocker, Instant)> = None;
        let mut consider = |blocker: AdmitBlocker, release: Instant| {
            if worst.is_none_or(|(_, latest)| release > latest) {
                worst = Some((blocker, release));
            }
        };

        if self.ip_pending >= self.max_in_flight {
            consider(AdmitBlocker::Handshake, now);
        } else if let Some(until) = self
            .last_grant
            .map(|last| last + self.spacing)
            .filter(|until| *until > now)
        {
            consider(AdmitBlocker::Spacing { until }, until);
        }
        if let Some(until) = self.throttle_until.filter(|until| *until > now) {
            consider(AdmitBlocker::ServerHold { until }, until);
        }
        if self.ip_count >= self.ip_cap && !self.address_idle(now) {
            let until = match self.ip_pending {
                0 => Some(
                    self.ip_last
                        .expect("address ack time exists when cap is reached")
                        + self.ip_window,
                ),
                _ => None,
            };
            consider(
                AdmitBlocker::AddressBudget {
                    attempts: self.ip_count as u32,
                    until,
                },
                until.unwrap_or(now + self.ip_window),
            );
        }
        if let Some(state) = self
            .by_uid
            .get(&uid)
            .filter(|state| state.count >= UID_GRANT_CAP)
        {
            if state.pending > 0 {
                consider(
                    AdmitBlocker::DeviceBudget { until: None },
                    now + UID_COOLDOWN,
                );
            } else if let Some(until) = state
                .last
                .map(|last| last + UID_COOLDOWN)
                .filter(|until| *until > now)
            {
                consider(AdmitBlocker::DeviceBudget { until: Some(until) }, until);
            }
        }
        worst.map(|(blocker, _)| blocker)
    }

    fn grant(&mut self, owner: u64, uid: i32, now: Instant) {
        debug_assert_eq!(
            self.queue.front().map(|entry| (entry.owner, entry.uid)),
            Some((owner, uid))
        );
        self.queue.pop_front();
        self.previous_grant = self.last_grant;
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
        // The engine's configured address threshold 30 rejects the 30th
        // attempt, hence 29 grants / 60 s idle.
        Self::new(LOGIN_START_SPACING, 29, Duration::from_secs(60))
    }
}

/// First retry after a transport failure or a world-full refusal.
const RETRY_BASE: Duration = Duration::from_secs(5);
/// Longest retry delay: the engine's address login-attempt TTL
/// (`World.ts:176`, 60 s). After that much quiet the server has forgotten
/// this address's earlier attempts.
const RETRY_CAP: Duration = Duration::from_secs(60);

/// One slot's escalating retry delays. Reset after a successful login.
#[derive(Debug, Default)]
pub struct LoginBackoff {
    throttle_hits: u32,
    retry_hits: u32,
}

impl LoginBackoff {
    pub fn new() -> Self {
        Self::default()
    }

    /// Delay after a response-16 (too many login attempts) rejection:
    /// 20 s + 45 s per prior hit.
    pub fn throttle_delay(&mut self) -> Duration {
        let delay =
            Duration::from_secs(20) + Duration::from_secs(45 * u64::from(self.throttle_hits));
        self.throttle_hits += 1;
        delay
    }

    /// Delay after a failed connection or a full world: 5, 10, 20, 40, then
    /// 60 s, each scaled by a jitter of 80–120 % taken from `entropy` and
    /// never above 60 s, so slots that failed together retry apart.
    pub fn retry_delay(&mut self, entropy: u32) -> Duration {
        let doublings = self.retry_hits.min(4);
        self.retry_hits = self.retry_hits.saturating_add(1);
        let nominal = (RETRY_BASE * (1 << doublings)).min(RETRY_CAP);
        let permille = 800 + u64::from(entropy) % 401;
        let jittered = Duration::from_millis(nominal.as_millis() as u64 * permille / 1000);
        jittered.min(RETRY_CAP)
    }

    pub fn reset(&mut self) {
        self.throttle_hits = 0;
        self.retry_hits = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_OWNER_NAMESPACE: u64 = 1 << 63;
    const TICK: Duration = Duration::from_millis(600);

    fn test_owner(uid: i32) -> u64 {
        TEST_OWNER_NAMESPACE | u64::from(uid as u32)
    }

    fn request(q: &mut LoginQueue, uid: i32, now: Instant) -> Permit {
        request_owner(q, test_owner(uid), uid, now)
    }

    fn request_owner(q: &mut LoginQueue, owner: u64, uid: i32, now: Instant) -> Permit {
        q.enqueue_owner(owner, uid);
        q.poll_owner(owner, uid, now)
    }

    /// Unpaced queue for the address/device accounting cases.
    fn unpaced(ip_cap: usize) -> LoginQueue {
        LoginQueue::new(Duration::ZERO, ip_cap, Duration::from_secs(60))
    }

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

    /// One login start: when it began and when its `client.login` returned.
    struct Attempt {
        uid: i32,
        start: Instant,
        end: Instant,
    }

    /// Drive the default queue the way slot workers do: every owner polls
    /// every 20 ms, each granted handshake returns after `handshake`.
    fn simulate_default(owners: i32, handshake: Duration) -> Vec<Attempt> {
        let base = Instant::now();
        let mut q = LoginQueue::default();
        for uid in 0..owners {
            q.enqueue_owner(test_owner(uid), uid);
        }
        let mut attempts: Vec<Attempt> = Vec::new();
        let mut in_flight: Vec<(i32, Instant)> = Vec::new();
        let mut now = base;
        while attempts.len() < owners as usize || !in_flight.is_empty() {
            in_flight.retain(|&(uid, end)| {
                if end <= now {
                    assert!(q.acknowledge_login_return(uid, end));
                    false
                } else {
                    true
                }
            });
            for uid in 0..owners {
                if q.status_owner(test_owner(uid)).is_none() {
                    continue;
                }
                if q.poll_owner(test_owner(uid), uid, now) == Permit::Grant {
                    in_flight.push((uid, now + handshake));
                    attempts.push(Attempt {
                        uid,
                        start: now,
                        end: now + handshake,
                    });
                }
            }
            now += QUEUE_POLL;
            assert!(
                now < base + Duration::from_secs(600),
                "{owners} owners must all start within ten minutes"
            );
        }
        attempts
    }

    #[test]
    fn default_starts_one_handshake_at_a_time_two_ticks_apart() {
        let attempts = simulate_default(20, Duration::from_millis(100));
        assert_eq!(attempts.len(), 20, "every bot gets its login");
        assert_eq!(
            attempts.iter().map(|a| a.uid).collect::<Vec<_>>(),
            (0..20).collect::<Vec<_>>(),
            "grants follow arrival order"
        );
        for pair in attempts.windows(2) {
            let gap = pair[1].start.saturating_duration_since(pair[0].start);
            assert!(
                gap >= 2 * TICK,
                "starts {gap:?} apart; the default spaces them two ticks"
            );
            assert!(
                pair[1].start >= pair[0].end,
                "a login starts only after the previous one returned"
            );
        }
    }

    #[test]
    fn slow_handshakes_still_keep_one_in_flight() {
        let attempts = simulate_default(5, Duration::from_secs(3));
        for pair in attempts.windows(2) {
            assert!(pair[1].start >= pair[0].end, "no overlapping handshakes");
        }
    }

    #[test]
    fn default_address_budget_still_waits_out_the_idle_ttl_after_29() {
        let attempts = simulate_default(31, Duration::from_millis(100));
        let starts: Vec<Instant> = attempts.iter().map(|a| a.start).collect();
        assert!(max_grants_in_60s(&starts) <= 29);
        let idle = attempts[29]
            .start
            .saturating_duration_since(attempts[28].end);
        assert!(
            idle >= Duration::from_secs(60),
            "the 30th attempt waits 60 s after the 29th returned, got {idle:?}"
        );
    }

    #[test]
    fn head_blocker_names_the_in_flight_login_then_the_spacing() {
        let base = Instant::now();
        let mut q = LoginQueue::default();
        assert_eq!(request(&mut q, 1, base), Permit::Grant);
        assert!(matches!(request(&mut q, 2, base), Permit::Wait(_)));
        assert!(matches!(request(&mut q, 3, base), Permit::Wait(_)));
        assert_eq!(
            q.head_blocker(test_owner(2), base),
            Some(AdmitBlocker::Handshake)
        );
        assert_eq!(
            q.head_blocker(test_owner(3), base),
            None,
            "behind the head, the bots ahead are the blocker"
        );

        let returned = base + Duration::from_millis(300);
        assert!(q.acknowledge_login_return(1, returned));
        assert_eq!(
            q.head_blocker(test_owner(2), returned),
            Some(AdmitBlocker::Spacing {
                until: base + LOGIN_START_SPACING
            })
        );
        assert_eq!(
            request(&mut q, 2, returned),
            Permit::Wait(LOGIN_START_SPACING - Duration::from_millis(300))
        );
        assert_eq!(
            request(&mut q, 2, base + LOGIN_START_SPACING),
            Permit::Grant
        );
    }

    #[test]
    fn head_blocker_shows_the_address_cooldown_deadline() {
        let base = Instant::now();
        let mut q = unpaced(2);
        assert_eq!(request(&mut q, 1, base), Permit::Grant);
        assert!(matches!(request(&mut q, 9, base), Permit::Wait(_)));
        assert_eq!(
            q.head_blocker(test_owner(9), base),
            Some(AdmitBlocker::Handshake)
        );
        assert!(q.acknowledge_login_return(1, base));
        q.leave_owner(test_owner(9));
        assert_eq!(request(&mut q, 2, base), Permit::Grant);
        assert!(matches!(request(&mut q, 9, base), Permit::Wait(_)));
        assert_eq!(
            q.head_blocker(test_owner(9), base),
            Some(AdmitBlocker::AddressBudget {
                attempts: 2,
                until: None
            }),
            "the budget is spent; its idle period starts when the login returns"
        );
        let returned = base + Duration::from_secs(2);
        assert!(q.acknowledge_login_return(2, returned));
        assert_eq!(
            q.head_blocker(test_owner(9), returned),
            Some(AdmitBlocker::AddressBudget {
                attempts: 2,
                until: Some(returned + Duration::from_secs(60))
            })
        );
        q.hold_for(returned, Duration::from_secs(90));
        assert_eq!(
            q.head_blocker(test_owner(9), returned),
            Some(AdmitBlocker::ServerHold {
                until: returned + Duration::from_secs(90)
            }),
            "the latest release wins"
        );
    }

    #[test]
    fn focused_owner_jumps_the_fifo_but_not_the_pacing() {
        let base = Instant::now();
        let mut q = LoginQueue::default();
        assert_eq!(request(&mut q, 1, base), Permit::Grant);
        for uid in 2..=4 {
            assert!(matches!(request(&mut q, uid, base), Permit::Wait(_)));
        }
        q.set_preferred_owner(Some(test_owner(4)));
        assert_eq!(q.queued_uids(), vec![4, 2, 3]);
        assert!(
            matches!(request(&mut q, 4, base), Permit::Wait(_)),
            "focus does not start a second handshake"
        );
        assert!(q.acknowledge_login_return(1, base + TICK));
        assert!(
            matches!(request(&mut q, 4, base + TICK), Permit::Wait(_)),
            "focus does not skip the start spacing"
        );
        assert_eq!(
            request(&mut q, 4, base + LOGIN_START_SPACING),
            Permit::Grant
        );
        assert_eq!(q.queued_uids(), vec![2, 3], "the rest keep arrival order");
    }

    #[test]
    fn preferred_owner_retains_priority_after_first_login() {
        let now = Instant::now();
        let mut q = LoginQueue::new(Duration::from_secs(1), 30, Duration::from_secs(60));
        q.prefer_owner(test_owner(1));
        assert_eq!(request(&mut q, 1, now), Permit::Grant);
        assert!(q.acknowledge_login_return(1, now));
        assert!(q.queued_uids().is_empty());
        assert!(matches!(request(&mut q, 2, now), Permit::Wait(_)));
        assert!(matches!(request(&mut q, 1, now), Permit::Wait(_)));
        assert_eq!(q.queued_uids(), vec![1, 2]);
        assert_eq!(
            request(&mut q, 1, now + Duration::from_secs(1)),
            Permit::Grant
        );
        assert!(q.acknowledge_login_return(1, now + Duration::from_secs(1)));
        assert_eq!(
            request(&mut q, 2, now + Duration::from_secs(2)),
            Permit::Grant
        );
    }

    #[test]
    fn duplicate_uids_keep_independent_fifo_owners() {
        let now = Instant::now();
        let later = now + LOGIN_START_SPACING;
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
        assert_eq!(q.poll_owner(22, 7, later), Permit::Grant);
        assert!(q.acknowledge_login_return(7, later));
        assert!(q.queued_uids().is_empty());
    }

    #[test]
    fn worker_arrival_order_has_no_missing_owner_gate() {
        let now = Instant::now();
        let later = now + LOGIN_START_SPACING;
        let mut q = LoginQueue::default();
        q.enqueue_owner(22, 2);
        q.enqueue_owner(11, 1);
        assert_eq!(q.queued_uids(), vec![2, 1]);
        assert_eq!(q.poll_owner(22, 2, now), Permit::Grant);
        assert!(q.acknowledge_login_return(2, now));
        assert_eq!(q.poll_owner(11, 1, later), Permit::Grant);
        assert!(q.acknowledge_login_return(1, later));
    }

    #[test]
    fn focus_changes_do_not_reserve_online_slots_or_bypass_limits() {
        let now = Instant::now();
        let mut q = LoginQueue::new(Duration::from_secs(1), 1, Duration::from_secs(60));
        q.set_preferred_owner(Some(test_owner(1)));
        assert!(q.queued_uids().is_empty());
        assert_eq!(request(&mut q, 2, now), Permit::Grant);
        assert!(matches!(request(&mut q, 3, now), Permit::Wait(_)));
        assert!(matches!(request(&mut q, 1, now), Permit::Wait(_)));
        q.set_preferred_owner(Some(test_owner(3)));
        assert_eq!(q.queued_uids(), vec![3, 1]);
        assert!(matches!(
            request(&mut q, 3, now + Duration::from_secs(1)),
            Permit::Wait(_)
        ));
        q.leave_owner(test_owner(3));
        q.leave_owner(test_owner(1));
        q.set_preferred_owner(None);
        assert!(matches!(request(&mut q, 2, now), Permit::Wait(_)));
        assert!(matches!(request(&mut q, 3, now), Permit::Wait(_)));
        assert_eq!(q.queued_uids(), vec![2, 3]);
    }

    #[test]
    fn prefer_moves_owner_to_the_front() {
        // Long spacing so only the first grant lands; the rest stay queued.
        let mut q = LoginQueue::new(Duration::from_secs(60), 30, Duration::from_secs(60));
        let now = Instant::now();
        assert!(matches!(request(&mut q, 2, now), Permit::Grant));
        assert!(matches!(request(&mut q, 3, now), Permit::Wait(_)));
        assert!(matches!(request(&mut q, 1, now), Permit::Wait(_)));
        q.prefer_owner(test_owner(1));
        assert_eq!(q.queue.front().map(|entry| entry.uid), Some(1));
        assert_eq!(q.status_owner(test_owner(1)).unwrap().position, 1);
    }

    #[test]
    fn ip_idle_ttl_cap_holds_when_spacing_is_small() {
        let base = Instant::now();
        let mut q = LoginQueue::new(Duration::from_millis(1), 29, Duration::from_secs(60));
        let mut now = base;
        let mut grants = Vec::new();
        for i in 0..50 {
            if let Permit::Grant = request(&mut q, i, now) {
                grants.push(now);
                assert!(q.acknowledge_login_return(i, now));
            }
            now += Duration::from_millis(1);
        }
        // Cap binds: the 30th grant must wait out the 60 s idle TTL.
        assert_eq!(grants.len(), 29);

        now = base + Duration::from_secs(61);
        for i in 29..50 {
            assert!(matches!(request(&mut q, i, now), Permit::Grant));
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
        let mut q = unpaced(2);
        assert_eq!(request(&mut q, 1, base), Permit::Grant);
        assert!(q.acknowledge_login_return(1, base));
        assert_eq!(
            request(&mut q, 2, base + Duration::from_secs(30)),
            Permit::Grant
        );
        assert!(q.acknowledge_login_return(2, base + Duration::from_secs(30)));

        match request(&mut q, 3, base + Duration::from_secs(60)) {
            Permit::Wait(wait) => assert_eq!(wait, Duration::from_secs(30)),
            Permit::Grant => panic!("oldest + TTL must not release the refreshed address key"),
        }
        assert!(matches!(
            request(&mut q, 4, base + Duration::from_secs(60)),
            Permit::Wait(_)
        ));
        assert_eq!(q.queued_uids(), vec![3, 4], "blocked callers remain FIFO");

        assert_eq!(
            request(&mut q, 3, base + Duration::from_secs(90)),
            Permit::Grant
        );
        assert!(q.acknowledge_login_return(3, base + Duration::from_secs(90)));
        assert_eq!(
            request(&mut q, 4, base + Duration::from_secs(90)),
            Permit::Grant
        );
    }

    #[test]
    fn unacked_reservation_does_not_expire_without_a_server_attempt_clock() {
        let base = Instant::now();
        let mut q = unpaced(1);
        assert_eq!(request(&mut q, 1, base), Permit::Grant);

        assert!(
            matches!(
                request(&mut q, 2, base + Duration::from_secs(120)),
                Permit::Wait(_)
            ),
            "a delayed in-flight attempt can still refresh the server TTL"
        );
    }

    #[test]
    fn reordered_login_returns_keep_the_latest_monotonic_ack_clock() {
        let base = Instant::now();
        let mut q = unpaced(2).with_max_in_flight(2);
        assert_eq!(request(&mut q, 1, base), Permit::Grant);
        assert_eq!(request(&mut q, 2, base), Permit::Grant);

        assert!(q.acknowledge_login_return(2, base + Duration::from_secs(120)));
        assert!(q.acknowledge_login_return(1, base + Duration::from_secs(61)));
        match request(&mut q, 3, base + Duration::from_secs(121)) {
            Permit::Wait(wait) => assert_eq!(wait, Duration::from_secs(59)),
            Permit::Grant => panic!("an older completion must not move the ack clock backward"),
        }
        assert_eq!(
            request(&mut q, 3, base + Duration::from_secs(180)),
            Permit::Grant
        );
    }

    #[test]
    fn one_old_ack_cannot_reset_count_while_another_attempt_is_pending() {
        let base = Instant::now();
        let mut q = unpaced(2).with_max_in_flight(2);
        assert_eq!(request(&mut q, 1, base), Permit::Grant);
        assert_eq!(request(&mut q, 2, base), Permit::Grant);
        assert!(q.acknowledge_login_return(1, base));

        assert!(
            matches!(
                request(&mut q, 3, base + Duration::from_secs(120)),
                Permit::Wait(_)
            ),
            "the pending attempt can still refresh all address accounting"
        );
        assert!(q.acknowledge_login_return(2, base + Duration::from_secs(130)));
        assert!(matches!(
            request(&mut q, 3, base + Duration::from_secs(189)),
            Permit::Wait(_)
        ));
        assert_eq!(
            request(&mut q, 3, base + Duration::from_secs(190)),
            Permit::Grant
        );
    }

    #[test]
    fn abandoning_unused_permit_releases_reservation_without_starting_ttl() {
        let base = Instant::now();
        let mut q = unpaced(1);
        assert_eq!(request(&mut q, 1, base), Permit::Grant);
        assert!(q.abandon_permit(1));
        assert!(!q.abandon_permit(1), "cleanup is exactly once");
        assert_eq!(request(&mut q, 2, base), Permit::Grant);
    }

    #[test]
    fn abandoned_grant_does_not_hold_the_next_start() {
        let base = Instant::now();
        let mut q = LoginQueue::default();
        assert_eq!(request(&mut q, 1, base), Permit::Grant);
        assert!(q.abandon_permit(1));
        assert_eq!(
            request(&mut q, 2, base),
            Permit::Grant,
            "no socket opened, so there is nothing to space from"
        );
        assert!(q.acknowledge_login_return(2, base));
        assert!(
            matches!(request(&mut q, 3, base), Permit::Wait(_)),
            "a real start is spaced"
        );
    }

    #[test]
    fn duplicate_uid_reservations_are_cleaned_up_one_at_a_time() {
        let base = Instant::now();
        let mut q = unpaced(2).with_max_in_flight(2);
        assert_eq!(request(&mut q, 7, base), Permit::Grant);
        assert_eq!(request(&mut q, 7, base), Permit::Grant);

        assert!(q.acknowledge_login_return(7, base + Duration::from_secs(10)));
        assert!(q.abandon_permit(7));
        assert!(!q.abandon_permit(7), "both pending grants were resolved");
        assert_eq!(
            request(&mut q, 8, base + Duration::from_secs(10)),
            Permit::Grant,
            "abandoning one duplicate must preserve only the acknowledged attempt"
        );
    }

    #[test]
    fn pending_uid_reservations_do_not_expire_before_login_returns() {
        let base = Instant::now();
        let mut q = unpaced(10).with_max_in_flight(10);
        for _ in 0..UID_GRANT_CAP {
            assert_eq!(request(&mut q, 7, base), Permit::Grant);
        }
        assert!(matches!(
            request(&mut q, 7, base + Duration::from_secs(120)),
            Permit::Wait(_)
        ));

        for _ in 0..UID_GRANT_CAP {
            assert!(q.abandon_permit(7));
        }
        assert_eq!(
            request(&mut q, 7, base + Duration::from_secs(120)),
            Permit::Grant
        );
    }

    #[test]
    fn same_uid_fifth_request_waits_remaining_15s_ttl() {
        let base = Instant::now();
        let mut q = unpaced(29);
        for _ in 0..4 {
            assert!(matches!(request(&mut q, 7, base), Permit::Grant));
            assert!(q.acknowledge_login_return(7, base));
        }
        match request(&mut q, 7, base) {
            Permit::Wait(d) => assert_eq!(d, Duration::from_secs(15), "fifth wait {d:?}"),
            Permit::Grant => panic!("fifth same-uid request must wait"),
        }
        assert_eq!(
            q.head_blocker(test_owner(7), base),
            Some(AdmitBlocker::DeviceBudget {
                until: Some(base + UID_COOLDOWN)
            })
        );
        let later = base + Duration::from_secs(10);
        match request(&mut q, 7, later) {
            Permit::Wait(d) => assert_eq!(d, Duration::from_secs(5), "remaining {d:?}"),
            Permit::Grant => panic!("still inside the 15 s device TTL"),
        }
        assert!(matches!(
            request(&mut q, 7, base + Duration::from_secs(15)),
            Permit::Grant
        ));
    }

    /// A waiting uid keeps its FIFO place, so its cooldown row survives
    /// pruning. Its retry lands a little after the 15 s TTL (a real sleep
    /// overshoots), which must grant, not panic.
    #[test]
    fn queued_uid_retrying_after_ttl_overshoot_grants() {
        let base = Instant::now();
        let mut q = unpaced(29);
        for _ in 0..4 {
            assert!(matches!(request(&mut q, 7, base), Permit::Grant));
            assert!(q.acknowledge_login_return(7, base));
        }
        assert!(matches!(request(&mut q, 7, base), Permit::Wait(_)));
        let retry = base + UID_COOLDOWN + Duration::from_millis(3);
        assert!(matches!(request(&mut q, 7, retry), Permit::Grant));
    }

    #[test]
    fn not_head_polls_20ms_not_depth_times_spacing() {
        let mut q = LoginQueue::default();
        let mut now = Instant::now();
        for i in 0..29 {
            assert!(matches!(request(&mut q, i, now), Permit::Grant));
            assert!(q.acknowledge_login_return(i, now));
            now += LOGIN_START_SPACING;
        }
        now -= LOGIN_START_SPACING;
        match request(&mut q, 29, now) {
            Permit::Wait(d) => assert!(
                d >= Duration::from_secs(59),
                "head waits out the address idle TTL, got {d:?}"
            ),
            other => panic!("head should wait the 60 s address idle TTL, got {other:?}"),
        }
        match request(&mut q, 30, now) {
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
        let mut q = unpaced(29);
        let now = Instant::now();
        for _ in 0..4 {
            assert!(matches!(request(&mut q, 7, now), Permit::Grant));
            assert!(q.acknowledge_login_return(7, now));
        }
        assert!(q.tracks(7));
        let now = now + UID_COOLDOWN;
        assert!(matches!(request(&mut q, 8, now), Permit::Grant));
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
        assert!(q.status_owner(test_owner(1)).is_none());
        assert!(matches!(request(&mut q, 1, now), Permit::Grant));
        assert!(q.status_owner(test_owner(1)).is_none());
        assert!(matches!(request(&mut q, 2, now), Permit::Wait(_)));
        assert!(matches!(request(&mut q, 3, now), Permit::Wait(_)));
        let s2 = q.status_owner(test_owner(2)).unwrap();
        let s3 = q.status_owner(test_owner(3)).unwrap();
        assert_eq!((s2.position, s2.total), (1, 2));
        assert_eq!((s3.position, s3.total), (2, 2));
        q.leave_owner(test_owner(2));
        let s3 = q.status_owner(test_owner(3)).unwrap();
        assert_eq!((s3.position, s3.total), (1, 1));
        assert!(q.status_owner(test_owner(2)).is_none());
    }

    #[test]
    fn partial_uid_count_expires_after_idle() {
        let base = Instant::now();
        let mut q = unpaced(29);
        for _ in 0..3 {
            assert_eq!(request(&mut q, 7, base), Permit::Grant);
            assert!(q.acknowledge_login_return(7, base));
        }
        let fresh = base + Duration::from_secs(100);
        assert_eq!(request(&mut q, 7, fresh), Permit::Grant);
        assert!(q.acknowledge_login_return(7, fresh));
        assert_eq!(request(&mut q, 7, fresh), Permit::Grant);
    }

    #[test]
    fn response_throttle_holds_sibling_attempts() {
        let base = Instant::now();
        let mut q = LoginQueue::default();
        q.hold_for(base, Duration::from_secs(20));
        assert!(
            matches!(request(&mut q, 8, base), Permit::Wait(wait) if wait == Duration::from_secs(20))
        );
        assert_eq!(
            request(&mut q, 8, base + Duration::from_secs(20)),
            Permit::Grant
        );
    }

    #[test]
    fn two_uids_enqueue_in_order() {
        let mut q = LoginQueue::new(Duration::from_secs(60), 30, Duration::from_secs(60));
        let now = Instant::now();
        let _ = request(&mut q, 10, now); // grant
        let _ = request(&mut q, 11, now);
        let _ = request(&mut q, 12, now);
        assert_eq!(q.status_owner(test_owner(11)).unwrap().position, 1);
        assert_eq!(q.status_owner(test_owner(12)).unwrap().position, 2);
        assert_eq!(q.status_owner(test_owner(12)).unwrap().total, 2);
    }

    #[test]
    fn enqueue_order_wins_request_lock_race() {
        let base = Instant::now();
        let later = base + LOGIN_START_SPACING;
        let mut q = LoginQueue::default();
        q.enqueue_owner(10, 10);
        q.enqueue_owner(11, 11);
        assert!(matches!(q.poll_owner(11, 11, base), Permit::Wait(_)));
        assert_eq!(q.poll_owner(10, 10, base), Permit::Grant);
        assert!(q.acknowledge_login_return(10, base));
        assert_eq!(q.poll_owner(11, 11, later), Permit::Grant);
    }

    #[test]
    fn preferred_owner_that_is_not_waiting_cannot_block_followers() {
        let mut q = LoginQueue::new(Duration::from_secs(60), 30, Duration::from_secs(60));
        let base = Instant::now();
        q.prefer_owner(test_owner(7));
        assert!(
            q.queued_uids().is_empty(),
            "preference alone is not waiting membership"
        );
        assert_eq!(
            request(&mut q, 8, base),
            Permit::Grant,
            "a terminal preferred owner cannot become a phantom head"
        );
        assert!(q.status_owner(test_owner(7)).is_none());
    }

    #[test]
    #[should_panic(expected = "ip_cap must be positive")]
    fn zero_cap_is_rejected_at_construction() {
        let _ = LoginQueue::new(Duration::ZERO, 0, Duration::from_secs(60));
    }

    #[test]
    fn login_backoff_escalates() {
        let mut b = LoginBackoff::new();
        assert_eq!(b.throttle_delay(), Duration::from_secs(20));
        assert_eq!(b.throttle_delay(), Duration::from_secs(65));
        assert_eq!(b.throttle_delay(), Duration::from_secs(110));
        b.reset();
        assert_eq!(b.throttle_delay(), Duration::from_secs(20));
    }

    #[test]
    fn retry_backoff_increases_to_a_60s_cap_with_jitter() {
        let nominal = [5, 10, 20, 40, 60, 60, 60];
        for entropy in [0, 200, 400, u32::MAX] {
            let mut b = LoginBackoff::new();
            let delays: Vec<Duration> = nominal.iter().map(|_| b.retry_delay(entropy)).collect();
            for (delay, secs) in delays.iter().zip(nominal) {
                let nominal = Duration::from_secs(secs);
                assert!(
                    *delay >= nominal.mul_f64(0.8) && *delay <= nominal.mul_f64(1.2),
                    "{delay:?} within ±20% of {nominal:?}"
                );
                assert!(*delay <= Duration::from_secs(60), "capped at 60 s");
            }
            assert!(
                delays[..4].windows(2).all(|pair| pair[1] > pair[0]),
                "delays grow until the cap: {delays:?}"
            );
        }
        let mut low = LoginBackoff::new();
        let mut high = LoginBackoff::new();
        assert_ne!(
            low.retry_delay(0),
            high.retry_delay(400),
            "different entropy spreads slots that failed together"
        );
        low.reset();
        assert!(
            low.retry_delay(400) <= Duration::from_secs(6),
            "reset starts over"
        );
    }
}
