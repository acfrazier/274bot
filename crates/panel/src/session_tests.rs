use super::{
    combo_index, debug_dest_cheats, debug_main_buttons_for, debug_maxme_cheats, is_local_engine,
    live_client_trail, live_or_walk_paint, load_live_example_card, nav_snapshot_for_follow,
    null_raster_live_entries_for_target, parse_getvar_line, publish_frontend_slot,
    publish_nav_debug, reset_frontend_slot_lifetime, script_active, script_pause_enabled,
    script_self_stop_observed, script_status_text, script_stop_enabled, seed_on_first_world,
    start_catalog_with_core, stress_live_entries_for_target, temp_live_vault_from, walkto_tele_cmd,
    ProfilePreparationCompletion, Session, SlotIo, WalkArm,
};
use crate::focus::draw_for_slot;
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::{Client, ClientConfig};
use client::config::if_type::{ComponentType, IfType, IfTypeMut};
use client::dash3d::CollisionFlag;
use client::io::{Packet, ServerProt};
use client::render::nav_debug::{CORNER_NE, FACE_N, FACE_S};
use host::{FrameBuf, SlotInput};
use host_play::profile::ProfileEnvironment;
use host_play::{ProfileOptions, SlotArm, SlotStatus, StartupPhase};
use nav::collision::WorldCollision;
use nav::paint::{MAX_DRAW_TILES, NEAR_FULL_DENSITY};
use nav::router::{Leg, Route};
use nav::tile::Tile;
use nav::transport::{TransportEdge, TransportGraph, TransportKind};
use nav::traveller::Traveller;
use nav::world::NavWorld;
use nav::WorldState;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use vault::{Profile, ProfileSettings, Vault};

#[test]
fn memory_override_changes_spawn_profile_without_persisting_it() {
    let profile = Profile {
        uid: 274,
        username: "alice".into(),
        password: "pw".into(),
        settings: ProfileSettings {
            lowmem: true,
            ..ProfileSettings::default()
        },
    };
    let effective = Session::profile_with_memory_override(profile.clone(), Some(false));
    assert!(
        !effective.settings.lowmem,
        "spawn must use explicit highmem"
    );
    assert!(
        profile.settings.lowmem,
        "the vault profile must remain unchanged"
    );
    assert!(
        Session::profile_with_memory_override(profile, None)
            .settings
            .lowmem
    );
}

use crate::nav_settings::{effective, NavSettings};
use script::IsolatedEnv;

#[test]
fn catalog_core_start_marker_precedes_actual_isolate_start() {
    let watch = host_play::catalog_core::CoreWatch::default();
    let mut baseline = host_play::catalog_core::Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some((2661, 3306, 0)),
        ..host_play::catalog_core::Observation::default()
    };
    baseline.levels.insert("thieving".into(), 50);
    baseline.levels.insert("hitpoints".into(), 50);
    baseline.effective_levels.insert("thieving".into(), 50);
    baseline.effective_levels.insert("hitpoints".into(), 50);
    baseline.items.insert("Lobster".into(), 10);
    watch.configure(host_play::catalog_core::CoreCase::Thiever, "catalogtest");
    watch.observe("catalogtest", baseline, false);

    start_catalog_with_core(&watch, "catalogtest", || {
        assert!(
            watch
                .qualify()
                .unwrap_err()
                .contains("no post-Start observations"),
            "the core baseline must be frozen before isolate Start"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn catalog_core_initial_login_then_preparation_then_start_uses_current_session() {
    let watch = host_play::catalog_core::CoreWatch::default();
    watch.configure(host_play::catalog_core::CoreCase::Thiever, "catalogtest");
    watch.observe(
        "catalogtest",
        host_play::catalog_core::Observation::default(),
        true,
    );

    let mut prepared = host_play::catalog_core::Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some((2661, 3306, 0)),
        ..host_play::catalog_core::Observation::default()
    };
    prepared.levels.insert("thieving".into(), 50);
    prepared.levels.insert("hitpoints".into(), 50);
    prepared.effective_levels.insert("thieving".into(), 50);
    prepared.effective_levels.insert("hitpoints".into(), 50);
    prepared.items.insert("Lobster".into(), 10);
    watch.observe("catalogtest", prepared, false);

    let mut started = false;
    start_catalog_with_core(&watch, "catalogtest", || {
        started = true;
        Ok(())
    })
    .unwrap();
    assert!(started);
    assert!(watch
        .qualify()
        .unwrap_err()
        .contains("no post-Start observations"));
}

fn checked_profile_fixture(revision: u16) -> (PathBuf, PathBuf, PathBuf) {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../host-play/tests/fixtures/profile");
    let root = std::env::temp_dir().join(format!(
        "274bot-panel-session-profile-{revision}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let cache = root.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    for jag in [
        "title",
        "config",
        "interface",
        "media",
        "versionlist",
        "textures",
        "wordenc",
        "sounds",
    ] {
        std::fs::copy(fixture.join(jag), cache.join(jag)).unwrap();
    }
    let manifest = fixture.join(format!("manifest-{revision}.json"));
    (root, cache, manifest)
}

const JAG_SLOTS: [&str; 8] = [
    "title",
    "config",
    "interface",
    "media",
    "versionlist",
    "textures",
    "wordenc",
    "sounds",
];

fn identity_packs() -> Vec<(String, Vec<u8>)> {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../host-play/tests/fixtures/profile");
    let mut tables: Vec<(String, Vec<u8>)> = Vec::new();
    for prefix in ["model", "anim", "midi", "map"] {
        tables.push((format!("{prefix}_version"), vec![0, 1]));
        tables.push((format!("{prefix}_crc"), 1u32.to_be_bytes().to_vec()));
        tables.push((format!("{prefix}_index"), vec![0]));
    }
    let refs: Vec<(&str, &[u8])> = tables
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    let versionlist = client::io::synthetic_jag(&refs);
    JAG_SLOTS
        .iter()
        .map(|name| {
            let bytes = if *name == "versionlist" {
                versionlist.clone()
            } else if *name == "config" || *name == "interface" {
                std::fs::read(fixture.join(name)).unwrap()
            } else {
                client::io::synthetic_jag(&[("data", b"content".as_slice())])
            };
            ((*name).to_string(), bytes)
        })
        .collect()
}
fn crc_body(packs: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut checksums = [0i32; 9];
    for (name, bytes) in packs {
        let slot = JAG_SLOTS
            .iter()
            .position(|candidate| candidate == name)
            .unwrap()
            + 1;
        checksums[slot] = Packet::getcrc(bytes, 0, bytes.len());
    }
    let mut body = Packet::alloc(0);
    for &checksum in &checksums {
        body.p4(checksum);
    }
    let mut hash = 1234i32;
    for &checksum in &checksums {
        hash = hash.wrapping_shl(1).wrapping_add(checksum);
    }
    body.p4(hash);
    body.data()[..body.pos].to_vec()
}

fn plant_snapshot(unpack: &Path, packs: &[(String, Vec<u8>)]) {
    let versionlist = &packs
        .iter()
        .find(|(name, _)| name == "versionlist")
        .unwrap()
        .1;
    let version = client::unpack::version_hash(versionlist);
    let dir = unpack.join(&version);
    std::fs::create_dir_all(&dir).unwrap();
    let mut manifest = format!(
        "version={version}\ndir={}\nsource=update-server\ncomplete=1\n",
        dir.display()
    );
    for (name, bytes) in packs {
        std::fs::write(dir.join(name), bytes).unwrap();
        manifest += &format!("jag.{name}.bytes={}\n", bytes.len());
    }
    for name in ["models", "anims", "midi", "maps"] {
        let mut bin = 0u32.to_le_bytes().to_vec();
        bin.extend_from_slice(&4u32.to_le_bytes());
        bin.extend_from_slice(b"body");
        std::fs::write(dir.join(format!("{name}.bin")), &bin).unwrap();
        manifest += &format!(
            "{name}.total=1\n{name}.unpacked=1\n{name}.skipped=0\n{name}.bytes={}\n",
            bin.len()
        );
    }
    std::fs::write(dir.join("manifest"), manifest).unwrap();
}

/// Mock update server: `/crc` matching the fixture packs, plus pack GETs if
/// a runtime refresh asks. Detached so unit tests never need a live engine.
fn serve_fixture_crc(packs: Vec<(String, Vec<u8>)>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let body = crc_body(&packs);
        while Instant::now() < deadline {
            let (mut sock, _) = match listener.accept() {
                Ok(conn) => conn,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(_) => break,
            };
            let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match sock.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
            }
            let path = String::from_utf8_lossy(&request)
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_string();
            let payload = if path == "/crc" {
                body.clone()
            } else {
                packs
                    .iter()
                    .find(|(name, _)| path.starts_with(&format!("/{name}")))
                    .map(|(_, bytes)| bytes.clone())
                    .unwrap_or_default()
            };
            let response = [
                b"HTTP/1.0 200 OK\r\nContent-Length: ".as_slice(),
                payload.len().to_string().as_bytes(),
                b"\r\n\r\n",
                &payload,
            ]
            .concat();
            let _ = sock.write_all(&response);
        }
    });
    port
}

fn runtime_profile_fixture(revision: u16) -> (PathBuf, PathBuf, PathBuf, PathBuf, u16) {
    let (root, cache, _) = checked_profile_fixture(revision);
    let packs = identity_packs();
    for (name, bytes) in &packs {
        std::fs::write(cache.join(name), bytes).unwrap();
    }
    let manifest_path = root.join("cache-manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec(&host_play::profile::CacheManifest::capture(revision, &cache).unwrap())
            .unwrap(),
    )
    .unwrap();
    let unpack = root.join("unpack");
    plant_snapshot(&unpack, &packs);
    let port = serve_fixture_crc(packs);
    (root, cache, manifest_path, unpack, port)
}

#[test]
fn explicit_profile_wins_saved_revision_and_bound_session_refuses_changes() {
    let (root, cache, manifest, unpack, port) = runtime_profile_fixture(274);
    let mut session = Session::new();
    session.ui.server_revision = 289;
    session
        .configure_profile(ProfileOptions {
            profile: Some("local-274".into()),
            cache_dir: Some(cache),
            cache_manifest: Some(manifest),
            unpack_dir: Some(unpack),
            http_port: Some(port),
            ..ProfileOptions::default()
        })
        .unwrap();
    let env = ProfileEnvironment {
        home: Some(root.clone()),
        working_dir: Some(root.clone()),
        rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
        rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
        ..ProfileEnvironment::default()
    };
    session.bind_profile_with_env(&env).unwrap();
    assert!(session.server_label().starts_with("local-274"));
    assert_eq!(session.catalog_root().unwrap(), None);
    assert!(session.set_server_revision(289).is_err());
    assert!(session.bind_profile_with_env(&env).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_profile_preparation_is_dropped_without_partial_session_state() {
    let (root, cache, manifest, unpack, port) = runtime_profile_fixture(274);
    let mut session = Session::new();
    session
        .configure_profile(ProfileOptions {
            profile: Some("local-274".into()),
            cache_dir: Some(cache),
            cache_manifest: Some(manifest),
            unpack_dir: Some(unpack),
            http_port: Some(port),
            ..ProfileOptions::default()
        })
        .unwrap();
    session.profile_environment = Some(ProfileEnvironment {
        home: Some(root.clone()),
        working_dir: Some(root.clone()),
        rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
        rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
        ..ProfileEnvironment::default()
    });
    let (generation, preparation) = session.profile_preparation().unwrap();
    let template = preparation.run().unwrap();

    session
        .configure_profile(ProfileOptions {
            profile: Some("local-289".into()),
            ..ProfileOptions::default()
        })
        .unwrap();
    assert_eq!(
        session.finish_profile_preparation(generation, Ok(template)),
        ProfilePreparationCompletion::Stale
    );
    assert!(!session.profile_bound());
    assert!(session.template.is_none());
    assert!(session.play.is_none());
    assert!(session.vault.is_none());
    assert!(session.slots.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn profile_preparation_failure_keeps_vault_play_and_slots_absent() {
    let mut session = Session::new();
    session
        .configure_profile(ProfileOptions::default())
        .unwrap();
    let generation = session.profile_generation();

    assert_eq!(
        session.finish_profile_preparation(generation, Err("cache changed".into())),
        ProfilePreparationCompletion::Failed
    );
    assert_eq!(session.error.as_deref(), Some("cache changed"));
    assert!(!session.profile_bound());
    assert!(session.template.is_none());
    assert!(session.play.is_none());
    assert!(session.vault.is_none());
    assert!(session.slots.is_empty());
}

#[test]
fn validated_profile_unlock_uses_the_ticket_without_rebinding() {
    let (root, cache, manifest, unpack, port) = runtime_profile_fixture(274);
    let mut session = Session::new();
    session
        .configure_profile(ProfileOptions {
            profile: Some("local-274".into()),
            cache_dir: Some(cache.clone()),
            cache_manifest: Some(manifest),
            unpack_dir: Some(unpack),
            http_port: Some(port),
            vault_path: Some(root.join("prepared.vault")),
            ..ProfileOptions::default()
        })
        .unwrap();
    session.profile_environment = Some(ProfileEnvironment {
        home: Some(root.clone()),
        working_dir: Some(root.clone()),
        rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
        rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
        ..ProfileEnvironment::default()
    });
    let (generation, preparation) = session.profile_preparation().unwrap();
    let template = preparation.run().unwrap();
    assert_eq!(
        session.finish_profile_preparation(generation, Ok(Arc::clone(&template))),
        ProfilePreparationCompletion::Installed
    );
    session
        .install_validated_template(template.validate_for_play().unwrap())
        .unwrap();

    std::fs::write(cache.join("config"), b"changed after validation").unwrap();
    assert!(session.unlock("prepared-pass"));
    assert!(session.profile_bound());
    assert!(session.play.is_some());
    assert!(session.vault.is_some());
    assert!(session.slots.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn prod_default_ignores_the_saved_local_revision() {
    let mut session = Session::new();
    session.ui.server_revision = 274;
    session
        .configure_profile(ProfileOptions {
            prod: true,
            ..ProfileOptions::default()
        })
        .unwrap();
    let home = std::env::temp_dir().join(format!("274bot-panel-public-{}", std::process::id()));
    session.profile_environment = Some(ProfileEnvironment {
        home: Some(home.clone()),
        ..ProfileEnvironment::default()
    });
    let selection = session.resolve_profile().unwrap();
    assert_eq!(selection.revision(), client::io::ClientRevision::R289);
    assert_eq!(selection.target(), client::BotTarget::Prod);
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn invalid_profile_refuses_vault_reset_without_deleting_explicit_path() {
    let root =
        std::env::temp_dir().join(format!("274bot-panel-invalid-reset-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let intended = root.join("intended.vault");
    std::fs::write(&intended, b"do not delete").unwrap();

    let mut session = Session::new();
    session
        .configure_profile(ProfileOptions {
            revision: Some("275".into()),
            vault_path: Some(intended.clone()),
            ..ProfileOptions::default()
        })
        .unwrap();
    assert!(!session.reset_vault());
    assert!(intended.is_file());
    assert!(session
        .error
        .as_deref()
        .is_some_and(|error| error.contains("unsupported revision")));
    std::fs::remove_dir_all(root).unwrap();
}

/// `register_name` is the ScriptRegistry name; `folder` is the class
/// file (`{folder}/{folder}.ts`) and the JS import binding.
fn write_looping_catalog(dir: &Path, cards: &[(&str, &str)]) -> PathBuf {
    let root = dir.join("rs2b0t");
    let scripts = root.join("src/bot/scripts");
    let mut index = String::new();
    for (register, folder) in cards {
        std::fs::create_dir_all(scripts.join(folder)).unwrap();
        index.push_str(&format!("import {folder} from './{folder}/{folder}.js';\n"));
        index.push_str(&format!(
            "ScriptRegistry.register({{ name: '{register}', create: () => new {folder}() }});\n"
        ));
        std::fs::write(
            scripts.join(folder).join(format!("{folder}.ts")),
            format!("export default class {folder} extends LoopingBot {{ override loop() {{}} }}"),
        )
        .unwrap();
    }
    std::fs::write(scripts.join("index.ts"), index).unwrap();
    root
}

#[test]
fn explicit_catalog_default_allows_manual_import_and_preserves_custom_cards() {
    let iso = IsolatedEnv::enter("prebind-catalog");
    let explicit = write_looping_catalog(&iso.dir.join("explicit"), &[("Chosen", "Chosen")]);
    let ambient = write_looping_catalog(&iso.dir.join("ambient"), &[("Ambient", "Ambient")]);
    iso.set_rs2b0t(&ambient);

    let mut session = Session::new();
    session.persist_ui = false;
    session
        .configure_profile(ProfileOptions {
            profile: Some("local-274".into()),
            catalog_root: Some(explicit.clone()),
            ..ProfileOptions::default()
        })
        .unwrap();
    assert_eq!(session.catalog_root().unwrap(), Some(explicit.clone()));
    session.fill_rs2b0t_cards_once();
    assert!(session
        .js
        .get(script::ScriptSource::Catalog, "Chosen")
        .is_some());
    assert!(session
        .js
        .get(script::ScriptSource::Catalog, "Ambient")
        .is_none());

    let custom = iso.dir.join("custom.ts");
    std::fs::write(
        &custom,
        "export default class Custom extends LoopingBot { override loop() {} }",
    )
    .unwrap();
    session.js.load(&custom).unwrap();
    session.import_rs2b0t_catalog(&ambient).unwrap();
    assert!(session
        .js
        .get(script::ScriptSource::File, "custom")
        .is_some());
    assert!(session
        .js
        .get(script::ScriptSource::Catalog, "Ambient")
        .is_some());
}

#[test]
fn revision_and_binding_allow_already_loaded_scripts_and_source_edits() {
    let (root, cache, manifest, unpack, port) = runtime_profile_fixture(274);
    let catalog = write_looping_catalog(&root.join("catalog"), &[("Chosen", "Chosen")]);
    let source = catalog.join("src/bot/scripts/Chosen/Chosen.ts");
    let mut session = Session::new();
    session.persist_ui = false;
    session
        .configure_profile(ProfileOptions {
            revision: None,
            cache_dir: Some(cache),
            cache_manifest: Some(manifest),
            unpack_dir: Some(unpack),
            http_port: Some(port),
            catalog_root: Some(catalog),
            ..ProfileOptions::default()
        })
        .unwrap();
    session.fill_rs2b0t_cards_once();
    session
        .js
        .ensure_js(script::ScriptSource::Catalog, "Chosen")
        .unwrap();
    std::fs::write(
        source,
        "export default class Chosen extends LoopingBot { override loop() { this.walk(); } }",
    )
    .unwrap();
    let env = ProfileEnvironment {
        home: Some(root.clone()),
        working_dir: Some(root.clone()),
        rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
        rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
        ..ProfileEnvironment::default()
    };
    session.set_server_revision(289).unwrap();
    assert!(session
        .js
        .get(script::ScriptSource::Catalog, "Chosen")
        .is_some());
    session.set_server_revision(274).unwrap();
    session.bind_profile_with_env(&env).unwrap();
    let another = write_looping_catalog(&root.join("another"), &[("Another", "Another")]);
    session.import_rs2b0t_catalog(&another).unwrap();
    assert!(session
        .js
        .get(script::ScriptSource::Catalog, "Another")
        .is_some());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn live_follow_route_wins_over_empty_walkto_arm() {
    let dest = WorldTile {
        x: 3220,
        z: 3220,
        level: 0,
    };
    let live_route = Route {
        legs: vec![Leg::Walk { tiles: vec![dest] }],
        dest,
        ticks: 1.0,
    };
    let (route, click) = live_or_walk_paint(
        true,
        (Some(live_route.clone()), Some(dest)),
        (None, None),
        (None, None),
    );
    assert_eq!(route.unwrap().dest, dest);
    assert_eq!(click, Some(dest));
    let (route, _) = live_or_walk_paint(true, (None, None), (None, None), (None, None));
    assert!(route.is_none(), "no live route falls back to WalkTo");
    let script_dest = WorldTile {
        x: 3185,
        z: 3440,
        level: 0,
    };
    let script_route = Route {
        legs: vec![Leg::Walk {
            tiles: vec![script_dest],
        }],
        dest: script_dest,
        ticks: 1.0,
    };
    let (route, click) = live_or_walk_paint(
        false,
        (None, None),
        (None, None),
        (Some(script_route), Some(script_dest)),
    );
    assert_eq!(
        click,
        Some(script_dest),
        "catalog walk paints when WalkTo is idle"
    );
    assert_eq!(route.unwrap().dest, script_dest);
}

#[test]
fn is_local_engine_is_loopback_only() {
    assert!(is_local_engine("127.0.0.1"));
    assert!(is_local_engine("localhost"));
    assert!(is_local_engine("::1"));
    assert!(!is_local_engine("w1.rs2b2t.com"));
    assert!(!is_local_engine("192.168.1.5"));
}

#[test]
fn debug_dest_lumbridge_sends_home() {
    let d = debug_dest_cheats();
    assert!(d
        .iter()
        .any(|x| x.label == "Lumbridge" && x.cheat == "~home"));
    assert!(d.iter().any(|x| x.label == "Seers" && x.cheat == "~seers"));
    assert!(!d.iter().any(|x| x.label == "North"));
}

#[test]
fn debug_dest_greenland_tooltip_is_the_script_comment() {
    let g = debug_dest_cheats()
        .iter()
        .find(|x| x.label == "Greenland")
        .expect("greenland dest");
    assert_eq!(g.cheat, "~greenland");
    assert!(
        g.tooltip.contains("Gnome Stronghold"),
        "hover must say where this is, got {:?}",
        g.tooltip
    );
}

#[test]
fn walkto_tele_cmd_is_engine_tele_args() {
    let t = Tile {
        x: 3253,
        z: 3266,
        level: 0,
    };
    assert_eq!(walkto_tele_cmd(t), "tele 0,50,51,53,2");
}

#[test]
fn mark_tutorial_skipped_persists_on_focused_profile() {
    let path = tmp_vault("tutskip-pref.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.focus.lock().unwrap().focused = Some("alice".into());
    assert_eq!(s.focused_tutorial_skipped(), None);
    s.mark_tutorial_skipped();
    assert_eq!(s.focused_tutorial_skipped(), Some(true));
    assert_eq!(
        s.vault
            .as_ref()
            .unwrap()
            .get("alice")
            .unwrap()
            .settings
            .tutorial_skipped,
        Some(true)
    );
}

#[test]
fn parse_getvar_line_reads_engine_reply() {
    assert_eq!(
        parse_getvar_line("get tutorial: 1000"),
        Some(("tutorial", 1000))
    );
    assert_eq!(parse_getvar_line("get tutorial: 0"), Some(("tutorial", 0)));
    assert_eq!(parse_getvar_line("hello"), None);
}

#[test]
fn tutskip_button_omitted_until_known_open() {
    assert_eq!(
        debug_main_buttons_for(client::BotTarget::Local, false),
        ["DebugPanel", "Lumbridge", "maxme", "Teles"]
    );
    assert_eq!(
        debug_main_buttons_for(client::BotTarget::Local, true),
        ["DebugPanel", "TutSkip", "Lumbridge", "maxme", "Teles"]
    );
}

#[test]
fn debug_main_buttons_prod_is_debug_panel_only() {
    assert_eq!(
        debug_main_buttons_for(client::BotTarget::Prod, true),
        ["DebugPanel"]
    );
    assert_eq!(
        debug_main_buttons_for(client::BotTarget::Prod, false),
        ["DebugPanel"]
    );
}

#[test]
fn debug_maxme_is_setstat_99_not_maxme_proc() {
    let cmds = debug_maxme_cheats();
    assert!(!cmds.contains(&"maxme"));
    assert!(cmds.contains(&"setstat attack 99"));
    assert_eq!(cmds.len(), 19);
}

fn empty_play() -> host_play::Play {
    host_play::run_with_io(
        &host_play::PlayOptions {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    )
}

fn status(name: &str, ingame: bool, scene: i32) -> SlotStatus {
    SlotStatus {
        username: name.into(),
        ingame,
        scene_state: scene,
        ..SlotStatus::default()
    }
}

/// A `w`×`h` all-walkable level-0 world at (0,0).
fn open_world(w: usize, h: usize) -> NavWorld {
    NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: w,
            height: h,
            walk: vec![0u8; w * h],
            blocked: vec![0u64; (w * h).div_ceil(64)],
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    )
}

/// The Rune Essence mine mapsquare (m45_75) as a sealed 64×64
/// all-walkable bake at (2880,4800): the pad and the four exit portal
/// placements inside, nothing packed — the session return hop is
/// synthesized by the router, so a walk out only arms with a latch.
fn mine_world() -> NavWorld {
    NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 2880,
                z: 4800,
                level: 0,
            },
            width: 64,
            height: 64,
            walk: vec![0u8; 64 * 64],
            blocked: vec![0u64; (64usize * 64).div_ceil(64)],
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    )
}

/// A synthetic offline client with a fake mainland scene base (same
/// trick as the app tests — no live server, no network).
fn paint_client() -> client::client::Client {
    let mut c = host::prepare_client(
        client::client::ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: true,
        },
        1,
        Arc::new(client::config::Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c
}

fn response_15_reconnect(c: &mut Client) {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    c.config.port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut header = [0; 2];
        stream.read_exact(&mut header).unwrap();
        assert_eq!(header[0], 14);
        stream.write_all(&[0; 17]).unwrap();
        stream.read_exact(&mut header).unwrap();
        assert_eq!(header[0], 18);
        let mut login = vec![0; header[1] as usize];
        stream.read_exact(&mut login).unwrap();
        stream.write_all(&[15]).unwrap();
    });
    c.login("snapshot", "test", true).unwrap();
    server.join().unwrap();
}

type FrontendFixture = (
    Arc<Mutex<std::collections::HashMap<String, client::client::ClientGens>>>,
    Arc<Mutex<std::collections::HashMap<String, (GameSnapshot, WorldState)>>>,
    super::SlotTravellers,
    Arc<Mutex<std::collections::HashMap<String, (u64, Tile)>>>,
);

fn frontend_fixture() -> FrontendFixture {
    (
        Arc::new(Mutex::new(std::collections::HashMap::new())),
        Arc::new(Mutex::new(std::collections::HashMap::new())),
        Arc::new(Mutex::new(std::collections::HashMap::new())),
        Arc::new(Mutex::new(std::collections::HashMap::new())),
    )
}

#[test]
fn frontend_logout_clears_facts_armed_work_and_tick_latch() {
    let (gens, states, travellers, latch) = frontend_fixture();
    let mut c = paint_client();
    c.ingame = true;
    c.scene_state = 2;
    c.local_player = Some(client::client::ClientPlayer::at(5, 6));
    c.bump_gens(ServerProt::PLAYER_INFO);
    assert!(!publish_frontend_slot(
        "alice",
        &c,
        &gens,
        &states,
        &travellers,
        &latch
    ));
    travellers
        .lock()
        .unwrap()
        .insert("alice".into(), Arc::new(Mutex::new(WalkArm::default())));
    latch.lock().unwrap().insert(
        "alice".into(),
        (
            c.gens.player,
            Tile {
                x: 3205,
                z: 3206,
                level: 0,
            },
        ),
    );

    c.logout();
    assert!(publish_frontend_slot(
        "alice",
        &c,
        &gens,
        &states,
        &travellers,
        &latch
    ));
    let states = states.lock().unwrap();
    assert!(!states["alice"].0.ingame());
    assert!(states["alice"].0.local_player().is_none());
    drop(states);
    assert!(!travellers.lock().unwrap().contains_key("alice"));
    assert!(!latch.lock().unwrap().contains_key("alice"));
}

#[test]
fn frontend_response_15_replacement_waits_for_post_grant_player_packet() {
    let (gens, states, travellers, latch) = frontend_fixture();
    let mut c = paint_client();
    c.ingame = true;
    c.scene_state = 2;
    c.local_player = Some(client::client::ClientPlayer::at(5, 6));
    c.bump_gens(ServerProt::PLAYER_INFO);
    publish_frontend_slot("alice", &c, &gens, &states, &travellers, &latch);
    travellers
        .lock()
        .unwrap()
        .insert("alice".into(), Arc::new(Mutex::new(WalkArm::default())));

    response_15_reconnect(&mut c);
    assert!(publish_frontend_slot(
        "alice",
        &c,
        &gens,
        &states,
        &travellers,
        &latch
    ));
    assert!(states.lock().unwrap()["alice"].0.local_player().is_none());
    assert!(!travellers.lock().unwrap().contains_key("alice"));

    let mut player = client::io::Packet::new(vec![0xe0, 0x50, 0xc0, 0]);
    c.psize = 4;
    c.handle_packet(ServerProt::PLAYER_INFO, &mut player);
    assert!(!publish_frontend_slot(
        "alice",
        &c,
        &gens,
        &states,
        &travellers,
        &latch
    ));
    assert!(states.lock().unwrap()["alice"].0.local_player().is_some());
}

#[test]
fn same_name_lifetime_resets_but_scene_change_and_guardian_hold_preserve_work() {
    let (gens, states, travellers, latch) = frontend_fixture();
    let mut c = paint_client();
    c.ingame = true;
    c.scene_state = 2;
    c.local_player = Some(client::client::ClientPlayer::at(5, 6));
    c.bump_gens(ServerProt::PLAYER_INFO);
    publish_frontend_slot("alice", &c, &gens, &states, &travellers, &latch);
    travellers
        .lock()
        .unwrap()
        .insert("alice".into(), Arc::new(Mutex::new(WalkArm::default())));
    latch.lock().unwrap().insert(
        "alice".into(),
        (
            c.gens.player,
            Tile {
                x: 1,
                z: 1,
                level: 0,
            },
        ),
    );

    c.scene_state = 1;
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    assert!(!publish_frontend_slot(
        "alice",
        &c,
        &gens,
        &states,
        &travellers,
        &latch
    ));
    assert!(!WalkArm::may_follow(true));
    assert!(travellers.lock().unwrap().contains_key("alice"));
    assert!(latch.lock().unwrap().contains_key("alice"));

    assert!(reset_frontend_slot_lifetime(
        "alice",
        &gens,
        &states,
        &travellers,
        &latch
    ));
    assert!(!gens.lock().unwrap().contains_key("alice"));
    assert!(!states.lock().unwrap().contains_key("alice"));
    assert!(!travellers.lock().unwrap().contains_key("alice"));
    assert!(!latch.lock().unwrap().contains_key("alice"));

    let fresh = paint_client();
    publish_frontend_slot("alice", &fresh, &gens, &states, &travellers, &latch);
    assert!(!travellers.lock().unwrap().contains_key("alice"));
}

/// A 64×64 level-0 world at (3200, 3200) with a face wall and a
/// WR_GRND ground block inside the scene region.
fn walled_world() -> NavWorld {
    let width = 64;
    let height = 64;
    let mut flags = vec![0u32; width * height];
    flags[width + 1] = CollisionFlag::W_N as u32 | CollisionFlag::W_S as u32;
    flags[2 * width + 2] = CollisionFlag::WR_GRND as u32;
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            width,
            height,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    )
}

#[test]
fn focused_slot_publishes_scene_collision_for_loaded_map() {
    let mut c = paint_client();
    let world = walled_world();
    // Collision + NSEW on so face bits publish (the live overlay no
    // longer forces NSEW).
    let layers = NavSettings {
        collision_fill: true,
        nsew_labels: true,
        show_nav_path: true,
        hop_labels: true,
        ..NavSettings::default()
    };
    let route = Route {
        legs: vec![Leg::Walk {
            tiles: vec![
                WorldTile {
                    x: 3200,
                    z: 3200,
                    level: 0,
                },
                WorldTile {
                    x: 3201,
                    z: 3200,
                    level: 0,
                },
                WorldTile {
                    x: 3202,
                    z: 3200,
                    level: 0,
                },
            ],
        }],
        dest: WorldTile {
            x: 3202,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let here = Some(WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    });
    // The traveller's current walk aim, world tile → scene (52, 52).
    let click = Some(WorldTile {
        x: 3252,
        z: 3252,
        level: 0,
    });
    publish_nav_debug(
        &mut c,
        &world,
        Some(&route),
        here,
        &[],
        false,
        click,
        &layers,
        true,
    );
    let paint = c.nav_debug_paint().expect("focused drawing slot publishes");
    assert!(
        !paint.collision.is_empty(),
        "the loaded scene must include the walled tiles"
    );
    assert!(
        paint
            .collision
            .iter()
            .all(|cell| (0..104).contains(&cell.lx) && (0..104).contains(&cell.lz)),
        "collision cells are scene tiles inside the loaded 104×104 map"
    );
    assert!(
        paint
            .collision
            .iter()
            .all(|cell| cell.lx < 64 && cell.lz < 64),
        "tiles outside the pack bake must not paint a phantom wall"
    );
    let nsew = paint
        .collision
        .iter()
        .find(|cell| cell.lx == 1 && cell.lz == 1);
    assert!(
        nsew.is_some_and(|cell| cell.bits & FACE_N != 0 && cell.bits & FACE_S != 0),
        "the W_N/W_S tile must pack the N and S face bits"
    );
    assert!(
        paint
            .collision
            .iter()
            .any(|cell| cell.lx == 2 && cell.lz == 2 && cell.bits == 0),
        "the WR_GRND tile blocks ground with no face bits"
    );
    // Path: world tiles convert to scene tiles (client clips).
    assert_eq!(
        paint.path,
        vec![(0, 0, false), (1, 0, false), (2, 0, false)],
        "remaining path tiles convert to scene lx,lz"
    );
    // Click: the traveller's walk aim converts to scene coords.
    assert_eq!(paint.click, Some((52, 52)));
    assert!(paint.show_collision && paint.show_nsew && paint.show_path);
}

#[test]
fn publish_nav_debug_carries_reach_from_the_bitset() {
    let mut c = paint_client();
    // A 65×65 world (a distinct grid from `walled_world`, so the cached
    // reach bake belongs to this graph): a sealed 1×1 courtyard floor
    // at scene (1,1) and a blocked door-loc seed at scene (2,2).
    let (width, height) = (65usize, 65usize);
    let mut flags = vec![0u32; width * height];
    flags[width + 1] = CollisionFlag::WALK_BLOCK_FLAGS as u32;
    flags[2 * width + 2] = CollisionFlag::WR_GRND as u32;
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    let world = NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            width,
            height,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph {
            edges: vec![TransportEdge {
                kind: TransportKind::Door,
                at: WorldTile {
                    x: 3202,
                    z: 3202,
                    level: 0,
                },
                to: WorldTile {
                    x: 3200,
                    z: 3200,
                    level: 0,
                },
                loc_id: 1530,
                option: 1,
                ticks: 1,
                dir: None,
                open_loc_id: None,
                skill_req: vec![],
                item_req: vec![],
                quest_req: vec![],
                varp_req: vec![],
                worn_req: vec![],
                members_req: false,
            }],
            ..Default::default()
        },
        Vec::new(),
    );
    let layers = NavSettings {
        collision_fill: true,
        nsew_labels: true,
        ..NavSettings::default()
    };
    publish_nav_debug(&mut c, &world, None, None, &[], false, None, &layers, true);
    let paint = c.nav_debug_paint().expect("focused drawing slot publishes");
    // The blocked door loc is a reach seed: reached despite the ground
    // block, so the client keeps its collision fill.
    let door = paint
        .collision
        .iter()
        .find(|cell| cell.lx == 2 && cell.lz == 2)
        .expect("the door loc cell");
    assert!(
        door.blocked && door.reach,
        "a transport seed is reached even when blocked"
    );
    // The sealed courtyard floor is standable but the network never
    // reaches it — the client draws the unreached fill there.
    let yard = paint
        .collision
        .iter()
        .find(|cell| cell.lx == 1 && cell.lz == 1)
        .expect("the courtyard cell");
    assert!(
        !yard.blocked && !yard.reach,
        "the walled floor is an unreached puddle"
    );
    // Open reached ground has no bits and is not blocked: no cell.
    assert!(
        !paint
            .collision
            .iter()
            .any(|cell| cell.lx == 0 && cell.lz == 0),
        "reached open ground publishes nothing"
    );
}

#[test]
fn unfocused_slot_clears_nav_debug_paint() {
    let mut c = paint_client();
    let world = walled_world();
    let layers = effective(&NavSettings::default(), true);
    publish_nav_debug(&mut c, &world, None, None, &[], false, None, &layers, true);
    assert!(c.nav_debug_paint().is_some());
    // Unfocused / skip-paint / renderer-off slots must not linger on a
    // stale paint.
    publish_nav_debug(&mut c, &world, None, None, &[], false, None, &layers, false);
    assert!(
        c.nav_debug_paint().is_none(),
        "a non-drawing slot stores None"
    );
}

#[test]
fn focused_slot_publishes_client_trail_tones() {
    let mut c = paint_client();
    let world = walled_world();
    let layers = NavSettings {
        show_nav_path: true,
        client_trail: true,
        ..NavSettings::default()
    };
    // The local player's last tryMove path, world tiles (the route
    // buffer minus the base), run on.
    let trail_world = vec![
        WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        },
        WorldTile {
            x: 3201,
            z: 3200,
            level: 0,
        },
        WorldTile {
            x: 3202,
            z: 3200,
            level: 0,
        },
    ];
    publish_nav_debug(
        &mut c,
        &world,
        None,
        None,
        &trail_world,
        true,
        None,
        &layers,
        true,
    );
    let paint = c.nav_debug_paint().expect("focused drawing slot publishes");
    assert_eq!(
        paint.trail,
        vec![(0, 0, false), (1, 0, true), (2, 0, false)],
        "run-on trail alternates Primary / RunAlt in scene coords"
    );
    assert!(paint.show_trail);
}

fn wt(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

#[test]
fn live_client_trail_retires_after_arrival_so_offpath_cannot_resurrect() {
    let mut c = paint_client();
    let world = walled_world();
    let layers = NavSettings {
        collision_fill: true,
        nsew_labels: true,
        show_nav_path: true,
        client_trail: true,
        ..NavSettings::default()
    };
    // Producer: last successful tryMove BFS, scene tiles src→dest.
    c.try_move_path = vec![(0, 0), (1, 0), (2, 0)];

    let mid = live_client_trail(&mut c, Some(wt(3201, 3200)));
    assert_eq!(mid, vec![wt(3201, 3200), wt(3202, 3200)]);
    assert_eq!(c.try_move_path.len(), 3, "mid-path must keep the producer");

    let arrived = live_client_trail(&mut c, Some(wt(3202, 3200)));
    assert!(arrived.is_empty(), "dest occupancy must hide the trail");
    publish_nav_debug(
        &mut c,
        &world,
        None,
        Some(wt(3202, 3200)),
        &arrived,
        false,
        None,
        &layers,
        true,
    );
    let paint = c.nav_debug_paint().expect("arrival still publishes");
    assert!(
        paint.trail.is_empty(),
        "arrived dest must not paint under the player"
    );
    assert!(
        !paint.collision.is_empty(),
        "retiring the trail must not drop the collision layer"
    );
    assert!(
        c.try_move_path.is_empty(),
        "arrival must retire the producer, not only the paint trim"
    );

    // Fire-lane shape: dest, then a west/off-path step, then a revisit
    // of an old BFS tile. Without producer clear, remaining_trail would
    // trim from that tile and republish the cyan/yellow trail.
    let off = live_client_trail(&mut c, Some(wt(3203, 3200)));
    assert!(off.is_empty());
    let revisit = live_client_trail(&mut c, Some(wt(3201, 3200)));
    assert!(
        revisit.is_empty(),
        "revisit of a retired click must not resurrect the trail"
    );
    assert!(c.try_move_path.is_empty());
    publish_nav_debug(
        &mut c,
        &world,
        None,
        Some(wt(3201, 3200)),
        &revisit,
        false,
        None,
        &layers,
        true,
    );
    let paint = c.nav_debug_paint().expect("off-path still publishes");
    assert!(
        paint.trail.is_empty(),
        "publisher must not resurrect a retired trail"
    );
    assert!(
        !paint.collision.is_empty(),
        "collision paint remains after trail retirement"
    );
}

#[test]
fn live_client_trail_rearms_on_fresh_path_and_keeps_unknown_here() {
    let mut c = paint_client();
    c.try_move_path = vec![(0, 0), (1, 0), (2, 0)];
    let _ = live_client_trail(&mut c, Some(wt(3202, 3200)));
    assert!(c.try_move_path.is_empty());

    c.try_move_path = vec![(3, 0), (4, 0), (5, 0)];
    let fresh = live_client_trail(&mut c, Some(wt(3203, 3200)));
    assert_eq!(fresh, vec![wt(3203, 3200), wt(3204, 3200), wt(3205, 3200)]);
    assert_eq!(c.try_move_path.len(), 3, "a new click re-arms the producer");

    let pending = live_client_trail(&mut c, None);
    assert_eq!(pending.len(), 3);
    assert_eq!(
        c.try_move_path.len(),
        3,
        "unknown here must not retire a pending trail"
    );
}

/// A walk leg then a Door transport (loc-backed), the shape the hull
/// and draw-budget tests need.
fn door_route() -> Route {
    Route {
        legs: vec![
            Leg::Walk {
                tiles: vec![
                    WorldTile {
                        x: 3200,
                        z: 3200,
                        level: 0,
                    },
                    WorldTile {
                        x: 3201,
                        z: 3200,
                        level: 0,
                    },
                ],
            },
            Leg::Transport {
                edge: TransportEdge {
                    kind: TransportKind::Door,
                    at: WorldTile {
                        x: 3202,
                        z: 3200,
                        level: 0,
                    },
                    to: WorldTile {
                        x: 3203,
                        z: 3200,
                        level: 0,
                    },
                    loc_id: 1530,
                    option: 1,
                    ticks: 1,
                    dir: None,
                    open_loc_id: None,
                    skill_req: vec![],
                    item_req: vec![],
                    quest_req: vec![],
                    varp_req: vec![],
                    worn_req: vec![],
                    members_req: false,
                },
            },
        ],
        dest: WorldTile {
            x: 3203,
            z: 3200,
            level: 0,
        },
        ticks: 0.0,
    }
}

#[test]
fn face_only_cells_publish_with_fill_without_labels() {
    // Face-only cardinal and corner tiles remain visible with fill on
    // even when the optional NSEW label layer is off.
    let width = 64;
    let height = 64;
    let mut flags = vec![0u32; width * height];
    flags[width + 1] = CollisionFlag::W_S as u32;
    flags[width + 2] = CollisionFlag::W_NE as u32;
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    let world = NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            width,
            height,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    );
    let mut c = paint_client();
    let layers = NavSettings {
        collision_fill: true,
        nsew_labels: false,
        ..NavSettings::default()
    };
    publish_nav_debug(&mut c, &world, None, None, &[], false, None, &layers, true);
    let paint = c.nav_debug_paint().expect("focused drawing slot publishes");
    let face_only = paint
        .collision
        .iter()
        .find(|cell| cell.lx == 1 && cell.lz == 1)
        .expect("the W_S tile must be in the NSEW set");
    assert!(
        face_only.bits & FACE_S != 0 && !face_only.blocked,
        "face-only cell keeps its letter but is never collision-blocked"
    );
    let corner_only = paint
        .collision
        .iter()
        .find(|cell| cell.lx == 2 && cell.lz == 1)
        .expect("the W_NE tile must be published under fill-only");
    assert_eq!(corner_only.bits, CORNER_NE);
    assert!(!corner_only.blocked);
    assert!(
        !paint
            .collision
            .iter()
            .any(|cell| cell.lx == 0 && cell.lz == 0),
        "open ground with no wall bits is omitted under fill-only"
    );
}

#[test]
fn show_nav_path_masters_hulls_click_and_trail() {
    let mut c = paint_client();
    let world = walled_world();
    let route = door_route();
    let click = Some(WorldTile {
        x: 3252,
        z: 3252,
        level: 0,
    });
    let trail_world = [WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    }];

    // Master off: hulls/click/trail stay off even with their own
    // toggles on.
    let layers = NavSettings {
        hop_labels: true,
        client_trail: true,
        ..NavSettings::default()
    };
    publish_nav_debug(
        &mut c,
        &world,
        Some(&route),
        None,
        &trail_world,
        false,
        click,
        &layers,
        true,
    );
    let paint = c.nav_debug_paint().unwrap();
    assert!(
        !paint.show_hulls && paint.hulls.is_empty(),
        "show_nav_path masters the hulls"
    );
    assert!(paint.click.is_none(), "no nav path, no walk-target paint");
    assert!(
        !paint.show_trail && paint.trail.is_empty(),
        "show_nav_path masters the trail"
    );

    // Master on, layer toggles off: hulls and trail still stay off;
    // the click (which has no extra toggle) comes on.
    let layers = NavSettings {
        show_nav_path: true,
        hop_labels: false,
        client_trail: false,
        ..NavSettings::default()
    };
    publish_nav_debug(
        &mut c,
        &world,
        Some(&route),
        None,
        &trail_world,
        false,
        click,
        &layers,
        true,
    );
    let paint = c.nav_debug_paint().unwrap();
    assert!(
        !paint.show_hulls && paint.hulls.is_empty(),
        "hop_labels is the hulls' second gate"
    );
    assert!(
        !paint.show_trail && paint.trail.is_empty(),
        "client_trail is the trail's second gate"
    );
    assert_eq!(
        paint.click,
        Some((52, 52)),
        "show_nav_path masters the click paint"
    );

    // Master + toggles on: the door hop hull and the trail publish.
    let layers = NavSettings {
        show_nav_path: true,
        hop_labels: true,
        client_trail: true,
        ..NavSettings::default()
    };
    publish_nav_debug(
        &mut c,
        &world,
        Some(&route),
        None,
        &trail_world,
        true,
        click,
        &layers,
        true,
    );
    let paint = c.nav_debug_paint().unwrap();
    assert!(
        paint.show_hulls && paint.hulls.iter().any(|h| h.loc_id == 1530),
        "the door hop hull publishes with master + hop_labels"
    );
    assert!(
        paint.show_trail && paint.trail.iter().any(|&(lx, lz, _)| lx == 0 && lz == 0),
        "the trail publishes with master + client_trail"
    );
}

#[test]
fn nav_path_subsamples_to_the_draw_budget_keeping_hops() {
    // 300 walk tiles + a door hop: the 3D path must stay under the
    // draw budget, full density near, keeping the transport hop and
    // the terminal.
    let mut c = paint_client();
    let world = walled_world();
    let mut tiles: Vec<WorldTile> = (0..300)
        .map(|x| WorldTile {
            x: 3200 + x,
            z: 3200,
            level: 0,
        })
        .collect();
    tiles.push(WorldTile {
        x: 3500,
        z: 3200,
        level: 0,
    });
    tiles.push(WorldTile {
        x: 3501,
        z: 3200,
        level: 0,
    });
    let route = Route {
        legs: vec![
            Leg::Walk {
                tiles: tiles[..300].to_vec(),
            },
            Leg::Transport {
                edge: TransportEdge {
                    kind: TransportKind::Door,
                    at: tiles[300],
                    to: tiles[301],
                    loc_id: 1530,
                    option: 1,
                    ticks: 1,
                    dir: None,
                    open_loc_id: None,
                    skill_req: vec![],
                    item_req: vec![],
                    quest_req: vec![],
                    varp_req: vec![],
                    worn_req: vec![],
                    members_req: false,
                },
            },
        ],
        dest: tiles[301],
        ticks: 0.0,
    };
    let layers = NavSettings {
        show_nav_path: true,
        ..NavSettings::default()
    };
    publish_nav_debug(
        &mut c,
        &world,
        Some(&route),
        None,
        &[],
        false,
        None,
        &layers,
        true,
    );
    let paint = c.nav_debug_paint().unwrap();
    assert!(
        paint.path.len() <= MAX_DRAW_TILES,
        "the 3D path respects the draw budget ({} tiles)",
        paint.path.len()
    );
    assert!(
        paint.path.contains(&(300, 0, true)),
        "the transport hop is never subsampled away"
    );
    assert_eq!(
        paint.path.last(),
        Some(&(301, 0, true)),
        "the terminal hop tile always survives"
    );
    assert!(
        paint.path.contains(&(0, 0, false))
            && paint
                .path
                .iter()
                .any(|&p| p == (NEAR_FULL_DENSITY as i32 - 1, 0, false)),
        "the near path stays at full density"
    );
}

#[test]
fn tv_name_follows_the_focused_slot() {
    let mut s = Session::new();
    s.play = Some(empty_play());
    s.slots.insert(
        "s00".into(),
        SlotIo {
            input: SlotInput::new(),
            pixels: FrameBuf::new(),
        },
    );
    s.slots.insert(
        "s05".into(),
        SlotIo {
            input: SlotInput::new(),
            pixels: FrameBuf::new(),
        },
    );
    s.select("s05");
    assert_eq!(
        s.tv_name().as_deref(),
        Some("s05"),
        "Login all must prefer the focused slot, not the first FrameBuf key"
    );
    assert_eq!(
        s.play.as_ref().unwrap().focused().as_deref(),
        Some("s05"),
        "select mirrors the sampled slot onto the play (pure bookkeeping)"
    );
}

#[test]
fn seed_on_first_world_skips_after_reconnect() {
    assert!(seed_on_first_world(None));
    assert!(seed_on_first_world(Some(false)));
    assert!(!seed_on_first_world(Some(true)));
}

#[test]
fn pump_status_log_is_per_username() {
    // two SlotStatus rows, pump twice with transitions; log_by["alice"] does not contain bob lines
    let mut s = Session::new();
    let play = empty_play();
    play.statuses
        .lock()
        .unwrap()
        .extend([status("alice", false, 0), status("bob", false, 0)]);
    s.play = Some(play);

    s.pump_status();
    {
        let log_by = s.log_by.lock().unwrap();
        let alice = log_by.get("alice").expect("alice log");
        let bob = log_by.get("bob").expect("bob log");
        assert!(alice.iter().any(|l| l.contains("slot up")));
        assert!(bob.iter().any(|l| l.contains("slot up")));
        assert!(alice.iter().all(|l| !l.contains("bob")));
        assert!(bob.iter().all(|l| !l.contains("alice")));
    }

    s.play
        .as_ref()
        .unwrap()
        .statuses
        .lock()
        .unwrap()
        .iter_mut()
        .for_each(|row| {
            if row.username == "alice" {
                row.ingame = true;
                row.scene_state = 2;
            } else if row.username == "bob" {
                row.ingame = true;
                row.scene_state = 1;
            }
        });
    s.pump_status();
    let log_by = s.log_by.lock().unwrap();
    let alice = log_by.get("alice").expect("alice log");
    let bob = log_by.get("bob").expect("bob log");
    assert!(alice.iter().any(|l| l.contains("ingame")));
    assert!(alice.iter().any(|l| l.contains("scene 2")));
    assert!(bob.iter().any(|l| l.contains("ingame")));
    assert!(bob.iter().any(|l| l.contains("scene 1")));
    assert!(
        alice
            .iter()
            .all(|l| !l.contains("bob") && !l.contains("scene 1")),
        "alice must not see bob lines: {alice:?}"
    );
    assert!(
        bob.iter()
            .all(|l| !l.contains("alice") && !l.contains("scene 2")),
        "bob must not see alice lines: {bob:?}"
    );
}

#[test]
fn music_toggle_mirrors_onto_the_audio_gate_live() {
    let path = tmp_vault("audio-toggle.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    // The default lowmem slot starts with Music/SFX off: no cpal.
    assert!(s.focused_lowmem());
    s.select("alice");
    assert!(
        !s.audio.music_on("alice"),
        "default lowmem must not arm music"
    );
    // Toggle on (highmem): the gate arms the focused slot's speaker.
    assert!(s.set_focused_lowmem(false));
    assert!(s.audio.music_on("alice"));
    assert!(!s.focused_lowmem());
    // Toggle off (lowmem): the gate tears the speaker down.
    assert!(s.set_focused_lowmem(true));
    assert!(!s.audio.music_on("alice"));
}

#[test]
fn sidecar_cadence_sync_raises_members_not_focus() {
    let mut s = Session::new();
    let a_in = SlotInput::new();
    let b_in = SlotInput::new();
    s.slots.insert(
        "a".into(),
        SlotIo {
            input: Arc::clone(&a_in),
            pixels: FrameBuf::new(),
        },
    );
    s.slots.insert(
        "b".into(),
        SlotIo {
            input: Arc::clone(&b_in),
            pixels: FrameBuf::new(),
        },
    );
    {
        let mut f = s.focus.lock().unwrap();
        f.focused = Some("a".into());
        f.only_render_selected = false;
        f.wall_open = true;
        f.wall = vec!["a".into(), "b".into()];
        f.renderer_by = std::collections::HashMap::from([("a".into(), true), ("b".into(), true)]);
        f.sidecar_50 = true;
    }
    s.focus.lock().unwrap().focused_50 = false;
    s.sync_sidecar_cadence();
    assert!(!a_in.full_rate(), "sidecar must not raise the Game pane");
    assert!(b_in.full_rate(), "the sidecar pref raises a drawing member");
    // Pref off returns the 1 fps watch cadence.
    s.set_sidecar_50(false);
    s.sync_sidecar_cadence();
    assert!(!b_in.full_rate());
}

fn tmp_vault(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("274bot-panel-session-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    if p.exists() {
        std::fs::remove_file(&p).unwrap();
    }
    p
}

fn profile(username: &str, password: &str, uid: i32) -> Profile {
    Profile {
        username: username.into(),
        password: password.into(),
        uid,
        settings: ProfileSettings::default(),
    }
}

#[test]
fn unlock_at_uses_the_given_path() {
    let path = tmp_vault("unlock-at.vault");
    let mut s = Session::new();
    assert!(s.unlock_at(&path, "bot"));
    assert!(s.vault.is_some());
}

#[test]
fn wrong_pass_does_not_delete_or_replace_the_vault() {
    let path = tmp_vault("wrong-pass.vault");
    let mut s = Session::new();
    assert!(s.unlock_at(&path, "bot"));
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    drop(s);

    let mut s = Session::new();
    assert!(!s.unlock_at(&path, "nope"));
    assert!(s.vault.is_none());
    assert!(path.is_file());
    let v = Vault::unlock(&path, "bot").unwrap();
    assert!(v.get("alice").is_some());
}

#[test]
fn reset_vault_at_refuses_while_unlocked() {
    let path = tmp_vault("reset-locked.vault");
    let mut s = Session::new();
    assert!(s.unlock_at(&path, "bot"));
    assert!(!s.reset_vault_at(&path));
    assert!(path.is_file());
    assert!(s.vault.is_some());
}

#[test]
fn reset_vault_at_deletes_while_locked() {
    let path = tmp_vault("reset-ok.vault");
    let mut s = Session::new();
    assert!(s.unlock_at(&path, "bot"));
    s.vault = None;
    assert!(s.reset_vault_at(&path));
    assert!(!path.exists());
    assert!(s.unlock_at(&path, "newpass"));
}

#[test]
fn session_starts_with_renderer_on_capture_on_by_default() {
    crate::ui_state::save(&crate::ui_state::PanelUiState::default());
    let s = Session::new();
    let f = s.focus.lock().unwrap();
    assert!(f.renderer, "rail is on; Game pane 50 fps is focused_50");
    assert!(f.capture, "capture pref defaults on");
    assert!(f.focused_50);
}

#[test]
fn multibox_toggle_does_not_arm_scatter() {
    let mut s = Session::new();
    s.set_multibox(true);
    assert!(
        !s.scatter.load(Ordering::Relaxed),
        "MultiBox must not arm the stress50 scatter-seed"
    );
    s.set_multibox(false);
    assert!(!s.scatter.load(Ordering::Relaxed));
}

#[test]
fn walk_status_is_dash_when_no_route() {
    let s = Session::new();
    assert_eq!(s.walk_status_text(), "—");
}

#[test]
fn confirm_picker_walk_uses_player_plane_not_dest_level() {
    let mut s = Session::new();
    s.focus.lock().unwrap().focused = Some("alice".into());
    s.statuses.push(SlotStatus {
        username: "alice".into(),
        tile_x: 5,
        tile_z: 5,
        tile_level: 1,
        ..SlotStatus::default()
    });
    assert_eq!(s.focused_tile(), Some((5, 5, 1)));
    let world = open_world_four_planes(10, 10);
    s.picker_sel = Some(Tile {
        x: 2,
        z: 2,
        level: 0,
    });
    assert!(s.confirm_picker_walk(&world));
    assert!(
        s.error.is_some(),
        "upstairs origin must not masquerade as the dest plane"
    );
    assert!(
        s.travellers
            .lock()
            .unwrap()
            .get("alice")
            .is_none_or(|a| a.lock().unwrap().route.is_none()),
        "a wrong-plane route must not arm"
    );
}

/// Like [`open_world`] but allocates all four level planes so routing
/// from an upstairs origin does not index past the walk bake.
fn open_world_four_planes(w: usize, h: usize) -> NavWorld {
    let cells = w * h * 4;
    NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: w,
            height: h,
            walk: vec![0u8; cells],
            blocked: vec![0u64; cells.div_ceil(64)],
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    )
}

#[test]
fn picker_select_does_not_arm_until_confirm() {
    let mut s = Session::new();
    let dest = Tile {
        x: 2,
        z: 2,
        level: 0,
    };
    s.picker_sel = Some(dest);
    assert_eq!(s.walk_status_text(), "—");
    assert!(s.confirm_picker_walk(&open_world(3, 3)));
    assert!(s.walk_status_text().contains("2"));
    assert!(s.picker_sel.is_none());
    assert!(!s.confirm_picker_walk(&open_world(3, 3)));
}

#[test]
fn arm_walk_sets_queued_text() {
    let mut s = Session::new();
    s.arm_walk(Tile {
        x: 3222,
        z: 3222,
        level: 0,
    });
    assert!(s.walk_status_text().contains("3222"));
}

#[test]
fn arm_walk_on_routes_and_arms_focused_traveller() {
    let mut s = Session::new();
    s.focus.lock().unwrap().focused = Some("alice".into());
    let world = open_world(3, 3);
    let dest = Tile {
        x: 2,
        z: 2,
        level: 0,
    };
    s.arm_walk_on(
        &world,
        Tile {
            x: 0,
            z: 0,
            level: 0,
        },
        dest,
    );
    assert_eq!(s.walk_dest, Some(dest), "dest stays stored on success");
    assert!(s.error.is_none(), "a found route clears the error banner");
    let queued = s
        .travellers
        .lock()
        .unwrap()
        .get("alice")
        .expect("focused walk arm exists")
        .lock()
        .unwrap()
        .queued_tile();
    assert_eq!(queued, Some(dest));
}

#[test]
fn arm_walk_on_feeds_the_focused_slots_latched_essence_session() {
    let mut s = Session::new();
    s.focus.lock().unwrap().focused = Some("alice".into());
    // Alice's walker already latched the mine session (entered via
    // Aubury): a WalkTo out of the mine must route through the exit
    // portal's return hop — the arm feeds the traveller's latch.
    s.travellers.lock().unwrap().insert(
        "alice".into(),
        Arc::new(Mutex::new(WalkArm {
            traveller: {
                let mut t = Traveller::new();
                t.set_essence(nav::essence::essence_session_for_wizard(553));
                t
            },
            route: None,
            bank_fetch: None,
        })),
    );
    let world = mine_world();
    let anchor = Tile {
        x: 3253,
        z: 3401,
        level: 0,
    };
    s.arm_walk_on(
        &world,
        Tile {
            x: 2912,
            z: 4833,
            level: 0, // the mine pad
        },
        anchor,
    );
    assert!(
        s.error.is_none(),
        "the latched session routes out of the mine"
    );
    let queued = s
        .travellers
        .lock()
        .unwrap()
        .get("alice")
        .expect("focused walk arm exists")
        .lock()
        .unwrap()
        .queued_tile();
    assert_eq!(
        queued,
        Some(anchor),
        "the exit route reaches Aubury's anchor"
    );
}

#[test]
fn arm_walk_on_without_a_latch_keeps_the_mine_sealed() {
    let mut s = Session::new();
    s.focus.lock().unwrap().focused = Some("alice".into());
    // No latch: the session return hop is never relaxed — the sealed
    // mine stays NoPath (fail-closed without a session is correct).
    let world = mine_world();
    s.arm_walk_on(
        &world,
        Tile {
            x: 2912,
            z: 4833,
            level: 0,
        },
        Tile {
            x: 3253,
            z: 3401,
            level: 0,
        },
    );
    assert!(
        s.error.as_deref().is_some_and(|e| e.contains("no path")),
        "no latch -> the mine is sealed, got {:?}",
        s.error
    );
}

#[test]
fn arm_walk_on_no_path_stores_dest_and_sets_error() {
    let mut s = Session::new();
    s.focus.lock().unwrap().focused = Some("alice".into());
    // Block the middle column: (1,0), (1,1), (1,2) on the 3x3 world.
    let mut flags = vec![0u32; 9];
    for z in 0..3 {
        flags[z * 3 + 1] = CollisionFlag::WALK_BLOCK_FLAGS as u32;
    }
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    let world = NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 3,
            height: 3,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    );
    let dest = Tile {
        x: 2,
        z: 1,
        level: 0,
    };
    s.arm_walk_on(
        &world,
        Tile {
            x: 0,
            z: 1,
            level: 0,
        },
        dest,
    );
    assert_eq!(s.walk_dest, Some(dest), "dest stays stored on NoPath");
    let err = s.error.clone().expect("no-path message set");
    assert!(
        err.contains("no path"),
        "short no-path message, got {err:?}"
    );
    assert!(
        s.travellers
            .lock()
            .unwrap()
            .get("alice")
            .is_none_or(|a| a.lock().unwrap().route.is_none()),
        "no route must be armed when find fails"
    );
}

/// A 5×5 world walled between x=1 and x=2, crossed only by a 10-coin
/// toll door (the `toll_edges` shape: loc 2882, `item_req` coins 10).
fn toll_world() -> NavWorld {
    let mut flags = vec![0u32; 25];
    for z in 0..5 {
        flags[z * 5 + 1] |= CollisionFlag::W_E as u32;
        flags[z * 5 + 2] |= CollisionFlag::W_W as u32;
    }
    let edge = TransportEdge {
        kind: TransportKind::Door,
        at: WorldTile {
            x: 1,
            z: 2,
            level: 0,
        },
        to: WorldTile {
            x: 2,
            z: 2,
            level: 0,
        },
        loc_id: 2882,
        option: 1,
        ticks: 2,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![(995, 10)],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
    };
    let mut graph = TransportGraph::default();
    graph.at.entry(edge.at).or_default().push(0);
    graph.edges.push(edge);
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 5,
            height: 5,
            walk,
            blocked,
            flags: None,
        },
        graph,
        Vec::new(),
    )
}

/// A `WorldState` with 10 coins, derived through the slot-thread
/// publish path: a client with a TYPE_INV iface, rebuilt into a
/// snapshot, mapped via [`WorldState::from_snapshot`]. The cache dir
/// is a unique scratch dir so a stray real cache (e.g. jags under
/// `/tmp`) cannot seed other ifaces.
fn coins_snapshot_state() -> WorldState {
    let cache_dir = std::env::temp_dir().join(format!(
        "274bot-panel-toll-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut c = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: cache_dir.to_str().unwrap().into(),
        members: true,
        lowmem: false,
    });
    let inv_id = c.push_iface(IfType {
        r#type: ComponentType::TYPE_INV,
        ..Default::default()
    });
    c.set_iface_mut(
        inv_id,
        IfTypeMut {
            link_obj_type: Some(vec![996]), // obj 995 → stored 996
            link_obj_number: Some(vec![10]),
            ..Default::default()
        },
    );
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    WorldState::from_snapshot(&snap)
}

/// The focused slot's published snapshot facts gate the WalkTo route:
/// 10 coins on the player let `arm_walk_on` cross a toll that an
/// empty state refuses.
#[test]
fn arm_walk_on_uses_focused_slot_state_across_a_toll() {
    let world = toll_world();
    let mut s = Session::new();
    s.focus.lock().unwrap().focused = Some("alice".into());
    // The slot thread published 10 coins (derived from a live
    // snapshot); the picker routes with those facts.
    s.nav_states.lock().unwrap().insert(
        "alice".into(),
        (GameSnapshot::new(), coins_snapshot_state()),
    );
    s.arm_walk_on(
        &world,
        Tile {
            x: 0,
            z: 0,
            level: 0,
        },
        Tile {
            x: 4,
            z: 4,
            level: 0,
        },
    );
    assert!(
        s.error.is_none(),
        "coins on the player pay the toll: {:?}",
        s.error
    );
    let route = s
        .travellers
        .lock()
        .unwrap()
        .get("alice")
        .expect("focused walk arm exists")
        .lock()
        .unwrap()
        .route
        .clone();
    assert!(
        route
            .expect("the toll route armed")
            .legs
            .iter()
            .any(|l| matches!(
                l,
                Leg::Transport { edge } if edge.item_req == vec![(995, 10)]
            )),
        "the route must cross the toll"
    );
}

/// Walk follow reads the slot's stored nav snapshot, not a from-scratch
/// `GameSnapshot::new()` on each hop tick.
#[test]
fn walk_follow_uses_stored_nav_snapshot() {
    use std::collections::HashMap;
    let mut states = HashMap::new();
    let snap = GameSnapshot::new();
    states.insert("alice".into(), (snap, WorldState::empty()));
    let stored = states.get("alice").unwrap().0.locs().as_ptr();
    let got = nav_snapshot_for_follow(&states, "alice").expect("published snapshot");
    assert!(
        std::ptr::eq(got.locs().as_ptr(), stored),
        "follow must borrow the stored snapshot"
    );
    assert!(nav_snapshot_for_follow(&states, "bob").is_none());
}

/// No published facts for the slot (still logging in / no player
/// decoded yet) keeps the fail-closed behavior: the toll stays
/// unusable and `arm_walk_on` reports `NoPath`.
#[test]
fn arm_walk_on_falls_back_to_empty_when_slot_has_no_state() {
    let world = toll_world();
    let mut s = Session::new();
    s.focus.lock().unwrap().focused = Some("alice".into());
    s.arm_walk_on(
        &world,
        Tile {
            x: 0,
            z: 0,
            level: 0,
        },
        Tile {
            x: 4,
            z: 4,
            level: 0,
        },
    );
    assert!(
        s.error.as_ref().unwrap().contains("no path"),
        "no published facts -> the toll stays unusable"
    );
    assert!(
        s.travellers
            .lock()
            .unwrap()
            .get("alice")
            .is_none_or(|a| a.lock().unwrap().route.is_none()),
        "no route armed when the toll is unpayable"
    );
}

#[test]
fn arm_walk_on_ignores_teles_until_allow_teleports() {
    // world: origin cannot walk to dest; a teleport edge can.
    let mut session = Session::new();
    session.focus.lock().unwrap().focused = Some("alice".into());
    // Wall splits the 5x5 between x=1 and x=2 (nav fixture shape), so
    // no walk crosses; only the any-tile teleport edge reaches (4,4).
    let mut flags = vec![0u32; 25];
    for z in 0..5 {
        flags[z * 5 + 1] |= CollisionFlag::W_E as u32;
        flags[z * 5 + 2] |= CollisionFlag::W_W as u32;
    }
    let dest_tile = Tile {
        x: 4,
        z: 4,
        level: 0,
    };
    let dest = WorldTile {
        x: 4,
        z: 4,
        level: 0,
    };
    let mut graph = TransportGraph::default();
    graph.teleports.push(TransportEdge {
        kind: TransportKind::Teleport,
        at: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: dest,
        loc_id: 0,
        option: 0,
        ticks: 3,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
    });
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    let world = NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 5,
            height: 5,
            walk,
            blocked,
            flags: None,
        },
        graph,
        Vec::new(),
    );
    let origin = Tile {
        x: 0,
        z: 0,
        level: 0,
    };
    session.ui.nav.allow_teleports = false;
    session.arm_walk_on(&world, origin, dest_tile);
    assert!(
        session.error.as_ref().unwrap().contains("no path"),
        "walk-only find must not use the teleport edge"
    );
    session.ui.nav.allow_teleports = true;
    session.arm_walk_on(&world, origin, dest_tile);
    assert!(
        session.error.is_none(),
        "allow_teleports routes the teleport"
    );
    let arm = session.travellers.lock().unwrap();
    let route = arm
        .get(&session.focused_name().unwrap())
        .unwrap()
        .lock()
        .unwrap()
        .route
        .clone();
    assert!(route.unwrap().legs.iter().any(|l| matches!(
        l,
        Leg::Transport { edge } if edge.kind == TransportKind::Teleport
    )));
}

#[test]
fn arm_walk_on_uses_find_with_options() {
    // The tele fixture moved to wildy-north coords, with the teleport
    // landing on a wilderness tile: the teleport edge is the only way
    // across the wall, and its landing is inside the zone. Neither
    // flag alone may route — `arm_walk_on` must pass both
    // `ui.nav.allow_teleports` and `ui.nav.allow_wilderness` through
    // to `find_with`.
    let mut session = Session::new();
    session.focus.lock().unwrap().focused = Some("alice".into());
    let mut flags = vec![0u32; 5 * 12];
    for z in 0..12 {
        flags[z * 5 + 1] |= CollisionFlag::W_E as u32;
        flags[z * 5 + 2] |= CollisionFlag::W_W as u32;
    }
    let dest_tile = Tile {
        x: 3102,
        z: 3525,
        level: 0,
    };
    let dest = WorldTile {
        x: 3102,
        z: 3525,
        level: 0,
    };
    let mut graph = TransportGraph::default();
    graph.teleports.push(TransportEdge {
        kind: TransportKind::Teleport,
        at: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: dest,
        loc_id: 0,
        option: 0,
        ticks: 3,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
    });
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    let world = NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 3099,
                z: 3518,
                level: 0,
            },
            width: 5,
            height: 12,
            walk,
            blocked,
            flags: None,
        },
        graph,
        Vec::new(),
    );
    let origin = Tile {
        x: 3100,
        z: 3519,
        level: 0,
    };
    session.ui.nav.allow_teleports = false;
    session.ui.nav.allow_wilderness = false;
    session.arm_walk_on(&world, origin, dest_tile);
    assert!(
        session.error.as_ref().unwrap().contains("no path"),
        "default find must not route into the wilderness"
    );
    session.ui.nav.allow_teleports = true;
    session.arm_walk_on(&world, origin, dest_tile);
    assert!(
        session.error.as_ref().unwrap().contains("no path"),
        "a teleport landing inside the wilderness stays blocked without allow_wilderness"
    );
    session.ui.nav.allow_wilderness = true;
    session.arm_walk_on(&world, origin, dest_tile);
    assert!(
        session.error.is_none(),
        "both UI flags must route the teleport into the wilderness"
    );
    let arm = session.travellers.lock().unwrap();
    let route = arm
        .get(&session.focused_name().unwrap())
        .unwrap()
        .lock()
        .unwrap()
        .route
        .clone();
    assert!(route.unwrap().legs.iter().any(|l| matches!(
        l,
        Leg::Transport { edge } if edge.kind == TransportKind::Teleport
    )));
}

#[test]
fn arm_walk_on_without_focus_skips_route_but_stores_dest() {
    let mut s = Session::new();
    let world = open_world(3, 3);
    let dest = Tile {
        x: 2,
        z: 2,
        level: 0,
    };
    s.arm_walk_on(
        &world,
        Tile {
            x: 0,
            z: 0,
            level: 0,
        },
        dest,
    );
    assert_eq!(s.walk_dest, Some(dest));
    assert!(
        s.travellers.lock().unwrap().is_empty(),
        "no focused name to key a walk arm"
    );
}

#[test]
fn arm_walk_on_success_bumps_route_gen() {
    let mut s = Session::new();
    s.focus.lock().unwrap().focused = Some("alice".into());
    let world = open_world(3, 3);
    assert_eq!(s.route_gen(), 0);
    s.arm_walk_on(
        &world,
        Tile {
            x: 0,
            z: 0,
            level: 0,
        },
        Tile {
            x: 2,
            z: 2,
            level: 0,
        },
    );
    assert_ne!(s.route_gen(), 0, "a new arm must bump the overlay gen");
}

#[test]
fn sync_walk_status_copies_queued_and_clears_dest_on_arrived() {
    let mut s = Session::new();
    s.focus.lock().unwrap().focused = Some("alice".into());
    s.statuses.push(SlotStatus {
        username: "alice".into(),
        ..SlotStatus::default()
    });
    let world = open_world(3, 3);
    let dest = Tile {
        x: 2,
        z: 2,
        level: 0,
    };
    s.arm_walk_on(
        &world,
        Tile {
            x: 0,
            z: 0,
            level: 0,
        },
        dest,
    );
    s.sync_walk_status();
    assert_eq!(
        (
            s.statuses[0].walk_x,
            s.statuses[0].walk_z,
            s.statuses[0].walk_level
        ),
        (2, 2, 0)
    );
    // The slot hook clears the route and flags walk_clear on Arrived.
    s.travellers
        .lock()
        .unwrap()
        .get("alice")
        .unwrap()
        .lock()
        .unwrap()
        .route = None;
    s.walk_clear
        .store(true, std::sync::atomic::Ordering::Relaxed);
    s.sync_walk_status();
    assert_eq!(s.walk_status_text(), "—");
    assert_eq!(
        (
            s.statuses[0].walk_x,
            s.statuses[0].walk_z,
            s.statuses[0].walk_level
        ),
        (-1, -1, -1)
    );
}

#[test]
fn select_bumps_route_gen_only_on_focus_change() {
    let mut s = Session::new();
    assert_eq!(s.route_gen(), 0);
    s.select("alice");
    assert_eq!(s.route_gen(), 1);
    assert_eq!(s.focused_name().as_deref(), Some("alice"));
    s.select("alice");
    assert_eq!(s.route_gen(), 1, "re-selecting the focused name is a no-op");
    s.select("bob");
    assert_eq!(s.route_gen(), 2);
}

#[test]
fn focused_tile_is_none_without_status() {
    let s = Session::new();
    s.focus.lock().unwrap().focused = Some("alice".into());
    assert_eq!(s.focused_tile(), None, "no status rows yet");
}

#[test]
fn combo_index_is_none_when_unfocused() {
    let names = vec!["alice".into(), "bob".into()];
    assert_eq!(combo_index(None, &names), None);
    assert_eq!(combo_index(Some("alice"), &names), Some(0));
    assert_eq!(combo_index(Some("bob"), &names), Some(1));
    assert_eq!(combo_index(Some("carol"), &names), None);
}

#[test]
fn focus_first_profile_selects_first_vault_name() {
    crate::ui_state::save(&crate::ui_state::PanelUiState::default());
    let path = tmp_vault("focus-first.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("bob", "pw", 43))
        .unwrap();
    s.focus_first_profile();
    assert_eq!(s.focused_name().as_deref(), Some("alice"));
    assert_eq!(s.cred_user, "alice");
    assert!(s.slots.contains_key("alice"));
    assert!(
        !s.slots.contains_key("bob"),
        "parked vault rows must not start a Client"
    );
}

#[test]
fn focus_first_prefers_last_focus() {
    let path = tmp_vault("focus-last.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("bob", "pw", 43))
        .unwrap();
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: Some("bob".into()),
        ..Default::default()
    });
    s.focus_first_profile();
    assert_eq!(s.focused_name().as_deref(), Some("bob"));
    assert_eq!(crate::ui_state::load().last_focus.as_deref(), Some("bob"));
}

#[test]
fn select_saves_last_focus() {
    crate::ui_state::save(&crate::ui_state::PanelUiState::default());
    let path = tmp_vault("select-last-focus.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.select("alice");
    assert_eq!(crate::ui_state::load().last_focus.as_deref(), Some("alice"));
}

#[test]
fn set_multibox_restores_last_focus_when_focus_not_on_wall() {
    let path = tmp_vault("multibox-last-focus.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("bob", "pw", 43))
        .unwrap();
    s.select("alice");
    s.wall.load("bob");
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: Some("bob".into()),
        ..Default::default()
    });
    // Focused alice is not a wall member; MultiBox-on should pick bob.
    s.set_multibox(true);
    assert_eq!(s.focused_name().as_deref(), Some("bob"));
    assert!(s.wall.members.iter().any(|m| m == "bob"));
}

#[test]
fn select_spawns_parked_profile_once() {
    let path = tmp_vault("select-spawn.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("bob", "pw", 43))
        .unwrap();
    s.select("alice");
    assert_eq!(s.slots.len(), 1);
    s.select("bob");
    assert_eq!(s.slots.len(), 2);
    s.select("alice");
    assert_eq!(s.slots.len(), 2);
}

#[test]
fn flat_model_spawns_every_member_as_a_client() {
    let path = tmp_vault("flat-spawn.vault");
    let mut s = Session::new();
    assert!(s.unlock_at(&path, "bot"));
    for (n, uid) in [("alice", 1), ("bob", 2), ("carol", 3)] {
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile(n, "pw", uid))
            .unwrap();
    }
    s.select("alice");
    s.load("bob");
    s.load("carol");
    // ensure_slot registers the IO map synchronously; wait only for the
    // slot threads to publish their status rows.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if s.play.as_ref().unwrap().statuses().len() == 3 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(s.slots.len(), 3, "every wall member owns a FrameBuf slot");
    assert_eq!(
        s.play.as_ref().unwrap().statuses().len(),
        3,
        "one full Client slot per profile — no lean channels"
    );
    assert!(
        s.play.as_ref().unwrap().arm("carol").is_some(),
        "every member has a control arm"
    );
    // Focus is pure bookkeeping: selecting bob redirects the sampled
    // slot without touching a socket.
    s.select("bob");
    assert_eq!(s.focused_name().as_deref(), Some("bob"));
    assert_eq!(s.play.as_ref().unwrap().focused().as_deref(), Some("bob"));
    assert_eq!(
        s.play.as_ref().unwrap().statuses().len(),
        3,
        "focus does not swap sockets; every slot stays up"
    );
    // No `stop_slot` joins here: the slot threads sit in `maininit`'s
    // bounded HTTP retry (host-play shrinks it only under its own
    // `#[cfg(test)]`), so a join would block the suite for minutes.
    // The threads are detached and die at process exit.
}

#[test]
fn sidecar_select_does_not_restart_when_game_is_highmem() {
    let path = tmp_vault("select-no-restart.vault");
    let mut s = Session::new();
    s.persist_ui = false;
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("bob", "pw", 43))
        .unwrap();
    s.ui.lowmem = false;
    s.select("alice");
    s.load("bob");
    let alice_px = std::sync::Arc::as_ptr(&s.slots.get("alice").unwrap().pixels);
    let bob_px = std::sync::Arc::as_ptr(&s.slots.get("bob").unwrap().pixels);
    s.select("bob");
    assert_eq!(s.focused_name().as_deref(), Some("bob"));
    assert_eq!(
        std::sync::Arc::as_ptr(&s.slots.get("alice").unwrap().pixels),
        alice_px
    );
    assert_eq!(
        std::sync::Arc::as_ptr(&s.slots.get("bob").unwrap().pixels),
        bob_px
    );
    let log = s.log_by.lock().unwrap();
    assert!(!log.values().flatten().any(|l| l.contains("slot restarted")));
}

#[test]
fn sidecar_select_does_not_restart_when_game_is_cpu() {
    let path = tmp_vault("select-no-restart-cpu.vault");
    let mut s = Session::new();
    s.persist_ui = false;
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("bob", "pw", 43))
        .unwrap();
    s.ui.raster = vault::RasterMode::Cpu;
    s.select("alice");
    s.load("bob");
    let alice_px = std::sync::Arc::as_ptr(&s.slots.get("alice").unwrap().pixels);
    s.select("bob");
    assert_eq!(
        std::sync::Arc::as_ptr(&s.slots.get("alice").unwrap().pixels),
        alice_px
    );
}

#[test]
fn logout_all_arms_every_wall_member() {
    let path = tmp_vault("logout-all-flat.vault");
    let mut s = Session::new();
    assert!(s.unlock_at(&path, "bot"));
    for (n, uid) in [("alice", 1), ("bob", 2)] {
        s.vault
            .as_mut()
            .unwrap()
            .upsert(profile(n, "pw", uid))
            .unwrap();
    }
    s.select("alice");
    s.load("bob");
    s.wall.load("alice");
    s.wall.load("bob");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if s.play.as_ref().unwrap().arm("bob").is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    s.logout_all();
    assert!(
        s.play
            .as_ref()
            .unwrap()
            .arm("alice")
            .unwrap()
            .want_logout
            .load(Ordering::Relaxed),
        "the focused member must logout"
    );
    assert!(
        s.play
            .as_ref()
            .unwrap()
            .arm("bob")
            .unwrap()
            .want_logout
            .load(Ordering::Relaxed),
        "every wall member must logout"
    );
    // No `stop_slot` joins: the slot threads stay in `maininit`'s
    // bounded HTTP retry (see `flat_model_spawns_every_member_as_a_client`).
}

#[test]
fn headed_stress_spawns_every_member_prefers_and_arms_s00() {
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: Some("s02".into()),
        ..Default::default()
    });
    let mut s = Session::new();
    s.live_prepare_stress(3, false).expect("prepare");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if s.play.as_ref().unwrap().statuses().len() == 3 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(s.slots.len(), 3, "every member owns its own Client slot");
    assert_eq!(
        s.focused_name().as_deref(),
        Some("s00"),
        "focused slot must be s00, not last_focus s02"
    );
    assert_eq!(s.tv_name().as_deref(), Some("s00"));
    assert!(
        s.play.as_ref().unwrap().login_queue_uids().is_empty(),
        "focus and login intent do not create control-thread membership"
    );
    assert!(
        s.play
            .as_ref()
            .unwrap()
            .arm("s00")
            .unwrap()
            .want_login
            .load(Ordering::Relaxed),
        "the focused slot arms immediately"
    );
    assert!(
        s.focus.lock().unwrap().only_render_selected,
        "RAM watch is cap-only (Game paints, rail skip-paint)"
    );
    assert!(
        !s.focus.lock().unwrap().live_full_rate,
        "RAM watch does not raise sidecar/game to 50 fps overlay"
    );
    assert!(
        s.scatter.load(Ordering::Relaxed),
        "stress wall scatter-seeds after scene 2"
    );
    // No `stop_slot` joins: the slot threads stay in `maininit`'s
    // bounded HTTP retry (see `flat_model_spawns_every_member_as_a_client`).
}

#[test]
fn login_all_arms_every_wall_member() {
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = Session::new();
    s.live_prepare_stress(2, false).expect("prepare");
    assert!(
        s.play
            .as_ref()
            .unwrap()
            .arm("s01")
            .unwrap()
            .want_login
            .load(Ordering::Relaxed),
        "login all arms every member immediately (the FIFO serializes)"
    );
    // No `stop_slot` joins (see `flat_model_spawns_every_member_as_a_client`).
}

#[test]
fn headed_stress_full_paints_every_member_at_50fps() {
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = Session::new();
    s.live_prepare_stress(2, true).expect("prepare");
    let f = s.focus.lock().unwrap();
    assert!(
        !f.only_render_selected,
        "full-rate 50 paints sidecar tiles, not cap-only"
    );
    assert!(f.live_full_rate, "full-rate overlay on Game + sidecar");
    drop(f);
    for name in ["s00", "s01"] {
        let slot = s.slots.get(name).expect("slot");
        assert!(slot.input.full_rate(), "{name} must run the 50 fps cadence");
    }
}

/// RAM watch members must be raster Off so a flipped only-render-
/// selected cannot attach 49 GPU heads. The Game pane's one seat
/// follows focus (click a working member after s00's −1).
#[test]
fn stress50_rail_members_are_raster_off() {
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = Session::new();
    s.live_prepare_stress(3, false).expect("prepare");
    let f = s.focus.lock().unwrap();
    assert_eq!(f.renderer_by.get("s00").copied(), Some(true));
    assert_eq!(
        f.renderer_by.get("s01").copied(),
        Some(false),
        "s01 must not be able to grow a GPU head"
    );
    assert_eq!(f.renderer_by.get("s02").copied(), Some(false));
    assert!(draw_for_slot(&f, "s00"));
    assert!(!draw_for_slot(&f, "s01"));
    drop(f);
    s.select("s01");
    let f = s.focus.lock().unwrap();
    assert!(
        draw_for_slot(&f, "s01"),
        "focus moves the one GPU seat onto a rail-Off member"
    );
    assert!(!draw_for_slot(&f, "s00"));
    drop(f);
    s.focus.lock().unwrap().only_render_selected = false;
    let f = s.focus.lock().unwrap();
    assert!(
        !draw_for_slot(&f, "s02"),
        "raster Off must keep unfocused rail members unheaded even with render-all"
    );
    assert!(draw_for_slot(&f, "s01"));
}

#[test]
fn live_prepare_script_boots_the_seed_profile_and_installs_runner() {
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = Session::new();
    let scenario = scenario::get("walk").expect("walk scenario in registry");
    s.live_prepare_script(scenario).expect("prepare");
    // Live boots a minted per-run account — never the registry's
    // `test` (engine auto-registers unknown names).
    let name = s
        .vault
        .as_ref()
        .expect("live vault open")
        .profiles()
        .next()
        .expect("minted profile")
        .username
        .clone();
    assert_ne!(name, "test", "live must not log in `test`");
    let play = s.play.as_ref().expect("play started");
    assert!(
        play.arm(&name).unwrap().want_login.load(Ordering::Relaxed),
        "login all arms the minted profile's handshake"
    );
    let runner = s.scenario.lock().unwrap();
    let runner = runner.as_ref().expect("scenario runner installed");
    assert_eq!(runner.profile_name(), name);
    assert!(
        matches!(runner.status(), scenario::RunnerStatus::Seeding),
        "a fresh runner holds in seeding until ingame scene 2"
    );
    assert!(
        runner.drives(&name) && !runner.drives("test"),
        "the runner ticks only its minted profile's slot"
    );
    // No `stop_slot` joins (see `flat_model_spawns_every_member_as_a_client`).
}

/// A scenario that names a script card (`start_script`) fills the
/// catalog and selects it on the driven slot — Start waits for the
/// `StartScript` step after seed. `$RS2B0T` is read here, and the
/// card's source is what the isolate will spawn on that step.
#[test]
fn live_prepare_bone_burier_starts_the_rs2b0t_card_on_the_driven_slot() {
    let iso = IsolatedEnv::enter("bone-live");
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let root = write_looping_catalog(&iso.dir, &[("BoneBurier", "BoneBurier")]);
    iso.set_rs2b0t(&root);
    let mut s = Session::new();
    let result = s.live_prepare_script(scenario::get("bone_burier").expect("registered"));
    let name = s
        .vault
        .as_ref()
        .expect("live vault open")
        .profiles()
        .next()
        .expect("minted profile")
        .username
        .clone();
    assert_ne!(name, "test", "live must not log in `test`");
    match result {
        Ok(()) => {}
        Err(e) => panic!("live_prepare must select the card: {e}"),
    }
    assert_eq!(
        s.script_sel,
        Some(script::ScriptSel::Loaded(
            script::ScriptSource::Catalog,
            "BoneBurier".into()
        )),
        "prepare sets script_sel to the catalog card"
    );
    let play = s.play.as_ref().expect("play started");
    assert_ne!(
        play.script_state(&name),
        script::RunState::Running,
        "isolate is not Running yet — Start waits for the StartScript step"
    );
}

#[test]
fn live_prepare_bone_burier_v2_selects_each_example_by_identity() {
    let ts = script::live_example_path("bone_burier_v2.ts").expect("ts example");
    let js = script::live_example_path("bone_burier_v2.js").expect("js example");
    for (name, file_name, path) in [
        ("bone_burier_v2_ts", "bone_burier_v2.ts", ts.as_path()),
        ("bone_burier_v2_js", "bone_burier_v2.js", js.as_path()),
    ] {
        let iso = IsolatedEnv::enter(&format!("bone-v2-{name}"));
        let mut s = Session::new();
        s.js = script::JsLibrary::with_cache(
            iso.dir.join("js-scripts.json"),
            iso.dir.join("js-cache"),
        );
        s.js.load(&ts).expect("preload ts");
        s.js.load(&js).expect("preload js");
        s.script_settings.set_str(
            script::ScriptSource::File,
            "bone_burier_v2",
            "boneName",
            "stem",
        );
        let identity = script::file_identity(path);
        s.script_settings.set_str(
            script::ScriptSource::File,
            &identity,
            "boneName",
            "identity",
        );
        let scenario = scenario::get(name).expect("registered");
        assert_eq!(scenario.settings.start_file, Some(file_name));
        assert_eq!(scenario.settings.start_script, None);
        let card = load_live_example_card(&mut s.js, file_name).expect("identity load");
        assert_eq!(
            card.identity_id(),
            identity,
            "{name} canonical-path identity"
        );
        assert_ne!(
            identity, "bone_burier_v2",
            "{name} must not use the shared stem"
        );
        let bag = s
            .pending_settings_bag(script::ScriptSource::File, &identity, &card.settings_schema)
            .expect("settings bag");
        assert_eq!(
            bag.get("boneName"),
            Some(&serde_json::json!("identity")),
            "{name} settings attach to the selected File identity"
        );
        let stem_bag = s.pending_settings_bag(
            script::ScriptSource::File,
            "bone_burier_v2",
            &card.settings_schema,
        );
        assert_eq!(
            stem_bag.as_ref().and_then(|bag| bag.get("boneName")),
            Some(&serde_json::json!("stem")),
            "{name} stem bag stays isolated from the File identity"
        );
    }
}

#[test]
fn live_prepare_keeps_v1_catalog_and_tradebot_stem() {
    let iso = IsolatedEnv::enter("bone-v1-trade");
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let root = write_looping_catalog(&iso.dir, &[("BoneBurier", "BoneBurier")]);
    iso.set_rs2b0t(&root);
    let mut s = Session::new();
    s.live_prepare_script(scenario::get("bone_burier").expect("registered"))
        .expect("v1");
    assert_eq!(
        s.script_sel,
        Some(script::ScriptSel::Loaded(
            script::ScriptSource::Catalog,
            "BoneBurier".into()
        ))
    );
    let mut trade = Session::new();
    trade.js =
        script::JsLibrary::with_cache(iso.dir.join("trade-js.json"), iso.dir.join("trade-cache"));
    trade
        .live_prepare_script(scenario::get("script_trade").expect("registered"))
        .expect("trade");
    assert_eq!(
        trade.script_sel,
        Some(script::ScriptSel::Loaded(
            script::ScriptSource::File,
            "trade_bot".into()
        ))
    );
}

#[test]
fn script_self_stop_requires_idle_and_receipt_reason() {
    let receipt = script::ScriptLifecycleReceipt {
        runtime_generation: 1,
        state: script::ScriptTerminalState::Stopped,
        tick: 9,
        reason: "confirmed loaded current-generation bank exhaustion".into(),
    };
    assert!(script_self_stop_observed(
        script::RunState::Idle,
        Some(&receipt),
        "confirmed loaded current-generation bank exhaustion"
    ));
    assert!(!script_self_stop_observed(
        script::RunState::Running,
        Some(&receipt),
        "confirmed loaded current-generation bank exhaustion"
    ));
    assert!(!script_self_stop_observed(
        script::RunState::Idle,
        Some(&receipt),
        "stalled while bury"
    ));
    assert!(!script_self_stop_observed(
        script::RunState::Idle,
        None,
        "confirmed"
    ));
}

#[test]
fn live_prepare_script_trade_loads_the_file_fixture_and_injects_partner() {
    let iso = IsolatedEnv::enter("trade-live");
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = Session::new();
    s.js = script::JsLibrary::with_cache(iso.dir.join("js-scripts.json"), iso.dir.join("js-cache"));
    s.live_prepare_script(scenario::get("script_trade").expect("registered"))
        .expect("prepare");
    assert_eq!(
        s.script_sel,
        Some(script::ScriptSel::Loaded(
            script::ScriptSource::File,
            "trade_bot".into()
        )),
        "script_trade loads the in-tree TradeBot fixture as File"
    );
    let names: Vec<_> = s
        .vault
        .as_ref()
        .expect("live vault")
        .profiles()
        .map(|p| p.username.clone())
        .collect();
    assert_eq!(names.len(), 2, "script_trade is a two-profile fleet");
    let pending = s.pending_script.lock().unwrap().clone();
    assert_eq!(
        pending.len(),
        2,
        "StartScript loads TradeBot on both fleet slots"
    );
    assert_eq!(pending[0].slot, names[0], "driven slot is first start");
    assert_eq!(pending[1].slot, names[1], "companion slot is second start");
    assert_eq!(
        pending[0]
            .bag
            .as_ref()
            .expect("driven partner inject")
            .get("partner"),
        Some(&serde_json::json!(client::util::JString::to_screen_name(
            &names[1]
        ))),
        "driven partner is the companion as the game shows it"
    );
    assert_eq!(
        pending[1]
            .bag
            .as_ref()
            .expect("companion partner inject")
            .get("partner"),
        Some(&serde_json::json!(client::util::JString::to_screen_name(
            &names[0]
        ))),
        "companion partner is the driven player as the game shows it"
    );
    assert!(
        names.iter().all(|name| name.contains('_')),
        "minted names carry the underscore the screen name replaces: {names:?}"
    );
    assert!(s.multibox, "fleet opens the MultiBox wall");
}

/// Thiever SETTINGS often fail to parse (`loadout: LOADOUT_SETTING`
/// stops the object walk), so the card schema is empty. The live
/// inject still has to post `target: Guard` or `str('target', 'Man')`
/// camps Men.
#[test]
fn live_prepare_thiever_posts_guard_target_when_schema_empty() {
    let iso = IsolatedEnv::enter("thiever-bag");
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let root = write_looping_catalog(&iso.dir, &[("Thiever", "ThievingBot")]);
    iso.set_rs2b0t(&root);
    let mut s = Session::new();
    s.live_prepare_script(scenario::get("thiever").expect("registered"))
        .expect("prepare");
    let pending = s.pending_script.lock().unwrap();
    let card = pending.first().expect("catalog start stashed");
    let bag = card
        .bag
        .clone()
        .expect("inject bag is posted even when the card schema is empty");
    assert_eq!(
        bag.get("target"),
        Some(&serde_json::json!("Guard")),
        "thiever inject must beat the Man fallback"
    );
    assert_eq!(bag.get("loot"), Some(&serde_json::json!("")));
    assert_eq!(bag.get("loadout"), Some(&serde_json::json!("Memory food")));
    assert_eq!(bag.get("banking"), Some(&serde_json::json!("Auto")));
    assert_eq!(bag.get("foodWithdraw"), Some(&serde_json::json!(22.0)));
    assert_eq!(bag.get("bankAtFood"), Some(&serde_json::json!(3.0)));
    assert_eq!(
        card.loadouts,
        vec![script::Loadout::new("Memory food").with_carry("Lobster", 1)],
        "harness Start must stash the fixture-owned loadout, not operator loadouts.json"
    );
}

#[test]
fn live_prepare_script_sets_script_sel_for_catalog_start_script() {
    let iso = IsolatedEnv::enter("alcher-sel");
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let root = write_looping_catalog(&iso.dir, &[("Alcher", "Alcher")]);
    iso.set_rs2b0t(&root);
    let mut s = Session::new();
    let mut scenario = scenario::get("alcher").expect("registered");
    scenario.settings.start_script = Some("Alcher");
    s.live_prepare_script(scenario).expect("prepare");
    assert_eq!(
        s.script_sel,
        Some(script::ScriptSel::Loaded(
            script::ScriptSource::Catalog,
            "Alcher".into()
        )),
        "live_prepare_script must set script_sel to the catalog card"
    );
}

#[test]
fn live_prepare_sherlock_starts_the_compiled_card_without_catalog() {
    let iso = IsolatedEnv::enter("sherlock-live");
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = Session::new();
    s.live_prepare_script(scenario::get("sherlock_talk").expect("registered"))
        .expect("prepare compiled Sherlock without $RS2B0T");
    assert_eq!(
        s.script_sel,
        Some(script::ScriptSel::Compiled(script::CompiledId("Sherlock"))),
        "live_prepare must select the compiled registry card"
    );
    let pending = s.pending_script.lock().unwrap();
    let card = pending.first().expect("compiled start stashed");
    assert_eq!(card.compiled, Some(script::CompiledId("Sherlock")));
    assert!(
        card.js.is_empty(),
        "compiled Start must not stash catalog JS"
    );
    let _ = iso;
}

#[test]
fn live_prepare_script_enables_multibox_for_a_fleet_only() {
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = Session::new();
    let fleet = scenario::get("nav_door").expect("nav_door is registered");
    assert!(
        fleet.seed.profiles.len() > 1,
        "nav_door is a two-profile fleet"
    );
    s.live_prepare_script(fleet).expect("prepare");
    assert!(
        s.multibox,
        "a fleet (2+ seed profiles) opens the MultiBox wall"
    );
    assert!(
        s.focus.lock().unwrap().wall_open,
        "multibox mirrors onto the focus so every bot rasters"
    );
    // Both fleet slots are minted fresh accounts, not `test`/`test2`.
    let vault_names: Vec<String> = s
        .vault
        .as_ref()
        .expect("live vault open")
        .profiles()
        .map(|p| p.username.clone())
        .collect();
    assert_eq!(vault_names.len(), 2, "a fleet mints both slots");
    for name in &vault_names {
        assert_ne!(name, "test", "live must not log in `test`");
        assert!(
            s.wall.members.iter().any(|m| m == name),
            "every minted profile is a wall member"
        );
    }
    assert!(!s.wall.chooser_open, "live keeps the chooser closed");
    // No `stop_slot` joins (see `flat_model_spawns_every_member_as_a_client`).

    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = Session::new();
    let solo = scenario::get("walk").expect("walk is registered");
    assert_eq!(solo.seed.profiles.len(), 1);
    s.live_prepare_script(solo).expect("prepare");
    assert!(!s.multibox, "a solo scenario keeps the single-bot boot");
    assert!(
        !s.focus.lock().unwrap().wall_open,
        "no wall members, no extra rasters"
    );
    // No `stop_slot` joins (see `flat_model_spawns_every_member_as_a_client`).
}

#[test]
fn null_raster_live_entries_prod_refuses_username_as_password() {
    let entries = null_raster_live_entries_for_target(client::BotTarget::Prod);
    for (user, pass) in &entries {
        assert_ne!(
            user, pass,
            "prod null_raster must not store username-as-password"
        );
    }
}

#[test]
fn null_raster_live_entries_local_allows_username_as_password() {
    let entries = null_raster_live_entries_for_target(client::BotTarget::Local);
    for (user, pass) in &entries {
        assert_eq!(user, pass, "local null_raster keeps username-as-password");
    }
}

#[test]
fn stress_live_entries_prod_refuses_username_as_password() {
    let entries = stress_live_entries_for_target(3, client::BotTarget::Prod);
    for (user, pass) in &entries {
        assert_ne!(
            user, pass,
            "prod stress must not store username-as-password"
        );
    }
}

#[test]
fn stress_live_entries_local_allows_username_as_password() {
    let entries = stress_live_entries_for_target(3, client::BotTarget::Local);
    for (user, pass) in &entries {
        assert_eq!(user, pass, "local stress keeps username-as-password");
    }
}

#[test]
fn temp_live_vault_prod_mint_does_not_persist_username_as_password() {
    let names = host_play::mint_live_names(2);
    let entries = host_play::mint_live_entries_for_target(&names, client::BotTarget::Prod);
    let pass = host_play::live_vault_passphrase_for(client::BotTarget::Prod);
    let path = temp_live_vault_from(&entries, 274_000_001, &pass, true);
    let vault = Vault::unlock(&path, &pass).unwrap();
    for p in vault.profiles() {
        assert_ne!(
            p.username, p.password,
            "prod live temp vault must not store username-as-password"
        );
    }
}

#[test]
fn live_prepare_script_never_upserts_the_operator_vault() {
    crate::ui_state::save(&crate::ui_state::PanelUiState::default());
    // The operator vault must stay byte-identical across a live boot:
    // `--live` unlocks only the throwaway temp vault.
    fn stamp(p: &std::path::Path) -> Option<(std::time::SystemTime, u64)> {
        std::fs::metadata(p)
            .ok()
            .map(|m| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len()))
    }
    let op = crate::session::default_vault_path();
    let before = stamp(&op);
    let mut s = Session::new();
    s.live_prepare_script(scenario::get("walk").unwrap())
        .expect("prepare");
    let after = stamp(&op);
    assert_eq!(
        before,
        after,
        "--live must never create or touch the operator vault at {}",
        op.display()
    );
    assert!(!s.persist_ui, "live boot keeps persist_ui off");
}

#[test]
fn focused_lowmem_follows_the_spawned_slot_not_a_session_leftover() {
    let path = tmp_vault("mem-gate.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.select("alice");
    assert!(s.focused_lowmem(), "throwaway profile defaults lowmem");
    // The slot's Music/SFX gate drives `Client.config.lowmem`; the HUD
    // must follow it even when `ui.lowmem` still reads the profile
    // default (the "lowmem while audio plays" bug).
    s.audio.set_music("alice", true);
    assert!(
        !s.focused_lowmem(),
        "status follows the slot, not ui.lowmem"
    );
    assert!(s.ui.lowmem, "ui.lowmem is a separate session leftover");
    s.audio.set_music("alice", false);
    assert!(s.focused_lowmem());
}

#[test]
fn live_prepare_script_does_not_write_last_focus() {
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: Some("alice".into()),
        ..Default::default()
    });
    let mut s = Session::new();
    let fleet = scenario::get("nav_door").expect("nav_door");
    s.live_prepare_script(fleet).expect("prepare");
    assert_eq!(
        crate::ui_state::load().last_focus.as_deref(),
        Some("alice"),
        "live boot must not clobber the operator last profile"
    );
}

#[test]
fn live_prepare_nav_door_applies_full_rate_and_leaves_sidecar_off() {
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = Session::new();
    let fleet = scenario::get("nav_door").expect("nav_door");
    s.live_prepare_script(fleet).expect("prepare");
    let f = s.focus.lock().unwrap();
    assert!(f.live_full_rate);
    assert!(!f.only_render_selected, "closer must paint");
    assert!(!f.capture);
    assert!(f.renderer);
    assert!(!f.sidecar_50, "sidecar stays the operator knob");
}

#[test]
fn live_prepare_nav_full_runner_already_has_deadline_and_shot() {
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = Session::new();
    s.live_prepare_script(scenario::get("nav_full").unwrap())
        .expect("prepare");
    let runner = s.scenario.lock().unwrap();
    let runner = runner.as_ref().expect("runner");
    assert_eq!(runner.deadline(), Duration::from_secs(360));
    assert_eq!(runner.terminal_shot(), Some("nav_full terminal"));
}

#[test]
fn live_prepare_smoke_runner_already_has_the_300s_deadline() {
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = Session::new();
    s.live_prepare_script(scenario::get("render_smoke").unwrap())
        .expect("prepare");
    let runner = s.scenario.lock().unwrap();
    let runner = runner.as_ref().expect("runner");
    assert_eq!(runner.deadline(), Duration::from_secs(300));
}

#[test]
fn requested_nav_paints_survive_scenario_install_without_changing_gameplay_or_prefs() {
    let saved = crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    };
    crate::ui_state::save(&saved);
    for choice in [None, Some(true), Some(false)] {
        let mut session = Session::new();
        session.set_nav_paints_override(choice);
        let mut scenario = scenario::get("nav_door").unwrap();
        scenario.settings.nav.allow_teleports = true;
        scenario.settings.nav.allow_wilderness = true;
        let baseline = crate::nav_settings::from_scenario(&scenario.settings.nav);
        let deadline = scenario.settings.deadline;
        let terminal_shot = scenario.settings.terminal_shot;
        session.live_prepare_script(scenario).unwrap();
        session.pump_status();
        let published = session.nav_publish.lock().unwrap().settings.clone();
        assert_eq!(published, session.effective_nav());
        assert!(published.allow_teleports && published.allow_wilderness);
        assert_eq!(published.allow_bank_fetch, baseline.allow_bank_fetch);
        assert_eq!(published.color_path, baseline.color_path);
        match choice {
            Some(on) => {
                assert_eq!(published.show_nav_path, on);
                assert_eq!(published.collision_fill, on);
                assert_eq!(published.client_trail, on);
            }
            None => assert_eq!(published, baseline),
        }
        let runner = session.scenario.lock().unwrap();
        let runner = runner.as_ref().unwrap();
        assert_eq!(
            runner.deadline(),
            scenario::budget_s_from_env().unwrap_or(deadline)
        );
        assert_eq!(runner.terminal_shot(), terminal_shot);
        assert_eq!(session.nav_overlay.as_ref(), Some(&baseline));
        assert_eq!(crate::ui_state::load().nav, saved.nav);
    }
}

#[test]
fn live_force_layers_does_not_write_panel_ui() {
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = Session::new();
    s.nav_overlay = Some(crate::nav_settings::from_scenario(
        &scenario::nav_test_paints(),
    ));
    assert!(
        s.effective_nav().show_nav_path,
        "the live overlay drives the effective paint layers at runtime"
    );
    // A save/load roundtrip of the panel prefs must not persist the
    // overlay bools: `nav_overlay` is session-only.
    let ui = crate::ui_state::load();
    assert!(
        !ui.nav.show_nav_path,
        "forced layers must never reach panel-ui.json"
    );
    let mut s = Session::new();
    s.live_prepare_script(scenario::get("nav_door").unwrap())
        .expect("prepare");
    assert!(
        s.nav_overlay
            .as_ref()
            .is_some_and(|n| n.show_nav_path && n.collision_fill && n.camera_follow),
        "nav scenario arms the live overlay"
    );
    let ui = crate::ui_state::load();
    assert!(
        !ui.nav.show_nav_path,
        "live boot of a nav scenario never writes the prefs"
    );
}

#[test]
fn live_full_rate_sync_raises_focus_and_members() {
    let mut s = Session::new();
    let a_in = SlotInput::new();
    let b_in = SlotInput::new();
    s.slots.insert(
        "a".into(),
        SlotIo {
            input: Arc::clone(&a_in),
            pixels: FrameBuf::new(),
        },
    );
    s.slots.insert(
        "b".into(),
        SlotIo {
            input: Arc::clone(&b_in),
            pixels: FrameBuf::new(),
        },
    );
    {
        let mut f = s.focus.lock().unwrap();
        f.focused = Some("a".into());
        f.only_render_selected = false;
        f.wall_open = true;
        f.wall = vec!["a".into(), "b".into()];
        f.renderer_by = std::collections::HashMap::from([("a".into(), true), ("b".into(), true)]);
        f.sidecar_50 = false;
        f.live_full_rate = true;
    }
    s.sync_sidecar_cadence();
    assert!(a_in.full_rate(), "focused slot is 50 fps via live overlay");
    assert!(b_in.full_rate(), "member is 50 fps via live overlay");
}

#[test]
fn queue_for_rejects_invalid_queue_tuple() {
    let mut s = Session::new();
    s.statuses.push(SlotStatus {
        username: "s00".into(),
        queue_position: 3,
        queue_total: 0,
        ..SlotStatus::default()
    });
    assert_eq!(s.queue_for("s00"), None);
}

#[test]
fn focus_first_profile_noop_when_empty() {
    let path = tmp_vault("focus-empty.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.focus_first_profile();
    assert!(s.focused_name().is_none());
}

#[test]
fn closing_game_pane_leaves_capture_pref_on() {
    let mut s = Session::new();
    s.select("alice");
    s.set_capture(true);
    assert!(s.focus.lock().unwrap().capture);
    s.set_game_pane_open(false);
    let f = s.focus.lock().unwrap();
    assert!(!f.game_pane_open);
    assert!(f.capture, "pane close must not clear the capture pref");
    assert!(
        s.capture_tx.is_none(),
        "drain drops while the pane is closed"
    );
}

#[test]
fn reopening_game_pane_resumes_capture_drain() {
    let mut s = Session::new();
    s.slots.insert(
        "alice".into(),
        SlotIo {
            input: SlotInput::new(),
            pixels: FrameBuf::new(),
        },
    );
    s.select("alice");
    s.set_capture(true);
    assert!(s.capture_tx.is_some());
    s.set_game_pane_open(false);
    assert!(s.capture_tx.is_none());
    s.set_game_pane_open(true);
    assert!(s.capture_tx.is_some(), "pref on resumes the drain");
}

#[test]
fn set_capture_persists_to_panel_ui() {
    crate::ui_state::save(&crate::ui_state::PanelUiState::default());
    let mut s = Session::new();
    s.set_capture(false);
    assert!(!crate::ui_state::load().capture);
    s.set_capture(true);
    assert!(crate::ui_state::load().capture);
}

#[test]
fn login_focuses_the_named_profile() {
    let mut s = Session::new();
    assert!(s.focused_name().is_none());
    s.login("alice");
    assert_eq!(s.focused_name().as_deref(), Some("alice"));
}

#[test]
fn login_after_logout_rearms_handshake_on_fake_arm() {
    // Logout latches + clears want_login; Log in must call arm_login_all
    // (clear latch, want_login, cancel want_logout) then select.
    let mut s = Session::new();
    let mut play = host_play::run_with_io(
        &host_play::PlayOptions {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    );
    let arm = SlotArm::new(7, false);
    arm.latch.store(true, Ordering::Relaxed);
    arm.want_login.store(false, Ordering::Relaxed);
    arm.want_logout.store(true, Ordering::Relaxed);
    play.attach_arm("alice", Arc::clone(&arm));
    s.play = Some(play);
    s.wall.load("alice");
    s.logout("alice");
    assert!(s.wall.latch.contains("alice"));

    s.login("alice");

    assert!(arm.want_login.load(Ordering::Relaxed));
    assert!(!arm.want_logout.load(Ordering::Relaxed));
    assert!(!arm.latch.load(Ordering::Relaxed));
    assert!(!s.wall.latch.contains("alice"));
    assert_eq!(s.focused_name().as_deref(), Some("alice"));
}

#[test]
fn arm_login_all_cancels_pending_logout() {
    // A title-screen member keeps want_logout=true (the slot body only
    // clears it when it observes ingame); Login all must cancel it or
    // the handshake would be undone on the first ingame frame.
    let arm = SlotArm::new(7, false);
    arm.latch.store(true, Ordering::Relaxed);
    arm.want_logout.store(true, Ordering::Relaxed);
    arm.arm_explicit_login();
    assert!(arm.want_login.load(Ordering::Relaxed));
    assert!(!arm.want_logout.load(Ordering::Relaxed));
    assert!(!arm.latch.load(Ordering::Relaxed));
}

#[test]
fn select_syncs_credentials_fields_from_focused_profile() {
    let path = tmp_vault("select-sync.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();

    s.select("alice");
    assert_eq!(s.cred_user, "alice");
    assert_eq!(s.cred_pass, "pw");
}

#[test]
fn save_credentials_upserts_under_username_key_keeping_uid() {
    let path = tmp_vault("save-creds.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "oldpass", 42))
        .unwrap();

    s.cred_user = "alice".into();
    s.cred_pass = "newpass".into();
    assert!(s.save_credentials());

    let p = s.vault.as_ref().unwrap().get("alice").unwrap();
    assert_eq!(p.password, "newpass");
    assert_eq!(p.uid, 42, "save must keep the existing uid");
}

#[test]
fn chooser_world_edit_persists_and_updates_running_slot() {
    let path = tmp_vault("world-choice.vault");
    let mut session = Session::new();
    session.vault = Some(Vault::create(&path, "bot").unwrap());
    session
        .vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    let mut play = host_play::run_with_io(
        &host_play::PlayOptions {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    );
    let arm = SlotArm::new(42, false);
    play.attach_arm("alice", Arc::clone(&arm));
    session.play = Some(play);
    session.begin_edit_profile(Some("alice"));
    assert_eq!(session.cred_settings.world, None);
    session.cred_settings.world = Some(2);
    assert!(session.save_credentials());
    assert_eq!(
        Vault::unlock(&path, "bot")
            .unwrap()
            .get("alice")
            .unwrap()
            .settings
            .world,
        Some(2)
    );
    assert_eq!(
        *arm.world.lock(),
        Some(2),
        "the profile-save path must update a running slot's next handshake"
    );
    session.cred_settings.world = None;
    session.cred_pass = "updated".into();
    assert!(session.save_credentials());
    assert_eq!(
        Vault::unlock(&path, "bot")
            .unwrap()
            .get("alice")
            .unwrap()
            .settings
            .world,
        Some(2),
        "a credentials save outside the editor must not unpin an account"
    );
    assert_eq!(
        *arm.world.lock(),
        Some(2),
        "a non-editor save must preserve the running slot's pin"
    );
}

#[test]
fn save_credentials_creates_new_profile_when_username_is_new() {
    let path = tmp_vault("new-user.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();

    s.cred_user = "bob".into();
    s.cred_pass = "bobpass".into();
    assert!(s.save_credentials());
    assert_eq!(s.focused_name().as_deref(), Some("bob"));
    assert!(s.slots.contains_key("bob"));

    let p = s.vault.as_ref().unwrap().get("bob").unwrap();
    assert_eq!(p.password, "bobpass");
    assert_ne!(p.uid, 42, "a new profile gets a fresh uid");
    assert_eq!(
        s.vault.as_ref().unwrap().get("alice").unwrap().password,
        "pw",
        "saving a new username must not touch existing profiles"
    );
}

#[test]
fn save_credentials_rejects_empty_username() {
    let path = tmp_vault("empty-user.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.cred_user = "  ".into();
    s.cred_pass = "x".into();
    assert!(!s.save_credentials());
    assert!(s.error.is_some());
}

#[test]
fn save_credentials_without_focus_upserts_spawns_and_selects() {
    let path = tmp_vault("empty-first-run.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    assert!(s.focused_name().is_none());
    s.cred_user = "test".into();
    s.cred_pass = "test".into();
    assert!(s.save_credentials());
    assert!(s.vault.as_ref().unwrap().get("test").is_some());
    assert_eq!(s.focused_name().as_deref(), Some("test"));
    assert!(s.slots.contains_key("test"));
}

#[test]
fn save_credentials_does_not_duplicate_running_slot() {
    let path = tmp_vault("no-dup-slot.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.slots.insert(
        "alice".into(),
        SlotIo {
            input: SlotInput::new(),
            pixels: FrameBuf::new(),
        },
    );
    s.cred_user = "alice".into();
    s.cred_pass = "newpw".into();
    assert!(s.save_credentials());
    assert_eq!(s.slots.len(), 1);
    assert_eq!(s.focused_name().as_deref(), Some("alice"));
}

#[test]
fn save_credentials_rename_editing_profile_replaces_old_key() {
    let path = tmp_vault("rename-creds.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();

    s.begin_edit_profile(Some("alice"));
    s.cred_user = "bob".into();
    s.cred_pass = "bobpass".into();
    assert!(s.save_credentials());

    let vault = s.vault.as_ref().unwrap();
    assert!(vault.get("bob").is_some(), "rename must upsert the new key");
    assert!(
        vault.get("alice").is_none(),
        "rename must remove the old key"
    );
    assert_eq!(
        vault.get("bob").unwrap().uid,
        42,
        "rename keeps the old uid"
    );
    assert_eq!(s.focused_name().as_deref(), Some("bob"));
}

#[test]
#[cfg(unix)]
fn save_credentials_upsert_error_surfaces_on_session_error() {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!("274bot-panel-save-err-{}", std::process::id()));
    if dir.exists() {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.cred_user = "alice".into();
    s.cred_pass = "pw".into();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    assert!(!s.save_credentials());
    assert!(
        s.error
            .as_ref()
            .is_some_and(|e| e.starts_with("credentials:")),
        "upsert failure must land on session.error, got {:?}",
        s.error
    );
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn clear_credentials_empties_fields_but_keeps_vault() {
    let path = tmp_vault("clear-creds.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.select("alice");
    s.cred_pass = "edited".into();

    s.clear_credentials();
    assert!(s.cred_user.is_empty());
    assert!(s.cred_pass.is_empty());
    assert!(
        s.vault.as_ref().unwrap().get("alice").is_some(),
        "clear must not delete the vault profile"
    );
}

#[test]
fn begin_edit_profile_loads_fields_and_opens_chooser() {
    let path = tmp_vault("edit-profile.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "secret", 42))
        .unwrap();

    s.begin_edit_profile(Some("alice"));
    assert!(s.wall.chooser_open);
    assert_eq!(s.chooser_edit.as_deref(), Some("alice"));
    assert_eq!(s.cred_user, "alice");
    assert_eq!(s.cred_pass, "secret");

    s.begin_edit_profile(None);
    assert_eq!(s.chooser_edit.as_deref(), Some(""));
    assert!(s.cred_user.is_empty());
    assert!(s.cred_pass.is_empty());

    s.cancel_edit_profile();
    assert!(s.chooser_edit.is_none());
    assert!(s.wall.chooser_open, "cancel leaves the picker open");
}

#[test]
fn begin_edit_profile_loads_guardian_settings() {
    let path = tmp_vault("edit-guardian.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    let mut p = profile("alice", "secret", 42);
    p.settings.auto_login = true;
    p.settings.lamp_auto = false;
    p.settings.lamp_skill = "magic".into();
    p.settings.random_events = false;
    s.vault.as_mut().unwrap().upsert(p).unwrap();

    s.begin_edit_profile(Some("alice"));
    assert!(s.cred_settings.auto_login);
    assert!(!s.cred_settings.lamp_auto);
    assert!(!s.cred_settings.random_events);
    assert_eq!(s.cred_settings.lamp_skill, "magic");

    s.begin_edit_profile(None);
    assert!(!s.cred_settings.auto_login);
    assert!(s.cred_settings.lamp_auto);
    assert!(s.cred_settings.random_events);
    assert_eq!(s.cred_settings.lamp_skill, "strength");
}

#[test]
fn set_multibox_off_cancels_picker_edit() {
    let path = tmp_vault("edit-off.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.begin_edit_profile(Some("alice"));
    s.set_multibox(true);
    s.set_multibox(false);
    assert!(s.chooser_edit.is_none());
    assert!(!s.wall.chooser_open);
}

#[test]
fn set_multibox_wires_wall_open_and_off_clears_grid() {
    let mut s = Session::new();
    assert!(!s.multibox);
    assert!(!s.focus.lock().unwrap().wall_open);
    s.set_multibox(true);
    assert!(s.multibox);
    assert!(
        s.focus.lock().unwrap().wall_open,
        "rail or grid: wall is open"
    );
    s.set_grid(true);
    assert!(s.wall.grid);
    assert!(
        s.focus.lock().unwrap().wall_open,
        "grid is a submode of MultiBox; wall_open stays on"
    );
    s.set_multibox(false);
    assert!(!s.multibox);
    assert!(!s.focus.lock().unwrap().wall_open, "extra rasters stop");
    assert!(!s.wall.grid, "MultiBox off clears grid");
}

#[test]
fn set_grid_is_noop_while_multibox_off() {
    let mut s = Session::new();
    s.set_grid(true);
    assert!(!s.wall.grid);
}

#[test]
fn multibox_on_never_latches_a_tv_mode() {
    let mut s = Session::new();
    s.set_multibox(true);
    s.set_grid(true);
    assert!(s.wall.grid, "every member is a full Client; grid works");
    s.set_multibox(false);
    assert!(!s.wall.grid);
}

#[test]
fn set_auto_login_upserts_without_spawning() {
    let path = tmp_vault("auto-login.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    assert!(s.set_auto_login("alice", true));
    assert!(
        s.vault
            .as_ref()
            .unwrap()
            .get("alice")
            .unwrap()
            .settings
            .auto_login
    );
    assert!(s.slots.is_empty(), "set_auto_login must not spawn a slot");
    assert!(s.set_auto_login("alice", false));
    assert!(
        !s.vault
            .as_ref()
            .unwrap()
            .get("alice")
            .unwrap()
            .settings
            .auto_login
    );
}

#[test]
fn set_random_settings_upserts_all_three_fields_without_spawning() {
    let path = tmp_vault("random-settings.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    assert!(s.set_random_settings("alice", false, "magic", false));
    let p = s.vault.as_ref().unwrap().get("alice").unwrap();
    assert!(!p.settings.random_events, "random events persist off");
    assert_eq!(p.settings.lamp_skill, "magic");
    assert!(!p.settings.lamp_auto, "lamp auto persists off");
    assert!(
        s.slots.is_empty(),
        "set_random_settings must not spawn a slot"
    );
    assert!(s.set_random_settings("alice", true, "strength", true));
    let p = s.vault.as_ref().unwrap().get("alice").unwrap();
    assert!(p.settings.random_events);
    assert_eq!(p.settings.lamp_skill, "strength");
    assert!(p.settings.lamp_auto);
}

#[test]
fn set_random_settings_mirrors_running_arm() {
    let path = tmp_vault("random-settings-arm.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    let mut play = host_play::run_with_io(
        &host_play::PlayOptions {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    );
    let arm = SlotArm::new(42, false);
    assert!(arm.random_events.load(Ordering::Relaxed), "default on");
    assert!(
        arm.lamp_auto.load(Ordering::Relaxed),
        "default lamp auto on"
    );
    assert_eq!(
        arm.lamp_skill.lock().unwrap().as_str(),
        "strength",
        "default lamp skill"
    );
    play.attach_arm("alice", Arc::clone(&arm));
    s.play = Some(play);
    assert!(s.set_random_settings("alice", false, "attack", true));
    assert!(
        !arm.random_events.load(Ordering::Relaxed),
        "toggle off must reach the live arm without respawn"
    );
    assert!(
        arm.lamp_auto.load(Ordering::Relaxed),
        "lamp auto must reach the live arm without respawn"
    );
    assert_eq!(
        arm.lamp_skill.lock().unwrap().as_str(),
        "attack",
        "lamp skill must reach the live arm without respawn"
    );
    assert!(s.set_random_settings("alice", true, "strength", true));
    assert!(arm.random_events.load(Ordering::Relaxed));
    assert!(arm.lamp_auto.load(Ordering::Relaxed));
    assert_eq!(arm.lamp_skill.lock().unwrap().as_str(), "strength");
}

#[test]
fn set_random_settings_rejects_unknown_or_locked_vault() {
    let path = tmp_vault("random-settings-unknown.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    assert!(
        !s.set_random_settings("nobody", false, "magic", false),
        "unknown profile must fail"
    );
    let mut s = Session::new();
    assert!(
        !s.set_random_settings("alice", false, "magic", false),
        "locked vault must fail"
    );
    assert!(s.error.is_some());
}

#[test]
fn music_sfx_persists_lowmem_false() {
    let path = tmp_vault("music-sfx.vault");
    let mut s = Session::new();
    assert!(s.focused_lowmem(), "no focused profile defaults to lowmem");
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.select("alice");
    assert!(s.focused_lowmem(), "fresh profile defaults to lowmem");
    assert!(s.set_focused_lowmem(false));
    assert!(
        !s.vault
            .as_ref()
            .unwrap()
            .get("alice")
            .unwrap()
            .settings
            .lowmem
    );
    assert!(!s.focused_lowmem(), "focused profile reflects the setting");
    assert!(s.set_focused_lowmem(true));
    assert!(
        s.vault
            .as_ref()
            .unwrap()
            .get("alice")
            .unwrap()
            .settings
            .lowmem
    );
}

#[test]
fn set_auto_login_mirrors_running_arm() {
    let path = tmp_vault("auto-login-arm.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    let mut play = host_play::run_with_io(
        &host_play::PlayOptions {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    );
    let arm = SlotArm::new(42, false);
    play.attach_arm("alice", Arc::clone(&arm));
    s.play = Some(play);
    assert!(s.set_auto_login("alice", true));
    assert!(arm.auto_login.load(Ordering::Relaxed));
    assert!(s.set_auto_login("alice", false));
    assert!(!arm.auto_login.load(Ordering::Relaxed));
}

#[test]
fn set_renderer_writes_renderer_by_for_focused() {
    let mut s = Session::new();
    s.focus.lock().unwrap().focused = Some("alice".into());
    s.set_renderer(false);
    let f = s.focus.lock().unwrap();
    assert!(!f.renderer);
    assert_eq!(f.renderer_by.get("alice").copied(), Some(false));
    drop(f);
    s.set_renderer(true);
    let f = s.focus.lock().unwrap();
    assert!(f.renderer);
    assert_eq!(f.renderer_by.get("alice").copied(), Some(true));
}

#[test]
fn raster_persists_and_off_keeps_prefer_cpu_until_cpu() {
    let path = tmp_vault("raster-mode.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.select("alice");
    assert_eq!(s.focused_raster(), vault::RasterMode::Gpu);
    assert!(!s.slots.get("alice").unwrap().input.prefer_cpu());
    assert!(s.set_focused_raster(vault::RasterMode::Off));
    assert_eq!(s.focused_raster(), vault::RasterMode::Off);
    assert_eq!(
        s.focus.lock().unwrap().renderer_by.get("alice").copied(),
        Some(false)
    );
    assert!(
        !s.slots.get("alice").unwrap().input.prefer_cpu(),
        "Off must not flip the GPU/CPU latch"
    );
    assert!(s.set_focused_raster(vault::RasterMode::Cpu));
    assert_eq!(s.focused_raster(), vault::RasterMode::Cpu);
    assert!(s.slots.get("alice").unwrap().input.prefer_cpu());
    assert_eq!(
        s.vault
            .as_ref()
            .unwrap()
            .get("alice")
            .unwrap()
            .settings
            .raster,
        vault::RasterMode::Cpu
    );
}

#[test]
fn raster_switch_confirm_only_when_backend_changes_on_spawned_slot() {
    use vault::RasterMode::*;
    // GPU↔CPU on a spawned slot is a drop+reattach (the Client and its
    // socket stay), so no logout/restart confirm is ever required —
    // not even Off↔Gpu/Cpu or a spawned slot.
    assert!(!Session::raster_switch_needs_confirm(Off, false, true));
    assert!(!Session::raster_switch_needs_confirm(Gpu, false, true));
    assert!(!Session::raster_switch_needs_confirm(Cpu, false, true));
    assert!(!Session::raster_switch_needs_confirm(Gpu, true, true));
    assert!(!Session::raster_switch_needs_confirm(Cpu, true, true));
    assert!(!Session::raster_switch_needs_confirm(Cpu, false, false));
    assert_eq!(Session::mem_status_text(true), "lowmem");
    assert_eq!(Session::mem_status_text(false), "highmem");
}

#[test]
fn request_raster_cpu_on_spawned_slot_applies_immediately() {
    let path = tmp_vault("raster-no-confirm.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.select("alice");
    s.request_focused_raster(vault::RasterMode::Cpu);
    assert_eq!(
        s.focused_raster(),
        vault::RasterMode::Cpu,
        "GPU→CPU applies at once: drop+reattach, never a logout"
    );
    s.request_focused_raster(vault::RasterMode::Off);
    assert_eq!(s.focused_raster(), vault::RasterMode::Off);
    assert!(s.slots.contains_key("alice"), "Off must keep the slot");
}

#[test]
fn request_focused_lowmem_applies_without_confirm_and_keeps_slot() {
    let path = tmp_vault("mem-no-confirm.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.select("alice");
    s.request_focused_lowmem(false);
    assert!(!s.focused_lowmem(), "mem flip applies at once");
    assert!(s.slots.contains_key("alice"), "mem flip must keep the slot");
}

#[test]
fn raster_switch_keeps_slot_frame_buf_and_input() {
    let path = tmp_vault("raster-no-restart.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.select("alice");
    let slot = s.slots.get("alice").expect("select spawns the slot");
    let buf = Arc::clone(&slot.pixels);
    let inp = Arc::clone(&slot.input);
    assert!(s.set_focused_raster(vault::RasterMode::Cpu));
    assert!(inp.prefer_cpu(), "GPU→CPU sets the slot's prefer_cpu latch");
    assert!(s.set_focused_raster(vault::RasterMode::Gpu));
    assert!(!inp.prefer_cpu(), "CPU→GPU clears the prefer_cpu latch");
    let slot = s.slots.get("alice").expect("GPU↔CPU must keep the slot");
    assert!(
        Arc::ptr_eq(&slot.pixels, &buf),
        "a GPU↔CPU flip must keep the same FrameBuf (no restart)"
    );
    assert!(
        Arc::ptr_eq(&slot.input, &inp),
        "a GPU↔CPU flip must keep the same SlotInput (no restart)"
    );
}

#[test]
fn lowmem_flip_keeps_slot_frame_buf_and_input() {
    let path = tmp_vault("mem-no-restart.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.select("alice");
    let slot = s.slots.get("alice").expect("select spawns the slot");
    let buf = Arc::clone(&slot.pixels);
    let inp = Arc::clone(&slot.input);
    assert!(s.set_focused_lowmem(false));
    assert!(s.set_focused_lowmem(true));
    let slot = s.slots.get("alice").expect("a mem flip must keep the slot");
    assert!(
        Arc::ptr_eq(&slot.pixels, &buf),
        "a mem flip must keep the same FrameBuf (no restart)"
    );
    assert!(
        Arc::ptr_eq(&slot.input, &inp),
        "a mem flip must keep the same SlotInput (no restart)"
    );
    assert!(s.focused_lowmem());
}

#[test]
fn arm_for_profile_respects_auto_login_and_latch() {
    let path = tmp_vault("arm-for-profile.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    let mut p = profile("alice", "pw", 42);
    p.settings.auto_login = true;
    s.vault.as_mut().unwrap().upsert(p).unwrap();
    let arm = s.arm_for_profile("alice").expect("arm");
    assert!(arm.want_login.load(Ordering::Relaxed));
    assert!(arm.auto_login.load(Ordering::Relaxed));
    s.wall.latch_logout("alice");
    let arm = s.arm_for_profile("alice").expect("arm");
    assert!(
        !arm.want_login.load(Ordering::Relaxed),
        "latch blocks handshake"
    );
    assert!(
        arm.auto_login.load(Ordering::Relaxed),
        "profile auto_login stays on the arm"
    );
}

#[test]
fn set_auto_login_rejects_unknown_profile_without_spawning() {
    let path = tmp_vault("auto-login-missing.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    assert!(!s.set_auto_login("nobody", true));
    assert!(s.error.is_some(), "missing profile sets the banner");
    assert!(s.slots.is_empty());
}

#[test]
fn logout_latches_member_until_login_all() {
    let mut s = Session::new();
    s.wall.load("alice");
    s.logout("alice");
    assert!(s.wall.latch.contains("alice"), "intentional logout latches");
    assert!(!s.wall.should_auto_login("alice", true));
    s.login_all();
    assert!(
        !s.wall.latch.contains("alice"),
        "Login all clears the latch"
    );
}

#[test]
fn login_all_during_loading_scene_publishes_no_control_owned_place() {
    let mut s = Session::new();
    let mut play = empty_play();
    let alice = SlotArm::new(1, false);
    let bob = SlotArm::new(2, false);
    play.attach_arm("alice", Arc::clone(&alice));
    play.attach_arm("bob", Arc::clone(&bob));
    play.statuses.lock().unwrap().extend([
        SlotStatus {
            username: "alice".into(),
            startup_phase: StartupPhase::LoadingScene,
            ingame: false,
            ..SlotStatus::default()
        },
        SlotStatus {
            username: "bob".into(),
            ..SlotStatus::default()
        },
    ]);
    for name in ["alice", "bob"] {
        s.wall.load(name);
        s.slots.insert(
            name.into(),
            SlotIo {
                input: SlotInput::new(),
                pixels: FrameBuf::new(),
            },
        );
    }
    s.focus.lock().unwrap().focused = Some("alice".into());
    s.play = Some(play);

    s.login_all();

    assert!(alice.want_login.load(Ordering::Relaxed));
    assert!(bob.want_login.load(Ordering::Relaxed));
    assert!(
        s.play.as_ref().unwrap().login_queue_uids().is_empty(),
        "only worker Queueing transitions create FIFO membership"
    );
}

#[test]
fn focused_ingame_is_false_without_status() {
    let mut s = Session::new();
    s.focus.lock().unwrap().focused = Some("alice".into());
    assert!(!s.focused_ingame());
    s.statuses.push(SlotStatus {
        username: "alice".into(),
        ingame: true,
        ..SlotStatus::default()
    });
    assert!(s.focused_ingame());
}

#[test]
fn queue_for_tracks_each_named_status_independent_of_focus() {
    let mut s = Session::new();
    s.focus.lock().unwrap().focused = Some("alice".into());
    assert_eq!(s.queue_for("alice"), None, "not queued by default");
    s.statuses.push(SlotStatus {
        username: "alice".into(),
        queue_position: 2,
        queue_total: 3,
        ..SlotStatus::default()
    });
    s.statuses.push(SlotStatus {
        username: "bob".into(),
        queue_position: 1,
        queue_total: 2,
        ..SlotStatus::default()
    });
    assert_eq!(s.queue_for("alice"), Some((2, 3)));
    assert_eq!(s.queue_for("bob"), Some((1, 2)));

    s.focus.lock().unwrap().focused = Some("bob".into());
    assert_eq!(s.queue_for("alice"), Some((2, 3)));
    assert_eq!(s.queue_for("bob"), Some((1, 2)));
}

#[test]
fn load_and_rail_remove_sync_focus_wall() {
    let path = tmp_vault("focus-wall-sync.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("bob", "pw", 43))
        .unwrap();
    s.load("alice");
    s.load("bob");
    assert_eq!(
        s.focus.lock().unwrap().wall,
        vec!["alice".to_string(), "bob".to_string()],
        "membership mirrors into Focus.wall for draw_for_slot"
    );
    assert_eq!(s.focused_name().as_deref(), Some("bob"));
    s.rail_remove("bob");
    assert_eq!(
        s.focus.lock().unwrap().wall,
        vec!["alice".to_string()],
        "rail ✕ drops the name from Focus.wall too"
    );
    assert_eq!(
        s.focused_name().as_deref(),
        Some("alice"),
        "rail ✕ focuses the neighbour when the focused member is removed"
    );
}

#[test]
fn rail_remove_clears_focus_when_last_member() {
    let path = tmp_vault("rail-remove-last.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.load("alice");
    assert_eq!(s.focused_name().as_deref(), Some("alice"));
    s.rail_remove("alice");
    assert!(s.focused_name().is_none());
    assert!(s.wall.members.is_empty());
}

#[test]
fn set_multibox_on_syncs_focus_wall() {
    let mut s = Session::new();
    s.set_multibox(true);
    assert_eq!(
        s.focus.lock().unwrap().wall,
        s.wall.members,
        "the seed path (running slots) mirrors into Focus.wall too"
    );
    s.set_multibox(false);
    assert_eq!(s.focus.lock().unwrap().wall, s.wall.members);
}

#[test]
fn load_all_loads_vault_profiles_and_syncs_focus_wall() {
    let path = tmp_vault("load-all.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("bob", "pw", 43))
        .unwrap();
    s.load("alice");
    let added = s.load_all();
    assert_eq!(added, 1, "only bob is new");
    assert_eq!(s.wall.members, vec!["alice".to_string(), "bob".to_string()]);
    assert_eq!(s.focus.lock().unwrap().wall, s.wall.members);
}

#[test]
fn chooser_vault_remove_keeps_wall_member_and_slot() {
    let path = tmp_vault("chooser-remove.vault");
    let mut s = Session::new();
    s.vault = Some(Vault::create(&path, "bot").unwrap());
    s.vault
        .as_mut()
        .unwrap()
        .upsert(profile("alice", "pw", 42))
        .unwrap();
    s.load("alice");
    assert!(s.vault_remove("alice"), "chooser ✕ deletes the vault row");
    assert!(
        s.vault.as_ref().unwrap().get("alice").is_none(),
        "profile row gone from the vault"
    );
    assert_eq!(
        s.wall.members,
        vec!["alice".to_string()],
        "chooser ✕ must not rail_remove a live member"
    );
    assert!(s.slots.contains_key("alice"), "slot stays up");
    assert!(
        s.focus.lock().unwrap().wall.contains(&"alice".to_string()),
        "Focus.wall still lists the member"
    );
}

#[test]
fn script_active_matches_rs2b0t() {
    assert!(script_active(script::RunState::Starting));
    assert!(script_active(script::RunState::Running));
    assert!(script_active(script::RunState::Paused));
    assert!(script_active(script::RunState::Stopping));
    assert!(!script_active(script::RunState::Idle));
    assert!(!script_active(script::RunState::Error));
}

#[test]
fn script_pause_resume_stop_enable_rules() {
    assert!(script_pause_enabled(script::RunState::Starting));
    assert!(script_pause_enabled(script::RunState::Running));
    assert!(script_pause_enabled(script::RunState::Paused));
    assert!(!script_pause_enabled(script::RunState::Idle));
    assert!(!script_pause_enabled(script::RunState::Stopping));
    assert!(!script_pause_enabled(script::RunState::Error));
    assert!(script_stop_enabled(script::RunState::Starting));
    assert!(script_stop_enabled(script::RunState::Running));
    assert!(script_stop_enabled(script::RunState::Paused));
    assert!(!script_stop_enabled(script::RunState::Stopping));
    assert!(!script_stop_enabled(script::RunState::Idle));
    assert!(!script_stop_enabled(script::RunState::Error));
}

#[test]
fn script_status_text_matches_rs2b0t_labels() {
    assert_eq!(script_status_text(script::RunState::Idle), "idle");
    assert_eq!(script_status_text(script::RunState::Starting), "starting");
    assert_eq!(script_status_text(script::RunState::Running), "running");
    assert_eq!(script_status_text(script::RunState::Paused), "paused");
    assert_eq!(script_status_text(script::RunState::Stopping), "stopping");
    assert_eq!(script_status_text(script::RunState::Error), "error");
}

#[test]
fn script_start_selected_unported_id_reports_not_ported() {
    let mut s = Session::new();
    let mut play = empty_play();
    play.attach_arm("alice", SlotArm::new(42, false));
    s.play = Some(play);
    s.focus.lock().unwrap().focused = Some("alice".into());
    s.script_sel = Some(script::ScriptSel::Compiled(script::CompiledId(
        "BoneBurier",
    )));
    s.script_start_selected();
    let err = s.error.clone().expect("not-ported message");
    assert!(err.contains("not ported"), "{err}");
    assert_eq!(s.focused_script_state(), script::RunState::Idle);
}

#[test]
fn load_js_registers_card_selects_and_persists_to_the_session_store() {
    let dir =
        std::env::temp_dir().join(format!("274bot-panel-session-load-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = dir.join("js-scripts.json");
    let path = dir.join("tickbot.js");
    std::fs::write(
        &path,
        "export function tick(api) { api._n = (api._n||0)+1 }",
    )
    .unwrap();

    let mut s = Session::new();
    s.js = script::JsLibrary::with_cache(store.clone(), dir.join("js-cache"));
    s.load_js(&path);
    assert_eq!(s.error, None, "load should succeed: {:?}", s.error);
    match &s.script_sel {
        Some(script::ScriptSel::Loaded(script::ScriptSource::File, id)) => {
            assert!(id.ends_with("tickbot.js") || id.contains("tickbot"), "{id}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(s.js.cards().len(), 1);
    assert!(!s.script_load_open, "success closes the load browser");
    assert!(store.exists(), "the card is persisted to the session store");

    // A path that is not a bot shape fails and keeps the error banner.
    let bad = dir.join("plain.js");
    std::fs::write(&bad, "const x = 1;").unwrap();
    s.load_js(&bad);
    assert!(s.error.as_deref().is_some_and(|e| e.contains("shape")));
}

#[test]
fn script_start_selected_refuses_without_selection_or_play() {
    let mut s = Session::new();
    s.script_start_selected();
    let err = s.error.clone().expect("no-focus banner");
    assert!(err.contains("focused"), "{err}");
    s.error = None;
    s.focus.lock().unwrap().focused = Some("alice".into());
    s.script_start_selected();
    let err = s.error.clone().expect("no-selection banner");
    assert!(
        err.contains("assignment") || err.contains("browse"),
        "{err}"
    );
    s.error = None;
    s.script_sel = Some(script::ScriptSel::Compiled(script::CompiledId(
        "BoneBurier",
    )));
    s.script_start_selected();
    let err = s.error.clone().expect("no-play banner");
    assert!(err.contains("play"), "{err}");
    assert_eq!(s.focused_script_state(), script::RunState::Idle);
}

#[test]
fn script_start_selected_refuses_unloadable_import() {
    let dir = std::env::temp_dir().join(format!("274bot-panel-unloadable-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ghost.js");
    std::fs::write(
            &path,
            "import x from '../../event/webwalk/Something.js';\nexport default class T extends LoopingBot { loop() {} }\n",
        )
        .unwrap();
    let mut s = Session::new();
    s.js = script::JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    let mut play = empty_play();
    play.attach_arm("alice", SlotArm::new(42, false));
    s.play = Some(play);
    s.focus.lock().unwrap().focused = Some("alice".into());
    s.load_js(&path);
    assert_eq!(
        s.error, None,
        "load still registers the card: {:?}",
        s.error
    );
    s.script_start_selected();
    let err = s.error.clone().expect("unloadable banner");
    assert!(
        err.contains("../../event/webwalk/Something.js"),
        "error must name the specifier: {err}"
    );
    assert_eq!(s.focused_script_state(), script::RunState::Idle);
}

#[test]
fn fill_rs2b0t_cards_once_happens_on_first_browse_not_session_new() {
    // `$RS2B0T` + HOME point at a fake catalog so the fill never
    // touches the operator's home. `Session::new()` must stay
    // catalog-free: the fill is first Load/Browse only, so panel unit
    // tests that merely construct a `Session` never parse a real
    // catalog or rewrite `~/.274bot/rs2b0t-path`.
    let iso = IsolatedEnv::enter("rs2b0t-fill");
    let root = write_looping_catalog(&iso.dir, &[("BoneBurier", "BoneBurier")]);
    iso.set_rs2b0t(&root);
    let mut s = Session::new();
    assert!(
        s.js.get(script::ScriptSource::Catalog, "BoneBurier")
            .is_none(),
        "Session::new must not parse $RS2B0T"
    );
    assert!(
        !iso.home.join(".274bot/rs2b0t-path").exists(),
        "Session::new must not rewrite the persisted root"
    );
    s.fill_rs2b0t_cards_once();
    let card =
        s.js.get(script::ScriptSource::Catalog, "BoneBurier")
            .expect("BoneBurier card filled on first Browse");
    assert_eq!(card.shape, script::LoadShape::CompatClass);
    assert!(
        iso.home.join(".274bot/rs2b0t-path").is_file(),
        "the first successful parse persists the root path"
    );
    assert_eq!(s.js.cards().len(), 1, "once-only fill");
    s.fill_rs2b0t_cards_once();
    assert_eq!(s.js.cards().len(), 1, "a second Browse does not re-parse");
}

fn fake_rs2b0t_tree(dir: &Path) -> PathBuf {
    let root = dir.join("rs2b0t");
    let scripts = root.join("src/bot/scripts");
    std::fs::create_dir_all(scripts.join("BoneBurier")).unwrap();
    std::fs::write(
        scripts.join("index.ts"),
        r#"
import BoneBurier from './BoneBurier/BoneBurier.js';
ScriptRegistry.register({
  name: 'BoneBurier',
  description: 'Buries bones',
  category: 'Prayer',
  tags: ['bones'],
  create: () => new BoneBurier(),
});
"#,
    )
    .unwrap();
    std::fs::write(
        scripts.join("BoneBurier/BoneBurier.ts"),
        "export default class BoneBurier extends LoopingBot { override loop() {} }",
    )
    .unwrap();
    root
}

#[test]
fn load_then_browse_still_prompts_without_rs2b0t_root() {
    let iso = IsolatedEnv::enter("load-browse");
    let mut s = Session::new();
    s.fill_rs2b0t_cards_once();
    s.on_script_browse_open();
    assert!(
        s.rs2b0t_catalog_open,
        "Load with no root must not skip the first Browse catalog prompt"
    );
    assert!(
        !iso.home.join(".274bot/rs2b0t-path").exists(),
        "Load with no root must not write rs2b0t-path"
    );
}

#[test]
fn defer_rs2b0t_catalog_leaves_no_path_and_zero_catalog_cards() {
    let iso = IsolatedEnv::enter("defer");
    let mut s = Session::new();
    s.on_script_browse_open();
    assert!(s.rs2b0t_catalog_open, "first browse opens folder picker");
    s.defer_rs2b0t_catalog();
    assert!(
        script::rs2b0t_import_deferred_at(&iso.home.join(".274bot/rs2b0t-import")),
        "defer flag written"
    );
    assert!(
        !iso.home.join(".274bot/rs2b0t-path").exists(),
        "defer must not write rs2b0t-path"
    );
    assert!(
        s.js.cards()
            .iter()
            .all(|c| c.source != script::ScriptSource::Catalog),
        "zero Catalog cards after defer"
    );
}

#[test]
fn import_rs2b0t_catalog_persists_path_and_registers_cards() {
    let iso = IsolatedEnv::enter("import");
    let root = fake_rs2b0t_tree(&iso.dir);
    let mut s = Session::new();
    let n = s.import_rs2b0t_catalog(&root).expect("import");
    assert_eq!(n, 1);
    assert!(iso.home.join(".274bot/rs2b0t-path").is_file());
    let card =
        s.js.get(script::ScriptSource::Catalog, "BoneBurier")
            .expect("catalog card");
    assert_eq!(card.category, "Prayer");
    assert_eq!(card.description, "Buries bones");
    assert_eq!(card.tags, vec!["bones"]);
}

#[test]
fn on_script_browse_open_with_rs2b0t_env_still_fills_catalog() {
    let iso = IsolatedEnv::enter("browse-env");
    let root = fake_rs2b0t_tree(&iso.dir);
    iso.set_rs2b0t(&root);
    let mut s = Session::new();
    s.on_script_browse_open();
    assert!(
        s.js.get(script::ScriptSource::Catalog, "BoneBurier")
            .is_some(),
        "RS2B0T env still fills catalog on first browse"
    );
    assert!(iso.home.join(".274bot/rs2b0t-path").is_file());
}

fn write_two_card_catalog(dir: &Path) -> PathBuf {
    write_looping_catalog(
        dir,
        &[("BoneBurier", "BoneBurier"), ("ShopRunner", "ShopRunner")],
    )
}

#[test]
fn select_script_card_queues_only_that_card_and_pumps_one_per_two_frames() {
    let iso = IsolatedEnv::enter("transpile-click");
    let root = write_two_card_catalog(&iso.dir);
    let mut s = Session::new();
    s.persist_ui = false;
    s.js = script::JsLibrary::with_cache(iso.dir.join("js-scripts.json"), iso.dir.join("js-cache"));
    s.js.register_rs2b0t(&root, &iso.dir.join("rs2b0t-path"))
        .expect("catalog");

    s.select_script_card(script::ScriptSource::Catalog, "BoneBurier");
    assert_eq!(
        s.script_sel,
        Some(script::ScriptSel::Loaded(
            script::ScriptSource::Catalog,
            "BoneBurier".into()
        ))
    );
    assert_eq!(s.transpile_queue.len(), 1);
    assert_eq!(
        s.transpile_queue.front().map(|q| q.1.as_str()),
        Some("BoneBurier")
    );
    assert!(
        s.js.get(script::ScriptSource::Catalog, "BoneBurier")
            .unwrap()
            .js
            .is_empty(),
        "click must not transpile on the same call"
    );
    assert!(
        s.js.get(script::ScriptSource::Catalog, "ShopRunner")
            .unwrap()
            .js
            .is_empty(),
        "a click must not warm the rest of the catalog"
    );

    s.pump_script_transpile();
    assert!(
        s.js.get(script::ScriptSource::Catalog, "BoneBurier")
            .unwrap()
            .js
            .is_empty(),
        "first pump only arms so the UI can paint transpiling…"
    );

    s.pump_script_transpile();
    assert!(!s
        .js
        .get(script::ScriptSource::Catalog, "BoneBurier")
        .unwrap()
        .js
        .is_empty());
    assert!(s
        .js
        .get(script::ScriptSource::Catalog, "ShopRunner")
        .unwrap()
        .js
        .is_empty());
    assert!(s.transpile_queue.is_empty());
}

#[test]
fn queue_transpile_all_warms_one_card_per_armed_frame() {
    let iso = IsolatedEnv::enter("transpile-all");
    let root = write_two_card_catalog(&iso.dir);
    let mut s = Session::new();
    s.persist_ui = false;
    s.js = script::JsLibrary::with_cache(iso.dir.join("js-scripts.json"), iso.dir.join("js-cache"));
    s.js.register_rs2b0t(&root, &iso.dir.join("rs2b0t-path"))
        .expect("catalog");

    s.queue_transpile_all();
    assert_eq!(s.transpile_queue.len(), 2);
    assert_eq!(s.transpile_total, 2);

    s.pump_script_transpile();
    s.pump_script_transpile();
    let warmed = s.js.cards().iter().filter(|c| !c.js.is_empty()).count();
    assert_eq!(warmed, 1, "all-at-once still one file per armed frame");
    assert_eq!(s.transpile_queue.len(), 1);

    s.pump_script_transpile();
    s.pump_script_transpile();
    assert_eq!(s.js.cards().iter().filter(|c| !c.js.is_empty()).count(), 2);
    assert!(s.transpile_queue.is_empty());
}

#[test]
fn select_script_card_skips_queue_on_cache_hit() {
    let iso = IsolatedEnv::enter("transpile-hit");
    let root = write_two_card_catalog(&iso.dir);
    let mut s = Session::new();
    s.persist_ui = false;
    s.js = script::JsLibrary::with_cache(iso.dir.join("js-scripts.json"), iso.dir.join("js-cache"));
    s.js.register_rs2b0t(&root, &iso.dir.join("rs2b0t-path"))
        .expect("catalog");
    s.js.ensure_js(script::ScriptSource::Catalog, "BoneBurier")
        .unwrap();

    s.select_script_card(script::ScriptSource::Catalog, "BoneBurier");
    assert!(s.transpile_queue.is_empty());
}
