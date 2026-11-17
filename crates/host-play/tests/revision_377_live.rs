//! Revision-377 constructor and bounded live interaction qualification.
//!
//! Tests are ignored and require `LIVE=1` plus an owned `BOT_LIVE_NAME_PREFIX`.
//! They use the canonical `--revision 377` profile with explicit paths
//! `ENGINE_DIR`, `CLIENT_377_CACHE_DIR`, `CLIENT_UNPACK_DIR`, `NAV_PACK`,
//! and `NAV_FLAGS`. `WORLD_GAME_PORT` and `WORLD_HTTP_PORT` may select another
//! local engine. The Make-X cell also requires `PARITY_3B_EVIDENCE_DIR`; the
//! two-player menu cell requires `PARITY_2C_EVIDENCE_DIR` and a served-identity
//! JSON file at `PARITY_2C_SERVED_IDENTITY`. Template preparation uses the
//! production runtime-cache path in a private home.
//! The measured-window cell requires `PARITY_1I_EVIDENCE_DIR` and records
//! the actual non-zero child exit for its intentionally INVALID variant.
//! Numeric NPC selection and reacquisition require a wall-valid adjacent
//! approach in the observed collision scene.
//! Pre-Start mainland fixtures request a fresh login after observing logout.
//! A server logout does not latch the slot, so an unlatched slot with
//! auto-login reconnects by itself; the fixture still posts a real one-shot
//! Login through its SlotArm once available, for its own relog.
//!
//! Live invocation (with the real resource paths supplied by the owner):
//! ```text
//! ${ISOHOME:?} env \
//!   LIVE=1 BOT_LIVE_NAME_PREFIX=f377 ENGINE_DIR=<377-engine> \
//!   CLIENT_377_CACHE_DIR=<377-raw-cache> CLIENT_UNPACK_DIR=<377-decoded-snapshot> \
//!   NAV_PACK=<377-nav-pack> NAV_FLAGS=<377-nav-flags> \
//!   PARITY_3B_EVIDENCE_DIR=<evidence-output> \
//!   PARITY_2C_EVIDENCE_DIR=<evidence-output> PARITY_2C_SERVED_IDENTITY=<identity-json> \
//!   cargo test --locked -p host-play --features test-support --test revision_377_live \
//!   -- --ignored --nocapture --test-threads=1
//! ```

#[path = "revision_377_live/gatherer.rs"]
mod gatherer;

#[path = "revision_377_live/machines.rs"]
mod machines;

#[path = "revision_377_live/cook.rs"]
mod cook;

#[path = "revision_377_live/quantity.rs"]
mod quantity;

#[path = "revision_377_live/menu.rs"]
mod menu;

#[path = "revision_377_live/interference.rs"]
mod interference;

#[path = "revision_377_live/keys.rs"]
mod keys;

#[path = "revision_377_live/quester.rs"]
mod quester;

#[path = "revision_377_live/quester_imp.rs"]
mod quester_imp;

#[path = "revision_377_live/actor.rs"]
mod actor;
#[path = "revision_377_live/quester_doric.rs"]
mod quester_doric;
#[path = "revision_377_live/quester_journal.rs"]
mod quester_journal;
#[path = "revision_377_live/varbit.rs"]
mod varbit;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact::{self, ActionSpec, Interactions, OpTarget, SendResult};
use api::query::SceneQuery;
use api::snapshot::{GameSnapshot, SceneView, WorldTile};
use client::client::Client;
use client::io::ClientRevision;
use host::Pump;
use host_play::profile::{NavAvailability, ProfileEnvironment};
use host_play::{ProfileClass, ProfileOptions, ServerProfile, SharedClientTemplate};
use serde_json::{json, Value};

const REVISION: u16 = 377;
const GAME_HOST: &str = "127.0.0.1";
const GAME_PORT: u16 = 43595;
const ASSET_PORT: u16 = 81;
const LUMBRIDGE_REGION: i32 = 50;
// Slot-377 content: `hans`, spawned in Lumbridge's castle courtyard.
const LUMBRIDGE_HANS_TYPE_ID: usize = 0;
const TALK_ACTION: &str = "Talk-to";

struct ScratchHome(PathBuf);

impl ScratchHome {
    fn new() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("system clock: {error}"))?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "274bot-revision-377-home-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)
            .map_err(|error| format!("create private profile home {}: {error}", path.display()))?;
        Ok(Self(path))
    }
}

impl Drop for ScratchHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Bound377 {
    _home: ScratchHome,
    engine_dir: PathBuf,
    cache_dir: PathBuf,
    unpack_dir: PathBuf,
    nav_pack: PathBuf,
    nav_flags: PathBuf,
    profile: Arc<ServerProfile>,
    template: Arc<SharedClientTemplate>,
}

fn required_path(name: &str) -> Result<PathBuf, String> {
    let value = std::env::var_os(name).ok_or_else(|| format!("{name} is required"))?;
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty() || !path.is_absolute() {
        return Err(format!("{name} must be a non-empty absolute path"));
    }
    Ok(path)
}

// The nightly controller owns this optional path (run_nightly.py).
fn nightly_quiet_lock_held() -> bool {
    nightly_quiet_lock_from(|name| std::env::var_os(name))
}

fn nightly_quiet_lock_from(read: impl FnOnce(&str) -> Option<std::ffi::OsString>) -> bool {
    read("NIGHTLY_QUIET_LOCK").is_some_and(|path| !path.is_empty() && Path::new(&path).exists())
}

fn require_live_guard() -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Err("set LIVE=1 to run revision-377 qualification".into());
    }
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX")
        .map_err(|_| "set BOT_LIVE_NAME_PREFIX to isolate the one live account")?;
    if !(1..=4).contains(&prefix.len()) {
        return Err("BOT_LIVE_NAME_PREFIX must be 1-4 characters".into());
    }
    if std::env::var_os("HOME").is_none() {
        return Err("run through isohome so HOME is throwaway".into());
    }
    Ok(())
}

fn path_is_dir(path: &Path, name: &str) -> Result<(), String> {
    if path.is_dir() {
        Ok(())
    } else {
        Err(format!("{name} is not a directory: {}", path.display()))
    }
}

fn path_is_file(path: &Path, name: &str) -> Result<(), String> {
    if path.is_file() {
        Ok(())
    } else {
        Err(format!("{name} is not a file: {}", path.display()))
    }
}

fn load_bound_377() -> Result<Bound377, String> {
    require_live_guard()?;
    let game_port = host_play::live_ports::env_port("WORLD_GAME_PORT", GAME_PORT);
    let asset_port = host_play::live_ports::env_port("WORLD_HTTP_PORT", ASSET_PORT);
    let engine_dir = required_path("ENGINE_DIR")?;
    let cache_dir = required_path("CLIENT_377_CACHE_DIR")?;
    let unpack_dir = required_path("CLIENT_UNPACK_DIR")?;
    let nav_pack = required_path("NAV_PACK")?;
    let nav_flags = required_path("NAV_FLAGS")?;
    path_is_dir(&engine_dir, "ENGINE_DIR")?;
    path_is_dir(&cache_dir, "CLIENT_377_CACHE_DIR")?;
    path_is_dir(&unpack_dir, "CLIENT_UNPACK_DIR")?;
    path_is_file(&nav_pack, "NAV_PACK")?;
    path_is_file(&nav_flags, "NAV_FLAGS")?;

    let home = ScratchHome::new()?;
    if game_port != GAME_PORT || asset_port != ASSET_PORT {
        let mut servers = host_play::Servers::builtins();
        let local = servers
            .servers
            .iter_mut()
            .find(|profile| profile.name == "local-377")
            .expect("built-in local-377 profile");
        local.worlds[0].port = game_port;
        local.worlds[0].asset_port = asset_port;
        let bot_dir = home.0.join(".274bot");
        fs::create_dir_all(&bot_dir).map_err(|error| error.to_string())?;
        let bytes = serde_json::to_vec_pretty(&servers).map_err(|error| error.to_string())?;
        fs::write(bot_dir.join("servers.json"), bytes).map_err(|error| error.to_string())?;
    }
    let environment = ProfileEnvironment {
        home: Some(home.0.clone()),
        engine_dir: Some(engine_dir.clone()),
        unpack_dir: Some(unpack_dir.clone()),
        nav_pack: Some(nav_pack.clone()),
        nav_flags: Some(nav_flags.clone()),
        ..ProfileEnvironment::default()
    };
    let options = ProfileOptions {
        // A saved local roster retains generated-data admission at custom ports.
        revision: Some(REVISION.to_string()),
        engine_dir: Some(engine_dir.clone()),
        cache_dir: Some(cache_dir.clone()),
        unpack_dir: Some(unpack_dir.clone()),
        nav_pack: Some(nav_pack.clone()),
        nav_flags: Some(nav_flags.clone()),
        ..ProfileOptions::default()
    };
    let selection = options.resolve_with_env(None, &environment)?;
    if selection.revision() != ClientRevision::R377
        || selection.profile_class() != ProfileClass::Local
        || selection.game_host() != GAME_HOST
        || selection.game_port() != game_port
        || selection.asset_host() != GAME_HOST
        || selection.asset_port() != asset_port
        || selection.engine_dir() != engine_dir.as_path()
        || selection.cache_dir() != cache_dir.as_path()
        || selection.unpack_dir() != unpack_dir.as_path()
        || selection.nav_pack() != nav_pack.as_path()
        || selection.nav_flags() != nav_flags.as_path()
    {
        return Err("resolved profile differs from the explicit 377 localhost resources".into());
    }

    let template = selection.prepare_template()?;
    let profile = Arc::clone(template.profile());
    let prepared = profile
        .prepared_cache()
        .ok_or("production template did not retain its runtime cache")?;
    let profile_world = profile
        .world()
        .ok_or("bound 377 profile has no loaded navigation world")?;
    if profile.revision() != ClientRevision::R377
        || profile.profile_class() != ProfileClass::Local
        || profile.client().revision() != ClientRevision::R377
        || profile.client().game_host() != GAME_HOST
        || profile.client().game_port() != game_port
        || profile.client().asset_host() != GAME_HOST
        || profile.client().asset_port() != asset_port
        || profile.client().cache_dir() != prepared.jag_dir.as_path()
        || profile.client().unpack_dir() != prepared.unpack_root()
        || profile.nav_pack() != nav_pack.as_path()
        || profile.nav_flags() != nav_flags.as_path()
        || profile.nav_availability() != &NavAvailability::Bound
    {
        return Err("bound profile lost the explicit 377 resources or navigation world".into());
    }
    let data = profile.game_data().ok_or_else(|| {
        format!(
            "377 generated game data was not attached: {}",
            profile.game_data_status().detail()
        )
    })?;
    if data.revision() != REVISION as i32 {
        return Err(format!(
            "bound game data revision {} does not match selected revision {REVISION}",
            data.revision()
        ));
    }

    let template_world = template
        .world()
        .ok_or("production template did not retain the loaded navigation world")?;
    if !Arc::ptr_eq(template.profile(), &profile)
        || !Arc::ptr_eq(&profile_world, &template_world)
        || template.game_data().is_none()
    {
        return Err(
            "production shared template lost the bound 377 profile, game data, or nav world".into(),
        );
    }

    Ok(Bound377 {
        _home: home,
        engine_dir,
        cache_dir,
        unpack_dir,
        nav_pack,
        nav_flags,
        profile,
        template,
    })
}

fn assert_377_client(client: &Client, lowmem: bool) -> Result<(), String> {
    if client.revision() != ClientRevision::R377 {
        return Err(format!(
            "prepared client revision is {}, not 377",
            client.revision().as_i32()
        ));
    }
    if client.config.lowmem != lowmem {
        return Err(format!(
            "prepared client lowmem={} does not match requested mode {lowmem}",
            client.config.lowmem
        ));
    }
    if !client.cache_from_shared {
        return Err("prepared client did not retain the production shared cache".into());
    }
    let Some(sidecar) = client.cache.revision_377.as_deref() else {
        return Err("prepared 377 client fell back to a legacy cache without 377 sidecars".into());
    };
    let config = &sidecar.config;
    if client.cache.locs.is_empty()
        || client.cache.loc_morphs.is_empty()
        || client.cache.loc_morphs.len() >= client.cache.locs.len()
    {
        return Err(format!(
            "377 location cache is empty or its morph records are not sparse: locations={}, morphs={}",
            client.cache.locs.len(),
            client.cache.loc_morphs.len()
        ));
    }
    for (name, decoded, adapted) in [
        ("objs", config.objs.len(), client.cache.objs.len()),
        ("npcs", config.npcs.len(), client.cache.npcs.len()),
        ("seqs", config.seqs.len(), client.cache.seqs.len()),
        ("varps", config.varps.len(), client.cache.varps.len()),
        ("varbits", config.varbits.len(), client.cache.varbits.len()),
    ] {
        if decoded == 0 || decoded != adapted {
            return Err(format!(
                "377 {name} sidecar is empty or misaligned: sidecar={decoded}, runtime={adapted}"
            ));
        }
    }
    if client.ifaces.iter().all(Option::is_none)
        || sidecar.interfaces.type1_fields.len() != client.ifaces.len()
        || sidecar.interfaces.null_string_masks.len() != client.ifaces.len()
        || sidecar.interfaces.initial_overlays.len() != client.ifaces_mut.len()
        || sidecar.interfaces.initial_overlays.len() != client.ifaces.len()
    {
        return Err(
            "377 interface sidecars are absent or not aligned with decoded components".into(),
        );
    }
    Ok(())
}

fn fresh_uid(offset: i32) -> i32 {
    37_700 + offset
}

fn resource_record(bound: &Bound377) {
    let data = bound.profile.game_data().expect("checked 377 game data");
    println!(
        "{}",
        json!({
            "test": "revision_377_live",
            "phase": "resources",
            "profile": bound.profile.label(),
            "revision": REVISION,
            "game_host": bound.profile.client().game_host(),
            "game_port": bound.profile.client().game_port(),
            "asset_host": bound.profile.client().asset_host(),
            "asset_port": bound.profile.client().asset_port(),
            "engine_dir": bound.engine_dir,
            "source_cache_dir": bound.cache_dir,
            "source_unpack_dir": bound.unpack_dir,
            "bound_cache_dir": bound.profile.client().cache_dir(),
            "bound_unpack_dir": bound.profile.client().unpack_dir(),
            "nav_pack": bound.nav_pack,
            "nav_flags": bound.nav_flags,
            "game_data_revision": data.revision(),
            "game_data_status": bound.profile.game_data_status().to_json(),
            "navigation": format!("{:?}", bound.profile.nav_availability()),
        })
    );
}

#[test]
#[ignore = "explicit 377 cache/snapshot/nav resources; LIVE=1 and an owned BOT_LIVE_NAME_PREFIX are required"]
fn retained_377_template_shares_mode_correct_revision_bound_caches() -> Result<(), String> {
    let bound = load_bound_377()?;
    resource_record(&bound);

    let low_a = bound.template.prepare_client(fresh_uid(0), true)?;
    let low_b = bound.template.prepare_client(fresh_uid(1), true)?;
    let high_a = bound.template.prepare_client(fresh_uid(2), false)?;
    let high_b = bound.template.prepare_client(fresh_uid(3), false)?;
    for client in [&low_a, &low_b] {
        assert_377_client(client, true)?;
    }
    for client in [&high_a, &high_b] {
        assert_377_client(client, false)?;
    }
    if !Arc::ptr_eq(&low_a.cache, &low_b.cache)
        || !Arc::ptr_eq(&high_a.cache, &high_b.cache)
        || Arc::ptr_eq(&low_a.cache, &high_a.cache)
    {
        return Err("prepared 377 clients do not share the exact cache within, and only within, one memory mode".into());
    }
    if !Arc::ptr_eq(&low_a.ifaces, &low_b.ifaces) || !Arc::ptr_eq(&high_a.ifaces, &high_b.ifaces) {
        return Err(
            "prepared 377 clients do not share immutable interface tables within their memory mode"
                .into(),
        );
    }
    println!(
        "{}",
        json!({
            "test": "revision_377_live",
            "phase": "retained-constructor",
            "revision": REVISION,
            "lowmem_requested": low_a.config.lowmem,
            "highmem_requested": high_a.config.lowmem,
            "lowmem_cache_shared": Arc::ptr_eq(&low_a.cache, &low_b.cache),
            "highmem_cache_shared": Arc::ptr_eq(&high_a.cache, &high_b.cache),
            "cross_mode_cache_shared": Arc::ptr_eq(&low_a.cache, &high_a.cache),
            "lowmem_377_sidecar": low_a.cache.revision_377.is_some(),
            "highmem_377_sidecar": high_a.cache.revision_377.is_some(),
        })
    );
    Ok(())
}

fn observation(snapshot: &GameSnapshot) -> Value {
    json!({
        "ingame": snapshot.ingame(),
        "attached": snapshot.attached(),
        "scene_state": snapshot.scene_state(),
        "tile": snapshot.tile(),
        "tick": snapshot.tick(),
        "npcs": snapshot.npcs().iter().map(|npc| json!({
            "index": npc.index,
            "type_id": npc.r#type,
            "name": npc.name,
            "tile": npc.tile,
            "actions": npc.actions,
        })).collect::<Vec<_>>(),
        "chat_modal": snapshot.modals().chat,
        "chat_texts": snapshot.chat_modal_texts(),
    })
}

fn record(phase: &str, snapshot: &GameSnapshot, detail: Value) {
    println!(
        "{}",
        json!({
            "test": "revision_377_live",
            "phase": phase,
            "state": observation(snapshot),
            "detail": detail,
        })
    );
}

fn accepted(result: SendResult<'_>, phase: &str) -> Result<(), String> {
    match result {
        SendResult::Sent { .. } => Ok(()),
        SendResult::Refused { reason, .. } => Err(format!("{phase}: refused {reason:?}")),
    }
}

fn wait_for(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    phase: &str,
    timeout: Duration,
    mut ready: impl FnMut(&GameSnapshot) -> bool,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let started = Instant::now();
        client.mainloop();
        host::publish_snapshot(snapshot, client, pump.drain_client(client));
        if ready(snapshot) {
            return Ok(());
        }
        if client.error_loading || Instant::now() >= deadline {
            record(
                phase,
                snapshot,
                json!({"outcome":"FAIL", "timeout_seconds":timeout.as_secs()}),
            );
            return Err(format!("{phase}: readiness predicate failed"));
        }
        if let Some(rest) = Duration::from_millis(20).checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

fn lumbridge(tile: WorldTile) -> bool {
    tile.level == 0
        && tile.x.div_euclid(64) == LUMBRIDGE_REGION
        && tile.z.div_euclid(64) == LUMBRIDGE_REGION
}

#[derive(Clone, Copy)]
struct NpcChoice {
    type_id: usize,
    index: usize,
    tile: WorldTile,
    distance: u32,
}

fn nearest_talk_npc(snapshot: &GameSnapshot, wanted_type: usize) -> Option<NpcChoice> {
    let (x, z, level) = snapshot.tile()?;
    let player_tile = WorldTile { x, z, level };
    let flood = SceneQuery::new(snapshot.scene(), Some(player_tile)).flood_reach()?;
    snapshot
        .npcs()
        .iter()
        .filter_map(|npc| {
            let type_id = npc.r#type?;
            if wanted_type != type_id
                || npc.network.level != level
                || !flood.at(&npc.network).1
                || !npc
                    .actions
                    .iter()
                    .flatten()
                    .any(|action| action.eq_ignore_ascii_case(TALK_ACTION))
            {
                return None;
            }
            let distance =
                u32::try_from(api::query::chebyshev_to(player_tile, npc.network)).ok()?;
            Some(NpcChoice {
                type_id,
                index: npc.index,
                tile: npc.network,
                distance,
            })
        })
        .min_by_key(|npc| (npc.distance, npc.type_id, npc.index))
}

fn direct_step_toward(
    scene: &SceneView,
    from: WorldTile,
    to: WorldTile,
) -> Result<WorldTile, String> {
    if !lumbridge(from) || !lumbridge(to) {
        return Err(format!(
            "direct walk endpoints must remain in Lumbridge: {from:?} to {to:?}"
        ));
    }
    let scene = SceneQuery::new(scene, Some(from));
    let dx = (to.x - from.x).signum();
    let dz = (to.z - from.z).signum();
    for (step_x, step_z) in [
        (dx, 0),
        (0, dz),
        (-dx, 0),
        (0, -dz),
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
    ] {
        let candidate = WorldTile {
            x: from.x + step_x,
            z: from.z + step_z,
            level: from.level,
        };
        if candidate != from && lumbridge(candidate) && scene.can_step(from, candidate) {
            return Ok(candidate);
        }
    }
    Err("no collision-reachable one-tile direct step remains inside loaded Lumbridge".into())
}

fn hans_patrol_points(source: &str) -> Result<Vec<(WorldTile, u32)>, String> {
    let section = source
        .split_once("[hans]\n")
        .ok_or("selected content has no [hans] NPC")?
        .1
        .split("\n[")
        .next()
        .unwrap_or_default();
    if !section.lines().any(|line| line == "defaultmode=patrol") {
        return Err("selected Hans does not use the expected patrol mode".into());
    }
    let mut route: Vec<(usize, WorldTile, u32)> = Vec::new();
    for line in section.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(ordinal) = key.strip_prefix("patrol") else {
            continue;
        };
        let ordinal: usize = ordinal.parse().map_err(|_| "invalid Hans patrol ordinal")?;
        let (coord, delay) = value
            .split_once(',')
            .ok_or("invalid Hans patrol waypoint")?;
        let coords = coord
            .split('_')
            .map(str::parse::<i32>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "invalid Hans patrol coordinate")?;
        let [level, mx, mz, x, z] = coords.as_slice() else {
            return Err("invalid Hans patrol coordinate width".into());
        };
        let tile = WorldTile {
            x: mx * 64 + x,
            z: mz * 64 + z,
            level: *level,
        };
        if !lumbridge(tile) {
            return Err("Hans patrol leaves the loaded Lumbridge region".into());
        }
        let delay: u32 = delay.parse().map_err(|_| "invalid Hans patrol delay")?;
        if route.iter().any(|(key, _, _)| *key == ordinal) {
            return Err("duplicate Hans patrol ordinal".into());
        }
        route.push((ordinal, tile, delay));
    }
    if route.len() < 2
        || (1..=route.len()).any(|ordinal| !route.iter().any(|(key, _, _)| *key == ordinal))
    {
        return Err("Hans patrol waypoints are missing or nonconsecutive".into());
    }
    Ok(route
        .into_iter()
        .map(|(_, tile, delay)| (tile, delay))
        .collect())
}

fn hans_stuck_ticks(source: &str) -> Result<u32, String> {
    source
        .split_once("this.stuckCounter >= ")
        .and_then(|(_, tail)| tail.split_whitespace().next())
        .ok_or("selected engine has no patrol stuck threshold")?
        .parse()
        .map_err(|_| "invalid patrol stuck threshold".into())
}

// Measured fixture calibration, not a period of the route: 17 November 2026,
// engine 13108a91 and slot-9 content; r1-observation/result.json records laps
// of 133 and 111 ticks. Each includes 31 idle ticks before a stuck teleport;
// lap 1 also has a 23-tick partial block. Npc.ts:378-380 resets stuckCounter
// on movement, so partial blocks can recur. Re-measure when the route changes.
const HANS_MEASURED_MAX_LAP_TICKS: u32 = 133;

fn hans_patrol_budget(stuck_ticks: u32, approach: u32) -> u32 {
    HANS_MEASURED_MAX_LAP_TICKS + stuck_ticks + approach
}

enum HansApproachAction {
    Dialogue,
    Deadline,
    Wait,
    Retry,
    Retarget(NpcChoice),
}

fn hans_approach_action(
    snapshot: &GameSnapshot,
    first_tick: u32,
    budget_ticks: u32,
    expired: bool,
    requested_tick: Option<u32>,
) -> HansApproachAction {
    if snapshot.modals().chat != -1 && !snapshot.chat_modal_texts().is_empty() {
        return HansApproachAction::Dialogue;
    }
    if !snapshot.ingame() || snapshot.tick().wrapping_sub(first_tick) >= budget_ticks || expired {
        return HansApproachAction::Deadline;
    }
    if requested_tick == Some(snapshot.tick()) {
        return HansApproachAction::Wait;
    }
    match nearest_talk_npc(snapshot, LUMBRIDGE_HANS_TYPE_ID) {
        Some(current) => HansApproachAction::Retarget(current),
        None => HansApproachAction::Retry,
    }
}

fn talk_to_patrolling_hans(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    budget_ticks: u32,
) -> Result<(), String> {
    let first_tick = snapshot.tick();
    let deadline = Instant::now() + Duration::from_millis(u64::from(budget_ticks) * 600);
    let mut requested_tick = None;
    let mut ever_observed_hans = false;
    let mut route_status: Result<&str, String> = Ok("not requested");
    loop {
        let started = Instant::now();
        let wall_expired = started >= deadline;
        match hans_approach_action(
            snapshot,
            first_tick,
            budget_ticks,
            client.error_loading || wall_expired,
            requested_tick,
        ) {
            HansApproachAction::Dialogue => {
                record(
                    "npc-dialogue-applied",
                    snapshot,
                    json!({"budget_ticks":budget_ticks}),
                );
                return Ok(());
            }
            HansApproachAction::Deadline => {
                let hans = snapshot
                    .npcs()
                    .iter()
                    .filter(|npc| npc.r#type == Some(LUMBRIDGE_HANS_TYPE_ID))
                    .map(|npc| json!({"index":npc.index,"tile":npc.network}))
                    .collect::<Vec<_>>();
                let termination = if client.error_loading {
                    "client-error"
                } else if !snapshot.ingame() {
                    "left-game"
                } else if snapshot.tick().wrapping_sub(first_tick) >= budget_ticks {
                    "game-tick-deadline"
                } else {
                    "wall-clock-deadline"
                };
                record(
                    "npc-dialogue-deadline",
                    snapshot,
                    json!({
                        "termination":termination,"first_tick":first_tick,"tick":snapshot.tick(),
                        "budget_ticks":budget_ticks,"wall_clock_expired":wall_expired,
                        "hans_present":!hans.is_empty(),"hans":hans,
                        "ever_observed_hans":ever_observed_hans,
                        "reachable_talk_target":nearest_talk_npc(snapshot,LUMBRIDGE_HANS_TYPE_ID).is_some(),
                        "dialogue_open":snapshot.modals().chat != -1 && !snapshot.chat_modal_texts().is_empty(),
                        "player_tile":snapshot.tile(),"route_result":route_status,
                    }),
                );
                return Err(format!("Hans approach/dialogue failed: {termination}"));
            }
            HansApproachAction::Wait => {}
            HansApproachAction::Retry => {
                let present = snapshot
                    .npcs()
                    .iter()
                    .any(|npc| npc.r#type == Some(LUMBRIDGE_HANS_TYPE_ID));
                ever_observed_hans |= present;
                record(
                    "npc-current-tile-unavailable",
                    snapshot,
                    json!({
                        "hans_present":present,"selection":"no-reachable-Talk-target",
                        "budget_ticks":budget_ticks,"route_result":route_status,
                    }),
                );
                requested_tick = Some(snapshot.tick());
            }
            HansApproachAction::Retarget(current) => {
                ever_observed_hans = true;
                route_status = if current.distance > 1 {
                    accepted(
                        Interactions::new(snapshot, client).walk(current.tile),
                        "approach current Hans",
                    )
                    .map(|()| "walk-current-Hans")
                } else {
                    let npc = snapshot
                        .npcs()
                        .iter()
                        .find(|npc| npc.index == current.index)
                        .expect("selected Hans remains in the same snapshot");
                    accepted(
                        Interactions::new(snapshot, client)
                            .interact(OpTarget::Npc(npc), ActionSpec::Label(TALK_ACTION.into())),
                        "current Hans Talk-to",
                    )
                    .map(|()| "Talk-to-current-Hans")
                };
                record(
                    "npc-current-tile-retarget",
                    snapshot,
                    json!({
                        "npc_index":current.index,"npc_tile":current.tile,
                        "distance":current.distance,"budget_ticks":budget_ticks,
                        "route_result":route_status,
                    }),
                );
                requested_tick = Some(snapshot.tick());
            }
        }
        client.mainloop();
        host::publish_snapshot(snapshot, client, pump.drain_client(client));
        if let Some(rest) = Duration::from_millis(20).checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

fn run_live_interaction() -> Result<(), String> {
    let bound = load_bound_377()?;
    resource_record(&bound);
    let username = host_play::mint_live_names(1).remove(0);
    let password = host_play::profile_password(&username);
    let mut client = bound.template.prepare_client(fresh_uid(10), false)?;
    assert_377_client(&client, false)?;
    if !Arc::ptr_eq(
        client
            .session_profile()
            .ok_or("prepared client has no bound session profile")?,
        bound.profile.client(),
    ) {
        return Err("prepared client session is not bound to the selected 377 profile".into());
    }
    client.draw = false;
    client.maininit();
    if client.error_loading {
        return Err(format!(
            "377 asset initialization failed: {}",
            client.last_progress_message
        ));
    }
    assert_377_client(&client, false)?;

    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    let result = (|| {
        println!(
            "{}",
            json!({
                "test": "revision_377_live",
                "phase": "login-request",
                "revision": client.revision().as_i32(),
                "username": username,
                "password": "<redacted>",
            })
        );
        if !interact::login(&mut client, &username, &password, false) {
            return Err(format!(
                "377 login was refused: {} {}",
                client.login_mes1, client.login_mes2
            ));
        }
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "attached-scene2",
            Duration::from_secs(90),
            |state| {
                state.ingame()
                    && state.attached()
                    && state.scene_state() == 2
                    && state.tile().is_some()
            },
        )?;
        record(
            "mainland-hop-request",
            &snapshot,
            json!({"expected_region": [LUMBRIDGE_REGION, LUMBRIDGE_REGION, 0]}),
        );
        interact::mainland_hop(&mut client);
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "mainland-hop-arrival",
            Duration::from_secs(30),
            |state| {
                state.ingame()
                    && state.attached()
                    && state.scene_state() == 2
                    && state
                        .tile()
                        .is_some_and(|(x, z, level)| lumbridge(WorldTile { x, z, level }))
            },
        )?;
        let (x, z, level) = snapshot.tile().ok_or("scene2 has no player tile")?;
        let origin = WorldTile { x, z, level };
        if !lumbridge(origin) {
            record(
                "wrong-start-region",
                &snapshot,
                json!({"expected_region": [LUMBRIDGE_REGION, LUMBRIDGE_REGION, 0]}),
            );
            return Err(format!(
                "fresh 377 account did not attach in Lumbridge: {origin:?}"
            ));
        }
        record(
            "attached-lumbridge-scene2",
            &snapshot,
            json!({"revision":client.revision().as_i32(), "username":username}),
        );
        let source_path = bound
            .engine_dir
            .parent()
            .ok_or("engine has no content sibling")?
            .join("content/scripts/_unpack/377/all.npc");
        let source = fs::read_to_string(&source_path).map_err(|error| {
            format!(
                "read selected Hans patrol {}: {error}",
                source_path.display()
            )
        })?;
        let patrol = hans_patrol_points(&source)?;
        let choice = nearest_talk_npc(&snapshot, LUMBRIDGE_HANS_TYPE_ID);
        let target = direct_step_toward(
            snapshot.scene(),
            origin,
            choice.map_or(patrol[0].0, |npc| npc.tile),
        )?;
        if !lumbridge(target) || target.level != origin.level {
            return Err(format!(
                "direct walk escaped the loaded Lumbridge region: {origin:?} to {target:?}"
            ));
        }
        record(
            "direct-walk-request",
            &snapshot,
            json!({
                "from": origin,
                "target": target,
                "same_loaded_region": true,
                "navigation_router_used": false,
                "nearby_npc_type_id": choice.map(|npc| npc.type_id),
                "nearby_npc_index": choice.map(|npc| npc.index),
                "nearby_npc_tile": choice.map(|npc| npc.tile),
            }),
        );
        accepted(
            Interactions::new(&snapshot, &mut client).walk(target),
            "direct Lumbridge walk",
        )?;
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "direct-walk-arrival",
            Duration::from_secs(20),
            |state| state.tile() == Some((target.x, target.z, target.level)),
        )?;
        let after = snapshot.tile().ok_or("walk result has no player tile")?;
        let after = WorldTile {
            x: after.0,
            z: after.1,
            level: after.2,
        };
        if !lumbridge(after) {
            return Err(format!("direct walk left Lumbridge: {after:?}"));
        }
        record(
            "direct-walk-arrived",
            &snapshot,
            json!({"from": origin, "target": target, "same_loaded_region": true}),
        );

        if snapshot.modals().chat != -1 || !snapshot.chat_modal_texts().is_empty() {
            return Err("NPC dialogue was already open before the interaction".into());
        }
        let current = nearest_talk_npc(&snapshot, LUMBRIDGE_HANS_TYPE_ID);
        let approach = current.map_or_else(
            || {
                patrol
                    .iter()
                    .map(|(tile, _)| after.x.abs_diff(tile.x) + after.z.abs_diff(tile.z))
                    .max()
                    .expect("selected patrol contains at least two waypoints")
            },
            |npc| npc.distance,
        );
        let engine_source_path = bound.engine_dir.join("src/engine/entity/Npc.ts");
        let engine_source = fs::read_to_string(&engine_source_path).map_err(|error| {
            format!("read patrol rule {}: {error}", engine_source_path.display())
        })?;
        let stuck_ticks = hans_stuck_ticks(&engine_source)?;
        let budget = hans_patrol_budget(stuck_ticks, approach);
        record(
            "npc-calibrated-fixture-budget",
            &snapshot,
            json!({"source":source_path,"engine_rule_source":engine_source_path,
                "measured_max_lap_ticks":HANS_MEASURED_MAX_LAP_TICKS,
                "stuck_ticks":stuck_ticks,"approach_tiles":approach,
                "approach_from_current_hans":current.is_some(),
                "budget_ticks":budget,"tick_ms":600}),
        );
        talk_to_patrolling_hans(&mut client, &mut snapshot, &mut pump, budget)?;
        Ok(())
    })();
    if client.ingame {
        client.logout();
    }
    result
}

#[test]
fn hans_deadline_uses_the_selected_patrol_in_file_order() {
    let route = "[hans]\ndefaultmode=patrol\n\
patrol1=0_50_50_7_33,0\npatrol10=0_50_50_2_32,0\n\
patrol2=0_50_50_11_30,0\npatrol3=0_50_50_19_30,0\n\
patrol4=0_50_50_19_22,10\npatrol5=0_50_50_21_22,0\n\
patrol6=0_50_50_21_12,0\npatrol7=0_50_50_18_9,0\n\
patrol8=0_50_50_14_5,0\npatrol9=0_50_50_2_5,0\n[man]\n";
    let points = hans_patrol_points(route).unwrap();
    assert_eq!((points[1].0.x, points[1].0.z), (3202, 3232));
    assert_eq!(
        hans_stuck_ticks("if (this.stuckCounter >= 32 || changedFloor)"),
        Ok(32)
    );
    assert_eq!(hans_patrol_budget(32, 13), 178);
    assert_eq!(hans_patrol_budget(32, 1), 166);
    assert!(hans_stuck_ticks("").is_err());
    assert!(hans_patrol_points("[man]\n").is_err());
    assert!(hans_patrol_points("[hans]\ndefaultmode=wander\n").is_err());
    assert!(hans_patrol_points("[hans]\ndefaultmode=patrol\n").is_err());
}

fn hans_target_snapshot(network_x: i32) -> GameSnapshot {
    let mut snapshot = talk_target_snapshot([(1, network_x, 58), (2, 58, 51)]);
    snapshot.seed_ingame(2);
    let mut hans = snapshot.npcs()[0].clone();
    hans.r#type = Some(LUMBRIDGE_HANS_TYPE_ID);
    snapshot.seed_npcs(vec![hans]);
    snapshot
}

#[test]
fn hans_absence_retries_until_its_deadline() {
    let mut snapshot = hans_target_snapshot(51);
    snapshot.seed_npcs(vec![]);
    for tick in 40..44 {
        snapshot.seed_tick(tick);
        assert!(matches!(
            hans_approach_action(&snapshot, 40, 4, false, None),
            HansApproachAction::Retry
        ));
    }
    snapshot.seed_tick(44);
    assert!(matches!(
        hans_approach_action(&snapshot, 40, 4, false, None),
        HansApproachAction::Deadline
    ));
    snapshot.seed_tick(40);
    assert!(matches!(
        hans_approach_action(&snapshot, 40, 4, true, None),
        HansApproachAction::Deadline
    ));
}

#[test]
fn hans_transient_absence_and_unreachable_tile_do_not_end_the_approach() {
    let mut snapshot = hans_target_snapshot(49);
    let npcs = snapshot.npcs().to_vec();
    let scene = snapshot.scene().clone();
    snapshot.seed_npcs(vec![]);
    assert!(matches!(
        hans_approach_action(&snapshot, 0, 4, false, None),
        HansApproachAction::Retry
    ));
    snapshot.seed_npcs(npcs);
    let mut blocked = scene.clone();
    for z in 0..blocked.height {
        blocked.collision_flags[(blocked.height + z) as usize] = client::dash3d::CollisionFlag::W_E;
        blocked.collision_flags[(2 * blocked.height + z) as usize] =
            client::dash3d::CollisionFlag::W_W;
    }
    snapshot.seed_scene(blocked);
    snapshot.seed_tick(1);
    assert!(matches!(
        hans_approach_action(&snapshot, 0, 4, false, Some(0)),
        HansApproachAction::Retry
    ));
    snapshot.seed_scene(scene);
    snapshot.seed_tick(2);
    assert!(matches!(
        hans_approach_action(&snapshot, 0, 4, false, Some(1)),
        HansApproachAction::Retarget(current) if current.index == 1 && current.tile.x == 49
    ));
    assert!(matches!(
        hans_approach_action(&snapshot, 0, 4, false, Some(2)),
        HansApproachAction::Wait
    ));
    snapshot.seed_chat_modal(1, vec!["dialogue".into()]);
    assert!(matches!(
        hans_approach_action(&snapshot, 0, 4, false, Some(2)),
        HansApproachAction::Dialogue
    ));
}

#[test]
fn hans_present_without_dialogue_still_expires() {
    let mut snapshot = hans_target_snapshot(51);
    for tick in 70..74 {
        snapshot.seed_tick(tick);
        assert!(matches!(
            hans_approach_action(&snapshot, 70, 4, false, None),
            HansApproachAction::Retarget(_)
        ));
    }
    snapshot.seed_tick(74);
    assert!(matches!(
        hans_approach_action(&snapshot, 70, 4, false, None),
        HansApproachAction::Deadline
    ));
}

#[test]
fn nightly_quiet_lock_uses_the_configured_path() -> Result<(), String> {
    let home = ScratchHome::new()?;
    let lock = home.0.join("configured-quiet");
    let configured = |name: &str| {
        assert_eq!(name, "NIGHTLY_QUIET_LOCK");
        Some(lock.as_os_str().to_owned())
    };
    assert!(!nightly_quiet_lock_from(|_| None));
    assert!(!nightly_quiet_lock_from(|_| Some("".into())));
    assert!(!nightly_quiet_lock_from(configured));
    fs::create_dir(&lock).map_err(|error| error.to_string())?;
    assert!(nightly_quiet_lock_from(configured));
    Ok(())
}

fn talk_target_snapshot(rows: [(usize, i32, i32); 2]) -> GameSnapshot {
    let here = WorldTile {
        x: 50,
        z: 50,
        level: 0,
    };
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_tile(here);
    snapshot.seed_scene(SceneView {
        available: true,
        base_x: 48,
        base_z: 48,
        level: 0,
        width: 12,
        height: 12,
        collision_flags: vec![0; 12 * 12],
    });
    snapshot.seed_npcs(
        rows.into_iter()
            .map(|(index, network_x, rendered_x)| api::snapshot::NpcView {
                index,
                r#type: Some(1),
                name: None,
                actions: vec![Some(TALK_ACTION.into())],
                tile: WorldTile {
                    x: rendered_x,
                    ..here
                },
                distance: 1,
                animation: -1,
                animation_frame: 0,
                pose_animation: -1,
                orientation: 0,
                target_orientation: 0,
                overhead_text: None,
                spot_animation: -1,
                spot_animation_stamp: 0,
                health: 1,
                total_health: 1,
                face_entity: -1,
                target: None,
                moving: true,
                running: false,
                in_combat: false,
                level: 1,
                size: 1,
                network: WorldTile {
                    x: network_x,
                    ..here
                },
                x: rendered_x * 128 + 64,
                z: here.z * 128 + 64,
                yaw: 0,
            })
            .collect(),
    );
    snapshot
}

#[test]
fn network_coords_live_driver_selects_and_reacquires_the_server_talk_target() {
    let snapshot = talk_target_snapshot([(1, 51, 58), (2, 58, 51)]);
    let choice = nearest_talk_npc(&snapshot, 1).expect("server-near Talk target");
    assert_eq!(choice.index, 1);
    assert_eq!(
        choice.tile,
        WorldTile {
            x: 51,
            z: 50,
            level: 0
        }
    );
    assert_eq!(choice.distance, 1);
}

#[test]
fn talk_target_selection_rejects_a_nearer_npc_behind_a_closed_wall() {
    let mut snapshot = talk_target_snapshot([(1, 49, 58), (2, 58, 51)]);
    let mut scene = snapshot.scene().clone();
    for z in 0..scene.height {
        scene.collision_flags[(scene.height + z) as usize] = client::dash3d::CollisionFlag::W_E;
        scene.collision_flags[(2 * scene.height + z) as usize] = client::dash3d::CollisionFlag::W_W;
    }
    snapshot.seed_scene(scene);
    let choice = nearest_talk_npc(&snapshot, 1).expect("farther reachable Talk target");
    assert_eq!(choice.index, 2);
    snapshot.seed_npcs(vec![snapshot.npcs()[0].clone()]);
    assert!(nearest_talk_npc(&snapshot, 1).is_none());
}

#[test]
fn talk_target_selection_requires_observed_collision() {
    let mut snapshot = talk_target_snapshot([(1, 51, 58), (2, 58, 51)]);
    let mut scene = snapshot.scene().clone();
    scene.available = false;
    snapshot.seed_scene(scene);
    assert!(nearest_talk_npc(&snapshot, 1).is_none());
}

#[test]
fn talk_target_selection_never_falls_back_to_another_type() {
    let mut snapshot = talk_target_snapshot([(1, 51, 51), (2, 58, 58)]);
    assert!(nearest_talk_npc(&snapshot, LUMBRIDGE_HANS_TYPE_ID).is_none());
    let mut npcs = snapshot.npcs().to_vec();
    npcs[1].r#type = Some(LUMBRIDGE_HANS_TYPE_ID);
    snapshot.seed_npcs(npcs);
    assert_eq!(
        nearest_talk_npc(&snapshot, LUMBRIDGE_HANS_TYPE_ID)
            .expect("only the named type is eligible")
            .index,
        2
    );
}

#[test]
fn direct_step_uses_loaded_collision_instead_of_the_npc_offset() {
    let from = WorldTile {
        x: 3220,
        z: 3220,
        level: 0,
    };
    let to = WorldTile {
        x: 3215,
        z: 3216,
        level: 0,
    };
    let mut scene = SceneView {
        available: true,
        base_x: 3200,
        base_z: 3200,
        level: 0,
        width: 64,
        height: 64,
        collision_flags: vec![0; 64 * 64],
    };
    let west = 19 * 64 + 20;
    // A blocked first-choice tile must not prevent a clear south step.
    scene.collision_flags[west] = i32::MAX;
    let target = direct_step_toward(&scene, from, to).unwrap();
    assert_eq!(target, WorldTile { z: 3219, ..from });
    assert!(SceneQuery::new(&scene, Some(from)).can_step(from, target));
    // A walkable tile behind a wall is not an adjacent reachable step either.
    scene.collision_flags[west] = client::dash3d::CollisionFlag::W_E;
    assert!(SceneQuery::new(&scene, Some(from)).walkable(WorldTile { x: 3219, ..from }));
    assert_eq!(direct_step_toward(&scene, from, to).unwrap(), target);

    // Missing or fully blocked collision must fail closed, not guess a tile.
    scene.collision_flags.fill(i32::MAX);
    assert!(direct_step_toward(&scene, from, to).is_err());
    scene.available = false;
    assert!(direct_step_toward(&scene, from, to).is_err());
}

#[test]
#[ignore = "requires LIVE=1, an owned BOT_LIVE_NAME_PREFIX, explicit 377 cache/snapshot/nav inputs, and a local 377 engine (WORLD_GAME_PORT/WORLD_HTTP_PORT override 43595/81)"]
fn live_377_attaches_walks_and_interacts_with_numeric_npc() -> Result<(), String> {
    run_live_interaction()
}
