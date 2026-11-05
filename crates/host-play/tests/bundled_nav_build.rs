//! Real build output: the staged revision-keyed navigation payloads decode and
//! the host binds the staged pack through the ordinary resource-root path.
//!
//!   cargo test -p host-play --test bundled_nav_build -- --ignored --nocapture
//!
//! Prerequisite: a default application build that stages local navigation
//! payloads. The test reads that build's resource directory.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use host_play::cache::CacheAvailability;
use host_play::profile::{NavAvailability, ProfileEnvironment, ProfileOptions};
use host_play::{nav_assets, ProfileProgressObserver, SharedClientTemplate};

static NEXT: AtomicUsize = AtomicUsize::new(0);
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

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "274bot-bundled-nav-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("temporary fixture root");
        Self(path)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn revision() -> u16 {
    match std::env::var("BOT_NAV_REVISION").as_deref() {
        Ok("274") => 274,
        Ok("289") | Err(_) => 289,
        Ok(other) => panic!("BOT_NAV_REVISION must be 274 or 289, got {other:?}"),
    }
}

fn resource_root() -> PathBuf {
    std::env::var_os("BOT_NAV_RESOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let exe = std::env::current_exe().expect("test binary path");
            nav_assets::install_resource_root(&exe)
        })
}

#[test]
#[ignore = "requires staged local navigation payloads"]
fn staged_bundle_uses_revision_layout_and_ordinary_decoders() {
    let revision = revision();
    let layout = nav_assets::artifact_layout(revision);
    let resource_root = resource_root();
    let expected_pack = resource_root.join(&layout.relative_pack);
    let description = format!("staged revision-{revision} navigation pack");
    let Some(_) =
        api::test_support::local_asset_path("", &expected_pack.to_string_lossy(), &description)
    else {
        return;
    };
    let resource_root = resource_root.canonicalize().expect("staged resource root");
    let pack_path = resource_root.join(&layout.relative_pack);
    let flags_path = resource_root.join(&layout.relative_flags);
    let reach_path = resource_root.join(&layout.relative_reach);
    let canlight_path = resource_root.join(&layout.relative_canlight);
    let pois_path = resource_root.join(&layout.relative_pois);

    let pack_bytes = std::fs::read(&pack_path).expect("staged navpack");
    let world = nav_planner::world::NavWorld::from_bytes(&pack_bytes).expect("decode navpack");
    let (flags_origin, flags_width, flags_height, flags) = nav_planner::pack::decode_flags_sidecar(
        &std::fs::read(&flags_path).expect("staged navflags"),
    )
    .expect("decode navflags");
    assert_eq!(flags_origin, world.collision.origin);
    assert_eq!(
        (flags_width, flags_height),
        (world.collision.width, world.collision.height)
    );
    assert_eq!(flags.len(), flags_width * flags_height);

    let reach = nav_planner::pack::decode_reach_sidecar(
        &std::fs::read(&reach_path).expect("staged navreach"),
    )
    .expect("decode navreach");
    assert_eq!(reach.origin, world.collision.origin);
    assert_eq!(
        (reach.width, reach.height),
        (world.collision.width, world.collision.height)
    );
    assert_eq!(
        reach.bits,
        nav_planner::paint::bake_reach(&world.collision, &world.graph),
        "staged reach data equals the decoded world bake"
    );

    let canlight = nav_planner::pack::decode_canlight_sidecar(
        &std::fs::read(&canlight_path).expect("staged navcanlight"),
    )
    .expect("decode navcanlight");
    assert_eq!(canlight.origin, world.collision.origin);
    assert_eq!(
        (canlight.width, canlight.height),
        (world.collision.width, world.collision.height)
    );
    let pois_bytes = std::fs::read(&pois_path).expect("staged navpois");
    let pois = nav_planner::map::formats::ServicePois::decode_navpois(&pois_bytes)
        .expect("decode navpois");
    pois.validate().expect("validate navpois");

    let fixture = TempRoot::new();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/profile");
    let cache = fixture.0.join("cache");
    std::fs::create_dir_all(&cache).expect("fixture cache");
    for name in ARCHIVES {
        std::fs::copy(source.join(name), cache.join(name))
            .unwrap_or_else(|error| panic!("copy fixture archive {name}: {error}"));
    }
    let selection = ProfileOptions {
        profile: Some(format!("local-{revision}")),
        revision: Some(revision.to_string()),
        host: Some("127.0.0.1".into()),
        port: Some(9),
        asset_host: Some("127.0.0.1".into()),
        http_port: Some(9),
        cache_dir: Some(cache),
        unpack_dir: Some(fixture.0.join("unpack")),
        ..ProfileOptions::default()
    }
    .resolve_with_env(
        None,
        &ProfileEnvironment {
            home: Some(fixture.0.clone()),
            engine_dir: Some(fixture.0.join("engine")),
            rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
            rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
            ..ProfileEnvironment::default()
        },
    )
    .expect("resolve navigation-only fixture");
    let profile = selection
        .bind_with_resource_root(&ProfileProgressObserver::default(), Some(&resource_root))
        .expect("bind staged navigation");
    assert_eq!(profile.nav_pack(), pack_path.as_path());
    assert_eq!(profile.nav_flags(), flags_path.as_path());
    assert_eq!(profile.nav_availability(), &NavAvailability::Bound);
    assert!(matches!(
        profile.cache_availability(),
        CacheAvailability::Degraded(_)
    ));
    assert_eq!(profile.nav_load_counters().pack_reads, 1);
    assert_eq!(profile.nav_load_counters().pack_decodes, 1);
    let template = SharedClientTemplate::load(profile.clone()).expect("shared template");
    assert!(template.world().is_some(), "staged world decoded");
    assert!(std::sync::Arc::ptr_eq(
        &template.world().unwrap(),
        &profile.world().unwrap()
    ));
}
