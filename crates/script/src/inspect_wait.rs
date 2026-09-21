//! Isolate-owned matching for inspect-route waiters.
//!
//! The host publishes a 2-deep inspect ring plus membership and explicit
//! accept/replace ids. This module has no running/pending index. Begin never
//! stales an existing waiter. Membership-stale requires positive host evidence
//! that *that* request was replaced (`replaced_id` / `replaced_prev_id`). A
//! later unrelated snapshot is not acknowledgment of a queued-but-not-drained
//! request.

use crate::isolate_fb::{InspectHopReader, SnapshotReader};
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
    prev_seq: u64,
    prev_id: u64,
    #[allow(dead_code)]
    running_id: u64,
    #[allow(dead_code)]
    pending_id: u64,
    #[allow(dead_code)]
    accepted_id: u64,
    replaced_id: u64,
    replaced_prev_id: u64,
}

struct Waiter {
    token: u64,
    seq_at_begin: u64,
    clock: InstantTaskClock,
    terminal: Option<Terminal>,
}

struct InspectSlot {
    host: HostInspect,
    applied_seq: u64,
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
                prev_seq: 0,
                prev_id: 0,
                running_id: 0,
                pending_id: 0,
                accepted_id: 0,
                replaced_id: 0,
                replaced_prev_id: 0,
            },
            applied_seq: 0,
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
        self.host = HostInspect {
            latest_seq: snap.route_inspect_seq(),
            latest_id: snap.route_inspect_request_id(),
            prev_seq: snap.route_inspect_prev_seq(),
            prev_id: snap.route_inspect_prev_request_id(),
            running_id: snap.route_inspect_running_id(),
            pending_id: snap.route_inspect_pending_id(),
            accepted_id: snap.route_inspect_accepted_id(),
            replaced_id: snap.route_inspect_replaced_id(),
            replaced_prev_id: snap.route_inspect_replaced_prev_id(),
        };
        if self.host.latest_seq != 0 {
            self.applied_seq = self.applied_seq.max(self.host.latest_seq);
        }
        if self.host.prev_seq != 0 {
            self.applied_seq = self.applied_seq.max(self.host.prev_seq);
        }
        self.apply_published(snap);
        self.apply_replaced();
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
        let mut i = 0;
        while i < self.unsettled.len() {
            let token = self.unsettled[i].token;
            if token != 0 && replaced.contains(&token) {
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

    fn begin(&mut self, input: &Value) -> u64 {
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
            });
            return token;
        }
        if self.unsettled.len() >= UNSETTLED_MAX {
            let token = self.alloc();
            self.push_settled(Waiter {
                token,
                seq_at_begin: self.host.latest_seq,
                clock: InstantTaskClock::new(),
                terminal: Some(Terminal {
                    request_id: token,
                    ..Terminal::stale()
                }),
            });
            return token;
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
        });
        token
    }

    fn alloc(&self) -> u64 {
        let mut avoid = self.host.latest_id;
        loop {
            let token = walk_wait::alloc_token(avoid);
            if token != self.host.latest_id && token != self.host.prev_id && token != 0 {
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

pub(crate) fn on_snapshot(snap: &SnapshotReader<'_>) {
    SLOT.with(|slot| slot.borrow_mut().observe(snap));
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
            widgets: &[],
        }
    }

    fn observe(input: SnapshotInput<'_>, native: NativeFactsInput<'_>) {
        let bytes = encode_snapshot_with_native(&input, native);
        let snap = SnapshotReader::from_bytes(&bytes).expect("snapshot");
        on_snapshot(&snap);
    }

    fn begin() -> u64 {
        dispatch(&json!({ "op": "begin" }))
            .as_u64()
            .expect("token")
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
        NativeFactsInput {
            route_inspect: RouteInspectFactsInput {
                latest,
                prev,
                running_id,
                pending_id,
                accepted_id,
                replaced_id,
                replaced_prev_id,
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
}
