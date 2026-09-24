//! Rust-owned `Reach.npcDialog` sequencing.
//!
//! JavaScript marshals name/stand/`openMs`/log and dispatches the returned
//! `walk-near` / `npc` verbs. Matching, clocks, freshness marks, one Clear
//! recovery and `done`/`retry`/`unreachable` stay here. Game actions reuse
//! the existing FlatBuffer walk and npc verbs.

use crate::isolate_fb::SnapshotReader;
use crate::task_clock::InstantTaskClock;
use crate::walk_wait;
use serde_json::{json, Value};
use std::cell::RefCell;

/// Frozen close-in / stand / Clear walk bound.
pub const WALK_BOUND_MS: u64 = 90_000;
/// Frozen `openMs` default.
pub const OPEN_MS: u64 = 15_000;
const CLOSE_IN_RADIUS: i32 = 3;
const STAND_RADIUS: i32 = 1;
const ADJACENT: i32 = 1;

thread_local! {
    static RUNTIME: RefCell<ReachRuntime> = const { RefCell::new(ReachRuntime::new()) };
    static OBSERVATION: RefCell<Observation> = const { RefCell::new(Observation::new()) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tile {
    x: i32,
    z: i32,
    level: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Npc {
    name: String,
    actions: Vec<String>,
    index: i32,
    tile: Tile,
    distance: i32,
    reachable_adj: bool,
}

#[derive(Clone)]
struct Observation {
    ingame: bool,
    here: Option<Tile>,
    hold: bool,
    ours: bool,
    chat_modal_id: i32,
    chat_continue: bool,
    chat_lines: Vec<(i32, String)>,
    npcs: Vec<Npc>,
}

impl Observation {
    const fn new() -> Self {
        Self {
            ingame: false,
            here: None,
            hold: false,
            ours: false,
            chat_modal_id: -1,
            chat_continue: false,
            chat_lines: Vec::new(),
            npcs: Vec::new(),
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
        if snap.has_here() {
            self.here = snap.here().map(|tile| Tile {
                x: tile.x(),
                z: tile.z(),
                level: tile.level(),
            });
        }
        if snap.has_hold() {
            self.hold = snap.hold();
        }
        if snap.has_ours() {
            self.ours = snap.ours();
        }
        if snap.has_chat_modal_id() {
            self.chat_modal_id = snap.chat_modal_id();
        }
        if snap.has_chat_continue() {
            self.chat_continue = snap.chat_continue();
        }
        if snap.has_chat_lines() {
            self.chat_lines = snap
                .chat_lines()
                .iter()
                .map(|line| (line.seq(), line.text().to_string()))
                .collect();
        }
        if snap.has_npcs() {
            self.npcs = snap
                .npcs()
                .iter()
                .map(|npc| Npc {
                    name: npc.name().unwrap_or_default().to_string(),
                    actions: npc
                        .actions()
                        .iter()
                        .filter(|action| !action.is_empty() && **action != "hidden")
                        .map(|action| (*action).to_string())
                        .collect(),
                    index: npc.index(),
                    tile: Tile {
                        x: npc.x(),
                        z: npc.z(),
                        level: npc.level(),
                    },
                    distance: npc.distance(),
                    reachable_adj: npc.reachable_adj(),
                })
                .collect();
        }
    }

    fn pending(&self) -> bool {
        self.hold || self.ours
    }

    fn is_open(&self) -> bool {
        self.chat_modal_id != -1
    }

    fn dialog_ready(&self) -> bool {
        self.is_open() || self.chat_continue
    }

    fn max_chat_seq(&self) -> i32 {
        self.chat_lines
            .iter()
            .map(|(seq, _)| *seq)
            .max()
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    CloseIn,
    WalkStand,
    WaitOpen,
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WalkSettle {
    Pending,
    Arrived,
    /// `walk_wait` settled false: matched `walk_outcome_failed` for this request.
    Failed,
    /// Reach 90s bound elapsed while `walk_wait` is still pending.
    Timeout,
}

struct ReachRuntime {
    clock: InstantTaskClock,
    token: u64,
    phase: Phase,
    npc_name: String,
    near: Tile,
    open_ms: u64,
    walk_token: u64,
    armed_modal: i32,
    armed_continue: bool,
    armed_seq: i32,
    cleared: bool,
    npc_action: String,
    npc_index: i32,
    npc_tile: Tile,
    interrupted: bool,
}

impl ReachRuntime {
    const fn new() -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token: 0,
            phase: Phase::Idle,
            npc_name: String::new(),
            near: Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            open_ms: OPEN_MS,
            walk_token: 0,
            armed_modal: -1,
            armed_continue: false,
            armed_seq: 0,
            cleared: false,
            npc_action: String::new(),
            npc_index: -1,
            npc_tile: Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            interrupted: false,
        }
    }

    fn frozen(&self) -> bool {
        self.clock.frozen()
    }

    fn set_freeze(&mut self, paused: bool, held: bool) {
        self.clock.set_freeze(paused, held);
    }

    fn arm(&mut self, window: u64) {
        self.clock.arm(window);
    }

    fn bound_reached(&self) -> bool {
        self.clock.bound_reached()
    }

    fn abort_runtime(&mut self) {
        self.token = self.token.wrapping_add(1);
        if self.token == 0 {
            self.token = 1;
        }
        self.phase = Phase::Idle;
        self.npc_name.clear();
        self.near = Tile {
            x: 0,
            z: 0,
            level: 0,
        };
        self.open_ms = OPEN_MS;
        self.walk_token = 0;
        self.armed_modal = -1;
        self.armed_continue = false;
        self.armed_seq = 0;
        self.cleared = false;
        self.npc_action.clear();
        self.npc_index = -1;
        self.interrupted = false;
        self.clock.deadline = None;
    }

    fn finish(&mut self, status: &str, log: Option<String>) -> Value {
        let token = self.token;
        self.phase = Phase::Idle;
        self.clock.deadline = None;
        with_log(
            json!({
                "kind": "done",
                "token": token,
                "status": status,
            }),
            log,
        )
    }

    fn wait(&self) -> Value {
        json!({ "kind": "wait", "token": self.token })
    }

    fn npc_verb(&self) -> Value {
        json!({
            "kind": "npc",
            "token": self.token,
            "name": self.npc_name,
            "action": self.npc_action,
            "index": self.npc_index,
        })
    }
}

pub fn on_snapshot(snap: &SnapshotReader<'_>) {
    walk_wait::on_snapshot(snap);
    OBSERVATION.with(|obs| {
        let mut obs = obs.borrow_mut();
        obs.update(snap);
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            if rt.phase != Phase::Idle && obs.pending() {
                rt.interrupted = true;
            }
        });
    });
}

pub fn on_pause() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().clock.held;
        rt.borrow_mut().set_freeze(true, held);
    });
}

pub fn on_resume() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().clock.held;
        rt.borrow_mut().set_freeze(false, held);
    });
}

pub fn on_hold(held: bool) {
    RUNTIME.with(|rt| {
        let paused = rt.borrow().clock.paused;
        rt.borrow_mut().set_freeze(paused, held);
    });
}

pub fn on_reset() {
    RUNTIME.with(|rt| rt.borrow_mut().abort_runtime());
    OBSERVATION.with(|obs| *obs.borrow_mut() = Observation::new());
}

pub fn dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => begin(input),
        "next" => next(input.get("token").and_then(Value::as_u64).unwrap_or(0)),
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

fn begin(input: &Value) -> Value {
    let npc_name = input
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let near = read_tile(input);
    let open_ms = match input.get("openMs") {
        Some(value) if value.is_u64() || value.is_i64() => value
            .as_u64()
            .or_else(|| value.as_i64().and_then(|n| u64::try_from(n).ok()))
            .unwrap_or(OPEN_MS),
        _ => OPEN_MS,
    };
    OBSERVATION.with(|o| {
        let obs = o.borrow();
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            rt.abort_runtime();
            rt.npc_name = npc_name;
            rt.near = near;
            rt.open_ms = open_ms;
            if !obs.ingame {
                return json!({ "kind": "aborted", "token": rt.token });
            }
            if obs.pending() {
                rt.interrupted = true;
                return rt.finish("retry", None);
            }
            start(&mut rt, &obs)
        })
    })
}

fn start(rt: &mut ReachRuntime, obs: &Observation) -> Value {
    if obs.is_open() {
        return if owned_adjacent(obs, &rt.npc_name) {
            rt.finish("done", None)
        } else {
            rt.finish("retry", None)
        };
    }
    match talk_target(&obs.npcs, &rt.npc_name) {
        None => {
            if within(obs.here, rt.near, CLOSE_IN_RADIUS) {
                return rt.finish("retry", None);
            }
            emit_walk(rt, Phase::CloseIn, rt.near, CLOSE_IN_RADIUS)
        }
        Some(npc) => {
            remember_npc(rt, npc);
            if within(obs.here, rt.near, STAND_RADIUS) {
                emit_talk(rt, obs)
            } else {
                emit_walk(rt, Phase::WalkStand, rt.near, STAND_RADIUS)
            }
        }
    }
}

fn next(token: u64) -> Value {
    OBSERVATION.with(|o| {
        let obs = o.borrow();
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            if token != rt.token || rt.phase == Phase::Idle {
                return json!({ "kind": "aborted", "token": rt.token });
            }
            if rt.frozen() {
                return rt.wait();
            }
            if !obs.ingame {
                return json!({ "kind": "aborted", "token": rt.token });
            }
            if rt.interrupted || obs.pending() {
                return rt.finish("retry", None);
            }
            match rt.phase {
                Phase::Idle => json!({ "kind": "aborted", "token": rt.token }),
                Phase::CloseIn | Phase::WalkStand | Phase::Clear => walk_step(&mut rt, &obs),
                Phase::WaitOpen => wait_open(&mut rt, &obs),
            }
        })
    })
}

fn walk_step(rt: &mut ReachRuntime, obs: &Observation) -> Value {
    match walk_settle(rt) {
        WalkSettle::Pending => rt.wait(),
        WalkSettle::Failed if rt.phase == Phase::Clear => rt.finish("unreachable", None),
        WalkSettle::Arrived | WalkSettle::Failed | WalkSettle::Timeout => match rt.phase {
            Phase::CloseIn => rt.finish("retry", None),
            Phase::WalkStand | Phase::Clear => match talk_target(&obs.npcs, &rt.npc_name) {
                Some(npc) => {
                    remember_npc(rt, npc);
                    emit_talk(rt, obs)
                }
                None => rt.finish("retry", None),
            },
            Phase::Idle | Phase::WaitOpen => rt.finish("retry", None),
        },
    }
}

fn wait_open(rt: &mut ReachRuntime, obs: &Observation) -> Value {
    if fresh_ready(rt, obs) {
        return rt.finish("done", None);
    }
    if fresh_cant_reach(rt, obs) {
        if rt.cleared {
            return rt.finish("unreachable", None);
        }
        let Some(npc) = talk_target(&obs.npcs, &rt.npc_name).or_else(|| {
            if rt.npc_index >= 0 {
                Some(Npc {
                    name: rt.npc_name.clone(),
                    actions: vec![rt.npc_action.clone()],
                    index: rt.npc_index,
                    tile: rt.npc_tile,
                    distance: 0,
                    reachable_adj: false,
                })
            } else {
                None
            }
        }) else {
            return rt.finish("retry", None);
        };
        if npc.reachable_adj {
            return rt.finish("unreachable", None);
        }
        remember_npc(rt, npc);
        rt.cleared = true;
        return emit_walk(rt, Phase::Clear, rt.npc_tile, STAND_RADIUS);
    }
    if rt.bound_reached() {
        return rt.finish("retry", None);
    }
    rt.wait()
}

fn emit_talk(rt: &mut ReachRuntime, obs: &Observation) -> Value {
    rt.armed_modal = obs.chat_modal_id;
    rt.armed_continue = obs.chat_continue;
    rt.armed_seq = obs.max_chat_seq();
    rt.phase = Phase::WaitOpen;
    rt.arm(rt.open_ms);
    rt.npc_verb()
}

fn emit_walk(rt: &mut ReachRuntime, phase: Phase, dest: Tile, radius: i32) -> Value {
    rt.phase = phase;
    rt.arm(WALK_BOUND_MS);
    let walk_token = walk_wait::dispatch(&json!({
        "op": "begin",
        "x": dest.x,
        "z": dest.z,
        "level": dest.level,
        "radius": radius,
        "allow_teleports": false,
    }))
    .as_u64()
    .unwrap_or(0);
    rt.walk_token = walk_token;
    json!({
        "kind": "walk-near",
        "token": rt.token,
        "x": dest.x,
        "z": dest.z,
        "level": dest.level,
        "radius": radius,
        "allow_teleports": false,
        "allow_wilderness": true,
        "allow_bank_fetch": true,
        "request_id": walk_token,
    })
}

fn walk_settle(rt: &ReachRuntime) -> WalkSettle {
    let settled = walk_wait::dispatch(&json!({
        "op": "settled",
        "token": rt.walk_token,
    }))
    .as_bool()
    .unwrap_or(false);
    if settled {
        if walk_wait::dispatch(&json!({
            "op": "value",
            "token": rt.walk_token,
        }))
        .as_bool()
        .unwrap_or(false)
        {
            WalkSettle::Arrived
        } else {
            WalkSettle::Failed
        }
    } else if rt.bound_reached() {
        WalkSettle::Timeout
    } else {
        WalkSettle::Pending
    }
}

fn remember_npc(rt: &mut ReachRuntime, npc: Npc) {
    rt.npc_name = npc.name;
    rt.npc_action = talk_op(&npc.actions).unwrap_or("Talk-to").to_string();
    rt.npc_index = npc.index;
    rt.npc_tile = npc.tile;
}

fn owned_adjacent(obs: &Observation, name: &str) -> bool {
    let Some(here) = obs.here else {
        return false;
    };
    let Some(npc) = nearest_named(&obs.npcs, name) else {
        return false;
    };
    npc.tile.level == here.level && chebyshev(npc.tile, here) <= ADJACENT
}

fn nearest_named<'a>(npcs: &'a [Npc], wanted: &str) -> Option<&'a Npc> {
    let want = wanted.trim().to_ascii_lowercase();
    if want.is_empty() {
        return None;
    }
    npcs.iter()
        .filter(|npc| npc.name.trim().to_ascii_lowercase() == want)
        .min_by_key(|npc| npc.distance)
}

fn talk_target(npcs: &[Npc], wanted: &str) -> Option<Npc> {
    let want = wanted.trim().to_ascii_lowercase();
    if want.is_empty() {
        return None;
    }
    npcs.iter()
        .filter(|npc| npc.name.trim().to_ascii_lowercase() == want)
        .filter_map(|npc| {
            let action = talk_op(&npc.actions)?;
            Some(Npc {
                name: npc.name.clone(),
                actions: vec![action.to_string()],
                index: npc.index,
                tile: npc.tile,
                distance: npc.distance,
                reachable_adj: npc.reachable_adj,
            })
        })
        .min_by_key(|npc| npc.distance)
}

fn talk_op(actions: &[String]) -> Option<&str> {
    actions.iter().find_map(|action| {
        action
            .get(..4)
            .is_some_and(|head| head.eq_ignore_ascii_case("talk"))
            .then_some(action.as_str())
    })
}

fn fresh_ready(rt: &ReachRuntime, obs: &Observation) -> bool {
    if !obs.dialog_ready() {
        return false;
    }
    obs.chat_modal_id != rt.armed_modal
        || (obs.is_open() && rt.armed_modal == -1)
        || (obs.chat_continue && !rt.armed_continue)
}

fn fresh_cant_reach(rt: &ReachRuntime, obs: &Observation) -> bool {
    obs.chat_lines
        .iter()
        .any(|(seq, text)| *seq > rt.armed_seq && is_cant_reach(text))
}

fn is_cant_reach(text: &str) -> bool {
    const PREFIX: &[u8] = b"i can't reach that";
    let bytes = text.as_bytes();
    bytes.len() >= PREFIX.len() && bytes[..PREFIX.len()].eq_ignore_ascii_case(PREFIX)
}

fn within(here: Option<Tile>, dest: Tile, radius: i32) -> bool {
    here.is_some_and(|here| here.level == dest.level && chebyshev(here, dest) <= radius)
}

fn chebyshev(a: Tile, b: Tile) -> i32 {
    (a.x - b.x).abs().max((a.z - b.z).abs())
}

fn read_tile(input: &Value) -> Tile {
    let src = input.get("near").unwrap_or(input);
    Tile {
        x: json_i32(src.get("x")),
        z: json_i32(src.get("z")),
        level: json_i32(src.get("level")),
    }
}

fn json_i32(value: Option<&Value>) -> i32 {
    value
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .unwrap_or(0)
}

fn with_log(mut value: Value, log: Option<String>) -> Value {
    if let Some(log) = log {
        value["log"] = json!(log);
    }
    value
}

#[cfg(test)]
pub(crate) fn expire_deadline_for_test() {
    use std::time::{Duration, Instant};
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.clock.deadline = Some(
            rt.clock
                .now()
                .checked_sub(Duration::from_millis(1))
                .unwrap_or_else(Instant::now),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isolate_fb::{
        encode_snapshot, encode_snapshot_with_native, ChatLineInput, NativeFactsInput,
        ReachViewInput, SceneEntityInput, SnapshotInput, TileInput,
    };

    fn npc<'a>(
        name: &'a str,
        actions: &'a [String],
        index: i32,
        x: i32,
        z: i32,
        distance: i32,
        reachable_adj: bool,
    ) -> SceneEntityInput<'a> {
        SceneEntityInput {
            index,
            id: 1,
            name: Some(name),
            x,
            z,
            level: 0,
            distance,
            health: 100,
            max_health: 100,
            in_combat: false,
            animating: false,
            actions,
            reachable: reachable_adj,
            reachable_adj,
            combat_level: 0,
            target_kind: 1,
            target_index: -1,
            size: 0,
            nx: 0,
            nz: 0,
        }
    }

    fn base<'a>() -> SnapshotInput<'a> {
        SnapshotInput {
            tick: 1,
            here: Some(TileInput {
                x: 5,
                z: 5,
                level: 0,
            }),
            ingame: true,
            inv: &[],
            inv_size: 28,
            stats: &[],
            booths: &[],
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
            side_tab: 0,
            varps: &[],
            combat_styles: &[],
            run_energy: 0,
            run_enabled: false,
            retaliate_enabled: false,
            my_name: Some("bot"),
            in_combat: false,
            animating: false,
            main_modal_id: -1,
            chat_modal_id: -1,
            make_products: &[],
            side_tab_ifaces: &[],
            spell_buttons: &[],
            chat_lines: &[],
            nearest_booth: None,
            bank_note_on: -1,
            bank_note_off: -1,
            scene_state: 2,
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

    fn observe(input: &SnapshotInput<'_>, native: NativeFactsInput<'_>) {
        let bytes = encode_snapshot_with_native(input, native);
        let snap = SnapshotReader::from_bytes(&bytes).expect("snapshot");
        on_snapshot(&snap);
    }

    fn begin_named(name: &str, x: i32, z: i32, open_ms: Option<u64>) -> Value {
        let mut payload = json!({
            "op": "begin",
            "name": name,
            "x": x,
            "z": z,
            "level": 0,
        });
        if let Some(open_ms) = open_ms {
            payload["openMs"] = json!(open_ms);
        }
        dispatch(&payload)
    }

    fn next_token(step: &Value) -> Value {
        dispatch(&json!({
            "op": "next",
            "token": step["token"].as_u64().unwrap_or(0),
        }))
    }

    fn fail_native(request_id: u64, x: i32, z: i32, radius: i32) -> NativeFactsInput<'static> {
        NativeFactsInput {
            walk_outcome_seq: 1,
            walk_outcome_generation: 1,
            walk_outcome_request_id: request_id,
            walk_outcome_failed: true,
            walk_outcome_x: x,
            walk_outcome_z: z,
            walk_outcome_level: 0,
            walk_outcome_radius: radius,
            walk_outcome_allow_teleports: false,
            ..Default::default()
        }
    }

    fn reset() {
        on_reset();
        walk_wait::on_reset();
        let bytes = encode_snapshot(&base());
        let snap = SnapshotReader::from_bytes(&bytes).expect("base");
        on_snapshot(&snap);
    }

    #[test]
    fn talk_op_matches_dialog_rule() {
        assert_eq!(
            talk_op(&["Examine".into(), "Talk-to".into()]),
            Some("Talk-to")
        );
        assert_eq!(talk_op(&["Talk".into()]), Some("Talk"));
        assert_eq!(talk_op(&["Bank".into()]), None);
        assert_eq!(talk_op(&["abcéxxxx".into()]), None);
    }

    #[test]
    fn open_chat_adjacent_name_without_talk_op_is_done() {
        reset();
        let actions = ["Bank".to_string()];
        let npcs = [npc("Traiborn", &actions, 3, 5, 6, 1, true)];
        let mut snap = base();
        snap.npcs = &npcs;
        snap.chat_modal_id = 968;
        observe(&snap, NativeFactsInput::default());
        let step = begin_named("Traiborn", 10, 10, None);
        assert_eq!(step["status"], "done");
        assert_ne!(step["kind"], "walk-near");
        assert_ne!(step["kind"], "npc");
    }

    #[test]
    fn open_chat_named_npc_not_adjacent_is_retry_without_talk() {
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 3, 20, 20, 10, true)];
        let mut snap = base();
        snap.npcs = &npcs;
        snap.chat_modal_id = 968;
        observe(&snap, NativeFactsInput::default());
        let step = begin_named("Traiborn", 5, 5, None);
        assert_eq!(step["status"], "retry");
        assert_ne!(step["kind"], "npc");
    }

    #[test]
    fn continue_without_open_does_not_own_the_chat() {
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 3, 5, 6, 1, true)];
        let mut snap = base();
        snap.npcs = &npcs;
        snap.chat_continue = true;
        snap.chat_modal_id = -1;
        observe(&snap, NativeFactsInput::default());
        let step = begin_named("Traiborn", 5, 5, None);
        assert_eq!(step["kind"], "npc");
        assert_eq!(step["action"], "Talk-to");
    }

    #[test]
    fn missing_npc_already_inside_close_in_retries() {
        reset();
        observe(&base(), NativeFactsInput::default());
        let step = begin_named("Traiborn", 6, 5, None);
        assert_eq!(step["status"], "retry");
        assert_ne!(step["kind"], "walk-near");
    }

    #[test]
    fn missing_npc_walks_close_in_then_retries_on_arrival_or_fail() {
        reset();
        let mut far = base();
        far.here = Some(TileInput {
            x: 0,
            z: 0,
            level: 0,
        });
        observe(&far, NativeFactsInput::default());
        let walk = begin_named("Traiborn", 20, 20, None);
        assert_eq!(walk["kind"], "walk-near");
        assert_eq!(walk["radius"], 3);
        assert_ne!(walk["request_id"], 0);

        far.here = Some(TileInput {
            x: 20,
            z: 20,
            level: 0,
        });
        far.tick = 2;
        observe(&far, NativeFactsInput::default());
        let after = next_token(&walk);
        assert_eq!(after["status"], "retry");
    }

    #[test]
    fn stand_generic_fail_still_talks() {
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 9, 8, 5, 2, false)];
        let mut snap = base();
        snap.here = Some(TileInput {
            x: 0,
            z: 0,
            level: 0,
        });
        snap.npcs = &npcs;
        observe(&snap, NativeFactsInput::default());
        let walk = begin_named("Traiborn", 5, 5, None);
        assert_eq!(walk["kind"], "walk-near");
        assert_eq!(walk["radius"], 1);
        let request_id = walk["request_id"].as_u64().unwrap();
        snap.tick = 2;
        observe(&snap, fail_native(request_id, 5, 5, 1));
        let talk = next_token(&walk);
        assert_eq!(talk["kind"], "npc");
        assert_eq!(talk["index"], 9);
        assert_ne!(talk["status"], "unreachable");
    }

    #[test]
    fn fresh_dialog_after_talk_is_done() {
        reset();
        let actions = ["Examine".to_string(), "Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 4, 6, 5, 1, true)];
        let mut snap = base();
        snap.npcs = &npcs;
        observe(&snap, NativeFactsInput::default());
        let talk = begin_named("Traiborn", 5, 5, None);
        assert_eq!(talk["kind"], "npc");
        assert_eq!(talk["action"], "Talk-to");
        snap.tick = 2;
        snap.chat_modal_id = 241;
        observe(&snap, NativeFactsInput::default());
        assert_eq!(next_token(&talk)["status"], "done");
    }

    #[test]
    fn leftover_continue_is_not_done_until_new_ready_state() {
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 4, 6, 5, 1, true)];
        let mut snap = base();
        snap.npcs = &npcs;
        snap.chat_continue = true;
        observe(&snap, NativeFactsInput::default());
        let talk = begin_named("Traiborn", 5, 5, None);
        assert_eq!(talk["kind"], "npc");
        snap.tick = 2;
        observe(&snap, NativeFactsInput::default());
        assert_eq!(next_token(&talk)["kind"], "wait");
        snap.tick = 3;
        snap.chat_modal_id = 241;
        observe(&snap, NativeFactsInput::default());
        assert_eq!(next_token(&talk)["status"], "done");
    }

    #[test]
    fn leftover_cant_reach_does_not_clear() {
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 4, 8, 5, 2, false)];
        let stale = [ChatLineInput {
            seq: 4,
            text: "I can't reach that!",
            type_: 0,
            username: None,
        }];
        let mut snap = base();
        snap.npcs = &npcs;
        snap.chat_lines = &stale;
        observe(&snap, NativeFactsInput::default());
        let talk = begin_named("Traiborn", 5, 5, None);
        assert_eq!(talk["kind"], "npc");
        snap.tick = 2;
        observe(&snap, NativeFactsInput::default());
        assert_eq!(next_token(&talk)["kind"], "wait");
    }

    #[test]
    fn fresh_cant_reach_with_reachable_adj_is_unreachable_without_clear() {
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 4, 8, 5, 2, true)];
        let mut snap = base();
        snap.npcs = &npcs;
        observe(&snap, NativeFactsInput::default());
        let talk = begin_named("Traiborn", 5, 5, None);
        let fresh = [ChatLineInput {
            seq: 8,
            text: "I can't reach that!",
            type_: 0,
            username: None,
        }];
        snap.tick = 2;
        snap.chat_lines = &fresh;
        observe(&snap, NativeFactsInput::default());
        let step = next_token(&talk);
        assert_eq!(step["status"], "unreachable");
        assert_ne!(step["kind"], "walk-near");
    }

    #[test]
    fn one_clear_then_second_cant_reach_is_unreachable() {
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 4, 8, 5, 2, false)];
        let mut snap = base();
        snap.npcs = &npcs;
        observe(&snap, NativeFactsInput::default());
        let talk = begin_named("Traiborn", 5, 5, None);
        let fresh = [ChatLineInput {
            seq: 8,
            text: "I can't reach that!",
            type_: 0,
            username: None,
        }];
        snap.tick = 2;
        snap.chat_lines = &fresh;
        observe(&snap, NativeFactsInput::default());
        let clear = next_token(&talk);
        assert_eq!(clear["kind"], "walk-near");
        assert_eq!(clear["x"], 8);
        assert_eq!(clear["z"], 5);
        assert_eq!(clear["radius"], 1);

        snap.tick = 3;
        snap.here = Some(TileInput {
            x: 8,
            z: 5,
            level: 0,
        });
        observe(&snap, NativeFactsInput::default());
        let talk_again = next_token(&clear);
        assert_eq!(talk_again["kind"], "npc");

        let later = [ChatLineInput {
            seq: 9,
            text: "I can't reach that!",
            type_: 0,
            username: None,
        }];
        snap.tick = 4;
        snap.chat_lines = &later;
        observe(&snap, NativeFactsInput::default());
        let done = next_token(&talk_again);
        assert_eq!(done["status"], "unreachable");
        assert_ne!(done["kind"], "walk-near");
    }

    #[test]
    fn clear_correlated_fail_is_unreachable_without_another_talk() {
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 4, 8, 5, 2, false)];
        let mut snap = base();
        snap.npcs = &npcs;
        observe(&snap, NativeFactsInput::default());
        let talk = begin_named("Traiborn", 5, 5, None);
        let fresh = [ChatLineInput {
            seq: 8,
            text: "I can't reach that!",
            type_: 0,
            username: None,
        }];
        snap.tick = 2;
        snap.chat_lines = &fresh;
        observe(&snap, NativeFactsInput::default());
        let clear = next_token(&talk);
        assert_eq!(clear["kind"], "walk-near");
        let request_id = clear["request_id"].as_u64().unwrap();
        snap.tick = 3;
        observe(&snap, fail_native(request_id, 8, 5, 1));
        let step = next_token(&clear);
        assert_eq!(step["status"], "unreachable");
        assert_ne!(
            step["kind"], "npc",
            "correlated Clear fail must not Talk again"
        );
    }

    #[test]
    fn clear_caller_timeout_still_talks_once() {
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 4, 8, 5, 2, false)];
        let mut snap = base();
        snap.npcs = &npcs;
        observe(&snap, NativeFactsInput::default());
        let talk = begin_named("Traiborn", 5, 5, None);
        let fresh = [ChatLineInput {
            seq: 8,
            text: "I can't reach that!",
            type_: 0,
            username: None,
        }];
        snap.tick = 2;
        snap.chat_lines = &fresh;
        observe(&snap, NativeFactsInput::default());
        let clear = next_token(&talk);
        assert_eq!(clear["kind"], "walk-near");
        expire_deadline_for_test();
        let step = next_token(&clear);
        assert_eq!(step["kind"], "npc");
        assert_ne!(step["status"], "unreachable");
    }

    #[test]
    fn nearest_same_name_talk_index_wins() {
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [
            npc("Traiborn", &actions, 2, 9, 5, 4, true),
            npc("Traiborn", &actions, 11, 6, 5, 1, true),
        ];
        let mut snap = base();
        snap.npcs = &npcs;
        observe(&snap, NativeFactsInput::default());
        let talk = begin_named("Traiborn", 5, 5, None);
        assert_eq!(talk["index"], 11);
    }

    #[test]
    fn open_ms_zero_times_out_retry() {
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 4, 6, 5, 1, true)];
        let mut snap = base();
        snap.npcs = &npcs;
        observe(&snap, NativeFactsInput::default());
        let talk = begin_named("Traiborn", 5, 5, Some(0));
        assert_eq!(talk["kind"], "npc");
        expire_deadline_for_test();
        assert_eq!(next_token(&talk)["status"], "retry");
    }

    #[test]
    fn pause_holds_clock_and_reset_aborts_old_token() {
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 4, 6, 5, 1, true)];
        let mut snap = base();
        snap.npcs = &npcs;
        observe(&snap, NativeFactsInput::default());
        let talk = begin_named("Traiborn", 5, 5, None);
        on_pause();
        assert_eq!(next_token(&talk)["kind"], "wait");
        on_resume();
        snap.ours = true;
        observe(&snap, NativeFactsInput::default());
        assert_eq!(next_token(&talk)["status"], "retry");

        snap.ours = false;
        observe(&snap, NativeFactsInput::default());
        let again = begin_named("Traiborn", 5, 5, None);
        let stale = talk["token"].as_u64().unwrap();
        on_reset();
        assert_eq!(
            dispatch(&json!({ "op": "next", "token": stale }))["kind"],
            "aborted"
        );
        assert_eq!(
            dispatch(&json!({
                "op": "next",
                "token": again["token"].as_u64().unwrap()
            }))["kind"],
            "aborted"
        );
    }

    #[test]
    fn second_begin_invalidates_first_token() {
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 4, 6, 5, 1, true)];
        let mut snap = base();
        snap.npcs = &npcs;
        observe(&snap, NativeFactsInput::default());
        let first = begin_named("Traiborn", 5, 5, None);
        let second = begin_named("Traiborn", 5, 5, None);
        assert_ne!(first["token"], second["token"]);
        assert_eq!(next_token(&first)["kind"], "aborted");
        assert_eq!(second["kind"], "npc");
    }

    #[test]
    fn close_in_generic_fail_is_retry_not_unreachable() {
        reset();
        let mut far = base();
        far.here = Some(TileInput {
            x: 0,
            z: 0,
            level: 0,
        });
        observe(&far, NativeFactsInput::default());
        let walk = begin_named("Traiborn", 20, 20, None);
        let request_id = walk["request_id"].as_u64().unwrap();
        far.tick = 2;
        observe(&far, fail_native(request_id, 20, 20, 3));
        assert_eq!(next_token(&walk)["status"], "retry");
    }
}
