//! Rust-owned player trade request, offer, remove, accept and decline, the
//! `trade` [`crate::machine`] family.
//!
//! Frozen `Trade.ts` uses player op 4 (`Trade with`), exact side/mine
//! id+slot inv-buttons, Offer-X plus a real count-dialog, and posted
//! offer-versus-confirm buttons. JavaScript starts one machine per call; the
//! optional `pick` callback is called from here over the same-name side
//! rows in order (the frozen `matches.find(pick)`). A queued click is not a
//! transfer. A noted identity, an over-offer, a stale screen or a changed
//! partner fail closed. `drive_partner_trade` reuses the offer, accept and
//! decline pieces.

use crate::machine::{Begin, Call, Cx, Family, Reply, Step};
use crate::observed::{self, ItemRow, Ops, Scene, Text};
use crate::shim::InteractReq;
use serde::Deserialize;
use serde_json::{json, Value};

/// Frozen `Input.interactPlayer` op slot: `set_player_op("Trade with", 4)`.
pub const TRADE_OP: usize = 4;
/// Frozen tradeside:inv component (`OFFER_INV`).
#[allow(dead_code)]
pub const OFFER_INV: i32 = 3322;
/// Frozen trademain:inv component (`MY_OFFER_INV`).
#[allow(dead_code)]
pub const MY_OFFER_INV: i32 = 3415;
/// Frozen tradeside option4 (`OFFER_ALL`).
pub const OFFER_ALL: i32 = 4;
/// Frozen tradeside option5 (`OFFER_X`).
pub const OFFER_X: i32 = 5;
/// Frozen trademain:inv option4 (`REMOVE_ALL`).
pub const REMOVE_ALL: i32 = 4;
/// Frozen count-dialog open wait.
pub const COUNT_OPEN_MS: u64 = 3_000;
/// Frozen offer settlement wait after Answer-Count.
pub const SETTLE_MS: u64 = 4_000;
/// Frozen decline-button close wait before falling back to close-modal.
pub const DECLINE_BUTTON_WAIT_MS: u64 = 1_200;
/// Frozen close-modal wait after a decline that left the screen up.
pub const DECLINE_CLOSE_MS: u64 = 3_000;
/// Frozen `removeAll` iteration cap.
pub const REMOVE_MAX: u32 = 28;
/// Per-removal progress window (one game tick in the frozen loop).
pub const REMOVE_STEP_MS: u64 = 600;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    name: Text,
    id: i32,
    slot: i32,
    component: i32,
    count: i32,
    noted: bool,
}

fn rows_of(rows: &[ItemRow]) -> Vec<Row> {
    rows.iter()
        .map(|row| Row {
            name: row.name.clone().unwrap_or_default(),
            id: row.id,
            slot: row.slot_or_unset(),
            component: row.component_or_unset(),
            count: row.count,
            noted: row.noted,
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlayerRef {
    name: Text,
    distance: i32,
    actions: Ops,
}

/// The posted facts this module decides from, read from the isolate scene.
struct NativeObservation {
    ingame: bool,
    offer_open: bool,
    confirm_open: bool,
    partner: Option<String>,
    accept_id: i32,
    decline_id: i32,
    count_dialog_open: bool,
    side: Vec<Row>,
    mine: Vec<Row>,
    players: Vec<PlayerRef>,
}

impl NativeObservation {
    /// A logout forgets the session: only pages posted since login count.
    fn from_scene(scene: &Scene) -> Self {
        let session = scene.since_login();
        Self {
            ingame: session.ingame().unwrap_or(false),
            offer_open: session.trade_offer_open().unwrap_or(false),
            confirm_open: session.trade_confirm_open().unwrap_or(false),
            partner: session
                .trade_partner()
                .map(|name| name.trim())
                .filter(|name| !name.is_empty())
                .map(str::to_string),
            accept_id: session.trade_accept_id().unwrap_or(-1),
            decline_id: session.trade_decline_id().unwrap_or(-1),
            count_dialog_open: session.count_dialog_open().unwrap_or(false),
            side: session
                .trade_side()
                .map(|rows| rows_of(rows))
                .unwrap_or_default(),
            mine: session
                .trade_mine()
                .map(|rows| rows_of(rows))
                .unwrap_or_default(),
            players: session
                .players()
                .map(|rows| {
                    rows.iter()
                        .map(|player| PlayerRef {
                            name: player.name.clone().unwrap_or_default(),
                            distance: player.distance,
                            actions: player.actions.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    fn probe(&self) -> Probe<'_> {
        Probe {
            ingame: self.ingame,
            offer_open: self.offer_open,
            confirm_open: self.confirm_open,
            partner: self.partner.as_deref(),
            accept_id: self.accept_id,
            decline_id: self.decline_id,
            count_dialog_open: self.count_dialog_open,
            side: &self.side,
            mine: &self.mine,
            players: &self.players,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Kind {
    Request,
    OfferAll,
    Offer,
    RemoveAll,
    Accept,
    Decline,
}

impl Kind {
    /// The frozen settle value: `decline` is void, the rest a boolean.
    fn value(self, ok: bool) -> Value {
        match self {
            Self::Decline => Value::Null,
            _ => json!(ok),
        }
    }
}

struct Probe<'a> {
    ingame: bool,
    offer_open: bool,
    confirm_open: bool,
    partner: Option<&'a str>,
    accept_id: i32,
    decline_id: i32,
    count_dialog_open: bool,
    side: &'a [Row],
    mine: &'a [Row],
    players: &'a [PlayerRef],
}

impl Probe<'_> {
    fn active(&self) -> bool {
        self.offer_open || self.confirm_open
    }
}

fn observe() -> NativeObservation {
    observed::with(NativeObservation::from_scene)
}

/// The screen a call started on; a later read that differs is stale.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Screen {
    offer_open: bool,
    confirm_open: bool,
    partner: Option<String>,
}

impl Screen {
    fn of(probe: &Probe<'_>) -> Self {
        Self {
            offer_open: probe.offer_open,
            confirm_open: probe.confirm_open,
            partner: probe.partner.map(str::to_string),
        }
    }

    fn stale(&self, probe: &Probe<'_>) -> bool {
        if self.offer_open != probe.offer_open || self.confirm_open != probe.confirm_open {
            return true;
        }
        match (self.partner.as_deref(), probe.partner) {
            (Some(want), Some(have)) => !want.eq_ignore_ascii_case(have),
            (Some(_), None) => true,
            (None, _) => false,
        }
    }
}

fn inv_button(row: &Row, operation: i32) -> InteractReq {
    InteractReq::InvButton {
        id: row.id,
        slot: row.slot,
        component: row.component,
        operation,
        bank_generation: 0,
    }
}

/// Frozen `Trade.accept`: the posted accept button of the open screen.
/// `Err` is the refusal reason; nothing was sent.
pub(crate) fn accept(cx: &mut Cx<'_>) -> Result<(), &'static str> {
    let obs = observe();
    let probe = obs.probe();
    if !probe.active() {
        return Err("missing-screen");
    }
    if probe.accept_id < 0 {
        return Err("no-accept");
    }
    cx.emit(InteractReq::IfButton {
        component_id: probe.accept_id,
    });
    Ok(())
}

/// Frozen `Trade.offerAll(name)` without `pick`: Offer-All on the first
/// side row named `name` (a noted row fails closed, as in [`Trade`]).
/// `Err` is the refusal reason; nothing was sent.
pub(crate) fn offer_all(name: &str, cx: &mut Cx<'_>) -> Result<(), &'static str> {
    let obs = observe();
    let probe = obs.probe();
    let rows = offer_rows(&probe, Kind::OfferAll, name, 0)?;
    let row = &rows[0];
    select(&probe, &Screen::of(&probe), name, row, Kind::OfferAll, 0).map(|pressed| {
        cx.emit(pressed);
    })
}

/// The same-name side rows an offer chooses from, after the frozen
/// preconditions.
fn offer_rows(probe: &Probe<'_>, kind: Kind, name: &str, n: i64) -> Result<Vec<Row>, &'static str> {
    if !probe.offer_open {
        return Err("missing-screen");
    }
    if name.is_empty() {
        return Err("no-match");
    }
    if kind == Kind::Offer && n <= 0 {
        return Err("invalid");
    }
    let rows: Vec<Row> = named_side(probe.side, name).into_iter().cloned().collect();
    if rows.is_empty() {
        return Err("no-match");
    }
    Ok(rows)
}

/// The chosen row, re-resolved on the current screen: the Offer-All or
/// Offer-X press to send.
fn select(
    probe: &Probe<'_>,
    screen: &Screen,
    name: &str,
    chosen: &Row,
    kind: Kind,
    amount: i32,
) -> Result<InteractReq, &'static str> {
    if !probe.ingame {
        return Err("not-ingame");
    }
    if !probe.offer_open || screen.stale(probe) {
        return Err("stale-screen");
    }
    let Some(row) = named_side(probe.side, name)
        .into_iter()
        .find(|row| row.id == chosen.id && row.slot == chosen.slot)
    else {
        return Err("wrong-identity");
    };
    if row.noted {
        return Err("noted");
    }
    if row.component < 0 || row.slot < 0 {
        return Err("no-identity");
    }
    match kind {
        Kind::OfferAll => Ok(inv_button(row, OFFER_ALL)),
        _ if amount > row.count => Err("over-offer"),
        _ => Ok(inv_button(row, OFFER_X)),
    }
}

/// One [`Decline::step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Declining {
    Wait,
    /// The button left the screen up: close-modal went out now.
    Closing,
    /// Ended; `true` when the screen closed.
    Done(bool),
}

/// Frozen `Trade.decline`: the decline button, then close-modal if the
/// screen stayed up.
pub(crate) struct Decline {
    closing: bool,
}

impl Decline {
    /// `None`: no trade screen is up, nothing to decline.
    pub(crate) fn begin(cx: &mut Cx<'_>) -> Option<Self> {
        let obs = observe();
        let probe = obs.probe();
        if !probe.active() {
            return None;
        }
        if probe.decline_id < 0 {
            cx.clock().arm(DECLINE_CLOSE_MS);
            cx.emit(InteractReq::CloseModal);
            return Some(Self { closing: true });
        }
        cx.clock().arm(DECLINE_BUTTON_WAIT_MS);
        cx.emit(InteractReq::IfButton {
            component_id: probe.decline_id,
        });
        Some(Self { closing: false })
    }

    pub(crate) fn step(&mut self, cx: &mut Cx<'_>) -> Declining {
        let active = observe().probe().active();
        if !active {
            return Declining::Done(true);
        }
        if !cx.clock().bound_reached() {
            return Declining::Wait;
        }
        if self.closing {
            return Declining::Done(false);
        }
        self.closing = true;
        cx.clock().arm(DECLINE_CLOSE_MS);
        cx.emit(InteractReq::CloseModal);
        Declining::Closing
    }
}

/// The caller's `pick` hook.
const PICK: usize = 0;

#[derive(Deserialize)]
pub(crate) struct TradeArgs {
    kind: Kind,
    #[serde(default)]
    name: String,
    #[serde(default)]
    n: Value,
}

enum Phase {
    /// Calling `pick` on each same-name candidate in order.
    Pick {
        rows: Vec<Row>,
        next: usize,
    },
    /// Offer-X sent; waiting `count_dialog_open`.
    WaitCountOpen,
    /// Answer-Count sent; waiting the named offer to land.
    WaitSettle,
    /// One Remove-All sent; waiting my-offer to shrink.
    WaitRemove,
    Decline(Decline),
}

/// One frozen `Trade` call.
pub(crate) struct Trade {
    kind: Kind,
    phase: Phase,
    name: String,
    amount: i32,
    screen: Screen,
    selected: Option<Row>,
    mine_baseline: i32,
    mine_len: usize,
    removals: u32,
}

impl Family for Trade {
    const NAME: &'static str = "trade";
    /// A new trade call replaces the one in flight.
    const EXCLUSIVE: bool = true;
    const CALLBACKS: &'static [&'static str] = &["pick"];
    /// `pick` runs in the caller's tick, like the frozen `find(pick)`.
    const KICK_ON_START: bool = true;
    type Args = TradeArgs;
    type Output = Value;

    fn begin(args: TradeArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let kind = args.kind;
        let name = args.name.trim().to_string();
        let n = whole(&args.n).unwrap_or(0);
        let obs = observe();
        let probe = obs.probe();
        let refused = Begin::Done(kind.value(false));
        if !probe.ingame {
            return refused;
        }
        let mut trade = Self {
            kind,
            phase: Phase::WaitSettle,
            name,
            amount: n as i32,
            screen: Screen::of(&probe),
            selected: None,
            mine_baseline: 0,
            mine_len: 0,
            removals: 0,
        };
        match kind {
            Kind::Request => {
                let Some((player, action)) = nearest_named(probe.players, &trade.name)
                    .and_then(|player| Some((player, trade_action(player)?)))
                else {
                    return refused;
                };
                cx.emit(InteractReq::Player {
                    name: player.name.to_string(),
                    action: action.to_string(),
                });
                Begin::Done(kind.value(true))
            }
            Kind::OfferAll | Kind::Offer => {
                let Ok(rows) = offer_rows(&probe, kind, &trade.name, n) else {
                    return refused;
                };
                if cx.has(PICK) {
                    trade.phase = Phase::Pick { rows, next: 0 };
                    return Begin::Run(trade);
                }
                match trade.offer(&probe, &rows[0], cx) {
                    Step::Done(value) => Begin::Done(value),
                    _ => Begin::Run(trade),
                }
            }
            Kind::RemoveAll => {
                if !probe.offer_open {
                    return refused;
                }
                let Some(first) = probe.mine.first() else {
                    return Begin::Done(kind.value(true));
                };
                if first.component < 0 || first.slot < 0 {
                    return refused;
                }
                cx.emit(inv_button(first, REMOVE_ALL));
                cx.clock().arm(REMOVE_STEP_MS);
                trade.mine_len = probe.mine.len();
                trade.removals = 1;
                trade.phase = Phase::WaitRemove;
                Begin::Run(trade)
            }
            Kind::Accept => match accept(cx) {
                Ok(()) => Begin::Done(kind.value(true)),
                Err(_) => refused,
            },
            Kind::Decline => match Decline::begin(cx) {
                Some(decline) => {
                    trade.phase = Phase::Decline(decline);
                    Begin::Run(trade)
                }
                None => refused,
            },
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        let reply = cx.reply();
        if let Some(Reply::Threw(thrown)) = reply {
            return Step::Fail(thrown);
        }
        let obs = observe();
        let probe = obs.probe();
        if !probe.ingame {
            return Step::Done(self.kind.value(false));
        }
        match &mut self.phase {
            Phase::Pick { rows, next } => {
                if let Some(Reply::Value(picked)) = reply {
                    if truthy(&picked) {
                        let row = rows[*next].clone();
                        return self.offer(&probe, &row, cx);
                    }
                    *next += 1;
                }
                let Some(row) = rows.get(*next) else {
                    return Step::Done(self.kind.value(false));
                };
                Step::Call(Call {
                    hook: PICK,
                    args: vec![json!({
                        "id": row.id,
                        "slot": row.slot,
                        "count": row.count,
                        "noted": row.noted,
                    })],
                })
            }
            Phase::WaitCountOpen | Phase::WaitSettle => self.offer_step(&probe, cx),
            Phase::WaitRemove => self.remove_step(&probe, cx),
            Phase::Decline(decline) => match decline.step(cx) {
                Declining::Done(_) => Step::Done(Value::Null),
                Declining::Wait | Declining::Closing => Step::Wait,
            },
        }
    }
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

/// A whole JS number (a float beyond int32).
fn whole(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_f64()
            .filter(|n| n.fract() == 0.0)
            .map(|n| n as i64)
    })
}

impl Trade {
    /// Press Offer-All / Offer-X on the chosen row.
    fn offer(&mut self, probe: &Probe<'_>, chosen: &Row, cx: &mut Cx<'_>) -> Step<Value> {
        let pressed = match select(
            probe,
            &self.screen,
            &self.name,
            chosen,
            self.kind,
            self.amount,
        ) {
            Ok(pressed) => pressed,
            Err(_) => return Step::Done(self.kind.value(false)),
        };
        cx.emit(pressed);
        if self.kind == Kind::OfferAll {
            return Step::Done(self.kind.value(true));
        }
        self.selected = Some(chosen.clone());
        self.mine_baseline = offered_of(probe.mine, chosen.id);
        self.phase = Phase::WaitCountOpen;
        cx.clock().arm(COUNT_OPEN_MS);
        Step::Wait
    }

    fn offer_step(&mut self, probe: &Probe<'_>, cx: &mut Cx<'_>) -> Step<Value> {
        if !probe.offer_open || self.screen.stale(probe) {
            return Step::Done(json!(false));
        }
        let Some(selected) = self.selected.clone() else {
            return Step::Done(json!(false));
        };
        match self.phase {
            Phase::WaitCountOpen => {
                if probe.count_dialog_open {
                    self.phase = Phase::WaitSettle;
                    cx.clock().arm(SETTLE_MS);
                    cx.emit(InteractReq::AnswerCount { value: self.amount });
                    return Step::Wait;
                }
                if cx.clock().bound_reached() {
                    return Step::Done(json!(false));
                }
                Step::Wait
            }
            _ => {
                if offered_of(probe.mine, selected.id) >= self.mine_baseline + self.amount {
                    return Step::Done(json!(true));
                }
                if cx.clock().bound_reached() {
                    return Step::Done(json!(false));
                }
                Step::Wait
            }
        }
    }

    fn remove_step(&mut self, probe: &Probe<'_>, cx: &mut Cx<'_>) -> Step<Value> {
        if !probe.offer_open || self.screen.stale(probe) {
            return Step::Done(json!(false));
        }
        if probe.mine.is_empty() {
            return Step::Done(json!(true));
        }
        if probe.mine.len() < self.mine_len {
            if self.removals >= REMOVE_MAX {
                return Step::Done(json!(false));
            }
            let next = &probe.mine[0];
            if next.component < 0 || next.slot < 0 {
                return Step::Done(json!(false));
            }
            cx.emit(inv_button(next, REMOVE_ALL));
            self.mine_len = probe.mine.len();
            self.removals += 1;
            cx.clock().arm(REMOVE_STEP_MS);
            return Step::Wait;
        }
        if cx.clock().bound_reached() {
            return Step::Done(json!(false));
        }
        Step::Wait
    }
}

fn matches_name(row: &Row, name: &str) -> bool {
    !name.is_empty() && row.name.eq_ignore_ascii_case(name)
}

fn named_side<'a>(side: &'a [Row], name: &str) -> Vec<&'a Row> {
    side.iter().filter(|row| matches_name(row, name)).collect()
}

fn offered_of(mine: &[Row], id: i32) -> i32 {
    mine.iter()
        .filter(|row| row.id == id)
        .map(|row| row.count.max(1))
        .sum()
}

fn trade_action(player: &PlayerRef) -> Option<&str> {
    player
        .actions
        .get(TRADE_OP - 1)
        .map(|action| &**action)
        .filter(|action| {
            let trimmed = action.trim();
            !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("hidden")
        })
}

fn nearest_named<'a>(players: &'a [PlayerRef], name: &str) -> Option<&'a PlayerRef> {
    let wanted = name.trim();
    if wanted.is_empty() {
        return None;
    }
    players
        .iter()
        .filter(|player| player.name.eq_ignore_ascii_case(wanted))
        .min_by_key(|player| player.distance)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trade_op_is_the_frozen_fourth_player_slot() {
        assert_eq!(TRADE_OP, 4);
        let player = PlayerRef {
            name: "bob".into(),
            distance: 1,
            actions: vec![
                "Attack".into(),
                "Follow".into(),
                "Report".into(),
                "Trade with".into(),
            ]
            .into(),
        };
        assert_eq!(trade_action(&player), Some("Trade with"));
        let missing = PlayerRef {
            name: "bob".into(),
            distance: 1,
            actions: vec!["Attack".into(), "Follow".into()].into(),
        };
        assert!(trade_action(&missing).is_none());
    }

    fn side(id: i32, slot: i32, count: i32, noted: bool) -> Row {
        Row {
            name: "Flax".into(),
            id,
            slot,
            component: 3322,
            count,
            noted,
        }
    }

    fn screen_probe<'a>(side: &'a [Row], partner: Option<&'a str>) -> Probe<'a> {
        Probe {
            ingame: true,
            offer_open: true,
            confirm_open: false,
            partner,
            accept_id: 3420,
            decline_id: 3422,
            count_dialog_open: false,
            side,
            mine: &[],
            players: &[],
        }
    }

    #[test]
    fn select_re_resolves_the_exact_row_and_fails_closed() {
        let rows = [side(1779, 3, 10, false), side(1780, 4, 50, true)];
        let probe = screen_probe(&rows, Some("Spinner"));
        let screen = Screen::of(&probe);
        assert_eq!(
            select(&probe, &screen, "flax", &rows[0], Kind::Offer, 10),
            Ok(InteractReq::InvButton {
                id: 1779,
                slot: 3,
                component: 3322,
                operation: OFFER_X,
                bank_generation: 0,
            })
        );
        assert_eq!(
            select(&probe, &screen, "flax", &rows[0], Kind::Offer, 11),
            Err("over-offer")
        );
        assert_eq!(
            select(&probe, &screen, "flax", &rows[1], Kind::OfferAll, 0),
            Err("noted")
        );
        assert_eq!(
            select(
                &probe,
                &screen,
                "flax",
                &side(1779, 9, 10, false),
                Kind::OfferAll,
                0
            ),
            Err("wrong-identity"),
            "an equal id in another slot is not the chosen row"
        );
        let other = screen_probe(&rows, Some("Stranger"));
        assert_eq!(
            select(&other, &screen, "flax", &rows[0], Kind::OfferAll, 0),
            Err("stale-screen"),
            "a changed partner is a stale screen"
        );
    }

    #[test]
    fn truthy_is_js_truthiness_of_the_pick_result() {
        for (value, want) in [
            (json!(null), false),
            (json!(false), false),
            (json!(0), false),
            (json!(""), false),
            (json!(1), true),
            (json!("x"), true),
            (json!({}), true),
            (json!([]), true),
        ] {
            assert_eq!(truthy(&value), want, "{value}");
        }
    }

    #[test]
    fn offer_constants_match_frozen_trade_ts() {
        assert_eq!(OFFER_INV, 3322);
        assert_eq!(MY_OFFER_INV, 3415);
        assert_eq!(OFFER_ALL, 4);
        assert_eq!(OFFER_X, 5);
        assert_eq!(REMOVE_ALL, 4);
        assert_eq!(COUNT_OPEN_MS, 3_000);
        assert_eq!(SETTLE_MS, 4_000);
        assert_eq!(DECLINE_BUTTON_WAIT_MS, 1_200);
        assert_eq!(DECLINE_CLOSE_MS, 3_000);
        assert_eq!(REMOVE_MAX, 28);
    }
}
