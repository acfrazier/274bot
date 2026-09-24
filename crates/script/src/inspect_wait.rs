//! Isolate-owned matching for inspect-route waiters.
//!
//! The host publishes a 2-deep inspect ring plus membership and explicit
//! accept/replace ids. This module has no running/pending index. Begin never
//! stales an existing waiter. Membership-stale requires positive host evidence
//! that *that* request was replaced (`replaced_id` / `replaced_prev_id`). A
//! later unrelated snapshot is not acknowledgment of a queued-but-not-drained
//! request.
//!
//! Token identity lives here: `inspectBegin` registers a waiter and the
//! canonical preview arguments. A public `inspect-route` reaches the host
//! only after this module authorizes the token. Invented or stale ids never
//! consume host jobs. `request_id` 0 is snapshot-only (no waiter). Typed
//! `InspectAck` is isolate-generated after apply; it is never taken from
//! `api.request`. Host `!can_admit` for a registered token posts that
//! identity in `refused_id{,_2,_3}` without touching the ring. Last-seen
//! `unobserved` may local-stale begin/authorize; it is not a reservation.

use crate::isolate_fb::{InspectHopReader, SnapshotReader};
use crate::shim::{InspectAvoidWire, InteractReq};
use crate::task_clock::InstantTaskClock;
use crate::walk_wait;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::VecDeque;

const UNSETTLED_MAX: usize = 3;
const SETTLED_MAX: usize = 2;

thread_local! {
    static SLOT: RefCell<InspectSlot> = const { RefCell::new(InspectSlot::new()) };
}

#[derive(Clone, Debug, PartialEq)]
struct InspectHop {
    kind: String,
    loc_id: i32,
    loc_name: String,
    action: String,
    option: i32,
    from_x: i32,
    from_z: i32,
    from_level: i32,
    to_x: i32,
    to_z: i32,
    to_level: i32,
    ticks: i32,
}

#[derive(Clone, Debug, PartialEq)]
struct Terminal {
    ok: bool,
    reason: String,
    bank_planned: bool,
    ticks: f64,
    hops: Vec<InspectHop>,
    request_id: u64,
}

impl Terminal {
    fn stale() -> Self {
        Self {
            ok: false,
            reason: "stale".into(),
            bank_planned: false,
            ticks: 0.0,
            hops: Vec::new(),
            request_id: 0,
        }
    }

    fn invalid_args() -> Self {
        Self {
            ok: false,
            reason: "invalid-args".into(),
            bank_planned: false,
            ticks: 0.0,
            hops: Vec::new(),
            request_id: 0,
        }
    }

    fn waiter_timeout() -> Self {
        Self {
            ok: false,
            reason: "waiter-timeout".into(),
            bank_planned: false,
            ticks: 0.0,
            hops: Vec::new(),
            request_id: 0,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "ok": self.ok,
            "reason": self.reason,
            "bankPlanned": self.bank_planned,
            "ticks": self.ticks,
            "request_id": self.request_id,
            "hops": self.hops.iter().map(|h| json!({
                "kind": h.kind,
                "locId": h.loc_id,
                "locName": h.loc_name,
                "action": h.action,
                "option": h.option,
                "from": { "x": h.from_x, "z": h.from_z, "level": h.from_level },
                "to": { "x": h.to_x, "z": h.to_z, "level": h.to_level },
                "ticks": h.ticks,
            })).collect::<Vec<_>>(),
        })
    }
}

#[derive(Clone, Copy, Default)]
struct HostInspect {
    latest_seq: u64,
    latest_id: u64,
    latest_generation: u64,
    prev_seq: u64,
    prev_id: u64,
    prev_generation: u64,
    #[allow(dead_code)]
    running_id: u64,
    #[allow(dead_code)]
    pending_id: u64,
    #[allow(dead_code)]
    accepted_id: u64,
    replaced_id: u64,
    replaced_prev_id: u64,
    refused_id: u64,
    refused_id_2: u64,
    refused_id_3: u64,
    unobserved: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RouteSpec {
    from: (i32, i32, i32),
    to: (i32, i32, i32),
    allow_teleports: bool,
    allow_wilderness: bool,
    allow_bank_fetch: bool,
    avoid: Vec<(i32, i32, i32, i32, Option<i32>)>,
}

impl RouteSpec {
    fn from_begin(input: &Value) -> Self {
        let opts = input.get("opts").unwrap_or(&Value::Null);
        let from = input.get("from").or_else(|| opts.get("from"));
        let to = input.get("to").or_else(|| opts.get("to"));
        let avoid = input
            .get("avoid")
            .or_else(|| opts.get("avoid"))
            .or_else(|| opts.get("avoidZones"));
        Self {
            from: tile_xyz(from),
            to: tile_xyz(to),
            allow_teleports: json_bool(
                input
                    .get("allow_teleports")
                    .or_else(|| opts.get("allow_teleports"))
                    .or_else(|| opts.get("useTeleportCatalog"))
                    .or_else(|| opts.get("policy").and_then(|p| p.get("useTeleports"))),
            ),
            allow_wilderness: json_bool(
                input
                    .get("allow_wilderness")
                    .or_else(|| opts.get("allow_wilderness")),
            ),
            allow_bank_fetch: json_bool(
                input
                    .get("allow_bank_fetch")
                    .or_else(|| opts.get("allow_bank_fetch")),
            ),
            avoid: avoid_spec(avoid),
        }
    }

    fn from_route(req: &InteractReq) -> Option<Self> {
        let InteractReq::InspectRoute {
            x,
            z,
            level,
            from_x,
            from_z,
            from_level,
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            avoid,
            ..
        } = req
        else {
            return None;
        };
        Some(Self {
            from: (*from_x, *from_z, *from_level),
            to: (*x, *z, *level),
            allow_teleports: *allow_teleports,
            allow_wilderness: *allow_wilderness,
            allow_bank_fetch: *allow_bank_fetch,
            avoid: avoid
                .iter()
                .map(|zone| match zone {
                    InspectAvoidWire::Rect {
                        min_x,
                        max_x,
                        min_z,
                        max_z,
                        level,
                    } => (*min_x, *max_x, *min_z, *max_z, *level),
                    InspectAvoidWire::Unsupported => (1, 0, 0, 0, None),
                })
                .collect(),
        })
    }
}

struct Waiter {
    token: u64,
    seq_at_begin: u64,
    clock: InstantTaskClock,
    terminal: Option<Terminal>,
    spec: RouteSpec,
    queued: bool,
}

struct PendingAck {
    seq: u64,
    generation: u64,
}

struct InspectSlot {
    host: HostInspect,
    applied_seq: u64,
    pending_ack: Option<PendingAck>,
    paused: bool,
    held: bool,
    unsettled: Vec<Waiter>,
    settled: VecDeque<Waiter>,
}

impl InspectSlot {
    const fn new() -> Self {
        Self {
            host: HostInspect {
                latest_seq: 0,
                latest_id: 0,
                latest_generation: 0,
                prev_seq: 0,
                prev_id: 0,
                prev_generation: 0,
                running_id: 0,
                pending_id: 0,
                accepted_id: 0,
                replaced_id: 0,
                replaced_prev_id: 0,
                refused_id: 0,
                refused_id_2: 0,
                refused_id_3: 0,
                unobserved: 0,
            },
            applied_seq: 0,
            pending_ack: None,
            paused: false,
            held: false,
            unsettled: Vec::new(),
            settled: VecDeque::new(),
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn set_freeze(&mut self, paused: bool, held: bool) {
        self.paused = paused;
        self.held = held;
        for waiter in &mut self.unsettled {
            waiter.clock.set_freeze(paused, held);
        }
    }

    fn push_settled(&mut self, waiter: Waiter) {
        if self.settled.len() == SETTLED_MAX {
            self.settled.pop_front();
        }
        self.settled.push_back(waiter);
    }

    fn observe(&mut self, snap: &SnapshotReader<'_>) {
        let before = self.applied_seq;
        self.host = HostInspect {
            latest_seq: snap.route_inspect_seq(),
            latest_id: snap.route_inspect_request_id(),
            latest_generation: snap.route_inspect_generation(),
            prev_seq: snap.route_inspect_prev_seq(),
            prev_id: snap.route_inspect_prev_request_id(),
            prev_generation: snap.route_inspect_prev_generation(),
            running_id: snap.route_inspect_running_id(),
            pending_id: snap.route_inspect_pending_id(),
            accepted_id: snap.route_inspect_accepted_id(),
            replaced_id: snap.route_inspect_replaced_id(),
            replaced_prev_id: snap.route_inspect_replaced_prev_id(),
            refused_id: snap.route_inspect_refused_id(),
            refused_id_2: snap.route_inspect_refused_id_2(),
            refused_id_3: snap.route_inspect_refused_id_3(),
            unobserved: snap.route_inspect_unobserved(),
        };
        if self.host.latest_seq != 0 {
            self.applied_seq = self.applied_seq.max(self.host.latest_seq);
        }
        if self.host.prev_seq != 0 {
            self.applied_seq = self.applied_seq.max(self.host.prev_seq);
        }
        if self.applied_seq > before {
            let generation = if self.host.latest_seq != 0 {
                self.host.latest_generation
            } else {
                self.host.prev_generation
            };
            self.pending_ack = Some(PendingAck {
                seq: self.applied_seq,
                generation,
            });
        }
        self.apply_published(snap);
        self.apply_replaced();
        self.apply_refused();
    }

    fn apply_published(&mut self, snap: &SnapshotReader<'_>) {
        let latest = published_terminal(
            snap.route_inspect_seq(),
            snap.route_inspect_request_id(),
            snap.route_inspect_ok(),
            snap.route_inspect_reason(),
            snap.route_inspect_bank_planned(),
            snap.route_inspect_ticks(),
            snap.route_inspect_hops(),
        );
        let prev = published_terminal(
            snap.route_inspect_prev_seq(),
            snap.route_inspect_prev_request_id(),
            snap.route_inspect_prev_ok(),
            snap.route_inspect_prev_reason(),
            snap.route_inspect_prev_bank_planned(),
            snap.route_inspect_prev_ticks(),
            snap.route_inspect_prev_hops(),
        );
        let mut i = 0;
        while i < self.unsettled.len() {
            let seq_at_begin = self.unsettled[i].seq_at_begin;
            let token = self.unsettled[i].token;
            let matched = [latest.as_ref(), prev.as_ref()]
                .into_iter()
                .flatten()
                .find(|term| {
                    term.request_id == token
                        && term.request_id != 0
                        && published_seq_for(snap, term.request_id) != 0
                        && published_seq_for(snap, term.request_id) != seq_at_begin
                })
                .cloned();
            if let Some(term) = matched {
                let mut waiter = self.unsettled.remove(i);
                waiter.terminal = Some(term);
                self.push_settled(waiter);
            } else {
                i += 1;
            }
        }
    }

    fn apply_replaced(&mut self) {
        let replaced = [self.host.replaced_id, self.host.replaced_prev_id];
        self.settle_matching(&replaced);
    }

    fn apply_refused(&mut self) {
        let refused = [
            self.host.refused_id,
            self.host.refused_id_2,
            self.host.refused_id_3,
        ];
        self.settle_matching(&refused);
    }

    fn settle_matching(&mut self, ids: &[u64]) {
        let mut i = 0;
        while i < self.unsettled.len() {
            let token = self.unsettled[i].token;
            if token != 0 && ids.contains(&token) {
                let mut waiter = self.unsettled.remove(i);
                waiter.terminal = Some(Terminal {
                    request_id: token,
                    ..Terminal::stale()
                });
                self.push_settled(waiter);
            } else {
                i += 1;
            }
        }
    }

    fn settle_stale_now(&mut self, spec: RouteSpec) -> u64 {
        let token = self.alloc();
        self.push_settled(Waiter {
            token,
            seq_at_begin: self.host.latest_seq,
            clock: InstantTaskClock::new(),
            terminal: Some(Terminal {
                request_id: token,
                ..Terminal::stale()
            }),
            spec,
            queued: false,
        });
        token
    }

    fn begin(&mut self, input: &Value) -> u64 {
        let spec = RouteSpec::from_begin(input);
        if begin_invalid(input) {
            let token = self.alloc();
            self.push_settled(Waiter {
                token,
                seq_at_begin: self.host.latest_seq,
                clock: InstantTaskClock::new(),
                terminal: Some(Terminal {
                    request_id: token,
                    ..Terminal::invalid_args()
                }),
                spec,
                queued: false,
            });
            return token;
        }
        if self.unsettled.len() >= UNSETTLED_MAX {
            return self.settle_stale_now(spec);
        }
        // Last-seen host fullness. Not a reservation: authorize can still
        // race a later host drain, which then posts refused_id.
        if self.host.unobserved >= UNSETTLED_MAX as u64 {
            return self.settle_stale_now(spec);
        }
        let token = self.alloc();
        let mut clock = InstantTaskClock::new();
        clock.set_freeze(self.paused, self.held);
        if let Some(ms) = input.get("timeout_ms").and_then(Value::as_u64) {
            clock.arm(ms);
        }
        self.unsettled.push(Waiter {
            token,
            seq_at_begin: self.host.latest_seq,
            clock,
            terminal: None,
            spec,
            queued: false,
        });
        token
    }

    fn settle_invalid(&mut self, token: u64) {
        if let Some(pos) = self.unsettled.iter().position(|w| w.token == token) {
            let mut waiter = self.unsettled.remove(pos);
            waiter.terminal = Some(Terminal {
                request_id: token,
                ..Terminal::invalid_args()
            });
            self.push_settled(waiter);
        }
    }

    fn authorize_route(&mut self, req: &InteractReq) -> bool {
        let InteractReq::InspectRoute { request_id, .. } = req else {
            return true;
        };
        if route_args_invalid(req) {
            if *request_id == 0 {
                return true;
            }
            if self.unsettled.iter().any(|w| w.token == *request_id) {
                self.settle_invalid(*request_id);
            }
            return false;
        }
        if *request_id == 0 {
            return true;
        }
        let Some(spec) = RouteSpec::from_route(req) else {
            return false;
        };
        let Some(pos) = self.unsettled.iter().position(|w| w.token == *request_id) else {
            return false;
        };
        if self.unsettled[pos].queued {
            return false;
        }
        if self.unsettled[pos].spec != spec {
            self.settle_invalid(*request_id);
            return false;
        }
        if self.host.unobserved >= UNSETTLED_MAX as u64 {
            self.settle_stale(*request_id);
            return false;
        }
        self.unsettled[pos].queued = true;
        true
    }

    fn settle_stale(&mut self, token: u64) {
        if let Some(pos) = self.unsettled.iter().position(|w| w.token == token) {
            let mut waiter = self.unsettled.remove(pos);
            waiter.terminal = Some(Terminal {
                request_id: token,
                ..Terminal::stale()
            });
            self.push_settled(waiter);
        }
    }

    fn alloc(&self) -> u64 {
        let mut avoid = self.host.latest_id;
        loop {
            let token = walk_wait::alloc_token(avoid);
            if token != 0
                && token != self.host.latest_id
                && token != self.host.prev_id
                && token != self.host.refused_id
                && token != self.host.refused_id_2
                && token != self.host.refused_id_3
            {
                return token;
            }
            avoid = token;
        }
    }

    fn find_settled(&self, token: u64) -> Option<&Waiter> {
        self.settled.iter().find(|w| w.token == token)
    }

    fn find_unsettled_mut(&mut self, token: u64) -> Option<&mut Waiter> {
        self.unsettled.iter_mut().find(|w| w.token == token)
    }

    fn force_timeout(&mut self, token: u64) {
        if let Some(pos) = self.unsettled.iter().position(|w| w.token == token) {
            let mut waiter = self.unsettled.remove(pos);
            waiter.terminal = Some(Terminal {
                request_id: token,
                ..Terminal::waiter_timeout()
            });
            self.push_settled(waiter);
        }
    }

    fn poll(&mut self, token: u64) -> bool {
        if token == 0 {
            return false;
        }
        if self.find_settled(token).is_some() {
            return true;
        }
        if let Some(pos) = self.unsettled.iter().position(|w| w.token == token) {
            if self.unsettled[pos].clock.bound_reached() {
                let mut waiter = self.unsettled.remove(pos);
                waiter.terminal = Some(Terminal {
                    request_id: token,
                    ..Terminal::waiter_timeout()
                });
                self.push_settled(waiter);
                return true;
            }
            return false;
        }
        true
    }

    fn value(&mut self, token: u64) -> Value {
        if token == 0 {
            return Value::Null;
        }
        if let Some(waiter) = self.find_settled(token) {
            return waiter
                .terminal
                .as_ref()
                .map(Terminal::to_json)
                .unwrap_or_else(|| Terminal::stale().to_json());
        }
        if self.find_unsettled_mut(token).is_some() {
            return Value::Null;
        }
        Terminal {
            request_id: token,
            ..Terminal::stale()
        }
        .to_json()
    }
}

fn published_seq_for(snap: &SnapshotReader<'_>, request_id: u64) -> u64 {
    if snap.route_inspect_request_id() == request_id {
        return snap.route_inspect_seq();
    }
    if snap.route_inspect_prev_request_id() == request_id {
        return snap.route_inspect_prev_seq();
    }
    0
}

fn published_terminal(
    seq: u64,
    request_id: u64,
    ok: bool,
    reason: &str,
    bank_planned: bool,
    ticks: f64,
    hops: Vec<InspectHopReader<'_>>,
) -> Option<Terminal> {
    if seq == 0 || request_id == 0 {
        return None;
    }
    Some(Terminal {
        ok,
        reason: reason.to_string(),
        bank_planned,
        ticks,
        hops: hops.iter().map(hop_from_reader).collect(),
        request_id,
    })
}

fn hop_from_reader(h: &InspectHopReader<'_>) -> InspectHop {
    InspectHop {
        kind: h.kind().to_string(),
        loc_id: h.loc_id(),
        loc_name: h.loc_name().to_string(),
        action: h.action().to_string(),
        option: h.option(),
        from_x: h.from_x(),
        from_z: h.from_z(),
        from_level: h.from_level(),
        to_x: h.to_x(),
        to_z: h.to_z(),
        to_level: h.to_level(),
        ticks: h.ticks(),
    }
}

fn begin_invalid(input: &Value) -> bool {
    let opts = input.get("opts").unwrap_or(&Value::Null);
    if opts.get("avoidDoors").is_some()
        || opts.get("maxExpansions").is_some()
        || opts.get("state").is_some()
    {
        return true;
    }
    if let Some(policy) = opts.get("policy") {
        if policy.get("distanceBeforeTeleport").is_some()
            || policy.get("allowTeleportIds").is_some()
            || policy.get("denyTeleportIds").is_some()
            || policy.get("useShips").is_some()
            || policy.get("useShortcuts").is_some()
        {
            return true;
        }
    }
    let zones = opts
        .get("avoidZones")
        .or_else(|| input.get("avoid"))
        .and_then(Value::as_array);
    if let Some(zones) = zones {
        if zones.len() > 16 {
            return true;
        }
        for zone in zones {
            if zone.as_str().is_some() {
                return true;
            }
            if !zone.is_object() {
                return true;
            }
            let min_x = json_i32(zone.get("minX").or_else(|| zone.get("min_x")));
            let max_x = json_i32(zone.get("maxX").or_else(|| zone.get("max_x")));
            let min_z = json_i32(zone.get("minZ").or_else(|| zone.get("min_z")));
            let max_z = json_i32(zone.get("maxZ").or_else(|| zone.get("max_z")));
            if min_x > max_x || min_z > max_z {
                return true;
            }
            if let Some(level) = zone.get("level") {
                if !level.is_null() {
                    let level = json_i32(Some(level));
                    if !(0..=3).contains(&level) {
                        return true;
                    }
                }
            }
        }
    }
    if let Some(from) = input.get("from") {
        if !tile_ok(from) {
            return true;
        }
    }
    if let Some(to) = input.get("to") {
        if !tile_ok(to) {
            return true;
        }
    }
    false
}

fn tile_ok(tile: &Value) -> bool {
    let level = json_i32(tile.get("level"));
    (0..=3).contains(&level)
}

fn json_i32(value: Option<&Value>) -> i32 {
    value
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .unwrap_or(0)
}

fn json_u64(value: Option<&Value>) -> u64 {
    value
        .and_then(Value::as_u64)
        .or_else(|| {
            value
                .and_then(Value::as_i64)
                .and_then(|n| u64::try_from(n).ok())
        })
        .unwrap_or(0)
}

fn json_bool(value: Option<&Value>) -> bool {
    value.and_then(Value::as_bool).unwrap_or(false)
}

fn tile_xyz(tile: Option<&Value>) -> (i32, i32, i32) {
    let Some(tile) = tile else {
        return (0, 0, 0);
    };
    (
        json_i32(tile.get("x")),
        json_i32(tile.get("z")),
        json_i32(tile.get("level")),
    )
}

fn avoid_spec(value: Option<&Value>) -> Vec<(i32, i32, i32, i32, Option<i32>)> {
    let Some(zones) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    zones
        .iter()
        .map(|zone| {
            let min_x = json_i32(zone.get("minX").or_else(|| zone.get("min_x")));
            let max_x = json_i32(zone.get("maxX").or_else(|| zone.get("max_x")));
            let min_z = json_i32(zone.get("minZ").or_else(|| zone.get("min_z")));
            let max_z = json_i32(zone.get("maxZ").or_else(|| zone.get("max_z")));
            let level = zone.get("level").and_then(|level| {
                if level.is_null() {
                    None
                } else {
                    Some(json_i32(Some(level)))
                }
            });
            (min_x, max_x, min_z, max_z, level)
        })
        .collect()
}

fn route_args_invalid(req: &InteractReq) -> bool {
    let InteractReq::InspectRoute {
        level,
        from_level,
        avoid,
        ..
    } = req
    else {
        return false;
    };
    if !(0..=3).contains(from_level) || !(0..=3).contains(level) {
        return true;
    }
    if avoid.len() > 16 {
        return true;
    }
    for zone in avoid {
        match zone {
            InspectAvoidWire::Unsupported => return true,
            InspectAvoidWire::Rect {
                min_x,
                max_x,
                min_z,
                max_z,
                level,
            } => {
                if min_x > max_x || min_z > max_z {
                    return true;
                }
                if let Some(level) = level {
                    if !(0..=3).contains(level) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Drop JS-forged `inspect-ack` and unauthorized `inspect-route` before the
/// isolate encodes the host FlatBuffer. Snapshot-only `request_id` 0 is
/// forwarded after the same argument checks; invented tokens never leave.
pub(crate) fn filter_public_inspect_wire(reqs: &mut Vec<InteractReq>) {
    SLOT.with(|slot| {
        let mut slot = slot.borrow_mut();
        reqs.retain(|req| match req {
            InteractReq::InspectAck { .. } => false,
            InteractReq::InspectRoute { .. } => slot.authorize_route(req),
            _ => true,
        });
    });
}

pub(crate) fn on_snapshot(snap: &SnapshotReader<'_>) {
    SLOT.with(|slot| slot.borrow_mut().observe(snap));
}

/// Consume-ack for the isolate snapshot path. `None` when applied_seq did
/// not advance. Separate from producer acceptance: posting a snapshot is
/// not this acknowledgment.
pub(crate) fn take_pending_ack() -> Option<(u64, u64)> {
    SLOT.with(|slot| {
        slot.borrow_mut()
            .pending_ack
            .take()
            .map(|ack| (ack.seq, ack.generation))
    })
}

pub(crate) fn on_reset() {
    SLOT.with(|slot| slot.borrow_mut().reset());
}

pub(crate) fn on_pause() {
    SLOT.with(|slot| {
        let held = slot.borrow().held;
        slot.borrow_mut().set_freeze(true, held);
    });
}

pub(crate) fn on_resume() {
    SLOT.with(|slot| {
        let held = slot.borrow().held;
        slot.borrow_mut().set_freeze(false, held);
    });
}

pub(crate) fn on_hold(held: bool) {
    SLOT.with(|slot| {
        let paused = slot.borrow().paused;
        slot.borrow_mut().set_freeze(paused, held);
    });
}

pub(crate) fn dispatch(input: &Value) -> Value {
    let op = input.get("op").and_then(Value::as_str).unwrap_or("");
    SLOT.with(|slot| {
        let mut slot = slot.borrow_mut();
        match op {
            "begin" => json!(slot.begin(input)),
            "settled" => json!(slot.poll(json_u64(input.get("token")))),
            "value" => {
                let token = json_u64(input.get("token"));
                if input.get("timed_out").and_then(Value::as_bool) == Some(true) {
                    slot.force_timeout(token);
                }
                slot.value(token)
            }
            "ack_seq" => json!(slot.applied_seq),
            _ => Value::Null,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isolate_fb::{
        encode_snapshot, encode_snapshot_with_native, InspectHopInput, NativeFactsInput,
        ReachViewInput, RouteInspectFactsInput, RouteInspectTerminalInput, SnapshotInput,
    };

    fn empty_input(tick: u64) -> SnapshotInput<'static> {
        SnapshotInput {
            tick,
            here: None,
            ingame: true,
            inv: &[],
            inv_size: 28,
            stats: &[],
            booths: &[],
            nearest_booth: None,
            banks: &[],
            bank: &[],
            bank_side: &[],
            bank_open: false,
            bank_loaded: false,
            bank_generation: 0,
            count_dialog_open: false,
            withdraw_x_result_seq: 0,
            withdraw_x_result: false,
            withdraw_load_result_seq: 0,
            withdraw_load_result: false,
            bank_op_result_seq: 0,
            bank_op_result: false,
            hold: false,
            ours: false,
            npcs: &[],
            locs: &[],
            players: &[],
            ground: &[],
            equipment: &[],
            chat_open: false,
            chat_continue: false,
            chat_text: None,
            chat_options: &[],
            side_tab: -1,
            varps: &[],
            combat_styles: &[],
            run_energy: 0,
            run_enabled: false,
            retaliate_enabled: false,
            my_name: None,
            in_combat: false,
            animating: false,
            main_modal_id: -1,
            chat_modal_id: -1,
            make_products: &[],
            side_tab_ifaces: &[],
            spell_buttons: &[],
            chat_lines: &[],
            bank_note_on: -1,
            bank_note_off: -1,
            scene_state: 0,
            weight: 0,
            combat_level: 0,
            camera_yaw: 0,
            camera_pitch: 0,
            teleports_enabled: false,
            self_slot: 0,
            trade_offer_open: false,
            trade_confirm_open: false,
            trade_partner: None,
            trade_mine: &[],
            trade_theirs: &[],
            trade_side: &[],
            trade_accept_id: -1,
            trade_decline_id: -1,
            shop_open: false,
            shop_stock: &[],
            reach: ReachViewInput::UNAVAILABLE,
            attacked_by_player: false,
            self_target_kind: 0,
            self_target_index: -1,
            widgets: &[],
        }
    }

    fn observe(input: SnapshotInput<'_>, native: NativeFactsInput<'_>) {
        let bytes = encode_snapshot_with_native(&input, native);
        let snap = SnapshotReader::from_bytes(&bytes).expect("snapshot");
        on_snapshot(&snap);
    }

    fn begin() -> u64 {
        dispatch(&json!({ "op": "begin" })).as_u64().expect("token")
    }

    fn settled(token: u64) -> bool {
        dispatch(&json!({ "op": "settled", "token": token }))
            .as_bool()
            .unwrap_or(false)
    }

    fn value(token: u64) -> Value {
        dispatch(&json!({ "op": "value", "token": token }))
    }

    fn hop<'a>(name: &'a str, loc_id: i32) -> InspectHopInput<'a> {
        InspectHopInput {
            kind: "boat",
            loc_id,
            loc_name: name,
            action: "Pay-fare",
            option: 1,
            from_x: 1,
            from_z: 2,
            from_level: 0,
            to_x: 3,
            to_z: 4,
            to_level: 1,
            ticks: 5,
        }
    }

    fn terminal<'a>(
        seq: u64,
        request_id: u64,
        ok: bool,
        reason: &'a str,
        hops: &'a [InspectHopInput<'a>],
    ) -> RouteInspectTerminalInput<'a> {
        RouteInspectTerminalInput {
            seq,
            generation: 1,
            request_id,
            ok,
            reason: Some(reason),
            bank_planned: false,
            ticks: if ok { 2.5 } else { 0.0 },
            hops,
        }
    }

    fn facts<'a>(
        latest: RouteInspectTerminalInput<'a>,
        prev: RouteInspectTerminalInput<'a>,
        running_id: u64,
        pending_id: u64,
        accepted_id: u64,
        replaced_id: u64,
        replaced_prev_id: u64,
    ) -> NativeFactsInput<'a> {
        facts_with(
            latest,
            prev,
            running_id,
            pending_id,
            accepted_id,
            replaced_id,
            replaced_prev_id,
            0,
            0,
            0,
            0,
        )
    }

    #[allow(clippy::too_many_arguments)] // native facts fixture packs route-inspect ids
    fn facts_with<'a>(
        latest: RouteInspectTerminalInput<'a>,
        prev: RouteInspectTerminalInput<'a>,
        running_id: u64,
        pending_id: u64,
        accepted_id: u64,
        replaced_id: u64,
        replaced_prev_id: u64,
        refused_id: u64,
        refused_id_2: u64,
        refused_id_3: u64,
        unobserved: u64,
    ) -> NativeFactsInput<'a> {
        NativeFactsInput {
            route_inspect: RouteInspectFactsInput {
                latest,
                prev,
                running_id,
                pending_id,
                accepted_id,
                replaced_id,
                replaced_prev_id,
                refused_id,
                refused_id_2,
                refused_id_3,
                unobserved,
            },
            ..Default::default()
        }
    }

    #[test]
    fn enqueue_before_drain_is_not_stale_from_unrelated_snapshot() {
        on_reset();
        let a = begin();
        observe(
            empty_input(1),
            facts(
                terminal(4, 99, true, "", &[]),
                RouteInspectTerminalInput::default(),
                0,
                0,
                99,
                0,
                0,
            ),
        );
        assert!(!settled(a), "queued-but-not-drained A is not acknowledged");
        assert_eq!(value(a), Value::Null);
    }

    #[test]
    fn promotion_race_begin_c_does_not_stale_running_b() {
        on_reset();
        let a = begin();
        let b = begin();
        let hop_a = hop("Captain Barnaby", 381);
        observe(
            empty_input(2),
            facts(
                terminal(1, a, true, "", std::slice::from_ref(&hop_a)),
                RouteInspectTerminalInput::default(),
                b,
                0,
                b,
                0,
                0,
            ),
        );
        assert!(settled(a));
        assert!(!settled(b));
        let c = begin();
        assert!(!settled(b), "begin(C) must not stale running B");
        let hop_b = hop("Seaman Thresnor", 378);
        observe(
            empty_input(3),
            facts(
                terminal(2, b, true, "", std::slice::from_ref(&hop_b)),
                terminal(1, a, true, "", std::slice::from_ref(&hop_a)),
                0,
                c,
                c,
                0,
                0,
            ),
        );
        assert!(settled(b));
        assert_eq!(value(b)["hops"][0]["locName"], "Seaman Thresnor");
        assert!(!settled(c));
    }

    #[test]
    fn host_pending_replace_stales_b_via_replaced_id() {
        on_reset();
        let a = begin();
        let b = begin();
        let c = begin();
        observe(
            empty_input(2),
            facts(
                terminal(1, b, false, "stale", &[]),
                RouteInspectTerminalInput::default(),
                a,
                c,
                c,
                b,
                0,
            ),
        );
        assert!(settled(b));
        assert_eq!(value(b)["reason"], "stale");
        assert!(!settled(a));
        assert!(!settled(c));
    }

    #[test]
    fn fast_complete_prev_ring_copies_a_and_does_not_mutate() {
        on_reset();
        let a = begin();
        let c = begin();
        let hop_a = hop("Captain Barnaby", 381);
        let hop_c = hop("Seaman Thresnor", 378);
        observe(
            empty_input(2),
            facts(
                terminal(2, c, true, "", std::slice::from_ref(&hop_c)),
                terminal(1, a, true, "", std::slice::from_ref(&hop_a)),
                0,
                0,
                c,
                0,
                0,
            ),
        );
        assert!(settled(a));
        assert_eq!(value(a)["hops"][0]["locName"], "Captain Barnaby");
        observe(
            empty_input(3),
            facts(
                terminal(3, 9, false, "NoPath", &[]),
                terminal(2, c, true, "", std::slice::from_ref(&hop_c)),
                0,
                0,
                9,
                0,
                0,
            ),
        );
        assert_eq!(
            value(a)["hops"][0]["locName"],
            "Captain Barnaby",
            "copied terminal must not mutate"
        );
        assert!(settled(c));
    }

    #[test]
    fn unknown_token_settles_stale_immediately_token_zero_never() {
        on_reset();
        assert!(!settled(0));
        assert_eq!(value(0), Value::Null);
        assert!(settled(9_001));
        assert_eq!(value(9_001)["reason"], "stale");
    }

    #[test]
    fn v2_caller_invented_id_is_immediate_stale() {
        on_reset();
        let invented = 4_242u64;
        assert!(settled(invented));
        assert_eq!(value(invented)["reason"], "stale");
    }

    #[test]
    fn fourth_begin_is_stale_without_queueing() {
        on_reset();
        let a = begin();
        let b = begin();
        let c = begin();
        let d = begin();
        assert!(!settled(a));
        assert!(!settled(b));
        assert!(!settled(c));
        assert!(settled(d));
        assert_eq!(value(d)["reason"], "stale");
    }

    #[test]
    fn leftover_seq_cannot_settle_new_wait() {
        on_reset();
        let leftover = hop("Captain Barnaby", 381);
        observe(
            empty_input(1),
            facts(
                terminal(4, 4, true, "", std::slice::from_ref(&leftover)),
                RouteInspectTerminalInput::default(),
                0,
                0,
                4,
                0,
                0,
            ),
        );
        let token = begin();
        assert!(!settled(token));
    }

    #[test]
    fn catalog_string_and_unsupported_opts_are_invalid_args() {
        on_reset();
        let token = dispatch(&json!({
            "op": "begin",
            "opts": { "avoidZones": ["wilderness"] }
        }))
        .as_u64()
        .unwrap();
        assert!(settled(token));
        assert_eq!(value(token)["reason"], "invalid-args");
        let doors = dispatch(&json!({
            "op": "begin",
            "opts": { "avoidDoors": true }
        }))
        .as_u64()
        .unwrap();
        assert!(settled(doors));
        assert_eq!(value(doors)["reason"], "invalid-args");
    }

    #[test]
    fn hold_does_not_advance_waiter_clock() {
        on_reset();
        let token = dispatch(&json!({ "op": "begin", "timeout_ms": 60_000 }))
            .as_u64()
            .unwrap();
        on_hold(true);
        assert!(
            !settled(token),
            "hold freezes InstantTaskClock; it does not abort search"
        );
    }

    #[test]
    fn omitted_inspect_fields_default_safe() {
        on_reset();
        let token = begin();
        let bytes = encode_snapshot(&empty_input(1));
        let snap = SnapshotReader::from_bytes(&bytes).expect("old snapshot");
        on_snapshot(&snap);
        assert_eq!(snap.route_inspect_seq(), 0);
        assert_eq!(snap.route_inspect_request_id(), 0);
        assert!(!settled(token));
    }

    #[test]
    fn walk_outcome_cannot_settle_inspect() {
        on_reset();
        let token = begin();
        observe(
            empty_input(2),
            NativeFactsInput {
                walk_outcome_seq: 1,
                walk_outcome_request_id: token,
                walk_outcome_failed: true,
                ..Default::default()
            },
        );
        assert!(!settled(token));
    }

    #[test]
    fn apply_advances_pending_ack_only_when_seq_grows() {
        on_reset();
        let hop_a = hop("Captain Barnaby", 381);
        observe(
            empty_input(1),
            facts(
                terminal(2, 11, true, "", std::slice::from_ref(&hop_a)),
                terminal(1, 10, true, "", &[]),
                0,
                0,
                11,
                0,
                0,
            ),
        );
        let first = take_pending_ack().expect("ack after apply");
        assert_eq!(first.0, 2);
        assert_eq!(first.1, 1);
        assert!(take_pending_ack().is_none());
        observe(
            empty_input(2),
            facts(
                terminal(2, 11, true, "", std::slice::from_ref(&hop_a)),
                terminal(1, 10, true, "", &[]),
                0,
                0,
                11,
                0,
                0,
            ),
        );
        assert!(
            take_pending_ack().is_none(),
            "same applied_seq must not re-ack"
        );
    }

    #[test]
    fn reset_stales_unsettled_and_does_not_reopen() {
        on_reset();
        let token = begin();
        on_reset();
        assert!(
            settled(token),
            "reset drops the matcher; leftover token is unknown-stale"
        );
        assert_eq!(value(token)["reason"], "stale");
    }

    fn route(request_id: u64, from: (i32, i32, i32), to: (i32, i32, i32)) -> InteractReq {
        InteractReq::InspectRoute {
            x: to.0,
            z: to.1,
            level: to.2,
            from_x: from.0,
            from_z: from.1,
            from_level: from.2,
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: false,
            avoid: Vec::new(),
            request_id,
        }
    }

    #[test]
    fn invented_token_is_dropped_before_host_queue() {
        on_reset();
        let mut reqs = vec![
            route(99, (1, 2, 0), (3, 4, 0)),
            InteractReq::InspectAck {
                seq: 4,
                generation: 1,
            },
        ];
        filter_public_inspect_wire(&mut reqs);
        assert!(reqs.is_empty(), "invented token and JS ACK never leave");
        assert!(settled(99));
        assert_eq!(value(99)["reason"], "stale");
    }

    #[test]
    fn registered_token_queues_once_and_rejects_mismatched_opts() {
        on_reset();
        let token = dispatch(&json!({
            "op": "begin",
            "from": { "x": 1, "z": 2, "level": 0 },
            "to": { "x": 3, "z": 4, "level": 0 },
            "allow_wilderness": true,
        }))
        .as_u64()
        .unwrap();
        let mut first = vec![route(token, (1, 2, 0), (3, 4, 0))];
        filter_public_inspect_wire(&mut first);
        assert_eq!(first.len(), 1);
        let mut dup = vec![route(token, (1, 2, 0), (3, 4, 0))];
        filter_public_inspect_wire(&mut dup);
        assert!(dup.is_empty(), "duplicate must not consume a second job");
        on_reset();
        let token = dispatch(&json!({
            "op": "begin",
            "from": { "x": 1, "z": 2, "level": 0 },
            "to": { "x": 3, "z": 4, "level": 0 },
            "allow_wilderness": true,
        }))
        .as_u64()
        .unwrap();
        let mut mismatch = vec![route(token, (9, 9, 0), (3, 4, 0))];
        filter_public_inspect_wire(&mut mismatch);
        assert!(mismatch.is_empty());
        assert!(settled(token));
        assert_eq!(value(token)["reason"], "invalid-args");
    }

    #[test]
    fn snapshot_only_zero_is_forwarded_without_waiter() {
        on_reset();
        let mut reqs = vec![route(0, (1, 2, 0), (3, 4, 0))];
        filter_public_inspect_wire(&mut reqs);
        assert_eq!(reqs.len(), 1);
        assert!(!settled(0));
        let mut bad = vec![InteractReq::InspectRoute {
            x: 1,
            z: 1,
            level: 9,
            from_x: 0,
            from_z: 0,
            from_level: 0,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            avoid: Vec::new(),
            request_id: 0,
        }];
        filter_public_inspect_wire(&mut bad);
        assert_eq!(bad.len(), 1, "id0 invalid-args still reaches host publish");
    }

    #[test]
    fn host_refused_ids_settle_registered_waiter_stale() {
        on_reset();
        let token = begin();
        observe(
            empty_input(1),
            facts_with(
                RouteInspectTerminalInput::default(),
                RouteInspectTerminalInput::default(),
                0,
                0,
                0,
                0,
                0,
                token,
                0,
                0,
                3,
            ),
        );
        assert!(settled(token));
        assert_eq!(value(token)["reason"], "stale");
        assert_ne!(value(token)["reason"], "waiter-timeout");
    }

    #[test]
    fn last_seen_unobserved_full_stales_begin_without_queue() {
        on_reset();
        observe(
            empty_input(1),
            facts_with(
                terminal(2, 11, false, "NoPath", &[]),
                terminal(1, 10, false, "NoPath", &[]),
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                3,
            ),
        );
        let token = begin();
        assert!(settled(token));
        assert_eq!(value(token)["reason"], "stale");
        let mut reqs = vec![route(token, (0, 0, 0), (1, 1, 0))];
        filter_public_inspect_wire(&mut reqs);
        assert!(reqs.is_empty(), "already-stale begin must not authorize");
    }
}
