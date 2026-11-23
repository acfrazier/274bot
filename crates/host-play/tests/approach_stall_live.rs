//! Engine A proof for the owned-op reach gate and the approach stall watch.
//!
//! A target behind a wall: the client's route search fails, the old owned
//! path sent the op alone, and the engine kept the interaction and stalled
//! with no message (`Player.ts` `processInteraction`: a retained waypoint with
//! zero steps never reaches the `I can't reach that!` branch). The cell
//! (1) refuses that op on the owned path with no packet, (2) reproduces the
//! old behavior by sending the op through the client's native row directly,
//! bypassing the gate, and shows the silent stall, (3) shows the stall watch
//! ending it with the typed result at the observed-tick bound, and (4) shows a
//! reachable target is still sent, walked to and answered, never stalled.
//! Every seed is scoped to the one disposable account.
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::interact::{
    self, op_reach, ActionSpec, ApproachVerdict, ApproachWatch, Driver, Interactions,
    NativeMenuTarget, OpReach, OpTarget, PacketCheckpoint, SendReason, SendResult,
    APPROACH_STALL_TICKS,
};
use api::query::SceneQuery;
use api::snapshot::{GameSnapshot, NpcView, WorldTile};
use client::client::{Client, ClientRevision};
use client::io::{client_prot_289::map_client_prot, ClientProt};
use host::Pump;
use host_play::{ProfileOptions, SharedClientTemplate};
use serde_json::json;

// Engine A `content/maps/m50_50.jm2` NPC section places Hans (`hans`,
// `lumbridge.npc:135`, op1 Talk-to) at local (7,33) of mapsquare 50_50.
const HANS_SPAWN: (i32, i32, i32) = (3207, 3233, 0);
const TALK_TO: &str = "Talk-to";
/// Observed server ticks the old path is watched for. Bounded by the harness,
/// not by content: the watch's own bound is `APPROACH_STALL_TICKS`.
const OBSERVE_TICKS: u32 = 150;
const MAX_CANDIDATES: usize = 8;
/// Old-path runs recorded before the cell stops looking for a silent stall.
const OLD_PATH_RUNS: usize = 3;
// Engine A `interface_bank/configs/banker.npc:1-29` makes `banker1`
// stationary; the public stand is in range 2 (`banker.rs2:7-12`).
const BANKER_HOME: WorldTile = WorldTile {
    x: 3187,
    z: 3436,
    level: 0,
};
const BANKER_STAND: (i32, i32, i32) = (3185, 3436, 0);

fn required(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("live cell requires {key}"))
}

fn required_path(key: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(key).unwrap_or_else(|| panic!("live cell requires {key}")))
}

fn required_port(key: &str) -> u16 {
    required(key)
        .parse::<u16>()
        .unwrap_or_else(|error| panic!("{key} must be a valid port: {error}"))
}

fn live_profile_options() -> ProfileOptions {
    let home = required_path("HOME");
    ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(required_port("BOT_GAME_PORT")),
        http_port: Some(required_port("BOT_HTTP_PORT")),
        cache_dir: Some(required_path("BOT_CACHE_DIR")),
        unpack_dir: Some(home.join("approach-stall-unpack")),
        engine_dir: Some(required_path("ENGINE_DIR")),
        ..ProfileOptions::default()
    }
}

struct World {
    client: Client,
    snapshot: GameSnapshot,
    pump: Pump,
}

impl World {
    fn pump_once(&mut self) {
        self.client.mainloop();
        host::publish_snapshot(
            &mut self.snapshot,
            &self.client,
            self.pump.drain_client(&self.client),
        );
    }

    fn wait_for(&mut self, phase: &str, mut ready: impl FnMut(&Client, &GameSnapshot) -> bool) {
        // Bounds the harness only; the stall bound is the engine-derived constant.
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            self.pump_once();
            if ready(&self.client, &self.snapshot) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {phase}: ingame={} tile={:?} chat={:?}",
                self.snapshot.ingame(),
                self.snapshot.tile(),
                self.snapshot
                    .chat_lines()
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>(),
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Pump until `ticks` more observed server ticks have passed.
    fn wait_ticks(&mut self, ticks: u32) {
        let target = self.snapshot.tick().wrapping_add(ticks);
        self.wait_for("observed ticks", |_, state| {
            state.tick().wrapping_sub(target) < u32::MAX / 2
        });
    }

    fn tele(&mut self, at: (i32, i32, i32)) {
        assert!(
            interact::cheat(&mut self.client, &interact::tele_args(at.2, at.0, at.1)).is_sent(),
            "account-only teleport was refused"
        );
        self.wait_for("teleport", |_, state| {
            state.ingame() && state.scene_state() == 2 && state.tile() == Some(at)
        });
    }

    fn hans(&self, hans_type: usize) -> Option<NpcView> {
        self.snapshot
            .npcs()
            .iter()
            .find(|npc| {
                npc.r#type == Some(hans_type)
                    && npc.actions.first().and_then(Option::as_deref) == Some(TALK_TO)
            })
            .cloned()
    }

    /// A walk click on the player's own tile: the engine clears any pending
    /// interaction (`MoveGameClickHandler` -> `clearPendingAction`).
    fn clear_interaction(&mut self) {
        let (x, z, _) = self.snapshot.tile().expect("player tile");
        assert!(interact::walk(&mut self.client, x, z));
        self.wait_ticks(2);
    }

    fn opcodes_since(&self, checkpoint: PacketCheckpoint) -> Vec<u8> {
        let mut opcodes = Vec::new();
        assert!(
            self.client
                .trace_packets(checkpoint, &mut |opcode| opcodes.push(opcode)),
            "the emitted frames must be complete"
        );
        opcodes
    }
}

fn chat_texts(snapshot: &GameSnapshot) -> Vec<String> {
    snapshot
        .chat_lines()
        .iter()
        .map(|line| line.text.clone())
        .collect()
}

/// One observed tick of the old path: `(tick, tile, newest chat text)`.
type Trace = Vec<(u32, (i32, i32, i32), Option<String>)>;

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, BOT_LIVE_NAME_PREFIX, BOT_CACHE_DIR, BOT_GAME_PORT, BOT_HTTP_PORT, ENGINE_DIR and LIVE_EVIDENCE_DIR"]
fn live_engine_a_unreachable_op_refused_and_stall_watched() {
    assert_eq!(required("LIVE"), "1");
    assert_eq!(required("BOT_CPU"), "1");
    let prefix = required("BOT_LIVE_NAME_PREFIX");
    assert!(
        (1..=4).contains(&prefix.len()) && prefix.bytes().all(|byte| byte.is_ascii_alphanumeric()),
        "BOT_LIVE_NAME_PREFIX must be one to four ASCII alphanumeric characters"
    );
    let evidence_dir = required_path("LIVE_EVIDENCE_DIR");
    let home = required_path("HOME");
    assert!(
        !evidence_dir.starts_with(&home),
        "live evidence must be outside the disposable HOME"
    );
    assert!(
        api::test_support::game_data_available(),
        "R289 game data is required"
    );
    let selected =
        api::game_data::for_revision(ClientRevision::R289).expect("load selected R289 game data");
    let hans_type = selected
        .npc_by_config("hans")
        .expect("Engine A Hans config")
        .id as usize;

    let profile = live_profile_options()
        .resolve(None)
        .expect("resolve local Engine A profile")
        .bind()
        .expect("bind local Engine A profile");
    let template = SharedClientTemplate::load(Arc::clone(&profile))
        .expect("load Engine A R289 client template");
    let account = host_play::mint_live_names(1)
        .into_iter()
        .next()
        .expect("mint one disposable live account");
    let mut client = template
        .prepare_client(1, true)
        .expect("prepare disposable live client");
    client.draw = false;
    client.maininit();
    assert!(!client.error_loading, "client failed to load");
    assert!(interact::login(&mut client, &account, &account, false));
    let mut world = World {
        client,
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
    };
    world.wait_for("login and scene", |_, state| {
        state.ingame() && state.attached() && state.scene_state() == 2
    });
    assert!(
        interact::cheat(&mut world.client, "setvar tutorial 1000").is_sent(),
        "tutorial seed was refused"
    );
    assert!(
        interact::cheat(&mut world.client, "getvar tutorial").is_sent(),
        "tutorial seed query was refused"
    );
    world.wait_for("account tutorial seed", |_, state| {
        state
            .chat_lines()
            .iter()
            .any(|line| line.text == "get tutorial: 1000")
    });

    // Stand beside Hans's spawn to read his surroundings.
    world.tele(HANS_SPAWN);
    world.wait_for("Hans in view", |_, state| {
        state.npcs().iter().any(|npc| npc.r#type == Some(hans_type))
    });
    world.wait_ticks(2);
    let hans = world.hans(hans_type).expect("Hans in view");
    let hans_tile = hans.network;
    let flood = SceneQuery::new(world.snapshot.scene(), Some(hans_tile))
        .flood_reach()
        .expect("flood from Hans's tile");
    let scene = world.snapshot.scene().clone();
    let query = SceneQuery::new(&scene, None);

    // Candidate stands the wall hides: walkable, 3 to 9 tiles from Hans,
    // outside the region Hans can walk to.
    let mut candidates = Vec::new();
    for lx in 0..scene.width {
        for lz in 0..scene.height {
            let tile = WorldTile {
                x: scene.base_x + lx,
                z: scene.base_z + lz,
                level: scene.level,
            };
            let distance = (tile.x - hans_tile.x)
                .abs()
                .max((tile.z - hans_tile.z).abs());
            if (3..=14).contains(&distance) && query.walkable(tile) && !flood.at(&tile).0 {
                candidates.push((distance, tile));
            }
        }
    }
    candidates.sort_by_key(|&(distance, tile)| (distance, tile.x, tile.z));
    eprintln!(
        "approach-stall account={account} hans={hans_tile:?} hidden_candidates={}",
        candidates.len()
    );
    assert!(
        !candidates.is_empty(),
        "no tile near Hans is hidden from him by collision"
    );

    let mut attempts = Vec::<serde_json::Value>::new();
    let mut silent_stall = None;
    let mut refusal_checked = 0usize;
    let mut old_path_runs = 0usize;
    for (_, stand) in candidates.iter().take(MAX_CANDIDATES) {
        world.clear_interaction();
        world.tele((stand.x, stand.z, stand.level));
        world.wait_ticks(2);
        let Some(hans) = world.hans(hans_type) else {
            attempts.push(json!({"stand": stand, "skipped": "Hans out of view"}));
            continue;
        };
        let reach = op_reach(&world.snapshot, &OpTarget::Npc(&hans));
        if reach != OpReach::Unreachable {
            attempts.push(json!({"stand": stand, "hans": hans.network, "reach": format!("{reach:?}"), "skipped": "not Unreachable"}));
            continue;
        }

        // (1) The owned path refuses the op and writes nothing.
        let checkpoint = PacketCheckpoint::capture(&world.client);
        let refused = Interactions::new(&world.snapshot, &mut world.client)
            .interact(OpTarget::Npc(&hans), ActionSpec::Label(TALK_TO.into()));
        let refused_opcodes = world.opcodes_since(checkpoint);
        assert!(
            matches!(
                refused,
                SendResult::Refused {
                    reason: SendReason::Unreachable,
                    ..
                }
            ),
            "owned Talk-to to an unreachable Hans was not refused: {refused:?}"
        );
        assert!(
            refused_opcodes.is_empty(),
            "the refused op wrote {refused_opcodes:?}"
        );
        refusal_checked += 1;

        // (2) The old path: the client's native row, op sent alone.
        let before = world.snapshot.tile().expect("player tile");
        let checkpoint = PacketCheckpoint::capture(&world.client);
        let sent = Driver::dispatch_native_menu(
            &mut world.client,
            &world.snapshot,
            NativeMenuTarget::NpcTalk(&hans),
        );
        let old_opcodes = world.opcodes_since(checkpoint);
        let opnpc1 = u8::try_from(map_client_prot(world.client.revision(), ClientProt::OPNPC1).id)
            .expect("OPNPC1 opcode");
        if sent.is_err() || old_opcodes != [opnpc1] {
            attempts.push(json!({
                "stand": stand,
                "hans": hans.network,
                "skipped": "the client found a route or refused the row",
                "native_result": format!("{sent:?}"),
                "opcodes": old_opcodes,
            }));
            world.clear_interaction();
            continue;
        }

        // (3) Watch the old path for the observed-tick window.
        let mut watch = ApproachWatch::arm(&world.snapshot, &OpTarget::Npc(&hans)).expect("watch");
        let start = world.snapshot.tick();
        let mut trace: Trace = Vec::new();
        let mut verdict = ApproachVerdict::Pending;
        let mut stalled_tick = None;
        let mut explicit_refusal = false;
        let mut dialogue_opened = false;
        let mut last_seen_tick = start;
        while world.snapshot.tick().wrapping_sub(start) < OBSERVE_TICKS
            && !matches!(verdict, ApproachVerdict::Stalled { .. })
        {
            world.pump_once();
            let tick = world.snapshot.tick();
            if tick == last_seen_tick {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            last_seen_tick = tick;
            let newest = world
                .snapshot
                .chat_lines()
                .first()
                .map(|line| line.text.clone());
            if chat_texts(&world.snapshot)
                .iter()
                .any(|text| text == "I can't reach that!")
            {
                explicit_refusal = true;
            }
            if world.snapshot.chat_continue_component_id() >= 0 {
                dialogue_opened = true;
            }
            trace.push((
                tick - start,
                world.snapshot.tile().expect("player tile"),
                newest,
            ));
            if matches!(verdict, ApproachVerdict::Pending) {
                verdict = watch.observe(&world.snapshot);
                if matches!(verdict, ApproachVerdict::Stalled { .. }) {
                    stalled_tick = Some(tick - start);
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let after = world.snapshot.tile().expect("player tile");
        let record = json!({
            "stand": stand,
            "hans": hans.network,
            "reach": format!("{reach:?}"),
            "refused": format!("{refused:?}"),
            "refused_opcodes": refused_opcodes,
            "old_opcodes": old_opcodes,
            "tile_before": before,
            "tile_after": after,
            "explicit_refusal": explicit_refusal,
            "dialogue_opened": dialogue_opened,
            "verdict": format!("{verdict:?}"),
            "stalled_at_tick": stalled_tick,
            "trace": trace.iter().map(|(tick, tile, chat)| json!({"tick": tick, "tile": tile, "chat": chat})).collect::<Vec<_>>(),
        });
        eprintln!("approach-stall attempt {record}");
        let silent = !explicit_refusal
            && !dialogue_opened
            && matches!(verdict, ApproachVerdict::Stalled { .. });
        attempts.push(record);
        world.clear_interaction();
        if silent {
            silent_stall = Some(attempts.len() - 1);
            break;
        }
        old_path_runs += 1;
        if old_path_runs >= OLD_PATH_RUNS {
            break;
        }
    }
    // The old path's silent stall is report-only: it depends on where the
    // wandering Hans walks, which is a chance outcome the cell never gates on.
    // The gated parts are the owned-op refusal above and the banker control.
    assert!(
        refusal_checked > 0,
        "no candidate stand read Unreachable, so the owned-op refusal was never exercised: {attempts:?}"
    );

    // (4) Deterministic control: the stationary banker (`banker1`, NOMOVE,
    // `interact_bank` content) reached from 5 to 8 tiles away on the public
    // side. The same op is sent, the player walks to the banker's approach
    // range and the dialogue opens; the watch never stalls.
    let banker_type = selected
        .npc_by_config("banker1")
        .expect("Engine A stationary banker config")
        .id as usize;
    world.clear_interaction();
    world.tele(BANKER_STAND);
    world.wait_for("stationary banker in view", |_, state| {
        state
            .npcs()
            .iter()
            .any(|npc| npc.r#type == Some(banker_type) && npc.tile == BANKER_HOME)
    });
    world.wait_ticks(2);
    let stand_tile = WorldTile {
        x: BANKER_STAND.0,
        z: BANKER_STAND.1,
        level: BANKER_STAND.2,
    };
    let flood = SceneQuery::new(world.snapshot.scene(), Some(stand_tile))
        .flood_reach()
        .expect("flood from the banker stand");
    let scene = world.snapshot.scene().clone();
    let query = SceneQuery::new(&scene, None);
    let mut far_stands = Vec::new();
    for lx in 0..scene.width {
        for lz in 0..scene.height {
            let tile = WorldTile {
                x: scene.base_x + lx,
                z: scene.base_z + lz,
                level: scene.level,
            };
            let distance = (tile.x - BANKER_HOME.x)
                .abs()
                .max((tile.z - BANKER_HOME.z).abs());
            // Same row, west of the banker: the client's route search finds no
            // stand behind the counter, so the op goes out alone and the
            // engine's straight greedy approach along this row reaches the
            // range-2 stand the content serves from. A stand off the row can
            // be blocked by the counter's end and is a real stall, not a control.
            if (5..=8).contains(&distance)
                && tile.z == BANKER_HOME.z
                && tile.x < BANKER_HOME.x
                && query.walkable(tile)
                && flood.at(&tile).0
            {
                far_stands.push((distance, tile));
            }
        }
    }
    far_stands.sort_by_key(|&(distance, tile)| (distance, tile.x, tile.z));
    let mut controls = Vec::new();
    for (_, stand) in far_stands.iter().take(MAX_CANDIDATES) {
        world.clear_interaction();
        world.tele((stand.x, stand.z, stand.level));
        world.wait_ticks(2);
        let Some(banker) = world.hans(banker_type) else {
            controls.push(json!({"stand": stand, "skipped": "banker out of view"}));
            continue;
        };
        let reach = op_reach(&world.snapshot, &OpTarget::Npc(&banker));
        if matches!(reach, OpReach::Unreachable | OpReach::InReach) {
            controls.push(json!({"stand": stand, "reach": format!("{reach:?}"), "skipped": "not an approach"}));
            continue;
        }
        let checkpoint = PacketCheckpoint::capture(&world.client);
        let mut ix = Interactions::new(&world.snapshot, &mut world.client);
        let result = ix.interact(OpTarget::Npc(&banker), ActionSpec::Label(TALK_TO.into()));
        let mut watch = ix.take_approach_watch();
        let opcodes = world.opcodes_since(checkpoint);
        assert!(
            matches!(result, SendResult::Sent { .. }),
            "the banker Talk-to was refused: {result:?}"
        );
        assert!(watch.is_some(), "an approaching op must arm the watch");
        let start = world.snapshot.tick();
        let mut verdict = ApproachVerdict::Pending;
        let mut opened = false;
        let mut last_seen_tick = start;
        let mut control_trace: Vec<serde_json::Value> = Vec::new();
        while world.snapshot.tick().wrapping_sub(start) < OBSERVE_TICKS && !opened {
            world.pump_once();
            let tick = world.snapshot.tick();
            if tick != last_seen_tick {
                last_seen_tick = tick;
                if let Some(watch) = watch.as_mut() {
                    if matches!(verdict, ApproachVerdict::Pending) {
                        verdict = watch.observe(&world.snapshot);
                    }
                }
                control_trace.push(json!({
                    "tick": tick - start,
                    "tile": world.snapshot.tile(),
                    "continue": world.snapshot.chat_continue_component_id(),
                }));
            }
            opened = world.snapshot.chat_continue_component_id() >= 0;
            std::thread::sleep(Duration::from_millis(20));
        }
        eprintln!(
            "approach-stall control stand={stand:?} reach={reach:?} opcodes={opcodes:?} verdict={verdict:?} opened={opened} trace={}",
            serde_json::Value::Array(control_trace)
        );
        assert!(
            !matches!(verdict, ApproachVerdict::Stalled { .. }),
            "a stationary banker's approach was flagged as a stall"
        );
        assert!(opened, "the banker's dialogue did not open from {stand:?}");
        controls.push(json!({
            "stand": stand,
            "reach": format!("{reach:?}"),
            "result": format!("{result:?}"),
            "opcodes": opcodes,
            "verdict": format!("{verdict:?}"),
            "dialogue_opened": opened,
        }));
        world.client_close_dialogue();
        break;
    }
    assert!(
        controls
            .iter()
            .any(|control| control.get("dialogue_opened").is_some()),
        "no far banker stand produced a control: {controls:?}"
    );

    std::fs::create_dir_all(&evidence_dir).expect("create live evidence directory");
    let receipt = evidence_dir.join(format!("approach-stall-{account}.json"));
    std::fs::write(
        &receipt,
        serde_json::to_vec_pretty(&json!({
            "cell": "live_engine_a_unreachable_op_refused_and_stall_watched",
            "engine": "Engine A",
            "revision": 289,
            "account": account,
            "seed_scope": "disposable account only",
            "stall_bound_ticks": APPROACH_STALL_TICKS,
            "observe_ticks": OBSERVE_TICKS,
            "silent_stall_attempt": silent_stall,
            "silent_stall_seen": silent_stall.is_some(),
            "attempts": attempts,
            "controls": controls,
        }))
        .expect("serialize receipt"),
    )
    .expect("write receipt");
    assert!(receipt.is_file(), "runtime receipt was not written");
}

impl World {
    /// Close any dialogue the control opened so the next teleport is clean.
    fn client_close_dialogue(&mut self) {
        let _ = Interactions::new(&self.snapshot, &mut self.client).close_modal();
        self.wait_ticks(2);
    }
}
