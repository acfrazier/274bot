//! Rust-owned giver/receiver exchange for an already-open trade.
//!
//! Frozen FlaxRunner calls `driveActivePartnerTrade` after `Trade.request`.
//! JavaScript invokes caller callbacks and projects their decisions/metrics;
//! this module owns posted trade identity, offer-then-confirm sequencing,
//! settlement and cancellation. Sends reuse native trade ops. This is not a
//! copy of `drivePartnerTrade.ts` or PartnerTrade policy.

use crate::isolate_fb::SnapshotReader;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::time::{Duration, Instant};

/// Frozen wall-clock wait for the offer screen to become ready.
pub const TRADE_OFFER_WAIT_MS: u64 = 5_000;
/// Frozen wall-clock wait for confirm / close after accept.
pub const TRADE_CONFIRM_WAIT_MS: u64 = 8_000;

thread_local! {
    static RUNTIME: RefCell<ExchangeRuntime> = const { RefCell::new(ExchangeRuntime::new()) };
    static NATIVE_OBSERVATION: RefCell<NativeObservation> =
        const { RefCell::new(NativeObservation::new()) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Giver,
    Receiver,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    MissingPartner,
    Offer,
    Receive,
    WaitConfirm,
    WaitClose,
    Declining,
}

struct NativeObservation {
    ingame: bool,
    offer_open: bool,
    confirm_open: bool,
    partner: Option<String>,
    accept_id: i32,
    decline_id: i32,
    mine_len: usize,
}

impl NativeObservation {
    const fn new() -> Self {
        Self {
            ingame: false,
            offer_open: false,
            confirm_open: false,
            partner: None,
            accept_id: -1,
            decline_id: -1,
            mine_len: 0,
        }
    }

    fn update(&mut self, snap: &SnapshotReader<'_>) {
        if snap.has_ingame() {
            if !snap.ingame() {
                *self = Self::new();
                return;
            }
            self.ingame = true;
        }
        if snap.has_trade_offer_open() {
            self.offer_open = snap.trade_offer_open();
        }
        if snap.has_trade_confirm_open() {
            self.confirm_open = snap.trade_confirm_open();
        }
        if snap.has_trade_partner() {
            self.partner = snap
                .trade_partner()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string);
        }
        if snap.has_trade_accept_id() {
            self.accept_id = snap.trade_accept_id();
        }
        if snap.has_trade_decline_id() {
            self.decline_id = snap.trade_decline_id();
        }
        if snap.has_trade_mine() {
            self.mine_len = snap.trade_mine().len();
        }
    }
}

struct Probe<'a> {
    ingame: bool,
    offer_open: bool,
    confirm_open: bool,
    partner: Option<&'a str>,
    accept_id: i32,
    mine_len: usize,
}

impl Probe<'_> {
    fn active(&self) -> bool {
        self.offer_open || self.confirm_open
    }
}

#[derive(Clone)]
struct TheirRow {
    matched: bool,
    count: i32,
}

struct Projection {
    names: Vec<String>,
    metric: Option<i32>,
    my_offer_ready: Option<bool>,
    their: Vec<TheirRow>,
}

struct ExchangeRuntime {
    paused: bool,
    held: bool,
    frozen_at: Option<Instant>,
    token: u64,
    phase: Phase,
    role: Role,
    partners: Vec<String>,
    verify_giver_partner: bool,
    seen_partner: Option<String>,
    metric_before: i32,
    next_name: usize,
    deadline: Option<Instant>,
    decline_reason: String,
    nested_token: u64,
}

impl ExchangeRuntime {
    const fn new() -> Self {
        Self {
            paused: false,
            held: false,
            frozen_at: None,
            token: 0,
            phase: Phase::Idle,
            role: Role::Giver,
            partners: Vec::new(),
            verify_giver_partner: false,
            seen_partner: None,
            metric_before: 0,
            next_name: 0,
            deadline: None,
            decline_reason: String::new(),
            nested_token: 0,
        }
    }

    fn frozen(&self) -> bool {
        self.paused || self.held
    }

    fn now(&self) -> Instant {
        self.frozen_at.unwrap_or_else(Instant::now)
    }

    fn set_freeze(&mut self, paused: bool, held: bool) {
        let was_frozen = self.frozen();
        self.paused = paused;
        self.held = held;
        let frozen = self.frozen();
        if !was_frozen && frozen {
            self.frozen_at = Some(Instant::now());
        } else if was_frozen && !frozen {
            if let Some(at) = self.frozen_at.take() {
                if let Some(deadline) = self.deadline.as_mut() {
                    *deadline += Instant::now().saturating_duration_since(at);
                }
            }
        }
    }

    fn arm(&mut self, window: u64) {
        self.deadline = Some(self.now() + Duration::from_millis(window));
    }

    fn bound_reached(&self) -> bool {
        self.deadline.is_some_and(|deadline| self.now() >= deadline)
    }

    fn abort_runtime(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Idle;
        self.partners.clear();
        self.verify_giver_partner = false;
        self.seen_partner = None;
        self.metric_before = 0;
        self.next_name = 0;
        self.deadline = None;
        self.decline_reason.clear();
        self.nested_token = 0;
    }

    fn wait(&self) -> Value {
        json!({ "kind": "wait", "token": self.token })
    }

    fn with_token(&self, mut step: Value) -> Value {
        step["token"] = json!(self.token);
        step
    }
}

pub fn on_snapshot(snap: &SnapshotReader<'_>) {
    NATIVE_OBSERVATION.with(|obs| obs.borrow_mut().update(snap));
}

pub fn on_pause() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().held;
        rt.borrow_mut().set_freeze(true, held);
    });
}

pub fn on_resume() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().held;
        rt.borrow_mut().set_freeze(false, held);
    });
}

pub fn on_hold(held: bool) {
    RUNTIME.with(|rt| {
        let paused = rt.borrow().paused;
        rt.borrow_mut().set_freeze(paused, held);
    });
}

pub fn on_reset() {
    RUNTIME.with(|rt| rt.borrow_mut().abort_runtime());
    NATIVE_OBSERVATION.with(|obs| *obs.borrow_mut() = NativeObservation::new());
}

pub fn dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => begin(input),
        "next" => next(input),
        "decision" => decision(input),
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

fn as_i32(value: Option<&Value>) -> Option<i32> {
    value.and_then(|v| {
        v.as_i64()
            .or_else(|| {
                v.as_f64()
                    .and_then(|n| (n.is_finite() && n.fract() == 0.0).then_some(n as i64))
            })
            .and_then(|n| i32::try_from(n).ok())
    })
}

fn parse_names(input: &Value) -> Vec<String> {
    input
        .get("names")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_partners(input: &Value) -> Vec<String> {
    input
        .get("partners")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_their(input: &Value) -> Vec<TheirRow> {
    input
        .get("their")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| TheirRow {
                    matched: row.get("matched").and_then(Value::as_bool).unwrap_or(false),
                    count: as_i32(row.get("count")).unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_projection(input: &Value) -> Projection {
    Projection {
        names: parse_names(input),
        metric: as_i32(input.get("metric")),
        my_offer_ready: input.get("myOfferReady").and_then(Value::as_bool),
        their: parse_their(input),
    }
}

fn matched_count(their: &[TheirRow]) -> i32 {
    their
        .iter()
        .filter(|row| row.matched)
        .map(|row| row.count.max(0))
        .sum()
}

fn partner_allowed(partners: &[String], name: &str) -> bool {
    let have = name.trim();
    if have.is_empty() {
        return false;
    }
    partners
        .iter()
        .any(|want| want.trim().eq_ignore_ascii_case(have))
}

fn observe() -> (bool, bool, bool, Option<String>, i32, usize) {
    NATIVE_OBSERVATION.with(|o| {
        let o = o.borrow();
        (
            o.ingame,
            o.offer_open,
            o.confirm_open,
            o.partner.clone(),
            o.accept_id,
            o.mine_len,
        )
    })
}

fn with_probe<R>(f: impl FnOnce(&Probe<'_>) -> R) -> R {
    let obs = observe();
    let probe = Probe {
        ingame: obs.0,
        offer_open: obs.1,
        confirm_open: obs.2,
        partner: obs.3.as_deref(),
        accept_id: obs.4,
        mine_len: obs.5,
    };
    f(&probe)
}

fn begin(input: &Value) -> Value {
    let role = match input.get("role").and_then(Value::as_str).unwrap_or("") {
        "giver" => Role::Giver,
        "receiver" => Role::Receiver,
        _ => return json!({ "kind": "notImpl", "reason": "unknown role" }),
    };
    let partners = parse_partners(input);
    if partners.is_empty() {
        return json!({ "kind": "notImpl", "reason": "missing partners" });
    }
    let projection = parse_projection(input);
    let Some(metric_before) = projection.metric else {
        return json!({ "kind": "notImpl", "reason": "missing metric" });
    };
    with_probe(|probe| {
        if !probe.ingame {
            return json!({ "kind": "aborted", "reason": "not ingame" });
        }
        if !probe.active() {
            return json!({ "kind": "aborted", "reason": "not-active" });
        }
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            rt.abort_runtime();
            rt.role = role;
            rt.partners = partners;
            rt.verify_giver_partner = input
                .get("verifyGiverPartner")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            rt.metric_before = metric_before;
            step(&mut rt, probe, &projection)
        })
    })
}

fn next(input: &Value) -> Value {
    let token = input.get("token").and_then(Value::as_u64).unwrap_or(0);
    let projection = parse_projection(input);
    with_probe(|probe| {
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            if token != rt.token || rt.phase == Phase::Idle {
                return json!({ "kind": "aborted", "token": rt.token });
            }
            if rt.frozen() {
                return rt.wait();
            }
            if !probe.ingame {
                rt.abort_runtime();
                return json!({ "kind": "aborted", "reason": "not-ingame" });
            }
            step(&mut rt, probe, &projection)
        })
    })
}

fn decision(input: &Value) -> Value {
    let token = input.get("token").and_then(Value::as_u64).unwrap_or(0);
    let projection = parse_projection(input);
    with_probe(|probe| {
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            if token != rt.token || rt.phase == Phase::Idle {
                return json!({ "kind": "aborted", "token": rt.token });
            }
            if rt.frozen() {
                return rt.wait();
            }
            if !probe.ingame {
                rt.abort_runtime();
                return json!({ "kind": "aborted", "reason": "not-ingame" });
            }
            match rt.phase {
                Phase::MissingPartner => match input.get("missing").and_then(Value::as_str) {
                    Some("decline") => start_decline(&mut rt, "partner header timeout"),
                    _ => {
                        rt.phase = Phase::MissingPartner;
                        rt.wait()
                    }
                },
                Phase::Receive => {
                    let gate = input.get("receiverGate");
                    let ok = gate
                        .and_then(|v| v.get("ok"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    if ok {
                        accept_offer(&mut rt, probe)
                    } else {
                        let reason = gate
                            .and_then(|v| v.get("reason"))
                            .and_then(Value::as_str)
                            .unwrap_or("receiver refused");
                        start_decline(&mut rt, reason)
                    }
                }
                _ => step(&mut rt, probe, &projection),
            }
        })
    })
}

fn step(rt: &mut ExchangeRuntime, probe: &Probe<'_>, projection: &Projection) -> Value {
    if rt.phase == Phase::Declining {
        return pump_decline(rt, probe);
    }
    if !probe.active() {
        if rt.phase == Phase::WaitClose {
            return settle_close(rt, projection);
        }
        return finish_declined(rt, "no-progress");
    }
    if let Some(seen) = rt.seen_partner.as_deref() {
        match probe.partner {
            Some(have) if !have.eq_ignore_ascii_case(seen) => {
                return start_decline(rt, "stale-phase");
            }
            None => return start_decline(rt, "stale-phase"),
            Some(_) => {}
        }
    }
    if rt.phase == Phase::WaitClose {
        if probe.offer_open && !probe.confirm_open {
            return start_decline(rt, "stale-phase");
        }
        if rt.bound_reached() {
            return start_decline(rt, "no-progress");
        }
        return rt.wait();
    }
    if rt.phase == Phase::WaitConfirm {
        if probe.confirm_open {
            return accept_confirm(rt, probe);
        }
        if rt.bound_reached() {
            return start_decline(rt, "no-progress");
        }
        return rt.wait();
    }
    match header_gate(rt, probe) {
        Header::NeedHook => {
            rt.phase = Phase::MissingPartner;
            json!({
                "kind": "need_missing_partner",
                "token": rt.token,
                "status": "waitHeader",
            })
        }
        Header::Stranger => start_decline(rt, "stranger"),
        Header::Ok => {
            if probe.confirm_open {
                on_confirm(rt, probe, projection)
            } else {
                on_offer(rt, probe, projection)
            }
        }
    }
}

enum Header {
    Ok,
    NeedHook,
    Stranger,
}

fn header_gate(rt: &mut ExchangeRuntime, probe: &Probe<'_>) -> Header {
    match probe.partner {
        None => {
            if rt.role == Role::Giver && !rt.verify_giver_partner {
                Header::Ok
            } else {
                Header::NeedHook
            }
        }
        Some(name) => {
            if !partner_allowed(&rt.partners, name) {
                Header::Stranger
            } else {
                if rt.seen_partner.is_none() {
                    rt.seen_partner = Some(name.to_string());
                }
                Header::Ok
            }
        }
    }
}

fn on_offer(rt: &mut ExchangeRuntime, probe: &Probe<'_>, projection: &Projection) -> Value {
    match rt.role {
        Role::Giver => giver_offer(rt, probe, projection),
        Role::Receiver => receiver_offer(rt, probe, projection),
    }
}

fn giver_offer(rt: &mut ExchangeRuntime, probe: &Probe<'_>, projection: &Projection) -> Value {
    if projection.my_offer_ready == Some(true) {
        return accept_offer(rt, probe);
    }
    if rt.next_name < projection.names.len() {
        let name = projection.names[rt.next_name].clone();
        rt.next_name += 1;
        rt.phase = Phase::Offer;
        if rt.deadline.is_none() {
            rt.arm(TRADE_OFFER_WAIT_MS);
        }
        return offer_all(rt, &name);
    }
    if projection.names.is_empty() {
        return start_decline(rt, "nothing to offer");
    }
    rt.phase = Phase::Offer;
    if rt.deadline.is_none() {
        rt.arm(TRADE_OFFER_WAIT_MS);
    }
    if rt.bound_reached() {
        return start_decline(rt, "no-progress");
    }
    json!({
        "kind": "wait",
        "token": rt.token,
        "status": "offering",
    })
}

fn receiver_offer(rt: &mut ExchangeRuntime, probe: &Probe<'_>, projection: &Projection) -> Value {
    if probe.mine_len > 0 {
        return start_decline(rt, "unintended own offer");
    }
    let count = matched_count(&projection.their);
    if count <= 0 {
        rt.phase = Phase::Receive;
        if rt.deadline.is_none() {
            rt.arm(TRADE_OFFER_WAIT_MS);
        }
        if rt.bound_reached() {
            return start_decline(rt, "no-progress");
        }
        return json!({
            "kind": "wait",
            "token": rt.token,
            "status": "waitOffer",
        });
    }
    rt.phase = Phase::Receive;
    json!({
        "kind": "need_receiver_gate",
        "token": rt.token,
        "theirProductCount": count,
        "status": "accepting",
    })
}

fn on_confirm(rt: &mut ExchangeRuntime, probe: &Probe<'_>, projection: &Projection) -> Value {
    match rt.role {
        Role::Giver if projection.my_offer_ready == Some(false) => {
            rt.phase = Phase::Offer;
            if rt.deadline.is_none() {
                rt.arm(TRADE_OFFER_WAIT_MS);
            }
            if rt.bound_reached() {
                return start_decline(rt, "no-progress");
            }
            rt.wait()
        }
        Role::Receiver
            if matched_count(&projection.their) <= 0 && rt.phase != Phase::WaitConfirm =>
        {
            rt.phase = Phase::Receive;
            if rt.deadline.is_none() {
                rt.arm(TRADE_OFFER_WAIT_MS);
            }
            if rt.bound_reached() {
                return start_decline(rt, "no-progress");
            }
            rt.wait()
        }
        _ => accept_confirm(rt, probe),
    }
}

fn offer_all(rt: &mut ExchangeRuntime, name: &str) -> Value {
    let begin = crate::trade::dispatch(&json!({
        "op": "begin",
        "kind": "offerAll",
        "name": name,
        "n": 0,
    }));
    match begin.get("kind").and_then(Value::as_str) {
        Some("candidates") => select_unnoted(rt, &begin),
        Some("notImpl") => {
            let reason = begin
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("Trade.offerAll")
                .to_string();
            json!({ "kind": "notImpl", "reason": reason, "token": rt.token })
        }
        Some("done") if begin.get("result") == Some(&Value::Bool(false)) => {
            let reason = begin
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("offerAll failed");
            start_decline(rt, &map_offer_reason(reason))
        }
        _ => tagged_ops(rt, begin, "offering"),
    }
}

fn map_offer_reason(reason: &str) -> String {
    match reason {
        "no-match" | "noted" | "wrong-identity" | "no-identity" | "missing-screen" => {
            "offerAll failed".into()
        }
        other => other.to_string(),
    }
}

fn select_unnoted(rt: &mut ExchangeRuntime, begin: &Value) -> Value {
    let token = begin.get("token").and_then(Value::as_u64).unwrap_or(0);
    let Some(chosen) = begin
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|rows| {
            rows.iter().find(|row| {
                row.get("noted").and_then(Value::as_bool) != Some(true)
                    && as_i32(row.get("id")).is_some()
                    && as_i32(row.get("slot")).is_some()
            })
        })
    else {
        return start_decline(rt, "offerAll failed");
    };
    let selected = crate::trade::dispatch(&json!({
        "op": "select",
        "token": token,
        "id": chosen.get("id"),
        "slot": chosen.get("slot"),
    }));
    if selected.get("kind").and_then(Value::as_str) == Some("done")
        && selected.get("result") == Some(&Value::Bool(false))
    {
        let reason = selected
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("offerAll failed");
        return start_decline(rt, &map_offer_reason(reason));
    }
    tagged_ops(rt, selected, "offering")
}

fn accept_offer(rt: &mut ExchangeRuntime, probe: &Probe<'_>) -> Value {
    if probe.accept_id < 0 {
        return json!({
            "kind": "notImpl",
            "reason": "no-accept",
            "token": rt.token,
        });
    }
    let sent = crate::trade::dispatch(&json!({
        "op": "begin",
        "kind": "accept",
        "name": "",
        "n": 0,
    }));
    if sent.get("kind").and_then(Value::as_str) == Some("done")
        && sent.get("result") == Some(&Value::Bool(false))
    {
        return start_decline(rt, "no-accept");
    }
    rt.phase = Phase::WaitConfirm;
    rt.arm(TRADE_CONFIRM_WAIT_MS);
    tagged_ops(rt, sent, "accepting")
}

fn accept_confirm(rt: &mut ExchangeRuntime, probe: &Probe<'_>) -> Value {
    if !probe.confirm_open {
        return rt.wait();
    }
    if probe.accept_id < 0 {
        return json!({
            "kind": "notImpl",
            "reason": "no-accept",
            "token": rt.token,
        });
    }
    let sent = crate::trade::dispatch(&json!({
        "op": "begin",
        "kind": "accept",
        "name": "",
        "n": 0,
    }));
    if sent.get("kind").and_then(Value::as_str) == Some("done")
        && sent.get("result") == Some(&Value::Bool(false))
    {
        return start_decline(rt, "no-accept");
    }
    rt.phase = Phase::WaitClose;
    rt.arm(TRADE_CONFIRM_WAIT_MS);
    tagged_ops(rt, sent, "confirming")
}

fn tagged_ops(rt: &ExchangeRuntime, mut step: Value, status: &str) -> Value {
    if step.get("ops").is_none() {
        if step.get("kind").and_then(Value::as_str) == Some("wait") {
            return rt.with_token(json!({ "kind": "wait", "status": status }));
        }
        return rt.with_token(step);
    }
    step["kind"] = json!("ops");
    step["token"] = json!(rt.token);
    step["status"] = json!(status);
    step
}

fn start_decline(rt: &mut ExchangeRuntime, reason: &str) -> Value {
    rt.decline_reason = reason.to_string();
    rt.phase = Phase::Declining;
    let sent = crate::trade::dispatch(&json!({
        "op": "begin",
        "kind": "decline",
        "name": "",
        "n": 0,
    }));
    match sent.get("kind").and_then(Value::as_str) {
        Some("ops") => {
            rt.nested_token = sent.get("token").and_then(Value::as_u64).unwrap_or(0);
            tagged_ops(rt, sent, "declining")
        }
        Some("done") => finish_declined(rt, reason),
        Some("notImpl") => json!({
            "kind": "notImpl",
            "reason": sent.get("reason").and_then(Value::as_str).unwrap_or("Trade.decline"),
            "token": rt.token,
        }),
        _ => finish_declined(rt, reason),
    }
}

fn pump_decline(rt: &mut ExchangeRuntime, probe: &Probe<'_>) -> Value {
    if !probe.active() {
        return finish_declined(rt, &rt.decline_reason.clone());
    }
    let sent = crate::trade::dispatch(&json!({
        "op": "next",
        "token": rt.nested_token,
    }));
    match sent.get("kind").and_then(Value::as_str) {
        Some("ops") => tagged_ops(rt, sent, "declining"),
        Some("wait") => rt.wait(),
        Some("done") | Some("aborted") => finish_declined(rt, &rt.decline_reason.clone()),
        Some("notImpl") => json!({
            "kind": "notImpl",
            "reason": sent.get("reason").and_then(Value::as_str).unwrap_or("Trade.decline"),
            "token": rt.token,
        }),
        _ => finish_declined(rt, &rt.decline_reason.clone()),
    }
}

fn settle_close(rt: &mut ExchangeRuntime, projection: &Projection) -> Value {
    let Some(after) = projection.metric else {
        return finish_declined(rt, "no-progress");
    };
    let delta = after.saturating_sub(rt.metric_before);
    if delta == 0 {
        return finish_declined(rt, "no-progress");
    }
    let token = rt.token;
    rt.phase = Phase::Idle;
    rt.deadline = None;
    json!({
        "kind": "complete",
        "token": token,
        "delta": delta,
        "reason": "transferred",
    })
}

fn finish_declined(rt: &mut ExchangeRuntime, reason: &str) -> Value {
    let token = rt.token;
    let reason = reason.to_string();
    rt.phase = Phase::Idle;
    rt.deadline = None;
    json!({
        "kind": "declined",
        "token": token,
        "reason": reason,
        "result": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_deadlines_match_the_drive_contract() {
        assert_eq!(TRADE_OFFER_WAIT_MS, 5_000);
        assert_eq!(TRADE_CONFIRM_WAIT_MS, 8_000);
    }

    #[test]
    fn exact_counterpart_is_case_insensitive_and_rejects_strangers() {
        let partners = vec!["Spinner".into()];
        assert!(partner_allowed(&partners, "spinner"));
        assert!(partner_allowed(&partners, " Spinner "));
        assert!(!partner_allowed(&partners, "runner"));
        assert!(!partner_allowed(&partners, ""));
        assert!(!partner_allowed(&[], "spinner"));
    }

    #[test]
    fn pause_and_hold_freeze_the_offer_deadline() {
        let mut rt = ExchangeRuntime::new();
        rt.arm(TRADE_OFFER_WAIT_MS);
        let before = rt.deadline.expect("armed");
        rt.set_freeze(true, false);
        assert!(rt.frozen());
        std::thread::sleep(Duration::from_millis(5));
        rt.set_freeze(false, false);
        assert!(!rt.frozen());
        assert!(rt.deadline.expect("still armed") > before);
        rt.set_freeze(false, true);
        assert!(rt.frozen(), "guardian hold freezes too");
    }

    #[test]
    fn abort_runtime_bumps_the_token_and_drops_pending_state() {
        let mut rt = ExchangeRuntime::new();
        rt.phase = Phase::WaitClose;
        rt.metric_before = 24;
        rt.arm(TRADE_CONFIRM_WAIT_MS);
        let before = rt.token;
        rt.abort_runtime();
        assert_eq!(rt.token, before.wrapping_add(1));
        assert_eq!(rt.phase, Phase::Idle);
        assert_eq!(rt.metric_before, 0);
        assert!(rt.deadline.is_none());
        assert!(rt.seen_partner.is_none());
    }

    #[test]
    fn matched_count_sums_only_projected_matches() {
        let rows = vec![
            TheirRow {
                matched: true,
                count: 24,
            },
            TheirRow {
                matched: false,
                count: 5,
            },
        ];
        assert_eq!(matched_count(&rows), 24);
        assert_eq!(matched_count(&[]), 0);
    }
}
