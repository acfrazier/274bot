use super::{
    combo_index, debug_dest_cheats, debug_main_buttons_for, debug_maxme_cheats, live_client_trail,
    live_or_walk_paint, null_raster_live_entries, parse_getvar_line, publish_nav_debug,
    retire_client_trail, script_active, script_pause_enabled, script_stop_enabled,
    seed_on_first_world, stress_live_entries, temp_live_vault_from, FeedbackOwner,
    PanelPreferenceMember, PreparationCompletion, SceneRoutePaint, Session, SlotIo,
    PANEL_SETTING_EVENTS_CAP,
};
use crate::focus::draw_for_slot;
use crate::picker::{
    format_walkto_status, walkto_actions_enabled, walkto_footer_labels, walkto_selection_caption,
};
#[cfg(feature = "js")]
use crate::test_support::save_store_patiently;
use crate::test_support::{
    patiently, set_patiently, thread_preferences, thread_state_root, TestDir, TestPath,
};
use api::identity::Deadline;
use api::snapshot::{GameSnapshot, WorldTile};
use api::work::{
    InspectionPhase, MonotonicClock, SharedCancellation, SliceBudget, StepContext, SystemClock,
};
use client::client::{Client, ClientConfig};
use client::config::if_type::{ComponentType, IfType, IfTypeMut};
use client::dash3d::CollisionFlag;
use client::io::{Packet, ServerProt};
use client::render::nav_debug::{CORNER_NE, FACE_N, FACE_S};
use frontend_core::launch::{
    InstanceKind, InstancePermit, ProfileArgs, ProfileClass, ProfileEnv, ProfileTemplate,
    ValidatedProfile, ValidationPending,
};
use frontend_core::scripts::{CardSource, CompiledCardId, Loadouts, ScriptChoice, ScriptRunState};
#[cfg(feature = "js")]
use frontend_core::scripts::{LoadShape, LoadStage};
use frontend_core::surface::InputSender;
use frontend_core::surface::{FrameBuf, InputEv, SlotInput};
#[cfg(feature = "js")]
use frontend_core::test_support::session::{
    assignment_key, core_vault_debug_rows, vault_debug_rows, with_settings_bag,
};
use frontend_core::test_support::session::{
    clear_vault, install_vault, vault_upsert, with_assignment, with_lamp_auto, with_lamp_skill,
    with_random_events, with_tutorial_skipped,
};
use frontend_core::test_support::{FixtureProfile, IsolatedStores, VaultFixture};
use host_play as map_host;
use host_play::profile::ProfileEnvironment;
use host_play::{ProfileOptions, SlotArm, SlotStatus, StartupPhase};
use nav_planner::collision::WorldCollision;
use nav_planner::paint::{MAX_DRAW_TILES, NEAR_FULL_DENSITY};
use nav_planner::router::{
    CursorStep, FindOptions, Leg, Route, SearchCompletion, SearchCursor, SearchRequest,
};
use nav_planner::tile::Tile;
use nav_planner::transport::{TransportEdge, TransportGraph, TransportKind};
use nav_planner::world::NavWorld;
use nav_planner::zones::ZoneExempt;
use nav_planner::WorldState;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[path = "../../host-play/tests/support/map_fixture.rs"]
pub(super) mod map_fixture;
use map_fixture::MapFixture;
#[path = "../../host-play/tests/support/upgrade_home.rs"]
mod upgrade_home;
use upgrade_home::UpgradeHome;

/// Seed the per-thread stand-in the next `Session::new()` reads its panel
/// preferences from (`ui_state::load`), before any session exists to pass
/// its own preferences.
fn seed_ui_state(state: &crate::ui_state::PanelUiState) {
    crate::ui_state::save(&thread_preferences(), state);
}

fn await_preference(session: &mut Session, key: &str, expected: serde_json::Value) {
    patiently(|| {
        session.pump_status();
        session.poll_panel_preferences();
        if !session.preferences_pending()
            && session.preferences.value(key) == Some(expected.clone())
        {
            Ok(())
        } else {
            Err("not durable within the panel preference wait".into())
        }
    })
    .unwrap();
}

fn hydrate_panel_preferences(session: &mut Session) {
    session.preferences.reload().unwrap();
    session.poll_panel_preferences();
    assert!(session.panel_preferences_ready());
}

#[test]
fn session_cache_is_unset_until_a_profile_is_bound() {
    let session = Session::new();
    let cache_dir = session.play_options().cache_dir();

    assert!(
        cache_dir.is_empty(),
        "cache path must come from the resolved profile, not the standalone-client default: {cache_dir:?}"
    );
}

#[test]
fn panel_member_intent_keeps_unedited_document_members() {
    let _iso = IsolatedStores::enter("panel-member-intent");
    let mut session = Session::new();
    let preferences = session.preferences.share();
    let mut chrome = serde_json::to_value(crate::theme::ChromeColors::default())
        .unwrap()
        .as_object()
        .unwrap()
        .clone();
    chrome.insert("accent".into(), serde_json::json!("#111111"));
    chrome.insert("text".into(), serde_json::json!("#222222"));
    chrome.insert("extension".into(), serde_json::json!("preserve"));
    set_patiently(&preferences, "chrome", serde_json::Value::Object(chrome));
    set_patiently(
        &preferences,
        "third_party",
        serde_json::json!({"untouched": true}),
    );

    session
        .persist_panel_member(&["chrome", "accent"], Some(serde_json::json!("#AABBCC")))
        .unwrap();
    patiently(|| {
        session.poll_panel_preferences();
        let Ok(document) = preferences.document() else {
            return Err("not durable within panel preference wait".into());
        };
        if session.panel_preference_writes_pending() == 0
            && document
                .get("chrome")
                .and_then(serde_json::Value::as_object)
                .and_then(|chrome| chrome.get("accent"))
                == Some(&serde_json::json!("#AABBCC"))
        {
            Ok(())
        } else {
            Err("not durable within panel preference wait".into())
        }
    })
    .unwrap();

    let document = preferences.document().unwrap();
    let chrome = document["chrome"].as_object().unwrap();
    assert_eq!(chrome["accent"], serde_json::json!("#AABBCC"));
    assert_eq!(chrome["text"], serde_json::json!("#222222"));
    assert_eq!(chrome["extension"], serde_json::json!("preserve"));
    assert_eq!(
        document["third_party"],
        serde_json::json!({"untouched": true})
    );
}

#[test]
fn memory_override_changes_spawn_profile_without_persisting_it() {
    let path = tmp_vault("memory-override-spawn.vault");
    let mut s = Session::new();
    s.core.set_spawn_workers(false);
    s.core.set_play(Some(empty_play()));
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    vault.put(profile("alice", "pw", 274).lowmem(true));
    install_vault(&mut s.core, vault);
    s.set_memory_override(Some(false));

    s.load("alice");
    let arm = s.core.play_for_test().unwrap().arm("alice").unwrap();
    assert_eq!(
        arm.lowmem_handshake(),
        Some(false),
        "the session override seeds the effective handshake mode"
    );
    s.core
        .play_for_test()
        .unwrap()
        .statuses
        .lock()
        .unwrap()
        .push(SlotStatus {
            username: "alice".into(),
            connected: true,
            ingame: true,
            scene_state: 2,
            login_lowmem: Some(false),
            ..SlotStatus::default()
        });
    s.core.poll();
    assert!(
        !s.core.memory_status("alice").unwrap().differs(),
        "the override must not create a permanent false notice"
    );

    assert!(
        s.core.profile("alice").unwrap().lowmem(),
        "the vault profile must remain unchanged"
    );
}

use crate::nav_settings::{effective, NavSettings};

fn checked_profile_fixture(revision: u16) -> (TestDir, PathBuf, PathBuf) {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../host-play/tests/fixtures/profile");
    let root = TestDir::new(&format!("session-profile-{revision}"));
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

fn plant_snapshot(unpack: &Path, revision: u16, packs: &[(String, Vec<u8>)]) {
    struct FixtureEntries(Vec<(i32, i32)>);
    impl client::unpack::EntrySource for FixtureEntries {
        fn request_entry(&mut self, archive: i32, file: i32) -> Result<(), String> {
            self.0.push((archive, file));
            Ok(())
        }

        fn take_entries(
            &mut self,
            _until: std::time::Instant,
            out: &mut Vec<client::unpack::FetchedEntry>,
        ) -> Result<(), String> {
            out.extend(
                self.0
                    .drain(..)
                    .map(|(archive, file)| client::unpack::FetchedEntry {
                        archive,
                        file,
                        payload: b"body".to_vec(),
                    }),
            );
            Ok(())
        }
    }
    let versionlist = &packs
        .iter()
        .find(|(name, _)| name == "versionlist")
        .unwrap()
        .1;
    let transfer = nav_planner::manifest::hash_bytes(versionlist);
    let negotiated = nav_planner::manifest::hash_bytes(&crc_body(packs)[..36]);
    let input = unpack.join("fixture-input");
    std::fs::create_dir_all(&input).unwrap();
    for (name, bytes) in packs {
        std::fs::write(input.join(name), bytes).unwrap();
    }
    let out = unpack
        .join(format!("revision-{revision}"))
        .join(negotiated)
        .join(transfer);
    // Use the same verified retained publisher fixture as host runtime_bind.
    // Legacy size-only markers are not ordinary-launch asset candidates.
    client::unpack::fetch_snapshot(
        &input.to_string_lossy(),
        &out.to_string_lossy(),
        &mut FixtureEntries(Vec::new()),
    )
    .unwrap();
}

/// Mock update server: `/crc` matching the fixture packs, plus pack GETs if
/// a runtime refresh asks. The owning fixture stops and joins the server.
struct FixtureCrcServer {
    port: u16,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl FixtureCrcServer {
    fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for FixtureCrcServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn serve_fixture_crc(packs: Vec<(String, Vec<u8>)>) -> FixtureCrcServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let worker = thread::spawn(move || {
        let body = crc_body(&packs);
        while !worker_stop.load(Ordering::Acquire) {
            let (mut sock, _) = match listener.accept() {
                Ok(conn) => {
                    // Accepted sockets inherit O_NONBLOCK from the listener on macOS/BSD.
                    let _ = conn.0.set_nonblocking(false);
                    conn
                }
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
    FixtureCrcServer {
        port,
        stop,
        worker: Some(worker),
    }
}

fn runtime_profile_fixture(
    revision: u16,
) -> (TestDir, PathBuf, PathBuf, PathBuf, FixtureCrcServer) {
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
    plant_snapshot(&unpack, revision, &packs);
    let server = serve_fixture_crc(packs);
    (root, cache, manifest_path, unpack, server)
}

fn preparation_only_session() -> Session {
    #[cfg(feature = "js")]
    frontend_core::test_support::scripts::install_catalog_cards();
    let mut session = Session::new();
    session.core.set_spawn_workers(false);
    session
}

fn isolated_profile_environment(home: &Path) -> ProfileEnvironment {
    ProfileEnvironment {
        home: Some(home.to_path_buf()),
        working_dir: Some(home.to_path_buf()),
        engine_dir: Some(home.join("engine")),
        rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
        rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
        ..ProfileEnvironment::default()
    }
}

fn configure_isolated_profile(
    session: &mut Session,
    options: ProfileOptions,
    home: &Path,
) -> Result<(), frontend_core::launch::ProfileError> {
    session.configure_profile_with_env(
        ProfileArgs::from_host(options),
        ProfileEnv::from_host(isolated_profile_environment(home)),
    )
}

/// Join a final resource validation on the shared worker, as boot does.
/// The caller holds `crate::work_service_test_lock()`.
fn join_validation(session: &mut Session, generation: u64) -> ValidatedProfile {
    let template = session.template_for_validation().unwrap();
    let job = ValidationPending::new(generation, template)
        .submit()
        .expect("validation submit");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !job.is_finished() && Instant::now() < deadline {
        session.core.poll();
        session.poll_scripts();
        thread::sleep(Duration::from_millis(5));
    }
    assert!(job.is_finished(), "validation did not settle");
    job.join().expect("validation")
}

#[test]
fn explicit_profile_wins_saved_revision_and_bound_session_refuses_changes() {
    let (root, cache, manifest, unpack, server) = runtime_profile_fixture(274);
    let mut session = Session::new();
    session.ui.server_revision = 289;
    configure_isolated_profile(
        &mut session,
        ProfileOptions {
            profile: Some("local-274".into()),
            cache_dir: Some(cache),
            cache_manifest: Some(manifest),
            unpack_dir: Some(unpack),
            http_port: Some(server.port()),
            ..ProfileOptions::default()
        },
        &root,
    )
    .unwrap();
    session.bind_profile().unwrap();
    assert!(session.server_label().starts_with("local-274"));
    #[cfg(feature = "js")]
    assert_eq!(session.catalog_root().unwrap(), None);
    assert!(session.set_server_revision(289).is_err());
    assert!(session.bind_profile().is_err());
}

#[test]
fn stale_profile_preparation_is_dropped_without_partial_session_state() {
    let _work_service_guard = crate::work_service_test_lock();
    let (root, cache, manifest, unpack, server) = runtime_profile_fixture(274);
    let mut session = Session::new();
    configure_isolated_profile(
        &mut session,
        ProfileOptions {
            profile: Some("local-274".into()),
            cache_dir: Some(cache),
            cache_manifest: Some(manifest),
            unpack_dir: Some(unpack),
            http_port: Some(server.port()),
            ..ProfileOptions::default()
        },
        &root,
    )
    .unwrap();
    let generation = session.profile_generation();
    let preparation = session.profile_preparation().unwrap();
    let template = preparation.run_blocking().unwrap();

    configure_isolated_profile(
        &mut session,
        ProfileOptions {
            profile: Some("local-289".into()),
            ..ProfileOptions::default()
        },
        &root,
    )
    .unwrap();
    assert_eq!(
        session.finish_profile_preparation(generation, Ok(template)),
        PreparationCompletion::Stale
    );
    assert!(!session.profile_bound());
    assert!(session.launcher.template().is_none());
    assert!(session.core.play_for_test().is_none());
    assert!(!session.core.vault_open());
    assert!(session.core.slots().is_empty());
}

#[test]
fn profile_preparation_failure_keeps_vault_play_and_slots_absent() {
    let home = TestDir::new("profile-preparation-failure");
    let mut session = Session::new();
    configure_isolated_profile(&mut session, ProfileOptions::default(), &home).unwrap();
    let generation = session.profile_generation();

    assert_eq!(
        session.finish_profile_preparation(
            generation,
            Err(
                frontend_core::launch::ProfileError::prepare_failed_for_test(
                    "cache changed".into()
                )
            )
        ),
        PreparationCompletion::Failed
    );
    assert_eq!(session.error.as_deref(), Some("cache changed"));
    assert!(!session.profile_bound());
    assert!(session.launcher.template().is_none());
    assert!(session.core.play_for_test().is_none());
    assert!(!session.core.vault_open());
    assert!(session.core.slots().is_empty());
}

#[test]
fn validated_profile_unlock_uses_the_ticket_without_rebinding() {
    let _work_service_guard = crate::work_service_test_lock();
    let (root, cache, manifest, unpack, server) = runtime_profile_fixture(274);
    let mut session = Session::new();
    configure_isolated_profile(
        &mut session,
        ProfileOptions {
            profile: Some("local-274".into()),
            cache_dir: Some(cache.clone()),
            cache_manifest: Some(manifest),
            unpack_dir: Some(unpack),
            http_port: Some(server.port()),
            vault_path: Some(root.join("prepared.vault")),
            ..ProfileOptions::default()
        },
        &root,
    )
    .unwrap();
    let generation = session.profile_generation();
    let preparation = session.profile_preparation().unwrap();
    let template = preparation.run_blocking().unwrap();
    assert_eq!(
        session.finish_profile_preparation(generation, Ok(template.share())),
        PreparationCompletion::Installed
    );
    let validated = join_validation(&mut session, generation);
    session.install_validated_template(validated).unwrap();

    std::fs::write(cache.join("config"), b"changed after validation").unwrap();
    assert!(session.unlock("prepared-pass"));
    assert!(session.profile_bound());
    assert!(session.core.play_for_test().is_some());
    assert!(session.core.vault_open());
    assert!(session.core.slots().is_empty());
}

#[cfg(feature = "js")]
/// `start_play` builds the Play with a state composition of its own, at the
/// profile's state root, and hands it this session's service before the
/// Start handle is taken: a Start without caller-owned loadouts leases the
/// operator rows through the service the Loadouts form saves to, not through
/// the Play's own.
#[test]
fn start_handle_reads_operator_loadouts_through_the_sessions_service() {
    let _iso = IsolatedStores::enter("start-handle-loadouts");
    let _work_service_guard = crate::work_service_test_lock();
    let (root, cache, manifest, unpack, server) = runtime_profile_fixture(274);
    let mut session = Session::new();
    configure_isolated_profile(
        &mut session,
        ProfileOptions {
            profile: Some("local-274".into()),
            cache_dir: Some(cache),
            cache_manifest: Some(manifest),
            unpack_dir: Some(unpack),
            http_port: Some(server.port()),
            vault_path: Some(root.join("prepared.vault")),
            ..ProfileOptions::default()
        },
        &root,
    )
    .unwrap();
    let generation = session.profile_generation();
    let preparation = session.profile_preparation().unwrap();
    let template = preparation.run_blocking().unwrap();
    assert_eq!(
        session.finish_profile_preparation(generation, Ok(template.share())),
        PreparationCompletion::Installed
    );
    let validated = join_validation(&mut session, generation);
    session.install_validated_template(validated).unwrap();

    // A row only the Play's own state root holds: the session's service
    // does not see it.
    let own_root = root.join(".274bot");
    assert_ne!(own_root.as_path(), thread_state_root().path());
    let mut own = Loadouts::operator(&crate::test_support::scratch_service(&own_root));
    own.upsert_for_test(api::loadout::Loadout::new("melee"));
    save_store_patiently(&mut own);

    assert!(session.unlock("prepared-pass"));
    let handle = session.core.live_runner().share();
    let start = || {
        let mut bag = serde_json::Map::new();
        bag.insert("loadout".into(), serde_json::json!("melee"));
        handle.start_native_tick_for_test("alice", "export function tick() {}".into(), Some(bag))
    };
    let refused = start().expect_err("the session's service holds no melee row");
    assert!(refused.contains("\"melee\""), "{refused}");

    // The Loadouts form saves through the session's service: the Start
    // leases that row.
    session
        .loadouts
        .stage_draft(api::loadout::Loadout::new("melee"));
    let saved = crate::test_support::save_draft_patiently(&mut session);
    assert!(saved, "{}", session.loadouts.status());
    start().expect("the Start leases the row the form saved");
}

/// T5.2 r3 round trip: the panel's composition root saves a preference
/// (its own `PanelUiState` keys) and a Loadouts edit through the session's
/// local-state service. Another process's service on the same home reads
/// both back; its own edits reach the panel's forms after they re-read, and
/// the panel's next save keeps them.
#[test]
fn a_preference_and_a_loadout_edit_round_trip_through_the_sessions_service() {
    use api::local_state::{AccountKey, ContentScope, DomainIdentity, LocalStateDomain};

    const WAIT: Duration = Duration::from_secs(60);
    let await_readback = |service: &host_play::local_state::LocalStateService| {
        for domain in [
            LocalStateDomain::Preferences,
            LocalStateDomain::LoadoutsReferences,
        ] {
            let identity = DomainIdentity::new(
                domain,
                AccountKey::try_new("operator", "operator").unwrap(),
                ContentScope::try_new(client::io::ClientRevision::R289, "operator").unwrap(),
            );
            service
                .readback_lease(&identity, WAIT)
                .expect("initial operator-store readback must complete");
        }
    };

    let _iso = IsolatedStores::enter("panel-state-round-trip");
    let mut session = Session::new();
    await_readback(session.core.local_state_for_test().unwrap());
    session.preferences.reload().unwrap();
    session.loadouts.reload_for_test().unwrap();
    assert!(
        session.loadouts.load_error().is_none(),
        "initial loadouts readback must complete"
    );
    let state = crate::ui_state::PanelUiState {
        last_focus: Some("alice".into()),
        ..Default::default()
    };
    crate::ui_state::save_to_checked(&session.preferences, &state).unwrap();
    session
        .loadouts
        .stage_draft(api::loadout::Loadout::new("roundtrip").with_carry("Lobster", 5));
    assert!(
        crate::loadouts::save_draft(&mut session),
        "{}",
        session.loadouts.status()
    );

    // Another 274bot process on this home: its own service, same files.
    let peer = Arc::new(host_play::local_state::LocalStateService::open_process(
        thread_state_root(),
    ));
    await_readback(&peer);
    assert_eq!(
        peer.preferences().value("last_focus"),
        Some(serde_json::json!("alice"))
    );
    let mut peer_form = Loadouts::operator(&peer);
    assert!(
        peer_form.load_error().is_none(),
        "the peer's initial loadouts readback must complete"
    );
    assert_eq!(
        peer_form.get("roundtrip").unwrap().carry,
        vec![api::loadout::CarryEntry::new("Lobster", 5)]
    );
    let on_disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(thread_state_root().loadouts_file()).unwrap())
            .unwrap();
    assert_eq!(on_disk[0]["name"], "roundtrip", "{on_disk}");

    // The peer's edits reach the panel through the panel's service (a key
    // the panel's own state does not hold, as another frontend's would be).
    peer.preferences()
        .set("peer_marker", serde_json::json!({"kept": 7}))
        .unwrap();
    peer_form.upsert_for_test(api::loadout::Loadout::new("peer-row"));
    peer_form.save_for_test().unwrap();
    session.preferences.reload().unwrap();
    assert_eq!(
        session.preferences.value("peer_marker"),
        Some(serde_json::json!({"kept": 7}))
    );
    session.loadouts.reload_for_test().unwrap();
    assert_eq!(session.loadouts.names(), ["roundtrip", "peer-row"]);

    // The panel's next saves keep the peer's row and preference.
    session.loadouts.stage_selection(0);
    session
        .loadouts
        .stage_draft(api::loadout::Loadout::new("roundtrip").with_carry("Lobster", 6));
    assert!(
        crate::loadouts::save_draft(&mut session),
        "{}",
        session.loadouts.status()
    );
    let state = crate::ui_state::PanelUiState {
        last_focus: Some("bob".into()),
        ..state
    };
    crate::ui_state::save_to_checked(&session.preferences, &state).unwrap();
    let mut reread = Loadouts::operator(&peer);
    reread.reload_for_test().unwrap();
    assert_eq!(reread.names(), ["roundtrip", "peer-row"]);
    assert_eq!(reread.get("roundtrip").unwrap().carry[0].qty, 6);
    let prefs = peer.preferences();
    prefs.reload().unwrap();
    assert_eq!(
        prefs.value("peer_marker"),
        Some(serde_json::json!({"kept": 7}))
    );
    assert_eq!(prefs.value("last_focus"), Some(serde_json::json!("bob")));
}

/// Install a template bound against the upgraded home and unlock a first-run
/// vault inside it, as panel startup does.
fn unlock_upgrade_home(template: &Arc<map_host::SharedClientTemplate>) -> Session {
    let home = template
        .profile()
        .vault_path()
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let mut session = preparation_only_session();
    configure_isolated_profile(&mut session, ProfileOptions::default(), &home).unwrap();
    let generation = session.profile_generation();
    assert_eq!(
        session.finish_profile_preparation(
            generation,
            Ok(ProfileTemplate::from_host(Arc::clone(template)))
        ),
        PreparationCompletion::Installed
    );
    let validated = join_validation(&mut session, generation);
    session.install_validated_template(validated).unwrap();
    assert!(session.unlock("upgrade-pass"), "{:?}", session.error);
    session
}

/// A 0.1.9.x home keeps its v10 pack at the default path. The panel's play
/// routes on the packaged v11 bundle's one shared world and leaves the old file
/// alone; an explicit override to the old file still unlocks, with no world.
#[test]
fn panel_upgraded_from_v10_home_plays_on_the_packaged_v11_world() {
    let home = UpgradeHome::new();
    let _work_service_guard = crate::work_service_test_lock();
    let profile = home.bind(None);
    assert!(profile.nav_origin().is_bundled());
    let template = map_host::SharedClientTemplate::load(profile).unwrap();
    let session = unlock_upgrade_home(&template);
    let world = session
        .core
        .play_for_test()
        .and_then(|play| play.world())
        .expect("the panel play has the packaged world");
    assert!(Arc::ptr_eq(&world, &template.world().unwrap()));
    assert!(home.old_pack_untouched());

    let overridden = home.bind(Some(home.old_pack.clone()));
    assert!(matches!(
        overridden.nav_availability(),
        map_host::profile::NavAvailability::Unavailable(message)
            if message.contains("rebuild it with nav-pack")
    ));
    let template = map_host::SharedClientTemplate::load(overridden).unwrap();
    let session = unlock_upgrade_home(&template);
    assert!(session.error.is_none(), "{:?}", session.error);
    assert!(session.core.play_for_test().unwrap().world().is_none());
    assert!(home.old_pack_untouched());
}

#[test]
fn panel_profile_install_does_not_decode_reach_until_layer_demand() {
    let _guard = crate::picker::lock_nav_statics();
    let home = UpgradeHome::new();
    let profile = home.bind(None);
    let template = map_host::SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    let mut session = preparation_only_session();
    session.install_prepared_template(ProfileTemplate::from_host(template));
    let world = profile.world().unwrap();
    assert_eq!(profile.nav_load_counters().reach_reads, 1);
    assert_eq!(profile.nav_load_counters().reach_hashes, 1);
    nav_planner::paint::reset_bake_reach_calls();
    // An eager install would keep the old verified payload and paint it even
    // after the file changes. First layer demand must instead fail closed.
    let path = profile.nav_pack().with_extension("navreach");
    let mut bytes = std::fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(path, bytes).unwrap();
    assert!(
        crate::picker::map_reach_bitset(&world).is_none(),
        "profile install must retain no decoded reach payload"
    );
    assert!(profile.reach().is_none());
    assert_eq!(profile.nav_load_counters().reach_reads, 2);
    assert_eq!(profile.nav_load_counters().reach_hashes, 2);
    assert_eq!(nav_planner::paint::bake_reach_calls(), 0);
}

#[test]
fn panel_map_uses_the_shared_lazy_sidecar_and_releases_its_binding() {
    let _guard = crate::picker::lock_nav_statics();
    let home = UpgradeHome::new();
    let profile = home.bind(None);
    let template = map_host::SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    let mut session = preparation_only_session();
    session.install_prepared_template(ProfileTemplate::from_host(template));
    let world = profile.world().unwrap();
    assert_eq!(profile.nav_load_counters().reach_reads, 1);
    assert_eq!(profile.nav_load_counters().reach_hashes, 1);
    nav_planner::paint::reset_bake_reach_calls();
    let map_bits = crate::picker::map_reach_bitset(&world).unwrap();
    assert_eq!(&*map_bits, &[0u64]);
    assert!(Arc::ptr_eq(&map_bits, &profile.reach().unwrap()));
    assert_eq!(Arc::strong_count(&map_bits), 2, "profile plus paint lease");
    assert_eq!(nav_planner::paint::bake_reach_calls(), 0);
    let mut other = nav_planner::world::NavWorld::load_pack(profile.nav_pack()).unwrap();
    other.collision.origin.x += 1;
    assert!(crate::picker::map_reach_bitset(&other).is_none());
    assert!(crate::picker::reach_bitset(&other).is_none());
    assert_eq!(nav_planner::paint::bake_reach_calls(), 0);
    session.release_walk_map();
    assert!(Arc::ptr_eq(
        &map_bits,
        &crate::picker::map_reach_bitset(&world).unwrap()
    ));
    assert_eq!(Arc::strong_count(&map_bits), 2);
    drop(map_bits);
    assert_eq!(Arc::strong_count(&profile.reach().unwrap()), 2);
    crate::picker::set_pack(None);
    assert!(crate::picker::map_reach_bitset(&world).is_none());
}

#[test]
fn rs2b2t_ignores_the_saved_local_revision() {
    let home = TestDir::new("public-profile");
    let mut session = Session::new();
    session.ui.server_revision = 274;
    let options = ProfileOptions {
        rs2b2t: true,
        ..ProfileOptions::default()
    };
    let env = ProfileEnvironment {
        home: Some(home.to_path_buf()),
        working_dir: Some(home.to_path_buf()),
        ..ProfileEnvironment::default()
    };
    session
        .configure_profile_with_env(
            ProfileArgs::from_host(options.clone()),
            ProfileEnv::from_host(env.clone()),
        )
        .unwrap();
    // Resolution moved behind the launch seam: the session configures, the
    // same host inputs resolve.
    let selection = options
        .resolve_with_env(Some(session.ui.server_revision), &env)
        .unwrap();
    assert_eq!(selection.revision(), client::io::ClientRevision::R289);
    assert_eq!(selection.profile_class(), host_play::ProfileClass::Remote);
}

#[test]
fn invalid_profile_refuses_vault_reset_without_deleting_explicit_path() {
    let root = TestDir::new("invalid-reset");
    let intended = root.join("intended.vault");
    std::fs::write(&intended, b"do not delete").unwrap();

    let mut session = Session::new();
    session
        .configure_profile_with_env(
            ProfileArgs::from_host(ProfileOptions {
                revision: Some("275".into()),
                vault_path: Some(intended.clone()),
                ..ProfileOptions::default()
            }),
            ProfileEnv::from_host(ProfileEnvironment {
                home: Some(root.to_path_buf()),
                working_dir: Some(root.to_path_buf()),
                ..ProfileEnvironment::default()
            }),
        )
        .unwrap();
    assert!(!session.reset_vault());
    assert!(intended.is_file());
    assert!(session
        .error
        .as_deref()
        .is_some_and(|error| error.contains("unsupported revision")));
}

#[test]
fn configured_profile_resolution_uses_only_the_injected_environment() {
    let ambient = TestDir::new("profile-ambient-guard");
    let injected = TestDir::new("profile-injected-home");
    let mut session = Session::new();
    let options = ProfileOptions {
        profile: Some("local-274".into()),
        ..ProfileOptions::default()
    };
    let env = ProfileEnvironment {
        home: Some(injected.to_path_buf()),
        working_dir: Some(injected.to_path_buf()),
        engine_dir: Some(injected.join("engine")),
        ..ProfileEnvironment::default()
    };
    session
        .configure_profile_with_env(
            ProfileArgs::from_host(options.clone()),
            ProfileEnv::from_host(env.clone()),
        )
        .unwrap();

    let selection = options
        .resolve_with_env(Some(session.ui.server_revision), &env)
        .unwrap();
    assert_eq!(selection.profile_class(), host_play::ProfileClass::Local);
    assert!(injected.join(".274bot/servers.json").is_file());
    assert!(
        !ambient.join(".274bot").exists(),
        "profile resolution must not inspect or create an uninjected home"
    );
}

#[test]
fn unbound_and_failed_profile_bind_hide_local_debug_capabilities() {
    let home = TestDir::new("unbound-remote-profile");
    let mut session = Session::new();
    session
        .configure_profile_with_env(
            ProfileArgs::from_host(ProfileOptions {
                rs2b2t: true,
                ..ProfileOptions::default()
            }),
            ProfileEnv::from_host(ProfileEnvironment {
                home: Some(home.to_path_buf()),
                ..ProfileEnvironment::default()
            }),
        )
        .unwrap();

    assert_eq!(session.profile_class(), ProfileClass::Remote);
    assert!(!session.debug_ui());
    assert!(!session.map_teleport_authorized());

    let generation = session.profile_generation();
    assert_eq!(
        session.finish_profile_preparation(
            generation,
            Err(
                frontend_core::launch::ProfileError::prepare_failed_for_test("bind refused".into())
            )
        ),
        PreparationCompletion::Failed
    );
    assert!(!session.profile_bound());
    assert_eq!(session.profile_class(), ProfileClass::Remote);
    assert!(!session.debug_ui());
    assert!(!session.map_teleport_authorized());
}

/// `register_name` is the ScriptRegistry name; `folder` is the class
/// file (`{folder}/{folder}.ts`) and the JS import binding.
#[cfg(feature = "js")]
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

#[cfg(feature = "js")]
fn settle_catalog_work(session: &mut Session) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while session.scripts.catalog_work_pending() && Instant::now() < deadline {
        session.core.poll();
        session.poll_scripts();
        thread::sleep(Duration::from_millis(5));
    }
    session.core.poll();
    session.poll_scripts();
    assert!(
        !session.scripts.catalog_work_pending(),
        "catalog preparation did not settle"
    );
}

#[cfg(feature = "js")]
fn settle_transpile_queue(session: &mut Session) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while session.transpile_front().is_some() && Instant::now() < deadline {
        session.core.poll();
        session.poll_scripts();
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        session.transpile_front().is_none(),
        "transpile queue did not settle"
    );
}

#[cfg(feature = "js")]
#[test]
fn explicit_catalog_default_allows_manual_import_and_preserves_custom_cards() {
    let _work_service_guard = crate::work_service_test_lock();
    let iso = IsolatedStores::enter("prebind-catalog");
    let explicit = write_looping_catalog(&iso.dir().join("explicit"), &[("Chosen", "Chosen")]);
    let ambient = write_looping_catalog(&iso.dir().join("ambient"), &[("Ambient", "Ambient")]);
    iso.set_catalog_root(&ambient);

    let mut session = Session::new();
    session.persist_ui = false;
    session
        .configure_profile_with_env(
            ProfileArgs::from_host(ProfileOptions {
                profile: Some("local-274".into()),
                catalog_root: Some(explicit.clone()),
                ..ProfileOptions::default()
            }),
            ProfileEnv::from_host(ProfileEnvironment {
                home: Some(iso.dir().to_path_buf()),
                working_dir: Some(iso.dir().to_path_buf()),
                engine_dir: Some(iso.dir().join("engine")),
                ..ProfileEnvironment::default()
            }),
        )
        .unwrap();
    assert_eq!(session.catalog_root().unwrap(), Some(explicit.clone()));
    session.fill_rs2b0t_cards_once();
    settle_catalog_work(&mut session);
    assert!(session
        .scripts
        .card(CardSource::Catalog, "Chosen")
        .is_some());
    assert!(session
        .scripts
        .card(CardSource::Catalog, "Ambient")
        .is_none());

    let custom = iso.dir().join("custom.ts");
    std::fs::write(
        &custom,
        "export default class Custom extends LoopingBot { override loop() {} }",
    )
    .unwrap();
    session.scripts.load_card(&custom).unwrap();
    session.import_rs2b0t_catalog(&ambient).unwrap();
    settle_catalog_work(&mut session);
    assert!(session.scripts.card(CardSource::File, "custom").is_some());
    assert!(session
        .scripts
        .card(CardSource::Catalog, "Ambient")
        .is_some());
}

#[cfg(feature = "js")]
#[test]
fn revision_and_binding_allow_already_loaded_scripts_and_source_edits() {
    let _work_service_guard = crate::work_service_test_lock();
    let (root, cache, manifest, unpack, server) = runtime_profile_fixture(274);
    let catalog = write_looping_catalog(&root.join("catalog"), &[("Chosen", "Chosen")]);
    let source = catalog.join("src/bot/scripts/Chosen/Chosen.ts");
    let env = ProfileEnvironment {
        home: Some(root.to_path_buf()),
        working_dir: Some(root.to_path_buf()),
        engine_dir: Some(root.join("engine")),
        rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
        rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
        ..ProfileEnvironment::default()
    };
    let mut session = Session::new();
    session.persist_ui = false;
    session
        .configure_profile_with_env(
            ProfileArgs::from_host(ProfileOptions {
                revision: None,
                cache_dir: Some(cache),
                cache_manifest: Some(manifest),
                unpack_dir: Some(unpack),
                http_port: Some(server.port()),
                catalog_root: Some(catalog),
                ..ProfileOptions::default()
            }),
            ProfileEnv::from_host(env.clone()),
        )
        .unwrap();
    session.fill_rs2b0t_cards_once();
    settle_catalog_work(&mut session);
    session
        .scripts
        .ensure_card(CardSource::Catalog, "Chosen")
        .unwrap();
    std::fs::write(
        source,
        "export default class Chosen extends LoopingBot { override loop() { this.walk(); } }",
    )
    .unwrap();
    session.set_server_revision(289).unwrap();
    assert!(session
        .scripts
        .card(CardSource::Catalog, "Chosen")
        .is_some());
    session.set_server_revision(274).unwrap();
    session.bind_profile().unwrap();
    let another = write_looping_catalog(&root.join("another"), &[("Another", "Another")]);
    session.import_rs2b0t_catalog(&another).unwrap();
    settle_catalog_work(&mut session);
    assert!(session
        .scripts
        .card(CardSource::Catalog, "Another")
        .is_some());
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
    let (route, click) =
        live_or_walk_paint(true, (Some(&live_route), Some(dest)), (None, None), None);
    assert_eq!(route.unwrap().projection().destination(), dest);
    assert_eq!(click, Some(dest));
    let (route, _) = live_or_walk_paint(true, (None, None), (None, None), None);
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
    let script_projection = frontend_core::walk_map::RouteProjectionView::legacy(
        &script_route,
        frontend_core::walk_map::WalkRouteSource::Script,
        1,
        Some(script_dest),
    );
    let (route, click) =
        live_or_walk_paint(false, (None, None), (None, None), Some(script_projection));
    assert_eq!(
        click,
        Some(script_dest),
        "catalog walk paints when WalkTo is idle"
    );
    assert_eq!(route.unwrap().projection().destination(), script_dest);
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
fn mark_tutorial_skipped_persists_on_focused_profile() {
    let path = tmp_vault("tutskip-pref.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    s.set_focus_for_test("alice");
    assert_eq!(s.focused_tutorial_skipped(), None);
    s.mark_tutorial_skipped();
    assert_eq!(s.focused_tutorial_skipped(), Some(true));
    assert_eq!(
        s.core.profile("alice").unwrap().tutorial_skipped(),
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

/// Pump fixture for the getvar-reply drain: focused `alice`, a local live
/// `Play`, our own capture channel standing in for the view's, and no
/// vault (TutSkip unknown, so the probe fires).
fn getvar_drain_session() -> (Session, frontend_core::surface::InputProbe) {
    let mut s = Session::new();
    s.core.set_spawn_workers(false);
    s.core.set_play(Some(empty_play()));
    s.core.insert_slot_io(
        "alice",
        SlotIo {
            input: SlotInput::new(),
            pixels: FrameBuf::new(),
        },
    );
    s.set_focus_for_test("alice");
    let (tx, rx) = InputSender::probe();
    s.map_desk.set_input_sender(Some(tx));
    (s, rx)
}

fn submit_staged_capture(s: &mut Session) {
    if !s.map_desk.has_staged_game_input() {
        return;
    }
    let submitted = s
        .submit(
            frontend_core::OperatorCommand::GameInput,
            frontend_core::Targets::Selected,
        )
        .expect("submit staged capture input");
    assert!(
        s.core.command_failure(submitted.request()).is_none(),
        "staged capture input was refused"
    );
}

/// One ingame scene-2 status row for `alice` with this chat head and
/// pending-box fact.
fn push_alice_row(s: &mut Session, chat_head: &str, pending: bool) {
    let mut rows = s.core.play_for_test().unwrap().statuses.lock().unwrap();
    rows.clear();
    rows.push(SlotStatus {
        username: "alice".into(),
        connected: true,
        ingame: true,
        scene_state: 2,
        chat_head: chat_head.into(),
        tutorial_pending: pending,
        ..SlotStatus::default()
    });
}

/// Drain whatever the capture channel holds.
fn drain_capture(rx: &frontend_core::surface::InputProbe) -> Vec<frontend_core::surface::InputEv> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

/// The drain click is one Move + left-Down/Up at the chat-surface point.
/// Position is not what makes it safe (any left click is consumed while
/// the box is present), so this asserts the exact point rather than a
/// "safe rect": the old rect test wrongly implied the click was harmless
/// with no box, where it would fall through to the chat minimenu.
fn assert_drain_click(clicks: &[frontend_core::surface::InputEv]) {
    assert_eq!(clicks.len(), 3, "one Move + Down + Up, got {clicks:?}");
    match &clicks[0] {
        frontend_core::surface::InputEv::Move { x, y } => assert_eq!((*x, *y), (250, 400)),
        other => panic!("first event is a Move, got {other:?}"),
    };
    match &clicks[1] {
        frontend_core::surface::InputEv::Down { button: 1, x, y } => {
            assert_eq!((*x, *y), (250, 400))
        }
        other => panic!("second event is a left Down at the Move point, got {other:?}"),
    };
    assert!(
        matches!(&clicks[2], frontend_core::surface::InputEv::Up),
        "third event releases the click, got {:?}",
        clicks[2]
    );
}

#[test]
fn game_input_overflow_is_reported_by_row() {
    let iso = IsolatedStores::enter("game-input-overflow");
    let (mut session, _rx) = getvar_drain_session();
    let event = frontend_core::GameInputEvent::Captured(frontend_core::surface::InputEv::Up);
    let mut overflowed = false;
    for _ in 0..1024 {
        if !session.map_desk.stage_game_input(event) {
            overflowed = true;
            break;
        }
    }
    assert!(overflowed, "test input exceeds the frame buffer capacity");

    let submitted = session
        .submit(
            frontend_core::OperatorCommand::GameInput,
            frontend_core::Targets::Selected,
        )
        .expect("submit overflowed game input frame");
    assert_eq!(
        session.core.command_failure(submitted.request()),
        Some("game input frame is full")
    );
    let _iso = iso;
}

#[test]
fn getvar_drain_clicks_pending_box_once() {
    let iso = IsolatedStores::enter("getvar-drain-once");
    let (mut s, rx) = getvar_drain_session();
    // The probe goes out on the first ingame scene-2 pump (TutSkip unknown).
    push_alice_row(&mut s, "", false);
    s.pump_status();
    submit_staged_capture(&mut s);
    assert!(drain_capture(&rx).is_empty());
    // The reply's pending box drains with exactly one chat click, keyed on
    // the box — not on the head text.
    push_alice_row(&mut s, "get tutorial: 1", true);
    s.pump_status();
    submit_staged_capture(&mut s);
    assert_drain_click(&drain_capture(&rx));
    // The same box on later pumps never clicks again.
    push_alice_row(&mut s, "get tutorial: 1", true);
    s.pump_status();
    submit_staged_capture(&mut s);
    assert!(drain_capture(&rx).is_empty(), "the box drains exactly once");
    // The box going away re-arms the slot, so the next box drains again —
    // even when the chat head shows unrelated text.
    push_alice_row(&mut s, "get tutorial: 1", false);
    s.pump_status();
    submit_staged_capture(&mut s);
    assert!(drain_capture(&rx).is_empty());
    push_alice_row(&mut s, "Welcome to RuneScape", true);
    s.pump_status();
    submit_staged_capture(&mut s);
    assert_drain_click(&drain_capture(&rx));
    let _iso = iso;
}

#[test]
fn getvar_drain_needs_the_pending_box() {
    let iso = IsolatedStores::enter("getvar-drain-no-box");
    let (mut s, rx) = getvar_drain_session();
    push_alice_row(&mut s, "", false);
    s.pump_status();
    submit_staged_capture(&mut s);
    assert!(drain_capture(&rx).is_empty());
    // The chat head parses as our reply, but the box is absent (tutorial
    // overlay down: the reply is a plain MESSAGE_GAME). A click at the
    // drain point would fall through to the chat minimenu — multi4 option
    // 2 spans applet x 17..496 y 393..410, covering the point — and select
    // it, so the drain must not click. Fails on the chat-head-keyed drain.
    push_alice_row(&mut s, "get tutorial: 1000", false);
    s.pump_status();
    submit_staged_capture(&mut s);
    assert!(
        drain_capture(&rx).is_empty(),
        "no click without the pending box, even for our own reply text"
    );
    let _iso = iso;
}

#[test]
fn getvar_drain_survives_chat_head_turnover() {
    let iso = IsolatedStores::enter("getvar-drain-turnover");
    let (mut s, rx) = getvar_drain_session();
    push_alice_row(&mut s, "", false);
    s.pump_status();
    submit_staged_capture(&mut s);
    assert!(drain_capture(&rx).is_empty());
    // Another line arrives before the drain runs: the head no longer shows
    // our reply, but the box is still present and must still drain. Fails
    // on the chat-head-keyed drain, which never sees the reply.
    push_alice_row(&mut s, "Welcome to RuneScape", true);
    s.pump_status();
    submit_staged_capture(&mut s);
    assert_drain_click(&drain_capture(&rx));
    let _iso = iso;
}

#[test]
fn getvar_drain_retries_when_capture_returns() {
    let iso = IsolatedStores::enter("getvar-drain-recapture");
    let (mut s, rx) = getvar_drain_session();
    push_alice_row(&mut s, "", false);
    s.pump_status();
    submit_staged_capture(&mut s);
    assert!(drain_capture(&rx).is_empty());
    // Capture off (pane closed / capture toggle): the box persists until a
    // left click or login, so the drain waits instead of losing the box.
    s.map_desk.set_input_sender(None);
    push_alice_row(&mut s, "get tutorial: 1", true);
    s.pump_status();
    submit_staged_capture(&mut s);
    push_alice_row(&mut s, "get tutorial: 1", true);
    s.pump_status();
    submit_staged_capture(&mut s);
    // Capture back on: the still-present box drains, exactly once.
    let (tx, rx2) = InputSender::probe();
    s.map_desk.set_input_sender(Some(tx));
    s.pump_status();
    submit_staged_capture(&mut s);
    assert_drain_click(&drain_capture(&rx2));
    push_alice_row(&mut s, "get tutorial: 1", true);
    s.pump_status();
    submit_staged_capture(&mut s);
    assert!(
        drain_capture(&rx2).is_empty(),
        "the deferred box still drains exactly once"
    );
    let _iso = iso;
}

#[test]
fn getvar_drain_waits_for_its_slot_to_focus() {
    let iso = IsolatedStores::enter("getvar-drain-focus");
    let (mut s, rx) = getvar_drain_session();
    s.core.insert_slot_io(
        "bob",
        SlotIo {
            input: SlotInput::new(),
            pixels: FrameBuf::new(),
        },
    );
    push_alice_row(&mut s, "", false);
    s.pump_status();
    submit_staged_capture(&mut s);
    assert!(drain_capture(&rx).is_empty());
    // Focus moves to bob before alice's reply box drains. The capture
    // channel feeds the focused slot's client only, so clicking now would
    // ack bob's client (whose box is absent) and hit his chat menu:
    // alice's box waits until she is focused with capture on. Capture is
    // deliberately left live: no click may go out for another slot's box.
    s.set_focus_for_test("bob");
    {
        let mut rows = s.core.play_for_test().unwrap().statuses.lock().unwrap();
        rows.clear();
        rows.push(SlotStatus {
            username: "alice".into(),
            connected: true,
            ingame: true,
            scene_state: 2,
            chat_head: "get tutorial: 1".into(),
            tutorial_pending: true,
            ..SlotStatus::default()
        });
        rows.push(SlotStatus {
            username: "bob".into(),
            connected: true,
            ingame: true,
            scene_state: 2,
            ..SlotStatus::default()
        });
    }
    s.pump_status();
    submit_staged_capture(&mut s);
    assert!(
        drain_capture(&rx).is_empty(),
        "no click for another slot's box while bob is focused"
    );
    // Focusing alice again with capture on drains her still-present box.
    s.set_focus_for_test("alice");
    let (tx, rx2) = InputSender::probe();
    s.map_desk.set_input_sender(Some(tx));
    s.pump_status();
    submit_staged_capture(&mut s);
    assert_drain_click(&drain_capture(&rx2));
    let _iso = iso;
}

#[test]
fn getvar_drain_leaves_unrelated_dialogue_text_alone() {
    let iso = IsolatedStores::enter("getvar-drain-unrelated");
    let (mut s, rx) = getvar_drain_session();
    push_alice_row(&mut s, "", false);
    s.pump_status();
    submit_staged_capture(&mut s);
    // No pending box: none of these drains, whatever the head says — a
    // chat line, the setvar reply, or a getvar reply for another var.
    for head in [
        "Welcome to RuneScape",
        "set tutorial: to 1000",
        "get coins: 25",
    ] {
        push_alice_row(&mut s, head, false);
        s.pump_status();
        submit_staged_capture(&mut s);
        assert!(
            drain_capture(&rx).is_empty(),
            "no click for unrelated text {head:?}"
        );
    }
    let _iso = iso;
}

#[test]
fn getvar_drain_ignores_matching_text_without_probe() {
    let iso = IsolatedStores::enter("getvar-drain-unprobed");
    let path = tmp_vault("getvar-drain-unprobed.vault");
    let (mut s, rx) = getvar_drain_session();
    // TutSkip already known: the probe never goes out, so even a present
    // box with matching head text is another actor's and must not drain.
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(
        &mut s.core,
        with_tutorial_skipped(profile("alice", "pw", 42), Some(true)),
    );
    push_alice_row(&mut s, "get tutorial: 1", true);
    s.pump_status();
    submit_staged_capture(&mut s);
    assert!(
        drain_capture(&rx).is_empty(),
        "only the dialogue our own probe caused drains"
    );
    let _iso = iso;
}

#[test]
fn getvar_drain_stays_local_only() {
    let iso = IsolatedStores::enter("getvar-drain-local");
    let (mut s, rx) = getvar_drain_session();
    // Non-loopback session host: no probe goes out and no reply ever drains.
    s.set_map_host("192.168.1.2");
    push_alice_row(&mut s, "", false);
    s.pump_status();
    submit_staged_capture(&mut s);
    push_alice_row(&mut s, "get tutorial: 1", true);
    s.pump_status();
    submit_staged_capture(&mut s);
    assert!(
        drain_capture(&rx).is_empty(),
        "the probe drain keeps its local-engine gating"
    );
    let _iso = iso;
}

#[test]
fn tutskip_button_omitted_until_known_open() {
    assert_eq!(
        debug_main_buttons_for(ProfileClass::Local, false),
        ["DebugPanel", "Lumbridge", "maxme", "Teles"]
    );
    assert_eq!(
        debug_main_buttons_for(ProfileClass::Local, true),
        ["DebugPanel", "TutSkip", "Lumbridge", "maxme", "Teles"]
    );
}

#[test]
fn debug_main_buttons_remote_is_debug_panel_only() {
    assert_eq!(
        debug_main_buttons_for(ProfileClass::Remote, true),
        ["DebugPanel"]
    );
    assert_eq!(
        debug_main_buttons_for(ProfileClass::Remote, false),
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
    frontend_core::test_support::empty_play(&host_play::PlayOptions {
        host: "127.0.0.1".into(),
        transport: client::Transport::Tcp,
        port: 43594,
        cache_dir: "/tmp".into(),
        lowmem: true,
        mainland: false,
    })
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

/// A synthetic offline client with a fake mainland scene base (same
/// trick as the app tests — no live server, no network).
fn paint_client() -> client::client::Client {
    let mut c = client::client::Client::from_shared(
        client::client::ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: true,
        },
        Arc::new(client::config::Cache::default()),
        Arc::new(vec![]),
        Vec::new(),
    );
    c.login_uid = 1;
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c
}

/// A 64×64 level-0 world at (3200, 3200) with a face wall and a
/// WR_GRND ground block inside the scene region.
fn walled_world() -> NavWorld {
    let width = 64;
    let height = 64;
    let mut flags = vec![0u32; width * height];
    flags[width + 1] = CollisionFlag::W_N as u32 | CollisionFlag::W_S as u32;
    flags[2 * width + 2] = CollisionFlag::WR_GRND as u32;
    let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
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

fn compact_route_completion(
    world: &Arc<NavWorld>,
    from: WorldTile,
    to: WorldTile,
) -> (nav_planner::contract::NavLease, SearchCompletion) {
    let lease = nav_planner::producer::lease(world).expect("fixture pack lease");
    let mut cursor = SearchCursor::new(
        SearchRequest::Single {
            from,
            to,
            options: FindOptions::default(),
            avoid: Box::default(),
            max_settles: 1_024,
        },
        lease.clone(),
        Arc::new(WorldState::default()),
    )
    .expect("small route cursor");
    let clock = SystemClock;
    let cancellation = SharedCancellation::new();
    let units = NonZeroU32::new(64).expect("nonzero slice");
    loop {
        let budget = SliceBudget::new(units, Deadline::after(clock.now(), Duration::from_secs(5)));
        let mut context = StepContext::<InspectionPhase>::new(&clock, budget, &cancellation);
        match cursor.advance(&mut context) {
            CursorStep::Complete(completion) => return (lease, completion),
            CursorStep::Yielded => {}
            CursorStep::Cancelled(reason) => panic!("fixture route cancelled: {reason:?}"),
            CursorStep::Failed(error) => panic!("fixture route failed: {error:?}"),
        }
    }
}

#[test]
fn compact_script_route_paints_its_scene_path_and_click_ahead_of_walkto() {
    let mut client = paint_client();
    let world = Arc::new(walled_world());
    let from = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let destination = WorldTile { x: 3202, ..from };
    let (lease, completion) = compact_route_completion(&world, from, destination);
    let view = completion
        .route_view(&lease)
        .expect("same-pack compact route view")
        .expect("completed route");
    let script = frontend_core::walk_map::RouteProjectionView::compact(
        view,
        frontend_core::walk_map::WalkRouteSource::Script,
        9,
        Some(destination),
    );
    let manual_destination = WorldTile { x: 3210, ..from };
    let manual = Route {
        legs: vec![Leg::Walk {
            tiles: vec![manual_destination],
        }],
        dest: manual_destination,
        ticks: 1.0,
    };
    let (route, click) = live_or_walk_paint(
        false,
        (None, None),
        (Some(&manual), Some(manual_destination)),
        Some(script),
    );
    let route = route.expect("script route beats a manual WalkTo fallback");
    assert_eq!(route.projection().destination(), destination);
    assert_eq!(click, Some(destination));

    let settings = NavSettings {
        show_nav_path: true,
        ..NavSettings::default()
    };
    let outputs = SceneRoutePaint::build(&world, Some(route.projection()), Some(from), true, true);
    // The scene painter consumes only outputs. It does not retain the compact
    // route borrow or require the route owner to remain locked or alive.
    drop(completion);
    drop(lease);
    publish_nav_debug(
        &mut client,
        &world,
        Some(outputs),
        &[],
        false,
        click,
        &settings,
        true,
        false,
    );
    let paint = client.nav_debug_paint().expect("focused scene paint");
    assert_eq!(
        paint.path,
        vec![(0, 0, false), (1, 0, false), (2, 0, false)]
    );
    assert_eq!(paint.click, Some((2, 0)));
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
        Some(SceneRoutePaint::build(
            &world,
            Some(frontend_core::walk_map::WalkRoutePath::Legacy(&route)),
            here,
            true,
            true,
        )),
        &[],
        false,
        click,
        &layers,
        true,
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
    let _guard = crate::picker::lock_nav_statics();
    let mut c = paint_client();
    // A 65×65 world (a distinct grid from `walled_world`, so the cached
    // reach bake belongs to this graph): a sealed 1×1 courtyard floor
    // at scene (1,1) and a blocked door-loc seed at scene (2,2).
    let (width, height) = (65usize, 65usize);
    let mut flags = vec![0u32; width * height];
    flags[width + 1] = CollisionFlag::WALK_BLOCK_FLAGS as u32;
    flags[2 * width + 2] = CollisionFlag::WR_GRND as u32;
    let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
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
                player_delta: None,
                takeoff: None,
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
                consumed_req: vec![],
                item_returns: vec![],
                quest_req: vec![],
                varp_req: vec![],
                worn_req: vec![],
                worn_all_req: vec![],
                forbidden_held: vec![],
                forbidden_worn: vec![],
                members_req: false,
                wildy_cap: None,
                quest_gates: None,
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
    publish_nav_debug(&mut c, &world, None, &[], false, None, &layers, true, true);
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
    publish_nav_debug(&mut c, &world, None, &[], false, None, &layers, true, true);
    assert!(c.nav_debug_paint().is_some());
    // Unfocused / skip-paint / renderer-off slots must not linger on a
    // stale paint.
    publish_nav_debug(
        &mut c,
        &world,
        None,
        &[],
        false,
        None,
        &layers,
        false,
        false,
    );
    assert!(
        c.nav_debug_paint().is_none(),
        "a non-drawing slot stores None"
    );
}

#[test]
fn saved_collision_prefs_without_drawing_do_not_load_navflags() {
    // Saved collision/NSEW paint preferences must not pull the flags
    // sidecar until a surface actually draws them.
    let _guard = crate::picker::lock_nav_statics();
    let dir = TestDir::new("flags-draw-gate");
    let path = dir.join("flags.navflags");
    let origin = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let flags = vec![0u32, 0, 0, 0];
    let bytes = nav_bake::pack::encode_flags_sidecar(origin, 1, 1, &flags);
    std::fs::write(&path, &bytes).unwrap();
    let digest = nav_planner::manifest::hash_bytes(&bytes);

    crate::picker::drop_flags_sidecar();
    crate::picker::reset_flags_load_count();
    crate::picker::set_navflags_binding(path, Some(digest), true);

    let mut c = paint_client();
    let world = walled_world();
    let layers = NavSettings {
        collision_fill: true,
        nsew_labels: true,
        ..NavSettings::default()
    };

    // Boot-like: prefs on, no drawing surface.
    publish_nav_debug(
        &mut c,
        &world,
        None,
        &[],
        false,
        None,
        &layers,
        false,
        true, // focused owner, but not drawing
    );
    assert_eq!(
        crate::picker::flags_load_count(),
        0,
        "saved prefs without drawing must not load navflags"
    );
    assert_eq!(
        crate::picker::flags_sidecar_state(),
        crate::picker::FlagsSidecarState::Unloaded
    );
    assert!(c.nav_debug_paint().is_none());

    // Enabling a drawn layer loads exactly once.
    publish_nav_debug(&mut c, &world, None, &[], false, None, &layers, true, true);
    assert_eq!(crate::picker::flags_load_count(), 1);
    assert_eq!(
        crate::picker::flags_sidecar_state(),
        crate::picker::FlagsSidecarState::Applied
    );
    assert!(c.nav_debug_paint().is_some());

    // Second drawn frame does not reload.
    publish_nav_debug(&mut c, &world, None, &[], false, None, &layers, true, true);
    assert_eq!(crate::picker::flags_load_count(), 1);

    // Last drawer off releases ownership.
    publish_nav_debug(&mut c, &world, None, &[], false, None, &layers, false, true);
    assert_eq!(
        crate::picker::flags_sidecar_state(),
        crate::picker::FlagsSidecarState::Unloaded
    );
    assert!(c.nav_debug_paint().is_none());
    crate::picker::drop_flags_sidecar();
}

#[test]
fn remove_only_focused_drawing_slot_releases_flags() {
    // When the last focused slot is rail-removed, no remaining drawer publishes
    // publish_nav_debug — the remove path must drop the flags sidecar.
    let _guard = crate::picker::lock_nav_statics();
    let dir = TestDir::new("flags-last-slot-remove");
    let path = dir.join("flags.navflags");
    let origin = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let flags = vec![0u32, 0, 0, 0];
    let bytes = nav_bake::pack::encode_flags_sidecar(origin, 1, 1, &flags);
    std::fs::write(&path, &bytes).unwrap();
    let digest = nav_planner::manifest::hash_bytes(&bytes);

    crate::picker::drop_flags_sidecar();
    crate::picker::set_navflags_binding(path, Some(digest), true);
    crate::picker::ensure_flags_sidecar();
    assert_eq!(
        crate::picker::flags_sidecar_state(),
        crate::picker::FlagsSidecarState::Applied
    );

    let vault_path = tmp_vault("flags-last-slot-remove.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&vault_path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    s.load("alice");
    assert_eq!(s.focused_name().as_deref(), Some("alice"));
    s.rail_remove("alice");
    assert!(s.focused_name().is_none());
    assert_eq!(
        crate::picker::flags_sidecar_state(),
        crate::picker::FlagsSidecarState::Unloaded,
        "removing the only focused slot must release the flags sidecar"
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
        &trail_world,
        true,
        None,
        &layers,
        true,
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
        &mut c, &world, None, &arrived, false, None, &layers, true, true,
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
        &mut c, &world, None, &revisit, false, None, &layers, true, true,
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

#[test]
fn inactive_slot_retires_stale_client_trail() {
    let mut c = paint_client();
    c.try_move_path = vec![(0, 0), (1, 0), (2, 0)];

    retire_client_trail(&mut c, Some(wt(3201, 3200)));
    assert_eq!(
        c.try_move_path.len(),
        3,
        "an inactive slot keeps a trail while still walking it"
    );
    retire_client_trail(&mut c, Some(wt(3202, 3200)));
    assert!(
        c.try_move_path.is_empty(),
        "arrival retires the producer without paint materialization"
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
                edge: Box::new(TransportEdge {
                    kind: TransportKind::Door,
                    player_delta: None,
                    takeoff: None,
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
                    consumed_req: vec![],
                    item_returns: vec![],
                    quest_req: vec![],
                    varp_req: vec![],
                    worn_req: vec![],
                    worn_all_req: vec![],
                    forbidden_held: vec![],
                    forbidden_worn: vec![],
                    members_req: false,
                    wildy_cap: None,
                    quest_gates: None,
                }),
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
    let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
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
    publish_nav_debug(&mut c, &world, None, &[], false, None, &layers, true, true);
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
        Some(SceneRoutePaint::build(
            &world,
            Some(frontend_core::walk_map::WalkRoutePath::Legacy(&route)),
            None,
            true,
            true,
        )),
        &trail_world,
        false,
        click,
        &layers,
        true,
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
        Some(SceneRoutePaint::build(
            &world,
            Some(frontend_core::walk_map::WalkRoutePath::Legacy(&route)),
            None,
            true,
            true,
        )),
        &trail_world,
        false,
        click,
        &layers,
        true,
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
        Some(SceneRoutePaint::build(
            &world,
            Some(frontend_core::walk_map::WalkRoutePath::Legacy(&route)),
            None,
            true,
            true,
        )),
        &trail_world,
        true,
        click,
        &layers,
        true,
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
                edge: Box::new(TransportEdge {
                    kind: TransportKind::Door,
                    player_delta: None,
                    takeoff: None,
                    at: tiles[300],
                    to: tiles[301],
                    loc_id: 1530,
                    option: 1,
                    ticks: 1,
                    dir: None,
                    open_loc_id: None,
                    skill_req: vec![],
                    item_req: vec![],
                    consumed_req: vec![],
                    item_returns: vec![],
                    quest_req: vec![],
                    varp_req: vec![],
                    worn_req: vec![],
                    worn_all_req: vec![],
                    forbidden_held: vec![],
                    forbidden_worn: vec![],
                    members_req: false,
                    wildy_cap: None,
                    quest_gates: None,
                }),
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
        Some(SceneRoutePaint::build(
            &world,
            Some(frontend_core::walk_map::WalkRoutePath::Legacy(&route)),
            None,
            true,
            true,
        )),
        &[],
        false,
        None,
        &layers,
        true,
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
    s.core.set_play(Some(empty_play()));
    s.core.insert_slot_io(
        "s00",
        SlotIo {
            input: SlotInput::new(),
            pixels: FrameBuf::new(),
        },
    );
    s.core.insert_slot_io(
        "s05",
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
        s.core.play_for_test().unwrap().focused().as_deref(),
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
fn sidecar_cadence_sync_raises_members_not_focus() {
    let mut s = Session::new();
    let a_in = SlotInput::new();
    let b_in = SlotInput::new();
    s.core.insert_slot_io(
        "a",
        SlotIo {
            input: Arc::clone(&a_in),
            pixels: FrameBuf::new(),
        },
    );
    s.core.insert_slot_io(
        "b",
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
    // Pref off returns the 1 fps watch cadence. The saved switch reaches
    // the render mirror when its durable value does (the surface callback).
    s.focus.lock().unwrap().sidecar_50 = false;
    s.sync_sidecar_cadence();
    assert!(!b_in.full_rate());
}

fn tmp_vault(name: &str) -> TestPath {
    TestPath::new(name, "vault.vault")
}

fn profile(username: &str, password: &str, uid: i32) -> FixtureProfile {
    FixtureProfile::new(username, password).uid(uid)
}

#[test]
fn unlock_at_uses_the_given_path() {
    let path = tmp_vault("unlock-at.vault");
    let mut s = Session::new();
    assert!(s.unlock_at(&path, "test-passphrase-01"));
    assert!(s.core.vault_open());
}

/// alice (catalog Thiever) and bob (compiled Gatherer), both auto-login,
/// last focus alice; the vault is locked and no worker threads spawn.
fn multibox_unlock_fixture() -> (Session, TestPath) {
    let path = tmp_vault("multibox-unlock.vault");
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    vault.put(with_assignment(
        profile("alice", "pw", 1).auto_login(true),
        "catalog",
        "Thiever",
        "Thiever",
        None,
    ));
    vault.put(with_assignment(
        profile("bob", "pw", 2).auto_login(true),
        "compiled",
        "Gatherer",
        "Gatherer",
        None,
    ));
    drop(vault);
    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: Some("alice".into()),
        ..Default::default()
    });
    let mut session = Session::new();
    session.persist_ui = false;
    session.core.set_spawn_workers(false);
    (session, path)
}

#[test]
fn multibox_before_unlock_registers_auto_focused_profile() {
    let (mut session, path) = multibox_unlock_fixture();
    hydrate_panel_preferences(&mut session);
    assert!(session.set_multibox(true));
    assert!(session.multibox && session.wall.chooser_open);
    assert!(!session.core.vault_open());
    assert!(session.core.slots().is_empty());
    assert!(session.core.members().is_empty());
    assert!(session.unlock_at(&path, "test-passphrase-01"));
    session.poll_panel_preferences();
    assert_eq!(session.focused_name().as_deref(), Some("alice"));
    assert!(session
        .core
        .play_for_test()
        .unwrap()
        .arm("alice")
        .unwrap()
        .wants_login());
    assert!(
        session.core.fleet().contains("alice"),
        "unlock while MultiBox is already on must register its spawned profile"
    );
    assert_eq!(session.core.script_state("alice"), ScriptRunState::Idle);
    session.pick_profile("bob");
    assert_eq!(session.focused_name().as_deref(), Some("bob"));
    assert_eq!(session.core.slots().len(), 2);
    assert!(session
        .core
        .play_for_test()
        .unwrap()
        .arm("alice")
        .unwrap()
        .wants_login());
    assert!(session.core.fleet().contains("alice"));
    assert!(session.core.fleet().contains("bob"));
}

/// MultiBox off, the unlock focuses and auto-logs in the last focused
/// profile: it runs, so it is a fleet member (the Fleet window lists it
/// loaded) without joining a rail, since there is none.
#[test]
fn single_mode_unlock_makes_the_focused_bot_a_fleet_member() {
    let (mut session, path) = multibox_unlock_fixture();
    hydrate_panel_preferences(&mut session);
    assert!(session.unlock_at(&path, "test-passphrase-01"));
    session.poll_panel_preferences();
    assert_eq!(session.focused_name().as_deref(), Some("alice"));
    assert!(session.core.slots().contains_key("alice"));
    assert!(!session.multibox, "no MultiBox, no rail");
    assert_eq!(session.core.members(), ["alice".to_string()]);
    // The views project the members at the next poll.
    session.core.poll();
    session.fleet_model.refresh(&session.core);
    let alice = session
        .fleet_model
        .rows()
        .iter()
        .find(|row| row.name() == "alice")
        .expect("alice row");
    assert!(
        alice.loaded(),
        "the Fleet window lists the running bot as loaded"
    );
    // A chooser pick of another profile opens its slot the same way.
    session.pick_profile("bob");
    assert_eq!(
        session.core.members(),
        ["alice".to_string(), "bob".to_string()]
    );
}

#[test]
fn o1_panel_late_preferences_publish_order_before_startup_load() {
    let iso = IsolatedStores::enter("o1-panel-late-order");
    let (mut session, vault) = multibox_unlock_fixture();
    let root = iso.dir().join("saved-state");
    let preferences = frontend_core::UiPreferences::open_for_test(root.clone());
    preferences
        .set(
            frontend_core::PROFILE_ORDER_KEY,
            serde_json::json!({"2": 10, "1": 20}),
        )
        .unwrap();
    preferences
        .set("last_focus", serde_json::json!("alice"))
        .unwrap();
    let service = host_play::local_state::LocalStateService::open_process(
        host_play::local_state::StateRoot::new(root),
    );
    session.core.bind_local_state(service);
    session.preferences = session.core.preferences().unwrap();
    session.preferences_ready = false;
    assert!(session.unlock_at(&vault, "test-passphrase-01"));
    assert!(
        session.core.slots().is_empty(),
        "unlock only installs the vault"
    );
    session.preferences.reload().unwrap();
    session.poll_panel_preferences();
    assert_eq!(session.core.profile_names(), ["bob", "alice"]);
    assert_eq!(session.focused_name().as_deref(), Some("alice"));
    session.load("bob");
    assert_eq!(session.core.members(), ["bob", "alice"]);
    let focus = session.focus.lock().unwrap();
    assert_eq!(focus.focused.as_deref(), Some("bob"));
    assert!(session
        .core
        .play_for_test()
        .unwrap()
        .arm("alice")
        .unwrap()
        .wants_login());
}

#[test]
fn o1_panel_warm_unlock_only_installs_the_vault() {
    let _iso = IsolatedStores::enter("o1-panel-warm-order");
    let (mut session, vault) = multibox_unlock_fixture();
    hydrate_panel_preferences(&mut session);
    assert!(session.unlock_at(&vault, "test-passphrase-01"));
    assert!(session.core.slots().is_empty());
    assert!(session.focused_name().is_none());
    session.poll_panel_preferences();
    assert_eq!(session.core.members(), ["alice"]);
    assert_eq!(session.focused_name().as_deref(), Some("alice"));
}

#[test]
fn unlock_defers_automatic_focus_until_preferences_are_ready() {
    let isolated = IsolatedStores::enter("panel-cold-start");
    let (mut session, path) = multibox_unlock_fixture();
    let preferences = session.preferences.share();
    let load_dir = isolated.dir().join("script-load");
    let catalog_dir = isolated.dir().join("script-catalog");
    let quester_dir = isolated.dir().join("quester-paths");
    for directory in [&load_dir, &catalog_dir, &quester_dir] {
        std::fs::create_dir_all(directory).unwrap();
    }
    set_patiently(&preferences, "last_focus", serde_json::json!("alice"));
    set_patiently(&preferences, "capture", serde_json::json!(false));
    set_patiently(&preferences, "sidecar_50", serde_json::json!(true));
    set_patiently(&preferences, "focused_50", serde_json::json!(false));
    set_patiently(
        &preferences,
        "map_bake",
        serde_json::to_value(frontend_core::MapBakeChoice::Always).unwrap(),
    );
    set_patiently(
        &preferences,
        "quester_paths",
        serde_json::json!({
            "enabled": true,
            "folder": quester_dir.to_string_lossy()
        }),
    );
    set_patiently(
        &preferences,
        "script_load_last_dir",
        serde_json::json!(load_dir.to_string_lossy()),
    );
    set_patiently(
        &preferences,
        "script_catalog_last_dir",
        serde_json::json!(catalog_dir.to_string_lossy()),
    );
    seed_ui_state(&crate::ui_state::PanelUiState {
        script_load_last_dir: Some(load_dir.clone()),
        script_catalog_last_dir: Some(catalog_dir.clone()),
        ..Default::default()
    });
    session.preferences_ready = false;
    let folder_draft = isolated.dir().join("uncommitted-folder");
    session.quester_paths_folder_edit = folder_draft.to_string_lossy().into_owned();
    session.quester_paths_folder_dirty = true;

    assert!(session.unlock_at(&path, "test-passphrase-01"));
    assert!(session.core.vault_open());
    assert!(session.core.slots().is_empty());
    assert!(session.focused_name().is_none());
    assert!(session.startup_focus_pending.is_some());
    session.preferences.reload().unwrap();

    patiently(|| {
        session.poll_panel_preferences();
        if session.panel_preferences_ready() {
            Ok(())
        } else {
            Err("not durable within panel preference adoption wait".into())
        }
    })
    .unwrap();

    assert!(session.startup_focus_pending.is_none());
    assert_eq!(session.focused_name().as_deref(), Some("alice"));
    assert!(session.core.slots().contains_key("alice"));
    assert!(!session.draw.capture());
    assert!(session.draw.sidecar_50());
    assert!(!session.draw.focused_50());
    {
        let focus = session.focus.lock().unwrap();
        assert!(!focus.capture);
        assert!(focus.sidecar_50);
        assert!(!focus.focused_50);
    }
    assert_eq!(
        session.map_desk.bake_gate().choice(),
        frontend_core::MapBakeChoice::Always
    );
    assert!(session.quester_paths.settings().enabled());
    assert_eq!(
        session.quester_paths.settings().folder(),
        quester_dir.as_path()
    );
    assert_eq!(
        session.quester_paths_folder_edit,
        folder_draft.to_string_lossy().into_owned()
    );
    #[cfg(feature = "js")]
    assert_eq!(session.script_load_dir, load_dir);
    #[cfg(feature = "js")]
    assert_eq!(session.rs2b0t_catalog_dir, catalog_dir);
    frontend_core::test_support::scripts::reset_quester_registry();
}

#[test]
fn operator_focus_before_preference_adoption_keeps_walk_target() {
    for path in ["select", "chooser", "multibox", "existing", "first-profile"] {
        let _iso = IsolatedStores::enter(&format!("panel-startup-focus-{path}"));
        let (mut session, vault) = multibox_unlock_fixture();
        session.preferences.reload().unwrap();
        session.preferences_ready = false;
        assert!(session.unlock_at(&vault, "test-passphrase-01"));
        assert!(session.startup_focus_pending.is_some());
        match path {
            "select" => session.select("bob"),
            "chooser" => session.pick_profile("bob"),
            "multibox" => {
                session.set_multibox(true);
                session.pick_profile("bob");
            }
            "existing" => {
                session.select("bob");
                session.focus_when_ready(super::StartupFocus::Restore);
                session.focus_existing("bob").unwrap();
            }
            "first-profile" => {
                session.focus_when_ready(super::StartupFocus::SavedProfile("alice".into()));
                session.select("bob");
            }
            _ => unreachable!(),
        }
        assert_eq!(session.focused_name().as_deref(), Some("bob"), "{path}");
        let world = open_world(4, 4);
        session.select_picker_tile(
            &world,
            Tile {
                x: 2,
                z: 2,
                level: 0,
            },
        );
        session.confirm_picker_walk(&world);
        assert!(
            session.panel_preferences_ready(),
            "{path}: WalkTo adopts preferences"
        );
        let report = session
            .core
            .commands()
            .reports()
            .find(|report| report.command() == frontend_core::OperatorCommand::WalkTo)
            .expect("WalkTo command report");
        let bob = session.core.profile_identity_or_synthetic("bob");
        assert!(
            report
                .member(frontend_core::command::Subject::Profile(bob))
                .is_some(),
            "{path}: the command must target the operator's selection: {report:?}"
        );
        assert_eq!(session.focused_name().as_deref(), Some("bob"), "{path}");
        session.poll_panel_preferences();
        assert!(session.startup_focus_pending.is_none(), "{path}");
        assert_eq!(session.focused_name().as_deref(), Some("bob"), "{path}");
    }
}

#[test]
fn walk_preference_adoption_does_not_apply_startup_focus_mid_command() {
    let _iso = IsolatedStores::enter("panel-command-startup-focus");
    let (mut session, vault) = multibox_unlock_fixture();
    session.preferences.reload().unwrap();
    session.preferences_ready = false;
    assert!(session.unlock_at(&vault, "test-passphrase-01"));
    // A captured command context is not an operator focus decision. Keep the
    // automatic restore pending while exercising lazy policy adoption.
    session.set_focus_for_test("bob");
    let world = open_world(4, 4);
    session.select_picker_tile(
        &world,
        Tile {
            x: 2,
            z: 2,
            level: 0,
        },
    );
    session.confirm_picker_walk(&world);
    assert!(session.panel_preferences_ready());
    let report = session
        .core
        .commands()
        .reports()
        .find(|report| report.command() == frontend_core::OperatorCommand::WalkTo)
        .expect("WalkTo command report");
    let bob = session.core.profile_identity_or_synthetic("bob");
    assert!(
        report
            .member(frontend_core::command::Subject::Profile(bob))
            .is_some(),
        "lazy preference adoption must not replace the captured target: {report:?}"
    );
    assert_eq!(session.focused_name().as_deref(), Some("bob"));
    assert!(session.startup_focus_pending.is_some());
    session.poll_panel_preferences();
    assert_eq!(session.focused_name().as_deref(), Some("alice"));
    assert!(session.startup_focus_pending.is_none());
}

#[test]
fn removing_the_focused_bot_cancels_startup_focus() {
    let _iso = IsolatedStores::enter("panel-remove-startup-focus");
    let (mut session, vault) = multibox_unlock_fixture();
    session.preferences.reload().unwrap();
    session.preferences_ready = false;
    assert!(session.unlock_at(&vault, "test-passphrase-01"));
    session.select("alice");
    session.focus_when_ready(super::StartupFocus::SavedProfile("alice".into()));
    session.rail_remove("alice");
    assert!(session.focused_name().is_none());
    session.poll_panel_preferences();
    assert!(
        session.focused_name().is_none(),
        "startup must not reload the removed focus"
    );
    assert!(session.startup_focus_pending.is_none());
}

fn fleet_login_profile(session: &mut Session, name: &str) {
    session.pump_status();
    session.fleet_model.clear_marks_for_test();
    let identity = session.core.profile_identity(name).unwrap();
    session.fleet_model.set_mark_for_test(identity, true);
    session.plan_fleet_action(frontend_core::FleetAction::Login);
    if session.core.fleet_plan().is_some() {
        session.confirm_fleet_plan();
    }
    session.pump_status();
}

#[test]
fn fleet_login_focus_cancels_pending_startup_restore() {
    let _iso = IsolatedStores::enter("panel-fleet-startup-focus");
    let (mut session, vault) = multibox_unlock_fixture();
    session.preferences.reload().unwrap();
    session.preferences_ready = false;
    assert!(session.unlock_at(&vault, "test-passphrase-01"));
    assert!(session.startup_focus_pending.is_some());
    fleet_login_profile(&mut session, "bob");
    assert_eq!(session.core.selected(), Some("bob"));
    assert_eq!(session.focused_name().as_deref(), Some("bob"));
    session.poll_panel_preferences();
    assert_eq!(session.core.selected(), Some("bob"));
    assert_eq!(
        session.focus.lock().unwrap().focused.as_deref(),
        Some("bob")
    );
    assert_eq!(session.core.members(), ["bob"]);
    assert!(session.startup_focus_pending.is_none());
}

#[test]
fn removing_unfocused_restore_target_cancels_startup_restore() {
    for saved_profile in [false, true] {
        let _iso = IsolatedStores::enter("panel-remove-unfocused-startup");
        let (mut session, vault) = multibox_unlock_fixture();
        session.preferences.reload().unwrap();
        session.preferences_ready = false;
        assert!(session.unlock_at(&vault, "test-passphrase-01"));
        fleet_login_profile(&mut session, "bob");
        fleet_login_profile(&mut session, "alice");
        assert_eq!(session.core.selected(), Some("bob"));
        assert!(session.core.members().iter().any(|name| name == "alice"));
        // Keep removal independent of the Fleet selection cancellation.
        session.focus_when_ready(if saved_profile {
            super::StartupFocus::SavedProfile("alice".into())
        } else {
            super::StartupFocus::Restore
        });
        session.rail_remove("alice");
        assert!(!session.core.members().iter().any(|name| name == "alice"));
        session.poll_panel_preferences();
        assert_eq!(session.core.members(), ["bob"]);
        assert_eq!(session.core.selected(), Some("bob"));
        assert!(session.startup_focus_pending.is_none());
    }
}

#[test]
fn deleting_pending_saved_profile_cancels_startup_restore() {
    let _iso = IsolatedStores::enter("panel-delete-startup-focus");
    let (mut session, vault) = multibox_unlock_fixture();
    session.preferences.reload().unwrap();
    session.preferences_ready = false;
    assert!(session.unlock_at(&vault, "test-passphrase-01"));
    session.focus_when_ready(super::StartupFocus::SavedProfile("alice".into()));
    assert!(session.vault_remove("alice"));
    assert!(!session.profile_names().iter().any(|name| name == "alice"));
    session.poll_panel_preferences();
    assert!(session.core.members().is_empty());
    assert!(session.core.selected().is_none());
    assert!(session.focus.lock().unwrap().focused.is_none());
    assert!(session.startup_focus_pending.is_none());
}

#[test]
fn deleting_other_profile_keeps_pending_saved_profile_restore() {
    let _iso = IsolatedStores::enter("panel-delete-other-startup-focus");
    let (mut session, vault) = multibox_unlock_fixture();
    session.preferences.reload().unwrap();
    session.preferences_ready = false;
    assert!(session.unlock_at(&vault, "test-passphrase-01"));
    session.focus_when_ready(super::StartupFocus::SavedProfile("bob".into()));
    assert!(session.vault_remove("alice"));
    assert!(session.startup_focus_pending.is_some());
    session.poll_panel_preferences();
    assert_eq!(session.core.selected(), Some("bob"));
    assert_eq!(
        session.focus.lock().unwrap().focused.as_deref(),
        Some("bob")
    );
    assert_eq!(session.core.members(), ["bob"]);
}

#[test]
fn multibox_profile_pick_restores_that_profiles_script_heading() {
    let (mut session, path) = multibox_unlock_fixture();
    hydrate_panel_preferences(&mut session);
    assert!(session.unlock_at(&path, "test-passphrase-01"));
    session.poll_panel_preferences();
    let thiever = Some(ScriptChoice::loaded(CardSource::Catalog, "Thiever"));
    let gatherer = Some(ScriptChoice::compiled(
        CompiledCardId::lookup("Gatherer").unwrap(),
    ));
    assert_eq!(session.script_sel, thiever, "unlock shows alice's own card");
    session.pick_profile("bob");
    assert_eq!(session.core.selected(), Some("bob"));
    assert_eq!(session.tv_name().as_deref(), Some("bob"));
    assert_eq!(
        session.script_sel, gatherer,
        "a MultiBox row pick must not leave the outgoing profile's card bound to the incoming profile"
    );
    // Both directions, onto profiles that are already loaded.
    session.pick_profile("alice");
    assert_eq!(session.script_sel, thiever);
    session.pick_profile("bob");
    assert_eq!(session.script_sel, gatherer);
    assert!(session
        .core
        .last_operation()
        .is_none_or(|op| op.action() != frontend_core::ActionKind::ScriptStart));
}

#[cfg(feature = "js")]
#[test]
fn multibox_roundtrip_keeps_profile_assignments_and_settings_isolated() {
    let dir = TestDir::new("multibox-roundtrip");
    let path = dir.join("vault");
    let legacy_path = dir.join("script-settings.json");
    let legacy_bytes = br#"{"catalog:Thiever":{"target":"legacy-only","radius":99}}"#;
    std::fs::write(&legacy_path, legacy_bytes).unwrap();
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    for (i, (name, kind, card)) in [
        ("alice", "catalog", "Thiever"),
        ("bob", "compiled", "Gatherer"),
    ]
    .into_iter()
    .enumerate()
    {
        let bag = serde_json::json!({"owner": name, "radius": i + 7})
            .as_object()
            .unwrap()
            .clone();
        vault.put(with_settings_bag(
            with_assignment(
                profile(name, "pw", i as i32 + 1).auto_login(true),
                kind,
                card,
                card,
                None,
            ),
            &assignment_key(kind, card),
            bag,
        ));
    }
    let expected = vault_debug_rows(&vault);
    drop(vault);
    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: Some("alice".into()),
        ..Default::default()
    });
    let mut session = Session::new();
    session.persist_ui = false;
    session.core.set_spawn_workers(false);
    session.scripts.set_legacy_store(legacy_path.clone());
    hydrate_panel_preferences(&mut session);
    session.set_multibox(true);
    assert!(session.unlock_at(&path, "test-passphrase-01"));
    session.poll_panel_preferences();

    for (step, selected) in [
        ("unlock", "alice"),
        ("pick other", "bob"),
        ("pick first back", "alice"),
        ("pick other back", "bob"),
    ] {
        if step != "unlock" {
            session.pick_profile(selected);
        }
        session.pump_status();
        assert_eq!(session.core.selected(), Some(selected));
        let disk = VaultFixture::unlock(&path, "test-passphrase-01");
        assert_eq!(
            core_vault_debug_rows(&session.core),
            expected,
            "{step}: in-memory settings must stay owned by their profile"
        );
        assert_eq!(
            vault_debug_rows(&disk),
            expected,
            "{step}: persisted settings must stay owned by their profile"
        );
        for name in ["alice", "bob"] {
            assert!(session.scripts.pending_browse(name).is_none());
            if session.core.slots().contains_key(name) {
                assert_eq!(
                    session.core.script_state(name),
                    ScriptRunState::Idle,
                    "{step}: no script may start without Start"
                );
                assert!(session
                    .core
                    .play_for_test()
                    .unwrap()
                    .arm(name)
                    .unwrap()
                    .wants_login());
            }
        }
        assert_eq!(std::fs::read(&legacy_path).unwrap(), legacy_bytes);
    }
    assert!(session.core.fleet().contains("alice"));
    assert!(session.core.fleet().contains("bob"));
}

#[test]
fn wrong_pass_does_not_delete_or_replace_the_vault() {
    let path = tmp_vault("wrong-pass.vault");
    let mut s = Session::new();
    assert!(s.unlock_at(&path, "test-passphrase-01"));
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    drop(s);

    let mut s = Session::new();
    assert!(!s.unlock_at(&path, "nope"));
    assert!(!s.core.vault_open());
    assert!(path.is_file());
    let v = VaultFixture::unlock(&path, "test-passphrase-01");
    assert!(v.profile("alice").is_some());
}

#[test]
fn reset_vault_at_refuses_while_unlocked() {
    let path = tmp_vault("reset-locked.vault");
    let mut s = Session::new();
    assert!(s.unlock_at(&path, "test-passphrase-01"));
    assert!(!s.reset_vault_at(&path));
    assert!(path.is_file());
    assert!(s.core.vault_open());
}

#[test]
fn reset_vault_at_deletes_while_locked() {
    let path = tmp_vault("reset-ok.vault");
    let mut s = Session::new();
    assert!(s.unlock_at(&path, "test-passphrase-01"));
    clear_vault(&mut s.core);
    assert!(s.reset_vault_at(&path));
    assert!(!path.exists());
    assert!(s.unlock_at(&path, "new-passphrase-02"));
}

#[test]
fn session_starts_with_renderer_on_capture_on_by_default() {
    seed_ui_state(&crate::ui_state::PanelUiState::default());
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
fn picker_walk_zone_policy_is_blocking_by_default_and_all_only_when_opted_in() {
    let mut session = Session::new();
    hydrate_panel_preferences(&mut session);
    assert_eq!(
        session.ui.nav.danger_level(),
        frontend_core::DangerLevel::WhenSurvivable
    );
    assert_eq!(
        session.ui.nav.walk_globals().effective_danger_level(),
        frontend_core::DangerLevel::Never
    );
    assert_eq!(
        frontend_core::walk_permissions::danger_routing_label(session.ui.nav.danger_level()),
        "When survivable (not available yet: acts as Never)"
    );
    assert_eq!(session.walk_find_options().zones, ZoneExempt::NONE);
    session.route_through_zones = true;
    assert!(session.walk_find_options().zones.is_all());
}

#[test]
fn manual_walk_options_refresh_durable_peer_changes_and_clear_one_shot() {
    let _isolated = IsolatedStores::enter("manual-walk-permissions");
    let mut session = Session::new();
    // The durable writes are a peer frontend's: a second service on the
    // session's files, seen once the session's preferences re-read.
    let peer = thread_preferences();
    let peer_sets_danger_zones = |session: &Session, enabled: bool| {
        frontend_core::nav_prefs::write_nav_preference_for_test(
            &peer,
            frontend_core::NavPreference::AllowDangerZones,
            enabled,
        )
        .map(|()| enabled)
        .unwrap();
        session.preferences.reload().unwrap();
    };
    session.ui.nav.allow_danger_zones = false;
    peer_sets_danger_zones(&session, false);
    assert_eq!(session.walk_find_options().zones, ZoneExempt::NONE);

    peer_sets_danger_zones(&session, true);
    let globally_allowed = session.walk_find_options();
    assert_eq!(globally_allowed.zones, ZoneExempt::all());
    assert!(session.ui.nav.allow_danger_zones);

    session.route_through_zones = true;
    assert_eq!(
        session.walk_find_options().zones,
        ZoneExempt::all(),
        "the global projection and a one-shot request permit this admission"
    );
    assert!(
        !session.route_through_zones,
        "the global grant consumes the one-shot"
    );

    peer_sets_danger_zones(&session, false);
    assert_eq!(session.walk_find_options().zones, ZoneExempt::NONE);
    assert!(!session.ui.nav.allow_danger_zones);

    std::fs::write(session.preferences.path(), b"{corrupt").unwrap();
    session
        .preferences
        .reload()
        .expect_err("corrupt preferences are refused");
    assert_eq!(session.walk_find_options().zones, ZoneExempt::NONE);
    assert!(!session.ui.nav.allow_danger_zones);
}

#[test]
fn restored_enabled_folder_loads_when_game_data_becomes_ready() {
    let iso = IsolatedStores::enter("panel-quester-paths-startup");
    let folder = iso.dir().join("paths");
    std::fs::create_dir_all(&folder).unwrap();
    for (id, name) in [("cook", "Folder Cook"), ("fresh-draft", "Fresh Draft")] {
        let mut document: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../script/paths/289/cook.json"
        )))
        .unwrap();
        document["id"] = id.into();
        document["display_name"] = name.into();
        std::fs::write(
            folder.join(format!("{id}.json")),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
    }
    let root = thread_state_root();
    crate::test_support::write_private_fixture(
        root.preferences_file(),
        serde_json::to_vec(&serde_json::json!({
            "quester_paths": {
                "enabled": true,
                "folder": folder.to_string_lossy()
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let mut session = Session::new();
    hydrate_panel_preferences(&mut session);
    let selected = api::selected::FamilyPreparation::run(|_| {
        api::game_data::for_revision(api::selected::ClientRevision::R289)
            .expect("selected 289 game data")
    })
    .expect("spawn selected data worker")
    .join()
    .expect("selected data worker");

    session.advance_quester_paths_reload(Some(Arc::clone(&selected)));
    assert!(
        session.quester_paths.is_running(),
        "saved folder starts a reload without a user action"
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    while session.quester_paths.is_running() {
        assert!(
            Instant::now() < deadline,
            "startup Path reload did not finish"
        );
        session.advance_quester_paths_reload(Some(Arc::clone(&selected)));
        thread::yield_now();
    }

    let report = session
        .quester_paths
        .notice_text()
        .expect("startup validation report");
    assert!(
        !report.is_empty(),
        "startup reload reports its result without a user action"
    );
    frontend_core::test_support::scripts::reset_quester_registry();
}

#[test]
fn quest_paths_checkbox_keeps_saved_folder_after_failed_folder_save() {
    let iso = IsolatedStores::enter("panel-quester-paths-failed-folder-then-toggle");
    let saved = iso.dir().join("saved-paths");
    let uncommitted = iso.dir().join("uncommitted-paths");
    std::fs::create_dir_all(&saved).unwrap();
    let root = thread_state_root();
    crate::test_support::write_private_fixture(
        root.preferences_file(),
        serde_json::to_vec(&serde_json::json!({
            "quester_paths": {
                "enabled": false,
                "folder": saved.to_string_lossy()
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let mut session = Session::new();
    hydrate_panel_preferences(&mut session);
    assert_eq!(session.quester_paths.settings().folder(), saved.as_path());
    assert!(!session.quester_paths.settings().enabled());

    session.quester_paths_folder_edit = uncommitted.to_string_lossy().into_owned();
    // A refused preferences file (larger than the bound) refuses writes.
    std::fs::write(
        session.preferences.path(),
        vec![b' '; host_play::local_state::MAX_PREFERENCES_BYTES as usize + 1],
    )
    .unwrap();
    assert!(session.preferences.reload().is_err());
    let failed = session.persist_quester_paths(frontend_core::QuesterPathsView::new(
        false,
        uncommitted.clone(),
    ));
    assert!(failed.is_err(), "{failed:?}");
    assert_eq!(session.quester_paths.settings().folder(), saved.as_path());
    assert_eq!(
        session.quester_paths_folder_edit,
        uncommitted.to_string_lossy()
    );

    std::fs::write(
        session.preferences.path(),
        serde_json::to_vec(&serde_json::json!({
            "quester_paths": {
                "enabled": false,
                "folder": saved.to_string_lossy()
            }
        }))
        .unwrap(),
    )
    .unwrap();
    session.preferences.reload().unwrap();

    session
        .persist_quester_paths_change(true, false)
        .expect("checkbox toggle saves enabled with the already-saved folder");
    await_preference(
        &mut session,
        "quester_paths",
        serde_json::json!({
            "enabled": true,
            "folder": saved.to_string_lossy()
        }),
    );
    assert!(session.quester_paths.settings().enabled());
    assert_eq!(session.quester_paths.settings().folder(), saved.as_path());
    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(session.preferences.path()).unwrap()).unwrap();
    assert_eq!(
        document["quester_paths"]["folder"],
        saved.to_string_lossy().as_ref()
    );
    assert_eq!(document["quester_paths"]["enabled"], true);
    frontend_core::test_support::scripts::reset_quester_registry();
}

#[cfg(unix)]
#[test]
fn quest_paths_refuses_a_non_utf8_folder_instead_of_saving_it_lossy() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let iso = IsolatedStores::enter("panel-quester-paths-non-utf8");
    let root = thread_state_root();
    crate::test_support::write_private_fixture(root.preferences_file(), b"{}").unwrap();

    let mut bytes = iso.dir().to_path_buf().into_os_string().into_vec();
    bytes.extend_from_slice(b"/paths-\xff");
    let folder = PathBuf::from(OsString::from_vec(bytes));
    let mut session = Session::new();
    hydrate_panel_preferences(&mut session);
    let saved = session.quester_paths.settings().folder().to_path_buf();
    // The saved source is what becomes active, and a JSON document cannot
    // hold this folder: it is refused at its setting, never saved as another
    // path, and the prior source stays active.
    assert!(session
        .persist_quester_paths(frontend_core::QuesterPathsView::new(false, folder.clone()))
        .is_err());
    assert!(session
        .core
        .setting_failure(frontend_core::Setting::QuesterPaths)
        .is_some_and(|text| text.starts_with("Not saved: ") && text.contains("UTF-8")));
    assert_eq!(session.quester_paths.settings().folder(), saved.as_path());

    // The checkbox then saves only its switch, with the saved folder.
    session
        .persist_quester_paths_change(true, false)
        .expect("checkbox toggle");
    await_preference(
        &mut session,
        "quester_paths",
        serde_json::json!({"enabled": true}),
    );
    assert!(session.quester_paths.settings().enabled());
    assert_eq!(session.quester_paths.settings().folder(), saved.as_path());
    frontend_core::test_support::scripts::reset_quester_registry();
}

#[test]
fn walk_permissions_projection_follows_a_peer_write() {
    let _isolated = IsolatedStores::enter("walk-permissions-peer-refresh");
    let mut session = Session::new();
    // The other frontend: a second service on the same files.
    let peer = thread_preferences();
    frontend_core::nav_prefs::write_nav_preference_for_test(
        &peer,
        frontend_core::NavPreference::AllowTeleports,
        false,
    )
    .map(|()| false)
    .unwrap();
    session.preferences.reload().unwrap();
    session.refresh_walk_permissions();
    assert!(!session.walk_permissions.globals().allow_teleports());

    // Frames with no write in between keep the projection as it was.
    let before = session.walk_permissions;
    for _ in 0..3 {
        session.refresh_walk_permissions();
        assert_eq!(session.walk_permissions, before);
    }

    // The peer's write lands next to a key the panel knows nothing about;
    // the projection follows once the session's preferences re-read.
    patiently(|| {
        frontend_core::nav_prefs::write_nav_preference_for_test(
            &peer,
            frontend_core::NavPreference::AllowTeleports,
            true,
        )
        .map(|()| true)
        .map(|_| ())
        .map_err(|error| error.to_string())
    })
    .unwrap();
    set_patiently(
        &peer,
        "peer_marker",
        serde_json::json!("written by the other frontend"),
    );
    session.preferences.reload().unwrap();
    session.refresh_walk_permissions();
    assert!(session.walk_permissions.globals().allow_teleports());
    assert!(
        session.ui.nav.allow_teleports,
        "the edit buffer follows the durable projection"
    );
    assert_eq!(
        session.preferences.value("peer_marker"),
        Some(serde_json::json!("written by the other frontend"))
    );
}

#[test]
fn map_rows_settle_empty_release_and_wrong_state_decline_explicitly() {
    let iso = IsolatedStores::enter("map-row-guard");
    let mut session = Session::new();
    let released = session
        .submit(
            frontend_core::OperatorCommand::ReleaseMapDemand,
            frontend_core::Targets::Application,
        )
        .expect("submit empty map release");
    let terminal = session
        .core
        .commands()
        .report(released.request())
        .and_then(|report| report.member(frontend_core::command::Subject::Application))
        .and_then(|member| member.terminal());
    assert_eq!(
        terminal,
        Some(&api::identity::Settlement::Completed(
            frontend_core::command::Completion::Skipped(
                frontend_core::command::SkipReason::NotReady,
            ),
        ))
    );

    let declined = session
        .submit(
            frontend_core::OperatorCommand::DeclineMapBake,
            frontend_core::Targets::Application,
        )
        .expect("submit decline without a prompt");
    let decline_terminal = session
        .core
        .commands()
        .report(declined.request())
        .and_then(|report| report.member(frontend_core::command::Subject::Application))
        .and_then(|member| member.terminal());
    assert_eq!(
        decline_terminal,
        Some(&api::identity::Settlement::Failed(
            frontend_core::command::CommandFailure::Rejected(
                frontend_core::command::Rejection::NotApplicable,
            ),
        ))
    );
    assert_eq!(
        session.core.command_failure(declined.request()),
        Some("map bake prompt is not asking")
    );
    let _iso = iso;
}

fn bind_picker_session(s: &mut Session, world: &NavWorld, origin: Tile) -> MapFixture {
    let fixture = MapFixture::new(world, "local-289");
    let play = fixture.play(origin);
    s.launcher
        .install_template(ProfileTemplate::from_host(Arc::clone(&fixture.template)));
    s.set_focus_for_test("alice");
    s.core.set_play(Some(play));
    s.statuses.refresh_from_play(&s.core);
    fixture
}

/// Frontend vocabulary for a nav tile asserted against route state.
fn world_tile(tile: Tile) -> WorldTile {
    WorldTile {
        x: tile.x,
        z: tile.z,
        level: tile.level,
    }
}

fn confirm_map_walk(s: &mut Session, world: &NavWorld, origin: Tile, dest: Tile) -> bool {
    let _fixture = bind_picker_session(s, world, origin);
    s.select_picker_tile(world, dest);
    s.confirm_picker_walk(world)
}

#[test]
fn picker_walk_receipts_reach_process_and_slot_logs_once() {
    use frontend_core::log::{global, LogScope, LogView};
    let log = global();
    let mut s = Session::new();
    let world = open_world(20, 20);
    let origin = Tile {
        x: 1,
        z: 1,
        level: 0,
    };
    let destination = Tile {
        x: 17,
        z: 18,
        level: 0,
    };
    s.select_picker_tile(&world, destination);
    assert!(!s.confirm_picker_walk(&world));
    let mut process = LogView::new(LogScope::Process);
    process.edit_filter(|filter| filter.set_text("WalkTo origin=unknown destination=(17,18,0)"));
    log.refresh(&mut process);
    assert_eq!(process.len(), 1, "{}", process.to_text());
    assert!(process.rows()[0].message().contains("refused=NoOrigin"));
    assert_eq!(process.rows()[0].slot(), None);

    let _fixture = bind_picker_session(&mut s, &world, origin);
    let name = "panel-walk-receipt";
    push_session_slot(&mut s, name, origin, None, true, true);
    s.statuses.refresh_from_play(&s.core);
    s.core.play_mut_for_test().unwrap().focus(name);
    s.set_focus_for_test(name);
    s.select_picker_tile(&world, destination);
    assert!(s.confirm_picker_walk(&world), "{:?}", s.error);
    let mut slot = LogView::new(LogScope::slot(name));
    slot.edit_filter(|filter| filter.set_text("WalkTo "));
    log.refresh(&mut slot);
    assert_eq!(slot.len(), 1, "{}", slot.to_text());
    let message = slot.rows()[0].message();
    assert!(message.contains("destination=(17,18,0)"), "{message}");
    assert!(message.contains("success legs="), "{message}");
    assert!(message.contains("members_source=unknown "), "{message}");
}

#[test]
fn picker_without_observed_origin_refuses_and_consumes_selection() {
    let mut s = Session::new();
    let world = open_world(3, 3);
    s.select_picker_tile(
        &world,
        Tile {
            x: 2,
            z: 2,
            level: 0,
        },
    );
    assert_eq!(s.map_desk.walk_dest(), None);
    assert!(!s.confirm_picker_walk(&world));
    assert_eq!(s.map_desk.walk_dest(), None);
    assert!(!s.core.walk_route_armed("alice"));
    assert!(s.map_model.pending().is_none());
    assert!(!s.confirm_picker_walk(&world));
}

#[test]
fn picker_select_then_confirm_arms_once_and_advances_route_generation() {
    let mut s = Session::new();
    let world = open_world(3, 3);
    let origin = Tile {
        x: 0,
        z: 1,
        level: 0,
    };
    let dest = Tile {
        x: 2,
        z: 2,
        level: 0,
    };
    let _fixture = bind_picker_session(&mut s, &world, origin);
    let generation = s.route_gen();
    assert_eq!(s.select_picker_tile(&world, dest), Some(dest));
    assert_eq!(s.map_desk.walk_dest(), None);
    assert!(!s.core.walk_route_armed("alice"));
    assert!(s.confirm_picker_walk(&world));
    assert_eq!(s.map_desk.walk_dest(), Some(dest));
    assert!(s.error.is_none());
    assert!(s.map_model.pending().is_none());
    let armed_generation = s.route_gen();
    assert_ne!(armed_generation, generation);
    assert_eq!(
        s.core.manual_walk_route("alice").unwrap().dest,
        world_tile(dest)
    );
    assert!(!s.confirm_picker_walk(&world));
    assert_eq!(s.route_gen(), armed_generation);
}

fn push_session_slot(
    s: &mut Session,
    name: &str,
    origin: Tile,
    world: Option<u16>,
    connected: bool,
    ingame: bool,
) {
    s.core
        .play_mut_for_test()
        .unwrap()
        .attach_arm(name, SlotArm::new(1, false));
    s.core
        .play_for_test()
        .unwrap()
        .statuses
        .lock()
        .unwrap()
        .push(SlotStatus {
            username: name.into(),
            world,
            connected,
            ingame,
            tile_x: origin.x,
            tile_z: origin.z,
            tile_level: origin.level,
            ..Default::default()
        });
}

#[test]
fn picker_focus_switch_walks_the_newly_focused_bot() {
    let mut s = Session::new();
    let world = open_world(3, 3);
    let origin = Tile {
        x: 0,
        z: 1,
        level: 0,
    };
    let dest = Tile {
        x: 2,
        z: 2,
        level: 0,
    };
    let _fixture = bind_picker_session(&mut s, &world, origin);
    push_session_slot(
        &mut s,
        "bob",
        Tile {
            x: 1,
            z: 1,
            level: 0,
        },
        None,
        true,
        true,
    );
    s.statuses.refresh_from_play(&s.core);
    s.core.fleet_mut().add("alice");
    s.core.fleet_mut().add("bob");
    assert_eq!(s.select_picker_tile(&world, dest), Some(dest));
    s.set_focus_for_test("bob");
    s.core.play_mut_for_test().unwrap().focus("bob");
    assert!(s.map_model.pending().is_some());
    assert!(s.confirm_picker_walk(&world));
    assert!(s.error.is_none());
    assert_eq!(
        s.core.manual_walk_route("bob").unwrap().dest,
        world_tile(dest)
    );
    assert!(!s.core.walk_route_armed("alice"));
}

/// A session whose vault holds `names` (uids 1..), so the shared marked
/// commands can resolve the fleet marks to profiles.
fn session_with_vault(file: &str, names: &[&str]) -> Session {
    let path = tmp_vault(file);
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    for (i, name) in names.iter().enumerate() {
        vault_upsert(&mut s.core, profile(name, "pw", 1 + i as i32));
    }
    s
}

#[test]
fn picker_group_walk_several_slots_reports_like_start_all() {
    use frontend_core::walk_map::WalkExclude;
    let mut s = session_with_vault(
        "group-walk-several.vault",
        &["alice", "bob", "logged-out", "nopos"],
    );
    let world = open_world(3, 3);
    let origin = Tile {
        x: 0,
        z: 1,
        level: 0,
    };
    let dest = Tile {
        x: 2,
        z: 2,
        level: 0,
    };
    let _fixture = bind_picker_session(&mut s, &world, origin);
    push_session_slot(
        &mut s,
        "bob",
        Tile {
            x: 1,
            z: 1,
            level: 0,
        },
        None,
        true,
        true,
    );
    push_session_slot(&mut s, "logged-out", origin, None, false, false);
    push_session_slot(
        &mut s,
        "nopos",
        Tile {
            x: 0,
            z: 0,
            level: 0,
        },
        None,
        true,
        true,
    );
    s.statuses.refresh_from_play(&s.core);
    for name in ["alice", "bob", "logged-out", "nopos"] {
        s.core.fleet_mut().add(name);
    }
    assert_eq!(s.select_picker_tile(&world, dest), Some(dest));
    s.refresh_walk_send();
    s.set_walk_send_mode(super::WalkSendMode::Group);
    s.walk_send_all_eligible();
    let rows: Vec<(&str, bool, Option<WalkExclude>)> = s
        .walk_send
        .rows()
        .iter()
        .map(|row| {
            (
                row.name(),
                row.checked(s.fleet_model.marks()),
                row.status.exclusion(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("alice", true, None),
            ("bob", true, None),
            ("logged-out", false, Some(WalkExclude::NotLoggedIn)),
            ("nopos", false, Some(WalkExclude::NoPosition)),
        ]
    );
    assert_eq!(s.walk_send.walk_label(), "Walk 2 bots");
    assert!(s.confirm_picker_group_walk(&world));
    assert!(s.map_model.pending().is_none());
    assert_eq!(s.map_desk.walk_dest(), Some(dest));
    assert_eq!(
        s.core.manual_walk_route("alice").unwrap().dest,
        world_tile(dest)
    );
    assert_eq!(
        s.core.manual_walk_route("bob").unwrap().dest,
        world_tile(dest)
    );
    assert!(!s.core.walk_route_armed("logged-out"));
    assert!(!s.core.walk_route_armed("nopos"));
}

/// The picker's group Walk goes through the shared command: a marked bot
/// that cannot walk is named with its reason instead of silently dropping
/// out of the count, and the bots that can walk still get the tile.
#[test]
fn picker_group_walk_names_marked_bots_that_cannot_walk() {
    let mut s = session_with_vault(
        "group-walk-marked.vault",
        &["alice", "bob", "logged-out", "nopos"],
    );
    let world = open_world(3, 3);
    let origin = Tile {
        x: 0,
        z: 1,
        level: 0,
    };
    let dest = Tile {
        x: 2,
        z: 2,
        level: 0,
    };
    let _fixture = bind_picker_session(&mut s, &world, origin);
    push_session_slot(
        &mut s,
        "bob",
        Tile {
            x: 1,
            z: 1,
            level: 0,
        },
        None,
        true,
        true,
    );
    push_session_slot(&mut s, "logged-out", origin, None, false, false);
    push_session_slot(
        &mut s,
        "nopos",
        Tile {
            x: 0,
            z: 0,
            level: 0,
        },
        None,
        true,
        true,
    );
    s.statuses.refresh_from_play(&s.core);
    for name in ["alice", "bob", "logged-out", "nopos"] {
        s.core.fleet_mut().add(name);
        let identity = s.fleet_identity(name);
        s.fleet_model.set_mark_for_test(identity, true);
    }
    assert_eq!(s.select_picker_tile(&world, dest), Some(dest));
    s.refresh_walk_send();
    s.set_walk_send_mode(super::WalkSendMode::Group);
    assert!(s.confirm_picker_group_walk(&world));
    assert_eq!(
        s.core.manual_walk_route("alice").unwrap().dest,
        world_tile(dest)
    );
    assert_eq!(
        s.core.manual_walk_route("bob").unwrap().dest,
        world_tile(dest)
    );
    assert!(!s.core.walk_route_armed("logged-out"));
    assert!(!s.core.walk_route_armed("nopos"));
}

/// The Fleet window's Walk opens the picker in group mode checked from the
/// marks alone: an eligible bot that is not marked is not added.
#[test]
fn fleet_walk_opens_the_picker_in_group_mode_for_the_marks_only() {
    let mut s = session_with_vault("fleet-walk-open.vault", &["alice", "bob"]);
    let world = open_world(3, 3);
    let origin = Tile {
        x: 0,
        z: 1,
        level: 0,
    };
    let _fixture = bind_picker_session(&mut s, &world, origin);
    push_session_slot(
        &mut s,
        "bob",
        Tile {
            x: 1,
            z: 1,
            level: 0,
        },
        None,
        true,
        true,
    );
    s.statuses.refresh_from_play(&s.core);
    for name in ["alice", "bob"] {
        s.core.fleet_mut().add(name);
    }
    let alice = s.fleet_identity("alice");
    s.fleet_model.set_mark_for_test(alice, true);

    s.open_walkto_for_fleet();

    assert!(s.walkto_open);
    assert_eq!(s.walk_send.mode, super::WalkSendMode::Group);
    let checked: Vec<(&str, bool)> = s
        .walk_send
        .rows()
        .iter()
        .map(|row| (row.name(), row.checked(s.fleet_model.marks())))
        .collect();
    assert_eq!(checked, [("alice", true), ("bob", false)]);
    assert_eq!(s.walk_send.walk_label(), "Walk 1 bots");
    assert_eq!(s.fleet_model.marks().len(), 1, "a mark is never added");
}

/// WalkTo's Send keeps no selection of its own: the Walk button's count reads
/// the shared marked-bots set every frame, so a mark made in the rail, Grid or
/// Fleet while the picker is open shows at once, without pressing Send.
#[test]
fn walk_count_follows_marks_made_elsewhere_while_the_picker_is_open() {
    let _guard = crate::test_support::imgui_context_guard();
    let _statics = crate::picker::lock_nav_statics();
    let mut s = session_with_vault("walk-live-count.vault", &["alice", "bob"]);
    let world = open_world(3, 3);
    let origin = Tile {
        x: 0,
        z: 1,
        level: 0,
    };
    let _fixture = bind_picker_session(&mut s, &world, origin);
    push_session_slot(
        &mut s,
        "bob",
        Tile {
            x: 1,
            z: 1,
            level: 0,
        },
        None,
        true,
        true,
    );
    s.statuses.refresh_from_play(&s.core);
    for name in ["alice", "bob"] {
        s.core.fleet_mut().add(name);
    }
    let (alice, bob) = (s.fleet_identity("alice"), s.fleet_identity("bob"));
    s.fleet_model.set_mark_for_test(alice, true);
    s.open_walkto_for_fleet();
    assert_eq!(s.walk_send.walk_label(), "Walk 1 bots");

    let mut ctx = crate::test_support::imgui_context();
    let mut label_after_frame = |s: &mut Session| {
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        {
            let mut open = true;
            let mut map = crate::walk_map::WalkMapRenderer::new();
            crate::picker::picker_map_window(ctx.frame(), s, &world, &mut open, None, &mut map);
        }
        ctx.render();
        s.walk_send.walk_label().to_string()
    };
    assert_eq!(label_after_frame(&mut s), "Walk 1 bots");
    // Another surface marks bob; Send was not pressed and the list was not
    // rebuilt.
    s.fleet_model.set_mark_for_test(bob, true);
    assert_eq!(label_after_frame(&mut s), "Walk 2 bots");
    s.fleet_model.set_mark_for_test(alice, false);
    assert_eq!(label_after_frame(&mut s), "Walk 1 bots");
    s.fleet_model.clear_marks_for_test();
    assert_eq!(label_after_frame(&mut s), "Walk 0 bots");
}

/// The 50 fps choices are saved when changed and restored at the next launch;
/// a preferences file without them behaves as it did before they were saved
/// (sidecar off, focused on).
#[test]
fn fifty_fps_choices_are_saved_and_restored() {
    let mut fresh = Session::new();
    hydrate_panel_preferences(&mut fresh);
    {
        let f = fresh.focus.lock().unwrap();
        assert!(!f.sidecar_50, "a missing key: sidecar off");
        assert!(f.focused_50, "a missing key: focused on, as before");
    }
    let mut s = Session::new();
    hydrate_panel_preferences(&mut s);
    s.set_sidecar_50(true);
    s.set_focused_50(false);
    await_preference(&mut s, "sidecar_50", serde_json::json!(true));
    await_preference(&mut s, "focused_50", serde_json::json!(false));
    let saved = crate::ui_state::load(&s.preferences);
    assert!(
        saved.sidecar_50 && !saved.focused_50,
        "written to the preferences"
    );

    let mut next = Session::new();
    hydrate_panel_preferences(&mut next);
    {
        let f = next.focus.lock().unwrap();
        assert!(f.sidecar_50, "the next launch restores sidecar 50 fps");
        assert!(!f.focused_50, "and focused 50 fps off");
    }
}

/// A restored 50 fps setting adds no work for a slot that isn't drawing: with
/// only-render-selected on (every launch), the rail and Grid members stay at
/// the watch cadence, and with the renderer off nothing runs at 50 Hz.
#[test]
fn restored_50_fps_costs_nothing_for_slots_that_do_not_draw() {
    seed_ui_state(&crate::ui_state::PanelUiState {
        sidecar_50: true,
        focused_50: true,
        ..Default::default()
    });
    let mut s = Session::new();
    let inputs: Vec<_> = ["a", "b", "c"]
        .into_iter()
        .map(|name| {
            let input = SlotInput::new();
            s.core.insert_slot_io(
                name,
                SlotIo {
                    input: Arc::clone(&input),
                    pixels: FrameBuf::new(),
                },
            );
            input
        })
        .collect();
    {
        let mut f = s.focus.lock().unwrap();
        assert!(f.sidecar_50 && f.focused_50, "both restored");
        assert!(f.only_render_selected, "starts on at every launch");
        f.focused = Some("a".into());
        f.wall_open = true;
        f.wall = vec!["b".into(), "c".into()];
        f.renderer_by = std::collections::HashMap::from([("b".into(), true), ("c".into(), true)]);
    }
    s.sync_sidecar_cadence();
    assert!(
        inputs[0].full_rate(),
        "the focused, drawing bot uses its setting"
    );
    assert!(
        !inputs[1].full_rate() && !inputs[2].full_rate(),
        "hidden by only-render-selected: no 50 Hz work"
    );
    s.focus.lock().unwrap().renderer = false;
    s.sync_sidecar_cadence();
    assert!(
        inputs.iter().all(|input| !input.full_rate()),
        "renderer off: nothing paces at 50 Hz"
    );
}

#[test]
fn picker_teleport_consumes_the_selection_and_checks_the_bound_target() {
    let world = open_world(3, 3);
    let fixture = MapFixture::new(&world, "public-289");
    let play = fixture.play(Tile {
        x: 0,
        z: 1,
        level: 0,
    });
    let mut s = Session::new();
    s.launcher
        .install_template(ProfileTemplate::from_host(Arc::clone(&fixture.template)));
    s.set_focus_for_test("alice");
    s.core.set_play(Some(play));
    s.statuses.refresh_from_play(&s.core);
    s.select_picker_tile(
        &world,
        Tile {
            x: 2,
            z: 2,
            level: 0,
        },
    );
    assert!(!s.confirm_picker_teleport(&world));
    assert_eq!(
        s.error.as_deref(),
        Some("Debug Teleport requires a local loopback engine target")
    );
    assert!(s.map_model.pending().is_none());
    assert!(!s.confirm_picker_teleport(&world));
    assert_eq!(s.error.as_deref(), Some("Select a map destination first"));
}

#[test]
fn picker_teleport_without_spawned_queue_is_no_focus() {
    let mut s = Session::new();
    let world = open_world(3, 3);
    let origin = Tile {
        x: 0,
        z: 1,
        level: 0,
    };
    let dest = Tile {
        x: 2,
        z: 2,
        level: 0,
    };
    let fixture = MapFixture::new(&world, "local-289");
    let play = fixture.play(origin);
    s.launcher
        .install_template(ProfileTemplate::from_host(Arc::clone(&fixture.template)));
    s.set_focus_for_test("alice");
    s.core.set_play(Some(play));
    s.statuses.refresh_from_play(&s.core);
    assert_eq!(s.select_picker_tile(&world, dest), Some(dest));
    assert!(!s.confirm_picker_teleport(&world));
    assert_eq!(
        s.error.as_deref(),
        Some("Walk/Teleport unavailable: no focused running slot")
    );
    assert!(s.map_model.pending().is_none());
}

fn walkto_panel_state(session: &Session) -> (&'static [&'static str], (bool, bool), String) {
    let teleport = session.map_teleport_authorized();
    (
        walkto_footer_labels(teleport),
        walkto_actions_enabled(session.map_model.pending(), teleport),
        format_walkto_status(
            walkto_selection_caption(session.map_model.pending(), teleport),
            "ok",
        ),
    )
}

#[test]
fn picker_teleport_follows_profile_class() {
    let world = open_world(3, 3);
    let origin = Tile {
        x: 0,
        z: 1,
        level: 0,
    };
    let blocked = Tile {
        x: 1000,
        z: 1001,
        level: 1,
    };

    let mut local = Session::new();
    let local_fixture = MapFixture::new(&world, "local-289");
    let play = local_fixture.play(origin);
    local
        .launcher
        .install_template(ProfileTemplate::from_host(Arc::clone(
            &local_fixture.template,
        )));
    local.set_focus_for_test("alice");
    local.core.set_play(Some(play));
    local.statuses.refresh_from_play(&local.core);
    local.select_picker_tile(&world, blocked);
    assert_eq!(local.profile_class(), ProfileClass::Local);
    assert!(local.map_teleport_authorized());
    let (labels, actions, status) = walkto_panel_state(&local);
    assert_eq!(labels, &["recentre", "Walk", "Send", "Teleport"][..]);
    assert_eq!(actions, (false, true));
    assert_eq!(status, "blocked 1000 1001 1 (teleport only) · ok");

    let mut prod = Session::new();
    let prod_fixture = MapFixture::new(&world, "public-289");
    prod.launcher
        .install_template(ProfileTemplate::from_host(Arc::clone(
            &prod_fixture.template,
        )));
    prod.set_map_host("127.0.0.1");
    prod.select_picker_tile(&world, blocked);
    assert_eq!(prod.profile_class(), ProfileClass::Remote);
    assert!(!prod.debug_ui());
    assert!(!prod.map_teleport_authorized());
    let (labels, actions, status) = walkto_panel_state(&prod);
    assert_eq!(labels, &["recentre", "Walk", "Send"][..]);
    assert_eq!(actions, (false, false));
    assert_eq!(status, "blocked 1000 1001 1 · ok");

    let mut remote = Session::new();
    let remote_fixture = MapFixture::new(&world, "local-289");
    remote
        .launcher
        .install_template(ProfileTemplate::from_host(Arc::clone(
            &remote_fixture.template,
        )));
    remote.set_map_host("192.168.1.2");
    remote.select_picker_tile(&world, blocked);
    assert_eq!(remote.profile_class(), ProfileClass::Local);
    assert!(!remote.map_teleport_authorized());
    let (labels, actions, status) = walkto_panel_state(&remote);
    assert_eq!(labels, &["recentre", "Walk", "Send"][..]);
    assert_eq!(actions, (false, false));
    assert_eq!(status, "blocked 1000 1001 1 · ok");
}

#[test]
fn picker_blocked_selection_refuses_walk_and_teleports_requested() {
    use frontend_core::walk_map::{WalkActionErrorKind, WalkActionKind};
    let mut s = Session::new();
    let world = open_world(3, 3);
    let origin = Tile {
        x: 0,
        z: 1,
        level: 0,
    };
    let blocked = Tile {
        x: 1000,
        z: 1001,
        level: 1,
    };
    let _fixture = bind_picker_session(&mut s, &world, origin);
    assert_eq!(s.select_picker_tile(&world, blocked), None);
    let pending = s.map_model.pending().expect("miss still selects requested");
    assert_eq!(pending.requested(), blocked);
    assert_eq!(pending.walk_target(), None);
    assert_eq!(
        s.picker_action_available(&world, WalkActionKind::Walk)
            .unwrap_err()
            .kind(),
        WalkActionErrorKind::Blocked
    );
    assert_eq!(
        s.picker_action_available(&world, WalkActionKind::Teleport)
            .expect("blocked tile stays teleportable"),
        blocked
    );
    assert!(
        s.map_model.pending().is_some(),
        "availability must not consume"
    );
    assert!(!s.confirm_picker_teleport(&world));
    assert_eq!(
        s.error.as_deref(),
        Some("Walk/Teleport unavailable: no focused running slot"),
        "blocked Teleport must not fail as Blocked"
    );
    assert!(s.map_model.pending().is_none());
}

/// Both endpoints are real standable map cells; the mine remains an island.
fn picker_mine_world() -> NavWorld {
    let (width, height) = (384, 1472);
    let mut flags = vec![CollisionFlag::SQ_BLOCKED as u32; 4 * width * height];
    for (west, south) in [(2880, 4800), (3200, 3392)] {
        for z in south..south + 64 {
            for x in west..west + 64 {
                flags[(z - 3392) * width + x - 2880] = 0;
            }
        }
    }
    let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
    NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 2880,
                z: 3392,
                level: 0,
            },
            width,
            height,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        vec![],
    )
}

#[test]
fn picker_confirm_feeds_the_focused_slots_latched_essence_session() {
    let mut s = Session::new();
    s.set_focus_for_test("alice");
    // Alice's walker already latched the mine session (entered via
    // Aubury): a WalkTo out of the mine must route through the exit
    // portal's return hop — the arm feeds the traveller's latch.
    let arms = host_play::WalkArms::default();
    arms.lock().unwrap().insert(
        "alice".into(),
        Arc::new(Mutex::new(host_play::WalkArm::with_latched_essence(
            nav_planner::essence::essence_session_for_wizard(553),
        ))),
    );
    s.core.set_walk_arms(arms);
    let world = picker_mine_world();
    let anchor = Tile {
        x: 3253,
        z: 3401,
        level: 0,
    };
    assert!(confirm_map_walk(
        &mut s,
        &world,
        Tile {
            x: 2912,
            z: 4833,
            level: 0, // the mine pad
        },
        anchor
    ));
    let route = s
        .core
        .manual_walk_route("alice")
        .expect("focused walk arm exists");
    assert_eq!(
        route.dest,
        api::snapshot::WorldTile {
            x: anchor.x,
            z: anchor.z,
            level: anchor.level,
        },
        "the exit route reaches Aubury's anchor"
    );
}

#[test]
fn picker_confirm_without_a_latch_keeps_the_mine_sealed() {
    let mut s = Session::new();
    s.set_focus_for_test("alice");
    // No latch: the session return hop is never relaxed — the sealed
    // mine stays NoPath (fail-closed without a session is correct).
    let world = picker_mine_world();
    assert!(!confirm_map_walk(
        &mut s,
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
        }
    ));
    assert!(!s.core.walk_route_armed("alice"));
}

#[test]
fn picker_confirm_no_path_keeps_destination_without_arming() {
    let mut s = Session::new();
    s.set_focus_for_test("alice");
    // Block the middle column: (1,0), (1,1), (1,2) on the 3x3 world.
    let mut flags = vec![0u32; 9];
    for z in 0..3 {
        flags[z * 3 + 1] = CollisionFlag::WALK_BLOCK_FLAGS as u32;
    }
    let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
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
    assert!(!confirm_map_walk(
        &mut s,
        &world,
        Tile {
            x: 0,
            z: 1,
            level: 0,
        },
        dest
    ));
    assert_eq!(s.map_desk.walk_dest(), Some(dest));
    assert!(
        !s.core.walk_route_armed("alice"),
        "no route must be armed when find fails"
    );
}

/// A 5×5 world walled between x=1 and x=2, crossed only by a 10-coin
/// toll door (the `toll_edges` shape: loc 2882, `consumed_req` coins 10).
fn toll_world() -> NavWorld {
    let mut flags = vec![0u32; 25];
    for z in 0..5 {
        flags[z * 5 + 1] |= CollisionFlag::W_E as u32;
        flags[z * 5 + 2] |= CollisionFlag::W_W as u32;
    }
    let edge = TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        kind: TransportKind::Door,
        player_delta: None,
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
        item_req: vec![],
        consumed_req: vec![(995, 10)],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    };
    let mut graph = TransportGraph::default();
    graph.at.entry(edge.at).or_default().push(0);
    graph.edges.push(edge);
    let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
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

/// A published snapshot and its `WorldState` with 10 coins, derived through
/// the slot-thread publish path: a client with a TYPE_INV iface, rebuilt into a
/// snapshot, mapped via [`WorldState::from_snapshot`]. The cache dir
/// is a unique scratch dir so a stray real cache (e.g. jags under
/// `/tmp`) cannot seed other ifaces.
fn coins_snapshot() -> (GameSnapshot, WorldState) {
    let cache_dir = TestDir::new("toll-cache");
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
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(&c);
    let state = WorldState::from_snapshot(&snapshot);
    (snapshot, state)
}

/// The focused slot's published snapshot facts gate the WalkTo route:
/// 10 coins on the player let a confirmed walk cross a toll that
/// an empty state refuses.
#[test]
fn picker_confirm_uses_focused_slot_state_across_a_toll() {
    let world = toll_world();
    let mut s = Session::new();
    s.set_focus_for_test("alice");
    // The slot thread published 10 coins (derived from a live
    // snapshot); the picker routes with those facts.
    let (snapshot, world_state) = coins_snapshot();
    s.core.publish_test_snapshot("alice", snapshot, world_state);
    assert!(confirm_map_walk(
        &mut s,
        &world,
        Tile {
            x: 0,
            z: 1,
            level: 0,
        },
        Tile {
            x: 4,
            z: 4,
            level: 0,
        }
    ));
    let route = s
        .core
        .manual_walk_route("alice")
        .expect("focused walk arm exists");
    assert!(
        route.legs.iter().any(|l| matches!(
            l,
            Leg::Transport { edge } if edge.consumed_req == vec![(995, 10)]
        )),
        "the route must cross the toll"
    );
}

/// A published slot snapshot is visible through the borrowed view used by
/// WalkTo follow.
#[test]
fn walk_follow_reads_the_published_slot_snapshot() {
    let mut s = Session::new();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_tick(7);
    s.core
        .publish_test_snapshot("alice", snapshot, WorldState::empty());
    let tick = s
        .core
        .with_slot_world_state("alice", |snapshot, _| snapshot.tick())
        .expect("published snapshot and world state");
    assert_eq!(tick, 7);
    assert!(s.core.with_slot_snapshot("bob", |_| ()).is_none());
}

/// Missing inventory facts leave the toll unusable even with an observed origin.
#[test]
fn picker_confirm_falls_back_to_empty_when_slot_has_no_state() {
    let world = toll_world();
    let mut s = Session::new();
    s.set_focus_for_test("alice");
    assert!(!confirm_map_walk(
        &mut s,
        &world,
        Tile {
            x: 0,
            z: 1,
            level: 0,
        },
        Tile {
            x: 4,
            z: 4,
            level: 0,
        }
    ));
    assert!(
        !s.core.walk_route_armed("alice"),
        "no route armed when the toll is unpayable"
    );
}

#[test]
fn picker_confirm_ignores_teles_until_allow_teleports() {
    let _isolated = IsolatedStores::enter("picker-teleport-permission");
    // world: origin cannot walk to dest; a teleport edge can.
    let mut session = Session::new();
    session.set_focus_for_test("alice");
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
        takeoff: None,
        worn_all_req: Vec::new(),
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        kind: TransportKind::Teleport,
        player_delta: None,
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
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    });
    let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
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
        z: 1,
        level: 0,
    };
    session.ui.nav.allow_teleports = false;
    frontend_core::nav_prefs::write_nav_preference_for_test(
        &session.preferences,
        frontend_core::NavPreference::AllowTeleports,
        false,
    )
    .map(|()| false)
    .unwrap();
    assert!(!confirm_map_walk(&mut session, &world, origin, dest_tile));
    session.ui.nav.allow_teleports = true;
    frontend_core::nav_prefs::write_nav_preference_for_test(
        &session.preferences,
        frontend_core::NavPreference::AllowTeleports,
        true,
    )
    .map(|()| true)
    .unwrap();
    assert!(confirm_map_walk(&mut session, &world, origin, dest_tile));
    let route = session.core.manual_walk_route("alice").expect("walk route");
    assert!(route.legs.iter().any(|l| matches!(
        l,
        Leg::Transport { edge } if edge.kind == TransportKind::Teleport
    )));
}

#[test]
fn picker_confirm_uses_find_with_options() {
    let _isolated = IsolatedStores::enter("picker-tele-wilderness-permissions");
    // The tele fixture moved to wildy-north coords, with the teleport
    // landing on a wilderness tile: the teleport edge is the only way
    // across the wall, and its landing is inside the zone. Neither
    // flag alone may route — confirmation must pass both durable
    // allow_teleports and allow_wilderness grants through to find_with.
    let mut session = Session::new();
    session.set_focus_for_test("alice");
    session.ui.nav.allow_teleports = false;
    session.ui.nav.allow_wilderness = false;
    frontend_core::nav_prefs::write_nav_preference_for_test(
        &session.preferences,
        frontend_core::NavPreference::AllowTeleports,
        false,
    )
    .map(|()| false)
    .unwrap();
    frontend_core::nav_prefs::write_nav_preference_for_test(
        &session.preferences,
        frontend_core::NavPreference::AllowWilderness,
        false,
    )
    .map(|()| false)
    .unwrap();
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
    // Routing gates wilderness entry on the packed `graph.wilderness`
    // rules; pack the surface zone so the fixture has a wilderness.
    let mut graph = TransportGraph {
        wilderness: nav_planner::transport::WildernessRules {
            zones: vec![nav_planner::transport::WildernessZone {
                x1: 2944,
                z1: 3520,
                x2: 3391,
                z2: 6399,
                level1: 0,
                level2: 3,
                origin_z: 3520,
            }],
            divisor: 8,
            offset: 1,
        },
        ..TransportGraph::default()
    };
    graph.teleports.push(TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        kind: TransportKind::Teleport,
        player_delta: None,
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
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    });
    let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
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
    assert!(!confirm_map_walk(&mut session, &world, origin, dest_tile));
    session.ui.nav.allow_teleports = true;
    frontend_core::nav_prefs::write_nav_preference_for_test(
        &session.preferences,
        frontend_core::NavPreference::AllowTeleports,
        true,
    )
    .map(|()| true)
    .unwrap();
    assert!(!confirm_map_walk(&mut session, &world, origin, dest_tile));
    session.ui.nav.allow_wilderness = true;
    frontend_core::nav_prefs::write_nav_preference_for_test(
        &session.preferences,
        frontend_core::NavPreference::AllowWilderness,
        true,
    )
    .map(|()| true)
    .unwrap();
    assert!(confirm_map_walk(&mut session, &world, origin, dest_tile));
    let route = session.core.manual_walk_route("alice").expect("walk route");
    assert!(route.legs.iter().any(|l| matches!(
        l,
        Leg::Transport { edge } if edge.kind == TransportKind::Teleport
    )));
}

/// A WalkTo walk the panel arms shows on the member's fleet row (what the
/// rail and grid caps draw) from the pump that follows it until the route
/// ends; then the picked dest clears. An arm its slot thread holds is not
/// waited on.
#[test]
fn a_picker_walk_shows_the_member_running_until_it_ends() {
    let mut s = Session::new();
    let arms = host_play::WalkArms::default();
    let arm = Arc::new(Mutex::new(host_play::WalkArm::default()));
    arms.lock()
        .unwrap()
        .insert("alice".into(), Arc::clone(&arm));
    s.core.set_walk_arms(arms);
    let world = open_world(3, 3);
    let origin = Tile {
        x: 0,
        z: 1,
        level: 0,
    };
    let _fixture = bind_picker_session(&mut s, &world, origin);
    s.core.fleet_mut().add("alice");
    s.pump_status();
    let before = s.core.fleet_view().rows_generation();
    assert!(!s.core.fleet_view().row("alice").unwrap().manual_walk());

    let dest = Tile {
        x: 2,
        z: 2,
        level: 0,
    };
    s.select_picker_tile(&world, dest);
    assert!(s.confirm_picker_walk(&world));
    s.pump_status();
    let walking = s.core.fleet_view().row("alice").unwrap();
    assert!(walking.manual_walk());
    assert_eq!(walking.brief(), "running");
    assert!(
        s.core.fleet_view().rows_generation() > before,
        "the rail's cached rows are refreshed"
    );

    // The slot hook ends the route while it holds the arm.
    {
        let mut held = arm.lock().unwrap();
        held.route = None;
        s.pump_status();
        assert!(
            s.core.fleet_view().row("alice").unwrap().manual_walk(),
            "an arm in use keeps its last answer; the pump does not wait"
        );
    }
    s.core.finish_walk_for_test("alice");
    s.pump_status();
    let arrived = s.core.fleet_view().row("alice").unwrap();
    assert!(!arrived.manual_walk());
    assert_eq!(arrived.brief(), "idle");
    assert_eq!(s.walk_status_text(), "—");
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
    let mut s = Session::new();
    s.set_focus_for_test("alice");
    assert_eq!(s.focused_tile(), None, "no status rows yet");
}

#[test]
fn disconnected_focused_slot_has_no_tile() {
    let mut session = Session::new();
    session.set_focus_for_test("alice");
    let play = empty_play();
    play.statuses.lock().unwrap().push(SlotStatus {
        username: "alice".into(),
        connected: false,
        ingame: false,
        tile_x: -1,
        tile_z: -1,
        tile_level: -1,
        ..SlotStatus::default()
    });
    session.core.set_play(Some(play));
    session.statuses.refresh_from_play(&session.core);
    assert_eq!(session.focused_tile(), None);
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
    seed_ui_state(&crate::ui_state::PanelUiState::default());
    let path = tmp_vault("focus-first.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    vault_upsert(&mut s.core, profile("bob", "pw", 43));
    s.focus_first_profile();
    assert_eq!(s.focused_name().as_deref(), Some("alice"));
    assert!(s.core.slots().contains_key("alice"));
    assert!(
        !s.core.slots().contains_key("bob"),
        "parked vault rows must not start a Client"
    );
}

#[test]
fn focus_first_prefers_last_focus() {
    let path = tmp_vault("focus-last.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    vault_upsert(&mut s.core, profile("bob", "pw", 43));
    crate::ui_state::save(
        &s.preferences,
        &crate::ui_state::PanelUiState {
            last_focus: Some("bob".into()),
            ..Default::default()
        },
    );
    s.ui.last_focus = Some("bob".into());
    s.focus_first_profile();
    assert_eq!(s.focused_name().as_deref(), Some("bob"));
    assert_eq!(
        crate::ui_state::load(&s.preferences).last_focus.as_deref(),
        Some("bob")
    );
}

#[test]
fn select_saves_last_focus() {
    // Model initial preference hydration completing after this focus write.
    // The test-only preference mirror must retain the committed key.
    seed_ui_state(&crate::ui_state::PanelUiState::default());
    let path = tmp_vault("select-last-focus.vault");
    let mut s = Session::new();
    s.preferences.reload().unwrap();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    s.select("alice");
    await_preference(&mut s, "last_focus", serde_json::json!("alice"));
    let stored = s.preferences.reload().unwrap();
    assert_eq!(stored.get("last_focus"), Some(&serde_json::json!("alice")));
    s.preferences_ready = false;
    s.poll_panel_preferences();
    assert_eq!(s.ui.last_focus.as_deref(), Some("alice"));
}

#[test]
fn set_multibox_restores_last_focus_when_focus_not_on_wall() {
    let path = tmp_vault("multibox-last-focus.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    vault_upsert(&mut s.core, profile("bob", "pw", 43));
    // Focus alice without opening her slot: a focused profile that is not
    // a fleet member (a Select that opens a slot loads the bot).
    s.set_focus_for_test("alice");
    assert!(!s.core.members().iter().any(|m| m == "alice"));
    s.core.fleet_mut().add("bob");
    crate::ui_state::save(
        &s.preferences,
        &crate::ui_state::PanelUiState {
            last_focus: Some("bob".into()),
            ..Default::default()
        },
    );
    // Focused alice is not a wall member; MultiBox-on should pick bob.
    s.set_multibox(true);
    assert_eq!(s.focused_name().as_deref(), Some("bob"));
    assert!(s.core.members().iter().any(|m| m == "bob"));
}

#[test]
fn select_spawns_parked_profile_once() {
    let path = tmp_vault("select-spawn.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    vault_upsert(&mut s.core, profile("bob", "pw", 43));
    s.select("alice");
    assert_eq!(s.core.slots().len(), 1);
    s.select("bob");
    assert_eq!(s.core.slots().len(), 2);
    s.select("alice");
    assert_eq!(s.core.slots().len(), 2);
}

#[test]
fn flat_model_spawns_every_member_as_a_client() {
    let path = tmp_vault("flat-spawn.vault");
    let mut s = Session::new();
    assert!(s.unlock_at(&path, "test-passphrase-01"));
    for (n, uid) in [("alice", 1), ("bob", 2), ("carol", 3)] {
        vault_upsert(&mut s.core, profile(n, "pw", uid));
    }
    s.select("alice");
    s.load("bob");
    s.load("carol");
    // ensure_slot registers the IO map synchronously; wait only for the
    // slot threads to publish their status rows.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        if s.core.play_for_test().unwrap().statuses().len() == 3 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        s.core.slots().len(),
        3,
        "every wall member owns a FrameBuf slot"
    );
    assert_eq!(
        s.core.play_for_test().unwrap().statuses().len(),
        3,
        "one full Client slot per profile — no lean channels"
    );
    assert!(
        s.core.play_for_test().unwrap().arm("carol").is_some(),
        "every member has a control arm"
    );
    // Focus is pure bookkeeping: selecting bob redirects the sampled
    // slot without touching a socket.
    s.select("bob");
    assert_eq!(s.focused_name().as_deref(), Some("bob"));
    assert_eq!(
        s.core.play_for_test().unwrap().focused().as_deref(),
        Some("bob")
    );
    assert_eq!(
        s.core.play_for_test().unwrap().statuses().len(),
        3,
        "focus does not swap sockets; every slot stays up"
    );
    // Session teardown stops every arm and joins each slot. The startup
    // progress callback transfers that stop into the client's shell, so an
    // unreachable update server cannot hold the join through its retry schedule.
}

#[test]
fn sidecar_select_does_not_restart_when_game_is_highmem() {
    let path = tmp_vault("select-no-restart.vault");
    let mut s = Session::new();
    s.persist_ui = false;
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    vault_upsert(&mut s.core, profile("bob", "pw", 43));
    s.ui.lowmem = false;
    s.select("alice");
    s.load("bob");
    let alice_px = std::sync::Arc::as_ptr(&s.core.slot_io("alice").unwrap().pixels);
    let bob_px = std::sync::Arc::as_ptr(&s.core.slot_io("bob").unwrap().pixels);
    s.select("bob");
    assert_eq!(s.focused_name().as_deref(), Some("bob"));
    assert_eq!(
        std::sync::Arc::as_ptr(&s.core.slot_io("alice").unwrap().pixels),
        alice_px
    );
    assert_eq!(
        std::sync::Arc::as_ptr(&s.core.slot_io("bob").unwrap().pixels),
        bob_px
    );
}

#[test]
fn sidecar_select_does_not_restart_when_game_is_cpu() {
    let path = tmp_vault("select-no-restart-cpu.vault");
    let mut s = Session::new();
    s.persist_ui = false;
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    vault_upsert(&mut s.core, profile("bob", "pw", 43));
    s.ui.raster = frontend_core::command::RasterMode::Cpu;
    s.select("alice");
    s.load("bob");
    let alice_px = std::sync::Arc::as_ptr(&s.core.slot_io("alice").unwrap().pixels);
    s.select("bob");
    assert_eq!(
        std::sync::Arc::as_ptr(&s.core.slot_io("alice").unwrap().pixels),
        alice_px
    );
}

#[test]
fn logout_all_arms_every_wall_member() {
    let path = tmp_vault("logout-all-flat.vault");
    let mut s = Session::new();
    assert!(s.unlock_at(&path, "test-passphrase-01"));
    for (n, uid) in [("alice", 1), ("bob", 2)] {
        vault_upsert(&mut s.core, profile(n, "pw", uid));
    }
    s.select("alice");
    s.load("bob");
    s.core.fleet_mut().add("alice");
    s.core.fleet_mut().add("bob");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        if s.core.play_for_test().unwrap().arm("bob").is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    s.logout_all();
    assert!(
        s.core
            .play_for_test()
            .unwrap()
            .arm("alice")
            .unwrap()
            .wants_logout(),
        "the focused member must logout"
    );
    assert!(
        s.core
            .play_for_test()
            .unwrap()
            .arm("bob")
            .unwrap()
            .wants_logout(),
        "every wall member must logout"
    );
}

#[test]
fn headed_stress_spawns_every_member_prefers_and_arms_s00() {
    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: Some("s02".into()),
        ..Default::default()
    });
    let mut s = Session::new();
    s.live_prepare_stress(3, false).expect("prepare");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        if s.core.play_for_test().unwrap().statuses().len() == 3 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        s.core.slots().len(),
        3,
        "every member owns its own Client slot"
    );
    assert_eq!(
        s.focused_name().as_deref(),
        Some("s00"),
        "focused slot must be s00, not last_focus s02"
    );
    assert_eq!(s.tv_name().as_deref(), Some("s00"));
    assert!(
        s.core
            .play_for_test()
            .unwrap()
            .arm("s00")
            .unwrap()
            .wants_login(),
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
}

#[test]
fn login_all_arms_every_wall_member() {
    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = Session::new();
    s.live_prepare_stress(2, false).expect("prepare");
    assert!(
        s.core
            .play_for_test()
            .unwrap()
            .arm("s01")
            .unwrap()
            .wants_login(),
        "login all arms every member immediately (the FIFO serializes)"
    );
}

#[test]
fn headed_stress_full_paints_every_member_at_50fps() {
    seed_ui_state(&crate::ui_state::PanelUiState {
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
        let slot = s.core.slot_io(name).expect("slot");
        assert!(slot.input.full_rate(), "{name} must run the 50 fps cadence");
    }
}

/// RAM watch members must be raster Off so a flipped only-render-
/// selected cannot attach 49 GPU heads. The Game pane's one seat
/// follows focus (click a working member after s00's −1).
#[test]
fn stress50_rail_members_are_raster_off() {
    seed_ui_state(&crate::ui_state::PanelUiState {
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
    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = preparation_only_session();
    let scenario = frontend_core::live::Scenario::get("walk").expect("walk scenario in registry");
    s.live_prepare_script(scenario).expect("prepare");
    // Live boots a minted per-run account — never the registry's
    // `test` (engine auto-registers unknown names).
    let name = s
        .core
        .profile_names()
        .into_iter()
        .next()
        .expect("minted profile");
    assert_ne!(name, "test", "live must not log in `test`");
    let play = s.core.play_for_test().expect("play started");
    assert!(
        play.arm(&name).unwrap().wants_login(),
        "login all arms the minted profile's handshake"
    );
    let runner = s.core.live_runner();
    assert_eq!(runner.profile_name().as_deref(), Some(name.as_str()));
    assert!(
        matches!(
            runner.status(),
            Some(frontend_core::live::RunnerStatus::Seeding)
        ),
        "a fresh runner holds in seeding until ingame scene 2"
    );
    assert!(
        runner.drives(&name) && !runner.drives("test"),
        "the runner ticks only its minted profile's slot"
    );
}

/// A scenario that names a script card (`start_script`) fills the
/// catalog and selects it on the driven slot — Start waits for the
/// `StartScript` step after seed. `$RS2B0T` is read here, and the
/// card's source is what the isolate will spawn on that step.
#[cfg(feature = "js")]
#[test]
fn live_prepare_bone_burier_starts_the_rs2b0t_card_on_the_driven_slot() {
    let iso = IsolatedStores::enter("bone-live");
    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let root = write_looping_catalog(iso.dir(), &[("BoneBurier", "BoneBurier")]);
    iso.set_catalog_root(&root);
    let mut s = preparation_only_session();
    let result = s.live_prepare_script(
        frontend_core::live::Scenario::get("bone_burier").expect("registered"),
    );
    let name = s
        .core
        .profile_names()
        .into_iter()
        .next()
        .expect("minted profile");
    assert_ne!(name, "test", "live must not log in `test`");
    match result {
        Ok(()) => {}
        Err(e) => panic!("live_prepare must select the card: {e}"),
    }
    assert_eq!(
        s.script_sel,
        Some(ScriptChoice::loaded(CardSource::Catalog, "BoneBurier")),
        "prepare sets script_sel to the catalog card"
    );
    assert_ne!(
        s.core.script_state(&name),
        ScriptRunState::Running,
        "isolate is not Running yet — Start waits for the StartScript step"
    );
}

#[cfg(feature = "js")]
#[test]
fn live_prepare_bone_burier_v2_selects_each_example_by_identity() {
    use frontend_core::test_support::scripts::{file_identity, live_example_path};
    let ts = live_example_path("bone_burier_v2.ts").expect("ts example");
    let js = live_example_path("bone_burier_v2.js").expect("js example");
    for (name, file_name, path) in [
        ("bone_burier_v2_ts", "bone_burier_v2.ts", ts.as_path()),
        ("bone_burier_v2_js", "bone_burier_v2.js", js.as_path()),
    ] {
        let iso = IsolatedStores::enter(&format!("bone-v2-{name}"));
        let mut s = Session::new();
        s.scripts.use_library_at(
            iso.dir().join("js-scripts.json"),
            iso.dir().join("js-cache"),
        );
        s.scripts.load_card(&ts).expect("preload ts");
        s.scripts.load_card(&js).expect("preload js");
        s.scripts
            .set_legacy_str(CardSource::File, "bone_burier_v2", "boneName", "stem");
        let identity = file_identity(path);
        s.scripts
            .set_legacy_str(CardSource::File, &identity, "boneName", "identity");
        let scenario = frontend_core::live::Scenario::get(name).expect("registered");
        assert_eq!(scenario.view().start_file(), Some(file_name));
        assert_eq!(scenario.view().start_script(), None);
        let card = s.scripts.load_card(path).expect("identity load");
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
            .pending_settings_bag(CardSource::File, &identity, card.schema_ref())
            .expect("settings bag");
        assert_eq!(
            bag.get("boneName"),
            Some(&serde_json::json!("identity")),
            "{name} settings attach to the selected File identity"
        );
        let stem_bag =
            s.pending_settings_bag(CardSource::File, "bone_burier_v2", card.schema_ref());
        assert_eq!(
            stem_bag.as_ref().and_then(|bag| bag.get("boneName")),
            Some(&serde_json::json!("stem")),
            "{name} stem bag stays isolated from the File identity"
        );
    }
}

#[cfg(feature = "js")]
#[test]
fn live_prepare_keeps_v1_catalog_and_tradebot_stem() {
    let iso = IsolatedStores::enter("bone-v1-trade");
    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let root = write_looping_catalog(iso.dir(), &[("BoneBurier", "BoneBurier")]);
    iso.set_catalog_root(&root);
    let mut s = preparation_only_session();
    s.live_prepare_script(frontend_core::live::Scenario::get("bone_burier").expect("registered"))
        .expect("v1");
    assert_eq!(
        s.script_sel,
        Some(ScriptChoice::loaded(CardSource::Catalog, "BoneBurier"))
    );
    let mut trade = preparation_only_session();
    trade.scripts.use_library_at(
        iso.dir().join("trade-js.json"),
        iso.dir().join("trade-cache"),
    );
    trade
        .live_prepare_script(
            frontend_core::live::Scenario::get("script_trade").expect("registered"),
        )
        .expect("trade");
    assert_eq!(
        trade.script_sel,
        Some(ScriptChoice::loaded(CardSource::File, "trade_bot"))
    );
}

#[cfg(feature = "js")]
#[test]
fn live_prepare_script_trade_loads_the_file_fixture_and_injects_partner() {
    let iso = IsolatedStores::enter("trade-live");
    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = preparation_only_session();
    s.scripts.use_library_at(
        iso.dir().join("js-scripts.json"),
        iso.dir().join("js-cache"),
    );
    s.live_prepare_script(frontend_core::live::Scenario::get("script_trade").expect("registered"))
        .expect("prepare");
    assert_eq!(
        s.script_sel,
        Some(ScriptChoice::loaded(CardSource::File, "trade_bot")),
        "script_trade loads the in-tree TradeBot fixture as File"
    );
    let names = s.core.profile_names();
    assert_eq!(names.len(), 2, "script_trade is a two-profile fleet");
    let pending = s.core.live_runner().pending_starts_for_test();
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

#[cfg(feature = "js")]
#[test]
fn live_prepare_jive_kq_stashes_four_starts_with_the_same_roster() {
    let iso = IsolatedStores::enter("jive-kq-fleet");
    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let root = write_looping_catalog(iso.dir(), &[("JiveKQ", "JiveKQ")]);
    iso.set_catalog_root(&root);
    let mut session = preparation_only_session();
    session
        .live_prepare_script(
            frontend_core::live::Scenario::get("jive_kq_four").expect("registered"),
        )
        .expect("prepare four-player JiveKQ");

    let names = session.core.profile_names();
    assert_eq!(names.len(), 4, "JiveKQ mints four fleet profiles");
    let roster = names
        .iter()
        .map(|name| client::util::JString::to_screen_name(name))
        .collect::<Vec<_>>()
        .join(",");
    let starts = session.core.live_runner().pending_starts_for_test();
    assert_eq!(starts.len(), 4, "StartScript prepares all four isolates");
    let mut prepared_slots = starts
        .iter()
        .map(|start| start.slot.clone())
        .collect::<Vec<_>>();
    prepared_slots.sort();
    let mut fleet_slots = names.clone();
    fleet_slots.sort();
    assert_eq!(
        prepared_slots, fleet_slots,
        "each fleet profile has exactly one prepared start"
    );
    for start in starts.iter() {
        assert_eq!(
            start.bag.as_ref().and_then(|settings| settings.get("team")),
            Some(&serde_json::json!(roster)),
            "{} receives the complete shared roster",
            start.slot
        );
    }
}

/// Thiever SETTINGS often fail to parse (`loadout: LOADOUT_SETTING`
/// stops the object walk), so the card schema is empty. The live
/// inject still has to post `target: Guard` or `str('target', 'Man')`
/// camps Men.
#[cfg(feature = "js")]
#[test]
fn live_prepare_thiever_posts_guard_target_when_schema_empty() {
    let iso = IsolatedStores::enter("thiever-bag");
    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let root = write_looping_catalog(iso.dir(), &[("Thiever", "ThievingBot")]);
    iso.set_catalog_root(&root);
    let mut s = preparation_only_session();
    s.live_prepare_script(frontend_core::live::Scenario::get("thiever").expect("registered"))
        .expect("prepare");
    let pending = s.core.live_runner().pending_starts_for_test();
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
        vec![api::loadout::Loadout::new("Memory food").with_carry("Lobster", 1)],
        "harness Start must stash the fixture-owned loadout, not operator loadouts.json"
    );
}

#[cfg(feature = "js")]
#[test]
fn live_prepare_script_sets_script_sel_for_catalog_start_script() {
    let iso = IsolatedStores::enter("alcher-sel");
    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let root = write_looping_catalog(iso.dir(), &[("Alcher", "Alcher")]);
    iso.set_catalog_root(&root);
    let mut s = preparation_only_session();
    let scenario =
        frontend_core::test_support::live::scenario_with_start_script_for_test("alcher", "Alcher")
            .expect("registered");
    s.live_prepare_script(scenario).expect("prepare");
    assert_eq!(
        s.script_sel,
        Some(ScriptChoice::loaded(CardSource::Catalog, "Alcher")),
        "live_prepare_script must set script_sel to the catalog card"
    );
}

#[cfg(feature = "js")]
#[test]
fn live_prepare_sherlock_starts_the_compiled_card_without_catalog() {
    let iso = IsolatedStores::enter("sherlock-live");
    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = preparation_only_session();
    s.live_prepare_script(frontend_core::live::Scenario::get("sherlock_talk").expect("registered"))
        .expect("prepare compiled Sherlock without $RS2B0T");
    assert_eq!(
        s.script_sel,
        Some(ScriptChoice::compiled(
            CompiledCardId::lookup("Sherlock").unwrap()
        )),
        "live_prepare must select the compiled registry card"
    );
    let pending = s.core.live_runner().pending_starts_for_test();
    let card = pending.first().expect("compiled start stashed");
    assert_eq!(
        frontend_core::test_support::live::pending_compiled_id(card).map(|id| id.as_str()),
        Some("Sherlock")
    );
    assert!(
        card.js.is_empty(),
        "compiled Start must not stash catalog JS"
    );
    let _ = iso;
}

#[test]
fn live_prepare_script_enables_multibox_for_a_fleet_only() {
    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = preparation_only_session();
    let fleet = frontend_core::live::Scenario::get("nav_door").expect("nav_door is registered");
    assert!(
        fleet.view().profile_count() > 1,
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
    let vault_names = s.core.profile_names();
    assert_eq!(vault_names.len(), 2, "a fleet mints both slots");
    for name in &vault_names {
        assert_ne!(name, "test", "live must not log in `test`");
        assert!(
            s.core.members().iter().any(|m| m == name),
            "every minted profile is a wall member"
        );
    }
    assert!(!s.wall.chooser_open, "live keeps the chooser closed");

    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = preparation_only_session();
    let solo = frontend_core::live::Scenario::get("walk").expect("walk is registered");
    assert_eq!(solo.view().profile_count(), 1);
    s.live_prepare_script(solo).expect("prepare");
    assert!(!s.multibox, "a solo scenario keeps the single-bot boot");
    assert!(
        !s.focus.lock().unwrap().wall_open,
        "no wall members, no extra rasters"
    );
}

#[test]
fn panel_live_credentials_are_fresh_game_secrets() {
    for entries in [null_raster_live_entries(), stress_live_entries(3)] {
        for (user, pass) in entries {
            assert_ne!(user, pass);
            assert_eq!(pass.len(), 20);
        }
    }
}

#[test]
fn temp_live_vault_mint_does_not_persist_username_as_password() {
    let names = host_play::mint_live_names(2);
    let entries = host_play::mint_live_entries(&names);
    let pass = host_play::live_vault_passphrase();
    let path = temp_live_vault_from(&entries, 274_000_001, &pass, true);
    let vault = VaultFixture::unlock(&path, &pass);
    for username in vault.usernames() {
        let profile = vault.profile(&username).expect("a minted live profile");
        assert_ne!(profile.username(), profile.draft().password_for_test());
    }
}

#[test]
fn live_prepare_script_never_upserts_the_operator_vault() {
    seed_ui_state(&crate::ui_state::PanelUiState::default());
    // The operator vault must stay byte-identical across a live boot:
    // `--live` unlocks only the throwaway temp vault.
    fn stamp(p: &std::path::Path) -> Option<(std::time::SystemTime, u64)> {
        std::fs::metadata(p)
            .ok()
            .map(|m| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len()))
    }
    let op = crate::session::default_vault_path();
    let before = stamp(&op);
    let mut s = preparation_only_session();
    s.live_prepare_script(frontend_core::live::Scenario::get("walk").unwrap())
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
fn live_prepare_script_does_not_write_last_focus() {
    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: Some("alice".into()),
        ..Default::default()
    });
    let mut s = preparation_only_session();
    let fleet = frontend_core::live::Scenario::get("nav_door").expect("nav_door");
    s.live_prepare_script(fleet).expect("prepare");
    assert_eq!(
        crate::ui_state::load(&s.preferences).last_focus.as_deref(),
        Some("alice"),
        "live boot must not clobber the operator last profile"
    );
}

#[test]
fn live_prepare_nav_door_applies_full_rate_and_leaves_sidecar_off() {
    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = preparation_only_session();
    let fleet = frontend_core::live::Scenario::get("nav_door").expect("nav_door");
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
    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = preparation_only_session();
    s.live_prepare_script(frontend_core::live::Scenario::get("nav_full").unwrap())
        .expect("prepare");
    assert_eq!(
        s.core.live_runner().deadline_for_test(),
        Some(Duration::from_secs(360))
    );
    assert_eq!(
        s.core.live_runner().terminal_shot(),
        Some("nav_full terminal")
    );
}

#[test]
fn live_prepare_smoke_runner_already_has_the_300s_deadline() {
    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = preparation_only_session();
    s.live_prepare_script(frontend_core::live::Scenario::get("render_smoke").unwrap())
        .expect("prepare");
    assert_eq!(
        s.core.live_runner().deadline_for_test(),
        Some(Duration::from_secs(300))
    );
}

#[test]
fn requested_nav_paints_survive_scenario_install_without_changing_gameplay_or_prefs() {
    let saved = crate::ui_state::PanelUiState {
        last_focus: None,
        nav: crate::nav_settings::NavSettings {
            pause_script_on_manual_walk_abort: false,
            ..Default::default()
        },
        ..Default::default()
    };
    seed_ui_state(&saved);
    for choice in [None, Some(true), Some(false)] {
        let mut session = preparation_only_session();
        session.set_nav_paints_override(choice);
        let scenario =
            frontend_core::test_support::live::scenario_with_nav_for_test("nav_door", true, true)
                .expect("registered");
        let (baseline, deadline, terminal_shot) = {
            let view = scenario.view();
            (
                crate::nav_settings::from_scenario(&view.nav(), &saved.nav),
                view.deadline(),
                view.terminal_shot().map(str::to_owned),
            )
        };
        session.live_prepare_script(scenario).unwrap();
        session.pump_status();
        let published = session
            .nav_publish
            .lock()
            .unwrap()
            .settings
            .as_ref()
            .clone();
        assert_eq!(published, session.effective_nav());
        assert!(published.allow_teleports && published.allow_wilderness);
        assert_eq!(published.allow_bank_fetch, baseline.allow_bank_fetch);
        assert!(
            !published.pause_script_on_manual_walk_abort,
            "the saved OFF preference survives each scenario overlay"
        );
        assert_eq!(published.color_path, baseline.color_path);
        match choice {
            Some(on) => {
                assert_eq!(published.show_nav_path, on);
                assert_eq!(published.collision_fill, on);
                assert_eq!(published.client_trail, on);
            }
            None => assert_eq!(published, baseline),
        }
        let runner = session.core.live_runner();
        assert_eq!(
            runner.deadline_for_test(),
            Some(
                frontend_core::test_support::live::budget_s_from_env_for_test().unwrap_or(deadline)
            )
        );
        assert_eq!(runner.terminal_shot(), terminal_shot.as_deref());
        assert_eq!(session.nav_overlay.as_ref(), Some(&baseline));
        assert_eq!(crate::ui_state::load(&session.preferences).nav, saved.nav);
    }
}

#[test]
fn live_force_layers_does_not_write_panel_ui() {
    seed_ui_state(&crate::ui_state::PanelUiState {
        last_focus: None,
        ..Default::default()
    });
    let mut s = Session::new();
    let scenario = frontend_core::live::Scenario::get("nav_door").unwrap();
    s.nav_overlay = Some(crate::nav_settings::from_scenario(
        &scenario.view().nav(),
        &s.ui.nav,
    ));
    assert!(
        s.effective_nav().show_nav_path,
        "the live overlay drives the effective paint layers at runtime"
    );
    // A save/load roundtrip of the panel prefs must not persist the
    // overlay bools: `nav_overlay` is session-only.
    let ui = crate::ui_state::load(&s.preferences);
    assert!(
        !ui.nav.show_nav_path,
        "forced layers must never reach panel-ui.json"
    );
    let mut s = preparation_only_session();
    s.live_prepare_script(frontend_core::live::Scenario::get("nav_door").unwrap())
        .expect("prepare");
    assert!(
        s.nav_overlay
            .as_ref()
            .is_some_and(|n| n.show_nav_path && n.collision_fill && n.camera_follow),
        "nav scenario arms the live overlay"
    );
    let ui = crate::ui_state::load(&s.preferences);
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
    s.core.insert_slot_io(
        "a",
        SlotIo {
            input: Arc::clone(&a_in),
            pixels: FrameBuf::new(),
        },
    );
    s.core.insert_slot_io(
        "b",
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
fn focus_first_profile_noop_when_empty() {
    let path = tmp_vault("focus-empty.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
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
        !s.map_desk.input_connected(),
        "drain drops while the pane is closed"
    );
}

#[test]
fn closing_game_pane_releases_a_captured_mouse_hold() {
    let input = SlotInput::new();
    let mut s = Session::new();
    s.core.set_spawn_workers(false);
    s.core.set_play(Some(empty_play()));
    s.core.insert_slot_io(
        "alice",
        SlotIo {
            input: Arc::clone(&input),
            pixels: FrameBuf::new(),
        },
    );
    s.set_focus_for_test("alice");
    s.set_game_pane_open(true);
    // The devices follow the saved capture preference (on by default); this
    // test starts from the attached state they reach.
    s.capture_on("alice");
    let sender = s
        .map_desk
        .duplicate_input_sender()
        .expect("capture is attached to the focused slot");
    assert!(sender.send(InputEv::Down {
        button: 2,
        x: 10,
        y: 20,
    }));
    let mut shell = client::client::GameShell::new();
    input.drain(&mut shell);
    assert_eq!(shell.mouse_button, 2);

    s.set_game_pane_open(false);
    input.drain(&mut shell);
    assert_eq!(shell.mouse_button, 0);
    assert!(!s.map_desk.input_connected());
}

#[test]
fn reopening_game_pane_resumes_capture_drain() {
    let mut s = Session::new();
    s.core.insert_slot_io(
        "alice",
        SlotIo {
            input: SlotInput::new(),
            pixels: FrameBuf::new(),
        },
    );
    s.select("alice");
    s.set_capture(true);
    assert!(s.map_desk.input_connected());
    s.set_game_pane_open(false);
    assert!(!s.map_desk.input_connected());
    s.set_game_pane_open(true);
    assert!(s.map_desk.input_connected(), "pref on resumes the drain");
}

#[test]
fn set_capture_persists_to_panel_ui() {
    seed_ui_state(&crate::ui_state::PanelUiState::default());
    let mut s = Session::new();
    s.set_capture(false);
    await_preference(&mut s, "capture", serde_json::json!(false));
    assert!(!crate::ui_state::load(&s.preferences).capture);
    s.set_capture(true);
    await_preference(&mut s, "capture", serde_json::json!(true));
    assert!(crate::ui_state::load(&s.preferences).capture);
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
    let mut play = frontend_core::test_support::empty_play(&host_play::PlayOptions {
        host: "127.0.0.1".into(),
        transport: client::Transport::Tcp,
        port: 43594,
        cache_dir: "/tmp".into(),
        lowmem: true,
        mainland: false,
    });
    let arm = SlotArm::new(7, false);
    play.attach_arm("alice", Arc::clone(&arm));
    s.core.set_play(Some(play));
    s.core.fleet_mut().add("alice");
    s.logout("alice");
    assert!(s.core.fleet().latched("alice"));

    s.login("alice");

    assert!(arm.wants_login());
    assert!(!arm.wants_logout());
    assert!(!arm.login_latched());
    assert!(!s.core.fleet().latched("alice"));
    assert_eq!(s.focused_name().as_deref(), Some("alice"));
}

#[test]
fn load_refresh_keeps_an_explicit_non_auto_login() {
    let path = tmp_vault("load-refresh-explicit-login.vault");
    let mut session = Session::new();
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    vault.put(profile("alice", "pw", 42));
    install_vault(&mut session.core, vault);
    let mut play = empty_play();
    let arm = SlotArm::new(42, false);
    arm.arm_explicit_login();
    play.attach_arm("alice", Arc::clone(&arm));
    session.core.set_play(Some(play));
    session.core.fleet_mut().add("alice");

    assert!(!session.load("alice"));
    assert!(
        arm.wants_login(),
        "refreshing a non-auto profile must not withdraw explicit Log in"
    );
}

#[test]
fn load_keeps_a_persisted_auto_login_member_logged_out() {
    let path = tmp_vault("load-persisted-logout-auto.vault");
    let mut session = Session::new();
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    let alice = profile("alice", "pw", 42).auto_login(true);
    vault.put(alice);
    install_vault(&mut session.core, vault);
    let mut play = empty_play();
    let arm = SlotArm::new(42, true);
    arm.hold_logged_out();
    play.attach_arm("alice", Arc::clone(&arm));
    session.core.set_play(Some(play));
    session.core.fleet_mut().add("alice");

    assert!(!session.load("alice"));
    assert!(arm.login_latched());
    assert!(
        !arm.wants_login(),
        "Load must not undo a persisted operator logout"
    );
}

#[test]
fn explicit_login_recreates_terminal_worker_and_reuses_slot_io() {
    let path = tmp_vault("terminal-worker-login.vault");
    let mut session = Session::new();
    install_vault(
        &mut session.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut session.core, profile("alice", "pw", 42));
    session.core.set_spawn_workers(false);
    session
        .core
        .set_play(Some(frontend_core::test_support::empty_play(
            &host_play::PlayOptions {
                host: "127.0.0.1".into(),
                transport: client::Transport::Tcp,
                port: 43594,
                cache_dir: "/tmp".into(),
                lowmem: true,
                mainland: false,
            },
        )));
    let input = SlotInput::new();
    session.core.insert_slot_io(
        "alice",
        SlotIo {
            input: Arc::clone(&input),
            pixels: FrameBuf::new(),
        },
    );

    session.login("alice");

    let replacement = session
        .core
        .play_for_test()
        .and_then(|play| play.arm("alice"))
        .expect("explicit login recreates the terminal worker arm");
    assert!(replacement.wants_login());
    assert!(Arc::ptr_eq(
        &session.core.slot_io("alice").unwrap().input,
        &input
    ));
}

#[test]
fn login_all_recreates_terminal_worker_and_reuses_slot_io() {
    let path = tmp_vault("terminal-worker-login-all.vault");
    let mut session = Session::new();
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    vault.put(profile("alice", "pw", 42));
    install_vault(&mut session.core, vault);
    session.core.set_spawn_workers(false);
    let play = empty_play();
    play.statuses.lock().unwrap().push(SlotStatus {
        username: "alice".into(),
        startup_phase: StartupPhase::Error,
        worker_terminal: Some(host_play::WorkerTerminal::Panicked),
        ..SlotStatus::default()
    });
    session.core.set_play(Some(play));
    session.core.fleet_mut().add("alice");
    let input = SlotInput::new();
    session.core.insert_slot_io(
        "alice",
        SlotIo {
            input: Arc::clone(&input),
            pixels: FrameBuf::new(),
        },
    );
    assert!(session.core.play_for_test().unwrap().arm("alice").is_none());

    session.login_all();

    let replacement = session
        .core
        .play_for_test()
        .and_then(|play| play.arm("alice"))
        .expect("Login all recreates the terminal worker arm");
    assert!(replacement.wants_login());
    assert!(Arc::ptr_eq(
        &session.core.slot_io("alice").unwrap().input,
        &input
    ));
}

#[test]
fn selecting_a_terminal_member_preserves_its_failure_until_explicit_login() {
    let path = tmp_vault("terminal-worker-select.vault");
    let mut session = Session::new();
    install_vault(
        &mut session.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut session.core, profile("alice", "pw", 42));
    session.core.set_spawn_workers(false);
    let play = empty_play();
    play.statuses.lock().unwrap().push(SlotStatus {
        username: "alice".into(),
        worker_terminal: Some(host_play::WorkerTerminal::Panicked),
        error: Some("slot worker panicked: synthetic".into()),
        ..SlotStatus::default()
    });
    session.core.set_play(Some(play));
    session.core.fleet_mut().add("alice");
    session.core.insert_slot_io(
        "alice",
        SlotIo {
            input: SlotInput::new(),
            pixels: FrameBuf::new(),
        },
    );

    session.select("alice");

    assert!(
        session.core.play_for_test().unwrap().arm("alice").is_none(),
        "focus alone must not replace a terminal worker"
    );
    let row = &session.core.play_for_test().unwrap().statuses()[0];
    assert_eq!(
        row.worker_terminal,
        Some(host_play::WorkerTerminal::Panicked)
    );
    assert_eq!(
        row.error.as_deref(),
        Some("slot worker panicked: synthetic")
    );
}

#[test]
fn explicit_login_arms_a_fresh_non_auto_profile() {
    let path = tmp_vault("fresh-explicit-login.vault");
    let mut session = Session::new();
    install_vault(
        &mut session.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    let alice = profile("alice", "pw", 42).auto_login(false);
    vault_upsert(&mut session.core, alice);
    session.core.set_spawn_workers(false);
    session
        .core
        .set_play(Some(frontend_core::test_support::empty_play(
            &host_play::PlayOptions {
                host: "127.0.0.1".into(),
                transport: client::Transport::Tcp,
                port: 43594,
                cache_dir: "/tmp".into(),
                lowmem: true,
                mainland: false,
            },
        )));

    session.login("alice");

    let arm = session
        .core
        .play_for_test()
        .and_then(|play| play.arm("alice"))
        .expect("fresh explicit login creates an arm");
    assert!(arm.wants_login());
    assert!(!arm.auto_login());
    assert!(!arm.login_latched());
}

#[test]
fn arm_login_all_cancels_pending_logout() {
    // A title-screen member keeps want_logout=true (the slot body only
    // clears it when it observes ingame); Login all must cancel it or
    // the handshake would be undone on the first ingame frame.
    let arm = SlotArm::new(7, false);
    arm.request_logout();
    arm.arm_explicit_login();
    assert!(arm.wants_login());
    assert!(!arm.wants_logout());
    assert!(!arm.login_latched());
}

/// Operator report: the editor showed "edit aindniK" with Hans's user and
/// password after Hans was loaded onto the wall, and Save renamed aindniK
/// onto Hans (overwriting Hans, deleting aindniK). Loading or focusing
/// another profile leaves the editor's fields on the edited profile.
#[test]
fn loading_another_profile_while_editing_keeps_the_editor_on_its_profile() {
    let path = tmp_vault("edit-focus-desync.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    for p in [
        profile("aindniK", "kpass", 42),
        profile("Hans", "hpass", 43),
    ] {
        vault_upsert(&mut s.core, p);
    }

    s.begin_edit_profile(Some("aindniK"));
    s.load("Hans");
    *s.profile_editor.draft_mut().settings_mut().world_mut() = Some(2);
    let saved = s.save_credentials();
    s.core.flush_writes();
    s.pump_status();

    let disk = VaultFixture::unlock(&path, "test-passphrase-01");
    let row = |name: &str| {
        disk.profile(name)
            .map(|p| (p.uid(), p.draft().password_for_test().to_string()))
    };
    assert_eq!(row("Hans"), Some((43, "hpass".into())), "Hans untouched");
    assert_eq!(row("aindniK"), Some((42, "kpass".into())), "aindniK kept");
    assert_eq!(
        disk.profile("aindniK").unwrap().draft().settings().world(),
        Some(2)
    );
    assert!(saved, "{:?}", s.error);
}

#[test]
fn renaming_onto_an_existing_username_is_refused_and_writes_nothing() {
    let path = tmp_vault("rename-onto-existing.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    for p in [
        profile("aindniK", "kpass", 42),
        profile("Hans", "hpass", 43),
    ] {
        vault_upsert(&mut s.core, p);
    }

    s.begin_edit_profile(Some("aindniK"));
    *s.profile_editor.draft_mut().username_mut() = "Hans".into();
    *s.profile_editor.draft_mut().password_mut() = "typed".into();
    let saved = s.save_credentials();
    s.core.flush_writes();
    s.pump_status();

    let disk = VaultFixture::unlock(&path, "test-passphrase-01");
    let row = |name: &str| {
        disk.profile(name)
            .map(|p| (p.uid(), p.draft().password_for_test().to_string()))
    };
    assert_eq!(row("Hans"), Some((43, "hpass".into())), "Hans untouched");
    assert_eq!(row("aindniK"), Some((42, "kpass".into())), "aindniK kept");
    assert!(!saved);
    assert_eq!(
        s.error.as_deref(),
        Some("credentials: a profile named Hans already exists")
    );
    assert_eq!(
        s.profile_editor.target(),
        Some("aindniK"),
        "the form stays open"
    );
}

#[test]
fn a_new_profile_with_an_existing_username_is_refused_and_writes_nothing() {
    let path = tmp_vault("new-onto-existing.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("Hans", "hpass", 43));

    s.begin_edit_profile(None);
    *s.profile_editor.draft_mut().username_mut() = "Hans".into();
    *s.profile_editor.draft_mut().password_mut() = "typed".into();
    let saved = s.save_credentials();
    s.core.flush_writes();
    s.pump_status();

    let disk = VaultFixture::unlock(&path, "test-passphrase-01");
    let hans = disk.profile("Hans").unwrap();
    assert_eq!(
        (hans.uid(), hans.draft().password_for_test()),
        (43, "hpass")
    );
    assert!(!saved);
    assert_eq!(
        s.error.as_deref(),
        Some("credentials: a profile named Hans already exists")
    );
    assert_eq!(s.profile_editor.target(), Some(""), "the form stays open");
}

/// Live finding: switching focus (the main-panel profile selector, the
/// rail, a wall load) while an edit form is open must never retarget the
/// form or rewrite its buffers — Save stays bound to the profile the form
/// was opened for.
#[test]
fn focus_changes_keep_the_edit_form_on_its_target() {
    let path = tmp_vault("edit-focus-keeps-target.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    for p in [
        profile("aindniK", "kpass", 42),
        profile("Hans", "hpass", 43),
    ] {
        vault_upsert(&mut s.core, p);
    }

    s.begin_edit_profile(Some("aindniK"));
    *s.profile_editor.draft_mut().password_mut() = "typed-but-unsaved".into();

    s.select("Hans");
    assert_eq!(
        s.profile_editor.target(),
        Some("aindniK"),
        "a focus change never retargets the form"
    );
    assert_eq!(
        (
            s.profile_editor.draft().username(),
            s.profile_editor.draft().password_for_test()
        ),
        ("aindniK", "typed-but-unsaved"),
        "a focus change never rewrites the buffers"
    );

    s.load("Hans");
    assert_eq!(
        s.profile_editor.target(),
        Some("aindniK"),
        "a wall load never retargets the form"
    );
    assert_eq!(
        (
            s.profile_editor.draft().username(),
            s.profile_editor.draft().password_for_test()
        ),
        ("aindniK", "typed-but-unsaved")
    );
}

/// Live finding: explicitly opening a different profile for editing while
/// the form holds unsaved changes silently discarded the draft. The form
/// must stay on its target until the operator confirms the switch.
#[test]
fn opening_another_profile_with_unsaved_changes_keeps_the_form_until_confirmed() {
    let path = tmp_vault("edit-switch-confirm.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    for p in [
        profile("aindniK", "kpass", 42),
        profile("Hans", "hpass", 43),
    ] {
        vault_upsert(&mut s.core, p);
    }

    s.begin_edit_profile(Some("aindniK"));
    *s.profile_editor.draft_mut().password_mut() = "typed-but-unsaved".into();

    // Explicitly opening Hans must not discard the dirty aindniK draft.
    s.begin_edit_profile(Some("Hans"));
    assert_eq!(
        s.profile_editor.target(),
        Some("aindniK"),
        "the form stays on its target until the switch is confirmed"
    );
    assert_eq!(
        (
            s.profile_editor.draft().username(),
            s.profile_editor.draft().password_for_test()
        ),
        ("aindniK", "typed-but-unsaved"),
        "the unconfirmed switch rewrites nothing"
    );
}

/// A clean form switches target at once, with no prompt staged.
#[test]
fn opening_another_profile_without_unsaved_changes_switches_at_once() {
    let path = tmp_vault("edit-switch-clean.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    for p in [
        profile("aindniK", "kpass", 42),
        profile("Hans", "hpass", 43),
    ] {
        vault_upsert(&mut s.core, p);
    }

    s.begin_edit_profile(Some("aindniK"));
    assert!(!s.edit_dirty(), "an untouched form is clean");
    s.begin_edit_profile(Some("Hans"));
    assert_eq!(
        s.profile_editor.pending(),
        None,
        "no prompt for a clean form"
    );
    assert_eq!(s.profile_editor.target(), Some("Hans"));
    assert_eq!(
        (
            s.profile_editor.draft().username(),
            s.profile_editor.draft().password_for_test()
        ),
        ("Hans", "hpass")
    );

    // A blank new-profile form is clean too.
    s.begin_edit_profile(None);
    assert_eq!(s.profile_editor.target(), Some(""));
    assert!(!s.edit_dirty());
}

#[test]
fn save_credentials_upserts_under_username_key_keeping_uid() {
    let path = tmp_vault("save-creds.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "oldpass", 42));

    // Save commits the open form, so alice's is open for the edits below.
    s.begin_edit_profile(Some("alice"));
    *s.profile_editor.draft_mut().username_mut() = "alice".into();
    *s.profile_editor.draft_mut().password_mut() = "newpass".into();
    assert!(s.save_credentials());
    s.core.flush_writes();
    s.pump_status();

    let p = s.core.profile("alice").unwrap();
    assert_eq!(p.draft().password_for_test(), "newpass");
    assert_eq!(p.uid(), 42, "save must keep the existing uid");
}

#[test]
fn profile_editor_persists_trimmed_global_clue_duel_partner() {
    let path = tmp_vault("clue-duel-partner.vault");
    let mut session = Session::new();
    install_vault(
        &mut session.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut session.core, profile("alice", "pw", 42));

    session.begin_edit_profile(Some("alice"));
    *session
        .profile_editor
        .draft_mut()
        .settings_mut()
        .clue_duel_partner_mut() = "  Helper_Account  ".into();
    assert!(session.save_credentials());
    session.core.flush_writes();
    session.pump_status();

    assert_eq!(
        session
            .core
            .profile("alice")
            .unwrap()
            .draft()
            .settings()
            .clue_duel_partner(),
        "Helper_Account"
    );
}

#[test]
fn chooser_world_edit_persists_and_updates_running_slot() {
    let path = tmp_vault("world-choice.vault");
    let mut session = Session::new();
    install_vault(
        &mut session.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut session.core, profile("alice", "pw", 42));
    let mut play = frontend_core::test_support::empty_play(&host_play::PlayOptions {
        host: "127.0.0.1".into(),
        transport: client::Transport::Tcp,
        port: 43594,
        cache_dir: "/tmp".into(),
        lowmem: true,
        mainland: false,
    });
    let arm = SlotArm::new(42, false);
    play.attach_arm("alice", Arc::clone(&arm));
    session.core.set_play(Some(play));
    session.begin_edit_profile(Some("alice"));
    assert_eq!(session.profile_editor.draft().settings().world(), None);
    *session
        .profile_editor
        .draft_mut()
        .settings_mut()
        .world_mut() = Some(2);
    assert!(session.save_credentials());
    session.core.flush_writes();
    assert_eq!(
        VaultFixture::unlock(&path, "test-passphrase-01")
            .profile("alice")
            .expect("alice is stored")
            .draft()
            .settings()
            .world(),
        Some(2)
    );
    assert_eq!(
        arm.world_choice(),
        Some(2),
        "the profile-save path must update a running slot's next handshake"
    );
    *session
        .profile_editor
        .draft_mut()
        .settings_mut()
        .world_mut() = None;
    *session.profile_editor.draft_mut().password_mut() = "updated".into();
    // Leave the editor first, dropping the blank-world draft: Save commits an
    // open form, and a re-opened one starts from the saved row, pin included,
    // so this save must not unpin the account.
    session.pump_status();
    session.cancel_edit_profile();
    session.begin_edit_profile(Some("alice"));
    *session.profile_editor.draft_mut().password_mut() = "updated".into();
    assert!(session.save_credentials());
    session.core.flush_writes();
    session.pump_status();
    assert_eq!(
        VaultFixture::unlock(&path, "test-passphrase-01")
            .profile("alice")
            .expect("alice is stored")
            .draft()
            .settings()
            .world(),
        Some(2),
        "a credentials save from a re-opened form must not unpin an account"
    );
    assert_eq!(
        arm.world_choice(),
        Some(2),
        "a save from a re-opened form must preserve the running slot's pin"
    );
}

#[test]
fn save_credentials_creates_new_profile_when_username_is_new() {
    let path = tmp_vault("new-user.vault");
    let mut s = Session::new();
    hydrate_panel_preferences(&mut s);
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));

    // Save commits the open form: the new-profile form takes the edits.
    s.begin_edit_profile(None);
    *s.profile_editor.draft_mut().username_mut() = "bob".into();
    *s.profile_editor.draft_mut().password_mut() = "bobpass".into();
    assert!(s.save_credentials());
    s.core.flush_writes();
    s.pump_status();
    assert_eq!(s.focused_name().as_deref(), Some("bob"));
    assert!(s.core.slots().contains_key("bob"));

    let p = s.core.profile("bob").unwrap();
    assert_eq!(p.draft().password_for_test(), "bobpass");
    assert_ne!(p.uid(), 42, "a new profile gets a fresh uid");
    assert_eq!(
        s.core.profile("alice").unwrap().draft().password_for_test(),
        "pw",
        "saving a new username must not touch existing profiles"
    );
}

#[test]
fn save_credentials_rejects_empty_username() {
    let path = tmp_vault("empty-user.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    // The refusal is the open form's own: the new-profile form takes the edits.
    s.begin_edit_profile(None);
    *s.profile_editor.draft_mut().username_mut() = "  ".into();
    *s.profile_editor.draft_mut().password_mut() = "x".into();
    assert!(!s.save_credentials());
    assert!(s.error.is_some());
}

#[test]
fn save_credentials_without_focus_upserts_spawns_and_selects() {
    let path = tmp_vault("empty-first-run.vault");
    let mut s = Session::new();
    hydrate_panel_preferences(&mut s);
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    assert!(s.focused_name().is_none());
    // Save commits the open form: the new-profile form takes the edits.
    s.begin_edit_profile(None);
    *s.profile_editor.draft_mut().username_mut() = "test".into();
    *s.profile_editor.draft_mut().password_mut() = "test".into();
    assert!(s.save_credentials());
    s.core.flush_writes();
    s.pump_status();
    assert!(s.core.profile("test").is_some());
    assert_eq!(s.focused_name().as_deref(), Some("test"));
    assert!(s.core.slots().contains_key("test"));
}

#[test]
fn first_profile_save_waits_for_preferences_before_selecting_and_loading_once() {
    let iso = IsolatedStores::enter("panel-first-profile-cold-preferences");
    let mut session = Session::new();
    session.core.set_spawn_workers(false);
    let (entered, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release, release_rx) = std::sync::mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let first = std::sync::atomic::AtomicBool::new(true);
    let service = host_play::local_state::LocalStateService::open_process_with_read_stall_for_test(
        host_play::local_state::StateRoot::new(iso.dir().join("cold-state")),
        Arc::new(move || {
            if first.swap(false, Ordering::SeqCst) {
                let _ = entered.send(());
                let _ = release_rx.lock().unwrap().recv();
            }
        }),
    );
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    session.core.bind_local_state(service);
    session.preferences = session.core.preferences().unwrap();
    session.preferences_ready = false;
    let path = iso.dir().join("vault");
    assert!(session.unlock_at(&path, "test-passphrase-01"));
    session.begin_edit_profile(None);
    *session.profile_editor.draft_mut().username_mut() = "alice".into();
    *session.profile_editor.draft_mut().password_mut() = "pw".into();
    *session
        .profile_editor
        .draft_mut()
        .settings_mut()
        .auto_login_mut() = true;
    assert!(session.save_credentials());
    session.core.flush_writes();

    for _ in 0..4 {
        session.poll_panel_preferences();
        session.pump_status();
        assert!(!session.preferences.ready());
        assert!(session.core.profile("alice").is_some());
        assert!(
            session.core.slots().is_empty(),
            "saving the first profile must not select or load before preferences are ready"
        );
        assert!(session.focused_name().is_none());
    }

    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while session.core.slots().is_empty() && Instant::now() < deadline {
        session.poll_panel_preferences();
        session.pump_status();
        thread::sleep(Duration::from_millis(5));
    }
    assert!(session.panel_preferences_ready());
    assert_eq!(session.focused_name().as_deref(), Some("alice"));
    let arm = session.core.play_for_test().unwrap().arm("alice").unwrap();
    assert!(arm.wants_login());
    let selections = |session: &Session| {
        session
            .core
            .commands()
            .reports()
            .filter(|report| report.command() == frontend_core::OperatorCommand::Select)
            .count()
    };
    // `select` also submits the render-focus mirror. Further pumps must
    // neither repeat that selection nor replace the loaded login arm.
    let selected = selections(&session);
    assert!(selected > 0);
    for _ in 0..4 {
        session.poll_panel_preferences();
        session.pump_status();
        assert_eq!(selections(&session), selected);
        assert_eq!(session.core.slots().len(), 1);
        assert!(Arc::ptr_eq(
            &arm,
            &session.core.play_for_test().unwrap().arm("alice").unwrap()
        ));
    }
}

#[test]
fn save_credentials_does_not_duplicate_running_slot() {
    let path = tmp_vault("no-dup-slot.vault");
    let mut s = Session::new();
    hydrate_panel_preferences(&mut s);
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    s.core.insert_slot_io(
        "alice",
        SlotIo {
            input: SlotInput::new(),
            pixels: FrameBuf::new(),
        },
    );
    // Save commits the open form, so alice's is open for the edits below.
    s.begin_edit_profile(Some("alice"));
    *s.profile_editor.draft_mut().username_mut() = "alice".into();
    *s.profile_editor.draft_mut().password_mut() = "newpw".into();
    assert!(s.save_credentials());
    s.core.flush_writes();
    s.pump_status();
    assert_eq!(s.core.slots().len(), 1);
    assert_eq!(s.focused_name().as_deref(), Some("alice"));
}

#[test]
fn save_credentials_rename_editing_profile_replaces_old_key() {
    let path = tmp_vault("rename-creds.vault");
    let mut s = Session::new();
    hydrate_panel_preferences(&mut s);
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));

    s.begin_edit_profile(Some("alice"));
    *s.profile_editor.draft_mut().username_mut() = "bob".into();
    *s.profile_editor.draft_mut().password_mut() = "bobpass".into();
    assert!(s.save_credentials());
    s.core.flush_writes();
    s.pump_status();

    assert!(
        s.core.profile("bob").is_some(),
        "rename must upsert the new key"
    );
    assert!(
        s.core.profile("alice").is_none(),
        "rename must remove the old key"
    );
    assert_eq!(
        s.core.profile("bob").unwrap().uid(),
        42,
        "rename keeps the old uid"
    );
    assert_eq!(s.focused_name().as_deref(), Some("bob"));
}

#[test]
#[cfg(unix)]
fn first_time_save_spawns_nothing_until_the_write_is_durable() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TestDir::new("save-first-time");
    let path = dir.join("vault.vault");
    let mut s = Session::new();
    hydrate_panel_preferences(&mut s);
    s.core.set_spawn_workers(false);
    s.core.set_play(Some(empty_play()));
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    // Save commits the open form: the new-profile form takes the edits.
    s.begin_edit_profile(None);
    *s.profile_editor.draft_mut().username_mut() = "alice".into();
    *s.profile_editor.draft_mut().password_mut() = "pw".into();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();

    assert!(s.save_credentials());
    assert!(
        s.core.play_for_test().unwrap().arm("alice").is_none(),
        "no worker before the write is durable"
    );
    s.core.flush_writes();
    s.pump_status();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();

    assert!(s
        .error
        .as_deref()
        .is_some_and(|e| e.starts_with("credentials:")));
    assert!(
        s.core.play_for_test().unwrap().arm("alice").is_none(),
        "no slot"
    );
    assert!(s.core.slot_io("alice").is_none());
    assert!(s.focused_name().is_none());
    assert!(s.core.profile("alice").is_none());

    // The same Save on a writable vault spawns once the write lands.
    assert!(s.save_credentials());
    assert!(s.core.play_for_test().unwrap().arm("alice").is_none());
    s.core.flush_writes();
    s.pump_status();
    assert!(s.core.play_for_test().unwrap().arm("alice").is_some());
    assert_eq!(s.focused_name().as_deref(), Some("alice"));
}

#[test]
#[cfg(unix)]
fn save_credentials_upsert_error_surfaces_on_session_error() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TestDir::new("save-err");
    let path = dir.join("vault.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    // Save commits the open form: the new-profile form takes the edits.
    s.begin_edit_profile(None);
    *s.profile_editor.draft_mut().username_mut() = "alice".into();
    *s.profile_editor.draft_mut().password_mut() = "pw".into();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    assert!(s.save_credentials(), "the edit is accepted and queued");
    s.core.flush_writes();
    s.pump_status();
    assert!(
        s.error
            .as_ref()
            .is_some_and(|e| e.starts_with("credentials:")),
        "a failed durable write must land on session.error, got {:?}",
        s.error
    );
    assert!(
        s.core.profile("alice").is_none(),
        "the unsaved profile is rolled back"
    );
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn clear_credentials_empties_fields_but_keeps_vault() {
    let path = tmp_vault("clear-creds.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    s.select("alice");
    *s.profile_editor.draft_mut().password_mut() = "edited".into();

    s.clear_credentials();
    assert!(s.profile_editor.draft().username().is_empty());
    assert!(s.profile_editor.draft().password_for_test().is_empty());
    assert!(
        s.core.profile("alice").is_some(),
        "clear must not delete the vault profile"
    );
}

#[test]
fn begin_edit_profile_loads_fields_and_opens_chooser() {
    let path = tmp_vault("edit-profile.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "secret", 42));

    s.begin_edit_profile(Some("alice"));
    assert!(s.wall.chooser_open);
    assert_eq!(s.profile_editor.target(), Some("alice"));
    assert_eq!(s.profile_editor.draft().username(), "alice");
    assert_eq!(s.profile_editor.draft().password_for_test(), "secret");

    s.begin_edit_profile(None);
    assert_eq!(s.profile_editor.target(), Some(""));
    assert!(s.profile_editor.draft().username().is_empty());
    assert!(s.profile_editor.draft().password_for_test().is_empty());

    s.cancel_edit_profile();
    assert!(!s.profile_editor.is_open());
    assert!(s.wall.chooser_open, "cancel leaves the picker open");
}

#[test]
fn begin_edit_profile_loads_guardian_settings() {
    let path = tmp_vault("edit-guardian.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(
        &mut s.core,
        with_random_events(
            with_lamp_skill(
                with_lamp_auto(profile("alice", "secret", 42).auto_login(true), false),
                "magic",
            ),
            false,
        ),
    );

    s.begin_edit_profile(Some("alice"));
    assert!(s.profile_editor.draft().settings().auto_login());
    assert!(!s.profile_editor.draft().settings().lamp_auto());
    assert!(!s.profile_editor.draft().settings().random_events());
    assert_eq!(s.profile_editor.draft().settings().lamp_skill(), "magic");

    s.begin_edit_profile(None);
    assert!(!s.profile_editor.draft().settings().auto_login());
    assert!(s.profile_editor.draft().settings().lamp_auto());
    assert!(s.profile_editor.draft().settings().random_events());
    assert_eq!(s.profile_editor.draft().settings().lamp_skill(), "strength");
}

#[test]
fn set_multibox_off_cancels_picker_edit() {
    let path = tmp_vault("edit-off.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    s.begin_edit_profile(Some("alice"));
    s.set_multibox(true);
    s.set_multibox(false);
    assert!(!s.profile_editor.is_open());
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
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    assert!(s.set_auto_login("alice", true));
    assert!(s.core.profile("alice").unwrap().auto_login());
    assert!(
        s.core.slots().is_empty(),
        "set_auto_login must not spawn a slot"
    );
    assert!(s.set_auto_login("alice", false));
    assert!(!s.core.profile("alice").unwrap().auto_login());
}

#[test]
fn set_random_settings_upserts_all_three_fields_without_spawning() {
    let path = tmp_vault("random-settings.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    assert!(s.set_random_settings("alice", false, "magic", false));
    let p = s.core.profile("alice").unwrap();
    assert!(!p.random_events(), "random events persist off");
    assert_eq!(p.draft().settings().lamp_skill(), "magic");
    assert!(!p.draft().settings().lamp_auto(), "lamp auto persists off");
    assert!(
        s.core.slots().is_empty(),
        "set_random_settings must not spawn a slot"
    );
    assert!(s.set_random_settings("alice", true, "strength", true));
    let p = s.core.profile("alice").unwrap();
    assert!(p.random_events());
    assert_eq!(p.draft().settings().lamp_skill(), "strength");
    assert!(p.draft().settings().lamp_auto());
}

#[test]
fn set_random_event_policy_persists_without_spawning() {
    let path = tmp_vault("random-event-policy.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    assert!(s.set_random_event_policy(
        "alice",
        "River Troll",
        frontend_core::GuardianPolicy::Ignore
    ));
    assert_eq!(
        s.core
            .profile("alice")
            .unwrap()
            .random_event_policy("river troll"),
        frontend_core::GuardianPolicy::Ignore
    );
    assert!(
        s.core.slots().is_empty(),
        "editing a policy must not spawn a slot"
    );
}

#[test]
fn set_survival_persists_and_mirrors_running_arm() {
    let path = tmp_vault("survival-settings-arm.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    let mut play = frontend_core::test_support::empty_play(&host_play::PlayOptions {
        host: "127.0.0.1".into(),
        transport: client::Transport::Tcp,
        port: 43594,
        cache_dir: "/tmp".into(),
        lowmem: true,
        mainland: false,
    });
    let arm = SlotArm::new(42, false);
    // The arm's mirrored byte: 0 = when needed, 1 = always, 2 = never
    // (the script policy's fixed encoding).
    assert_eq!(
        arm.cure_poison.load(Ordering::Relaxed),
        0,
        "default: cure when needed"
    );
    play.attach_arm("alice", Arc::clone(&arm));
    s.core.set_play(Some(play));
    assert!(s.set_survival("alice", frontend_core::CurePoison::Never));
    s.core.flush_writes();
    assert_eq!(
        arm.cure_poison.load(Ordering::Relaxed),
        2,
        "the choice must reach the live arm without respawn"
    );
    assert_eq!(
        s.core.profile("alice").unwrap().survival().cure_poison(),
        frontend_core::CurePoison::Never,
        "the choice persists on the profile"
    );
    assert!(s.set_survival("alice", frontend_core::CurePoison::Always));
    s.core.flush_writes();
    assert_eq!(arm.cure_poison.load(Ordering::Relaxed), 1);
}

#[test]
fn set_random_settings_mirrors_running_arm() {
    let path = tmp_vault("random-settings-arm.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    let mut play = frontend_core::test_support::empty_play(&host_play::PlayOptions {
        host: "127.0.0.1".into(),
        transport: client::Transport::Tcp,
        port: 43594,
        cache_dir: "/tmp".into(),
        lowmem: true,
        mainland: false,
    });
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
    s.core.set_play(Some(play));
    assert!(s.set_random_settings("alice", false, "attack", true));
    s.core.flush_writes();
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
    s.core.flush_writes();
    assert!(arm.random_events.load(Ordering::Relaxed));
    assert!(arm.lamp_auto.load(Ordering::Relaxed));
    assert_eq!(arm.lamp_skill.lock().unwrap().as_str(), "strength");
}

#[test]
fn set_random_settings_rejects_unknown_or_locked_vault() {
    let path = tmp_vault("random-settings-unknown.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
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
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    s.select("alice");
    assert!(s.focused_lowmem(), "fresh profile defaults to lowmem");
    assert!(s.set_focused_lowmem(false));
    settle_panel_writes(&mut s);
    assert!(!s.core.profile("alice").unwrap().lowmem());
    assert!(!s.focused_lowmem(), "focused profile reflects the setting");
    assert!(s.set_focused_lowmem(true));
    settle_panel_writes(&mut s);
    assert!(s.core.profile("alice").unwrap().lowmem());
}

#[test]
fn set_auto_login_mirrors_running_arm() {
    let path = tmp_vault("auto-login-arm.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    let mut play = frontend_core::test_support::empty_play(&host_play::PlayOptions {
        host: "127.0.0.1".into(),
        transport: client::Transport::Tcp,
        port: 43594,
        cache_dir: "/tmp".into(),
        lowmem: true,
        mainland: false,
    });
    let arm = SlotArm::new(42, false);
    play.attach_arm("alice", Arc::clone(&arm));
    s.core.set_play(Some(play));
    assert!(s.set_auto_login("alice", true));
    s.core.flush_writes();
    assert!(arm.auto_login());
    assert!(s.set_auto_login("alice", false));
    s.core.flush_writes();
    assert!(!arm.auto_login());
}

#[test]
fn set_renderer_writes_renderer_by_for_focused() {
    let mut s = Session::new();
    s.set_focus_for_test("alice");
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
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    s.select("alice");
    assert_eq!(s.focused_raster(), frontend_core::command::RasterMode::Gpu);
    assert!(!s.core.slot_io("alice").unwrap().input.prefer_cpu());
    assert!(s.set_focused_lowmem(false));
    settle_panel_writes(&mut s);
    s.core.flush_writes();
    assert!(
        !s.core.profile("alice").unwrap().lowmem(),
        "the memory switch is persisted before an unrelated raster write"
    );
    assert!(s.set_focused_raster(frontend_core::command::RasterMode::Off));
    settle_panel_writes(&mut s);
    assert_eq!(s.focused_raster(), frontend_core::command::RasterMode::Off);
    assert_eq!(
        s.focus.lock().unwrap().renderer_by.get("alice").copied(),
        Some(false)
    );
    assert!(
        !s.core.slot_io("alice").unwrap().input.prefer_cpu(),
        "Off must not flip the GPU/CPU latch"
    );
    assert!(s.set_focused_raster(frontend_core::command::RasterMode::Cpu));
    settle_panel_writes(&mut s);
    assert_eq!(s.focused_raster(), frontend_core::command::RasterMode::Cpu);
    assert!(s.core.slot_io("alice").unwrap().input.prefer_cpu());
    assert_eq!(
        s.core.profile("alice").unwrap().raster(),
        frontend_core::command::RasterMode::Cpu
    );
    assert!(
        !s.core.profile("alice").unwrap().lowmem(),
        "changing raster must not restore the pre-toggle memory mode"
    );
}

#[test]
fn raster_switch_confirm_only_when_backend_changes_on_spawned_slot() {
    use frontend_core::command::RasterMode::*;
    // GPU↔CPU on a spawned slot is a drop+reattach (the Client and its
    // socket stay), so no logout/restart confirm is ever required —
    // not even Off↔Gpu/Cpu or a spawned slot.
    assert!(!Session::raster_switch_needs_confirm(Off, false, true));
    assert!(!Session::raster_switch_needs_confirm(Gpu, false, true));
    assert!(!Session::raster_switch_needs_confirm(Cpu, false, true));
    assert!(!Session::raster_switch_needs_confirm(Gpu, true, true));
    assert!(!Session::raster_switch_needs_confirm(Cpu, true, true));
    assert!(!Session::raster_switch_needs_confirm(Cpu, false, false));
}

#[test]
fn request_raster_cpu_on_spawned_slot_applies_immediately() {
    let path = tmp_vault("raster-no-confirm.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    s.select("alice");
    s.request_focused_raster(frontend_core::command::RasterMode::Cpu);
    settle_panel_writes(&mut s);
    assert_eq!(
        s.focused_raster(),
        frontend_core::command::RasterMode::Cpu,
        "GPU→CPU applies at once: drop+reattach, never a logout"
    );
    s.request_focused_raster(frontend_core::command::RasterMode::Off);
    settle_panel_writes(&mut s);
    assert_eq!(s.focused_raster(), frontend_core::command::RasterMode::Off);
    assert!(
        s.core.slots().contains_key("alice"),
        "Off must keep the slot"
    );
}

#[test]
fn raster_switch_keeps_slot_frame_buf_and_input() {
    let path = tmp_vault("raster-no-restart.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    s.select("alice");
    let slot = s.core.slot_io("alice").expect("select spawns the slot");
    let buf = Arc::clone(&slot.pixels);
    let inp = Arc::clone(&slot.input);
    assert!(s.set_focused_raster(frontend_core::command::RasterMode::Cpu));
    settle_panel_writes(&mut s);
    assert!(inp.prefer_cpu(), "GPU→CPU sets the slot's prefer_cpu latch");
    assert!(s.set_focused_raster(frontend_core::command::RasterMode::Gpu));
    settle_panel_writes(&mut s);
    assert!(!inp.prefer_cpu(), "CPU→GPU clears the prefer_cpu latch");
    let slot = s.core.slot_io("alice").expect("GPU↔CPU must keep the slot");
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
fn arm_for_profile_respects_auto_login_and_latch() {
    let path = tmp_vault("arm-for-profile.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    let p = profile("alice", "pw", 42).auto_login(true);
    vault_upsert(&mut s.core, p);
    let arm = s.core.arm_for_profile("alice").expect("arm");
    assert!(arm.wants_login());
    assert!(arm.auto_login());
    s.core.fleet_mut().latch_logout("alice");
    let arm = s.core.arm_for_profile("alice").expect("arm");
    assert!(!arm.wants_login(), "latch blocks handshake");
    assert!(arm.auto_login(), "profile auto_login stays on the arm");
}

#[test]
fn set_auto_login_rejects_unknown_profile_without_spawning() {
    let path = tmp_vault("auto-login-missing.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    assert!(!s.set_auto_login("nobody", true));
    assert!(s.error.is_some(), "missing profile sets the banner");
    assert!(s.core.slots().is_empty());
}

#[test]
fn logout_latches_member_until_login_all() {
    let mut s = Session::new();
    s.core.fleet_mut().add("alice");
    s.logout("alice");
    assert!(
        s.core.fleet().latched("alice"),
        "intentional logout latches"
    );
    assert!(!s.core.fleet().should_auto_login("alice", true));
    s.login_all();
    assert!(
        !s.core.fleet().latched("alice"),
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
        s.core.fleet_mut().add(name);
        s.core.insert_slot_io(
            name,
            SlotIo {
                input: SlotInput::new(),
                pixels: FrameBuf::new(),
            },
        );
    }
    s.set_focus_for_test("alice");
    s.core.set_play(Some(play));

    s.login_all();

    assert!(alice.wants_login());
    assert!(bob.wants_login());
    assert!(
        s.core
            .play_for_test()
            .unwrap()
            .login_queue_uids()
            .is_empty(),
        "only worker Queueing transitions create FIFO membership"
    );
}

/// Log out follows the shared projection for the focused bot. A connected
/// session is eligible before the scene is ready and in game, a refused login that the
/// slot still retries is eligible too, and a bot that never logged in or
/// that Log out already parked is not (an operator logout's latch does not
/// make it eligible again; only the repeat guard's park does).
#[test]
fn logout_retry_focused_eligibility_follows_the_shared_projection() {
    use frontend_core::LogoutEligibility as E;
    let mut s = Session::new();
    let mut play = empty_play();
    for (name, uid) in [("alice", 1), ("bob", 2)] {
        play.attach_arm(name, SlotArm::new(uid, true));
        s.core.fleet_mut().add(name);
    }
    s.core.set_play(Some(play));
    s.set_focus_for_test("alice");
    assert_eq!(s.focused_logout_eligibility(), Some(E::NotLoggedIn));
    let publish = |s: &Session, edit: &dyn Fn(&mut SlotStatus)| {
        let play = s.core.play_for_test().unwrap();
        let mut rows = play.statuses.lock().unwrap();
        if rows.is_empty() {
            rows.push(SlotStatus {
                username: "alice".into(),
                ..SlotStatus::default()
            });
        }
        edit(&mut rows[0]);
    };
    publish(&s, &|row| {
        row.connected = true;
        row.ingame = false;
        row.startup_phase = StartupPhase::LoadingScene;
    });
    s.core.poll_host();
    assert_eq!(
        s.focused_logout_eligibility(),
        Some(E::LoggingIn),
        "connected, scene still loading"
    );
    assert!(!s.focused_ingame());
    publish(&s, &|row| {
        row.ingame = true;
        row.scene_state = 2;
        row.startup_phase = StartupPhase::Ready;
    });
    s.core.poll_host();
    assert_eq!(s.focused_logout_eligibility(), Some(E::LoggedIn));

    // A refused login the slot still wants: the host retries it.
    s.core
        .play_for_test()
        .unwrap()
        .arm("alice")
        .unwrap()
        .arm_explicit_login();
    publish(&s, &|row| {
        row.connected = false;
        row.ingame = false;
        row.scene_state = 0;
        row.startup_phase = StartupPhase::Error;
        row.error = Some("Error connecting to server.".into());
    });
    s.core.poll_host();
    assert_eq!(s.focused_logout_eligibility(), Some(E::RetryingLogin));

    // Log out parks it: nothing is left to cancel.
    s.logout("alice");
    s.core.poll_host();
    assert_eq!(s.focused_logout_eligibility(), Some(E::NotLoggedIn));
    assert!(!s
        .core
        .play_for_test()
        .unwrap()
        .arm("alice")
        .unwrap()
        .wants_login());

    // The repeat guard's park (offline, unqueued, may hold script work) is
    // eligible; another bot's park is not this bot's.
    let arm = s.core.play_for_test().unwrap().arm("alice").unwrap();
    arm.arm_explicit_login();
    arm.complete_login_for_test();
    arm.exit_and_trip_repeat_guard_for_test();
    publish(&s, &|row| {
        row.login_latched = true;
        row.login_latch_reason = arm.login_latch_reason();
    });
    s.core.poll_host();
    assert_eq!(s.focused_logout_eligibility(), Some(E::GuardParked));
    s.set_focus_for_test("bob");
    assert_eq!(s.focused_logout_eligibility(), Some(E::NotLoggedIn));
    s.set_focus_for_test("nobody");
    assert_eq!(
        s.focused_logout_eligibility(),
        Some(E::NotLoggedIn),
        "an unprojected focus has nothing to cancel"
    );
}

/// The queue card reads each bot's own place from the shared fleet rows,
/// whichever bot is focused.
#[test]
fn queue_for_tracks_each_named_row_independent_of_focus() {
    let mut s = Session::new();
    let mut play = empty_play();
    for (name, uid) in [("alice", 1), ("bob", 2)] {
        play.attach_arm(name, SlotArm::new(uid, true));
        s.core.fleet_mut().add(name);
    }
    s.core.set_play(Some(play));
    s.set_focus_for_test("alice");
    s.core.poll_host();
    assert_eq!(s.queue_for("alice"), None, "no row published yet");
    let queued = |position, total| SlotStatus {
        startup_phase: StartupPhase::Queueing,
        queue_position: position,
        queue_total: total,
        ..SlotStatus::default()
    };
    s.core
        .play_for_test()
        .unwrap()
        .statuses
        .lock()
        .unwrap()
        .extend([
            SlotStatus {
                username: "alice".into(),
                ..queued(2, 3)
            },
            SlotStatus {
                username: "bob".into(),
                ..queued(1, 2)
            },
        ]);
    s.core.poll_host();
    let place = |position, total| Some(frontend_core::QueuePlace::new(position, total));
    assert_eq!(s.queue_for("alice"), place(2, 3));
    assert_eq!(s.queue_for("bob"), place(1, 2));

    s.set_focus_for_test("bob");
    s.core.poll_host();
    assert_eq!(s.queue_for("alice"), place(2, 3));
    assert_eq!(s.queue_for("bob"), place(1, 2));
}

#[test]
fn load_and_rail_remove_sync_focus_wall() {
    let path = tmp_vault("focus-wall-sync.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    vault_upsert(&mut s.core, profile("bob", "pw", 43));
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
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    s.load("alice");
    assert_eq!(s.focused_name().as_deref(), Some("alice"));
    s.rail_remove("alice");
    assert!(s.focused_name().is_none());
    assert!(s.core.members().is_empty());
}

#[test]
fn rail_remove_treats_loading_session_as_connected_without_blocking() {
    let mut s = Session::new();
    let mut play = empty_play();
    let arm = SlotArm::new(7, false);
    play.attach_arm("alice", Arc::clone(&arm));
    play.statuses.lock().unwrap().push(SlotStatus {
        username: "alice".into(),
        connected: true,
        ingame: false,
        ..SlotStatus::default()
    });
    s.core.set_play(Some(play));
    s.core.fleet_mut().add("alice");
    s.set_focus_for_test("alice");
    s.core.insert_slot_io(
        "alice",
        SlotIo {
            input: SlotInput::new(),
            pixels: FrameBuf::new(),
        },
    );

    let started = Instant::now();
    s.rail_remove_at("alice", started);
    assert!(
        arm.wants_logout(),
        "a connected loading session must receive clean Logout"
    );
    assert!(
        s.core.removal_pending("alice"),
        "clean removal remains pending until a later UI pump"
    );
    assert!(
        !arm.stop.load(Ordering::Relaxed),
        "rail removal must return without stopping the connected worker"
    );

    s.core.play_for_test().unwrap().statuses.lock().unwrap()[0].connected = false;
    s.core.advance_removals(started);
    assert!(
        arm.stop.load(Ordering::Relaxed),
        "the later pump stops the lifetime after disconnect"
    );
}

fn connected_removal_session(test_name: &str) -> (Session, Arc<SlotArm>) {
    let path = tmp_vault(test_name);
    let mut session = Session::new();
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    vault.put(profile("alice", "pw", 42));
    install_vault(&mut session.core, vault);

    let mut play = empty_play();
    let arm = SlotArm::new(42, true);
    play.attach_arm("alice", Arc::clone(&arm));
    play.statuses.lock().unwrap().push(SlotStatus {
        username: "alice".into(),
        connected: true,
        ..SlotStatus::default()
    });
    session.core.set_play(Some(play));
    session.core.fleet_mut().add("alice");
    session.core.insert_slot_io(
        "alice",
        SlotIo {
            input: SlotInput::new(),
            pixels: FrameBuf::new(),
        },
    );
    (session, arm)
}

#[test]
fn readd_during_rail_removal_cancels_the_old_lifetime() {
    let (mut session, arm) = connected_removal_session("rail-remove-readd.vault");
    let started = Instant::now();

    session.rail_remove_at("alice", started);
    assert!(!session.core.slots().contains_key("alice"));
    assert!(session.load("alice"));
    assert!(
        session.core.slots().contains_key("alice"),
        "re-adding must restore the running lifetime's IO"
    );

    session
        .core
        .play_for_test()
        .unwrap()
        .statuses
        .lock()
        .unwrap()[0]
        .connected = false;
    session
        .core
        .advance_removals(started + frontend_core::SLOT_REMOVE_TIMEOUT);

    let current = session.core.play_for_test().unwrap().arm("alice").unwrap();
    assert!(
        Arc::ptr_eq(&current, &arm),
        "the stale removal must not stop the re-added lifetime"
    );
}

#[test]
fn multibox_reseed_cancels_pending_removal_for_running_member() {
    let (mut session, arm) = connected_removal_session("rail-remove-multibox-reseed.vault");
    session.core.fleet_mut().add("bob");
    session.set_focus_for_test("bob");
    let started = Instant::now();

    session.rail_remove_at("alice", started);
    assert!(!session.core.slots().contains_key("alice"));
    session.set_multibox(false);
    session.set_multibox(true);

    assert!(session.core.members().iter().any(|name| name == "alice"));
    assert!(
        session.core.slots().contains_key("alice"),
        "re-seeding a running member must restore its retained IO"
    );
    assert!(!session.core.removal_pending("alice"));

    session
        .core
        .play_for_test()
        .unwrap()
        .statuses
        .lock()
        .unwrap()[0]
        .connected = false;
    session
        .core
        .advance_removals(started + frontend_core::SLOT_REMOVE_TIMEOUT);
    let current = session.core.play_for_test().unwrap().arm("alice").unwrap();
    assert!(Arc::ptr_eq(&current, &arm));
    assert!(
        !arm.stop.load(Ordering::Relaxed),
        "the cancelled removal must not stop the re-seeded lifetime"
    );
}

#[test]
fn login_during_rail_removal_cancels_the_old_timeout() {
    let (mut session, arm) = connected_removal_session("rail-remove-login.vault");
    let started = Instant::now();

    session.rail_remove_at("alice", started);
    session.login("alice");
    session
        .core
        .advance_removals(started + frontend_core::SLOT_REMOVE_TIMEOUT);

    let current = session.core.play_for_test().unwrap().arm("alice").unwrap();
    assert!(Arc::ptr_eq(&current, &arm));
    assert!(
        arm.wants_login(),
        "Log in must survive the cancelled removal"
    );
    assert!(
        session.core.slots().contains_key("alice"),
        "Log in must restore the running lifetime's IO"
    );
}

#[test]
fn rail_removal_pump_stops_its_lifetime_on_disconnect_or_timeout() {
    let started = Instant::now();
    let (mut disconnected, disconnected_arm) =
        connected_removal_session("rail-remove-disconnect.vault");
    disconnected.rail_remove_at("alice", started);
    disconnected
        .core
        .play_for_test()
        .unwrap()
        .statuses
        .lock()
        .unwrap()[0]
        .connected = false;
    disconnected.core.advance_removals(started);
    assert!(disconnected_arm.stop.load(Ordering::Relaxed));
    assert!(disconnected
        .core
        .play_for_test()
        .unwrap()
        .arm("alice")
        .is_none());
    assert!(!disconnected.core.removal_pending("alice"));

    let (mut timed_out, timed_out_arm) = connected_removal_session("rail-remove-timeout.vault");
    timed_out.rail_remove_at("alice", started);
    timed_out
        .core
        .advance_removals(started + frontend_core::SLOT_REMOVE_TIMEOUT - Duration::from_nanos(1));
    assert!(!timed_out_arm.stop.load(Ordering::Relaxed));
    assert!(timed_out
        .core
        .play_for_test()
        .unwrap()
        .arm("alice")
        .is_some());
    timed_out
        .core
        .advance_removals(started + frontend_core::SLOT_REMOVE_TIMEOUT);
    assert!(timed_out_arm.stop.load(Ordering::Relaxed));
    assert!(timed_out
        .core
        .play_for_test()
        .unwrap()
        .arm("alice")
        .is_none());
    assert!(!timed_out.core.removal_pending("alice"));
}

#[test]
fn removal_pump_retires_terminal_row_after_owner_arm_disappears() {
    let started = Instant::now();
    let (mut session, _) = connected_removal_session("rail-remove-terminal-owner.vault");
    session.rail_remove_at("alice", started);
    {
        let play = session.core.play_mut_for_test().unwrap();
        play.begin_stop_slot("alice");
        play.statuses.lock().unwrap().push(SlotStatus {
            username: "alice".into(),
            startup_phase: StartupPhase::Error,
            worker_terminal: Some(host_play::WorkerTerminal::Failed),
            ..SlotStatus::default()
        });
    }
    assert!(session.core.removal_pending("alice"));

    session.core.advance_removals(started);

    assert!(!session.core.removal_pending("alice"));
    assert!(
        session
            .core
            .play_for_test()
            .unwrap()
            .statuses()
            .iter()
            .all(|status| status.username != "alice"),
        "an off-wall terminal lifetime must not leave a ghost status row"
    );

    let (mut replaced, _) = connected_removal_session("rail-remove-replaced-owner.vault");
    replaced.rail_remove_at("alice", started);
    let replacement = SlotArm::new(42, true);
    replaced
        .core
        .play_mut_for_test()
        .unwrap()
        .attach_arm("alice", Arc::clone(&replacement));
    replaced
        .core
        .play_for_test()
        .unwrap()
        .statuses
        .lock()
        .unwrap()[0]
        .connected = false;

    replaced
        .core
        .advance_removals(started + frontend_core::SLOT_REMOVE_TIMEOUT);

    let current = replaced.core.play_for_test().unwrap().arm("alice").unwrap();
    assert!(Arc::ptr_eq(&current, &replacement));
    assert!(
        !replacement.stop.load(Ordering::Relaxed),
        "a stale removal must not stop a replacement lifetime"
    );
    assert!(!replaced.core.removal_pending("alice"));
}

#[test]
fn set_multibox_on_syncs_focus_wall() {
    let mut s = Session::new();
    s.set_multibox(true);
    assert_eq!(
        s.focus.lock().unwrap().wall,
        s.core.members(),
        "the seed path (running slots) mirrors into Focus.wall too"
    );
    s.set_multibox(false);
    assert_eq!(s.focus.lock().unwrap().wall, s.core.members());
}

#[test]
fn load_all_loads_vault_profiles_and_syncs_focus_wall() {
    let path = tmp_vault("load-all.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    vault_upsert(&mut s.core, profile("bob", "pw", 43));
    s.load("alice");
    let added = s.load_all();
    assert_eq!(added, 1, "only bob is new");
    assert_eq!(
        s.core.members(),
        vec!["alice".to_string(), "bob".to_string()]
    );
    assert_eq!(s.focus.lock().unwrap().wall, s.core.members());
    assert_eq!(
        s.focused_name().as_deref(),
        Some("alice"),
        "Load all must not steal focus from the already-focused member"
    );
}

fn publish_slot_login(s: &mut Session, name: &str, scene_state: i32) {
    let mut rows = s.core.play_for_test().unwrap().statuses.lock().unwrap();
    rows.retain(|row| row.username != name);
    rows.push(SlotStatus {
        username: name.into(),
        connected: true,
        ingame: true,
        scene_state,
        ..SlotStatus::default()
    });
    drop(rows);
    s.pump_status();
}

fn assert_alice_still_drawing(s: &Session, why: &str) {
    let (focused, draws) = s.focus_draw_state();
    assert_eq!(
        focused.as_deref(),
        Some("alice"),
        "{why}: focused={focused:?}"
    );
    assert_eq!(s.focused_name().as_deref(), Some("alice"), "{why}");
    let alice = draws.iter().find(|(name, _)| name == "alice");
    assert_eq!(
        alice,
        Some(&("alice".into(), true)),
        "{why}: draws={draws:?}"
    );
    for (name, draw) in &draws {
        if name != "alice" {
            assert!(
                !*draw,
                "{why}: {name} must stay skip-paint under only-render-selected; {draws:?}"
            );
        }
    }
}

#[test]
fn load_all_keeps_focused_bot_drawing_when_others_log_in() {
    let path = tmp_vault("load-all-keeps-focus.vault");
    let mut s = Session::new();
    s.persist_ui = false;
    s.core.set_spawn_workers(false);
    s.core.set_play(Some(empty_play()));
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    for (name, uid) in [("alice", 42), ("bob", 43), ("carol", 44), ("dave", 45)] {
        vault.put(profile(name, "pw", uid).auto_login(name != "alice"));
    }
    install_vault(&mut s.core, vault);
    s.set_multibox(true);
    s.load("alice");
    s.set_renderer(true);
    s.focus.lock().unwrap().only_render_selected = true;
    s.focus.lock().unwrap().game_pane_open = true;
    publish_slot_login(&mut s, "alice", 2);
    assert_alice_still_drawing(&s, "before Load all");

    let added = s.load_all();
    assert_eq!(added, 3, "bob, carol and dave are new");
    assert_eq!(
        s.core.members(),
        vec![
            "alice".to_string(),
            "bob".to_string(),
            "carol".to_string(),
            "dave".to_string()
        ]
    );
    assert_alice_still_drawing(&s, "immediately after Load all");

    s.login_all();
    for name in ["bob", "carol", "dave"] {
        publish_slot_login(&mut s, name, 1);
        assert_alice_still_drawing(&s, &format!("{name} at scene_state==1"));
        publish_slot_login(&mut s, name, 2);
        assert_alice_still_drawing(&s, &format!("{name} logged in"));
    }
    assert_alice_still_drawing(&s, "after B, C and D logged in");
}

#[test]
fn load_all_picks_focus_when_none_was_focused() {
    let path = tmp_vault("load-all-no-focus.vault");
    let mut s = Session::new();
    s.persist_ui = false;
    s.core.set_spawn_workers(false);
    s.core.set_play(Some(empty_play()));
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    vault.put(profile("bob", "pw", 43));
    vault.put(profile("carol", "pw", 44));
    install_vault(&mut s.core, vault);
    s.set_multibox(true);
    s.set_renderer(true);
    s.focus.lock().unwrap().only_render_selected = true;
    s.focus.lock().unwrap().game_pane_open = true;
    s.load_all();
    let picked = frontend_core::pick_focus(s.core.members(), None);
    assert_eq!(picked.as_deref(), Some("bob"));
    assert_eq!(s.focused_name(), picked);
    publish_slot_login(&mut s, "bob", 2);
    let (focused, draws) = s.focus_draw_state();
    assert_eq!(focused, picked);
    let bob = draws.iter().find(|(name, _)| name == "bob");
    assert_eq!(
        bob,
        Some(&("bob".into(), true)),
        "picked bot must draw; {draws:?}"
    );
    for (name, draw) in &draws {
        if name != "bob" {
            assert!(
                !*draw,
                "{name} must stay skip-paint under only-render-selected; {draws:?}"
            );
        }
    }
}

#[test]
fn load_all_keeps_focus_with_render_all_on() {
    let path = tmp_vault("load-all-render-all.vault");
    let mut s = Session::new();
    s.persist_ui = false;
    s.core.set_spawn_workers(false);
    s.core.set_play(Some(empty_play()));
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    for (name, uid) in [("alice", 42), ("bob", 43), ("carol", 44)] {
        vault.put(profile(name, "pw", uid).auto_login(name != "alice"));
    }
    install_vault(&mut s.core, vault);
    s.set_multibox(true);
    s.load("alice");
    s.set_renderer(true);
    s.focus.lock().unwrap().only_render_selected = false;
    s.focus.lock().unwrap().game_pane_open = true;
    publish_slot_login(&mut s, "alice", 2);

    let added = s.load_all();
    assert_eq!(added, 2, "bob and carol are new");
    let (focused, draws) = s.focus_draw_state();
    assert_eq!(focused.as_deref(), Some("alice"));
    assert_eq!(s.focused_name().as_deref(), Some("alice"));
    for (name, draw) in &draws {
        assert!(*draw, "{name} must draw with render-all; {draws:?}");
    }

    s.login_all();
    for name in ["bob", "carol"] {
        publish_slot_login(&mut s, name, 1);
        let (focused, draws) = s.focus_draw_state();
        assert_eq!(
            focused.as_deref(),
            Some("alice"),
            "{name} at scene_state==1 must not steal focus"
        );
        assert!(
            draws.iter().all(|(_, draw)| *draw),
            "{name} at scene 1: render-all still paints; {draws:?}"
        );
        publish_slot_login(&mut s, name, 2);
    }
    let (focused, draws) = s.focus_draw_state();
    assert_eq!(focused.as_deref(), Some("alice"));
    assert!(draws.iter().all(|(_, draw)| *draw), "{draws:?}");
}

#[test]
fn chooser_row_load_still_focuses_the_member() {
    let path = tmp_vault("chooser-load-focuses.vault");
    let mut s = Session::new();
    s.persist_ui = false;
    s.core.set_spawn_workers(false);
    s.core.set_play(Some(empty_play()));
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    vault.put(profile("alice", "pw", 42));
    vault.put(profile("bob", "pw", 43));
    install_vault(&mut s.core, vault);
    s.load("alice");
    s.load("bob");
    assert_eq!(
        s.focused_name().as_deref(),
        Some("bob"),
        "single-row Load is still the click that focuses that member"
    );
}

#[test]
#[ignore = "LIVE=1 Engine A: 5-bot Load all while one is focused"]
fn live_load_all_keeps_focus_on_engine_a() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_NAV_BUILD").as_deref(), Ok("skip"));
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").expect("BOT_LIVE_NAME_PREFIX");
    assert!(
        (1..=4).contains(&prefix.len()),
        "BOT_LIVE_NAME_PREFIX must be 1-4 characters"
    );
    let evidence = PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR"));
    std::fs::create_dir_all(&evidence).unwrap();
    let engine = PathBuf::from(std::env::var_os("WORLD_ENGINE_DIR").expect("WORLD_ENGINE_DIR"));
    let nav_pack = PathBuf::from(std::env::var_os("WORLD_NAV_PACK").expect("WORLD_NAV_PACK"));
    let cache = PathBuf::from(std::env::var_os("BOT_CACHE_DIR").expect("BOT_CACHE_DIR"));
    let unpack = PathBuf::from(
        std::env::var_os("BOT_NAV_SNAPSHOT_ROOT").unwrap_or_else(|| cache.clone().into()),
    );

    let names = host_play::mint_live_names(5);
    let entries = host_play::mint_live_entries(&names);
    let pass = host_play::live_vault_passphrase();
    let vault_path = super::temp_live_vault_from(&entries, 274_000_001, &pass, false);

    let mut s = Session::new();
    s.persist_ui = false;
    s.configure_profile(ProfileArgs::from_host(host_play::ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(44594),
        http_port: Some(1080),
        engine_dir: Some(engine),
        nav_pack: Some(nav_pack.clone()),
        nav_flags: Some(nav_pack.with_extension("navflags")),
        cache_dir: Some(cache),
        unpack_dir: Some(unpack),
        ..host_play::ProfileOptions::default()
    }))
    .expect("configure Engine A");
    s.bind_profile().expect("bind Engine A");
    assert!(s.unlock_at(&vault_path, &pass), "{:?}", s.error);
    s.set_multibox(true);
    s.wall.chooser_open = false;
    let focused = names[0].clone();
    s.load(&focused);
    s.select(&focused);
    s.set_renderer(true);
    s.focus.lock().unwrap().only_render_selected = true;
    s.focus.lock().unwrap().game_pane_open = true;
    s.login(&focused);

    let wait_scene =
        |s: &mut Session, name: &str, scene: i32, deadline: Instant, stride: Duration| {
            while Instant::now() < deadline {
                s.pump_status();
                if s.statuses()
                    .iter()
                    .any(|row| row.username == name && row.ingame && row.scene_state == scene)
                {
                    return true;
                }
                std::thread::sleep(stride);
            }
            false
        };
    let focused_gen = |s: &Session| s.focused_pixels().map(|p| p.generation()).unwrap_or(0);
    let wait_gen = |s: &mut Session, min: u64, deadline: Instant| -> u64 {
        let mut last = focused_gen(s);
        while Instant::now() < deadline {
            s.pump_status();
            last = focused_gen(s);
            if last > min {
                return last;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        last
    };
    let deadline = Instant::now() + Duration::from_secs(150);
    assert!(
        wait_scene(&mut s, &focused, 2, deadline, Duration::from_millis(50)),
        "focused bot never reached scene 2: {:?}",
        s.statuses()
    );
    let mut gen0 = 0;
    while Instant::now() < deadline {
        s.pump_status();
        gen0 = focused_gen(&s);
        if gen0 > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(gen0 > 0, "focused bot produced no frames before Load all");
    let before = s.focus_draw_state();
    eprintln!("focus-draw before Load all: {before:?} gen={gen0}");

    let added = s.load_all();
    assert_eq!(added, 4, "the other four vault profiles load");
    s.login_all();
    let after_load = s.focus_draw_state();
    let gen_after_load = focused_gen(&s);
    eprintln!("focus-draw immediately after Load all: {after_load:?} gen={gen_after_load}");
    assert_eq!(after_load.0.as_deref(), Some(focused.as_str()));
    assert_eq!(
        after_load.1.iter().find(|(n, _)| n == &focused),
        Some(&(focused.clone(), true))
    );

    let mut scene1_gens = Vec::new();
    for name in names.iter().skip(1) {
        let mut sampled_scene1 = false;
        while Instant::now() < deadline {
            s.pump_status();
            let snap = s
                .statuses()
                .iter()
                .find(|row| row.username == *name)
                .map(|row| (row.scene_state, row.ingame));
            if let Some((scene_state, ingame)) = snap {
                if scene_state == 1 && !sampled_scene1 {
                    let mid1 = s.focus_draw_state();
                    let gen1 = focused_gen(&s);
                    eprintln!("focus-draw while {name} at scene_state==1: {mid1:?} gen={gen1}");
                    assert_eq!(
                        mid1.0.as_deref(),
                        Some(focused.as_str()),
                        "{name} at scene 1 must not steal focus"
                    );
                    assert_eq!(
                        mid1.1.iter().find(|(n, _)| n == &focused),
                        Some(&(focused.clone(), true))
                    );
                    scene1_gens.push((name.clone(), gen1));
                    sampled_scene1 = true;
                }
                if ingame && scene_state == 2 {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            s.statuses()
                .iter()
                .any(|row| row.username == *name && row.ingame && row.scene_state == 2),
            "{name} never reached scene 2: {:?}",
            s.statuses()
        );
        let mid = s.focus_draw_state();
        let gen2 = focused_gen(&s);
        eprintln!("focus-draw while {name} logging in: {mid:?} gen={gen2}");
        assert_eq!(
            mid.0.as_deref(),
            Some(focused.as_str()),
            "{name} must not steal focus"
        );
        assert_eq!(
            mid.1.iter().find(|(n, _)| n == &focused),
            Some(&(focused.clone(), true))
        );
    }
    assert!(
        !scene1_gens.is_empty(),
        "Load all never sampled a member at scene_state==1: {:?}",
        s.statuses()
    );
    let gen_done = wait_gen(&mut s, gen0, deadline);
    let done = s.focus_draw_state();
    eprintln!("focus-draw after Load all logins: {done:?} gen={gen_done}");
    let receipt = evidence.join("focus-draw.json");
    std::fs::write(
        &receipt,
        serde_json::json!({
            "focused": done.0,
            "draws": done.1,
            "names": names,
            "generation_before": gen0,
            "generation_after_load": gen_after_load,
            "generation_done": gen_done,
            "scene1_generations": scene1_gens,
        })
        .to_string(),
    )
    .unwrap();
    assert!(
        gen_done > gen0,
        "focused slot must keep producing frames across Load all; wrote {}",
        receipt.display()
    );
    assert_eq!(done.0.as_deref(), Some(focused.as_str()));
    assert_eq!(
        done.1.iter().find(|(n, _)| n == &focused),
        Some(&(focused.clone(), true)),
        "focused bot must keep drawing; wrote {}",
        receipt.display()
    );
    for (name, draw) in &done.1 {
        if name != &focused {
            assert!(
                !*draw,
                "{name} must stay skip-paint under only-render-selected"
            );
        }
    }
}

#[test]
fn chooser_vault_remove_keeps_wall_member_and_slot() {
    let path = tmp_vault("chooser-remove.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    s.load("alice");
    let identity = s.core.profile_identity("alice").unwrap();
    s.fleet_model.set_mark_for_test(identity, true);
    assert!(s.vault_remove("alice"), "chooser ✕ deletes the vault row");
    assert!(
        !s.fleet_model.marks().contains(identity),
        "deleting a profile removes its shared mark"
    );
    assert!(
        s.core.profile("alice").is_none(),
        "profile row gone from the vault"
    );
    assert_eq!(
        s.core.members(),
        vec!["alice".to_string()],
        "chooser ✕ must not rail_remove a live member"
    );
    assert!(s.core.slots().contains_key("alice"), "slot stays up");
    assert!(
        s.focus.lock().unwrap().wall.contains(&"alice".to_string()),
        "Focus.wall still lists the member"
    );
}

#[test]
fn script_active_matches_rs2b0t() {
    assert!(script_active(ScriptRunState::Starting));
    assert!(script_active(ScriptRunState::Running));
    assert!(script_active(ScriptRunState::Paused));
    assert!(script_active(ScriptRunState::Stopping));
    assert!(!script_active(ScriptRunState::Idle));
    assert!(!script_active(ScriptRunState::Error));
}

#[test]
fn script_pause_resume_stop_enable_rules() {
    assert!(script_pause_enabled(ScriptRunState::Starting));
    assert!(script_pause_enabled(ScriptRunState::Running));
    assert!(script_pause_enabled(ScriptRunState::Paused));
    assert!(!script_pause_enabled(ScriptRunState::Idle));
    assert!(!script_pause_enabled(ScriptRunState::Stopping));
    assert!(!script_pause_enabled(ScriptRunState::Error));
    assert!(script_stop_enabled(ScriptRunState::Starting));
    assert!(script_stop_enabled(ScriptRunState::Running));
    assert!(script_stop_enabled(ScriptRunState::Paused));
    assert!(!script_stop_enabled(ScriptRunState::Stopping));
    assert!(!script_stop_enabled(ScriptRunState::Idle));
    assert!(!script_stop_enabled(ScriptRunState::Error));
}

#[cfg(feature = "js")]
#[test]
fn load_js_registers_card_selects_and_persists_to_the_session_store() {
    let dir = TestDir::new("session-load");
    let store = dir.join("js-scripts.json");
    let path = dir.join("tickbot.js");
    std::fs::write(
        &path,
        "export function tick(api) { api._n = (api._n||0)+1 }",
    )
    .unwrap();

    let mut s = Session::new();
    s.scripts
        .use_library_at(store.clone(), dir.join("js-cache"));
    s.load_js(&path);
    assert_eq!(s.error, None, "load should succeed: {:?}", s.error);
    match &s.script_sel {
        Some(choice) => {
            let (source, id) = choice.loaded_card().expect("loaded file card");
            assert_eq!(source, CardSource::File);
            assert!(id.ends_with("tickbot.js") || id.contains("tickbot"), "{id}");
        }
        None => panic!("no card selected: {:?}", s.script_sel),
    }
    assert_eq!(s.scripts.cards().len(), 1);
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
    s.set_focus_for_test("alice");
    s.script_start_selected();
    let err = s.error.clone().expect("no-selection banner");
    assert!(
        err.contains("assignment") || err.contains("browse"),
        "{err}"
    );
    s.error = None;
    s.select_script_selection(ScriptChoice::compiled(
        CompiledCardId::lookup("Gatherer").unwrap(),
    ));
    s.script_start_selected();
    let err = s.error.clone().expect("no-play banner");
    assert!(err.contains("play"), "{err}");
    assert_eq!(s.focused_script_state(), ScriptRunState::Idle);
}

#[cfg(feature = "js")]
#[test]
fn script_start_selected_refuses_unloadable_import() {
    let dir = TestDir::new("unloadable");
    let path = dir.join("ghost.js");
    std::fs::write(
            &path,
            "import x from '../../event/webwalk/Something.js';\nexport default class T extends LoopingBot { loop() {} }\n",
        )
        .unwrap();
    let mut s = Session::new();
    s.scripts
        .use_library_at(dir.join("js-scripts.json"), dir.join("js-cache"));
    let mut play = empty_play();
    play.attach_arm("alice", SlotArm::new(42, false));
    s.core.set_play(Some(play));
    s.core.fleet_mut().add("alice");
    s.set_focus_for_test("alice");
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
    assert_eq!(s.focused_script_state(), ScriptRunState::Idle);
}

/// M-269: after a bad-import Load the panel retains the full failure for
/// the Browse card (`browse_script_card` paints `failed {stage}` plus the
/// wrapped `named_line` on the selected card), even though the banner is
/// cleared. Headless: no headed window.
#[cfg(feature = "js")]
#[test]
fn bad_import_load_retains_the_full_failure_for_browse() {
    let dir = TestDir::new("unloadable-retained");
    let path = dir.join("ghost.js");
    std::fs::write(
            &path,
            "import x from '../../event/webwalk/Something.js';\nexport default class T extends LoopingBot { loop() {} }\n",
        )
        .unwrap();
    let mut s = Session::new();
    s.scripts
        .use_library_at(dir.join("js-scripts.json"), dir.join("js-cache"));
    let mut play = empty_play();
    play.attach_arm("alice", SlotArm::new(42, false));
    s.core.set_play(Some(play));
    s.set_focus_for_test("alice");
    s.load_js(&path);
    assert_eq!(s.error, None, "load still registers the card");
    let failures: Vec<_> = s.scripts.load_failures().collect();
    assert_eq!(failures.len(), 1, "one retained failure: {failures:?}");
    let failure = failures[0];
    assert_eq!(failure.stage(), LoadStage::ImportResolution);
    assert!(
        failure
            .diagnostic()
            .contains("../../event/webwalk/Something.js"),
        "diagnostic names the specifier: {}",
        failure.diagnostic()
    );
    assert!(
        failure
            .named_line()
            .contains("../../event/webwalk/Something.js"),
        "the wrapped Browse line keeps the specifier: {}",
        failure.named_line()
    );
}

#[cfg(feature = "js")]
#[test]
fn fill_rs2b0t_cards_once_happens_on_first_browse_not_session_new() {
    // `$RS2B0T` + HOME point at a fake catalog so the fill never
    // touches the operator's home. `Session::new()` must stay
    // catalog-free: the fill is first Load/Browse only, so panel unit
    // tests that merely construct a `Session` never parse a real
    // catalog or rewrite `~/.274bot/rs2b0t-path`.
    let _work_service_guard = crate::work_service_test_lock();
    let iso = IsolatedStores::enter("rs2b0t-fill");
    let root = write_looping_catalog(iso.dir(), &[("BoneBurier", "BoneBurier")]);
    iso.set_catalog_root(&root);
    let mut s = Session::new();
    assert!(
        s.scripts.card(CardSource::Catalog, "BoneBurier").is_none(),
        "Session::new must not parse $RS2B0T"
    );
    assert!(
        !iso.home().join(".274bot/rs2b0t-path").exists(),
        "Session::new must not rewrite the persisted root"
    );
    s.fill_rs2b0t_cards_once();
    assert!(
        s.scripts.catalog_work_pending(),
        "first browse queues catalog work"
    );
    assert!(
        s.scripts.card(CardSource::Catalog, "BoneBurier").is_none(),
        "catalog cards are not parsed on the browse frame"
    );
    settle_catalog_work(&mut s);
    let card = s
        .scripts
        .card(CardSource::Catalog, "BoneBurier")
        .expect("BoneBurier card filled on first Browse");
    assert_eq!(card.shape(), LoadShape::CompatClass);
    assert!(
        iso.home().join(".274bot/rs2b0t-path").is_file(),
        "the first successful parse persists the root path"
    );
    assert_eq!(s.scripts.cards().len(), 1, "once-only fill");
    s.fill_rs2b0t_cards_once();
    assert_eq!(
        s.scripts.cards().len(),
        1,
        "a second Browse does not re-parse"
    );
}

#[cfg(feature = "js")]
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

#[cfg(feature = "js")]
#[test]
fn load_then_browse_still_prompts_without_rs2b0t_root() {
    let iso = IsolatedStores::enter("load-browse");
    let mut s = Session::new();
    s.fill_rs2b0t_cards_once();
    s.on_script_browse_open();
    assert!(
        s.rs2b0t_catalog_open,
        "Load with no root must not skip the first Browse catalog prompt"
    );
    assert!(
        !iso.home().join(".274bot/rs2b0t-path").exists(),
        "Load with no root must not write rs2b0t-path"
    );
}

#[cfg(feature = "js")]
#[test]
fn defer_rs2b0t_catalog_leaves_no_path_and_zero_catalog_cards() {
    let iso = IsolatedStores::enter("defer");
    let mut s = Session::new();
    s.on_script_browse_open();
    assert!(s.rs2b0t_catalog_open, "first browse opens folder picker");
    s.defer_rs2b0t_catalog();
    assert!(
        frontend_core::scripts::paths::rs2b0t_import_deferred_at(
            &iso.home().join(".274bot/rs2b0t-import")
        ),
        "defer flag written"
    );
    assert!(
        !iso.home().join(".274bot/rs2b0t-path").exists(),
        "defer must not write rs2b0t-path"
    );
    assert!(
        s.scripts.cards().all(|c| c.source() != CardSource::Catalog),
        "zero Catalog cards after defer"
    );
}

#[cfg(feature = "js")]
#[test]
fn import_rs2b0t_catalog_persists_path_and_registers_cards() {
    let _work_service_guard = crate::work_service_test_lock();
    let iso = IsolatedStores::enter("import");
    let root = fake_rs2b0t_tree(iso.dir());
    let mut s = Session::new();
    s.import_rs2b0t_catalog(&root).expect("import");
    assert!(
        s.scripts.catalog_work_pending(),
        "catalog import is admitted as worker work"
    );
    assert!(
        !iso.home().join(".274bot/rs2b0t-path").exists(),
        "root is persisted only after preparation settles"
    );
    settle_catalog_work(&mut s);
    assert!(iso.home().join(".274bot/rs2b0t-path").is_file());
    let card = s
        .scripts
        .card(CardSource::Catalog, "BoneBurier")
        .expect("catalog card");
    assert_eq!(card.category(), "Prayer");
    assert_eq!(card.description(), "Buries bones");
    assert_eq!(card.tags(), vec!["bones"]);
}

#[cfg(feature = "js")]
#[test]
fn on_script_browse_open_with_rs2b0t_env_still_fills_catalog() {
    let _work_service_guard = crate::work_service_test_lock();
    let iso = IsolatedStores::enter("browse-env");
    let root = fake_rs2b0t_tree(iso.dir());
    iso.set_catalog_root(&root);
    let mut s = Session::new();
    s.on_script_browse_open();
    assert!(s.scripts.catalog_work_pending());
    settle_catalog_work(&mut s);
    assert!(
        s.scripts.card(CardSource::Catalog, "BoneBurier").is_some(),
        "RS2B0T env still fills catalog on first browse"
    );
    assert!(iso.home().join(".274bot/rs2b0t-path").is_file());
}

#[cfg(feature = "js")]
fn write_two_card_catalog(dir: &Path) -> PathBuf {
    write_looping_catalog(
        dir,
        &[("BoneBurier", "BoneBurier"), ("ShopRunner", "ShopRunner")],
    )
}

#[cfg(feature = "js")]
#[test]
fn select_script_card_queues_only_that_card_through_worker() {
    let _work_service_guard = crate::work_service_test_lock();
    let iso = IsolatedStores::enter("transpile-click");
    let root = write_two_card_catalog(iso.dir());
    let mut s = Session::new();
    s.persist_ui = false;
    s.scripts.use_library_at(
        iso.dir().join("js-scripts.json"),
        iso.dir().join("js-cache"),
    );
    s.scripts
        .register_catalog(&root, &iso.dir().join("rs2b0t-path"))
        .expect("catalog");

    s.select_script_card(CardSource::Catalog, "BoneBurier");
    assert_eq!(
        s.script_sel,
        Some(ScriptChoice::loaded(CardSource::Catalog, "BoneBurier"))
    );
    assert_eq!(
        s.transpile_front().map(|(_, name)| name),
        Some("BoneBurier")
    );
    assert!(
        s.scripts
            .card_snapshot(CardSource::Catalog, "BoneBurier")
            .unwrap()
            .js()
            .is_empty(),
        "click must not transpile on the same call"
    );
    assert!(
        s.scripts
            .card_snapshot(CardSource::Catalog, "ShopRunner")
            .unwrap()
            .js()
            .is_empty(),
        "a click must not warm the rest of the catalog"
    );

    settle_transpile_queue(&mut s);
    assert!(!s
        .scripts
        .card_snapshot(CardSource::Catalog, "BoneBurier")
        .unwrap()
        .js()
        .is_empty());
    assert!(s
        .scripts
        .card_snapshot(CardSource::Catalog, "ShopRunner")
        .unwrap()
        .js()
        .is_empty());
}

#[cfg(feature = "js")]
#[test]
fn queue_transpile_all_uses_the_shared_worker_slot() {
    let _work_service_guard = crate::work_service_test_lock();
    let iso = IsolatedStores::enter("transpile-all");
    let root = write_two_card_catalog(iso.dir());
    let mut s = Session::new();
    s.persist_ui = false;
    s.scripts.use_library_at(
        iso.dir().join("js-scripts.json"),
        iso.dir().join("js-cache"),
    );
    s.scripts
        .register_catalog(&root, &iso.dir().join("rs2b0t-path"))
        .expect("catalog");

    s.queue_transpile_all();
    assert!(s.transpile_front().is_some());
    assert_eq!(s.transpile_progress().1, 2);

    assert_eq!(
        s.scripts
            .cards()
            .filter(|card| card.is_transpiled())
            .count(),
        0,
        "the click frame does not compile any card"
    );

    settle_transpile_queue(&mut s);
    assert_eq!(
        s.scripts
            .cards()
            .filter(|card| card.is_transpiled())
            .count(),
        2
    );
}

#[cfg(feature = "js")]
#[test]
fn select_script_card_queues_cached_output_through_worker() {
    let _work_service_guard = crate::work_service_test_lock();
    let iso = IsolatedStores::enter("transpile-hit");
    let root = write_two_card_catalog(iso.dir());
    let store = iso.dir().join("js-scripts.json");
    let cache = iso.dir().join("js-cache");
    let mut warm =
        frontend_core::test_support::scripts::TestLibrary::with_cache(store.clone(), cache.clone());
    warm.register_catalog(&root, &iso.dir().join("rs2b0t-path"))
        .expect("catalog");
    warm.ensure(CardSource::Catalog, "BoneBurier").unwrap();
    drop(warm);

    let mut s = Session::new();
    s.persist_ui = false;
    s.scripts.use_library_at(store, cache);
    s.scripts
        .register_catalog(&root, &iso.dir().join("rs2b0t-path"))
        .expect("catalog");
    assert!(s
        .scripts
        .card_snapshot(CardSource::Catalog, "BoneBurier")
        .unwrap()
        .js()
        .is_empty());
    assert!(s.scripts.is_transpile_cached(
        s.scripts
            .card_snapshot(CardSource::Catalog, "BoneBurier")
            .unwrap()
            .origin()
    ));

    s.select_script_card(CardSource::Catalog, "BoneBurier");
    assert_eq!(
        s.transpile_front().map(|(_, name)| name),
        Some("BoneBurier")
    );
    assert!(s
        .scripts
        .card_snapshot(CardSource::Catalog, "BoneBurier")
        .unwrap()
        .js()
        .is_empty());
    settle_transpile_queue(&mut s);
    assert!(!s
        .scripts
        .card_snapshot(CardSource::Catalog, "BoneBurier")
        .unwrap()
        .js()
        .is_empty());
}

fn two_live_session(label: &str) -> (Session, IsolatedStores) {
    let iso = IsolatedStores::enter(label);
    let path = tmp_vault(label);
    let mut s = Session::new();
    s.core.set_spawn_workers(false);
    s.core.set_play(Some(empty_play()));
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    vault_upsert(&mut s.core, profile("bob", "pw", 43));
    hydrate_panel_preferences(&mut s);
    s.load("alice");
    s.select("alice");
    (s, iso)
}

#[test]
fn single_mode_select_offers_background_ack_until_persisted() {
    let (mut s, _iso) = two_live_session("bg-ack-select");
    hydrate_panel_preferences(&mut s);
    assert!(
        !s.background_ack_open,
        "one live slot is not a background fleet"
    );
    s.select("bob");
    assert_eq!(s.background_bot_count(), 1);
    assert!(s.background_ack_open);
    s.dismiss_background_ack();
    assert!(!s.background_ack_open);
    s.select("alice");
    assert!(
        s.background_ack_open,
        "dismiss without ack shows the notice again"
    );
    s.ack_background_bots();
    await_preference(&mut s, "background_bots_ack", serde_json::json!(true));
    assert!(!s.background_ack_open);
    assert!(frontend_core::resources::background_bots_acked(
        &s.preferences
    ));
    s.select("bob");
    assert!(!s.background_ack_open);
}

#[test]
fn panel_sees_ack_file_written_outside_session() {
    let (mut s, iso) = two_live_session("bg-ack-shared");
    // Another frontend (a second service on the same files) records the
    // acknowledgement; the session sees it once its preferences re-read.
    thread_preferences()
        .set("background_bots_ack", serde_json::Value::Bool(true))
        .unwrap();
    s.preferences.reload().unwrap();
    s.select("alice");
    s.select("bob");
    assert!(!s.background_ack_open);
    let _iso = iso;
}

#[test]
fn background_ack_survives_session_restart() {
    let (mut s, iso) = two_live_session("bg-ack-restart");
    s.select("alice");
    s.select("bob");
    assert!(s.background_ack_open);
    s.ack_background_bots();
    await_preference(&mut s, "background_bots_ack", serde_json::json!(true));
    drop(s);
    let mut s2 = Session::new();
    s2.core.set_spawn_workers(false);
    s2.core.set_play(Some(empty_play()));
    install_vault(
        &mut s2.core,
        VaultFixture::create(&tmp_vault("bg-ack-restart-2"), "test-passphrase-01"),
    );
    vault_upsert(&mut s2.core, profile("bob", "pw", 43));
    hydrate_panel_preferences(&mut s2);
    s2.select("alice");
    s2.select("bob");
    assert!(
        !s2.background_ack_open,
        "reconstructed session must see the ack file"
    );
    let _iso = iso;
}

#[test]
fn multibox_off_offers_background_ack_when_others_keep_running() {
    let (mut s, _iso) = two_live_session("bg-ack-off");
    s.set_multibox(true);
    s.load("alice");
    s.load("bob");
    assert!(!s.background_ack_open);
    s.set_multibox(false);
    assert!(!s.multibox);
    assert!(s.background_ack_open);
}

#[test]
fn multibox_on_select_does_not_offer_background_ack() {
    let (mut s, _iso) = two_live_session("bg-ack-mb");
    s.set_multibox(true);
    s.load("alice");
    s.load("bob");
    s.select("alice");
    assert!(!s.background_ack_open);
}

#[test]
fn live_persist_off_skips_background_ack() {
    let (mut s, _iso) = two_live_session("bg-ack-live");
    s.persist_ui = false;
    s.select("alice");
    s.select("bob");
    assert!(!s.background_ack_open);
    s.ack_background_bots();
    assert!(!frontend_core::resources::background_bots_acked(
        &s.preferences
    ));
}

#[test]
fn failed_ack_persist_then_success_clears_only_that_error() {
    let (mut s, iso) = two_live_session("bg-ack-fail");
    hydrate_panel_preferences(&mut s);
    s.select("bob");
    assert!(s.background_ack_open);
    await_preference(&mut s, "last_focus", serde_json::json!("bob"));
    // An unreadable preferences file refuses the store on every platform. (A
    // folder replaced by a file does not: a read through it fails as
    // `NotADirectory` on Unix but as `NotFound` on Windows, which the store
    // reads as "no file yet", and the write then fails later, not at once.)
    let path = s.preferences.path();
    crate::test_support::write_private_fixture(&path, b"{malformed").unwrap();
    assert!(s.preferences.reload().is_err());
    s.error = None;
    s.ack_background_bots();
    assert!(
        s.background_ack_open,
        "persist failure must keep the acknowledgement visible"
    );
    assert!(
        s.core
            .setting_failure(frontend_core::Setting::Notice(
                frontend_core::OperatorNotice::BackgroundBots
            ))
            .is_some_and(
                |text| text.starts_with("Not saved: ") && text.contains("background bots:")
            ),
        "the failure is shown at the notice's setting: {:?}",
        s.core.setting_failure(frontend_core::Setting::Notice(
            frontend_core::OperatorNotice::BackgroundBots
        ))
    );
    assert!(!frontend_core::resources::background_bots_acked(
        &s.preferences
    ));
    std::fs::remove_file(&path).unwrap();
    s.preferences.reload().unwrap();
    s.ack_background_bots();
    await_preference(&mut s, "background_bots_ack", serde_json::json!(true));
    assert!(!s.background_ack_open);
    assert!(s.error.is_none(), "got {:?}", s.error);
    assert!(frontend_core::resources::background_bots_acked(
        &s.preferences
    ));
    let _iso = iso;
}

#[test]
fn successful_ack_preserves_unrelated_error() {
    let (mut s, _iso) = two_live_session("bg-ack-unrelated");
    hydrate_panel_preferences(&mut s);
    s.select("bob");
    s.error = Some("chooser: vault locked".into());
    s.ack_background_bots();
    await_preference(&mut s, "background_bots_ack", serde_json::json!(true));
    assert!(!s.background_ack_open);
    assert_eq!(s.error.as_deref(), Some("chooser: vault locked"));
    assert!(frontend_core::resources::background_bots_acked(
        &s.preferences
    ));
}

#[test]
fn background_ack_closes_when_the_last_other_bot_retires() {
    let (mut s, _iso) = two_live_session("bg-ack-retire");
    s.select("alice");
    s.select("bob");
    assert!(s.background_ack_open);
    assert_eq!(s.background_bot_count(), 1);
    s.select("alice");
    let mut play = empty_play();
    play.attach_arm("alice", host_play::SlotArm::new(1, false));
    s.core.set_play(Some(play));
    s.pump_status();
    assert_eq!(s.background_bot_count(), 0);
    assert!(
        !s.background_ack_open,
        "zero live others must dismiss the acknowledgement"
    );
}

#[test]
fn skip_lock_session_does_not_create_instance_lock() {
    let iso = IsolatedStores::enter("session-skip-lock");
    let _s = Session::with_instance(InstancePermit::skip());
    assert!(!host_play::instance_lock_path().exists());
    let _iso = iso;
}

#[test]
fn locked_session_releases_the_instance_lock_on_drop() {
    let iso = IsolatedStores::enter("session-hold-lock");
    let permit = match frontend_core::launch::acquire_instance(InstanceKind::Panel, false) {
        Ok(Ok(permit)) => permit,
        Ok(Err(_)) => panic!("expected acquire"),
        Err(error) => panic!("expected acquire: {error}"),
    };
    let session = Session::with_instance(permit);
    drop(session);
    match host_play::try_acquire_instance_lock(host_play::InstanceKind::Tui).unwrap() {
        host_play::InstanceLockResult::Acquired(_) => {}
        host_play::InstanceLockResult::Contested(h) => panic!("session drop left the lock: {h:?}"),
    }
    let _iso = iso;
}

#[test]
fn background_count_ignores_terminal_status_rows() {
    let iso = IsolatedStores::enter("bg-ack-terminal");
    let mut s = Session::new();
    s.core.set_spawn_workers(false);
    let mut play = empty_play();
    play.attach_arm("alice", host_play::SlotArm::new(1, false));
    play.statuses.lock().unwrap().extend([
        SlotStatus {
            username: "alice".into(),
            ingame: true,
            connected: true,
            ..SlotStatus::default()
        },
        SlotStatus {
            username: "bob".into(),
            ingame: false,
            worker_terminal: Some(host_play::WorkerTerminal::Failed),
            ..SlotStatus::default()
        },
    ]);
    s.core.set_play(Some(play));
    s.set_focus_for_test("alice");
    assert_eq!(s.background_bot_count(), 0);
    let _iso = iso;
}

#[test]
fn a_profile_still_saving_cannot_be_selected_loaded_or_deleted_from_the_chooser() {
    let path = tmp_vault("saving-gate.vault");
    let mut s = Session::new();
    hydrate_panel_preferences(&mut s);
    s.core.set_spawn_workers(false);
    s.core.set_play(Some(empty_play()));
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    vault.put(profile("alice", "pw", 42));
    install_vault(&mut s.core, vault);
    s.set_multibox(true);
    let gate = s.core.write_gate();
    let held = gate.lock().unwrap();
    assert!(s
        .submit(
            frontend_core::OperatorCommand::OpenNewProfileForm,
            frontend_core::Targets::Application,
        )
        .is_some());
    *s.profile_editor.username_mut() = "bob".into();
    assert!(s
        .submit(
            frontend_core::OperatorCommand::EditProfileDraft(
                frontend_core::command::ProfileDraftField::Username,
            ),
            frontend_core::Targets::Application,
        )
        .is_some());
    *s.profile_editor.password_mut() = "pw".into();
    assert!(s
        .submit(
            frontend_core::OperatorCommand::EditProfileDraft(
                frontend_core::command::ProfileDraftField::Password,
            ),
            frontend_core::Targets::Application,
        )
        .is_some());
    assert!(s.save_credentials());
    // The writer may already hold bob's job, but it cannot commit it yet.

    s.select("bob");
    assert_ne!(s.focused_name().as_deref(), Some("bob"));
    assert!(!s.load("bob"), "chooser row Load is refused");
    s.load_all();
    assert_ne!(
        s.focus.lock().unwrap().focused.as_deref(),
        Some("bob"),
        "Load all never focuses a profile still saving"
    );
    let report = s.core.last_operation().unwrap();
    assert!(
        matches!(
            report.outcome("bob"),
            Some(o) if o.is_skipped()
        ),
        "{report:?}"
    );
    assert_eq!(
        report.outcome("alice"),
        Some(&frontend_core::Outcome::Completed)
    );
    assert!(!s.vault_remove("bob"), "chooser Delete is refused");
    assert!(
        s.core.play_for_test().unwrap().arm("bob").is_none(),
        "no slot from staged credentials"
    );
    assert!(!s.core.members().iter().any(|m| m == "bob"));

    drop(held);
    s.core.flush_writes();
    s.pump_status();
    assert!(!s.core.profile_saving("bob"));
    assert_eq!(
        s.focused_name().as_deref(),
        Some("bob"),
        "selected once durable"
    );
    assert!(s.core.play_for_test().unwrap().arm("bob").is_some());
}

#[test]
fn memory_toggle_reports_login_divergence_through_the_shared_command() {
    let path = tmp_vault("mem-toggle.vault");
    let mut s = Session::new();
    s.core.set_spawn_workers(false);
    s.core.set_play(Some(empty_play()));
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    let alice = profile("alice", "pw", 42).lowmem(true);
    vault.put(alice);
    install_vault(&mut s.core, vault);
    assert!(s.load("alice"));
    s.select("alice");
    s.core
        .play_for_test()
        .unwrap()
        .statuses
        .lock()
        .unwrap()
        .push(SlotStatus {
            username: "alice".into(),
            connected: true,
            ingame: true,
            scene_state: 2,
            login_lowmem: Some(true),
            ..SlotStatus::default()
        });
    s.core.poll();

    let clean = s
        .focused_memory_notice()
        .expect("a spawn records its login mode");
    assert!(!clean.differs());

    assert!(s.set_focused_lowmem(false));
    settle_panel_writes(&mut s);
    assert_eq!(
        s.audio.owner(),
        None,
        "queuing highmem cannot open the speaker"
    );
    let notice = s.focused_memory_notice().expect("recorded login");
    assert!(
        notice.differs(),
        "the entire client still follows the login mode"
    );
    assert_eq!(
        s.core
            .play_for_test()
            .unwrap()
            .arm("alice")
            .unwrap()
            .lowmem_handshake(),
        Some(false),
        "the shared command arms the next handshake"
    );
    s.core.flush_writes();
    assert!(
        !s.core.profile("alice").unwrap().lowmem(),
        "the setting is durable"
    );
}

#[test]
fn memory_relog_starts_at_once_without_a_running_script() {
    let path = tmp_vault("mem-relog.vault");
    let mut s = Session::new();
    s.core.set_spawn_workers(false);
    s.core.set_play(Some(empty_play()));
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    vault.put(profile("alice", "pw", 42));
    install_vault(&mut s.core, vault);
    assert!(s.load("alice"));
    s.core
        .play_for_test()
        .unwrap()
        .statuses
        .lock()
        .unwrap()
        .push(SlotStatus {
            username: "alice".into(),
            connected: true,
            ingame: true,
            scene_state: 2,
            login_lowmem: Some(true),
            ..SlotStatus::default()
        });
    s.core.poll();
    s.select("alice");

    assert!(!s.focused_memory_relog_warning(), "no script runs");
    assert!(!s.mem_relog_armed());
    assert!(s.request_focused_memory_relog());
    let arm = s.core.play_for_test().unwrap().arm("alice").unwrap();
    assert!(!arm.login_latched() && !arm.wants_logout() && arm.wants_login());
    assert!(
        s.core.memory_status("alice").unwrap().relog_pending,
        "the poll must see the armed relog"
    );
}

#[test]
fn memory_relog_arm_is_per_bot() {
    let path = tmp_vault("mem-arm.vault");
    let mut s = Session::new();
    s.core.set_spawn_workers(false);
    s.core.set_play(Some(empty_play()));
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    vault.put(profile("alice", "pw", 42));
    vault.put(profile("bob", "pw", 43));
    install_vault(&mut s.core, vault);
    assert!(s.load("alice"));
    assert!(s.load("bob"));
    s.select("alice");
    s.mem_relog_armed = Some("alice".into());
    assert!(s.mem_relog_armed());
    // An armed execute clears the arm.
    assert!(s.request_focused_memory_relog());
    assert!(!s.mem_relog_armed());
    s.select("bob");
    assert!(!s.mem_relog_armed(), "arming never leaks across bots");
}

#[test]
fn marked_debug_command_queues_ready_and_reports_ineligible_rows() {
    let mut play = empty_play();
    profile("alice", "pw", 11).spawn_on(&mut play, Some(SlotArm::new(11, false)));
    profile("bob", "pw", 22).spawn_on(&mut play, Some(SlotArm::new(22, false)));
    {
        let mut statuses = play.statuses.lock().unwrap();
        statuses
            .iter_mut()
            .find(|status| status.username == "alice")
            .unwrap()
            .ingame = true;
        statuses
            .iter_mut()
            .find(|status| status.username == "alice")
            .unwrap()
            .scene_state = 2;
        statuses
            .iter_mut()
            .find(|status| status.username == "bob")
            .unwrap()
            .ingame = false;
    }

    let path = tmp_vault("marked-debug-command.vault");
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    vault.put(profile("alice", "pw", 11));
    vault.put(profile("bob", "pw", 22));
    vault.put(profile("missing", "pw", 33));

    let mut session = Session::new();
    session.core.set_spawn_workers(false);
    install_vault(&mut session.core, vault);
    session.core.set_play(Some(play));
    session.fleet_model.mark_all_for_test([
        frontend_core::ProfileIdentity::uid(11),
        frontend_core::ProfileIdentity::uid(22),
        frontend_core::ProfileIdentity::uid(33),
    ]);

    // A marked row can disappear after the target snapshot; the shared
    // adapter still accounts for its identity instead of dropping the mark.
    let snapshot = session
        .debug_target_snapshot()
        .into_iter()
        .filter(|(_, name)| name != "missing")
        .collect();
    let report = session.send_debug_command_marked_snapshot("getcoord", snapshot);
    assert_eq!(report.accepted(), 1);
    assert_eq!(report.total(), 3);
    assert_eq!(
        report
            .skipped()
            .iter()
            .map(|skip| (skip.profile(), skip.reason()))
            .collect::<Vec<_>>(),
        vec![
            ("bob", "bot is not in game"),
            ("profile#33", "profile unavailable"),
        ]
    );
}

#[test]
fn panel_frame_hook_receives_client_through_slot_frame() {
    let session = Session::new();
    let hook = session.panel_frame_hook();
    let mut client = paint_client();
    client.ingame = true;
    client.scene_state = 2;

    frontend_core::frames::run_frame_for_test(
        &mut client,
        "alice",
        hook,
        host_play::SlotFrameInput::default(),
        (
            host_play::player_here_tile,
            host_play::Host::publish_frontend_snapshot,
            host_play::step_walk_arm_follow,
            |client: &_| client.gens.player,
        ),
    );

    assert!(session.mainland_sent.lock().unwrap().contains("alice"));
}

/// The banner message stays until the shared timeout passes, or a newer
/// message replaces it and restarts the timer: the same rule as the terminal
/// app's message line (`frontend_core::ERROR_MESSAGE_TIMEOUT`). An action
/// that resolves a message clears it earlier; that is proved by
/// `a_refusal_before_submission_logs_once_and_the_resolving_action_clears_it`.
#[test]
fn the_banner_message_expires_unless_replaced() {
    use frontend_core::ERROR_MESSAGE_TIMEOUT as TIMEOUT;
    let second = std::time::Duration::from_secs(1);
    let mut s = Session::new();
    let t0 = Instant::now();

    // No action: it stays for the whole timeout, then goes.
    s.error = Some("first".into());
    s.expire_error(t0);
    s.expire_error(t0 + TIMEOUT - second);
    assert_eq!(s.error.as_deref(), Some("first"));
    s.expire_error(t0 + TIMEOUT);
    assert_eq!(s.error, None);

    // A newer message replaces it and restarts the timer.
    s.error = Some("older".into());
    s.expire_error(t0);
    s.error = Some("newer".into());
    s.expire_error(t0 + TIMEOUT - second);
    s.expire_error(t0 + TIMEOUT + second);
    assert_eq!(s.error.as_deref(), Some("newer"), "the timer restarted");
    s.expire_error(t0 + TIMEOUT * 2 - second);
    assert_eq!(s.error, None);
}

/// The lines the process log holds for refusals that name `bot`.
fn refusal_lines_naming(bot: &str) -> Vec<String> {
    use frontend_core::log::{global, LogScope, LogView};
    let mut view = LogView::new(LogScope::Process);
    view.edit_filter(|filter| filter.set_text("command refused"));
    global().refresh(&mut view);
    view.rows()
        .iter()
        .map(|row| row.message().to_owned())
        .filter(|line| line.contains(bot))
        .collect()
}

/// PANEL-STATE-GAP-1023: a refusal the panel makes before it can submit
/// (Copy settings with no focus, or on a bot with no card) logs one `info`
/// line at the shared command boundary, with the command, the bots and the
/// reason; and the action that resolves it (focusing a bot, here) clears the
/// banner at once, not only when the timeout passes.
#[test]
fn a_refusal_before_submission_logs_once_and_the_resolving_action_clears_it() {
    let _iso = IsolatedStores::enter("psg-resolve-refusal");
    let mut s = Session::new();
    s.core.set_spawn_workers(false);
    s.core.set_play(Some(empty_play()));
    install_vault(
        &mut s.core,
        VaultFixture::create(&tmp_vault("psg-resolve-refusal"), "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("psg-f3-zed", "pw", 44));
    s.persist_ui = false;
    assert_eq!(s.focused_name(), None);
    let before = s.core.refusals();

    s.copy_focused_settings_to_same_card();
    assert_eq!(s.error.as_deref(), Some("Copy settings: focus a bot first"));
    assert_eq!(s.core.refusals(), before + 1, "one refusal counted");
    s.select("psg-f3-zed");
    assert_eq!(s.focused_name().as_deref(), Some("psg-f3-zed"));
    assert_eq!(s.error, None, "focusing a bot resolved the message");

    // A bot with no card: the line names that bot.
    s.copy_focused_settings_to_same_card();
    assert_eq!(
        s.error.as_deref(),
        Some("Copy settings: the focused bot has no card")
    );
    assert_eq!(s.core.refusals(), before + 2);
    let lines = refusal_lines_naming("psg-f3-zed");
    assert_eq!(lines.len(), 1, "{lines:?}");
    for part in [
        "PlanFleetAction(CopySettings)",
        "on psg-f3-zed",
        "Copy settings: the focused bot has no card",
    ] {
        assert!(
            lines[0].contains(part),
            "{part:?} missing from {:?}",
            lines[0]
        );
    }
}

#[cfg(feature = "js")]
/// Whether the process log holds a refusal line containing `line`. The log
/// redacts every registered password (the short ones other tests register
/// too, such as "test"), wherever it appears: so a line that holds a path is
/// compared after the same redaction, whatever the temporary folder is called.
fn refusal_logged(line: &str) -> bool {
    use frontend_core::log::{global, LogScope, LogView};
    let mut view = LogView::new(LogScope::Process);
    view.edit_filter(|filter| filter.set_text("command refused: "));
    global().refresh(&mut view);
    let wanted = api::hostlog::redact(line);
    view.rows().iter().any(|row| {
        row.message().starts_with("command refused: ")
            && api::hostlog::redact(row.message()).contains(&*wanted)
    })
}

/// PANEL-STATE-GAP-1023 (F2): a command the panel refuses before it can
/// submit it is refused at the shared command boundary, as every other
/// refusal is: one `info` line names the command, the bots and the reason,
/// and the banner shows the reason. The Refresh catalog button with no
/// catalog configured, Load on a path that is not a file, and Use this
/// folder on a folder with no catalog.
#[cfg(feature = "js")]
#[test]
fn the_panels_own_refusals_log_at_the_command_boundary() {
    let iso = IsolatedStores::enter("psg-panel-own-refusals");
    let mut s = Session::new();
    s.persist_ui = false;
    assert_eq!(s.catalog_root().unwrap(), None);
    let before = s.core.refusals();

    s.begin_refresh_catalog();
    assert_eq!(
        s.error.as_deref(),
        Some("Refresh catalog: no catalog configured")
    );
    assert_eq!(s.core.refusals(), before + 1, "one refusal counted");
    assert!(refusal_logged(
        "command refused: ImportCatalog on no bots: Refresh catalog: no catalog configured"
    ));

    let missing = iso.dir().join("psg-missing.ts");
    s.load_js(&missing);
    assert_eq!(s.core.refusals(), before + 2);
    assert!(refusal_logged(&format!(
        "command refused: LoadScriptFile on no bots: script: not a file: {}",
        missing.display()
    )));

    let empty = iso.dir().join("psg-no-catalog");
    std::fs::create_dir_all(&empty).unwrap();
    let refused = s.import_rs2b0t_catalog(&empty).unwrap_err();
    assert!(refused.starts_with("no catalog at "), "{refused}");
    assert_eq!(s.core.refusals(), before + 3);
    assert!(refusal_logged(&format!(
        "command refused: ImportCatalog on no bots: {refused}"
    )));
}

/// PANEL-STATE-GAP-1023 (R3-M1, panel): the refusal of a profile whose
/// first save is still being written waits for that save. It stays while
/// the write is held and clears on the first frame after the save is
/// durable, long before its timeout.
#[test]
fn a_still_saving_refusal_clears_when_the_save_finishes() {
    let path = tmp_vault("saving-clears.vault");
    let mut s = Session::new();
    s.core.set_spawn_workers(false);
    s.core.set_play(Some(empty_play()));
    let mut vault = VaultFixture::create(&path, "test-passphrase-01");
    vault.put(profile("alice", "pw", 42));
    install_vault(&mut s.core, vault);
    let gate = s.core.write_gate();
    let held = gate.lock().unwrap();
    assert!(s
        .submit(
            frontend_core::OperatorCommand::OpenNewProfileForm,
            frontend_core::Targets::Application,
        )
        .is_some());
    *s.profile_editor.username_mut() = "bob".into();
    assert!(s
        .submit(
            frontend_core::OperatorCommand::EditProfileDraft(
                frontend_core::command::ProfileDraftField::Username,
            ),
            frontend_core::Targets::Application,
        )
        .is_some());
    *s.profile_editor.password_mut() = "pw".into();
    assert!(s
        .submit(
            frontend_core::OperatorCommand::EditProfileDraft(
                frontend_core::command::ProfileDraftField::Password,
            ),
            frontend_core::Targets::Application,
        )
        .is_some());
    assert!(s.save_credentials());

    let refused = "bob: profile is still saving";
    let before = s.core.refusals();
    assert!(!s.load("bob"), "Load is refused while bob saves");
    assert_eq!(s.core.refusals(), before + 1);
    assert_eq!(s.error.as_deref(), Some(refused));
    s.pump_status();
    assert_eq!(s.error.as_deref(), Some(refused), "the save is still held");

    drop(held);
    s.core.flush_writes();
    s.pump_status();
    assert!(!s.core.profile_saving("bob"));
    assert_eq!(s.error, None, "the finished save resolved the message");
}

#[test]
fn panel_setting_events_follow_pending_failed_retry_clear_and_saved_owner_outcomes() {
    let _iso = IsolatedStores::enter("panel-setting-event-owner-outcomes");
    let mut session = Session::new();
    hydrate_panel_preferences(&mut session);
    session.watch_setting_events();
    let path = ["chrome", "accent"];
    let disk = PanelDisk::install(&session);

    session
        .persist_panel_member(&path, Some(serde_json::json!("#AABBCC")))
        .expect("the first write is accepted");
    let (events, incomplete) = session.take_panel_setting_events();
    assert!(!incomplete);
    assert_eq!(
        events.iter().map(|event| event.state()).collect::<Vec<_>>(),
        [frontend_core::SettingEventState::Pending]
    );
    assert_eq!(events[0].setting(), "chrome.accent");

    let failing = disk.release();
    settle_panel_writes(&mut session);
    let (events, incomplete) = session.take_panel_setting_events();
    assert!(!incomplete);
    assert_eq!(
        events.iter().map(|event| event.state()).collect::<Vec<_>>(),
        [frontend_core::SettingEventState::Failed]
    );
    assert_eq!(events[0].setting(), "chrome.accent");
    assert!(
        events[0]
            .error()
            .is_some_and(|error| error.contains("store I/O error")),
        "{events:?}"
    );
    assert!(session.panel_member_error(&["chrome", "accent"]).is_some());

    failing.store(false, Ordering::SeqCst);
    session
        .persist_panel_member(&path, Some(serde_json::json!("#DDEEFF")))
        .expect("the retry is accepted");
    let (events, incomplete) = session.take_panel_setting_events();
    assert!(!incomplete);
    assert_eq!(
        events.iter().map(|event| event.state()).collect::<Vec<_>>(),
        [
            frontend_core::SettingEventState::Cleared,
            frontend_core::SettingEventState::Pending,
        ]
    );
    assert_eq!(events[0].setting(), "chrome.accent");
    assert_eq!(events[1].setting(), "chrome.accent");
    assert!(session.panel_member_error(&["chrome", "accent"]).is_none());

    settle_panel_writes(&mut session);
    let (events, incomplete) = session.take_panel_setting_events();
    assert!(!incomplete);
    assert_eq!(
        events.iter().map(|event| event.state()).collect::<Vec<_>>(),
        [frontend_core::SettingEventState::Saved]
    );
    assert_eq!(events[0].setting(), "chrome.accent");
    assert_eq!(
        session.preferences.document().unwrap()["chrome"]["accent"],
        "#DDEEFF"
    );
    struct RejectSerialization;
    impl serde::Serialize for RejectSerialization {
        fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(<S::Error as serde::ser::Error>::custom(
                "serialization refused",
            ))
        }
    }
    assert_eq!(
        session
            .persist_panel_serializable(&path, &RejectSerialization)
            .unwrap_err(),
        "serialization refused"
    );
    let (events, incomplete) = session.take_panel_setting_events();
    assert!(!incomplete);
    assert_eq!(
        events.iter().map(|event| event.state()).collect::<Vec<_>>(),
        [frontend_core::SettingEventState::Failed]
    );
    assert_eq!(events[0].setting(), "chrome.accent");
    assert_eq!(events[0].error(), Some("serialization refused"));
}

#[test]
fn panel_setting_event_overflow_marks_the_drained_batch_incomplete() {
    let _iso = IsolatedStores::enter("panel-setting-event-overflow");
    let mut session = Session::new();
    session.watch_setting_events();
    let member = PanelPreferenceMember {
        request: 1,
        path: vec!["chrome".into(), "accent".into()],
        value: Some(serde_json::json!("#AABBCC")),
        owner: FeedbackOwner::Panel,
    };
    for _ in 0..PANEL_SETTING_EVENTS_CAP {
        session.note_panel_setting(&member, frontend_core::SettingEventState::Pending, None);
    }
    session.note_panel_setting(
        &member,
        frontend_core::SettingEventState::Failed,
        Some("preferences is busy"),
    );

    let (events, incomplete) = session.take_panel_setting_events();
    assert_eq!(events.len(), PANEL_SETTING_EVENTS_CAP);
    assert!(
        incomplete,
        "the dropped terminal failure makes the batch unsuitable as complete evidence"
    );
    assert_eq!(
        session.take_panel_setting_events(),
        (Vec::new(), false),
        "draining clears the queue and its overflow flag"
    );
}

// --- T5.3-S3: saved settings follow the durable snapshot, and a failed save
// shows at its setting ---
use frontend_core::MapBakeChoice;

/// A disk for one panel session's service: every file write waits at its
/// start until [`PanelDisk::release`], and fails while `failing` is set.
struct PanelDisk {
    release: std::sync::mpsc::Sender<()>,
    failing: Arc<std::sync::atomic::AtomicBool>,
}

impl PanelDisk {
    fn install(session: &Session) -> Self {
        let service = session
            .core
            .local_state_for_test()
            .expect("the panel owns its process state service");
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        service.set_write_stall_for_test(Arc::new(move || {
            let _ = gate.lock().unwrap().recv();
        }));
        let failing = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let refusing = Arc::clone(&failing);
        service.set_write_fault_for_test(Arc::new(move || refusing.load(Ordering::SeqCst)));
        Self { release, failing }
    }

    /// Let every held write (and every later one) run.
    fn release(self) -> Arc<std::sync::atomic::AtomicBool> {
        drop(self.release);
        self.failing
    }
}

/// Pump the session until no core or panel write is pending.
fn settle_panel_writes(s: &mut Session) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        s.pump_status();
        s.poll_panel_preferences();
        if !s.preferences_pending() {
            return;
        }
        assert!(Instant::now() < deadline, "the writes never settled");
        thread::sleep(Duration::from_millis(2));
    }
}

/// Every saved operational choice of the panel while its write is held, then
/// failed: the control shows the choice while the write is pending, the
/// applied state (draw mirror, bake gate, Quester source) never moves, and
/// when the write fails the control shows the saved value again with the
/// failure at that setting.
#[test]
fn settings_feedback_held_then_failed_operational_choices_show_the_saved_value_and_the_failure() {
    use frontend_core::command::DrawSwitch;
    use frontend_core::{OperatorNotice, Setting, SettingStatus};
    let _iso = IsolatedStores::enter("panel-s3-held-failed");
    let mut s = Session::new();
    hydrate_panel_preferences(&mut s);
    let disk = PanelDisk::install(&s);
    let capture = s.draw_shown(DrawSwitch::CaptureInput);
    let sidecar = s.draw_shown(DrawSwitch::SidecarFullRate);
    let focused = s.draw_shown(DrawSwitch::FocusedFullRate);
    let quester = s.quester_enabled_shown();
    let logging = s.session_log_shown();
    let bake = s.map_bake_shown();
    let source = s.quester_paths.settings().folder().to_path_buf();

    s.set_capture(!capture);
    s.set_sidecar_50(!sidecar);
    s.set_focused_50(!focused);
    s.persist_quester_paths_change(!quester, false).unwrap();
    s.set_session_log(!logging);
    s.set_map_bake_choice(MapBakeChoice::Always);
    let settings = [
        Setting::Draw(DrawSwitch::CaptureInput),
        Setting::Draw(DrawSwitch::SidecarFullRate),
        Setting::Draw(DrawSwitch::FocusedFullRate),
        Setting::QuesterPaths,
        Setting::SessionLog,
        Setting::MapBake,
    ];
    for _ in 0..3 {
        s.pump_status();
        s.poll_panel_preferences();
        for setting in settings {
            assert!(
                matches!(s.core.setting_status(setting), SettingStatus::Pending),
                "{setting:?} says it is saving"
            );
        }
        assert_eq!(s.draw_shown(DrawSwitch::CaptureInput), !capture);
        assert_eq!(s.draw_shown(DrawSwitch::SidecarFullRate), !sidecar);
        assert_eq!(s.draw_shown(DrawSwitch::FocusedFullRate), !focused);
        assert_eq!(s.quester_enabled_shown(), !quester);
        assert_eq!(s.session_log_shown(), !logging);
        assert_eq!(s.map_bake_shown(), MapBakeChoice::Always);
        // Nothing is applied: the render mirror, the bake gate and the
        // source are what they were.
        let focus = s.focus.lock().unwrap();
        assert_eq!(focus.capture, capture);
        assert_eq!(focus.sidecar_50, sidecar);
        assert_eq!(focus.focused_50, focused);
        drop(focus);
        assert_eq!(s.map_desk.bake_gate().choice(), bake);
        assert_eq!(s.quester_paths.settings().enabled(), quester);
    }

    disk.release();
    settle_panel_writes(&mut s);
    for setting in settings {
        assert!(
            s.core
                .setting_failure(setting)
                .is_some_and(|text| text.starts_with("Not saved: ")),
            "{setting:?}: the failure is retained at its setting"
        );
    }
    assert_eq!(s.draw_shown(DrawSwitch::CaptureInput), capture);
    assert_eq!(s.draw_shown(DrawSwitch::SidecarFullRate), sidecar);
    assert_eq!(s.draw_shown(DrawSwitch::FocusedFullRate), focused);
    assert_eq!(s.quester_enabled_shown(), quester);
    assert_eq!(s.session_log_shown(), logging);
    assert_eq!(s.map_bake_shown(), bake);
    assert_eq!(s.quester_paths.settings().folder(), source.as_path());
    assert!(!s.preferences.document().unwrap().contains_key("capture"));
    // The failures name nothing else: the notice settings are untouched.
    assert!(matches!(
        s.core
            .setting_status(Setting::Notice(OperatorNotice::BackgroundBots)),
        SettingStatus::Saved
    ));
    frontend_core::test_support::scripts::reset_quester_registry();
}

/// A "Got it, don't show again" that failed stays at the notice, and the
/// failure survives the panel pumping on; the resolving action (a new
/// attempt) clears it, and the durable acknowledgement closes the window.
#[test]
fn settings_feedback_a_failed_dont_show_again_stays_at_the_notice_until_retried() {
    use frontend_core::{OperatorNotice, Setting, SettingStatus};
    let (mut s, _iso) = two_live_session("panel-s3-notice");
    hydrate_panel_preferences(&mut s);
    s.select("bob");
    assert!(s.background_ack_open);
    let disk = PanelDisk::install(&s);
    let notice = Setting::Notice(OperatorNotice::BackgroundBots);
    s.ack_background_bots();
    assert!(matches!(
        s.core.setting_status(notice),
        SettingStatus::Pending
    ));
    assert!(s.background_ack_open, "the window stays while it saves");
    let failing = disk.release();
    settle_panel_writes(&mut s);
    assert!(s.background_ack_open, "a failed save keeps the window");
    assert!(!s.ui.background_bots_ack);
    assert!(s
        .core
        .setting_failure(notice)
        .is_some_and(|text| text.starts_with("Not saved: ")));
    for _ in 0..3 {
        s.pump_status();
        s.poll_panel_preferences();
    }
    assert!(s.core.setting_failure(notice).is_some());

    // A new attempt resolves it.
    failing.store(false, Ordering::SeqCst);
    s.ack_background_bots();
    assert_eq!(s.core.setting_failure(notice), None);
    settle_panel_writes(&mut s);
    assert!(s.ui.background_bots_ack);
    assert!(!s.background_ack_open, "the durable ack closes the window");
    assert_eq!(s.core.setting_failure(notice), None);
}

/// Headed defect 3 (2026-10-27): "Don't show again" in the render-all
/// warning renders every wall member only once its save is durable. While
/// another program holds the real preferences lock, the box and the render
/// policy stay at "only render focused"; after the failure, and after the
/// lock is released, they still do, and nothing the panel reported as not
/// saved (this acknowledgement and the background-bots one) is written
/// later without a new action.
#[test]
fn settings_feedback_render_all_waits_for_a_durable_dont_show_again_and_a_failure_changes_nothing()
{
    use frontend_core::{OperatorNotice, Setting, SettingStatus};
    let (mut s, _iso) = two_live_session("panel-s3-r5-render-all");
    s.select("bob");
    assert!(s.background_ack_open);
    assert!(s.focus.lock().unwrap().only_render_selected);
    let lock = crate::test_support::hold_preferences_lock(&s.preferences);
    s.ack_background_bots();
    s.wall.render_all_warn_open = true;
    s.wall.render_all_understood = true;
    s.confirm_render_all(true);
    let notices = [
        Setting::Notice(OperatorNotice::BackgroundBots),
        Setting::Notice(OperatorNotice::RenderAllWarning),
    ];
    for _ in 0..3 {
        s.pump_status();
        s.poll_panel_preferences();
        assert!(matches!(
            s.core.setting_status(notices[1]),
            SettingStatus::Pending
        ));
        assert!(
            s.focus.lock().unwrap().only_render_selected,
            "nothing renders all while the save is pending"
        );
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    while notices
        .iter()
        .any(|notice| s.core.setting_failure(*notice).is_none())
    {
        s.pump_status();
        s.poll_panel_preferences();
        assert!(Instant::now() < deadline, "the held writes never failed");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        s.focus.lock().unwrap().only_render_selected,
        "after the failure the box and the rendering stay at only render focused"
    );
    assert!(
        s.wall.render_all_warn_open,
        "the warning stays open with its failure"
    );

    drop(lock);
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until || s.preferences_pending() {
        s.pump_status();
        s.poll_panel_preferences();
        assert!(
            Instant::now() < until + Duration::from_secs(30),
            "a write never ended"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        s.focus.lock().unwrap().only_render_selected,
        "the released lock changes nothing without a new action"
    );
    let saved = crate::test_support::preferences_on_disk(&s.preferences);
    for key in ["render_all_warning_ack", "background_bots_ack"] {
        assert_eq!(
            saved.get(key),
            None,
            "{key} was written after the lock was released, without a user action"
        );
    }
}

/// Panel-owned presentation choices (palette, layout): the preview shows at
/// once, and a failed write drops only that preview, shows the durable value
/// again, and reports at the setting. The remembered launch choices are not
/// previews: see `settings_feedback_launch_choices_take_effect_only_from_the_durable_value`.
#[test]
fn settings_feedback_held_then_failed_presentation_choices_revert_to_the_durable_value() {
    let _iso = IsolatedStores::enter("panel-s3-presentation");
    let saved = crate::ui_state::PanelUiState {
        show_parameters_rail: true,
        chrome: crate::theme::ChromeColors {
            accent: "#112233".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    seed_ui_state(&saved);
    let mut s = Session::new();
    hydrate_panel_preferences(&mut s);
    s.watch_setting_events();
    let saved_accent = s.ui.chrome.accent.clone();
    assert_eq!(saved_accent, "#112233");
    let disk = PanelDisk::install(&s);

    s.persist_panel_member(
        &["show_parameters_rail"],
        Some(serde_json::Value::Bool(false)),
    )
    .unwrap();
    s.persist_panel_serializable(&["chrome", "accent"], "#445566")
        .unwrap();
    s.ui.chrome.accent = "#445566".into();
    s.ui.show_parameters_rail = false;
    for _ in 0..3 {
        s.pump_status();
        s.poll_panel_preferences();
        assert_eq!(s.ui.chrome.accent, "#445566");
        assert!(!s.ui.show_parameters_rail);
    }
    let (events, incomplete) = s.take_panel_setting_events();
    assert!(!incomplete);
    assert_eq!(
        events
            .iter()
            .map(|event| (event.setting(), event.state()))
            .collect::<Vec<_>>(),
        [
            (
                "show_parameters_rail",
                frontend_core::SettingEventState::Pending
            ),
            ("chrome.accent", frontend_core::SettingEventState::Pending),
        ]
    );

    disk.release();
    settle_panel_writes(&mut s);
    let (events, incomplete) = s.take_panel_setting_events();
    assert!(!incomplete);
    assert_eq!(
        events
            .iter()
            .map(|event| (event.setting(), event.state()))
            .collect::<Vec<_>>(),
        [
            (
                "show_parameters_rail",
                frontend_core::SettingEventState::Failed
            ),
            ("chrome.accent", frontend_core::SettingEventState::Failed),
        ]
    );
    assert_eq!(s.ui.chrome.accent, saved_accent);
    assert!(s.ui.show_parameters_rail);
    for path in [&["show_parameters_rail"][..], &["chrome"]] {
        assert!(
            s.panel_member_error(path)
                .is_some_and(|text| text.starts_with("Preferences not saved: ")),
            "{path:?}: the failure shows at its setting"
        );
    }
    let document = s.preferences.document().unwrap();
    assert!(!document.contains_key("chrome"), "chrome was never stored");

    // The failure leaves by itself after the error timeout.
    s.expire_panel_write_errors(Instant::now() + frontend_core::ERROR_MESSAGE_TIMEOUT);
    assert_eq!(s.panel_member_error(&["chrome"]), None);
    let (events, incomplete) = s.take_panel_setting_events();
    assert!(!incomplete);
    assert_eq!(
        events
            .iter()
            .map(|event| (event.setting(), event.state()))
            .collect::<Vec<_>>(),
        [
            (
                "show_parameters_rail",
                frontend_core::SettingEventState::Cleared
            ),
            ("chrome.accent", frontend_core::SettingEventState::Cleared),
        ]
    );
}

/// A typed Quest Paths folder survives any number of refresh frames, whether
/// or not another process saved in between, and a held operational choice
/// keeps showing the operator's choice across them.
#[test]
fn settings_feedback_a_typed_pending_draft_survives_refresh_frames_and_a_second_process_version() {
    use frontend_core::command::DrawSwitch;
    let _iso = IsolatedStores::enter("panel-s3-draft");
    let mut s = Session::new();
    hydrate_panel_preferences(&mut s);
    let capture = s.draw_shown(DrawSwitch::CaptureInput);
    let peer = crate::test_support::thread_preferences();
    s.quester_paths_folder_edit = "typed-but-not-committed".into();
    s.quester_paths_folder_dirty = true;
    let disk = PanelDisk::install(&s);
    s.set_capture(!capture);
    for frame in 0..5 {
        crate::test_support::set_patiently(&peer, "peer_marker", serde_json::json!(frame));
        // A frame's re-read; it may find the held writer busy.
        let _ = s.preferences.reload();
        s.pump_status();
        s.poll_panel_preferences();
        assert_eq!(s.quester_paths_folder_edit, "typed-but-not-committed");
        assert_eq!(s.draw_shown(DrawSwitch::CaptureInput), !capture);
    }
    disk.failing.store(false, Ordering::SeqCst);
    drop(disk.release());
    settle_panel_writes(&mut s);
    assert_eq!(s.draw_shown(DrawSwitch::CaptureInput), !capture);
    assert_eq!(s.quester_paths_folder_edit, "typed-but-not-committed");
}

/// A cold panel (preferences not yet read) adopts every saved choice when
/// the read is released and writes no default over the file meanwhile.
#[test]
fn settings_feedback_a_cold_panel_adopts_every_saved_choice_when_released_and_writes_no_default() {
    let iso = IsolatedStores::enter("panel-s3-cold");
    let state = iso.dir().join("cold-state");
    let saved: &[u8] = br##"{"capture":false,"sidecar_50":true,"focused_50":true,"map_bake":"always","quester_paths":{"enabled":false,"folder":"saved-source"}}"##;
    let path = host_play::local_state::StateRoot::new(state.clone()).preferences_file();
    crate::test_support::write_private_fixture(&path, saved).unwrap();
    let mut s = Session::new();
    s.core.set_spawn_workers(false);
    let (entered, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release, release_rx) = std::sync::mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let first = std::sync::atomic::AtomicBool::new(true);
    let service = host_play::local_state::LocalStateService::open_process_with_read_stall_for_test(
        host_play::local_state::StateRoot::new(state),
        Arc::new(move || {
            if first.swap(false, Ordering::SeqCst) {
                let _ = entered.send(());
                let _ = release_rx.lock().unwrap().recv();
            }
        }),
    );
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    s.core.bind_local_state(service);
    s.preferences = s.core.preferences().unwrap();
    s.preferences_ready = false;
    for _ in 0..4 {
        s.pump_status();
        s.poll_panel_preferences();
        assert!(!s.panel_preferences_ready());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            saved,
            "no default is written"
        );
    }
    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !s.panel_preferences_ready() && Instant::now() < deadline {
        s.pump_status();
        s.poll_panel_preferences();
        thread::sleep(Duration::from_millis(5));
    }
    assert!(s.panel_preferences_ready());
    s.poll_panel_preferences();
    {
        let focus = s.focus.lock().unwrap();
        assert!(!focus.capture);
        assert!(focus.sidecar_50);
        assert!(focus.focused_50);
    }
    assert_eq!(s.map_desk.bake_gate().choice(), MapBakeChoice::Always);
    // The panel's own presentation members are the stand-in's under test;
    // their cold adoption is `ui_state::from_document`.
    assert_eq!(
        s.quester_paths.settings().folder(),
        std::path::Path::new("saved-source")
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        saved,
        "no default is written"
    );
    frontend_core::test_support::scripts::reset_quester_registry();
}

// --- T5.3-S3 round 2: a panel write owns its preview until its own result ---

/// A disk for one panel session's service that holds every file write at its
/// start until [`SeqDisk::release_one`] lets the oldest go, and fails every
/// write numbered above the count set by [`SeqDisk::fail_after`] (the first
/// write is number 1; counting starts when the disk is installed).
struct SeqDisk {
    release: std::sync::mpsc::Sender<()>,
    fail_after: Arc<std::sync::atomic::AtomicUsize>,
}

impl SeqDisk {
    fn install(session: &Session) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        let service = session
            .core
            .local_state_for_test()
            .expect("the panel owns its process state service");
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let writes = Arc::new(AtomicUsize::new(0));
        let fail_after = Arc::new(AtomicUsize::new(usize::MAX));
        let started = Arc::clone(&writes);
        service.set_write_stall_for_test(Arc::new(move || {
            started.fetch_add(1, SeqCst);
            let _ = gate.lock().unwrap().recv();
        }));
        let counted = Arc::clone(&writes);
        let limit = Arc::clone(&fail_after);
        service
            .set_write_fault_for_test(Arc::new(move || counted.load(SeqCst) > limit.load(SeqCst)));
        Self {
            release,
            fail_after,
        }
    }

    /// Let the oldest held write run.
    fn release_one(&self) {
        self.release.send(()).expect("a write waits at the gate");
    }

    /// Writes numbered above `n` fail.
    fn fail_after(&self, n: usize) {
        self.fail_after
            .store(n, std::sync::atomic::Ordering::SeqCst);
    }
}

/// A panel-owned member is a request: A (`X` to `Y`) and B (`Y` back to `X`,
/// the value the document held before A) are accepted behind a held writer.
/// While both are held the preview is B's; when A becomes durable and B fails
/// the control shows the durable `Y`, not B's `X`, and B's failure is
/// reported at the setting (it is not mistaken for the already stored `X`).
/// Two requests of the same value are two requests: the first landing does
/// not end the second, and its failure shows.
#[test]
fn settings_feedback_panel_requests_own_their_preview_and_failure_by_identity() {
    let _iso = IsolatedStores::enter("panel-s3-request-owner");
    let _reader = crate::ui_state::read_documents_for_test();
    let colors = |accent: &str| {
        serde_json::to_value(crate::theme::ChromeColors {
            accent: accent.into(),
            ..Default::default()
        })
        .unwrap()
    };
    crate::test_support::set_patiently(
        &crate::test_support::thread_preferences(),
        "chrome",
        colors("#AA0000"),
    );
    let mut s = Session::new();
    hydrate_panel_preferences(&mut s);
    assert_eq!(s.ui.chrome.accent, "#AA0000");
    let disk = SeqDisk::install(&s);
    disk.fail_after(1);

    s.persist_panel_serializable(&["chrome", "accent"], "#112233")
        .unwrap();
    s.ui.chrome.accent = "#112233".into();
    s.persist_panel_serializable(&["chrome", "accent"], "#AA0000")
        .unwrap();
    s.ui.chrome.accent = "#AA0000".into();
    for _ in 0..3 {
        s.pump_status();
        s.poll_panel_preferences();
        assert_eq!(
            s.ui.chrome.accent, "#AA0000",
            "B's preview shows while both are held"
        );
        assert_eq!(s.panel_member_error(&["chrome"]), None);
    }

    // A lands, B fails.
    disk.release_one();
    disk.release_one();
    settle_panel_writes(&mut s);
    assert_eq!(
        s.ui.chrome.accent, "#112233",
        "B failed: the control shows the durable value A saved"
    );
    assert!(
        s.panel_member_error(&["chrome"])
            .is_some_and(|text| text.starts_with("Preferences not saved: ")),
        "B's failure is reported at its setting"
    );
    assert_eq!(
        s.preferences.document().unwrap()["chrome"]["accent"],
        "#112233"
    );

    // Two requests of one value: the first landing does not end the second.
    disk.fail_after(3);
    for _ in 0..2 {
        s.persist_panel_serializable(&["chrome", "accent"], "#334455")
            .unwrap();
    }
    s.ui.chrome.accent = "#334455".into();
    disk.release_one();
    // The first write is durable (the document now holds the value): the
    // second request is still pending and still previews it.
    let deadline = Instant::now() + Duration::from_secs(30);
    while s.preferences.document().unwrap()["chrome"]["accent"] != "#334455" {
        s.pump_status();
        s.poll_panel_preferences();
        assert!(Instant::now() < deadline, "the first request never landed");
        thread::sleep(Duration::from_millis(2));
    }
    s.poll_panel_preferences();
    assert_eq!(s.panel_preference_writes_pending(), 1, "B is still pending");
    assert_eq!(s.ui.chrome.accent, "#334455");
    disk.release_one();
    settle_panel_writes(&mut s);
    assert_eq!(s.ui.chrome.accent, "#334455");
    assert!(
        s.panel_member_error(&["chrome"]).is_some(),
        "the second request failed and says so"
    );

    // Three requests end before one poll sees them: A and B land, the newest
    // (C) fails. The control shows B's durable value, and an older success
    // never ends C's failure, whatever order the poll meets them in.
    s.expire_panel_write_errors(Instant::now() + frontend_core::ERROR_MESSAGE_TIMEOUT);
    assert_eq!(s.panel_member_error(&["chrome"]), None);
    disk.fail_after(6);
    for accent in ["#0A0A0A", "#0B0B0B", "#0C0C0C"] {
        s.persist_panel_serializable(&["chrome", "accent"], accent)
            .unwrap();
        s.ui.chrome.accent = accent.into();
    }
    for _ in 0..3 {
        disk.release_one();
    }
    await_panel_terminals(&s);
    s.poll_panel_preferences();
    assert_eq!(s.panel_preference_writes_pending(), 0);
    assert_eq!(
        s.preferences.document().unwrap()["chrome"]["accent"],
        "#0B0B0B"
    );
    assert_eq!(s.ui.chrome.accent, "#0B0B0B", "B's durable value shows");
    assert!(
        s.panel_member_error(&["chrome"])
            .is_some_and(|text| text.starts_with("Preferences not saved: ")),
        "the newest request failed; an older success must not clear its error"
    );
}

/// Wait at the service, without polling the panel, until every queued panel
/// write has its terminal result, so that the next poll meets them all
/// completed at once.
fn await_panel_terminals(s: &Session) {
    let service = s
        .core
        .local_state_for_test()
        .expect("the panel owns its process state service");
    for write in &s.panel_preference_writes {
        if let frontend_core::prefs::Update::Queued(update) = &write.update {
            assert_ne!(
                service.wait_request(update.request_id(), Duration::from_secs(30)),
                host_play::local_state::RequestState::Pending,
                "a held write never ended"
            );
        }
    }
}

/// Three remembered raster choices (CPU, Off, GPU) end before one poll sees
/// them: what takes effect (the panel's applied raster and the profile's
/// saved raster) is the newest durable choice, GPU, as the document holds.
#[test]
fn settings_feedback_completed_launch_writes_take_effect_in_request_order() {
    use frontend_core::command::RasterMode;
    let _iso = IsolatedStores::enter("panel-s3-r3-launch-order");
    let path = tmp_vault("launch-order.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    hydrate_panel_preferences(&mut s);
    s.select("alice");
    assert_eq!(s.focused_raster(), RasterMode::Gpu);
    // The selection's own saves end before the disk holds writes, so that
    // each release below lets one raster write go.
    settle_panel_writes(&mut s);
    s.core.flush_writes();
    let disk = SeqDisk::install(&s);
    for raster in [RasterMode::Cpu, RasterMode::Off, RasterMode::Gpu] {
        assert!(s.set_focused_raster(raster));
    }
    // All three run (and every later write runs unheld); the panel does not
    // poll until each has its result.
    drop(disk);
    await_panel_terminals(&s);
    s.poll_panel_preferences();
    s.core.flush_writes();
    let durable = s.preferences.document().unwrap()["raster"].clone();
    assert_eq!(durable, "gpu");
    assert_eq!(
        (
            s.focused_raster(),
            s.core.profile("alice").unwrap().raster()
        ),
        (RasterMode::Gpu, RasterMode::Gpu),
        "all writes completed before one panel poll; durable raster={durable}"
    );
    assert_eq!(s.ui.raster, RasterMode::Gpu);
}

/// A second process's later saved values reach every panel-owned control, not
/// only the first read: the palette, layout, folds, remembered folders,
/// order members and, while no bot is focused, the remembered raster and
/// memory mode follow each new durable version.
#[test]
fn settings_feedback_a_second_process_version_reaches_every_panel_control_after_hydration() {
    let _iso = IsolatedStores::enter("panel-s3-second-process-panel");
    let _reader = crate::ui_state::read_documents_for_test();
    let mut s = Session::new();
    hydrate_panel_preferences(&mut s);
    let peer = crate::test_support::thread_preferences();
    let column = crate::fleet_columns::FleetColumn::Idle;
    let flipped = !column.default_on();
    let refresh = |s: &mut Session| {
        s.preferences.reload().unwrap();
        s.pump_status();
        s.poll_panel_preferences();
    };
    let peer_set = |key: &str, value: serde_json::Value| {
        crate::test_support::set_patiently(&peer, key, value);
    };

    peer_set("fleet_columns", serde_json::json!({ column.id(): flipped }));
    peer_set("section_order", serde_json::json!(["log", "status"]));
    peer_set(
        "script_category_order",
        serde_json::json!(["Zeta", "Alpha"]),
    );
    peer_set(
        "collapsed",
        serde_json::json!({ "alice": { "status": true } }),
    );
    peer_set("script_load_last_dir", serde_json::json!("/peer/load"));
    peer_set("show_parameters_rail", serde_json::Value::Bool(true));
    peer_set(
        "nav",
        serde_json::json!({ "show_nav_path": true, "color_path": "#0A0B0C" }),
    );
    refresh(&mut s);
    assert_eq!(
        s.ui.fleet_columns.get(column.id()),
        Some(&flipped),
        "the Fleet column choice follows the other process"
    );
    assert_eq!(s.ui.section_order, ["log", "status"]);
    assert_eq!(s.ui.script_category_order, ["Zeta", "Alpha"]);
    assert!(s.ui.collapsed["alice"]["status"]);
    assert_eq!(
        s.ui.script_load_last_dir.as_deref(),
        Some(std::path::Path::new("/peer/load"))
    );
    assert!(s.ui.show_parameters_rail);
    assert!(s.ui.nav.show_nav_path);
    assert_eq!(s.ui.nav.color_path, "#0A0B0C");

    // The remembered launch choices follow too: no bot is focused, so the
    // raster and memory controls show the saved choice.
    use frontend_core::command::RasterMode;
    assert_eq!(
        (s.focused_raster(), s.focused_lowmem()),
        (RasterMode::Gpu, true)
    );
    peer_set("raster", serde_json::json!("cpu"));
    peer_set("lowmem", serde_json::Value::Bool(false));
    refresh(&mut s);
    assert_eq!(
        (s.focused_raster(), s.focused_lowmem()),
        (RasterMode::Cpu, false),
        "later durable document: raster={} lowmem={}",
        s.preferences.document().unwrap()["raster"],
        s.preferences.document().unwrap()["lowmem"]
    );

    // A later version of the palette arrives too, and this panel's own
    // pending request keeps previewing until its result.
    let colors = |accent: &str| {
        serde_json::to_value(crate::theme::ChromeColors {
            accent: accent.into(),
            ..Default::default()
        })
        .unwrap()
    };
    peer_set("chrome", colors("#998877"));
    refresh(&mut s);
    assert_eq!(s.ui.chrome.accent, "#998877");
    let disk = PanelDisk::install(&s);
    s.persist_panel_serializable(&["chrome", "accent"], "#112233")
        .unwrap();
    s.ui.chrome.accent = "#112233".into();
    for _ in 0..3 {
        s.pump_status();
        s.poll_panel_preferences();
        assert_eq!(
            s.ui.chrome.accent, "#112233",
            "the pending request previews"
        );
    }
    disk.failing.store(false, Ordering::SeqCst);
    drop(disk.release());
    settle_panel_writes(&mut s);
    assert_eq!(s.ui.chrome.accent, "#112233");
    assert_eq!(s.ui.section_order, ["log", "status"], "nothing else moved");
}

/// When focus ends (here the last focused bot is removed), the raster and
/// memory controls show the shared saved value at once, also a value another
/// process saved while the bot was focused. No later saved-file version is
/// necessary. While the bot is focused, they show its own saved modes.
#[test]
fn settings_feedback_focus_loss_shows_the_saved_launch_choices_without_a_new_version() {
    use frontend_core::command::RasterMode;
    let _iso = IsolatedStores::enter("panel-s3-r4-focus-loss");
    let _reader = crate::ui_state::read_documents_for_test();
    let path = tmp_vault("focus-loss.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    hydrate_panel_preferences(&mut s);
    s.load("alice");
    assert_eq!(s.focused_name().as_deref(), Some("alice"));
    settle_panel_writes(&mut s);
    s.core.flush_writes();
    assert_eq!(
        (s.focused_raster(), s.focused_lowmem()),
        (RasterMode::Gpu, true)
    );

    let peer = crate::test_support::thread_preferences();
    crate::test_support::set_patiently(&peer, "raster", serde_json::json!("cpu"));
    crate::test_support::set_patiently(&peer, "lowmem", serde_json::Value::Bool(false));
    s.preferences.reload().unwrap();
    s.pump_status();
    s.poll_panel_preferences();
    assert_eq!(
        (s.focused_raster(), s.focused_lowmem()),
        (RasterMode::Gpu, true),
        "the focused bot's controls show its own saved modes"
    );
    let version = s.preferences.snapshot().unwrap().version();

    s.rail_remove("alice");
    assert!(s.focused_name().is_none());
    // The next frame's poll; the saved file has no new version.
    s.poll_panel_preferences();
    assert_eq!(s.preferences.snapshot().unwrap().version(), version);
    let document = s.preferences.document().unwrap();
    assert_eq!(
        (s.focused_raster(), s.focused_lowmem()),
        (RasterMode::Cpu, false),
        "no bot is focused: the controls show the saved file (raster={} lowmem={})",
        document["raster"],
        document["lowmem"]
    );
}

/// The Quest Paths folder field follows another process's saved folder while
/// it is clean, and keeps what the operator is typing.
#[test]
fn settings_feedback_the_quester_folder_field_follows_a_second_process_unless_typed_in() {
    let _iso = IsolatedStores::enter("panel-s3-quester-field");
    let _reader = crate::ui_state::read_documents_for_test();
    let mut s = Session::new();
    hydrate_panel_preferences(&mut s);
    let peer = crate::test_support::thread_preferences();
    let quester = |folder: &str| serde_json::json!({ "enabled": true, "folder": folder });
    let refresh = |s: &mut Session| {
        s.preferences.reload().unwrap();
        s.pump_status();
        s.poll_panel_preferences();
    };
    crate::test_support::set_patiently(&peer, "quester_paths", quester("peer-folder-one"));
    refresh(&mut s);
    assert_eq!(
        s.quester_paths.settings().folder(),
        std::path::Path::new("peer-folder-one")
    );
    assert_eq!(
        s.quester_paths_folder_edit, "peer-folder-one",
        "a clean field shows the saved folder"
    );

    s.quester_paths_folder_edit = "typed-not-committed".into();
    s.quester_paths_folder_dirty = true;
    crate::test_support::set_patiently(&peer, "quester_paths", quester("peer-folder-two"));
    refresh(&mut s);
    assert_eq!(
        s.quester_paths.settings().folder(),
        std::path::Path::new("peer-folder-two")
    );
    assert_eq!(
        s.quester_paths_folder_edit, "typed-not-committed",
        "what the operator is typing stays"
    );
    frontend_core::test_support::scripts::reset_quester_registry();
}

// --- T5.3-S3 round 2: remembered launch choices take effect only when durable ---

/// The remembered Game-pane raster and memory mode are saved choices: while
/// their write is held the control shows the choice, but the profile's saved
/// raster and memory mode, the slot's GPU/CPU latch and the panel's applied
/// values stay what they were; when the write fails nothing was applied and
/// the control shows the applied value again with the failure at the setting;
/// when it is durable the effects follow.
#[test]
fn settings_feedback_launch_choices_take_effect_only_from_the_durable_value() {
    use frontend_core::command::RasterMode;
    let _iso = IsolatedStores::enter("panel-s3-launch-choices");
    let path = tmp_vault("launch-choices.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    hydrate_panel_preferences(&mut s);
    s.select("alice");
    assert_eq!(s.focused_raster(), RasterMode::Gpu);
    assert!(s.focused_lowmem());
    let applied = |s: &mut Session| {
        s.core.flush_writes();
        let profile = s.core.profile("alice").unwrap();
        (
            profile.lowmem(),
            profile.raster(),
            s.core.slot_io("alice").unwrap().input.prefer_cpu(),
            s.ui.lowmem,
            s.ui.raster,
        )
    };
    let before = applied(&mut s);
    assert_eq!(
        before,
        (true, RasterMode::Gpu, false, true, RasterMode::Gpu)
    );
    let disk = PanelDisk::install(&s);

    assert!(s.set_focused_lowmem(false));
    assert!(s.set_focused_raster(RasterMode::Cpu));
    for _ in 0..3 {
        s.pump_status();
        s.poll_panel_preferences();
        assert!(!s.focused_lowmem(), "the control shows the choice");
        assert_eq!(s.focused_raster(), RasterMode::Cpu);
        assert_eq!(
            applied(&mut s),
            before,
            "nothing is applied while the write is held"
        );
    }

    // The write fails: nothing was applied, the control shows the applied
    // value, and the failure is at the setting.
    let failing = disk.release();
    settle_panel_writes(&mut s);
    assert_eq!(applied(&mut s), before, "a failed write applied nothing");
    assert!(s.focused_lowmem());
    assert_eq!(s.focused_raster(), RasterMode::Gpu);
    for member in [&["lowmem"][..], &["raster"]] {
        assert!(
            s.panel_member_error(member)
                .is_some_and(|text| text.starts_with("Preferences not saved: ")),
            "{member:?}: the failure shows at its setting"
        );
    }

    // The retry lands: the effects follow the durable value.
    failing.store(false, Ordering::SeqCst);
    assert!(s.set_focused_lowmem(false));
    assert!(s.set_focused_raster(RasterMode::Cpu));
    settle_panel_writes(&mut s);
    assert_eq!(
        applied(&mut s),
        (false, RasterMode::Cpu, true, false, RasterMode::Cpu)
    );
    assert!(!s.focused_lowmem());
    assert_eq!(s.focused_raster(), RasterMode::Cpu);
    assert_eq!(s.panel_member_error(&["lowmem"]), None);
    assert_eq!(s.panel_member_error(&["raster"]), None);
}

/// The server revision is a saved choice too: while its write is held the
/// revision control shows the choice, but the preparation and the binding
/// read the durable revision, so a profile bound meanwhile binds with it
/// (whatever the write does later), and a failed write leaves the failure
/// at the setting.
#[test]
fn settings_feedback_a_held_server_revision_binds_the_durable_revision_and_shows_the_choice() {
    let (root, cache, manifest, unpack, server) = runtime_profile_fixture(274);
    let mut s = Session::new();
    configure_isolated_profile(
        &mut s,
        ProfileOptions {
            revision: None,
            cache_dir: Some(cache),
            cache_manifest: Some(manifest),
            unpack_dir: Some(unpack),
            http_port: Some(server.port()),
            ..ProfileOptions::default()
        },
        &root,
    )
    .unwrap();
    hydrate_panel_preferences(&mut s);
    assert_eq!(s.effective_revision_label().unwrap(), "274");
    let disk = PanelDisk::install(&s);

    s.set_server_revision(289).unwrap();
    for _ in 0..3 {
        s.pump_status();
        s.poll_panel_preferences();
        assert_eq!(
            s.revision_choice_label().unwrap(),
            "289",
            "the control shows the choice"
        );
        assert_eq!(
            s.effective_revision_label().unwrap(),
            "274",
            "the revision the launch reads is the durable one"
        );
        assert_eq!(s.ui.server_revision, 274);
    }
    s.bind_profile().unwrap();
    assert_eq!(
        s.effective_revision_label().unwrap(),
        "274",
        "the profile bound with the durable revision"
    );

    disk.release();
    settle_panel_writes(&mut s);
    assert!(s
        .panel_member_error(&["server_revision"])
        .is_some_and(|text| text.starts_with("Preferences not saved: ")));
    assert_eq!(s.effective_revision_label().unwrap(), "274");
    assert_eq!(s.revision_choice_label().unwrap(), "274");
    assert!(
        s.set_server_revision(289).is_err(),
        "a bound session refuses"
    );
}

// --- T5.3-S3 round 3: a pending launch choice belongs to its profile ---

/// A pending raster or memory choice belongs to the bot that was focused
/// when it was made. Another focused bot's controls show that bot's own
/// values, say nothing about the first bot's save, and accept a choice of
/// their own. The first bot's controls show its pending choice again when
/// it is focused again, and each failure shows at the bot it belongs to.
#[test]
fn settings_feedback_a_pending_launch_choice_stays_with_its_profile() {
    use frontend_core::command::RasterMode;
    let _iso = IsolatedStores::enter("panel-s3-r3-launch-owner");
    let path = tmp_vault("launch-owner.vault");
    let mut s = Session::new();
    install_vault(
        &mut s.core,
        VaultFixture::create(&path, "test-passphrase-01"),
    );
    vault_upsert(&mut s.core, profile("alice", "pw", 42));
    vault_upsert(&mut s.core, profile("bob", "pw", 43));
    hydrate_panel_preferences(&mut s);
    s.select("alice");
    assert_eq!(
        (s.focused_raster(), s.focused_lowmem()),
        (RasterMode::Gpu, true)
    );
    let disk = PanelDisk::install(&s);
    s.request_focused_raster(RasterMode::Cpu);
    s.request_focused_lowmem(false);
    assert_eq!(
        (s.focused_raster(), s.focused_lowmem()),
        (RasterMode::Cpu, false)
    );
    assert!(s.panel_member_pending(&["raster"]) && s.panel_member_pending(&["lowmem"]));

    s.select("bob");
    assert_eq!(
        (s.focused_raster(), s.core.profile("bob").unwrap().raster()),
        (RasterMode::Gpu, RasterMode::Gpu),
        "Alice owns the pending CPU choice, not Bob"
    );
    assert!(s.focused_lowmem(), "Alice owns the pending memory choice");
    assert!(
        !s.panel_member_pending(&["raster"]) && !s.panel_member_pending(&["lowmem"]),
        "Bob's controls are not saving"
    );
    // A choice for Bob is a choice of its own: it is submitted.
    let before = s.panel_preference_writes_pending();
    s.request_focused_raster(RasterMode::Cpu);
    assert_eq!(
        s.panel_preference_writes_pending(),
        before + 1,
        "Bob's choice is submitted"
    );
    assert_eq!(s.focused_raster(), RasterMode::Cpu);
    assert!(s.panel_member_pending(&["raster"]));
    assert!(!s.panel_member_pending(&["lowmem"]));

    s.select("alice");
    assert_eq!(
        (s.focused_raster(), s.focused_lowmem()),
        (RasterMode::Cpu, false),
        "Alice's pending choices show again"
    );
    assert!(s.panel_member_pending(&["raster"]) && s.panel_member_pending(&["lowmem"]));

    // Every held write fails: each failure shows at the bot it belongs to.
    disk.release();
    settle_panel_writes(&mut s);
    assert_eq!(
        (s.focused_raster(), s.focused_lowmem()),
        (RasterMode::Gpu, true)
    );
    assert!(s.panel_member_error(&["raster"]).is_some());
    assert!(s.panel_member_error(&["lowmem"]).is_some());
    s.select("bob");
    assert_eq!(
        (s.focused_raster(), s.focused_lowmem()),
        (RasterMode::Gpu, true)
    );
    assert!(
        s.panel_member_error(&["raster"]).is_some(),
        "Bob's own raster choice failed"
    );
    assert_eq!(
        s.panel_member_error(&["lowmem"]),
        None,
        "Alice's memory failure is not Bob's"
    );
}

#[test]
fn panel_skip_quest_refusal_uses_expiring_shared_command_feedback() {
    use frontend_core::ERROR_MESSAGE_TIMEOUT as TIMEOUT;

    let _iso = IsolatedStores::enter("panel-quester-skip-refusal");
    let mut s = Session::new();
    s.core.set_spawn_workers(false);
    s.core.set_play(Some(empty_play()));
    install_vault(
        &mut s.core,
        VaultFixture::create(
            &tmp_vault("panel-quester-skip-refusal"),
            "test-passphrase-01",
        ),
    );
    let name = "skiprefusalpanel";
    vault_upsert(&mut s.core, profile(name, "pw", 42));
    s.persist_ui = false;
    s.load(name);
    s.select(name);

    let before = s.core.refusals();
    s.script_skip_quest(None);
    let message = "bot is not running the Quester queue";
    assert_eq!(s.error.as_deref(), Some(message));
    assert_eq!(s.core.refusals(), before + 1);
    let lines = refusal_lines_naming(name);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("SkipQuest") && lines[0].contains(message));

    let now = Instant::now();
    s.expire_error(now);
    s.expire_error(now + TIMEOUT - Duration::from_secs(1));
    assert_eq!(s.error.as_deref(), Some(message));
    s.expire_error(now + TIMEOUT);
    assert_eq!(s.error, None, "the refusal expires at the shared timeout");
}
