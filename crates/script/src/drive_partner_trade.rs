//! Rust-owned giver/receiver exchange for an already-open trade, the
//! `partner-trade` [`crate::machine`] family.
//!
//! Frozen FlaxRunner calls `driveActivePartnerTrade` after `Trade.request`.
//! JavaScript starts one machine with the caller's callbacks; this module
//! calls them in the frozen order (each tick: `productNamesToOffer`,
//! `inventoryMetric`, `myOfferReady`, then `theirProductMatch` per posted
//! row of their offer, read from the scene; then `setStatus`, the missing
//! partner / receiver gate decisions, and `onComplete` / `onDecline`). It
//! owns posted trade identity, offer-then-confirm sequencing, settlement
//! and cancellation. Sends reuse native trade ops. This is not a copy of
//! `drivePartnerTrade.ts` or PartnerTrade policy.

use crate::machine::{Begin, Call, Cx, Family, Reply, Step, Thrown};
use crate::observed;
use crate::trade::{self, Decline, Declining};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Frozen wall-clock wait for the offer screen to become ready.
pub const TRADE_OFFER_WAIT_MS: u64 = 5_000;
/// Frozen wall-clock wait for confirm / close after accept.
pub const TRADE_CONFIRM_WAIT_MS: u64 = 8_000;
/// Frozen continuous-inactive period required before accepting closure.
pub const TRADE_CLOSE_DEBOUNCE_MS: u64 = 600;

/// The caller's callbacks, by `hooks` key.
const NAMES: usize = 0;
const METRIC: usize = 1;
const READY: usize = 2;
const MATCH: usize = 3;
const SET_STATUS: usize = 4;
const ON_MISSING: usize = 5;
const RECEIVER_GATE: usize = 6;
const ON_COMPLETE: usize = 7;
const ON_DECLINE: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Role {
    Giver,
    Receiver,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Start,
    MissingPartner,
    Offer,
    Receive,
    WaitConfirm,
    WaitClose,
    Declining,
}

struct Probe<'a> {
    ingame: bool,
    offer_open: bool,
    confirm_open: bool,
    partner: Option<&'a str>,
    accept_id: i32,
    mine_len: usize,
    tick: u64,
}

impl Probe<'_> {
    fn active(&self) -> bool {
        self.offer_open || self.confirm_open
    }
}

#[derive(Clone)]
struct TheirRow {
    name: String,
    matched: bool,
    count: i32,
}

/// The caller's callback results for one exchange step.
#[derive(Default)]
struct Projection {
    names: Vec<String>,
    metric: Option<i32>,
    my_offer_ready: Option<bool>,
    their: Vec<TheirRow>,
}

/// What one exchange step decided (the old JS page kinds).
enum Page {
    /// Nothing more this tick; `status` is shown when set.
    Wait(Option<&'static str>),
    NeedMissingPartner,
    NeedReceiverGate(i32),
    Complete(i32),
    Declined(String),
    /// A missing host control: the call rejects `not impl`.
    NotImpl(&'static str),
    /// Not in game any more: the call resolves void.
    Aborted,
}

/// The decision a `need_*` page asked the caller for.
enum Decision {
    Missing { decline: bool },
    Gate(Result<(), String>),
}

/// Where the callback sequence is.
enum Stage {
    /// Next tick's projection starts from `productNamesToOffer`.
    Idle,
    /// Projecting for `then`; `their` rows get `theirProductMatch` in turn.
    Project {
        next: usize,
        projection: Projection,
        then: Option<Decision>,
    },
    /// `setStatus` was called for `page`.
    Status(Page),
    /// `onMissingPartner` was called.
    Missing,
    /// `receiverCanAccept(count)` was called.
    Gate,
    /// `onComplete` / `onDecline` was called: the call is over.
    Finished,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PartnerTradeArgs {
    #[serde(default)]
    role: Value,
    #[serde(default)]
    partners: Vec<String>,
    #[serde(default)]
    verify_giver_partner: bool,
    /// `productNamesToOffer` when it is a list, not a function.
    #[serde(default)]
    names: Option<Value>,
    #[serde(default)]
    labels: HashMap<String, Value>,
}

/// One frozen `driveActivePartnerTrade`.
pub(crate) struct PartnerTrade {
    stage: Stage,
    phase: Phase,
    role: Role,
    partners: Vec<String>,
    verify_giver_partner: bool,
    names: Option<Value>,
    labels: HashMap<String, Value>,
    seen_partner: Option<String>,
    /// `inventoryMetric` at the first step.
    metric_before: Option<i32>,
    next_name: usize,
    decline_reason: String,
    decline: Option<Decline>,
    inactive_since: Option<Instant>,
    settle_after_tick: Option<u64>,
}

impl Family for PartnerTrade {
    const NAME: &'static str = "partner-trade";
    /// One trade screen: a new drive replaces the one in flight.
    const EXCLUSIVE: bool = true;
    const CALLBACKS: &'static [&'static str] = &[
        "productNamesToOffer",
        "inventoryMetric",
        "myOfferReady",
        "theirProductMatch",
        "setStatus",
        "onMissingPartner",
        "receiverCanAccept",
        "onComplete",
        "onDecline",
    ];
    /// The first projection and exchange step run in the caller's tick.
    const KICK_ON_START: bool = true;
    type Args = PartnerTradeArgs;
    /// Always void: outcomes reach the caller through its callbacks.
    type Output = Value;

    fn begin(args: PartnerTradeArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
        let active = with_probe(|probe| probe.ingame && probe.active());
        if !active {
            return Begin::Done(Value::Null);
        }
        let role = match serde_json::from_value::<Role>(args.role) {
            Ok(role) => role,
            Err(_) => return Begin::Refuse("unknown role".into()),
        };
        let partners: Vec<String> = args
            .partners
            .iter()
            .map(|name| name.trim())
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect();
        if partners.is_empty() {
            return Begin::Refuse("missing partners".into());
        }
        Begin::Run(Self {
            stage: Stage::Idle,
            phase: Phase::Start,
            role,
            partners,
            verify_giver_partner: args.verify_giver_partner,
            names: args.names,
            labels: args.labels,
            seen_partner: None,
            metric_before: None,
            next_name: 0,
            decline_reason: String::new(),
            decline: None,
            inactive_since: None,
            settle_after_tick: None,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        let reply = cx.reply();
        match std::mem::replace(&mut self.stage, Stage::Idle) {
            Stage::Idle => {
                if !with_probe(|probe| probe.ingame) {
                    return Step::Done(Value::Null);
                }
                self.project(Projection::default(), 0, None, cx)
            }
            Stage::Project {
                next,
                mut projection,
                then,
            } => {
                if let Err(thrown) = self.take_projected(&mut projection, next, reply) {
                    return Step::Fail(thrown);
                }
                self.project(projection, next + 1, then, cx)
            }
            Stage::Status(page) => match reply {
                Some(Reply::Threw(thrown)) => Step::Fail(thrown),
                _ => self.follow(page, cx),
            },
            Stage::Missing => match reply {
                Some(Reply::Threw(thrown)) => Step::Fail(thrown),
                Some(Reply::Value(action)) => {
                    let decline = action.as_str() == Some("decline");
                    self.project(
                        Projection::default(),
                        0,
                        Some(Decision::Missing { decline }),
                        cx,
                    )
                }
                None => self.project(
                    Projection::default(),
                    0,
                    Some(Decision::Missing { decline: false }),
                    cx,
                ),
            },
            Stage::Gate => match reply {
                Some(Reply::Threw(thrown)) => Step::Fail(thrown),
                Some(Reply::Value(result)) => {
                    let gate = receiver_gate(&result);
                    self.project(Projection::default(), 0, Some(Decision::Gate(gate)), cx)
                }
                None => self.project(Projection::default(), 0, Some(Decision::Gate(Ok(()))), cx),
            },
            Stage::Finished => match reply {
                Some(Reply::Threw(thrown)) => Step::Fail(thrown),
                _ => Step::Done(Value::Null),
            },
        }
    }
}

/// The frozen `receiverCanAccept` verdict: `true` or `{ ok: true }` accept;
/// `false` is `receiver refused`; else the result's `reason`.
fn receiver_gate(result: &Value) -> Result<(), String> {
    if result == &Value::Bool(true) || result.get("ok") == Some(&Value::Bool(true)) {
        return Ok(());
    }
    if result == &Value::Bool(false) {
        return Err("receiver refused".into());
    }
    let reason = result.get("reason").filter(|reason| truthy(reason));
    Err(match reason {
        Some(Value::String(reason)) => reason.clone(),
        Some(reason) => reason.to_string(),
        None => "receiver refused".into(),
    })
}

/// JS truthiness of a callback's settled value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// Frozen `asNames`: a list's entries as non-empty strings, else none.
fn names_of(value: &Value) -> Vec<String> {
    let Value::Array(rows) = value else {
        return Vec::new();
    };
    rows.iter()
        .map(|name| match name {
            Value::Null => String::new(),
            Value::String(name) => name.clone(),
            other => other.to_string(),
        })
        .filter(|name| !name.is_empty())
        .collect()
}

/// Frozen `asMetric`: `Number(fn())` when finite, as a whole i32.
fn metric_of(value: &Value) -> Option<i32> {
    let n = match value {
        Value::Number(n) => n.as_f64()?,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Null => 0.0,
        Value::String(s) if s.trim().is_empty() => 0.0,
        Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    (n.is_finite() && n.fract() == 0.0)
        .then_some(n as i64)
        .and_then(|n| i32::try_from(n).ok())
}

fn matched_count(their: &[TheirRow]) -> i32 {
    their
        .iter()
        .filter(|row| row.matched)
        .map(|row| row.count.max(0))
        .sum()
}

fn partner_allowed(partners: &[String], name: &str) -> bool {
    crate::partner_trade::is_configured_partner(Some(name), partners)
}

/// The posted trade facts, read from the isolate scene. A logout forgets the
/// session: only pages posted since login count.
fn with_probe<R>(f: impl FnOnce(&Probe<'_>) -> R) -> R {
    observed::with(|scene| {
        let session = scene.since_login();
        let probe = Probe {
            ingame: session.ingame().unwrap_or(false),
            offer_open: session.trade_offer_open().unwrap_or(false),
            confirm_open: session.trade_confirm_open().unwrap_or(false),
            partner: session
                .trade_partner()
                .map(|name| name.trim())
                .filter(|name| !name.is_empty()),
            accept_id: session.trade_accept_id().unwrap_or(-1),
            mine_len: session.trade_mine().map_or(0, Vec::len),
            tick: scene.session_tick().unwrap_or(0),
        };
        f(&probe)
    })
}

/// Their posted offer rows: `(name, count)`.
fn their_rows() -> Vec<TheirRow> {
    observed::with(|scene| {
        scene
            .since_login()
            .trade_theirs()
            .map(|rows| {
                rows.iter()
                    .map(|row| TheirRow {
                        name: row.name_or_empty().to_string(),
                        matched: false,
                        count: row.count,
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn call(hook: usize, args: Vec<Value>) -> Step<Value> {
    Step::Call(Call { hook, args })
}

impl PartnerTrade {
    /// Projection call `index`: names, metric, ready, then one match per
    /// their row. Absent callbacks are skipped with the frozen defaults.
    fn project(
        &mut self,
        mut projection: Projection,
        mut index: usize,
        then: Option<Decision>,
        cx: &mut Cx<'_>,
    ) -> Step<Value> {
        loop {
            let hook = match index {
                0 if self.names.is_none() => NAMES,
                0 => {
                    projection.names = names_of(self.names.as_ref().unwrap_or(&Value::Null));
                    index += 1;
                    continue;
                }
                1 => METRIC,
                2 => READY,
                3 if projection.their.is_empty() => {
                    projection.their = their_rows();
                    if projection.their.is_empty() {
                        return self.exchange(projection, then, cx);
                    }
                    MATCH
                }
                i if i - 3 < projection.their.len() => MATCH,
                _ => return self.exchange(projection, then, cx),
            };
            if !cx.has(hook) {
                index += 1;
                continue;
            }
            let args = match hook {
                MATCH => vec![json!(projection.their[index - 3].name)],
                _ => Vec::new(),
            };
            self.stage = Stage::Project {
                next: index,
                projection,
                then,
            };
            return call(hook, args);
        }
    }

    /// Record the settled projection callback `index`.
    fn take_projected(
        &self,
        projection: &mut Projection,
        index: usize,
        reply: Option<Reply>,
    ) -> Result<(), Thrown> {
        let value = match reply {
            Some(Reply::Value(value)) => value,
            // A throwing `productNamesToOffer` offers nothing (frozen `asNames`).
            Some(Reply::Threw(_)) if index == 0 => Value::Null,
            Some(Reply::Threw(thrown)) => return Err(thrown),
            None => Value::Null,
        };
        match index {
            0 => projection.names = names_of(&value),
            1 => projection.metric = metric_of(&value),
            2 => projection.my_offer_ready = Some(truthy(&value)),
            i => projection.their[i - 3].matched = truthy(&value),
        }
        Ok(())
    }

    /// One exchange step (or decision) over a finished projection, then its
    /// status and follow-up.
    fn exchange(
        &mut self,
        projection: Projection,
        then: Option<Decision>,
        cx: &mut Cx<'_>,
    ) -> Step<Value> {
        if self.metric_before.is_none() {
            let Some(metric) = projection.metric else {
                return Step::Fail(Thrown::new(
                    "not impl: driveActivePartnerTrade: missing metric",
                ));
            };
            self.metric_before = Some(metric);
        }
        let page = with_probe(|probe| {
            if !probe.ingame {
                return Page::Aborted;
            }
            match then {
                Some(decision) => self.decide(decision, probe, &projection, cx),
                None => self.exchange_step(probe, &projection, cx),
            }
        });
        let status = match &page {
            Page::Wait(status) => *status,
            Page::NeedMissingPartner => Some("waitHeader"),
            Page::NeedReceiverGate(_) => Some("accepting"),
            _ => None,
        };
        if let Some(key) = status.filter(|_| cx.has(SET_STATUS)) {
            let label = match self.labels.get(key) {
                Some(Value::String(label)) if !label.is_empty() => label.clone(),
                Some(label) if truthy(label) => label.to_string(),
                _ => key.to_string(),
            };
            self.stage = Stage::Status(page);
            return call(SET_STATUS, vec![json!(label)]);
        }
        self.follow(page, cx)
    }

    /// After the status: the page's own callback, or the tick's end.
    fn follow(&mut self, page: Page, cx: &mut Cx<'_>) -> Step<Value> {
        match page {
            Page::Wait(_) => Step::Wait,
            Page::Aborted => Step::Done(Value::Null),
            Page::NotImpl(reason) => Step::Fail(Thrown::new(format!(
                "not impl: driveActivePartnerTrade: {reason}"
            ))),
            Page::NeedMissingPartner => {
                if cx.has(ON_MISSING) {
                    self.stage = Stage::Missing;
                    return call(ON_MISSING, Vec::new());
                }
                let then = Some(Decision::Missing { decline: false });
                self.project(Projection::default(), 0, then, cx)
            }
            Page::NeedReceiverGate(count) => {
                if cx.has(RECEIVER_GATE) {
                    self.stage = Stage::Gate;
                    return call(RECEIVER_GATE, vec![json!(count)]);
                }
                self.project(Projection::default(), 0, Some(Decision::Gate(Ok(()))), cx)
            }
            Page::Complete(delta) => self.finish(ON_COMPLETE, json!(delta), cx),
            Page::Declined(reason) => self.finish(ON_DECLINE, json!(reason), cx),
        }
    }

    fn finish(&mut self, hook: usize, arg: Value, cx: &mut Cx<'_>) -> Step<Value> {
        if !cx.has(hook) {
            return Step::Done(Value::Null);
        }
        self.stage = Stage::Finished;
        call(hook, vec![arg])
    }

    fn decide(
        &mut self,
        decision: Decision,
        probe: &Probe<'_>,
        projection: &Projection,
        cx: &mut Cx<'_>,
    ) -> Page {
        match (self.phase, decision) {
            (Phase::MissingPartner, Decision::Missing { decline: true }) => {
                self.start_decline("partner header timeout", cx)
            }
            (Phase::MissingPartner, Decision::Missing { .. }) => Page::Wait(None),
            (Phase::Receive, Decision::Gate(Ok(()))) => self.accept_offer(probe, cx),
            (Phase::Receive, Decision::Gate(Err(reason))) => self.start_decline(&reason, cx),
            _ => self.exchange_step(probe, projection, cx),
        }
    }

    fn now(cx: &mut Cx<'_>) -> Instant {
        cx.clock().now()
    }

    fn stable_closed(&self, cx: &mut Cx<'_>) -> bool {
        let now = Self::now(cx);
        self.inactive_since.is_some_and(|since| {
            now.saturating_duration_since(since) >= Duration::from_millis(TRADE_CLOSE_DEBOUNCE_MS)
        })
    }

    fn arm_once(&mut self, window: u64, cx: &mut Cx<'_>) {
        if cx.clock().deadline.is_none() {
            cx.clock().arm(window);
        }
    }

    fn exchange_step(
        &mut self,
        probe: &Probe<'_>,
        projection: &Projection,
        cx: &mut Cx<'_>,
    ) -> Page {
        if self.phase == Phase::Declining {
            return self.pump_decline(probe, cx);
        }
        if let Some(seen) = self.seen_partner.as_deref() {
            match probe.partner {
                Some(have) if !have.eq_ignore_ascii_case(seen) => {
                    return self.start_decline("stale-phase", cx);
                }
                None if probe.active() => return self.start_decline("stale-phase", cx),
                Some(_) | None => {}
            }
        }
        if probe.active() {
            self.inactive_since = None;
        } else if self.inactive_since.is_none() {
            self.inactive_since = Some(Self::now(cx));
        }
        if self.phase == Phase::WaitClose {
            if probe.offer_open && !probe.confirm_open {
                return self.start_decline("stale-phase", cx);
            }
            if probe.active() || !self.stable_closed(cx) {
                self.settle_after_tick = None;
                if cx.clock().bound_reached() {
                    return self.start_decline("no-progress", cx);
                }
                return Page::Wait(None);
            }
            if let Some(after_tick) = self.settle_after_tick {
                if cx.clock().bound_reached() {
                    return self.start_decline("no-progress", cx);
                }
                if probe.tick >= after_tick {
                    return self.settle_close(projection);
                }
                return Page::Wait(None);
            }
            if cx.clock().bound_reached() {
                return self.start_decline("no-progress", cx);
            }
            self.settle_after_tick = Some(probe.tick.saturating_add(1));
            return Page::Wait(None);
        }
        if self.phase == Phase::WaitConfirm {
            if probe.confirm_open {
                return self.accept_confirm(probe, cx);
            }
            if probe.active() || !self.stable_closed(cx) {
                if cx.clock().bound_reached() {
                    return self.start_decline("no-progress", cx);
                }
                return Page::Wait(None);
            }
            if cx.clock().bound_reached() {
                return self.start_decline("no-progress", cx);
            }
            return self.finish_declined("no-progress");
        }
        if !probe.active() {
            return self.finish_declined("no-progress");
        }
        match self.header_gate(probe) {
            Header::NeedHook => {
                self.phase = Phase::MissingPartner;
                Page::NeedMissingPartner
            }
            Header::Stranger => self.start_decline("stranger", cx),
            Header::Ok => {
                if probe.confirm_open {
                    self.on_confirm(probe, projection, cx)
                } else {
                    self.on_offer(probe, projection, cx)
                }
            }
        }
    }

    fn header_gate(&mut self, probe: &Probe<'_>) -> Header {
        match probe.partner {
            None => {
                if self.role == Role::Giver && !self.verify_giver_partner {
                    Header::Ok
                } else {
                    Header::NeedHook
                }
            }
            Some(name) => {
                if !partner_allowed(&self.partners, name) {
                    Header::Stranger
                } else {
                    if self.seen_partner.is_none() {
                        self.seen_partner = Some(name.to_string());
                    }
                    Header::Ok
                }
            }
        }
    }

    fn on_offer(&mut self, probe: &Probe<'_>, projection: &Projection, cx: &mut Cx<'_>) -> Page {
        match self.role {
            Role::Giver => self.giver_offer(probe, projection, cx),
            Role::Receiver => self.receiver_offer(probe, projection, cx),
        }
    }

    fn giver_offer(&mut self, probe: &Probe<'_>, projection: &Projection, cx: &mut Cx<'_>) -> Page {
        if projection.my_offer_ready == Some(true) {
            return self.accept_offer(probe, cx);
        }
        if self.next_name < projection.names.len() {
            let name = projection.names[self.next_name].clone();
            self.next_name += 1;
            self.phase = Phase::Offer;
            self.arm_once(TRADE_OFFER_WAIT_MS, cx);
            return self.offer_all(&name, cx);
        }
        if projection.names.is_empty() {
            return self.start_decline("nothing to offer", cx);
        }
        self.phase = Phase::Offer;
        self.arm_once(TRADE_OFFER_WAIT_MS, cx);
        if cx.clock().bound_reached() {
            return self.start_decline("no-progress", cx);
        }
        Page::Wait(Some("offering"))
    }

    fn receiver_offer(
        &mut self,
        probe: &Probe<'_>,
        projection: &Projection,
        cx: &mut Cx<'_>,
    ) -> Page {
        if probe.mine_len > 0 {
            return self.start_decline("unintended own offer", cx);
        }
        let count = matched_count(&projection.their);
        self.phase = Phase::Receive;
        if count <= 0 {
            self.arm_once(TRADE_OFFER_WAIT_MS, cx);
            if cx.clock().bound_reached() {
                return self.start_decline("no-progress", cx);
            }
            return Page::Wait(Some("waitOffer"));
        }
        Page::NeedReceiverGate(count)
    }

    fn on_confirm(&mut self, probe: &Probe<'_>, projection: &Projection, cx: &mut Cx<'_>) -> Page {
        match self.role {
            Role::Giver if projection.my_offer_ready == Some(false) => {
                self.phase = Phase::Offer;
                self.arm_once(TRADE_OFFER_WAIT_MS, cx);
                if cx.clock().bound_reached() {
                    return self.start_decline("no-progress", cx);
                }
                Page::Wait(None)
            }
            Role::Receiver
                if matched_count(&projection.their) <= 0 && self.phase != Phase::WaitConfirm =>
            {
                self.phase = Phase::Receive;
                self.arm_once(TRADE_OFFER_WAIT_MS, cx);
                if cx.clock().bound_reached() {
                    return self.start_decline("no-progress", cx);
                }
                Page::Wait(None)
            }
            _ => self.accept_confirm(probe, cx),
        }
    }

    fn offer_all(&mut self, name: &str, cx: &mut Cx<'_>) -> Page {
        match trade::offer_all_unnoted(name, cx) {
            Ok(()) => Page::Wait(Some("offering")),
            Err(reason) => self.start_decline(map_offer_reason(reason), cx),
        }
    }

    fn accept_offer(&mut self, probe: &Probe<'_>, cx: &mut Cx<'_>) -> Page {
        if probe.accept_id < 0 {
            return Page::NotImpl("no-accept");
        }
        if trade::accept(cx).is_err() {
            return self.start_decline("no-accept", cx);
        }
        self.phase = Phase::WaitConfirm;
        cx.clock().arm(TRADE_CONFIRM_WAIT_MS);
        Page::Wait(Some("accepting"))
    }

    fn accept_confirm(&mut self, probe: &Probe<'_>, cx: &mut Cx<'_>) -> Page {
        if !probe.confirm_open {
            return Page::Wait(None);
        }
        if probe.accept_id < 0 {
            return Page::NotImpl("no-accept");
        }
        if trade::accept(cx).is_err() {
            return self.start_decline("no-accept", cx);
        }
        self.phase = Phase::WaitClose;
        cx.clock().arm(TRADE_CONFIRM_WAIT_MS);
        Page::Wait(Some("confirming"))
    }

    fn start_decline(&mut self, reason: &str, cx: &mut Cx<'_>) -> Page {
        self.decline_reason = reason.to_string();
        self.phase = Phase::Declining;
        match Decline::begin(cx) {
            Some(decline) => {
                self.decline = Some(decline);
                Page::Wait(Some("declining"))
            }
            None => self.finish_declined(reason),
        }
    }

    fn pump_decline(&mut self, probe: &Probe<'_>, cx: &mut Cx<'_>) -> Page {
        let reason = self.decline_reason.clone();
        if !probe.active() {
            return self.finish_declined(&reason);
        }
        let Some(decline) = self.decline.as_mut() else {
            return self.finish_declined(&reason);
        };
        match decline.step(cx) {
            Declining::Wait => Page::Wait(None),
            Declining::Closing => Page::Wait(Some("declining")),
            Declining::Done(_) => self.finish_declined(&reason),
        }
    }

    fn settle_close(&mut self, projection: &Projection) -> Page {
        let (Some(after), Some(before)) = (projection.metric, self.metric_before) else {
            return self.finish_declined("no-progress");
        };
        let delta = after.saturating_sub(before);
        if delta == 0 {
            return self.finish_declined("no-progress");
        }
        Page::Complete(delta)
    }

    fn finish_declined(&mut self, reason: &str) -> Page {
        Page::Declined(reason.to_string())
    }
}

enum Header {
    Ok,
    NeedHook,
    Stranger,
}

fn map_offer_reason(reason: &str) -> &str {
    match reason {
        "no-match" | "noted" | "wrong-identity" | "no-identity" | "missing-screen" => {
            "offerAll failed"
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_deadlines_match_the_drive_contract() {
        assert_eq!(TRADE_OFFER_WAIT_MS, 5_000);
        assert_eq!(TRADE_CONFIRM_WAIT_MS, 8_000);
        assert_eq!(TRADE_CLOSE_DEBOUNCE_MS, 600);
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
    fn matched_count_sums_only_projected_matches() {
        let row = |matched, count| TheirRow {
            name: "Bow string".into(),
            matched,
            count,
        };
        assert_eq!(matched_count(&[row(true, 24), row(false, 5)]), 24);
        assert_eq!(matched_count(&[]), 0);
    }

    #[test]
    fn caller_results_keep_the_frozen_coercions() {
        assert_eq!(names_of(&json!(["Flax", "", null, 7])), vec!["Flax", "7"]);
        assert!(names_of(&json!("Flax")).is_empty());
        assert_eq!(metric_of(&json!(24)), Some(24));
        assert_eq!(metric_of(&json!("24")), Some(24));
        assert_eq!(metric_of(&json!(null)), Some(0), "Number(null) is 0");
        assert_eq!(metric_of(&json!("x")), None);
        assert_eq!(metric_of(&json!(2.5)), None);
        assert_eq!(receiver_gate(&json!(true)), Ok(()));
        assert_eq!(receiver_gate(&json!({ "ok": true })), Ok(()));
        assert_eq!(receiver_gate(&json!(false)), Err("receiver refused".into()));
        assert_eq!(
            receiver_gate(&json!({ "ok": false, "reason": "full" })),
            Err("full".into())
        );
        assert_eq!(receiver_gate(&json!(null)), Err("receiver refused".into()));
    }
}
