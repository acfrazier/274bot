//! `driveActivePartnerTrade`, the `partner-trade` [`crate::machine`]
//! family: one frozen `drivePartnerTrade.ts` iteration per call.
//!
//! Frozen FlaxRunner calls it from a task while `Trade.active()`; each
//! call handles the current screen once (confirm accept, the receiver's
//! header / partner / product / gate decision and accept, or the giver's
//! offer or accept) and returns, and the next call handles what follows.
//! This module ports that iteration branch for branch: the caller's
//! callbacks run in the frozen order, `setStatus` gets the frozen label or
//! `mule: …` default, and `log` gets the frozen lines. Callbacks are
//! frozen synchronous calls ([`Family::AWAIT_CALLBACKS`] is `false`): a
//! returned promise is used as a value, not awaited, and a throw rejects
//! the call. Their offer, my offer, the partner header and the screens are
//! read from the scene; sends reuse the native trade ops.

use crate::machine::{Begin, Call, Cx, Family, Reply, Step, Thrown};
use crate::observed;
use crate::trade::{self, Decline, Declining};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Frozen wall-clock wait for the offer screen to settle.
pub const TRADE_OFFER_WAIT_MS: u64 = 5_000;
/// Frozen wall-clock wait for confirm / close after accept.
pub const TRADE_CONFIRM_WAIT_MS: u64 = 8_000;
/// Frozen `stableClosedPoll` continuous-inactive period.
pub const TRADE_CLOSE_DEBOUNCE_MS: u64 = 600;

/// The caller's callbacks, by `hooks` key.
const NAMES: usize = 0;
const METRIC: usize = 1;
const READY: usize = 2;
const MATCH: usize = 3;
const SET_STATUS: usize = 4;
const LOG: usize = 5;
const ON_MISSING: usize = 6;
const GATE: usize = 7;
const ON_COMPLETE: usize = 8;
const ON_DECLINE: usize = 9;
const BASELINE: usize = 10;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PartnerTradeArgs {
    #[serde(default)]
    role: Value,
    #[serde(default)]
    partners: Value,
    #[serde(default)]
    verify_giver_partner: bool,
    #[serde(default)]
    labels: Value,
}

/// The posted trade facts this iteration reads, since login.
struct Screen {
    offer: bool,
    confirm: bool,
    partner: Option<String>,
    mine_len: usize,
    /// Their offer rows that have a name: `(name, count)`.
    theirs: Vec<(String, i32)>,
    /// Frozen `Inventory.used()`: named pack rows.
    used: i32,
}

impl Screen {
    fn read() -> Self {
        observed::with(|scene| {
            let session = scene.since_login();
            Self {
                offer: session.trade_offer_open().unwrap_or(false),
                confirm: session.trade_confirm_open().unwrap_or(false),
                partner: session
                    .trade_partner()
                    .map(|name| name.trim())
                    .filter(|name| !name.is_empty())
                    .map(str::to_string),
                mine_len: session.trade_mine().map_or(0, Vec::len),
                theirs: session
                    .trade_theirs()
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|row| Some((row.name.as_deref()?.to_string(), row.count)))
                            .collect()
                    })
                    .unwrap_or_default(),
                used: session.inv().map_or(0, |rows| {
                    rows.iter()
                        .filter(|row| !row.name_or_empty().is_empty())
                        .count() as i32
                }),
            }
        })
    }

    fn active(&self) -> bool {
        self.offer || self.confirm
    }

    /// Frozen `tradeScreen()`.
    fn name(&self) -> &'static str {
        if self.offer {
            "offer"
        } else if self.confirm {
            "confirm"
        } else {
            "closed"
        }
    }
}

/// Frozen `stableClosedPoll()`: closed once the trade stayed inactive for
/// [`TRADE_CLOSE_DEBOUNCE_MS`] across polls.
#[derive(Default)]
struct Stable {
    inactive_since: Option<Instant>,
}

impl Stable {
    fn poll(&mut self, cx: &mut Cx<'_>) -> bool {
        if Screen::read().active() {
            self.inactive_since = None;
            return false;
        }
        let now = cx.clock().now();
        let since = *self.inactive_since.get_or_insert(now);
        now.saturating_duration_since(since) >= Duration::from_millis(TRADE_CLOSE_DEBOUNCE_MS)
    }
}

/// Where a metric value goes once read.
enum MetricFor {
    ConfirmBefore,
    ConfirmDelta {
        before: f64,
    },
    /// `status`: the giver's label, shown after the metric.
    AcceptBefore {
        status: Option<Value>,
    },
    AcceptAfter {
        before: f64,
    },
}

/// Why the rows are being counted.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Count {
    /// `decideReceiverOfferScreen`'s `theirProductCount`.
    Decide,
    /// `theirN` for the receiver gate.
    Gate,
}

enum State {
    Start,
    Baseline,
    Metric(MetricFor),
    ConfirmAccept {
        before: f64,
    },
    ConfirmWait {
        before: f64,
        stable: Stable,
    },
    ConfirmSettle {
        before: f64,
    },
    Missing,
    Counting {
        why: Count,
        who: String,
        next: usize,
        total: f64,
    },
    Gate,
    AcceptClick {
        before: f64,
    },
    AcceptWait {
        before: f64,
        stable: Stable,
    },
    Ready,
    Names,
    OfferWait {
        stable: Stable,
    },
    Declining {
        reason: Value,
        decline: Option<Decline>,
        begun: bool,
    },
    WaitTick,
}

/// What one advance decided.
enum Next {
    /// The state moved on; advance again (after any queued call).
    Continue,
    Wait,
    Done,
    /// A callback whose value the current state reads.
    Call(usize, Vec<Value>),
    Fail(Thrown),
}

/// One frozen `driveActivePartnerTrade` iteration.
pub(crate) struct PartnerTrade {
    receiver: bool,
    partners: Vec<String>,
    verify_giver_partner: bool,
    labels: Map<String, Value>,
    state: State,
    /// Callbacks whose result is ignored (`setStatus`, `log`, `onComplete`,
    /// `onDecline`), called in order before the state goes on.
    says: VecDeque<(usize, Value)>,
    saying: bool,
    after: Option<Next>,
    /// `opts.onComplete` was given.
    has_complete: bool,
}

impl Family for PartnerTrade {
    const NAME: &'static str = "partner-trade";
    /// One trade screen: a new call replaces the one in flight.
    const EXCLUSIVE: bool = true;
    const CALLBACKS: &'static [&'static str] = &[
        "productNamesToOffer",
        "inventoryMetric",
        "myOfferReady",
        "theirProductMatch",
        "setStatus",
        "log",
        "onMissingPartner",
        "receiverCanAccept",
        "onComplete",
        "onDecline",
        "baseline",
    ];
    /// The frozen iteration's synchronous stretch runs in the caller's turn.
    const KICK_ON_START: bool = true;
    const AWAIT_CALLBACKS: bool = false;
    type Args = PartnerTradeArgs;
    /// Always void: outcomes reach the caller through its callbacks.
    type Output = Value;

    fn begin(args: PartnerTradeArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
        let partners = match args.partners {
            Value::Array(rows) => rows
                .into_iter()
                .filter_map(|row| row.as_str().map(str::to_string))
                .collect(),
            _ => Vec::new(),
        };
        Begin::Run(Self {
            receiver: args.role.as_str() == Some("receiver"),
            partners,
            verify_giver_partner: args.verify_giver_partner,
            labels: match args.labels {
                Value::Object(labels) => labels,
                _ => Map::new(),
            },
            state: State::Start,
            says: VecDeque::new(),
            saying: false,
            after: None,
            has_complete: false,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        let mut reply = match cx.reply() {
            Some(Reply::Threw(thrown)) => return Step::Fail(thrown),
            Some(Reply::Value(value)) => Some(value),
            None => None,
        };
        if std::mem::take(&mut self.saying) {
            reply = None;
        }
        loop {
            if let Some((hook, arg)) = self.says.pop_front() {
                self.saying = true;
                return Step::Call(Call {
                    hook,
                    args: vec![arg],
                });
            }
            let next = match self.after.take() {
                Some(next) => next,
                None => self.advance(reply.take(), cx),
            };
            if !self.says.is_empty() && !matches!(next, Next::Continue) {
                self.after = Some(next);
                continue;
            }
            match next {
                Next::Continue => {}
                Next::Wait => return Step::Wait,
                Next::Done => return Step::Done(Value::Null),
                Next::Call(hook, args) => return Step::Call(Call { hook, args }),
                Next::Fail(thrown) => return Step::Fail(thrown),
            }
        }
    }
}

impl PartnerTrade {
    fn say(&mut self, hook: usize, arg: Value) {
        self.says.push_back((hook, arg));
    }

    fn log(&mut self, line: String) {
        self.say(LOG, json!(line));
    }

    /// `labels[key] ?? default`.
    fn label(&self, key: &str, default: &str) -> Value {
        match self.labels.get(key) {
            Some(Value::Null) | None => json!(default),
            Some(label) => label.clone(),
        }
    }

    fn advance(&mut self, reply: Option<Value>, cx: &mut Cx<'_>) -> Next {
        match std::mem::replace(&mut self.state, State::WaitTick) {
            State::Start => self.start(cx),
            State::Baseline => match reply {
                // `baseline?.() ?? metric()`
                Some(value) if !value.is_null() => self.confirm_click(to_number(&value)),
                _ => self.metric(MetricFor::ConfirmBefore, cx),
            },
            State::Metric(want) => self.got_metric(want, to_number(&reply.unwrap_or(Value::Null))),
            State::ConfirmAccept { before } => {
                if let Some(fail) = self.accept(cx) {
                    return fail;
                }
                cx.clock().arm(TRADE_CONFIRM_WAIT_MS);
                self.state = State::ConfirmWait {
                    before,
                    stable: Stable::default(),
                };
                Next::Wait
            }
            State::ConfirmWait { before, mut stable } => {
                let closed = stable.poll(cx);
                if !closed && !cx.clock().bound_reached() {
                    self.state = State::ConfirmWait { before, stable };
                    return Next::Wait;
                }
                let screen = Screen::read();
                self.log(format!(
                    "trade: confirm wait {} after the last click — screen now {}",
                    if closed { "satisfied" } else { "TIMED OUT" },
                    screen.name()
                ));
                if screen.active() {
                    self.log(
                        "mule: confirm still open after wait — partner may not have accepted"
                            .into(),
                    );
                    return Next::Done;
                }
                // Settle a beat after the modal reports closed.
                self.state = State::ConfirmSettle { before };
                Next::Wait
            }
            State::ConfirmSettle { before } => self.metric(MetricFor::ConfirmDelta { before }, cx),
            State::Missing => {
                // `onMissingPartner?.() ?? 'wait'`
                let decline = reply.as_ref().and_then(Value::as_str) == Some("decline");
                self.missing(decline)
            }
            State::Counting {
                why,
                who,
                next,
                mut total,
            } => {
                let theirs = Screen::read().theirs;
                if let Some(matched) = reply {
                    if truthy(&matched) {
                        total += f64::from(theirs.get(next - 1).map_or(1, |row| row.1.max(1)));
                    }
                }
                if let Some((name, _)) = theirs.get(next) {
                    let name = name.clone();
                    self.state = State::Counting {
                        why,
                        who,
                        next: next + 1,
                        total,
                    };
                    return Next::Call(MATCH, vec![json!(name)]);
                }
                match why {
                    Count::Decide => self.decide(who, total),
                    Count::Gate => self.gate(total, cx),
                }
            }
            State::Gate => {
                let gate = reply.unwrap_or(Value::Null);
                let ok = gate == Value::Bool(true) || gate.get("ok") == Some(&Value::Bool(true));
                if ok {
                    return self.accept_receiver(cx);
                }
                let reason = match &gate {
                    Value::Object(fields) if fields.contains_key("reason") => {
                        fields["reason"].clone()
                    }
                    _ => json!("receiver cannot accept offer"),
                };
                let line = format!("trade: declining ({})", js_string(&reason));
                let status = self.label("declining", "mule: declining trade");
                self.decline(status, line, reason)
            }
            State::AcceptClick { before } => {
                if let Some(fail) = self.accept(cx) {
                    return fail;
                }
                cx.clock().arm(TRADE_OFFER_WAIT_MS);
                self.state = State::AcceptWait {
                    before,
                    stable: Stable::default(),
                };
                Next::Wait
            }
            State::AcceptWait { before, mut stable } => {
                let confirm = Screen::read().confirm;
                if !confirm && !stable.poll(cx) && !cx.clock().bound_reached() {
                    self.state = State::AcceptWait { before, stable };
                    return Next::Wait;
                }
                let screen = Screen::read();
                if screen.confirm {
                    self.log("trade: offer accepted — confirm screen is up".into());
                    Next::Done
                } else if stable.poll(cx) && !screen.active() {
                    self.metric(MetricFor::AcceptAfter { before }, cx)
                } else {
                    self.log(format!(
                        "trade: offer-accept wait TIMED OUT — screen now {}",
                        screen.name()
                    ));
                    Next::Done
                }
            }
            State::Ready => {
                // `myOfferReady?.() ?? Trade.myOffer().length > 0`
                let ready = match reply {
                    Some(value) if !value.is_null() => truthy(&value),
                    _ => Screen::read().mine_len > 0,
                };
                if ready {
                    self.metric(
                        MetricFor::AcceptBefore {
                            status: Some(self.label("acceptingOffer", "mule: accepting handoff")),
                        },
                        cx,
                    )
                } else {
                    self.state = State::Names;
                    Next::Call(NAMES, Vec::new())
                }
            }
            State::Names => self.offer(reply.unwrap_or(Value::Null), cx),
            State::OfferWait { mut stable } => {
                if cx.has(READY) && reply.is_none() {
                    self.state = State::OfferWait { stable };
                    return Next::Call(READY, Vec::new());
                }
                let ready = match reply {
                    Some(value) if !value.is_null() => truthy(&value),
                    _ => Screen::read().mine_len > 0,
                };
                let settled = ready || Screen::read().confirm || stable.poll(cx);
                if !settled && !cx.clock().bound_reached() {
                    self.state = State::OfferWait { stable };
                    return Next::Wait;
                }
                if stable.poll(cx) && !Screen::read().active() {
                    self.log(
                        "trade: window closed while waiting for the offer to register — partner declined, walked or cancelled"
                            .into(),
                    );
                }
                Next::Done
            }
            State::Declining {
                reason,
                mut decline,
                begun,
            } => {
                // Frozen `Trade.decline()`: the button now, its wait from the
                // next tick.
                let ended = if begun {
                    decline
                        .as_mut()
                        .is_none_or(|decline| matches!(decline.step(cx), Declining::Done(_)))
                } else {
                    decline = Decline::begin(cx);
                    decline.is_none()
                };
                if !ended {
                    self.state = State::Declining {
                        reason,
                        decline,
                        begun: true,
                    };
                    return Next::Wait;
                }
                let screen = Screen::read();
                self.log(format!(
                    "trade: decline clicked — screen now {}",
                    screen.name()
                ));
                if cx.has(ON_DECLINE) {
                    self.say(ON_DECLINE, reason);
                }
                Next::Done
            }
            State::WaitTick => Next::Done,
        }
    }

    fn start(&mut self, cx: &mut Cx<'_>) -> Next {
        self.has_complete = cx.has(ON_COMPLETE);
        let screen = Screen::read();
        if screen.confirm {
            let status = self.label("confirming", "mule: confirming trade");
            self.say(SET_STATUS, status);
            if cx.has(BASELINE) {
                self.state = State::Baseline;
                return Next::Call(BASELINE, Vec::new());
            }
            return self.metric(MetricFor::ConfirmBefore, cx);
        }
        if !screen.offer {
            return Next::Done;
        }
        if self.receiver || self.verify_giver_partner {
            let Some(who) = screen.partner else {
                if cx.has(ON_MISSING) {
                    self.state = State::Missing;
                    return Next::Call(ON_MISSING, Vec::new());
                }
                return self.missing(false);
            };
            if !self.receiver {
                // The giver's partner gate: a stranger is declined.
                if !crate::partner_trade::is_configured_partner(Some(&who), &self.partners) {
                    return self.decline_stranger(&who);
                }
            } else {
                self.state = State::Counting {
                    why: Count::Decide,
                    who,
                    next: 0,
                    total: 0.0,
                };
                return Next::Continue;
            }
        }
        self.state = State::Ready;
        if cx.has(READY) {
            return Next::Call(READY, Vec::new());
        }
        Next::Continue
    }

    fn missing(&mut self, decline: bool) -> Next {
        if decline {
            let status = self.label("declining", "mule: declining trade");
            return self.decline(
                status,
                "trade: declining — partner name never appeared on the modal".into(),
                json!("partner header timeout"),
            );
        }
        let status = self.label("waitHeader", "mule: reading partner");
        self.say(SET_STATUS, status);
        self.state = State::WaitTick;
        Next::Wait
    }

    fn decline_stranger(&mut self, who: &str) -> Next {
        let reason = format!("not a configured partner ({who})");
        let status = self.label("declining", "mule: declining trade");
        self.decline(
            status,
            format!("trade: declining ({reason})"),
            json!(reason),
        )
    }

    /// Frozen `decideReceiverOfferScreen` with a posted header.
    fn decide(&mut self, who: String, their_count: f64) -> Next {
        if !crate::partner_trade::is_configured_partner(Some(&who), &self.partners) {
            return self.decline_stranger(&who);
        }
        if Screen::read().mine_len > 0 {
            let status = self.label("declining", "mule: declining trade");
            return self.decline(
                status,
                "trade: declining (safety: own offer not empty)".into(),
                json!("safety: own offer not empty"),
            );
        }
        if their_count <= 0.0 {
            let status = self.label("waitOffer", "mule: waiting for product offer");
            self.say(SET_STATUS, status);
            self.state = State::WaitTick;
            return Next::Wait;
        }
        // `theirN` is counted again for the gate.
        self.state = State::Counting {
            why: Count::Gate,
            who,
            next: 0,
            total: 0.0,
        };
        Next::Continue
    }

    fn gate(&mut self, their_n: f64, cx: &mut Cx<'_>) -> Next {
        if cx.has(GATE) {
            self.state = State::Gate;
            return Next::Call(GATE, vec![js_number(their_n)]);
        }
        self.accept_receiver(cx)
    }

    fn accept_receiver(&mut self, cx: &mut Cx<'_>) -> Next {
        let status = self.label("accepting", "mule: accepting product");
        self.say(SET_STATUS, status);
        self.metric(MetricFor::AcceptBefore { status: None }, cx)
    }

    /// Frozen `metric()`: the caller's `inventoryMetric`, else
    /// `Inventory.used()`.
    fn metric(&mut self, want: MetricFor, cx: &mut Cx<'_>) -> Next {
        if cx.has(METRIC) {
            self.state = State::Metric(want);
            return Next::Call(METRIC, Vec::new());
        }
        self.got_metric(want, f64::from(Screen::read().used))
    }

    fn got_metric(&mut self, want: MetricFor, value: f64) -> Next {
        match want {
            MetricFor::ConfirmBefore => self.confirm_click(value),
            MetricFor::ConfirmDelta { before } => {
                let delta = value - before;
                // `onComplete` presence was read at start.
                if self.has_complete {
                    self.say(ON_COMPLETE, js_number(delta));
                } else {
                    self.log(format!("mule: trade complete (inv Δ{})", signed(delta)));
                }
                Next::Done
            }
            MetricFor::AcceptBefore { status } => {
                if let Some(status) = status {
                    self.say(SET_STATUS, status);
                }
                self.log(format!(
                    "trade: clicking Accept on the offer screen ({})",
                    Screen::read().name()
                ));
                self.state = State::AcceptClick { before: value };
                Next::Continue
            }
            MetricFor::AcceptAfter { before } => {
                self.log(format!(
                    "trade: window closed after OUR offer-accept without reaching confirm — partner declined, walked or cancelled (inv Δ{})",
                    signed(value - before)
                ));
                Next::Done
            }
        }
    }

    fn confirm_click(&mut self, before: f64) -> Next {
        self.log(format!(
            "trade: clicking Accept on the confirm screen ({})",
            Screen::read().name()
        ));
        self.state = State::ConfirmAccept { before };
        Next::Continue
    }

    /// Frozen `Trade.accept()`: the posted accept button. A missing control
    /// is a host gap and fails closed; no screen sends nothing, as frozen.
    fn accept(&mut self, cx: &mut Cx<'_>) -> Option<Next> {
        match trade::accept(cx) {
            Err("no-accept") => Some(Next::Fail(Thrown::new(
                "not impl: driveActivePartnerTrade: no-accept",
            ))),
            _ => None,
        }
    }

    /// The giver's offer step over `productNamesToOffer()`.
    fn offer(&mut self, names: Value, cx: &mut Cx<'_>) -> Next {
        let Value::Array(names) = names else {
            return Next::Fail(Thrown::new(
                "driveActivePartnerTrade: productNamesToOffer() did not return a list",
            ));
        };
        if names.is_empty() {
            return self.decline(
                json!("mule: nothing to offer — declining"),
                "trade: declining (nothing to offer)".into(),
                json!("nothing to offer"),
            );
        }
        let joined = names
            .iter()
            .map(|name| {
                if name.is_null() {
                    String::new()
                } else {
                    js_string(name)
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let status = self.label("offering", &format!("mule: offering {joined}"));
        self.say(SET_STATUS, status);
        let mut any = false;
        for name in &names {
            let name = js_string(name);
            if trade::offer_all(&name, cx).is_ok() {
                any = true;
            } else {
                self.log(format!("mule: offerAll failed for {name}"));
            }
        }
        if !any {
            return self.decline(
                json!("mule: declining trade"),
                "trade: declining (offerAll failed for every product)".into(),
                json!("offerAll failed"),
            );
        }
        cx.clock().arm(TRADE_OFFER_WAIT_MS);
        self.state = State::OfferWait {
            stable: Stable::default(),
        };
        Next::Wait
    }

    /// `setStatus(status)`, `log(line)`, `Trade.decline()`, the decline log
    /// and `onDecline?.(reason)`.
    fn decline(&mut self, status: Value, line: String, reason: Value) -> Next {
        self.say(SET_STATUS, status);
        self.log(line);
        self.state = State::Declining {
            reason,
            decline: None,
            begun: false,
        };
        Next::Continue
    }
}

/// JS truthiness of a callback's value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// JS `ToNumber` for the metric values. The shim passes `inventoryMetric`
/// and `baseline` results as `String(Number(v))` so `NaN` crosses intact.
fn to_number(value: &Value) -> f64 {
    match value {
        Value::Null => 0.0,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) if s.trim().is_empty() => 0.0,
        Value::String(s) => s.trim().parse().unwrap_or(f64::NAN),
        Value::Array(_) | Value::Object(_) => f64::NAN,
    }
}

/// JS `String(n)` for the numbers these lines print.
fn js_num(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity" } else { "-Infinity" }.into()
    } else if n.fract() == 0.0 && n.abs() < 1e21 {
        format!("{}", n as i64)
    } else {
        n.to_string()
    }
}

/// Frozen `${d >= 0 ? '+' : ''}${d}`.
fn signed(delta: f64) -> String {
    format!("{}{}", if delta >= 0.0 { "+" } else { "" }, js_num(delta))
}

/// A number argument; a non-finite one crosses as its JS string, which the
/// shim turns back with `Number(…)`.
fn js_number(n: f64) -> Value {
    if !n.is_finite() {
        json!(js_num(n))
    } else if n.fract() == 0.0 && n.abs() < 9e15 {
        json!(n as i64)
    } else {
        json!(n)
    }
}

/// JS `String(value)` for template literals.
fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => js_num(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                if item.is_null() {
                    String::new()
                } else {
                    js_string(item)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_waits_match_drive_partner_trade_ts() {
        assert_eq!(TRADE_OFFER_WAIT_MS, 5_000);
        assert_eq!(TRADE_CONFIRM_WAIT_MS, 8_000);
        assert_eq!(TRADE_CLOSE_DEBOUNCE_MS, 600);
    }

    #[test]
    fn metric_values_keep_js_number_coercion() {
        assert_eq!(to_number(&json!(24)), 24.0);
        assert_eq!(to_number(&json!("24")), 24.0);
        assert!(to_number(&json!("NaN")).is_nan(), "undefined - n is NaN");
        assert_eq!(to_number(&json!("-Infinity")), f64::NEG_INFINITY);
        assert!(to_number(&json!({})).is_nan(), "a promise is NaN");
        assert_eq!(signed(3.0), "+3");
        assert_eq!(signed(0.0), "+0");
        assert_eq!(signed(-24.0), "-24");
        assert_eq!(signed(f64::NAN), "NaN");
        assert_eq!(js_number(f64::NAN), json!("NaN"));
        assert_eq!(js_number(-24.0), json!(-24));
    }
}
