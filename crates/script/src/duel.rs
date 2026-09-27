//! Clue Duel Arena protocol.
//!
//! The compatibility modules are only await/name adapters. Partner matching,
//! no-stake/rule validation, timing, retries, leaving and travel live here.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use api::game_data::DuelControls;
use serde::Deserialize;

use crate::machine::{Begin, Cx, Family, Step};
use crate::observed::{self, SceneRow, Tile};
use crate::shim::InteractReq;

const RULES: i32 = 1024;
const LOBBY: Tile = Tile {
    x: 3368,
    z: 3274,
    level: 0,
};
const CHALLENGE_COOLDOWN: Duration = Duration::from_secs(5);
const OFFER_TIMEOUT: Duration = Duration::from_secs(30);
const ENTRY_TIMEOUT_MS: u64 = 60_000;
const WALK_TIMEOUT_MS: u64 = 45_000;
const HELPER_TIMEOUT: Duration = Duration::from_secs(180);
const LEAVE_STEP_TIMEOUT: Duration = Duration::from_secs(10);

const PENS: [(i32, i32, i32, i32); 6] = [
    (3333, 3357, 3244, 3258),
    (3364, 3388, 3225, 3239),
    (3333, 3357, 3206, 3220),
    (3364, 3388, 3244, 3258),
    (3333, 3357, 3225, 3239),
    (3364, 3388, 3206, 3220),
];

thread_local! {
    static CONTROLS: RefCell<Option<DuelControls>> = const { RefCell::new(None) };
    static HANDSHAKE: RefCell<HandshakeMemory> = RefCell::new(HandshakeMemory::default());
    static HELPER: RefCell<HelperMemory> = RefCell::new(HelperMemory::default());
}

pub(crate) fn configure(data: Option<&api::game_data::SelectedGameData>) {
    CONTROLS.with(|slot| *slot.borrow_mut() = data.and_then(|data| data.duel_controls().copied()));
}

pub(crate) fn on_reset() {
    HANDSHAKE.with(|state| *state.borrow_mut() = HandshakeMemory::default());
    HELPER.with(|state| *state.borrow_mut() = HelperMemory::default());
}

fn controls() -> Option<DuelControls> {
    CONTROLS.with(|slot| *slot.borrow())
}

fn normalize(name: &str) -> String {
    name.replace('_', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn partner_header(text: &str) -> String {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    let name = lower
        .strip_prefix("dueling with:")
        .map(str::trim)
        .unwrap_or(lower.as_str());
    normalize(name)
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

#[derive(Default)]
struct HandshakeMemory {
    partner: String,
    opened_at: Option<Instant>,
    challenged_at: Option<Instant>,
    option_sent_at: Option<Instant>,
    validated_offer: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Closed,
    Offer,
    Confirm,
    Win,
    Other,
}
pub(crate) fn dispatch(value: &serde_json::Value) -> serde_json::Value {
    let op = value
        .get("op")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    match op {
        "name" => serde_json::json!(normalize(
            value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
        )),
        "valid-partner" => serde_json::json!(valid_partner(
            value
                .get("partner")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(""),
            value
                .get("self_name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(""),
        )),
        "header" => {
            let Some(text) = value.get("text").and_then(serde_json::Value::as_str) else {
                return serde_json::Value::Null;
            };
            let parsed = partner_header(text);
            if parsed.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::json!(parsed)
            }
        }
        "pen" => {
            let Some(tile) = json_tile(value.get("tile")) else {
                return serde_json::Value::Null;
            };
            let Some(index) = pen_at(tile) else {
                return serde_json::Value::Null;
            };
            let (min_x, max_x, min_z, max_z) = PENS[index];
            serde_json::json!({
                "minX": min_x,
                "maxX": max_x,
                "minZ": min_z,
                "maxZ": max_z,
            })
        }
        "crosses" => {
            let Some(dest) = json_tile(value.get("tile")) else {
                return serde_json::json!(false);
            };
            let here_in_pen =
                observed::with(|scene| scene.latest().here().and_then(pen_at).is_some());
            serde_json::json!(
                (dest
                    == Tile {
                        x: 3374,
                        z: 3250,
                        level: 0
                    })
                    || here_in_pen
            )
        }
        "facts" => {
            let Some(controls) = controls() else {
                return serde_json::Value::Null;
            };
            observed::with(|scene| {
                let scene = scene.latest();
                let current = screen(&scene, controls);
                let partner = widget(&scene, controls.select_partner)
                    .map(|(text, _)| partner_header(&text))
                    .filter(|name| !name.is_empty());
                let status = match current {
                    Screen::Offer => controls.select_status,
                    Screen::Confirm => controls.confirm_status,
                    _ => -1,
                };
                serde_json::json!({
                    "offer": current == Screen::Offer,
                    "confirm": current == Screen::Confirm,
                    "win": current == Screen::Win,
                    "partner": partner,
                    "waiting": status >= 0 && waiting(&scene, status),
                })
            })
        }
        _ => serde_json::Value::Null,
    }
}

fn json_tile(value: Option<&serde_json::Value>) -> Option<Tile> {
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

fn reject(reason: &str, cx: &mut Cx<'_>, state: &mut HandshakeMemory) -> bool {
    if std::env::var_os("BOT_DEBUG").is_some() {
        eprintln!("[clue-duel] rejecting handshake: {reason}");
    }
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
fn handshake_tick(partner: &str, initiator: bool, cx: &mut Cx<'_>) -> bool {
    let Some(controls) = controls() else {
        return false;
    };
    let expected = normalize(partner);
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
                        normalize(player.name_or_empty()) == expected
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
                if std::env::var_os("BOT_DEBUG").is_some() {
                    let status_id = match current {
                        Screen::Offer => controls.select_status,
                        Screen::Confirm => controls.confirm_status,
                        _ => -1,
                    };
                    eprintln!(
                        "[clue-duel] timeout initiator={initiator} screen={current:?} options={:?} status={:?}",
                        varp(&scene, controls.options_varp),
                        widget(&scene, status_id).map(|(text, _)| text)
                    );
                }
                return reject("offer timed out", cx, &mut state);
            }
            let (mine_id, theirs_id, status_id) = match current {
                Screen::Offer => (
                    controls.select_mine,
                    controls.select_theirs,
                    controls.select_status,
                ),
                Screen::Confirm => (
                    controls.confirm_mine,
                    controls.confirm_theirs,
                    controls.confirm_status,
                ),
                _ => unreachable!(),
            };
            let mine = widget(&scene, mine_id).map(|(_, count)| count);
            let theirs = widget(&scene, theirs_id).map(|(_, count)| count);
            if current == Screen::Offer && (mine.is_none() || theirs.is_none()) {
                return true;
            }
            if mine.is_some_and(|count| count != 0)
                || theirs.is_some_and(|count| count != 0)
            {
                return reject("stake container is not empty", cx, &mut state);
            }
            if current == Screen::Offer {
                let actual =
                    widget(&scene, controls.select_partner).map(|(text, _)| partner_header(&text));
                if actual.as_deref() != Some(expected.as_str()) {
                    return reject("offer partner does not match", cx, &mut state);
                }
                state.validated_offer = true;
            } else if !state.validated_offer {
                return reject("confirm arrived without a validated offer", cx, &mut state);
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
                return reject("duel rules are not obstacle-only", cx, &mut state);
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

pub(crate) fn valid_partner(partner: &str, self_name: &str) -> bool {
    let partner = normalize(partner);
    let self_name = normalize(self_name);
    !partner.is_empty() && !self_name.is_empty() && partner != self_name
}

#[derive(Deserialize)]
pub(crate) struct HandshakeArgs {
    partner: String,
    #[serde(default)]
    initiator: bool,
}

pub(crate) struct Handshake(HandshakeArgs);

impl Family for Handshake {
    const NAME: &'static str = "clue_duel_handshake";
    const KICK_ON_START: bool = true;
    type Args = HandshakeArgs;
    type Output = bool;

    fn begin(args: Self::Args, _cx: &mut Cx<'_>) -> Begin<Self> {
        if controls().is_none() || normalize(&args.partner).is_empty() {
            Begin::Refuse("duel-unavailable".into())
        } else {
            Begin::Run(Self(args))
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Self::Output> {
        Step::Done(handshake_tick(&self.0.partner, self.0.initiator, cx))
    }
}

#[derive(Default)]
struct HelperMemory {
    entered_at: Option<Instant>,
    leaving: LeaveState,
}

#[derive(Clone, Copy, Default)]
enum LeaveState {
    #[default]
    Idle,
    WaitYes {
        since: Instant,
    },
    WaitOutside {
        since: Instant,
    },
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

fn advance_leave(cx: &mut Cx<'_>, state: &mut LeaveState) -> Option<bool> {
    observed::with(|scene| {
        let scene = scene.latest();
        if pen_at(scene.here().unwrap_or_default()).is_none() {
            *state = LeaveState::Idle;
            return Some(true);
        }
        match *state {
            LeaveState::Idle => {
                let loc = forfeit_loc(&scene)?;
                cx.emit(InteractReq::Loc {
                    x: loc.x,
                    z: loc.z,
                    level: loc.level,
                    action: "Forfeit".into(),
                    id: Some(loc.id),
                });
                *state = LeaveState::WaitYes {
                    since: Instant::now(),
                };
                None
            }
            LeaveState::WaitYes { since } => {
                if let Some(option) = scene
                    .chat_options()
                    .and_then(|rows| rows.iter().position(|row| row.trim() == "Yes"))
                {
                    cx.emit(InteractReq::Answer {
                        option: i32::try_from(option + 1).unwrap_or(1),
                    });
                    *state = LeaveState::WaitOutside {
                        since: Instant::now(),
                    };
                } else if Instant::now().saturating_duration_since(since) >= LEAVE_STEP_TIMEOUT {
                    *state = LeaveState::Idle;
                }
                None
            }
            LeaveState::WaitOutside { since } => {
                if Instant::now().saturating_duration_since(since) >= LEAVE_STEP_TIMEOUT {
                    *state = LeaveState::Idle;
                }
                None
            }
        }
    })
}

#[derive(Deserialize)]
pub(crate) struct HelperArgs {
    partner: String,
}

pub(crate) struct Helper(HelperArgs);

impl Family for Helper {
    const NAME: &'static str = "clue_duel_helper";
    const KICK_ON_START: bool = true;
    type Args = HelperArgs;
    type Output = bool;

    fn begin(args: Self::Args, _cx: &mut Cx<'_>) -> Begin<Self> {
        if controls().is_none() || normalize(&args.partner).is_empty() {
            Begin::Refuse("duel-unavailable".into())
        } else {
            Begin::Run(Self(args))
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Self::Output> {
        let controls = controls().expect("begin checked controls");
        let done = HELPER.with(|memory| {
            let mut memory = memory.borrow_mut();
            observed::with(|scene| {
                let scene = scene.latest();
                let here = scene.here().unwrap_or_default();
                if pen_at(here).is_some() {
                    let entered = *memory.entered_at.get_or_insert_with(Instant::now);
                    if Instant::now().saturating_duration_since(entered) >= HELPER_TIMEOUT {
                        let _ = advance_leave(cx, &mut memory.leaving);
                    }
                    return true;
                }
                memory.entered_at = None;
                memory.leaving = LeaveState::Idle;
                match screen(&scene, controls) {
                    Screen::Win => {
                        cx.emit(InteractReq::CloseModal);
                        true
                    }
                    Screen::Offer | Screen::Confirm => handshake_tick(&self.0.partner, false, cx),
                    _ if !within(here, LOBBY, 4) => {
                        cx.emit(InteractReq::WalkNear {
                            x: LOBBY.x,
                            z: LOBBY.z,
                            level: LOBBY.level,
                            radius: 3,
                            allow_teleports: false,
                            allow_wilderness: false,
                            allow_bank_fetch: false,
                            request_id: 0,
                            avoid: Vec::new(),
                        });
                        true
                    }
                    _ => handshake_tick(&self.0.partner, false, cx),
                }
            })
        });
        Step::Done(done)
    }
}

#[derive(Deserialize)]
pub(crate) struct TravelArgs {
    partner: String,
    #[serde(default)]
    self_name: String,
    x: i32,
    z: i32,
    level: i32,
    radius: i32,
    #[serde(default)]
    leave_only: bool,
    #[serde(default)]
    allow_teleports: bool,
    #[serde(default)]
    allow_wilderness: bool,
    #[serde(default)]
    allow_bank_fetch: bool,
}

pub(crate) struct Travel {
    args: TravelArgs,
    state: TravelState,
    entry_attempt: u8,
}

#[derive(Clone, Copy)]
enum TravelState {
    Start,
    Leaving(LeaveState),
    WalkingLobby,
    Handshake,
    WalkingDest,
}

impl Travel {
    pub(crate) fn clue(
        partner: String,
        self_name: String,
        x: i32,
        z: i32,
        level: i32,
        radius: i32,
    ) -> Self {
        Self {
            args: TravelArgs {
                partner,
                self_name,
                x,
                z,
                level,
                radius,
                leave_only: false,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
            },
            state: TravelState::Start,
            entry_attempt: 0,
        }
    }

    fn dest(&self) -> Tile {
        Tile {
            x: self.args.x,
            z: self.args.z,
            level: self.args.level,
        }
    }

    fn walk(cx: &mut Cx<'_>, dest: Tile, radius: i32, flags: (bool, bool, bool)) {
        cx.emit(InteractReq::WalkNear {
            x: dest.x,
            z: dest.z,
            level: dest.level,
            radius,
            allow_teleports: flags.0,
            allow_wilderness: flags.1,
            allow_bank_fetch: flags.2,
            request_id: 0,
            avoid: Vec::new(),
        });
    }

    fn walk_dest(&self, cx: &mut Cx<'_>) {
        Self::walk(
            cx,
            self.dest(),
            self.args.radius,
            (
                self.args.allow_teleports,
                self.args.allow_wilderness,
                self.args.allow_bank_fetch,
            ),
        );
    }
}

impl Family for Travel {
    const NAME: &'static str = "clue_duel_travel";
    const EXCLUSIVE: bool = true;
    const KICK_ON_START: bool = true;
    type Args = TravelArgs;
    type Output = bool;

    fn begin(args: Self::Args, _cx: &mut Cx<'_>) -> Begin<Self> {
        if !(0..=32).contains(&args.radius) {
            Begin::Refuse("invalid-radius".into())
        } else {
            Begin::Run(Self {
                args,
                state: TravelState::Start,
                entry_attempt: 0,
            })
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Self::Output> {
        let (here, interrupted) = observed::with(|scene| {
            let scene = scene.latest();
            (scene.here(), crate::event_signal::pending())
        });
        let Some(here) = here else {
            return Step::Wait;
        };
        if interrupted {
            if matches!(self.state, TravelState::Handshake) {
                cx.emit(InteractReq::CloseModal);
            }
            return Step::Done(false);
        }
        let dest = self.dest();
        let target_pen = pen_at(dest);
        let here_pen = pen_at(here);

        match self.state {
            TravelState::Start => {
                if self.args.leave_only || (here_pen.is_some() && here_pen != target_pen) {
                    if controls().is_none() {
                        return Step::Done(false);
                    }
                    self.state = TravelState::Leaving(LeaveState::Idle);
                    cx.clock().arm(10_000);
                    return Step::Wait;
                }
                if target_pen.is_some() && here_pen != target_pen {
                    if controls().is_none()
                        || !valid_partner(&self.args.partner, &self.args.self_name)
                    {
                        return Step::Done(false);
                    }
                    Self::walk(cx, LOBBY, 3, (false, false, false));
                    self.state = TravelState::WalkingLobby;
                    cx.clock().arm(60_000);
                    Step::Wait
                } else {
                    self.walk_dest(cx);
                    self.state = TravelState::WalkingDest;
                    cx.clock().arm(WALK_TIMEOUT_MS);
                    Step::Wait
                }
            }
            TravelState::Leaving(mut leave) => {
                if let Some(done) = advance_leave(cx, &mut leave) {
                    if self.args.leave_only {
                        return Step::Done(done);
                    }
                    if target_pen.is_none() {
                        self.walk_dest(cx);
                        self.state = TravelState::WalkingDest;
                        cx.clock().arm(WALK_TIMEOUT_MS);
                    } else {
                        Self::walk(cx, LOBBY, 3, (false, false, false));
                        self.state = TravelState::WalkingLobby;
                        cx.clock().arm(ENTRY_TIMEOUT_MS);
                    }
                    return Step::Wait;
                }
                self.state = TravelState::Leaving(leave);
                if cx.clock().bound_reached() {
                    Step::Done(false)
                } else {
                    Step::Wait
                }
            }
            TravelState::WalkingLobby => {
                if within(here, LOBBY, 3) {
                    self.state = TravelState::Handshake;
                    cx.clock().arm(ENTRY_TIMEOUT_MS);
                    Step::Wait
                } else if cx.clock().bound_reached() {
                    Step::Done(false)
                } else {
                    Step::Wait
                }
            }
            TravelState::Handshake => {
                if here_pen == target_pen && here_pen.is_some() {
                    self.walk_dest(cx);
                    self.state = TravelState::WalkingDest;
                    cx.clock().arm(WALK_TIMEOUT_MS);
                    return Step::Wait;
                }
                if let Some(actual) = here_pen {
                    if Some(actual) != target_pen {
                        self.entry_attempt += 1;
                        if self.entry_attempt >= 2 {
                            return Step::Done(false);
                        }
                        self.state = TravelState::Leaving(LeaveState::Idle);
                        cx.clock().arm(10_000);
                        return Step::Wait;
                    }
                }
                if cx.clock().bound_reached() || !handshake_tick(&self.args.partner, true, cx) {
                    Step::Done(false)
                } else {
                    Step::Wait
                }
            }
            TravelState::WalkingDest => {
                if within(here, dest, self.args.radius) {
                    Step::Done(true)
                } else if cx.clock().bound_reached() {
                    Step::Done(false)
                } else {
                    Step::Wait
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arena_classification_is_exact_and_plane_bound() {
        assert_eq!(
            pen_at(Tile {
                x: 3374,
                z: 3250,
                level: 0
            }),
            Some(3)
        );
        assert_eq!(
            pen_at(Tile {
                x: 3374,
                z: 3250,
                level: 1
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
    fn names_match_underscores_case_and_whitespace() {
        assert_eq!(normalize("  Some_Name  "), "some name");
        assert_eq!(partner_header("Dueling with: SOME_name"), "some name");
        assert!(valid_partner("Some_Helper", "Other"));
        assert!(!valid_partner("", "Other"));
        assert!(!valid_partner("Some Helper", ""));
        assert!(!valid_partner(" some_helper ", "Some Helper"));
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
