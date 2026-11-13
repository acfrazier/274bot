//! Revision-377 constructor and bounded live interaction qualification.
//!
//! Both tests are ignored and require `LIVE=1` plus an owned `BOT_LIVE_NAME_PREFIX`.
//! They use the canonical `--revision 377` local profile with explicit paths
//! `ENGINE_DIR`, `CLIENT_377_CACHE_DIR`, `CLIENT_UNPACK_DIR`, `NAV_PACK`,
//! and `NAV_FLAGS`. `WORLD_GAME_PORT` and `WORLD_HTTP_PORT` may select another
//! local engine. Template preparation uses the production runtime-cache path.
//! Pre-Start mainland fixtures request a fresh login after observing logout:
//! idle slots park on a server logout, so the fixture posts Login through its
//! SlotArm once available rather than relying on automatic reconnect policy.
//!
//! Live invocation (with the real resource paths supplied by the owner):
//! ```text
//! ${ISOHOME:?} env \
//!   LIVE=1 BOT_LIVE_NAME_PREFIX=f377 ENGINE_DIR=<377-engine> \
//!   CLIENT_377_CACHE_DIR=<377-raw-cache> CLIENT_UNPACK_DIR=<377-decoded-snapshot> \
//!   NAV_PACK=<377-nav-pack> NAV_FLAGS=<377-nav-flags> \
//!   cargo test --locked -p host-play --features test-support --test revision_377_live \
//!   -- --ignored --nocapture --test-threads=1
//! ```

#[path = "revision_377_live/gatherer.rs"]
mod gatherer;

#[path = "revision_377_live/machines.rs"]
mod machines;

#[path = "revision_377_live/cook.rs"]
mod cook;

#[path = "revision_377_live/quester.rs"]
mod quester;

#[path = "revision_377_live/quester_imp.rs"]
mod quester_imp;

#[path = "revision_377_live/quester_doric.rs"]
mod quester_doric;
#[path = "revision_377_live/quester_journal.rs"]
mod quester_journal;

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

fn chebyshev(a: WorldTile, b: WorldTile) -> u32 {
    a.x.abs_diff(b.x).max(a.z.abs_diff(b.z))
}

#[derive(Clone, Copy)]
struct NpcChoice {
    type_id: usize,
    index: usize,
    tile: WorldTile,
    distance: u32,
}

fn nearest_talk_npc(snapshot: &GameSnapshot, wanted_type: Option<usize>) -> Option<NpcChoice> {
    let (x, z, level) = snapshot.tile()?;
    let player_tile = WorldTile { x, z, level };
    snapshot
        .npcs()
        .iter()
        .filter_map(|npc| {
            let type_id = npc.r#type?;
            if wanted_type.is_some_and(|wanted| wanted != type_id)
                || npc.tile.level != level
                || !npc
                    .actions
                    .iter()
                    .flatten()
                    .any(|action| action.eq_ignore_ascii_case(TALK_ACTION))
            {
                return None;
            }
            let distance = chebyshev(player_tile, npc.tile);
            (distance <= 8).then_some(NpcChoice {
                type_id,
                index: npc.index,
                tile: npc.tile,
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
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "nearby-numeric-npc",
            Duration::from_secs(20),
            |state| nearest_talk_npc(state, None).is_some(),
        )?;
        let choice = nearest_talk_npc(&snapshot, None)
            .ok_or("no nearby numeric NPC with Talk-to in Lumbridge")?;
        let target = direct_step_toward(snapshot.scene(), origin, choice.tile)?;
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
                "nearby_npc_type_id": choice.type_id,
                "nearby_npc_index": choice.index,
                "nearby_npc_tile": choice.tile,
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

        let (px, pz, level) = snapshot
            .tile()
            .ok_or("missing tile before NPC interaction")?;
        let npc = snapshot
            .npcs()
            .iter()
            .filter(|npc| {
                npc.r#type == Some(choice.type_id)
                    && npc.tile.level == level
                    && chebyshev(
                        WorldTile {
                            x: px,
                            z: pz,
                            level,
                        },
                        npc.tile,
                    ) <= 8
                    && npc
                        .actions
                        .iter()
                        .flatten()
                        .any(|action| action.eq_ignore_ascii_case(TALK_ACTION))
            })
            .min_by_key(|npc| (npc.distance, npc.index))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "NPC type {} is no longer nearby with {TALK_ACTION}",
                    choice.type_id
                )
            })?;
        if snapshot.modals().chat != -1 || !snapshot.chat_modal_texts().is_empty() {
            return Err("NPC dialogue was already open before the interaction".into());
        }
        record(
            "npc-talk-request",
            &snapshot,
            json!({
                "npc_type_id": npc.r#type,
                "npc_index": npc.index,
                "npc_name": npc.name,
                "npc_tile": npc.tile,
                "action": TALK_ACTION,
            }),
        );
        accepted(
            Interactions::new(&snapshot, &mut client)
                .interact(OpTarget::Npc(&npc), ActionSpec::Label(TALK_ACTION.into())),
            "numeric NPC Talk-to",
        )?;
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "npc-dialogue-applied",
            Duration::from_secs(30),
            |state| state.modals().chat != -1 && !state.chat_modal_texts().is_empty(),
        )?;
        record(
            "npc-dialogue-applied",
            &snapshot,
            json!({"npc_type_id":choice.type_id, "npc_index":npc.index}),
        );
        Ok(())
    })();
    if client.ingame {
        client.logout();
    }
    result
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
