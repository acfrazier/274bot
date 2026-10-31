//! Real Engine L proof for ClientAdapter's posted readers, live bank count
//! dialog, scene bounds and stale-session guards.

#![cfg(feature = "live-harness")]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::GameSnapshot;
use client::client::Client;
use client::render::Renderer;
use host_play::{
    mint_live_entries, mint_live_names, run_with_template, ProfileOptions, SharedClientTemplate,
};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings};

thread_local! {
    static CLIENT_ADAPTER_RENDERER: RefCell<Option<Renderer>> = const { RefCell::new(None) };
}

const TEST_NAME: &str = "reader_bank_count_stats_bounds_and_stale_guards";
const POLL: Duration = Duration::from_millis(50);
const PREP_DEADLINE: Duration = Duration::from_secs(240);
const PROBE_DEADLINE: Duration = Duration::from_secs(240);
const STOP_DEADLINE: Duration = Duration::from_secs(20);
const BANK_TILE: (i32, i32, i32) = (3093, 3243, 0);
const PROBE_SOURCE: &str = r#"import { LoopingBot } from '../../api/bot/Bot.js';
import { reader, actions } from '../../adapter/ClientAdapter.js';
import { Bank } from '../../api/bank/Bank.js';
import { Game } from '../../api/game/Game.js';
import { Execution } from '../../api/execution/Execution.js';
import { Input } from '../../input/Input.js';
import { Inventory } from '../../api/inventory/Inventory.js';
async function face(x, z) {
    const here = Game.tile();
    if (!here) return false;
    const yaw = (Math.trunc(Math.atan2(x - here.x, z - here.z) * -325.949) & 0x7ff);
    if (!Game.setCameraYaw(yaw)) return false;
    if (!(await Execution.delayUntil(() => Game.cameraYaw() === yaw, 3000))) return false;
    await Execution.delayTicks(1);
    return true;
}

function npcBoundsFor(rows) {
    for (const npc of rows) {
        const points = reader.npcBox(npc.index);
        if (Array.isArray(points) && points.length === 8) {
            return { index: npc.index, id: npc.id, points };
        }
    }
    return null;
}

const bankLoc = __BANK_LOC_JSON__;

export default class ClientAdapterLiveProbe extends LoopingBot {
    complete = false;

    async loop() {
        if (this.complete) return;
        this.complete = true;
        const attack = reader.stat(0);
        if (attack.base !== 20) {
            throw new Error(`ClientAdapter attack stat was ${attack.base} after login, expected 20`);
        }
        if (reader.skillCount() !== 25) {
            throw new Error(`ClientAdapter skill catalog count was ${reader.skillCount()}, expected 25`);
        }
        // Face the selected booth before the input and its projected bounds.
        if (!(await face(bankLoc.x, bankLoc.z))) {
            throw new Error('ClientAdapter probe could not face the seeded Draynor bank');
        }
        // The ClientAdapter shim resolves location input by world tile.
        if (!Input.interactLoc(bankLoc.x, bankLoc.z, bankLoc.typecode, bankLoc.op)) {
            throw new Error('ClientAdapter probe could not open the seeded Draynor bank');
        }
        if (!(await Execution.delayUntil(
            () => reader.bankSnapshotReady() && reader.bankItems().length > 0,
            3000,
        ))) {
            throw new Error('ClientAdapter bank snapshot did not become ready');
        }
        const bankGeneration = reader.bankSnapshotGeneration();
        const coins = reader.bankItems().find((row) => row.name === 'Coins' && row.count > 0);
        if (bankGeneration < 0 || !coins) throw new Error('ClientAdapter bank session or seeded coins missing');
        const withdrawX = coins.ops.findIndex((op) => typeof op === 'string' && /^withdraw[\s-]*x$/i.test(op));
        if (withdrawX < 0) throw new Error('live bank did not expose Withdraw X');
        if (!(await Input.invButton(coins.id, coins.slot, coins.comId, withdrawX + 1))) {
            throw new Error('ClientAdapter probe could not select Withdraw X');
        }
        if (!(await Execution.delayUntil(() => reader.countDialogOpen(), 3000))) {
            throw new Error('Withdraw X did not open the count dialog');
        }
        let npcRows = reader.npcs();
        let npcBounds = npcBoundsFor(npcRows);
        let locPoints = reader.locBox({
            id: bankLoc.id,
            x: bankLoc.x,
            z: bankLoc.z,
            level: bankLoc.level,
        });
        if (!npcBounds || !Array.isArray(locPoints) || locPoints.length !== 8) {
            const npcTargets = npcRows
                .filter((npc) => Number.isInteger(npc.x) && Number.isInteger(npc.z))
                .slice()
                .sort((left, right) => {
                    const leftDistance = Number.isFinite(left.distance) ? left.distance : Infinity;
                    const rightDistance = Number.isFinite(right.distance) ? right.distance : Infinity;
                    return leftDistance - rightDistance;
                });
            for (const targetNpc of npcTargets) {
                if (!(await face(targetNpc.x, targetNpc.z))) continue;
                npcRows = reader.npcs();
                npcBounds = npcBoundsFor(npcRows);
                locPoints = reader.locBox({
                    id: bankLoc.id,
                    x: bankLoc.x,
                    z: bankLoc.z,
                    level: bankLoc.level,
                });
                if (npcBounds && Array.isArray(locPoints) && locPoints.length === 8) break;
            }
        }
        const locBounds = Array.isArray(locPoints) && locPoints.length === 8
            ? { id: bankLoc.id, x: bankLoc.x, z: bankLoc.z, points: locPoints }
            : null;
        if (!npcBounds || !locBounds) {
            const npcDiagnostics = npcRows.slice(0, 8).map((npc) => {
                const points = reader.npcBox(npc.index);
                const pointCount = Array.isArray(points) ? points.length : -1;
                return `${npc.index}@${npc.x},${npc.z}/d${npc.distance}:box=${pointCount}`;
            });
            throw new Error(
                `ClientAdapter did not expose real NPC and loc bounds (npcCount=${npcRows.length}, npcs=${npcDiagnostics.join(',')}, locPoints=${Array.isArray(locPoints) ? locPoints.length : -1}, cameraYaw=${Game.cameraYaw()}, cameraPitch=${Game.cameraPitch()})`,
            );
        }

        const unknownControl = {
            button: reader.buttonByText(2147483647, 'unknown control'),
            close: reader.closeButtonComId(2147483647),
        };
        if (unknownControl.button !== -1 || unknownControl.close !== -1) {
            throw new Error(`unknown ClientAdapter controls did not use -1: ${JSON.stringify(unknownControl)}`);
        }
        const modalBefore = reader.modalCloseObservation();
        const countDialogWasOpen = reader.countDialogOpen();
        const coinsBefore = Inventory.count('Coins');
        if (!actions.answerCountDialog(7)) throw new Error('ClientAdapter count answer was refused while the dialog was open');
        if (!(await Execution.delayUntil(() => Inventory.count('Coins') === coinsBefore + 7, 4000))) {
            throw new Error('ClientAdapter count answer did not withdraw seven coins');
        }
        if (!(await Bank.close())) throw new Error('ClientAdapter probe could not close the bank');
        if (!(await Execution.delayUntil(() => reader.bankSnapshotGeneration() === -1, 3000))) {
            throw new Error('closed bank retained a stale ClientAdapter bank generation');
        }
        const staleCountAnswer = actions.answerCountDialog(1);
        if (staleCountAnswer !== false) throw new Error('stale count-dialog action was not refused');
        const modalAfter = reader.modalCloseObservation();
        const result = {
            attack,
            statsReady: reader.statsReady(),
            skillCount: reader.skillCount(),
            bankGeneration,
            countDialogWasOpen,
            countDialogClosedAfterBankClose: !reader.countDialogOpen(),
            staleBankGeneration: reader.bankSnapshotGeneration(),
            staleCountAnswer,
            modalSessionType: typeof modalBefore.session,
            sameModalSession: modalBefore.session === modalAfter.session,
            npcCount: npcRows.length,
            coinsBefore,
            coinsAfter: Inventory.count('Coins'),
            bankBooth: { id: bankLoc.id, x: bankLoc.x, z: bankLoc.z },
            npc: npcBounds,
            loc: locBounds,
            unknownControl,
        };
        this.log(`ClientAdapterLive: ${JSON.stringify(result)}`);
    }
}
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrepPhase {
    InitialReady,
    InitialTutorial,
    WaitInitialTutorial,
    WaitLogout,
    WaitRelog,
    ReseedTutorial,
    WaitReseed,
    SeedAccount,
    WaitSeed,
    Ready,
}

#[derive(Default)]
struct SeedPrep {
    phase: Option<PrepPhase>,
    error: Option<String>,
    tutorial_reseed: Option<scenario::tutorial::PostRelogTutorial>,
    evidence: Option<Value>,
    rendered_yaw: Option<i32>,
}

/// `npc_overlay_box` needs the model height populated by rendering. The live
/// adapter harness is headless by default, so render at each camera yaw that
/// the probe inspects.
fn render_ready_frame(client: &mut Client, prep: &mut SeedPrep) -> Result<(), String> {
    if prep.rendered_yaw == Some(client.orbit_camera_yaw) {
        return Ok(());
    }
    let is_cpu_frame = CLIENT_ADAPTER_RENDERER.with(|cell| {
        let mut renderer = cell.borrow_mut();
        let renderer =
            renderer.get_or_insert_with(|| Renderer::new_prefer(client.config.lowmem, false));
        let was_draw = client.draw;
        client.set_draw(true);
        let frame = renderer.mainredraw(client);
        client.set_draw(was_draw);
        matches!(frame, client::render::backend::FrameOutput::PixMap(_))
    });
    if !is_cpu_frame {
        return Err("ClientAdapter live scene render did not produce a CPU PixMap".into());
    }
    prep.rendered_yaw = Some(client.orbit_camera_yaw);
    Ok(())
}

fn send_cheat(client: &mut Client, command: &str) -> Result<(), String> {
    if interact::cheat(client, command).is_sent() {
        Ok(())
    } else {
        Err(format!(
            "live ClientAdapter seed command was refused: {command}"
        ))
    }
}
fn nearby_bank_loc(snapshot: &GameSnapshot) -> Option<Value> {
    snapshot
        .locs()
        .iter()
        .filter_map(|loc| {
            if loc.distance > 8
                || !loc
                    .name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case("Bank booth"))
            {
                return None;
            }
            let op = loc.actions.iter().position(|action| {
                action
                    .as_deref()
                    .is_some_and(|action| action.eq_ignore_ascii_case("Use-quickly"))
            })?;
            let op = i32::try_from(op + 1).ok()?;
            if !(1..=5).contains(&op) {
                return None;
            }
            Some((
                loc.distance,
                json!({
                    "id": loc.id,
                    "typecode": loc.typecode,
                    "x": loc.tile.x,
                    "z": loc.tile.z,
                    "level": loc.tile.level,
                    "op": op,
                }),
            ))
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, bank_loc)| bank_loc)
}

fn seed_frame(client: &mut Client, held: bool, shared: &Mutex<SeedPrep>) {
    let mut prep = shared.lock().unwrap_or_else(|error| error.into_inner());
    if prep.error.is_some() {
        return;
    }
    if prep.phase == Some(PrepPhase::Ready) {
        if let Err(error) = render_ready_frame(client, &mut prep) {
            prep.error = Some(error);
        }
        return;
    }
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(client);
    if prep.phase == Some(PrepPhase::WaitLogout) && !client.ingame {
        prep.phase = Some(PrepPhase::WaitRelog);
        return;
    }
    if held {
        return;
    }
    let result = (|| -> Result<(), String> {
        match prep.phase.unwrap_or(PrepPhase::InitialReady) {
            PrepPhase::InitialReady if client.ingame && client.scene_state == 2 => {
                interact::mainland_hop(client);
                prep.phase = Some(PrepPhase::InitialTutorial);
            }
            PrepPhase::InitialTutorial => {
                send_cheat(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                prep.phase = Some(PrepPhase::WaitInitialTutorial);
            }
            PrepPhase::WaitInitialTutorial
                if scenario::tutorial::tutorial_confirmed(&snapshot, 0) =>
            {
                let ifaces = Arc::clone(&client.ifaces);
                if !interact::logout(client, &ifaces) {
                    return Err(
                        "logout interface unavailable during ClientAdapter seed preparation".into(),
                    );
                }
                prep.phase = Some(PrepPhase::WaitLogout);
            }
            PrepPhase::WaitRelog
                if client.ingame
                    && client.scene_state == 2
                    && snapshot
                        .side_tabs()
                        .iter()
                        .any(|tab| tab.index == 3 && tab.available) =>
            {
                prep.phase = Some(PrepPhase::ReseedTutorial);
            }
            PrepPhase::ReseedTutorial => {
                let baseline = scenario::tutorial::chat_baseline(&snapshot);
                send_cheat(client, scenario::tutorial::TUTORIAL_SETVAR)?;
                send_cheat(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                prep.tutorial_reseed = Some(scenario::tutorial::PostRelogTutorial::new(baseline));
                prep.phase = Some(PrepPhase::WaitReseed);
            }
            PrepPhase::WaitReseed => {
                let confirmed = prep
                    .tutorial_reseed
                    .as_ref()
                    .expect("ClientAdapter tutorial confirmation was armed")
                    .check(&snapshot)?;
                if confirmed {
                    prep.phase = Some(PrepPhase::SeedAccount);
                }
            }
            PrepPhase::SeedAccount => {
                send_cheat(client, "setstat attack 20")?;
                send_cheat(client, "setstat defence 98")?;
                send_cheat(client, "setstat hitpoints 99")?;
                send_cheat(client, "givebank coins 300")?;
                send_cheat(
                    client,
                    &interact::tele_args(BANK_TILE.2, BANK_TILE.0, BANK_TILE.1),
                )?;
                prep.phase = Some(PrepPhase::WaitSeed);
            }
            PrepPhase::WaitSeed => {
                let attack = snapshot
                    .stats()
                    .iter()
                    .find(|stat| stat.name.eq_ignore_ascii_case("attack"))
                    .map(|stat| stat.base);
                let tile = snapshot.tile();
                if client.ingame
                    && client.scene_state == 2
                    && tile == Some(BANK_TILE)
                    && attack == Some(20)
                {
                    let bank_loc = nearby_bank_loc(&snapshot).ok_or_else(|| {
                        "no nearby Draynor bank booth with Use-quickly".to_string()
                    })?;
                    prep.evidence = Some(json!({
                        "tutorial_relogin_confirmed": true,
                        "observed_attack": attack,
                        "observed_tile": tile,
                        "ingame": client.ingame,
                        "scene_state": client.scene_state,
                        "bank_loc": bank_loc,
                    }));
                    render_ready_frame(client, &mut prep)?;
                    prep.phase = Some(PrepPhase::Ready);
                }
            }
            _ => {}
        }
        Ok(())
    })();
    if let Err(error) = result {
        prep.error = Some(error);
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required for the ClientAdapter live cell"))
}

fn required_path(name: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(required(name)?);
    if !path.is_absolute() {
        return Err(format!("{name} must be absolute: {}", path.display()));
    }
    Ok(path)
}

fn git_output(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| format!("run git in {}: {error}", root.display()))?;
    if !output.status.success() {
        return Err(format!(
            "git -C {} {} failed: {}",
            root.display(),
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|error| format!("git output is not UTF-8: {error}"))
}

fn run_cell(receipt: &mut Value) -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Err("LIVE=1 is required; ignored live tests must not pass vacuously".into());
    }
    if std::env::var("BOT_CPU").as_deref() != Ok("1") {
        return Err("BOT_CPU=1 is required for headless live scene capture".into());
    }
    if std::env::var("BOT_NAV_BUILD").as_deref() != Ok("skip") {
        return Err("BOT_NAV_BUILD=skip and the explicit v17 WORLD_NAV_PACK are required".into());
    }
    if required("WORLD_GAME_PORT")? != "47594" || required("WORLD_HTTP_PORT")? != "3080" {
        return Err("ClientAdapter live cell requires Aubaine Engine L ports 47594/3080".into());
    }
    let prefix = required("BOT_LIVE_NAME_PREFIX")?;
    if prefix.is_empty() || prefix.len() > 4 {
        return Err("BOT_LIVE_NAME_PREFIX must contain one to four characters".into());
    }
    let test_repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let product = required("CLIENT_ADAPTER_PRODUCT_COMMIT")?;
    let tested_tree = git_output(&test_repo, &["rev-parse", "HEAD"])?;
    let product_full = git_output(
        &test_repo,
        &["rev-parse", "--verify", &format!("{product}^{{commit}}")],
    )?;
    let ancestor = Command::new("git")
        .arg("-C")
        .arg(&test_repo)
        .args(["merge-base", "--is-ancestor", &product_full, &tested_tree])
        .status()
        .map_err(|error| format!("check ClientAdapter product commit ancestry: {error}"))?;
    if !ancestor.success() {
        return Err(format!(
            "product commit {product_full} is not an ancestor of tested tree {tested_tree}"
        ));
    }
    let nav_pack = required_path("WORLD_NAV_PACK")?;
    let engine_dir = required_path("WORLD_ENGINE_DIR")?;
    let catalog_root = required_path("RS2B0T")?;
    let catalog_commit = git_output(&catalog_root, &["rev-parse", "--short=10", "HEAD"])?;
    if !catalog_commit.starts_with("ae56ccc95") {
        return Err(format!(
            "expected pinned ClientAdapter catalog ae56ccc95, found {catalog_commit}"
        ));
    }
    let cache_dir = required_path("BOT_CACHE_DIR")?;
    let evidence_dir = required_path("LIVE_EVIDENCE_DIR")?;
    let isolated = script::IsolatedEnv::enter(TEST_NAME);
    if evidence_dir.starts_with(&isolated.home) {
        return Err("LIVE_EVIDENCE_DIR must be outside the throwaway HOME".into());
    }
    for (name, path, file) in [
        ("WORLD_NAV_PACK", &nav_pack, true),
        ("WORLD_ENGINE_DIR", &engine_dir, false),
        ("RS2B0T", &catalog_root, false),
        ("BOT_CACHE_DIR", &cache_dir, false),
    ] {
        if file && !path.is_file() || !file && !path.is_dir() {
            return Err(format!("{name} is unavailable: {}", path.display()));
        }
    }
    std::fs::create_dir_all(&evidence_dir)
        .map_err(|error| format!("create {}: {error}", evidence_dir.display()))?;
    isolated.set_rs2b0t(&catalog_root);

    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(47594),
        http_port: Some(3080),
        cache_dir: Some(cache_dir),
        nav_pack: Some(nav_pack),
        engine_dir: Some(engine_dir),
        unpack_dir: Some(isolated.home.join(".274bot/unpack")),
        vault_path: Some(isolated.home.join("vault")),
        catalog_root: Some(catalog_root.clone()),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None)?.bind_runtime()?;
    let template = SharedClientTemplate::load(Arc::clone(&profile))?;
    if template.world().is_none() {
        return Err("selected 289 ClientAdapter template has no navigation world".into());
    }

    let names = mint_live_names(1);
    let entries = mint_live_entries(&names);
    if entries.len() != 1 {
        return Err("could not mint one ClientAdapter live account".into());
    }
    let (account, password) = &entries[0];
    receipt["account"] = json!(account);
    receipt["product_commit"] = json!(product_full);
    receipt["tested_tree_commit"] = json!(tested_tree);
    receipt["catalog_root"] = json!(catalog_root.display().to_string());
    receipt["catalog_commit"] = json!(catalog_commit);
    receipt["probe_source_sha256"] = json!(script_js::JsCache::origin_sha(PROBE_SOURCE.as_bytes()));

    let prep = Arc::new(Mutex::new(SeedPrep::default()));
    let frame_prep = Arc::clone(&prep);
    let frame_account = account.clone();
    let mut play = run_with_template(
        template,
        false,
        vec![],
        |_| (None, None),
        move |client, username, frame| {
            if username == frame_account {
                seed_frame(client, frame.hold, &frame_prep);
            }
        },
    )?;
    let uid = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock: {error}"))?
        .as_millis()
        % i32::MAX as u128) as i32;
    play.try_spawn_slot(
        Profile {
            username: account.clone(),
            password: password.clone().into(),
            uid,
            settings: ProfileSettings::default(),
        },
        None,
        None,
        None,
    )?;
    play.focus(account);

    let prep_deadline = Instant::now() + PREP_DEADLINE;
    loop {
        let state = prep.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(error) = &state.error {
            receipt["preparation"] = json!({
                "phase": format!("{:?}", state.phase),
                "error": error
            });
            return Err(format!("ClientAdapter seed preparation failed: {error}"));
        }
        if state.phase == Some(PrepPhase::Ready) {
            receipt["preparation"] = state.evidence.clone().unwrap_or(Value::Null);
            break;
        }
        let phase = state.phase;
        drop(state);
        if Instant::now() >= prep_deadline {
            return Err(format!(
                "ClientAdapter account seeding timed out in phase {phase:?}"
            ));
        }
        std::thread::sleep(POLL);
    }

    let bank_loc = receipt["preparation"]["bank_loc"].clone();
    if bank_loc.is_null() {
        return Err("ClientAdapter preparation did not capture a nearby bank booth".into());
    }
    let probe_source = PROBE_SOURCE.replace("__BANK_LOC_JSON__", &bank_loc.to_string());
    play.script_start_load(
        account,
        probe_source,
        script_js::LoadShape::CompatClass,
        None,
        vec![],
    )?;
    let deadline = Instant::now() + PROBE_DEADLINE;
    let mut logs = Vec::new();
    let probe_result = loop {
        logs.extend(play.script_take_pending_logs(account));
        if let Some(line) = logs
            .iter()
            .find(|line| line.starts_with("ClientAdapterLive: "))
        {
            break serde_json::from_str::<Value>(line.trim_start_matches("ClientAdapterLive: "))
                .map_err(|error| format!("ClientAdapter probe receipt is not JSON: {error}"));
        }
        if play.script_state(account) == script::RunState::Error {
            break Err(format!(
                "ClientAdapter probe failed: {:?}; logs={logs:?}",
                play.script_last_error(account)
            ));
        }
        if Instant::now() >= deadline {
            break Err(format!(
                "ClientAdapter probe exceeded its fixed deadline; logs={logs:?}"
            ));
        }
        std::thread::sleep(POLL);
    }?;
    receipt["result"] = probe_result;
    receipt["script_logs"] = json!(logs);

    play.script_stop(account);
    let stop_deadline = Instant::now() + STOP_DEADLINE;
    while play.script_state(account) != script::RunState::Idle && Instant::now() < stop_deadline {
        logs.extend(play.script_take_pending_logs(account));
        std::thread::sleep(POLL);
    }
    if play.script_state(account) != script::RunState::Idle {
        play.stop_slot(account);
        return Err("ClientAdapter live script did not stop before the cleanup deadline".into());
    }
    receipt["script_logs"] = json!(logs);
    Ok(())
}

fn run_and_record() -> Result<(), String> {
    let evidence_dir = PathBuf::from(required("LIVE_EVIDENCE_DIR")?);
    std::fs::create_dir_all(&evidence_dir)
        .map_err(|error| format!("create {}: {error}", evidence_dir.display()))?;
    let mut receipt = json!({
        "cell": TEST_NAME,
        "verdict": "FAIL",
        "engine": "Engine L",
        "host": "Aubaine",
        "game_port": 47594,
        "http_port": 3080,
        "bank_tile": BANK_TILE,
        "started_at_unix_ms": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("system clock: {error}"))?
            .as_millis(),
    });
    let result = run_cell(&mut receipt);
    let path = evidence_dir.join("receipt.json");
    if result.is_ok() {
        receipt["verdict"] = json!("PASS");
    } else if let Err(error) = &result {
        receipt["failure"] = json!(error);
    }
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("write {}: {error}", path.display()))?;
    result.map_err(|error| format!("{error}; evidence={}", path.display()))?;
    println!("PASS {TEST_NAME} evidence={}", path.display());
    Ok(())
}

#[test]
#[ignore = "requires LIVE=1, isolated HOME, pinned catalog and Aubaine Engine L"]
fn reader_bank_count_stats_bounds_and_stale_guards() {
    run_and_record().unwrap_or_else(|error| panic!("{error}"));
}
