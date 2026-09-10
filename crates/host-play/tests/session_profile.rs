//! Profile selection and actual shared-client construction, without game servers.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use client::{io::ClientRevision, BotTarget};
use host_play::profile::{CacheManifest, NavAvailability, NavManifest, ProfileEnvironment};
use host_play::{parse_profile_args, ProfileOptions, SharedClientTemplate};

static NEXT: AtomicUsize = AtomicUsize::new(0);
static CLIENTS: Mutex<()> = Mutex::new(());
const ARCHIVES: [&str; 8] = [
    "title",
    "config",
    "interface",
    "media",
    "versionlist",
    "textures",
    "wordenc",
    "sounds",
];

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "274bot-session-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/profile");
        for file in ARCHIVES
            .into_iter()
            .chain(["manifest-274.json", "manifest-289.json"])
        {
            std::fs::copy(source.join(file), path.join(file)).unwrap();
        }
        Self(path)
    }
    fn env(&self) -> ProfileEnvironment {
        ProfileEnvironment {
            home: Some(self.0.clone()),
            rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
            rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
            ..Default::default()
        }
    }
    fn options(&self, revision: u16) -> ProfileOptions {
        ProfileOptions {
            revision: Some(revision.to_string()),
            cache_dir: Some(self.0.clone()),
            cache_manifest: Some(self.0.join(format!("manifest-{revision}.json"))),
            nav_pack: Some(self.0.join("missing.navpack")),
            ..Default::default()
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn flags_are_order_independent_and_override_saved_or_environment_revision() {
    let fixture = Fixture::new();
    let mut environment = fixture.env();
    environment.revision = Some("377".into());
    for args in [
        vec![
            "--revision",
            "289",
            "--host",
            "localhost",
            "--port",
            "45123",
            "--script",
            "foo",
        ],
        vec![
            "--port",
            "45123",
            "--script",
            "foo",
            "--host",
            "localhost",
            "--revision",
            "289",
        ],
    ] {
        let (options, rest) = parse_profile_args(args).unwrap();
        assert_eq!(rest, ["--script", "foo"]);
        let selected = options.resolve_with_env(Some(377), &environment).unwrap();
        assert_eq!(selected.revision(), ClientRevision::R289);
        assert_eq!(selected.game_host(), "localhost");
        assert_eq!(selected.game_port(), 45123);
        assert_eq!(selected.asset_port(), 1080);
        assert!(selected.vault_path().ends_with("vault-289"));
        assert!(selected
            .cache_dir()
            .ends_with("lostcity-289/engine/data/pack/client"));
        assert!(selected.nav_pack().ends_with("289/274bot.navpack"));
    }
    let (options, _) = parse_profile_args(["--profile", "local-274"]).unwrap();
    environment.profile = Some("unsupported".into());
    let selected = options.resolve_with_env(Some(289), &environment).unwrap();
    assert_eq!(selected.revision(), ClientRevision::R274);
    assert_eq!(selected.game_port(), 43594);
    assert!(selected.vault_path().ends_with(".274bot/vault"));

    let mut environment = fixture.env();
    environment.working_dir = Some(fixture.0.clone());
    let (options, _) =
        parse_profile_args(["--cache", "relative-cache", "--vault", "relative-vault"]).unwrap();
    let selected = options.resolve_with_env(None, &environment).unwrap();
    assert_eq!(selected.cache_dir(), fixture.0.join("relative-cache"));
    assert_eq!(selected.vault_path(), fixture.0.join("relative-vault"));
}

#[test]
fn public_289_defaults_and_named_profile_override_lower_priority_inputs() {
    let fixture = Fixture::new();
    let mut lower_priority = fixture.env();
    lower_priority.profile = Some("local-274".into());
    lower_priority.revision = Some("274".into());
    lower_priority.target = Some("local".into());

    let (named, _) = parse_profile_args(["--profile", "public-289"]).unwrap();
    let selected = named.resolve_with_env(Some(274), &lower_priority).unwrap();
    assert_eq!(selected.revision(), ClientRevision::R289);
    assert_eq!(selected.target(), BotTarget::Prod);
    assert_eq!(selected.game_host(), "w1.rs2b2t.com");
    assert_eq!(selected.game_port(), 443);
    assert_eq!(selected.asset_host(), "w1.rs2b2t.com");
    assert_eq!(selected.asset_port(), 443);
    assert!(selected.unpack_dir().ends_with(".274bot/unpack-289"));
    assert_eq!(selected.cache_dir(), selected.unpack_dir());
    assert!(selected.nav_pack().ends_with(".274bot/289/274bot.navpack"));
    assert!(selected
        .nav_flags()
        .ends_with(".274bot/289/274bot.navflags"));
    assert!(selected
        .content_dir()
        .ends_with("experiments/lostcity-289/content"));
    assert!(selected.vault_path().ends_with(".274bot/vault-prod"));

    let (prod, _) = parse_profile_args(["--prod"]).unwrap();
    let selected = prod.resolve_with_env(Some(274), &fixture.env()).unwrap();
    assert_eq!(selected.revision(), ClientRevision::R289);
    assert_eq!(selected.target(), BotTarget::Prod);

    let mut named_environment = fixture.env();
    named_environment.profile = Some("public-289".into());
    let selected = ProfileOptions::default()
        .resolve_with_env(Some(274), &named_environment)
        .unwrap();
    assert_eq!(selected.revision(), ClientRevision::R289);
    assert_eq!(selected.target(), BotTarget::Prod);

    let mut prod_environment = fixture.env();
    prod_environment.target = Some("prod".into());
    let selected = ProfileOptions::default()
        .resolve_with_env(Some(274), &prod_environment)
        .unwrap();
    assert_eq!(selected.revision(), ClientRevision::R289);
    assert_eq!(selected.target(), BotTarget::Prod);

    let local_default = ProfileOptions::default()
        .resolve_with_env(None, &fixture.env())
        .unwrap();
    assert_eq!(local_default.revision(), ClientRevision::R274);
    assert_eq!(local_default.target(), BotTarget::Local);
    let saved_local_289 = ProfileOptions::default()
        .resolve_with_env(Some(289), &fixture.env())
        .unwrap();
    assert_eq!(saved_local_289.revision(), ClientRevision::R289);
    assert_eq!(saved_local_289.target(), BotTarget::Local);

    let explicit = ProfileOptions {
        profile: Some("public-289".into()),
        cache_dir: Some("explicit-cache".into()),
        unpack_dir: Some("explicit-unpack".into()),
        nav_pack: Some("explicit.navpack".into()),
        nav_flags: Some("explicit.navflags".into()),
        content_dir: Some("explicit-content".into()),
        vault_path: Some("explicit-vault".into()),
        ..ProfileOptions::default()
    };
    let mut explicit_environment = fixture.env();
    explicit_environment.working_dir = Some(fixture.0.clone());
    let selected = explicit
        .resolve_with_env(Some(274), &explicit_environment)
        .unwrap();
    assert_eq!(selected.cache_dir(), fixture.0.join("explicit-cache"));
    assert_eq!(selected.unpack_dir(), fixture.0.join("explicit-unpack"));
    assert_eq!(selected.nav_pack(), fixture.0.join("explicit.navpack"));
    assert_eq!(selected.nav_flags(), fixture.0.join("explicit.navflags"));
    assert_eq!(selected.content_dir(), fixture.0.join("explicit-content"));
    assert_eq!(selected.vault_path(), fixture.0.join("explicit-vault"));
}

#[test]
fn invalid_revision_public_pairing_and_conflicts_fail_before_vault_access() {
    let fixture = Fixture::new();
    for args in [
        vec!["--revision", "377"],
        vec!["--revision"],
        vec!["--port", "0"],
        vec!["--revision", "--prod"],
    ] {
        assert!(parse_profile_args(args).is_err());
    }
    for args in [
        vec!["--prod", "--revision", "274"],
        vec!["--profile", "local-274", "--revision", "289"],
        vec!["--profile", "local-289", "--prod"],
        vec!["--profile", "public-274"],
        vec!["--profile", "public-289", "--host", "localhost"],
        vec!["--profile", "public-289", "--port", "43594"],
        vec!["--profile", "public-289", "--asset-host", "localhost"],
        vec!["--profile", "public-289", "--http-port", "80"],
    ] {
        let (options, _) = parse_profile_args(args).unwrap();
        assert!(options.resolve_with_env(None, &fixture.env()).is_err());
    }
    let mut explicit_env_274 = fixture.env();
    explicit_env_274.revision = Some("274".into());
    let error = ProfileOptions {
        prod: true,
        ..ProfileOptions::default()
    }
    .resolve_with_env(None, &explicit_env_274)
    .unwrap_err();
    assert!(error.contains("public revision 274 is unavailable"));

    let mut public_env_274 = fixture.env();
    public_env_274.target = Some("prod".into());
    let error = ProfileOptions {
        revision: Some("274".into()),
        ..ProfileOptions::default()
    }
    .resolve_with_env(None, &public_env_274)
    .unwrap_err();
    assert!(error.contains("public revision 274 is unavailable"));
    assert!(!fixture.0.join(".274bot").exists());
}

#[test]
fn cache_and_nav_mismatch_are_rejected_before_creating_resources() {
    let fixture = Fixture::new();
    let mut options = fixture.options(289);
    options.cache_manifest = Some(fixture.0.join("manifest-274.json"));
    let selection = options.resolve_with_env(None, &fixture.env()).unwrap();
    assert!(selection
        .bind()
        .unwrap_err()
        .contains("cache/profile mismatch"));
    options.cache_manifest = Some(fixture.0.join("manifest-289.json"));
    let nav = fixture.0.join("wrong.navpack");
    std::fs::write(&nav, b"fixture-nav").unwrap();
    options.nav_pack = Some(nav.clone());
    let selection = options.resolve_with_env(None, &fixture.env()).unwrap();
    assert!(selection
        .bind()
        .unwrap_err()
        .contains("navigation/profile mismatch"));
    let manifest = NavManifest {
        revision: 274,
        cache_id: "wrong".into(),
        nav_sha256: "wrong".into(),
        flags_sha256: None,
    };
    std::fs::write(
        host_play::profile::nav_manifest_path(&nav),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert!(selection
        .bind()
        .unwrap_err()
        .contains("navigation/profile mismatch"));
    assert!(!selection.vault_path().exists());
    assert!(!selection.unpack_dir().exists());
}

#[test]
fn invalid_explicit_rsa_fails_without_fallback_and_binding_detects_resource_changes() {
    let fixture = Fixture::new();
    let options = fixture.options(274);
    for modulus in ["", "not-a-number", "0", "-23"] {
        let mut environment = fixture.env();
        environment.rsa_modulus = Some(modulus.into());
        assert!(options
            .resolve_with_env(None, &environment)
            .unwrap()
            .bind()
            .is_err());
    }
    let profile = options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap();
    assert!(matches!(
        profile.nav_availability(),
        NavAvailability::Unavailable(_)
    ));
    profile.validate_resources().unwrap();
    std::fs::write(fixture.0.join("config"), b"changed after binding").unwrap();
    assert!(profile
        .validate_resources()
        .unwrap_err()
        .contains("cache changed"));
    assert!(!profile.vault_path().exists());
}

#[test]
fn both_revisions_reach_real_shared_client_constructor_and_keep_the_binding() {
    let _clients = CLIENTS.lock().unwrap();
    for revision in [274, 289] {
        let fixture = Fixture::new();
        let options = fixture.options(revision);
        let profile = options
            .resolve_with_env(None, &fixture.env())
            .unwrap()
            .bind()
            .unwrap();
        let template = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
        let mut first = template.prepare_client(740_001, false).unwrap();
        let second = template.prepare_client(740_002, true).unwrap();
        assert_eq!(first.revision().as_i32(), i32::from(revision));
        assert_eq!(first.login_uid, 740_001);
        assert_eq!(second.login_uid, 740_002);
        assert!(Arc::ptr_eq(
            first.session_profile().unwrap(),
            profile.client()
        ));
        assert!(Arc::ptr_eq(
            first.session_profile().unwrap(),
            second.session_profile().unwrap()
        ));
        assert!(Arc::ptr_eq(&first.cache, &second.cache));
        assert!(Arc::ptr_eq(&first.ifaces, &second.ifaces));
        assert_eq!(first.cache.objs[0].name, "Fixture obj");
        assert_eq!(first.session_target(), BotTarget::Local);
        first.config.host = "invalid.invalid".into();
        first.config.port = 1;
        first.config.cache_dir = "invalid-cache".into();
        assert_eq!(first.session_profile().unwrap().game_host(), "127.0.0.1");
        assert_eq!(
            first.session_profile().unwrap().cache_dir(),
            fixture.0.as_path()
        );
        assert_eq!(template.scatter_tile_for(42), template.scatter_tile_for(42));
    }
}

#[test]
fn qualified_revision_289_accepts_an_unarmed_slot_and_script_loading() {
    let _clients = CLIENTS.lock().unwrap();
    let fixture = Fixture::new();
    let profile = fixture
        .options(289)
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap();
    let template = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    let account = vault::Profile {
        username: "fixture".into(),
        password: "fixture".into(),
        uid: 42,
        settings: Default::default(),
    };
    let mut play = host_play::run_with_template(
        Arc::clone(&template),
        false,
        vec![],
        |_| (None, None),
        |_, _, _| {},
    )
    .unwrap();
    // An unarmed slot exercises the real spawn path without a login handshake.
    play.try_spawn_slot(
        account.clone(),
        None,
        None,
        Some(host_play::SlotArm::new(account.uid, false)),
    )
    .unwrap();
    assert!(play.arm("fixture").is_some());
    assert!(play.login_queue_uids().is_empty());
    // The loader/start handle accepts the user's script under revision 289.
    // Runtime actions remain subject to the actual host/client capabilities.
    play.script_start_handle()
        .start_load(
            "fixture",
            "export function tick(api) {}".into(),
            script::LoadShape::NativeTick,
            None,
            vec![],
        )
        .unwrap();
    play.script_stop("fixture");
    play.stop_slot("fixture");
    assert!(play.arm("fixture").is_none());
    assert!(!profile.vault_path().exists());
}

#[test]
fn bound_public_client_refuses_fixture_cheats_after_mutable_config_changes() {
    let _clients = CLIENTS.lock().unwrap();
    let fixture = Fixture::new();
    let mut options = fixture.options(289);
    options.prod = true;
    let profile = options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap();
    assert_eq!(profile.target(), BotTarget::Prod);
    assert_eq!(profile.revision(), ClientRevision::R289);
    let template = SharedClientTemplate::load(profile).unwrap();
    let mut client = template.prepare_client(740_003, true).unwrap();
    client.config.host = "127.0.0.1".into();
    client.config.port = 43594;
    let position = client.out.pos;
    assert!(!api::interact::cheat(&mut client, "ping"));
    api::interact::mainland_hop(&mut client);
    assert_eq!(client.out.pos, position);
}

#[test]
fn catalog_is_a_default_path_while_cache_identity_remains_revision_bound() {
    let fixture = Fixture::new();
    let root = fixture.0.join("catalog");
    std::fs::create_dir_all(root.join("src/bot")).unwrap();
    std::fs::write(root.join("src/bot/Fixture.js"), "export default {};").unwrap();
    let mut options = fixture.options(274);
    options.catalog_root = Some(root.clone());
    let profile = options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap();
    assert_eq!(profile.catalog_root(), Some(root.as_path()));
    std::fs::write(
        root.join("src/bot/Fixture.js"),
        "export default { changed: true };",
    )
    .unwrap();
    profile.validate_resources().unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    // Binding a server neither reads nor qualifies the user's script tree.
    options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap();
    let first = CacheManifest::capture(274, &fixture.0).unwrap();
    let second = CacheManifest::capture(289, &fixture.0).unwrap();
    assert_ne!(first.identity(), second.identity());
}

#[test]
fn navigation_and_scatter_use_the_selected_shared_world_and_validate_its_sidecar() {
    use api::snapshot::WorldTile;
    use nav::collision::{pack_walk, WorldCollision};
    use nav::transport::{TransportEdge, TransportGraph, TransportKind};
    use sha2::{Digest, Sha256};
    let fixture = Fixture::new();
    let mut options = fixture.options(289);
    let origin = WorldTile {
        x: 4500,
        z: 4600,
        level: 0,
    };
    let adjacent = WorldTile { x: 4501, ..origin };
    let (walk, blocked) = pack_walk(&[0; 8]);
    let collision = WorldCollision {
        origin,
        width: 2,
        height: 1,
        walk,
        blocked,
        flags: None,
    };
    let mut graph = TransportGraph::default();
    graph.edges.push(TransportEdge {
        kind: TransportKind::Door,
        at: origin,
        to: adjacent,
        loc_id: 1,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
    });
    let bytes = nav::pack::encode(&collision, &graph, &[]);
    let flags = nav::pack::encode_flags_sidecar(origin, 2, 1, &[0; 8]);
    let pack = fixture.0.join("selected.navpack");
    let flags_path = fixture.0.join("selected.navflags");
    std::fs::write(&pack, &bytes).unwrap();
    std::fs::write(&flags_path, &flags).unwrap();
    let manifest = NavManifest {
        revision: 289,
        cache_id: CacheManifest::capture(289, &fixture.0).unwrap().identity(),
        nav_sha256: format!("{:x}", Sha256::digest(&bytes)),
        flags_sha256: Some(format!("{:x}", Sha256::digest(&flags))),
    };
    std::fs::write(
        host_play::profile::nav_manifest_path(&pack),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    options.nav_pack = Some(pack.clone());
    options.nav_flags = Some(flags_path.clone());
    let profile = options
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap();
    assert_eq!(profile.nav_availability(), &NavAvailability::Bound);
    let template = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    let first = template.world().unwrap();
    let second = template.world().unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(first.collision.origin, origin);
    for uid in [0, 1, -5, i32::MIN] {
        assert!([origin, adjacent].contains(&template.scatter_tile_for(uid)));
    }
    std::fs::write(flags_path, b"different flags").unwrap();
    let error = match template.validate_for_play() {
        Ok(_) => panic!("changed flags must fail final validation"),
        Err(error) => error,
    };
    assert!(error.contains("flags changed"));
    std::fs::write(options.nav_flags.unwrap(), &flags).unwrap();
    template.validate_for_play().unwrap();
    std::fs::write(pack, b"different navigation").unwrap();
    let error = match template.validate_for_play() {
        Ok(_) => panic!("changed navigation must fail final validation"),
        Err(error) => error,
    };
    assert!(error.contains("navigation changed"));
}

#[test]
fn checked_play_entry_revalidates_while_a_consuming_ticket_does_not_hash_again() {
    let fixture = Fixture::new();
    let profile = fixture
        .options(274)
        .resolve_with_env(None, &fixture.env())
        .unwrap()
        .bind()
        .unwrap();
    let template = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    let config = fixture.0.join("config");
    let original = std::fs::read(&config).unwrap();

    std::fs::write(&config, b"changed before checked play").unwrap();
    let error = match host_play::run_with_template(
        Arc::clone(&template),
        false,
        vec![],
        |_| (None, None),
        |_, _, _| {},
    ) {
        Ok(_) => panic!("checked play must refuse a changed cache"),
        Err(error) => error,
    };
    assert!(error.contains("cache changed"));

    std::fs::write(&config, &original).unwrap();
    let ticket = template.validate_for_play().unwrap();
    std::fs::write(&config, b"changed after the final validation").unwrap();
    let play =
        host_play::run_prepared_template(ticket, false, vec![], |_| (None, None), |_, _, _| {})
            .unwrap();
    assert!(Arc::ptr_eq(play.server_profile().unwrap(), &profile));
}
