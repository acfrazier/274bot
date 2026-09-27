//! Clue Duel Arena protocol.
//!
//! The compatibility modules are only await/name adapters. Partner matching,
//! no-stake and rule validation, timing, retries, leaving, travel legs, the
//! modal verbs and the arena tables live here. The account's global clue duel
//! partner and its own name are read here too, from the posted settings bag
//! and scene, never echoed back through JavaScript.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use api::game_data::DuelControls;
use api::snapshot::WorldTile;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::machine::{Begin, Cx, Family, Step};
use crate::observed::{self, SceneRow, Tile};
use crate::shim::InteractReq;
use crate::walk::Resilient;

/// Frozen `CLUE_DUEL_OPTIONS`: obstacles only.
const RULES: i32 = 1024;
/// Frozen `CLUE_DUEL_LOBBY`.
const LOBBY: Tile = Tile {
    x: 3368,
    z: 3274,
    level: 0,
};
/// Frozen `DUEL_CLUE_ID` and `DUEL_CLUE_TILE` (`duelTravel.ts:11-12`).
pub(crate) const CLUE_ID: i32 = 3554;
pub(crate) const CLUE_TILE: Tile = Tile {
    x: 3374,
    z: 3250,
    level: 0,
};
const CHALLENGE_COOLDOWN: Duration = Duration::from_secs(5);
const OFFER_TIMEOUT: Duration = Duration::from_secs(30);
/// Frozen handshake window and entry attempts (`duelTravel.ts:27-30`).
const ENTRY_TIMEOUT_MS: u64 = 60_000;
const ENTRY_ATTEMPTS: u8 = 2;
/// Frozen travel legs: `walkResilient` with three attempts, 60 s to the
/// lobby and 45 s to the destination (`duelTravel.ts:28, 51`).
const LEG_ATTEMPTS: u32 = 3;
const LOBBY_WALK_MS: u64 = 60_000;
const DEST_WALK_MS: u64 = 45_000;
const HELPER_TIMEOUT: Duration = Duration::from_secs(180);
/// Frozen `leaveClueDuel`: the 'Yes' option within 6 s, out of the pen
/// within 10 s of answering it (`ClueDuel.ts:19-21`).
const LEAVE_YES: Duration = Duration::from_secs(6);
const LEAVE_EXIT: Duration = Duration::from_secs(10);

const MISMATCHED: &str = "rejecting a mismatched, staked or stalled clue duel";
const UNSAFE_RULES: &str = "rejecting unsafe clue duel rules";
const NO_PARTNER: &str =
    "set Global clue duel partner and run that account in Duel Arena Clue helper mode";

/// Frozen `DUEL_FIGHT_ARENAS` as `(minX, maxX, minZ, maxZ)`.
const PENS: [(i32, i32, i32, i32); 6] = [
    (3333, 3357, 3244, 3258),
    (3364, 3388, 3225, 3239),
    (3333, 3357, 3206, 3220),
    (3364, 3388, 3244, 3258),
    (3333, 3357, 3225, 3239),
    (3364, 3388, 3206, 3220),
];

/// The `log` hook of the duel families' `runMachine` calls.
const LOG: usize = 0;

thread_local! {
    static CONTROLS: RefCell<Option<DuelControls>> = const { RefCell::new(None) };
    /// The account's global clue duel partner as last posted.
    static PARTNER: RefCell<String> = const { RefCell::new(String::new()) };
    static HANDSHAKE: RefCell<HandshakeMemory> = RefCell::new(HandshakeMemory::default());
    static HELPER: RefCell<HelperMemory> = RefCell::new(HelperMemory::default());
}

pub(crate) fn configure(data: Option<&api::game_data::SelectedGameData>) {
    CONTROLS.with(|slot| *slot.borrow_mut() = data.and_then(|data| data.duel_controls().copied()));
}

/// Record the global partner from a posted settings bag (frozen
/// `SettingsStore.globalBag().str('clueDuelPartner', '').trim()`).
pub(crate) fn settings(bag: &serde_json::Map<String, Value>) {
    let partner = bag
        .get(crate::CLUE_DUEL_PARTNER)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    PARTNER.with(|slot| *slot.borrow_mut() = partner);
}

pub(crate) fn on_reset() {
    HANDSHAKE.with(|state| *state.borrow_mut() = HandshakeMemory::default());
    HELPER.with(|state| *state.borrow_mut() = HelperMemory::default());
}

fn controls() -> Option<DuelControls> {
    CONTROLS.with(|slot| *slot.borrow())
}

fn global_partner() -> String {
    PARTNER.with(|slot| slot.borrow().clone())
}

/// Frozen `clueDuelName`: underscores read as spaces, trimmed, lowercased.
/// The host's accept gate compares names the same way.
pub fn duel_name(name: &str) -> String {
    name.replace('_', " ").trim().to_lowercase()
}

/// Frozen `parseDuelPartnerHeader`: the display name after an optional
/// `Dueling with :` prefix (any case, optional spaces around the colon).
pub fn duel_partner_name(text: &str) -> Option<String> {
    let text = text.trim();
    let name = strip_dueling_with(text).unwrap_or(text).trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn strip_dueling_with(text: &str) -> Option<&str> {
    const PREFIX: &str = "dueling with";
    let head = text.get(..PREFIX.len())?;
    if !head.eq_ignore_ascii_case(PREFIX) {
        return None;
    }
    text[PREFIX.len()..].trim_start().strip_prefix(':')
}

pub(crate) fn pen_at(tile: Tile) -> Option<usize> {
    (tile.level == 0).then_some(())?;
    PENS.iter().position(|&(min_x, max_x, min_z, max_z)| {
        tile.x >= min_x && tile.x <= max_x && tile.z >= min_z && tile.z <= max_z
    })
}

fn within(here: Tile, dest: Tile, radius: i32) -> bool {
    here.level == dest.level && (here.x - dest.x).abs().max((here.z - dest.z).abs()) <= radius
}

fn world(tile: Tile) -> WorldTile {
    WorldTile {
        x: tile.x,
        z: tile.z,
        level: tile.level,
    }
}

fn tile_json(tile: Tile) -> Value {
    json!({ "x": tile.x, "z": tile.z, "level": tile.level })
}

/// Frozen `!partner || clueDuelName(partner) === clueDuelName(localPlayerName())`
/// refuses; anything else is a partner to enter a pen with.
pub(crate) fn valid_partner(partner: &str, self_name: &str) -> bool {
    !partner.trim().is_empty() && duel_name(partner) != duel_name(self_name)
}

/// Forward queued log lines through the row's log hook `hook` (frozen
/// `log(...)`, a synchronous call). `false`: the row ended during a call.
fn flush_logs(cx: &mut Cx<'_>, hook: usize, logs: &mut VecDeque<String>) -> bool {
    while let Some(line) = logs.pop_front() {
        if cx.has(hook) && cx.ask(hook, &[json!(line)]).is_err() {
            return false;
        }
    }
    true
}

#[derive(Default)]
struct HandshakeMemory {
    partner: String,
    opened_at: Option<Instant>,
    challenged_at: Option<Instant>,
    option_sent_at: Option<Instant>,
    validated_offer: bool,
}

/// A fresh frozen `new ClueDuelHandshake(partner, …)`.
fn fresh_handshake(partner: &str) {
    HANDSHAKE.with(|memory| {
        *memory.borrow_mut() = HandshakeMemory {
            partner: duel_name(partner),
            ..HandshakeMemory::default()
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Closed,
    Offer,
    Confirm,
    Win,
    Other,
}

pub(crate) fn dispatch(value: &Value) -> Value {
    let op = value.get("op").and_then(Value::as_str).unwrap_or("");
    match op {
        "name" => json!(duel_name(
            value.get("name").and_then(Value::as_str).unwrap_or("")
        )),
        "header" => value
            .get("text")
            .and_then(Value::as_str)
            .and_then(duel_partner_name)
            .map_or(Value::Null, Value::String),
        "handshake-new" => {
            fresh_handshake(value.get("partner").and_then(Value::as_str).unwrap_or(""));
            Value::Null
        }
        // The index into `tables.pens`: the shim answers with that frozen
        // object, so callers can compare pens by identity as frozen does.
        "pen" => json_tile(value.get("tile"))
            .and_then(pen_at)
            .map_or(Value::Null, |index| json!(index)),
        "crosses" => {
            let Some(dest) = json_tile(value.get("tile")) else {
                return json!(false);
            };
            let here_in_pen =
                observed::with(|scene| scene.latest().here().and_then(pen_at).is_some());
            json!(dest == CLUE_TILE || here_in_pen)
        }
        "tables" => json!({
            "pens": PENS.map(|(min_x, max_x, min_z, max_z)| {
                json!({ "minX": min_x, "maxX": max_x, "minZ": min_z, "maxZ": max_z })
            }),
            "lobby": tile_json(LOBBY),
            "rules": RULES,
            "clue": { "id": CLUE_ID, "tile": tile_json(CLUE_TILE) },
        }),
        "facts" => {
            let Some(controls) = controls() else {
                return Value::Null;
            };
            observed::with(|scene| {
                let scene = scene.latest();
                let current = screen(&scene, controls);
                let partner = widget(&scene, controls.select_partner)
                    .and_then(|(text, _)| duel_partner_name(&text));
                let status = match current {
                    Screen::Offer => controls.select_status,
                    Screen::Confirm => controls.confirm_status,
                    _ => -1,
                };
                json!({
                    "offer": current == Screen::Offer,
                    "confirm": current == Screen::Confirm,
                    "win": current == Screen::Win,
                    "partner": partner,
                    "waiting": status >= 0 && waiting(&scene, status),
                })
            })
        }
        // Frozen `Duel.accept`: the open screen's accept button, as the op
        // the shim queues; null when neither screen is open.
        "accept" => {
            let Some(controls) = controls() else {
                return Value::Null;
            };
            let component_id = observed::with(|scene| match screen(&scene.latest(), controls) {
                Screen::Offer => Some(controls.select_accept),
                Screen::Confirm => Some(controls.confirm_accept),
                _ => None,
            });
            component_id.map_or(
                Value::Null,
                |component_id| json!({ "op": "if-button", "component_id": component_id }),
            )
        }
        _ => Value::Null,
    }
}

fn json_tile(value: Option<&Value>) -> Option<Tile> {
    let value = value?;
    Some(Tile {
        x: i32::try_from(value.get("x")?.as_i64()?).ok()?,
        z: i32::try_from(value.get("z")?.as_i64()?).ok()?,
        level: i32::try_from(value.get("level")?.as_i64()?).ok()?,
    })
}

fn screen(scene: &observed::Lens<'_>, controls: DuelControls) -> Screen {
    match scene.main_modal_id().unwrap_or(-1) {
        id if id == controls.select_modal => Screen::Offer,
        id if id == controls.confirm_modal => Screen::Confirm,
        id if id == controls.win_modal => Screen::Win,
        -1 => Screen::Closed,
        _ => Screen::Other,
    }
}

/// Frozen `Duel.active()`: the offer or the confirm screen is open.
fn duel_active(controls: DuelControls) -> bool {
    observed::with(|scene| {
        matches!(
            screen(&scene.latest(), controls),
            Screen::Offer | Screen::Confirm
        )
    })
}

fn widget(scene: &observed::Lens<'_>, component_id: i32) -> Option<(String, i32)> {
    scene.widgets()?.iter().find_map(|row| {
        (row.component_id == component_id).then(|| (row.text.to_string(), row.item_count))
    })
}

fn varp(scene: &observed::Lens<'_>, index: i32) -> Option<i32> {
    scene
        .varps()?
        .iter()
        .find(|row| row.index == index)
        .map(|row| row.value)
}

fn waiting(scene: &observed::Lens<'_>, component_id: i32) -> bool {
    widget(scene, component_id).is_some_and(|(text, _)| {
        text.trim()
            .to_ascii_lowercase()
            .starts_with("waiting for other player")
    })
}

fn other_accepted(scene: &observed::Lens<'_>, component_id: i32) -> bool {
    widget(scene, component_id).is_some_and(|(text, _)| {
        text.trim()
            .to_ascii_lowercase()
            .starts_with("other player has accepted")
    })
}

/// Frozen: log, then `Duel.cancel()` on the open offer or confirm screen.
fn reject(
    line: &'static str,
    detail: &str,
    cx: &mut Cx<'_>,
    state: &mut HandshakeMemory,
    logs: &mut VecDeque<String>,
) -> bool {
    if std::env::var_os("BOT_DEBUG").is_some() {
        eprintln!("[clue-duel] rejecting handshake: {detail}");
    }
    logs.push_back(line.to_string());
    cx.emit(InteractReq::CloseModal);
    state.opened_at = None;
    state.validated_offer = false;
    false
}

fn offer_stalled(opened_at: &mut Option<Instant>, now: Instant) -> bool {
    let opened = *opened_at.get_or_insert(now);
    now.saturating_duration_since(opened) >= OFFER_TIMEOUT
}

/// One bounded frozen `ClueDuelHandshake.tick` iteration.
fn handshake_tick(
    partner: &str,
    initiator: bool,
    cx: &mut Cx<'_>,
    logs: &mut VecDeque<String>,
) -> bool {
    let Some(controls) = controls() else {
        return false;
    };
    let expected = duel_name(partner);
    HANDSHAKE.with(|memory| {
        let mut state = memory.borrow_mut();
        if state.partner != expected {
            *state = HandshakeMemory {
                partner: expected.clone(),
                ..HandshakeMemory::default()
            };
        }
        observed::with(|scene| {
            let scene = scene.latest();
            let current = screen(&scene, controls);
            if !matches!(current, Screen::Offer | Screen::Confirm) {
                state.opened_at = None;
                state.validated_offer = false;
                if pen_at(scene.here().unwrap_or_default()).is_some() {
                    return true;
                }
                let now = Instant::now();
                if state
                    .challenged_at
                    .is_some_and(|last| now.saturating_duration_since(last) < CHALLENGE_COOLDOWN)
                {
                    return true;
                }
                let target = scene.players().and_then(|players| {
                    players.iter().find(|player| {
                        duel_name(player.name_or_empty()) == expected
                            && !player.in_combat
                            && pen_at(player.tile()).is_none()
                    })
                });
                if let Some(target) = target {
                    state.challenged_at = Some(now);
                    cx.emit(InteractReq::Player {
                        name: target.name_or_empty().to_string(),
                        action: "Challenge".into(),
                    });
                }
                return true;
            }

            let now = Instant::now();
            if offer_stalled(&mut state.opened_at, now) {
                let status_id = match current {
                    Screen::Offer => controls.select_status,
                    _ => controls.confirm_status,
                };
                let detail = format!(
                    "offer timed out initiator={initiator} screen={current:?} options={:?} status={:?}",
                    varp(&scene, controls.options_varp),
                    widget(&scene, status_id).map(|(text, _)| text)
                );
                return reject(MISMATCHED, &detail, cx, &mut state, logs);
            }
            let (mine_id, theirs_id, status_id) = match current {
                Screen::Offer => (
                    controls.select_mine,
                    controls.select_theirs,
                    controls.select_status,
                ),
                _ => (
                    controls.confirm_mine,
                    controls.confirm_theirs,
                    controls.confirm_status,
                ),
            };
            let mine = widget(&scene, mine_id).map(|(_, count)| count);
            let theirs = widget(&scene, theirs_id).map(|(_, count)| count);
            if current == Screen::Offer && (mine.is_none() || theirs.is_none()) {
                return true;
            }
            if mine.is_some_and(|count| count != 0) || theirs.is_some_and(|count| count != 0) {
                return reject(
                    MISMATCHED,
                    "stake container is not empty",
                    cx,
                    &mut state,
                    logs,
                );
            }
            if current == Screen::Offer {
                let actual = widget(&scene, controls.select_partner)
                    .and_then(|(text, _)| duel_partner_name(&text))
                    .map(|name| duel_name(&name));
                if actual.as_deref() != Some(expected.as_str()) {
                    return reject(
                        MISMATCHED,
                        "offer partner does not match",
                        cx,
                        &mut state,
                        logs,
                    );
                }
                state.validated_offer = true;
            } else if !state.validated_offer {
                return reject(
                    MISMATCHED,
                    "confirm arrived without a validated offer",
                    cx,
                    &mut state,
                    logs,
                );
            }

            let options = varp(&scene, controls.options_varp);
            if current == Screen::Offer && options.unwrap_or(0) == 0 {
                if initiator
                    && state.option_sent_at.is_none_or(|last| {
                        now.saturating_duration_since(last) >= CHALLENGE_COOLDOWN
                    })
                {
                    state.option_sent_at = Some(now);
                    cx.emit(InteractReq::IfButton {
                        component_id: controls.obstacles,
                    });
                }
                return true;
            }
            let Some(options) = options else {
                return true;
            };
            if options != RULES {
                return reject(
                    UNSAFE_RULES,
                    "duel rules are not obstacle-only",
                    cx,
                    &mut state,
                    logs,
                );
            }
            if !waiting(&scene, status_id) && (initiator || other_accepted(&scene, status_id)) {
                cx.emit(InteractReq::DuelAccept {
                    screen: if current == Screen::Offer {
                        "offer".into()
                    } else {
                        "confirm".into()
                    },
                    partner: partner.to_string(),
                    rules: RULES,
                });
            }
            true
        })
    })
}

#[derive(Deserialize)]
pub(crate) struct HandshakeArgs {
    partner: String,
    #[serde(default)]
    initiator: bool,
}

/// Frozen `ClueDuelHandshake.tick()`.
pub(crate) struct Handshake(HandshakeArgs);

impl Family for Handshake {
    const NAME: &'static str = "clue_duel_handshake";
    const KICK_ON_START: bool = true;
    const CALLBACKS: &'static [&'static str] = &["log"];
    type Args = HandshakeArgs;
    type Output = bool;

    fn begin(args: Self::Args, _cx: &mut Cx<'_>) -> Begin<Self> {
        if controls().is_none() || duel_name(&args.partner).is_empty() {
            Begin::Refuse("duel-unavailable".into())
        } else {
            Begin::Run(Self(args))
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Self::Output> {
        let mut logs = VecDeque::new();
        let result = handshake_tick(&self.0.partner, self.0.initiator, cx, &mut logs);
        if !flush_logs(cx, LOG, &mut logs) {
            return Step::Wait;
        }
        Step::Done(result)
    }
}

fn forfeit_loc(scene: &observed::Lens<'_>) -> Option<SceneRow> {
    let here = scene.here()?;
    scene
        .locs()?
        .iter()
        .filter(|loc| {
            loc.actions
                .iter()
                .any(|action| action.eq_ignore_ascii_case("Forfeit"))
        })
        .min_by_key(|loc| (loc.x - here.x).abs().max((loc.z - here.z).abs()))
        .cloned()
}

/// Frozen `leaveClueDuel`: forfeit at the nearest Forfeit loc, answer
/// 'Yes', then wait to stand outside every pen.
#[derive(Clone, Copy, Default)]
enum Leave {
    #[default]
    Start,
    WaitYes {
        since: Instant,
    },
    WaitOutside {
        since: Instant,
    },
}

impl Leave {
    /// `Some(true)` outside a pen, `Some(false)` when there is no Forfeit
    /// loc or a stage timed out, `None` while leaving.
    fn advance(&mut self, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Option<bool> {
        observed::with(|scene| {
            let scene = scene.latest();
            if pen_at(scene.here().unwrap_or_default()).is_none() {
                return Some(true);
            }
            let now = Instant::now();
            match *self {
                Self::Start => {
                    logs.push_back("forfeiting the clue duel".into());
                    let Some(loc) = forfeit_loc(&scene) else {
                        return Some(false);
                    };
                    cx.emit(InteractReq::Loc {
                        x: loc.x,
                        z: loc.z,
                        level: loc.level,
                        action: "Forfeit".into(),
                        id: Some(loc.id),
                    });
                    *self = Self::WaitYes { since: now };
                    None
                }
                Self::WaitYes { since } => {
                    if let Some(option) = scene
                        .chat_options()
                        .and_then(|rows| rows.iter().position(|row| row.trim() == "Yes"))
                    {
                        cx.emit(InteractReq::Answer {
                            option: i32::try_from(option + 1).unwrap_or(1),
                        });
                        *self = Self::WaitOutside { since: now };
                        None
                    } else {
                        (now.saturating_duration_since(since) >= LEAVE_YES).then_some(false)
                    }
                }
                Self::WaitOutside { since } => {
                    (now.saturating_duration_since(since) >= LEAVE_EXIT).then_some(false)
                }
            }
        })
    }
}

#[derive(Default)]
struct HelperMemory {
    entered_at: Option<Instant>,
}

#[derive(Deserialize)]
pub(crate) struct HelperArgs {
    partner: String,
}

/// Frozen `ClueDuelHelper.execute()`: one pass, awaiting the lobby walk or
/// the leave it starts.
pub(crate) struct Helper {
    partner: String,
    phase: HelperPhase,
    logs: VecDeque<String>,
}

enum HelperPhase {
    Decide,
    Walking(Box<Resilient>),
    Leaving(Leave),
}

impl Family for Helper {
    const NAME: &'static str = "clue_duel_helper";
    const KICK_ON_START: bool = true;
    const CALLBACKS: &'static [&'static str] = &["log"];
    type Args = HelperArgs;
    type Output = bool;

    fn begin(args: Self::Args, _cx: &mut Cx<'_>) -> Begin<Self> {
        if controls().is_none() || duel_name(&args.partner).is_empty() {
            Begin::Refuse("duel-unavailable".into())
        } else {
            Begin::Run(Self {
                partner: args.partner,
                phase: HelperPhase::Decide,
                logs: VecDeque::new(),
            })
        }
    }

    fn release(&self) -> Option<InteractReq> {
        match &self.phase {
            HelperPhase::Walking(walk) => walk.release(),
            _ => None,
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Self::Output> {
        let out = self.drive(cx);
        if !flush_logs(cx, LOG, &mut self.logs) {
            return Step::Wait;
        }
        out.map_or(Step::Wait, Step::Done)
    }
}

impl Helper {
    fn drive(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        match &mut self.phase {
            HelperPhase::Walking(walk) => {
                let out = walk.step(cx);
                while let Some(line) = walk.pop_log() {
                    self.logs.push_back(line);
                }
                return out.map(|_| true);
            }
            HelperPhase::Leaving(leave) => return leave.advance(cx, &mut self.logs).map(|_| true),
            HelperPhase::Decide => {}
        }
        let Some(controls) = controls() else {
            return Some(false);
        };
        let (here, current) = observed::with(|scene| {
            let scene = scene.latest();
            (scene.here(), screen(&scene, controls))
        });
        if here.and_then(pen_at).is_some() {
            let expired = HELPER.with(|memory| {
                let entered = *memory
                    .borrow_mut()
                    .entered_at
                    .get_or_insert_with(Instant::now);
                entered.elapsed() >= HELPER_TIMEOUT
            });
            if !expired {
                return Some(true);
            }
            let mut leave = Leave::Start;
            let out = leave.advance(cx, &mut self.logs);
            self.phase = HelperPhase::Leaving(leave);
            return out.map(|_| true);
        }
        HELPER.with(|memory| memory.borrow_mut().entered_at = None);
        match current {
            Screen::Win => {
                cx.emit(InteractReq::CloseModal);
                Some(true)
            }
            Screen::Offer | Screen::Confirm => {
                Some(handshake_tick(&self.partner, false, cx, &mut self.logs))
            }
            _ if !here.is_some_and(|here| within(here, LOBBY, 4)) => {
                match Resilient::new(world(LOBBY), 3, LOBBY_WALK_MS, Some(LEG_ATTEMPTS), false)
                    .start(cx)
                {
                    Ok(walk) => {
                        self.phase = HelperPhase::Walking(Box::new(walk));
                        None
                    }
                    Err(_) => Some(true),
                }
            }
            _ => Some(handshake_tick(&self.partner, false, cx, &mut self.logs)),
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct TravelArgs {
    #[serde(default)]
    x: i32,
    #[serde(default)]
    z: i32,
    #[serde(default)]
    level: i32,
    #[serde(default)]
    radius: i32,
    #[serde(default)]
    leave_only: bool,
}

/// Frozen `walkAcrossClueDuel(dest, radius, log)`, and with `leave_only`
/// frozen `leaveClueDuel(log)`. The partner is the account's global one,
/// read when the travel begins.
pub(crate) struct Travel {
    dest: Tile,
    radius: i32,
    leave_only: bool,
    partner: String,
    state: TravelState,
    entry_attempt: u8,
    logs: VecDeque<String>,
}

enum TravelState {
    Start,
    Leaving { leave: Leave, then: AfterLeave },
    WalkingLobby(Box<Resilient>),
    Handshake,
    WalkingDest(Box<Resilient>),
}

/// What a finished leave goes on to.
#[derive(Clone, Copy)]
enum AfterLeave {
    /// `leaveClueDuel` itself: done.
    Done,
    /// Out of a pen that was not the target: carry on to the destination.
    Continue,
    /// Out of a wrongly assigned pen: the next entry attempt, if any.
    Retry,
}

impl Travel {
    /// The 3554 crossing a compiled clue trail drives (`walkAcrossClueDuel`).
    pub(crate) fn clue(x: i32, z: i32, level: i32, radius: i32) -> Self {
        Self::new(TravelArgs {
            x,
            z,
            level,
            radius,
            leave_only: false,
        })
    }

    fn new(args: TravelArgs) -> Self {
        Self {
            dest: Tile {
                x: args.x,
                z: args.z,
                level: args.level,
            },
            radius: args.radius,
            leave_only: args.leave_only,
            partner: global_partner(),
            state: TravelState::Start,
            entry_attempt: 0,
            logs: VecDeque::new(),
        }
    }

    /// A log line the travel queued for its owner's `log`.
    pub(crate) fn pop_log(&mut self) -> Option<String> {
        self.logs.pop_front()
    }

    fn leave(&mut self, cx: &mut Cx<'_>, then: AfterLeave) -> Option<bool> {
        self.state = TravelState::Leaving {
            leave: Leave::Start,
            then,
        };
        self.drive(cx)
    }

    /// Outside every pen: enter the target pen through the lobby handshake,
    /// or walk the destination leg.
    fn enter_or_walk(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        if pen_at(self.dest).is_none() {
            return self.walk_dest(cx);
        }
        let self_name = observed::with(|scene| {
            scene
                .latest()
                .my_name()
                .map(String::as_str)
                .unwrap_or_default()
                .to_string()
        });
        if !valid_partner(&self.partner, &self_name) {
            self.logs.push_back(NO_PARTNER.into());
            return Some(false);
        }
        if controls().is_none() {
            return Some(false);
        }
        self.walk_lobby(cx)
    }

    fn walk_lobby(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        match Resilient::new(world(LOBBY), 3, LOBBY_WALK_MS, Some(LEG_ATTEMPTS), false).start(cx) {
            Ok(walk) => {
                self.state = TravelState::WalkingLobby(Box::new(walk));
                None
            }
            Err(true) => self.begin_handshake(cx),
            Err(false) => Some(false),
        }
    }

    fn begin_handshake(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        fresh_handshake(&self.partner);
        self.logs
            .push_back(format!("waiting for clue helper {}", self.partner));
        cx.clock().arm(ENTRY_TIMEOUT_MS);
        self.state = TravelState::Handshake;
        None
    }

    fn walk_dest(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        match Resilient::new(
            world(self.dest),
            self.radius,
            DEST_WALK_MS,
            Some(LEG_ATTEMPTS),
            false,
        )
        .start(cx)
        {
            Ok(walk) => {
                self.state = TravelState::WalkingDest(Box::new(walk));
                None
            }
            Err(arrived) => Some(arrived),
        }
    }

    /// Drive one step: `Some(result)` once the travel ended, `None` while
    /// it runs. Emits ops and queues log lines; it never calls a hook, so
    /// an owning family forwards [`Travel::pop_log`] through its own.
    pub(crate) fn drive(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        let here = observed::with(|scene| scene.latest().here())?;
        let target = pen_at(self.dest);
        let here_pen = pen_at(here);
        match &mut self.state {
            TravelState::Start => {
                if self.leave_only {
                    return self.leave(cx, AfterLeave::Done);
                }
                if here_pen.is_some() && here_pen != target {
                    return self.leave(cx, AfterLeave::Continue);
                }
                if here_pen.is_some() {
                    // Already in the target pen: the destination leg.
                    return self.walk_dest(cx);
                }
                self.enter_or_walk(cx)
            }
            TravelState::Leaving { leave, then } => {
                let then = *then;
                match leave.advance(cx, &mut self.logs)? {
                    false => Some(false),
                    true => match then {
                        AfterLeave::Done => Some(true),
                        AfterLeave::Continue => self.enter_or_walk(cx),
                        AfterLeave::Retry if self.entry_attempt >= ENTRY_ATTEMPTS => Some(false),
                        AfterLeave::Retry => self.walk_lobby(cx),
                    },
                }
            }
            TravelState::WalkingLobby(walk) => {
                let out = walk.step(cx);
                while let Some(line) = walk.pop_log() {
                    self.logs.push_back(line);
                }
                match out? {
                    true => self.begin_handshake(cx),
                    false => Some(false),
                }
            }
            TravelState::Handshake => {
                let Some(controls) = controls() else {
                    return Some(false);
                };
                if here_pen.is_some() && here_pen == target {
                    return self.walk_dest(cx);
                }
                if here_pen.is_some() {
                    if duel_active(controls) {
                        cx.emit(InteractReq::CloseModal);
                    }
                    self.entry_attempt += 1;
                    self.logs
                        .push_back("assigned a different obstacle arena, retrying".into());
                    return self.leave(cx, AfterLeave::Retry);
                }
                if cx.clock().bound_reached() {
                    if duel_active(controls) {
                        cx.emit(InteractReq::CloseModal);
                    }
                    self.logs
                        .push_back("clue duel partner did not complete the handshake".into());
                    return Some(false);
                }
                if crate::event_signal::pending() {
                    if duel_active(controls) {
                        cx.emit(InteractReq::CloseModal);
                    }
                    return Some(false);
                }
                let partner = self.partner.clone();
                (!handshake_tick(&partner, true, cx, &mut self.logs)).then_some(false)
            }
            TravelState::WalkingDest(walk) => {
                let out = walk.step(cx);
                while let Some(line) = walk.pop_log() {
                    self.logs.push_back(line);
                }
                out
            }
        }
    }
}

impl Family for Travel {
    const NAME: &'static str = "clue_duel_travel";
    const EXCLUSIVE: bool = true;
    const KICK_ON_START: bool = true;
    const CALLBACKS: &'static [&'static str] = &["log"];
    type Args = TravelArgs;
    type Output = bool;

    fn begin(args: Self::Args, _cx: &mut Cx<'_>) -> Begin<Self> {
        if !(0..=32).contains(&args.radius) {
            Begin::Refuse("invalid-radius".into())
        } else {
            Begin::Run(Self::new(args))
        }
    }

    fn release(&self) -> Option<InteractReq> {
        match &self.state {
            TravelState::WalkingLobby(walk) | TravelState::WalkingDest(walk) => walk.release(),
            _ => None,
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Self::Output> {
        let out = self.drive(cx);
        if !flush_logs(cx, LOG, &mut self.logs) {
            return Step::Wait;
        }
        out.map_or(Step::Wait, Step::Done)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum CloseKind {
    Cancel,
    CloseWin,
}

#[derive(Deserialize)]
pub(crate) struct CloseArgs {
    kind: CloseKind,
}

/// Frozen `Duel.cancel()` / `Duel.closeWin()`: the screen gate, then
/// `Modals.close()` (one close, then the main modal changes within 3 s).
pub(crate) struct Close {
    kind: CloseKind,
    before: i32,
}

/// `(ingame, main modal, screen)` since the last login.
fn close_facts(controls: DuelControls) -> (bool, i32, Screen) {
    observed::with(|scene| {
        let session = scene.since_login();
        (
            session.ingame().unwrap_or(false),
            session.main_modal_id().unwrap_or(-1),
            screen(&session, controls),
        )
    })
}

impl Family for Close {
    const NAME: &'static str = "duel_close";
    const EXCLUSIVE: bool = true;
    type Args = CloseArgs;
    type Output = bool;

    fn begin(args: Self::Args, cx: &mut Cx<'_>) -> Begin<Self> {
        let Some(controls) = controls() else {
            return Begin::Refuse("duel-unavailable".into());
        };
        let (ingame, main, current) = close_facts(controls);
        let open = match args.kind {
            CloseKind::Cancel => matches!(current, Screen::Offer | Screen::Confirm),
            CloseKind::CloseWin => current == Screen::Win,
        };
        if !open || !ingame {
            return Begin::Done(false);
        }
        cx.clock().arm(crate::modals::CLOSE_TIMEOUT_MS);
        cx.emit(InteractReq::CloseModal);
        Begin::Run(Self {
            kind: args.kind,
            before: main,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Self::Output> {
        let Some(controls) = controls() else {
            return Step::Done(false);
        };
        let (ingame, main, current) = close_facts(controls);
        if !ingame {
            return Step::Done(false);
        }
        if main != self.before {
            return Step::Done(match self.kind {
                CloseKind::Cancel => !matches!(current, Screen::Offer | Screen::Confirm),
                CloseKind::CloseWin => true,
            });
        }
        if cx.clock().bound_reached() {
            return Step::Done(false);
        }
        Step::Wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arena_classification_is_exact_and_plane_bound() {
        assert_eq!(pen_at(CLUE_TILE), Some(3));
        assert_eq!(
            pen_at(Tile {
                level: 1,
                ..CLUE_TILE
            }),
            None
        );
        assert_eq!(
            pen_at(Tile {
                x: 3363,
                z: 3250,
                level: 0
            }),
            None
        );
    }

    #[test]
    fn names_compare_like_frozen_clue_duel_name() {
        assert_eq!(duel_name("  Some_Name  "), "some name");
        assert!(valid_partner("Some_Helper", "Other"));
        assert!(!valid_partner("", "Other"));
        assert!(!valid_partner("   ", "Other"));
        assert!(
            valid_partner("Some Helper", ""),
            "an unknown local name refuses only a missing partner"
        );
        assert!(!valid_partner(" some_helper ", "Some Helper"));
    }

    #[test]
    fn the_partner_header_keeps_the_display_name() {
        assert_eq!(
            duel_partner_name("Dueling with: SOME_name").as_deref(),
            Some("SOME_name")
        );
        assert_eq!(
            duel_partner_name("dueling with :  Helper Two ").as_deref(),
            Some("Helper Two")
        );
        assert_eq!(duel_partner_name("Helper").as_deref(), Some("Helper"));
        assert_eq!(duel_partner_name("Dueling with:"), None);
        assert_eq!(duel_partner_name("  "), None);
    }

    #[test]
    fn duel_offer_stall_boundary_is_thirty_seconds() {
        let now = Instant::now();
        let mut before = Some(now - OFFER_TIMEOUT + Duration::from_nanos(1));
        assert!(!offer_stalled(&mut before, now));
        let mut at = Some(now - OFFER_TIMEOUT);
        assert!(offer_stalled(&mut at, now));
    }
}
