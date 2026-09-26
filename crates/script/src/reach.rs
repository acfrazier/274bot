//! Rust-owned `Reach.npcDialog` sequencing, the `reach-npc-dialog`
//! [`crate::machine`] family.
//!
//! JavaScript passes name/stand/`openMs` and the `log` / `sustain` hooks and
//! awaits the status. The close-in and stand walks are the frozen
//! `walkResilient` ladder (`Reach.ts:61–68, 283–287`: radius 3 / 1,
//! `attempts: 4`, 90 s), whose `unreachable` ending (a dead verify probe)
//! is the reach's `unreachable`; any other walk ending keeps the frozen
//! `retry` (close-in) or goes on to the talk (stand). The talk is frozen
//! `reachThroughDoors` (`Reach.ts:156–211, 288–300`: up to eight rounds,
//! retry after a timeout, the scene probe and door clearing, a dialogue the
//! click produced), [`crate::reach_entity::NpcReach`]. Game actions reuse the
//! existing FlatBuffer walk, loc and npc verbs.

use crate::isolate_fb::SnapshotReader;
use crate::machine::{Begin, Call, Cx, Family, Reply, Step};
use crate::observed::{self, Ops, Scene, Text};
use crate::reach_entity::{NpcReach, NpcReachOpts, TalkExpect};
use crate::shim::InteractReq;
use crate::walk::Resilient;
use crate::walk_wait;
use api::snapshot::WorldTile;
use serde::Deserialize;
use serde_json::{json, Value};
use std::cell::Cell;
use std::collections::VecDeque;

/// Frozen close-in / stand walk bound.
pub const WALK_BOUND_MS: u64 = 90_000;
/// Frozen `openMs` default.
pub const OPEN_MS: u64 = 15_000;
/// Frozen close-in and stand `walkResilient` `attempts` (`Reach.ts:62, 283`).
const WALK_ATTEMPTS: u32 = 4;
const CLOSE_IN_RADIUS: i32 = 3;
const STAND_RADIUS: i32 = 1;
const ADJACENT: i32 = 1;
const LOG: usize = 0;
const SUSTAIN: usize = 1;

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

impl Tile {
    fn world(self) -> WorldTile {
        WorldTile {
            x: self.x,
            z: self.z,
            level: self.level,
        }
    }
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
}

enum Phase {
    /// Frozen `closeIn` (`Reach.ts:61–68`).
    CloseIn(Resilient),
    /// Frozen stand walk before the talk (`Reach.ts:283–287`).
    WalkStand(Resilient),
    /// Frozen `reachThroughDoors` over the talk op (`Reach.ts:288–300`).
    Talk(NpcReach),
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
    /// [`pending_posts`] when the reach began.
    pending_mark: u64,
    logs: VecDeque<String>,
    result: Option<&'static str>,
    /// This tick's advance already ended waiting (a log was written after).
    waiting: bool,
    /// `Sustain.run()` already ran for this ladder pass.
    pumped: bool,
}

impl Family for NpcDialog {
    const NAME: &'static str = "reach-npc-dialog";
    /// A new reach replaces the one in flight.
    const EXCLUSIVE: bool = true;
    const CALLBACKS: &'static [&'static str] = &["log", "sustain"];
    /// Frozen `log(...)` is not awaited; the ladder's `await Sustain.run()`
    /// (`Traversal.ts:130`) is.
    const SYNC_HOOKS: &'static [usize] = &[LOG];
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
            // Replaced by `start` before the row runs.
            phase: Phase::Talk(NpcReach::new("", talk_opts(open_ms))),
            npc_name: args.name.trim().to_string(),
            near: Tile {
                x: json_i32(&args.near.x),
                z: json_i32(&args.near.z),
                level: json_i32(&args.near.level),
            },
            open_ms,
            pending_mark: pending_posts(),
            logs: VecDeque::new(),
            result: None,
            waiting: false,
            pumped: false,
        };
        match reach.start(&obs, cx) {
            // A start that settles at once has written no ladder log.
            Some(status) => Begin::Done(status),
            None => Begin::Run(reach),
        }
    }

    /// A superseded reach stops the walk it armed.
    fn release(&self) -> Option<InteractReq> {
        match &self.phase {
            Phase::CloseIn(walk) | Phase::WalkStand(walk) => walk.release(),
            Phase::Talk(reach) => reach.release(),
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<&'static str> {
        if let Some(Reply::Threw(thrown)) = cx.reply() {
            return Step::Fail(thrown);
        }
        loop {
            if let Some(line) = self.logs.pop_front() {
                if cx.has(LOG) {
                    return Step::Call(Call {
                        hook: LOG,
                        args: vec![json!(line)],
                    });
                }
                continue;
            }
            if let Some(status) = self.result {
                return Step::Done(status);
            }
            if std::mem::take(&mut self.waiting) {
                self.pumped = false;
                return Step::Wait;
            }
            let obs = observed::with(Observation::from_scene);
            if !obs.ingame || pending_posts() != self.pending_mark || obs.pending() {
                // Stop the walk this reach armed: the host follow is only
                // frozen by a hold and would resume after it.
                if let Some(op) = self.release() {
                    cx.emit(op);
                }
                return Step::Done("retry");
            }
            let walking = matches!(self.phase, Phase::CloseIn(_) | Phase::WalkStand(_));
            if walking && !self.pumped && cx.has(SUSTAIN) {
                self.pumped = true;
                return Step::Call(Call {
                    hook: SUSTAIN,
                    args: Vec::new(),
                });
            }
            let out = match self.phase {
                Phase::CloseIn(_) | Phase::WalkStand(_) => self.ladder_step(cx),
                Phase::Talk(_) => self.talk_step(cx),
            };
            match out {
                Some(status) => self.result = Some(status),
                None if self.logs.is_empty() => {
                    self.pumped = false;
                    return Step::Wait;
                }
                None => self.waiting = true,
            }
        }
    }
}

/// `reachThroughDoors` as `Reach.npcDialog` calls it (`Reach.ts:288–300`):
/// retry after a timeout, probe the scene for an unreachable NPC, always
/// click. A dialogue counts only when the click produced it.
fn talk_opts(open_ms: u64) -> NpcReachOpts {
    NpcReachOpts {
        expect: TalkExpect::FreshDialog,
        expect_ms: open_ms,
        retry_after_timeout: true,
        probe_unreachable: true,
        skip_click_when_expected: false,
    }
}

impl NpcDialog {
    /// `Some(status)` when the reach settled before any wait.
    fn start(&mut self, obs: &Observation, cx: &mut Cx<'_>) -> Option<&'static str> {
        if obs.is_open() {
            return Some(if owned_adjacent(obs, &self.npc_name) {
                "done"
            } else {
                "retry"
            });
        }
        match talk_target(&obs.npcs, &self.npc_name) {
            None => {
                if within(obs.here, self.near, CLOSE_IN_RADIUS) {
                    return Some("retry");
                }
                match self.ladder(CLOSE_IN_RADIUS, cx) {
                    Ok(walk) => {
                        self.phase = Phase::CloseIn(walk);
                        None
                    }
                    // `closeIn` is `retry` unless the ladder said unreachable.
                    Err(_) => Some("retry"),
                }
            }
            Some(_) => {
                if !within(obs.here, self.near, STAND_RADIUS) {
                    if let Ok(walk) = self.ladder(STAND_RADIUS, cx) {
                        self.phase = Phase::WalkStand(walk);
                        return None;
                    }
                }
                self.talk(cx)
            }
        }
    }

    /// Frozen `Traversal.walkResilient(near, { radius, attempts: 4,
    /// timeoutMs: 90_000, log })` (`Reach.ts:62, 283`). `Err` when it ended
    /// before a walk (arrived, or no posted tile).
    fn ladder(&self, radius: i32, cx: &mut Cx<'_>) -> Result<Resilient, bool> {
        Resilient::new(
            self.near.world(),
            radius,
            WALK_BOUND_MS,
            Some(WALK_ATTEMPTS),
            false,
        )
        .start(cx)
    }

    fn ladder_step(&mut self, cx: &mut Cx<'_>) -> Option<&'static str> {
        let (Phase::CloseIn(walk) | Phase::WalkStand(walk)) = &mut self.phase else {
            return None;
        };
        let out = walk.step(cx);
        let unreachable = walk.unreachable();
        while let Some(line) = walk.pop_log() {
            self.logs.push_back(line);
        }
        let arrived = out?;
        let near = self.near;
        let close_in = matches!(self.phase, Phase::CloseIn(_));
        if !arrived && unreachable {
            self.logs.push_back(if close_in {
                format!(
                    "reach: hint ({},{},{}) is unreachable",
                    near.x, near.z, near.level
                )
            } else {
                format!(
                    "reach: stand ({},{},{}) unreachable",
                    near.x, near.z, near.level
                )
            });
            return Some("unreachable");
        }
        if close_in {
            return Some("retry");
        }
        self.talk(cx)
    }

    /// Frozen `reachThroughDoors(...)` after the stand (`Reach.ts:288–300`).
    fn talk(&mut self, cx: &mut Cx<'_>) -> Option<&'static str> {
        self.phase = Phase::Talk(NpcReach::new(&self.npc_name, talk_opts(self.open_ms)));
        self.talk_step(cx)
    }

    fn talk_step(&mut self, cx: &mut Cx<'_>) -> Option<&'static str> {
        let Phase::Talk(reach) = &mut self.phase else {
            return None;
        };
        let out = reach.step(cx);
        while let Some(line) = reach.pop_log() {
            self.logs.push_back(line);
        }
        out
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
    use crate::shim::InteractReq;

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
            (None, Some(InteractReq::WalkTo { x, z, level })) => {
                json!({ "kind": "walk-to", "x": x, "z": z, "level": level })
            }
            (None, Some(InteractReq::InspectRoute { request_id, .. })) => {
                json!({ "kind": "probe", "request_id": request_id })
            }
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
        fail_native_at(1, request_id, x, z, radius)
    }

    fn fail_native_at(
        seq: u64,
        request_id: u64,
        x: i32,
        z: i32,
        radius: i32,
    ) -> NativeFactsInput<'static> {
        NativeFactsInput {
            walk_outcome_seq: seq,
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
        crate::inspect_wait::on_reset();
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
    fn stand_walk_failure_takes_the_scene_step_before_the_talk() {
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
        let scene = next_token(&walk);
        assert_eq!(scene["kind"], "walk-to");
        assert!(scene["status"].is_null(), "the ladder walks on");
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

    fn fresh_cant_reach(snap: &mut SnapshotInput<'_>, lines: &'static [ChatLineInput<'static>]) {
        snap.tick += 1;
        snap.chat_lines = lines;
        observe(snap, NativeFactsInput::default());
    }

    const CANT_REACH: &[ChatLineInput<'static>] = &[ChatLineInput {
        seq: 8,
        text: "I can't reach that!",
        type_: 0,
        username: None,
    }];

    #[test]
    fn fresh_cant_reach_with_no_door_to_clear_is_unreachable_without_a_walk() {
        // Frozen `reachThroughDoors` (Reach.ts:193-200): "I can't reach
        // that" with no leaf to close and no door to open is unreachable.
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 4, 8, 5, 2, false)];
        let mut snap = base();
        snap.npcs = &npcs;
        observe(&snap, NativeFactsInput::default());
        let talk = begin_named("Traiborn", 5, 5, None);
        assert_eq!(talk["kind"], "npc");
        fresh_cant_reach(&mut snap, CANT_REACH);
        let step = next_token(&talk);
        assert_eq!(step["status"], "unreachable");
        assert_ne!(step["kind"], "walk-near", "no walk onto the NPC's tile");
    }

    #[test]
    fn an_unanswered_talk_is_clicked_again_in_the_next_round() {
        // Frozen `retryAfterTimeout` (Reach.ts:202-208): a timed-out wait
        // waits one tick and runs the next round.
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 4, 6, 5, 1, true)];
        let mut snap = base();
        snap.npcs = &npcs;
        observe(&snap, NativeFactsInput::default());
        let talk = begin_named("Traiborn", 5, 5, Some(0));
        assert_eq!(talk["kind"], "npc");
        let tick = next_token(&talk);
        assert_eq!(tick["kind"], "wait", "the frozen one-tick gap");
        let again = next_token(&tick);
        assert_eq!(again["kind"], "npc", "the next round talks again");
        assert!(again["status"].is_null());
    }

    #[test]
    fn eight_unanswered_rounds_end_retry() {
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 4, 6, 5, 1, true)];
        let mut snap = base();
        snap.npcs = &npcs;
        observe(&snap, NativeFactsInput::default());
        let mut page = begin_named("Traiborn", 5, 5, Some(0));
        let mut talks = 1;
        for _ in 0..40 {
            page = next_token(&page);
            if !page["status"].is_null() {
                break;
            }
            if page["kind"] == "npc" {
                talks += 1;
            }
        }
        assert_eq!(page["status"], "retry");
        assert_eq!(talks, 8, "frozen REACH_DOOR_ATTEMPTS rounds");
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
    fn open_ms_bounds_each_round_wait() {
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 4, 6, 5, 1, true)];
        let mut snap = base();
        snap.npcs = &npcs;
        observe(&snap, NativeFactsInput::default());
        let talk = begin_named("Traiborn", 5, 5, Some(60_000));
        assert_eq!(talk["kind"], "npc");
        assert_eq!(next_token(&talk)["kind"], "wait", "inside openMs");
        machine::tests::expire_deadlines();
        let tick = next_token(&talk);
        assert_eq!(tick["kind"], "wait");
        assert_eq!(next_token(&tick)["kind"], "npc", "past openMs: next round");
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
    fn a_superseded_reach_stops_its_close_in_walk() {
        reset();
        let mut far = base();
        far.here = Some(TileInput {
            x: 0,
            z: 0,
            level: 0,
        });
        observe(&far, NativeFactsInput::default());
        let first = begin_named("Traiborn", 20, 20, None);
        let token = first["request_id"].as_u64().expect("close-in walk");
        let Started::Running(_) = machine::start(
            "reach-npc-dialog",
            json!({ "name": "Traiborn", "near": { "x": 30, "z": 30, "level": 0 } }),
            Vec::new(),
            0,
        ) else {
            panic!("second reach runs");
        };
        let ops = machine::merge_ops(Vec::new());
        assert_eq!(
            ops.first(),
            Some(&InteractReq::AbortWalk { request_id: token }),
            "the superseded close-in walk is stopped first: {ops:?}"
        );
        assert!(matches!(
            ops.get(1),
            Some(InteractReq::WalkNear { x: 30, .. })
        ));
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

    /// One no-progress ladder pass from a pending baked `walk` page: the
    /// walk fails, the scene step runs out, the unstick finds no door or
    /// step. Returns the page after the pass.
    fn ladder_pass(walk: &Value, snap: &mut SnapshotInput<'_>, seq: u64) -> Value {
        assert_eq!(walk["kind"], "walk-near", "a pass starts on the baked walk");
        let request_id = walk["request_id"].as_u64().unwrap();
        let (x, z) = (walk["x"].as_i64().unwrap(), walk["z"].as_i64().unwrap());
        let radius = walk["radius"].as_i64().unwrap();
        snap.tick += 1;
        observe(
            snap,
            fail_native_at(seq, request_id, x as i32, z as i32, radius as i32),
        );
        let scene = next_token(walk);
        assert_eq!(
            scene["kind"], "walk-to",
            "a failed baked walk takes the ladder's scene step"
        );
        machine::tests::expire_deadlines();
        next_token(&scene)
    }

    /// Steps through a backoff until the next baked walk goes out.
    fn until_walk(page: Value) -> Value {
        let mut page = page;
        for _ in 0..16 {
            if page["kind"] == "walk-near" {
                return page;
            }
            assert_eq!(page["kind"], "wait", "backoff sends nothing");
            page = next_token(&page);
        }
        panic!("the backoff never re-walked: {page}");
    }

    /// The ladder's three no-progress passes; returns the verify probe page.
    fn passes_to_probe(walk: Value, snap: &mut SnapshotInput<'_>) -> Value {
        let mut page = walk;
        for seq in 1..=3 {
            page = ladder_pass(&page, snap, seq);
            if seq < 3 {
                page = until_walk(page);
            }
        }
        assert_eq!(page["kind"], "probe", "the third pass verifies");
        page
    }

    #[test]
    fn close_in_walk_failure_keeps_the_ladder_going() {
        reset();
        let mut far = base();
        far.here = Some(TileInput {
            x: 0,
            z: 0,
            level: 0,
        });
        observe(&far, NativeFactsInput::default());
        let walk = begin_named("Traiborn", 20, 20, None);
        let after = ladder_pass(&walk, &mut far, 1);
        assert_eq!(
            after["kind"], "wait",
            "one failed pass backs off, it does not end the reach"
        );
    }

    #[test]
    fn close_in_dead_probe_is_unreachable() {
        reset();
        let mut far = base();
        far.here = Some(TileInput {
            x: 0,
            z: 0,
            level: 0,
        });
        observe(&far, NativeFactsInput::default());
        let walk = begin_named("Traiborn", 20, 20, None);
        let probe = passes_to_probe(walk, &mut far);
        crate::inspect_wait::settle_for_tests(probe["request_id"].as_u64().unwrap(), false);
        assert_eq!(next_token(&probe)["status"], "unreachable");
    }

    #[test]
    fn close_in_fresh_probe_is_not_unreachable() {
        reset();
        let mut far = base();
        far.here = Some(TileInput {
            x: 0,
            z: 0,
            level: 0,
        });
        observe(&far, NativeFactsInput::default());
        let walk = begin_named("Traiborn", 20, 20, None);
        let probe = passes_to_probe(walk, &mut far);
        crate::inspect_wait::settle_for_tests(probe["request_id"].as_u64().unwrap(), true);
        let after = next_token(&probe);
        assert_eq!(
            after["kind"], "wait",
            "a fresh probe backs off and walks on"
        );
        assert!(after["status"].is_null());
    }

    #[test]
    fn stand_dead_probe_is_unreachable_without_a_talk() {
        reset();
        let actions = ["Talk-to".to_string()];
        let npcs = [npc("Traiborn", &actions, 9, 30, 30, 30, false)];
        let mut snap = base();
        snap.here = Some(TileInput {
            x: 0,
            z: 0,
            level: 0,
        });
        snap.npcs = &npcs;
        observe(&snap, NativeFactsInput::default());
        let walk = begin_named("Traiborn", 20, 20, None);
        assert_eq!(walk["radius"], 1);
        let probe = passes_to_probe(walk, &mut snap);
        crate::inspect_wait::settle_for_tests(probe["request_id"].as_u64().unwrap(), false);
        let done = next_token(&probe);
        assert_eq!(done["status"], "unreachable");
        assert_ne!(done["kind"], "npc");
    }
}
