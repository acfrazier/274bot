//! Rust-owned `Reach.npcDialog` sequencing, the `reach-npc-dialog`
//! [`crate::machine`] family.
//!
//! JavaScript passes name/stand/`openMs` and awaits the status. The
//! `walk-near` / `npc` ops, matching, clocks, freshness marks, one Clear
//! recovery and `done`/`retry`/`unreachable` stay here. Game actions reuse
//! the existing FlatBuffer walk and npc verbs.

use crate::isolate_fb::SnapshotReader;
use crate::machine::{Begin, Cx, Family, Step};
use crate::observed::{self, Ops, Scene, Text};
use crate::shim::InteractReq;
use crate::walk_wait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::cell::Cell;

/// Frozen close-in / stand / Clear walk bound.
pub const WALK_BOUND_MS: u64 = 90_000;
/// Frozen `openMs` default.
pub const OPEN_MS: u64 = 15_000;
const CLOSE_IN_RADIUS: i32 = 3;
const STAND_RADIUS: i32 = 1;
const ADJACENT: i32 = 1;

thread_local! {
    /// Posts that carried the `hold || ours` cooperative interrupt.
    static PENDING_POSTS: Cell<u64> = const { Cell::new(0) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tile {
    x: i32,
    z: i32,
    level: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Npc {
    name: Text,
    actions: Ops,
    index: i32,
    tile: Tile,
    distance: i32,
    reachable_adj: bool,
}

/// The posted facts this module decides from, read from the isolate scene.
#[derive(Clone)]
struct Observation {
    ingame: bool,
    here: Option<Tile>,
    hold: bool,
    ours: bool,
    chat_modal_id: i32,
    chat_continue: bool,
    chat_lines: Vec<(i32, Text)>,
    npcs: Vec<Npc>,
}

impl Observation {
    /// A logout forgets the session: only pages posted since login count.
    fn from_scene(scene: &Scene) -> Self {
        let session = scene.since_login();
        Self {
            ingame: session.ingame().unwrap_or(false),
            here: session.here().map(|tile| Tile {
                x: tile.x,
                z: tile.z,
                level: tile.level,
            }),
            hold: session.hold().unwrap_or(false),
            ours: session.ours().unwrap_or(false),
            chat_modal_id: session.chat_modal_id().unwrap_or(-1),
            chat_continue: session.chat_continue().unwrap_or(false),
            chat_lines: session
                .chat_lines()
                .map(|lines| {
                    lines
                        .iter()
                        .map(|line| (line.seq, line.text.clone()))
                        .collect()
                })
                .unwrap_or_default(),
            npcs: session
                .npcs()
                .map(|rows| {
                    rows.iter()
                        // Shared with the scene: no per-call string copies.
                        // `talk_op` never matches an empty or `hidden` slot.
                        .map(|npc| Npc {
                            name: npc.name.clone().unwrap_or_default(),
                            actions: npc.actions.clone(),
                            index: npc.index,
                            tile: Tile {
                                x: npc.x,
                                z: npc.z,
                                level: npc.level,
                            },
                            distance: npc.distance,
                            reachable_adj: npc.reachable_adj,
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// The posted `hold || ours` cooperative interrupt.
    fn pending_in(scene: &Scene) -> bool {
        let session = scene.since_login();
        session.hold().unwrap_or(false) || session.ours().unwrap_or(false)
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

/// After the isolate applied `snap` to the scene: count a post that carries
/// the cooperative interrupt, so a live reach or dialogue sees one posted
/// between its steps.
pub fn on_snapshot(snap: &SnapshotReader<'_>) {
    walk_wait::on_snapshot(snap);
    if observed::with(Observation::pending_in) {
        PENDING_POSTS.with(|posts| posts.set(posts.get() + 1));
    }
}

/// How many posts so far carried the cooperative interrupt.
pub(crate) fn pending_posts() -> u64 {
    PENDING_POSTS.with(Cell::get)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NpcDialogArgs {
    #[serde(default)]
    name: String,
    #[serde(default)]
    near: NearArg,
    /// A negative, fractional or absent value keeps the frozen 15 s.
    #[serde(default)]
    open_ms: Value,
}

#[derive(Default, Deserialize)]
struct NearArg {
    #[serde(default)]
    x: Value,
    #[serde(default)]
    z: Value,
    #[serde(default)]
    level: Value,
}

/// One frozen `Reach.npcDialog`.
pub(crate) struct NpcDialog {
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
    /// [`pending_posts`] when the reach began.
    pending_mark: u64,
}

impl Family for NpcDialog {
    const NAME: &'static str = "reach-npc-dialog";
    /// A new reach replaces the one in flight.
    const EXCLUSIVE: bool = true;
    type Args = NpcDialogArgs;
    /// `done`, `retry` or `unreachable`.
    type Output = &'static str;

    fn begin(args: NpcDialogArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let obs = observed::with(Observation::from_scene);
        if !obs.ingame || obs.pending() {
            return Begin::Done("retry");
        }
        let open_ms = args
            .open_ms
            .as_u64()
            .or_else(|| {
                args.open_ms
                    .as_f64()
                    .filter(|n| *n >= 0.0)
                    .map(|n| n as u64)
            })
            .unwrap_or(OPEN_MS);
        let mut reach = Self {
            phase: Phase::WaitOpen,
            npc_name: args.name.trim().to_string(),
            near: Tile {
                x: json_i32(&args.near.x),
                z: json_i32(&args.near.z),
                level: json_i32(&args.near.level),
            },
            open_ms,
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
            pending_mark: pending_posts(),
        };
        match reach.start(&obs, cx) {
            Step::Done(status) => Begin::Done(status),
            _ => Begin::Run(reach),
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<&'static str> {
        let obs = observed::with(Observation::from_scene);
        if !obs.ingame || pending_posts() != self.pending_mark || obs.pending() {
            return Step::Done("retry");
        }
        match self.phase {
            Phase::CloseIn | Phase::WalkStand | Phase::Clear => self.walk_step(&obs, cx),
            Phase::WaitOpen => self.wait_open(&obs, cx),
        }
    }
}

impl NpcDialog {
    fn start(&mut self, obs: &Observation, cx: &mut Cx<'_>) -> Step<&'static str> {
        if obs.is_open() {
            return Step::Done(if owned_adjacent(obs, &self.npc_name) {
                "done"
            } else {
                "retry"
            });
        }
        match talk_target(&obs.npcs, &self.npc_name) {
            None => {
                if within(obs.here, self.near, CLOSE_IN_RADIUS) {
                    return Step::Done("retry");
                }
                self.emit_walk(Phase::CloseIn, self.near, CLOSE_IN_RADIUS, cx)
            }
            Some(npc) => {
                self.remember_npc(npc);
                if within(obs.here, self.near, STAND_RADIUS) {
                    self.emit_talk(obs, cx)
                } else {
                    self.emit_walk(Phase::WalkStand, self.near, STAND_RADIUS, cx)
                }
            }
        }
    }

    fn walk_step(&mut self, obs: &Observation, cx: &mut Cx<'_>) -> Step<&'static str> {
        match self.walk_settle(cx) {
            WalkSettle::Pending => Step::Wait,
            WalkSettle::Failed if self.phase == Phase::Clear => Step::Done("unreachable"),
            WalkSettle::Arrived | WalkSettle::Failed | WalkSettle::Timeout => match self.phase {
                Phase::WalkStand | Phase::Clear => match talk_target(&obs.npcs, &self.npc_name) {
                    Some(npc) => {
                        self.remember_npc(npc);
                        self.emit_talk(obs, cx)
                    }
                    None => Step::Done("retry"),
                },
                Phase::CloseIn | Phase::WaitOpen => Step::Done("retry"),
            },
        }
    }

    fn wait_open(&mut self, obs: &Observation, cx: &mut Cx<'_>) -> Step<&'static str> {
        if self.fresh_ready(obs) {
            return Step::Done("done");
        }
        if self.fresh_cant_reach(obs) {
            if self.cleared {
                return Step::Done("unreachable");
            }
            let Some(npc) = talk_target(&obs.npcs, &self.npc_name).or_else(|| {
                (self.npc_index >= 0).then(|| Npc {
                    name: Text::from(self.npc_name.as_str()),
                    actions: std::iter::once(Text::from(self.npc_action.as_str())).collect(),
                    index: self.npc_index,
                    tile: self.npc_tile,
                    distance: 0,
                    reachable_adj: false,
                })
            }) else {
                return Step::Done("retry");
            };
            if npc.reachable_adj {
                return Step::Done("unreachable");
            }
            self.remember_npc(npc);
            self.cleared = true;
            return self.emit_walk(Phase::Clear, self.npc_tile, STAND_RADIUS, cx);
        }
        if cx.clock().bound_reached() {
            return Step::Done("retry");
        }
        Step::Wait
    }

    fn emit_talk(&mut self, obs: &Observation, cx: &mut Cx<'_>) -> Step<&'static str> {
        self.armed_modal = obs.chat_modal_id;
        self.armed_continue = obs.chat_continue;
        self.armed_seq = obs.max_chat_seq();
        self.phase = Phase::WaitOpen;
        cx.clock().arm(self.open_ms);
        cx.emit(InteractReq::Npc {
            name: self.npc_name.clone(),
            action: self.npc_action.clone(),
            index: Some(self.npc_index),
        });
        Step::Wait
    }

    fn emit_walk(
        &mut self,
        phase: Phase,
        dest: Tile,
        radius: i32,
        cx: &mut Cx<'_>,
    ) -> Step<&'static str> {
        self.phase = phase;
        cx.clock().arm(WALK_BOUND_MS);
        self.walk_token = walk_wait::dispatch(&json!({
            "op": "begin",
            "x": dest.x,
            "z": dest.z,
            "level": dest.level,
            "radius": radius,
            "allow_teleports": false,
        }))
        .as_u64()
        .unwrap_or(0);
        cx.emit(InteractReq::WalkNear {
            x: dest.x,
            z: dest.z,
            level: dest.level,
            radius,
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: true,
            request_id: self.walk_token,
        });
        Step::Wait
    }

    fn walk_settle(&self, cx: &mut Cx<'_>) -> WalkSettle {
        let settled = walk_wait::dispatch(&json!({
            "op": "settled",
            "token": self.walk_token,
        }))
        .as_bool()
        .unwrap_or(false);
        if settled {
            if walk_wait::dispatch(&json!({
                "op": "value",
                "token": self.walk_token,
            }))
            .as_bool()
            .unwrap_or(false)
            {
                WalkSettle::Arrived
            } else {
                WalkSettle::Failed
            }
        } else if cx.clock().bound_reached() {
            WalkSettle::Timeout
        } else {
            WalkSettle::Pending
        }
    }

    fn remember_npc(&mut self, npc: Npc) {
        self.npc_name = npc.name.to_string();
        self.npc_action = talk_op(&npc.actions).unwrap_or("Talk-to").to_string();
        self.npc_index = npc.index;
        self.npc_tile = npc.tile;
    }

    fn fresh_ready(&self, obs: &Observation) -> bool {
        if !obs.dialog_ready() {
            return false;
        }
        obs.chat_modal_id != self.armed_modal
            || (obs.is_open() && self.armed_modal == -1)
            || (obs.chat_continue && !self.armed_continue)
    }

    fn fresh_cant_reach(&self, obs: &Observation) -> bool {
        obs.chat_lines
            .iter()
            .any(|(seq, text)| *seq > self.armed_seq && is_cant_reach(text))
    }
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
        .filter(|npc| talk_op(&npc.actions).is_some())
        .min_by_key(|npc| npc.distance)
        .cloned()
}

fn talk_op(actions: &[Text]) -> Option<&str> {
    actions.iter().find_map(|action| {
        action
            .get(..4)
            .is_some_and(|head| head.eq_ignore_ascii_case("talk"))
            .then_some(&**action)
    })
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

/// A whole JS number (a float beyond int32), else 0.
fn json_i32(value: &Value) -> i32 {
    value
        .as_i64()
        .or_else(|| {
            value
                .as_f64()
                .filter(|n| n.fract() == 0.0)
                .map(|n| n as i64)
        })
        .and_then(|n| i32::try_from(n).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isolate_fb::{
        encode_snapshot, encode_snapshot_with_native, ChatLineInput, NativeFactsInput,
        ReachViewInput, SceneEntityInput, SnapshotInput, TileInput,
    };
    use crate::machine::{self, Called, Handle, Outcome, Pending, Reply, Started, Take};

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
            shape: 0,
            angle: 0,
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
        observed::apply(&snap);
        on_snapshot(&snap);
    }

    /// No script callbacks: the reach machine calls none.
    struct NoJs;

    impl machine::Js for NoJs {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(
            &mut self,
            _hook: Option<&crate::load::callback_v8::HeldCallback>,
            _args: &[Value],
        ) -> Called {
            panic!("reach calls no script callback");
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            panic!("reach calls no script callback");
        }

        fn claimed(&mut self) -> bool {
            false
        }
    }

    /// What a begin or a step did, as a page: the op it sent (`npc` /
    /// `walk-near` with the op's fields), `wait`, or its settled `status`.
    fn page(handle: Option<Handle>, out: Option<Outcome>) -> Value {
        let ops = machine::merge_ops(Vec::new());
        let mut page = match (out, ops.first()) {
            (Some(Outcome::Done(status)), None) => json!({ "kind": "done", "status": status }),
            (Some(Outcome::Aborted(_)), None) => json!({ "kind": "aborted" }),
            (
                None,
                Some(InteractReq::Npc {
                    name,
                    action,
                    index,
                }),
            ) => json!({ "kind": "npc", "name": name, "action": action, "index": index }),
            (
                None,
                Some(InteractReq::WalkNear {
                    x,
                    z,
                    level,
                    radius,
                    request_id,
                    ..
                }),
            ) => json!({
                "kind": "walk-near",
                "x": x,
                "z": z,
                "level": level,
                "radius": radius,
                "request_id": request_id,
            }),
            (None, None) => json!({ "kind": "wait" }),
            other => panic!("unexpected reach page {other:?}"),
        };
        page["token"] = json!(handle);
        page
    }

    fn begin_named(name: &str, x: i32, z: i32, open_ms: Option<u64>) -> Value {
        let mut args = json!({ "name": name, "near": { "x": x, "z": z, "level": 0 } });
        if let Some(open_ms) = open_ms {
            args["openMs"] = json!(open_ms);
        }
        match machine::start("reach-npc-dialog", args, Vec::new(), 0) {
            Started::Running(handle) => page(Some(handle), None),
            Started::Settled(out) => page(None, Some(out)),
            Started::Refused(why) => panic!("refused: {why}"),
        }
    }

    /// One tick: every live reach steps once.
    fn next_token(step: &Value) -> Value {
        let handle = step["token"].as_u64().expect("a running reach");
        machine::step(&mut NoJs);
        match machine::take(handle) {
            Take::Settled(out) => page(Some(handle), Some(out)),
            Take::Pending => page(Some(handle), None),
        }
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
        machine::on_reset();
        walk_wait::on_reset();
        observed::on_reset();
        let bytes = encode_snapshot(&base());
        let snap = SnapshotReader::from_bytes(&bytes).expect("base");
        observed::apply(&snap);
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
        machine::tests::expire_deadlines();
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
        machine::tests::expire_deadlines();
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
        machine::on_pause();
        assert_eq!(next_token(&talk)["kind"], "wait");
        machine::on_resume();
        snap.ours = true;
        observe(&snap, NativeFactsInput::default());
        assert_eq!(next_token(&talk)["status"], "retry");

        snap.ours = false;
        observe(&snap, NativeFactsInput::default());
        let again = begin_named("Traiborn", 5, 5, None);
        assert_eq!(again["kind"], "npc");
        machine::on_reset();
        assert_eq!(next_token(&again)["kind"], "aborted");
        assert!(machine::merge_ops(Vec::new()).is_empty());
    }

    #[test]
    fn a_pending_post_between_steps_still_interrupts() {
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 4, 6, 5, 1, true)];
        let mut snap = base();
        snap.npcs = &npcs;
        observe(&snap, NativeFactsInput::default());
        let talk = begin_named("Traiborn", 5, 5, None);
        snap.hold = true;
        observe(&snap, NativeFactsInput::default());
        snap.hold = false;
        observe(&snap, NativeFactsInput::default());
        assert_eq!(next_token(&talk)["status"], "retry");
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
