//! Engine L witness for the v1 Reachability.probeable scene-availability contract.

#![cfg(feature = "live-harness")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::snapshot::GameSnapshot;
use client::client::Client;
use host::Pump;
use host_play::{mint_live_entries, mint_live_names, run_with_template, Play, ProfileOptions};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings};

const GAME_PORT: u16 = 47_594;
const HTTP_PORT: u16 = 3_080;
const READY_DEADLINE: Duration = Duration::from_secs(300);
const CELL_DEADLINE: Duration = Duration::from_secs(240);
const STOP_DEADLINE: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const LOG_PREFIX: &str = "WALK_PROBE_RESULT:";

const PROBE_SOURCE: &str = r#"
import { reader } from '../../adapter/ClientAdapter.js';
import { Reachability } from '../../event/webwalk/geometry/Reachability.js';
import { wildernessLevelAt } from '../../event/webwalk/wilderness.js';
import { WalkExecutor } from '../../event/webwalk/WalkExecutor.js';
import { Traversal } from '../../api/walking/Traversal.js';
export default class CompatWalkingProbe extends LoopingBot {
    async loop() {
        if (this.done || !reader.ingame() || reader.sceneState() !== 2) return;
        const here = reader.worldTile();
        if (!here) return;
        const routeProbe = await WalkExecutor.probeDest(here, 23);
        if (!routeProbe.ok || routeProbe.terminal?.x !== here.x
            || routeProbe.terminal?.z !== here.z
            || routeProbe.terminal?.level !== here.level) {
            throw new Error(`current-tile probe failed: ${JSON.stringify(routeProbe)}`);
        }
        const resilientCompleted = await Traversal.walkResilient(
            here,
            { radius: 0, attempts: 1, maxBudget: 120000 },
        );
        if (!resilientCompleted) throw new Error('walkResilient did not accept the current tile');
        const walkCompleted = await Traversal.walkTo(here, { radius: 0 });
        if (!walkCompleted) throw new Error('walkTo did not accept the current tile');
        const lastOutcome = WalkExecutor.lastOutcome;
        if (lastOutcome !== 'arrived') throw new Error(`unexpected lastOutcome: ${lastOutcome}`);
        const remaining = Traversal.remaining();
        if (!Number.isInteger(remaining) || remaining < 0) {
            throw new Error(`invalid walking remaining count: ${remaining}`);
        }
        Traversal.requestRepath('compatibility probe live coverage');


        let blocked = null;
        for (let radius = 0; radius <= 52 && !blocked; radius++) {
            for (let dx = -radius; dx <= radius && !blocked; dx++) {
                for (let dz = -radius; dz <= radius; dz++) {
                    if (Math.abs(dx) !== radius && Math.abs(dz) !== radius) continue;
                    const candidate = { x: here.x + dx, z: here.z + dz, level: here.level };
                    if (Reachability.probeable(candidate) && !Reachability.walkable(candidate)) {
                        blocked = candidate;
                        break;
                    }
                }
            }
        }
        if (!blocked) throw new Error('loaded scene has no blocked tile to probe');

        const otherPlane = { x: here.x, z: here.z, level: here.level === 0 ? 1 : 0 };
        if (Reachability.probeable(otherPlane)) {
            throw new Error('probeable accepted the player coordinate on another plane');
        }

        let inside = here;
        let edge = null;
        for (let delta = 1; delta <= 128; delta++) {
            const candidate = { x: here.x + delta, z: here.z, level: here.level };
            if (!Reachability.probeable(candidate)) {
                edge = candidate;
                break;
            }
            inside = candidate;
        }
        if (!edge) throw new Error('could not find an unavailable scene edge on the player row');
        if (!Reachability.probeable(inside)) throw new Error('the tile before the scene edge is not probeable');
        if (reader.toLocal(edge.x, edge.z) !== null) {
            throw new Error('the unavailable scene edge still maps to a local tile');
        }
        if (reader.toLocal(inside.x, inside.z) === null) {
            throw new Error('the tile before the scene edge is not in the local scene');
        }

        const wildernessTile = { x: 3000, z: 3600, level: 0 };
        const wildernessLevel = wildernessLevelAt(wildernessTile);
        if (wildernessLevel !== 11) {
            throw new Error(`v17 wilderness tile ${JSON.stringify(wildernessTile)} returned ${wildernessLevel}, expected 11`);
        }

        this.done = true;
        this.log('WALK_PROBE_RESULT:' + JSON.stringify({
            player: here,
            blocked,
            blockedProbeable: Reachability.probeable(blocked),
            blockedWalkable: Reachability.walkable(blocked),
            otherPlane,
            otherPlaneProbeable: Reachability.probeable(otherPlane),
            insideEdge: inside,
            insideEdgeProbeable: Reachability.probeable(inside),
            sceneEdge: edge,
            sceneEdgeProbeable: Reachability.probeable(edge),
            insideLocal: reader.toLocal(inside.x, inside.z),
            edgeLocal: reader.toLocal(edge.x, edge.z),
            routeProbe,
            walkCompleted,
            resilientCompleted,
            lastOutcome,
            remaining,
            wildernessTile,
            wildernessLevel,
        }));
    }
}
"#;

#[derive(Default)]
struct FrameState {
    pump: Option<Pump>,
    snapshot: Option<GameSnapshot>,
    ready: bool,
}

impl FrameState {
    fn publish(&mut self, client: &mut Client) {
        let pump = self.pump.get_or_insert_with(Pump::new);
        let snapshot = self.snapshot.get_or_insert_with(GameSnapshot::new);
        let drain = pump.drain_client(client);
        host::publish_snapshot(snapshot, client, drain);
        self.ready = client.ingame && client.scene_state == 2 && snapshot.tile().is_some();
    }
}

fn required_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is required")))
}

fn wait_until(deadline: Duration, account: &str, play: &Play, mut ready: impl FnMut() -> bool) {
    let expires = Instant::now() + deadline;
    while !ready() {
        assert!(
            Instant::now() < expires,
            "timed out waiting for Engine L state: account={account}, script={:?}, native={:?}",
            play.script_state(account),
            play.script_native_status(account),
        );
        thread::sleep(POLL_INTERVAL);
    }
}

fn parse_result(play: &Play, account: &str, deadline: Duration) -> Value {
    let expires = Instant::now() + deadline;
    loop {
        if let Some(error) = play.script_last_error(account) {
            panic!("walking probe script failed: {error}");
        }
        for line in play.script_take_pending_logs(account) {
            if let Some(raw) = line.strip_prefix(LOG_PREFIX) {
                return serde_json::from_str(raw).expect("walking probe JSON result");
            }
        }
        assert!(
            Instant::now() < expires,
            "timed out waiting for the walking probe result: account={account}, script={:?}, native={:?}",
            play.script_state(account),
            play.script_native_status(account),
        );
        thread::sleep(POLL_INTERVAL);
    }
}

fn stop_script(play: &Play, account: &str) {
    play.script_stop(account);
    wait_until(STOP_DEADLINE, account, play, || {
        play.script_state(account) == script::RunState::Idle
    });
}

#[test]
#[ignore = "requires LIVE=1, Engine L, WORLD_ENGINE_DIR, WORLD_NAV_PACK and isolated run paths"]
fn compat_v1_walking_probeable() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    script_js::install_cards().expect("install canonical compatibility cards");
    api::hostlog::set_debug(true);

    let home = required_path("HOME");
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(GAME_PORT),
        http_port: Some(HTTP_PORT),
        cache_dir: Some(required_path("BOT_CACHE_DIR")),
        unpack_dir: Some(home.join("unpack-289")),
        vault_path: Some(home.join("vault")),
        nav_pack: Some(required_path("WORLD_NAV_PACK")),
        engine_dir: Some(required_path("WORLD_ENGINE_DIR")),
        ..ProfileOptions::default()
    };
    let template = options
        .resolve(None)
        .expect("resolve Engine L local-289 profile")
        .prepare_template()
        .expect("prepare cached Engine L client template");
    assert!(
        template.world().is_some(),
        "Engine L needs its selected nav world"
    );

    let entries = mint_live_entries(&mint_live_names(1));
    assert_eq!(entries.len(), 1, "mint one live account");
    let (account, password) = &entries[0];
    let frame_state = Arc::new(Mutex::new(FrameState::default()));
    let callback_state = Arc::clone(&frame_state);
    let callback_account = account.clone();
    let mut play = run_with_template(
        template,
        false,
        vec![],
        |_| (None, None),
        move |client, username, _frame| {
            if username == callback_account {
                callback_state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .publish(client);
            }
        },
    )
    .expect("start client against the already-running Engine L");
    let uid = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock")
        .as_millis();
    play.try_spawn_slot(
        Profile {
            username: account.clone(),
            password: password.clone().into(),
            uid: (uid % i32::MAX as u128) as i32,
            settings: ProfileSettings::default(),
        },
        None,
        None,
        None,
    )
    .expect("spawn fresh live account");
    play.focus(account);
    wait_until(READY_DEADLINE, account, &play, || {
        frame_state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .ready
    });

    play.script_start_load(
        account,
        PROBE_SOURCE.to_owned(),
        script_js::LoadShape::CompatClass,
        None,
        vec![],
    )
    .expect("start compatibility v1 walking probe through normal Load");
    let result = parse_result(&play, account, CELL_DEADLINE);
    assert_eq!(
        result["blockedProbeable"].as_bool(),
        Some(true),
        "blocked tile: {result}"
    );
    assert_eq!(
        result["blockedWalkable"].as_bool(),
        Some(false),
        "blocked tile: {result}"
    );
    assert_eq!(
        result["otherPlaneProbeable"].as_bool(),
        Some(false),
        "other plane: {result}"
    );
    assert_eq!(
        result["insideEdgeProbeable"].as_bool(),
        Some(true),
        "loaded edge neighbor: {result}"
    );
    assert_eq!(
        result["sceneEdgeProbeable"].as_bool(),
        Some(false),
        "scene edge: {result}"
    );
    assert!(
        !result["insideLocal"].is_null(),
        "loaded edge neighbor: {result}"
    );
    assert_eq!(result["edgeLocal"], Value::Null, "scene edge: {result}");

    stop_script(&play, account);
    let head = std::process::Command::new("git")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("read tested tree commit");
    assert!(head.status.success(), "git rev-parse HEAD failed");
    let tested_tree = String::from_utf8(head.stdout)
        .expect("git rev-parse HEAD returns UTF-8")
        .trim()
        .to_owned();
    let evidence = required_path("LIVE_EVIDENCE_DIR");
    let receipt = json!({
        "cell": "compat_v1_walking_probeable",
        "product_commit": std::env::var("COMPAT_W2_PRODUCT_COMMIT").expect("product commit"),
        "engine": "Engine L",
        "account": account,
        "tested_tree": tested_tree,
        "result": result,
    });
    std::fs::write(
        evidence.join("compat-v1-walking-probeable-receipt.json"),
        serde_json::to_vec_pretty(&receipt).expect("serialize live receipt"),
    )
    .expect("write live receipt");
    println!("PASS: compat_v1_walking_probeable: {receipt}");
}
