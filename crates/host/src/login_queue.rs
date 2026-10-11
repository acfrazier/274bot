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
//!   in `processLogins`, once per 600 ms tick, and answers them there
//!   (response 2).
//! - `src/server/tcp/TcpServer.ts:19,61-67` — the engine destroys a socket
//!   that has been silent for 30 s.
//!
//! The host stays under the production numbers (so flipping
//! `NODE_PRODUCTION` does not 16 us), and it does not burst: at most one
//! handshake is in flight, attempts start at least [`LOGIN_START_SPACING`]
//! apart, and one handshake holds the in-flight slot for at most
//! [`HANDSHAKE_HOLD`] (plus [`LEASE_SLACK`]). A handshake still running at
//! its lease end has its socket shut before the slot goes to the next one.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use client::io::client_stream::{LoginCancel, READ_TIMEOUT};

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
    /// The minimum gap after a prior login start or this owner's release wait.
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

/// Minimum gap between two login attempts, measured from the moment each
/// attempt starts (its connect). This is a host policy, not a server rule:
/// the engine has no spacing requirement, and two ticks of spacing alone
/// would not keep two attempts in separate admission batches, because
/// network delay can bunch or split them. Batches are kept apart by the
/// one-in-flight rule instead: a successful handshake returns only after
/// `processLogins` answered it, so the next attempt starts after that
/// tick. The spacing only stops a fleet of fast handshakes (about 0.4 s
/// on a local engine) from following each other back to back.
pub const LOGIN_START_SPACING: Duration = Duration::from_millis(2 * SERVER_TICK.as_millis() as u64);

/// Longest one login handshake may hold the in-flight slot, measured from
/// its start. This is a host policy. The 30 s matches the read timeouts on
/// both ends: the original client's `setSoTimeout(30000)`
/// (`ClientStream.java:47`, this client's [`READ_TIMEOUT`]) and the engine
/// destroying a socket silent for 30 s (`TcpServer.ts:19,61-67`). Neither
/// bounds a whole multi-step handshake; this deadline does. The slot worker
/// hands the deadline from [`LoginQueue::start_attempt`] to the client,
/// which stops the connect and every read and write of the handshake at it.
pub const HANDSHAKE_HOLD: Duration = READ_TIMEOUT;

/// How long after a handshake's deadline the queue still treats its slot as
/// taken. This is host policy, not a measured bound: it gives the client
/// time to fail at its own deadline and report that. A handshake still
/// running at the lease end does not rely on it: the queue shuts the
/// attempt's socket ([`LoginCancel`]) before it frees the slot.
pub const LEASE_SLACK: Duration = Duration::from_secs(1);

/// Production `rateLimitDeviceLogin` is 5 in a 15 s TTL (`>= 5` rejects,
/// so 4 grants then wait the remaining TTL). Each grant refreshes the
/// host reservation count; cooldown is measured conservatively from the
/// latest `client.login` return (or lease end) while any unreturned grant
/// prevents expiry.
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
    handshake_hold: Duration,
    ip_cap: usize,
    ip_window: Duration,
    queue: VecDeque<QueueEntry>,
    preferred: Option<u64>,
    /// When the latest attempt started (its grant, until it starts).
    last_start: Option<Instant>,
    /// The start before `last_start`, restored when an unused grant is
    /// abandoned.
    previous_start: Option<Instant>,
    /// Granted logins that have not returned, abandoned or expired.
    flights: Vec<Flight>,
    ip_count: usize,
    ip_last: Option<Instant>,
    throttle_until: Option<Instant>,
    by_uid: HashMap<i32, UidState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueueEntry {
    owner: u64,
    uid: i32,
    not_before: Option<Instant>,
}

/// One granted login, owned by its slot owner until it returns.
#[derive(Debug, Clone)]
struct Flight {
    owner: u64,
    uid: i32,
    granted: Instant,
    /// When the slot worker started the attempt ([`LoginQueue::start_attempt`]).
    started: Option<Instant>,
    /// Shuts the started attempt's socket if its lease ends first.
    cancel: Option<LoginCancel>,
}

impl Flight {
    /// When this flight stops holding the in-flight slot even if it never
    /// returns: its client stops at `start + hold`, and the queue shuts its
    /// socket at the lease end in any case.
    fn lease_end(&self, hold: Duration) -> Instant {
        self.started.unwrap_or(self.granted) + hold + LEASE_SLACK
    }
}

#[derive(Debug)]
struct UidState {
    count: usize,
    last: Option<Instant>,
}

impl LoginQueue {
    /// Construct a queue with an explicit start spacing, address permit
    /// cap, and address idle TTL. At most one granted login is in flight at
    /// a time, for at most [`HANDSHAKE_HOLD`]. `ip_cap` counts completed
    /// attempts plus pending reservations; a full `ip_window` starts at the
    /// latest login return only after no reservation remains in flight.
    pub fn new(spacing: Duration, ip_cap: usize, ip_window: Duration) -> Self {
        assert!(ip_cap > 0, "ip_cap must be positive");
        Self {
            spacing,
            max_in_flight: 1,
            handshake_hold: HANDSHAKE_HOLD,
            ip_cap,
            ip_window,
            queue: VecDeque::new(),
            preferred: None,
            last_start: None,
            previous_start: None,
            flights: Vec::new(),
            ip_count: 0,
            ip_last: None,
            throttle_until: None,
            by_uid: HashMap::new(),
        }
    }

    /// Replace [`HANDSHAKE_HOLD`] (regressions that wait for a lease to run
    /// out use a short one).
    pub fn with_handshake_hold(mut self, hold: Duration) -> Self {
        self.handshake_hold = hold;
        self
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
        let entry = QueueEntry {
            owner,
            uid,
            not_before: None,
        };
        if self.preferred == Some(owner) {
            self.queue.push_front(entry);
        } else {
            self.queue.push_back(entry);
        }
    }

    /// Pace an already queued owner without reserving a handshake or charging
    /// an attempt. This preserves its FIFO place and all shared admission holds.
    pub fn defer_owner_until(&mut self, owner: u64, until: Instant) {
        if let Some(entry) = self.queue.iter_mut().find(|entry| entry.owner == owner) {
            entry.not_before = Some(entry.not_before.map_or(until, |old| old.max(until)));
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
        match self.head_blocker(owner, now) {
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
        self.blocker(head.uid, now).or_else(|| {
            head.not_before
                .filter(|until| *until > now)
                .map(|until| AdmitBlocker::Spacing { until })
        })
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

    /// Mark `owner`'s granted login as started now, its connect about to
    /// begin. Start spacing counts from here, so a worker delayed between
    /// its grant and its connect cannot start closer than the spacing to the
    /// previous attempt. Returns the deadline by which the attempt must stop
    /// all socket I/O, or `None` if the grant's lease already ran out
    /// (nothing may be attempted on it; the owner queues again). The queue
    /// keeps `cancel`: if the attempt has not returned when its lease ends,
    /// the queue shuts its socket before the next login can be granted.
    pub fn start_attempt(
        &mut self,
        owner: u64,
        now: Instant,
        cancel: LoginCancel,
    ) -> Option<Instant> {
        self.expire(now);
        let flight = self
            .flights
            .iter_mut()
            .find(|flight| flight.owner == owner && flight.started.is_none())?;
        flight.started = Some(now);
        flight.cancel = Some(cancel);
        self.last_start = Some(self.last_start.map_or(now, |last| last.max(now)));
        Some(now + self.handshake_hold)
    }

    /// Record the return of `owner`'s login call. Success and error returns
    /// both count as an attempt: the client may have reached the server
    /// before either result. `false` when the owner holds no grant, which
    /// is the case after its lease ran out (it was counted then).
    pub fn acknowledge_login_return(&mut self, owner: u64, now: Instant) -> bool {
        let Some(i) = self.flights.iter().position(|flight| flight.owner == owner) else {
            return false;
        };
        let flight = self.flights.remove(i);
        self.settle(flight.uid, now);
        true
    }

    /// Give back `owner`'s grant that never started a login. This removes its
    /// reservation without refreshing either server TTL, and the start
    /// spacing runs from the previous real start again.
    pub fn abandon_permit(&mut self, owner: u64) -> bool {
        let Some(i) = self
            .flights
            .iter()
            .position(|flight| flight.owner == owner && flight.started.is_none())
        else {
            return false;
        };
        let flight = self.flights.remove(i);
        let state = self
            .by_uid
            .get_mut(&flight.uid)
            .expect("a granted uid is tracked");
        state.count = state
            .count
            .checked_sub(1)
            .expect("pending uid grant is counted");
        self.ip_count = self
            .ip_count
            .checked_sub(1)
            .expect("pending address grant is counted");
        if self.flights.is_empty() {
            self.last_start = self.previous_start;
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

    /// The server TTLs run from an attempt's return (or its lease end).
    fn settle(&mut self, uid: i32, at: Instant) {
        if let Some(state) = self.by_uid.get_mut(&uid) {
            state.last = Some(state.last.map_or(at, |last| last.max(at)));
        }
        self.ip_last = Some(self.ip_last.map_or(at, |last| last.max(at)));
    }

    /// Forget limits whose time has passed: logins whose lease ran out (each
    /// counted as an attempt that returned at its lease end), the
    /// response-16 hold, an idle address count and idle device rows. A
    /// started login whose lease ran out has its socket shut here, before
    /// its slot is free: every grant runs after this under the same lock.
    fn expire(&mut self, now: Instant) {
        let hold = self.handshake_hold;
        while let Some(i) = self
            .flights
            .iter()
            .position(|flight| flight.lease_end(hold) <= now)
        {
            let flight = self.flights.remove(i);
            if let Some(cancel) = &flight.cancel {
                cancel.cancel();
            }
            self.settle(flight.uid, flight.lease_end(hold));
        }
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
        self.flights.is_empty()
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

        let in_flight = self
            .flights
            .iter()
            .filter(|flight| flight.lease_end(self.handshake_hold) > now)
            .count();
        if in_flight >= self.max_in_flight {
            consider(AdmitBlocker::Handshake, now);
        } else if let Some(until) = self
            .last_start
            .map(|last| last + self.spacing)
            .filter(|until| *until > now)
        {
            consider(AdmitBlocker::Spacing { until }, until);
        }
        if let Some(until) = self.throttle_until.filter(|until| *until > now) {
            consider(AdmitBlocker::ServerHold { until }, until);
        }
        if self.ip_count >= self.ip_cap && !self.address_idle(now) {
            let until = self.flights.is_empty().then(|| {
                self.ip_last
                    .expect("address ack time exists when cap is reached")
                    + self.ip_window
            });
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
            if self.flights.iter().any(|flight| flight.uid == uid) {
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
        self.previous_start = self.last_start;
        self.last_start = Some(now);
        self.ip_count += 1;

        let pending = self.flights.iter().any(|flight| flight.uid == uid);
        let state = self.by_uid.entry(uid).or_insert(UidState {
            count: 0,
            last: None,
        });
        if state.count >= UID_GRANT_CAP {
            // Cooldown has elapsed; restart the uid window.
            debug_assert!(!pending);
            state.count = 0;
            state.last = None;
        }
        state.count += 1;
        self.flights.push(Flight {
            owner,
            uid,
            granted: now,
            started: None,
            cancel: None,
        });
    }

    /// Drop every uid accounting row after 15 s idle when it has no pending
    /// permit. Queue identity is stored separately, so an expired waiting uid
    /// keeps its FIFO place while its partial counter resets.
    fn prune_uid(&mut self, now: Instant) {
        let flights = &self.flights;
        self.by_uid.retain(|uid, state| {
            flights.iter().any(|flight| flight.uid == *uid)
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
        let delay = self.next_throttle_delay();
        self.throttle_hits += 1;
        delay
    }

    /// What the next [`Self::throttle_delay`] returns, without counting a
    /// hit: the shared hold a response 16 publishes when its login returns.
    pub fn next_throttle_delay(&self) -> Duration {
        Duration::from_secs(20) + Duration::from_secs(45 * u64::from(self.throttle_hits))
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

    #[test]
    fn deferred_owner_keeps_fifo_without_reserving_or_charging_attempts() {
        let now = Instant::now();
        let mut queue = LoginQueue::default();
        queue.enqueue_owner(1, 7);
        queue.enqueue_owner(2, 8);
        queue.defer_owner_until(1, now + Duration::from_secs(5));
        queue.defer_owner_until(1, now + Duration::from_secs(1));
        assert_eq!(
            queue.poll_owner(1, 7, now),
            Permit::Wait(Duration::from_secs(5))
        );
        assert_eq!(queue.ip_count, 0);
        assert!(queue.flights.is_empty());
        assert!(queue.by_uid.is_empty());
        assert_eq!(queue.status_owner(2).unwrap().position, 2);
        queue.hold_for(now, Duration::from_secs(20));
        assert_eq!(
            queue.poll_owner(1, 7, now),
            Permit::Wait(Duration::from_secs(20))
        );
        queue.leave_owner(1);
        queue.enqueue_owner(1, 7);
        assert_eq!(queue.status_owner(1).unwrap().position, 2);
        let later = now + Duration::from_secs(20);
        assert_eq!(queue.poll_owner(2, 8, later), Permit::Grant);
        assert!(queue.abandon_permit(2));
        assert_eq!(queue.poll_owner(1, 7, later), Permit::Grant);
        assert_eq!(queue.ip_count, 1);
    }

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

    fn ack(q: &mut LoginQueue, uid: i32, now: Instant) -> bool {
        q.acknowledge_login_return(test_owner(uid), now)
    }

    fn abandon(q: &mut LoginQueue, uid: i32) -> bool {
        q.abandon_permit(test_owner(uid))
    }

    /// Long enough that the accounting cases with logins still in flight
    /// never reach a lease end.
    const NO_LEASE_END: Duration = Duration::from_secs(3600);

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
                    assert!(ack(&mut q, uid, end));
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
                    assert!(q
                        .start_attempt(test_owner(uid), now, LoginCancel::new())
                        .is_some());
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
    fn default_starts_one_handshake_at_a_time_1200ms_apart() {
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
                gap >= LOGIN_START_SPACING,
                "starts {gap:?} apart; the default spaces them 1.2 s"
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
        assert!(ack(&mut q, 1, returned));
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
        assert!(ack(&mut q, 1, base));
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
        assert!(ack(&mut q, 2, returned));
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
        assert!(ack(&mut q, 1, base + TICK));
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
        assert!(ack(&mut q, 1, now));
        assert!(q.queued_uids().is_empty());
        assert!(matches!(request(&mut q, 2, now), Permit::Wait(_)));
        assert!(matches!(request(&mut q, 1, now), Permit::Wait(_)));
        assert_eq!(q.queued_uids(), vec![1, 2]);
        assert_eq!(
            request(&mut q, 1, now + Duration::from_secs(1)),
            Permit::Grant
        );
        assert!(ack(&mut q, 1, now + Duration::from_secs(1)));
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
        assert!(q.acknowledge_login_return(11, now));
        assert_eq!(
            q.status_owner(22),
            Some(QueuePos {
                position: 1,
                total: 1,
            })
        );
        assert_eq!(q.poll_owner(22, 7, later), Permit::Grant);
        assert!(q.acknowledge_login_return(22, later));
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
        assert!(q.acknowledge_login_return(22, now));
        assert_eq!(q.poll_owner(11, 1, later), Permit::Grant);
        assert!(q.acknowledge_login_return(11, later));
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
                assert!(ack(&mut q, i, now));
            }
            now += Duration::from_millis(1);
        }
        // Cap binds: the 30th grant must wait out the 60 s idle TTL.
        assert_eq!(grants.len(), 29);

        now = base + Duration::from_secs(61);
        for i in 29..50 {
            assert!(matches!(request(&mut q, i, now), Permit::Grant));
            grants.push(now);
            assert!(ack(&mut q, i, now));
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
        assert!(ack(&mut q, 1, base));
        assert_eq!(
            request(&mut q, 2, base + Duration::from_secs(30)),
            Permit::Grant
        );
        assert!(ack(&mut q, 2, base + Duration::from_secs(30)));

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
        assert!(ack(&mut q, 3, base + Duration::from_secs(90)));
        assert_eq!(
            request(&mut q, 4, base + Duration::from_secs(90)),
            Permit::Grant
        );
    }

    #[test]
    fn unreturned_login_counts_until_its_lease_ends_then_waits_the_idle_ttl() {
        let base = Instant::now();
        let mut q = unpaced(1);
        assert_eq!(request(&mut q, 1, base), Permit::Grant);
        assert!(q
            .start_attempt(test_owner(1), base, LoginCancel::new())
            .is_some());
        let lease_end = base + HANDSHAKE_HOLD + LEASE_SLACK;
        assert!(
            matches!(
                request(&mut q, 2, lease_end - Duration::from_millis(1)),
                Permit::Wait(_)
            ),
            "the attempt may still reach the server until its deadline"
        );
        assert_eq!(
            request(&mut q, 2, lease_end),
            Permit::Wait(Duration::from_secs(60)),
            "the spent budget's idle period runs from the lease end"
        );
        assert_eq!(
            request(&mut q, 2, lease_end + Duration::from_secs(60)),
            Permit::Grant
        );
    }

    /// A login that never returns holds the only in-flight slot until its
    /// lease ends, not forever. At the lease end the queue shuts its socket
    /// before the next bot can start, and the late return cannot free the
    /// slot a second time. A login that returned in time is never cancelled.
    #[test]
    fn a_handshake_that_never_returns_is_cancelled_at_its_lease_end() {
        let base = Instant::now();
        let mut q = LoginQueue::default();
        assert_eq!(request(&mut q, 1, base), Permit::Grant);
        let hung = LoginCancel::new();
        assert_eq!(
            q.start_attempt(test_owner(1), base, hung.clone()),
            Some(base + HANDSHAKE_HOLD),
            "the client gets the handshake deadline"
        );
        let lease_end = base + HANDSHAKE_HOLD + LEASE_SLACK;
        let before = lease_end - Duration::from_millis(1);
        assert!(matches!(request(&mut q, 2, before), Permit::Wait(_)));
        assert_eq!(
            q.head_blocker(test_owner(2), before),
            Some(AdmitBlocker::Handshake)
        );
        assert!(!hung.is_cancelled(), "not before its lease end");
        assert_eq!(
            request(&mut q, 2, lease_end),
            Permit::Grant,
            "the next bot starts at the lease end"
        );
        assert!(
            hung.is_cancelled(),
            "the hung attempt's socket is shut by the poll that grants the next bot"
        );
        let next = LoginCancel::new();
        assert!(q
            .start_attempt(test_owner(2), lease_end, next.clone())
            .is_some());
        assert!(matches!(request(&mut q, 3, lease_end), Permit::Wait(_)));
        let late = lease_end + Duration::from_secs(5);
        assert!(
            !ack(&mut q, 1, late),
            "the expired attempt was counted at its lease end"
        );
        assert_eq!(
            q.head_blocker(test_owner(3), late),
            Some(AdmitBlocker::Handshake),
            "the late return does not free bot 2's slot"
        );
        assert!(ack(&mut q, 2, late));
        assert_eq!(
            request(&mut q, 3, late + HANDSHAKE_HOLD + LEASE_SLACK),
            Permit::Grant,
            "a poll past bot 2's lease end"
        );
        assert!(
            !next.is_cancelled(),
            "a returned attempt is never cancelled"
        );
    }

    /// A worker stalled between its grant and its connect loses the grant at
    /// the lease end and must queue again instead of starting late.
    #[test]
    fn a_grant_never_started_expires_and_cannot_start_afterwards() {
        let base = Instant::now();
        let mut q = LoginQueue::default();
        assert_eq!(request(&mut q, 1, base), Permit::Grant);
        let lease_end = base + HANDSHAKE_HOLD + LEASE_SLACK;
        assert_eq!(request(&mut q, 2, lease_end), Permit::Grant);
        assert_eq!(
            q.start_attempt(test_owner(1), lease_end, LoginCancel::new()),
            None
        );
        assert!(!abandon(&mut q, 1));
    }

    /// Spacing counts from when each attempt really starts: a worker that
    /// starts two seconds after its grant and fails at once does not let the
    /// next attempt start right behind it.
    #[test]
    fn spacing_counts_from_the_attempt_start_not_the_grant() {
        let base = Instant::now();
        let mut q = LoginQueue::default();
        assert_eq!(request(&mut q, 1, base), Permit::Grant);
        let started = base + Duration::from_secs(2);
        assert!(q
            .start_attempt(test_owner(1), started, LoginCancel::new())
            .is_some());
        let returned = started + Duration::from_millis(1);
        assert!(ack(&mut q, 1, returned));
        assert_eq!(
            request(&mut q, 2, returned),
            Permit::Wait(LOGIN_START_SPACING - Duration::from_millis(1))
        );
        assert_eq!(
            q.head_blocker(test_owner(2), returned),
            Some(AdmitBlocker::Spacing {
                until: started + LOGIN_START_SPACING
            })
        );
        assert_eq!(
            request(&mut q, 2, started + LOGIN_START_SPACING),
            Permit::Grant
        );
    }

    #[test]
    fn reordered_login_returns_keep_the_latest_monotonic_ack_clock() {
        let base = Instant::now();
        let mut q = unpaced(2)
            .with_max_in_flight(2)
            .with_handshake_hold(NO_LEASE_END);
        assert_eq!(request(&mut q, 1, base), Permit::Grant);
        assert_eq!(request(&mut q, 2, base), Permit::Grant);

        assert!(ack(&mut q, 2, base + Duration::from_secs(120)));
        assert!(ack(&mut q, 1, base + Duration::from_secs(61)));
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
        let mut q = unpaced(2)
            .with_max_in_flight(2)
            .with_handshake_hold(NO_LEASE_END);
        assert_eq!(request(&mut q, 1, base), Permit::Grant);
        assert_eq!(request(&mut q, 2, base), Permit::Grant);
        assert!(ack(&mut q, 1, base));

        assert!(
            matches!(
                request(&mut q, 3, base + Duration::from_secs(120)),
                Permit::Wait(_)
            ),
            "the pending attempt can still refresh all address accounting"
        );
        assert!(ack(&mut q, 2, base + Duration::from_secs(130)));
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
        assert!(abandon(&mut q, 1));
        assert!(!abandon(&mut q, 1), "cleanup is exactly once");
        assert_eq!(request(&mut q, 2, base), Permit::Grant);
    }

    #[test]
    fn abandoned_grant_does_not_hold_the_next_start() {
        let base = Instant::now();
        let mut q = LoginQueue::default();
        assert_eq!(request(&mut q, 1, base), Permit::Grant);
        assert!(abandon(&mut q, 1));
        assert_eq!(
            request(&mut q, 2, base),
            Permit::Grant,
            "no socket opened, so there is nothing to space from"
        );
        assert!(ack(&mut q, 2, base));
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

        assert!(ack(&mut q, 7, base + Duration::from_secs(10)));
        assert!(abandon(&mut q, 7));
        assert!(!abandon(&mut q, 7), "both pending grants were resolved");
        assert_eq!(
            request(&mut q, 8, base + Duration::from_secs(10)),
            Permit::Grant,
            "abandoning one duplicate must preserve only the acknowledged attempt"
        );
    }

    #[test]
    fn pending_uid_reservations_do_not_expire_before_login_returns() {
        let base = Instant::now();
        let mut q = unpaced(10)
            .with_max_in_flight(10)
            .with_handshake_hold(NO_LEASE_END);
        for _ in 0..UID_GRANT_CAP {
            assert_eq!(request(&mut q, 7, base), Permit::Grant);
        }
        assert!(matches!(
            request(&mut q, 7, base + Duration::from_secs(120)),
            Permit::Wait(_)
        ));

        for _ in 0..UID_GRANT_CAP {
            assert!(abandon(&mut q, 7));
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
            assert!(ack(&mut q, 7, base));
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
            assert!(ack(&mut q, 7, base));
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
            assert!(ack(&mut q, i, now));
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
            assert!(ack(&mut q, 7, now));
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
            assert!(ack(&mut q, 7, base));
        }
        let fresh = base + Duration::from_secs(100);
        assert_eq!(request(&mut q, 7, fresh), Permit::Grant);
        assert!(ack(&mut q, 7, fresh));
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
