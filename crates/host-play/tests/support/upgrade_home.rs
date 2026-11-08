//! An operator home upgraded from 0.1.9.x: the old `274V10` pack still sits
//! at the default `~/.274bot/289/274bot.navpack` while the installed app ships
//! a format-19 bundle under its revision-keyed resource directory. Shared by
//! host-play profile tests and the panel and TUI adapters.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use api::snapshot::WorldTile;
use host_play::profile::ProfileEnvironment;
use host_play::progress::ProfileProgressObserver;
use host_play::{ProfileOptions, ServerProfile};
use nav_planner::collision::{pack_walk, WorldCollision};
use nav_planner::transport::TransportGraph;

static NEXT: AtomicU64 = AtomicU64::new(0);

/// The real 0.1.9.x pack: written by the `274V10` encoder (see
/// `fixtures/nav-v10/README.md`).
pub const V10_PACK: &[u8] = include_bytes!("../fixtures/nav-v10/tiny-274V10.bin");

pub struct UpgradeHome {
    pub root: PathBuf,
    /// The stale user pack at the default home path.
    pub old_pack: PathBuf,
    /// The install resource root holding the packaged revision-keyed bundle.
    pub resources: PathBuf,
}

impl UpgradeHome {
    pub fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "274bot-upgrade-v10-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let cache = root.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../host-play/tests/fixtures/profile");
        for name in [
            "title",
            "config",
            "interface",
            "media",
            "versionlist",
            "textures",
            "wordenc",
            "sounds",
        ] {
            std::fs::copy(source.join(name), cache.join(name)).unwrap();
        }

        // What 0.1.9.x left behind: the old external pack remains untouched.
        let old_pack = root.join("home/.274bot/289/274bot.navpack");
        std::fs::create_dir_all(old_pack.parent().unwrap()).unwrap();
        std::fs::write(&old_pack, V10_PACK).unwrap();

        // What the new install ships: a format-19 world and its data sidecars
        // at the normal revision-keyed resource paths.
        let resources = root.join("Resources");
        let layout = host_play::nav_assets::artifact_layout(289);
        let pack = resources.join(&layout.relative_pack);
        std::fs::create_dir_all(pack.parent().unwrap()).unwrap();
        let origin = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let (walk, blocked) = pack_walk(&[0; 8]);
        let bundle = nav_bake::pack::encode(
            &WorldCollision {
                origin,
                width: 2,
                height: 1,
                walk,
                blocked,
                flags: None,
            },
            &TransportGraph::default(),
            &[],
        )
        .unwrap();
        std::fs::write(&pack, &bundle).unwrap();
        let flags = nav_bake::pack::encode_flags_sidecar(origin, 2, 1, &[0u32; 2]);
        std::fs::write(resources.join(&layout.relative_flags), &flags).unwrap();
        let reach = nav_bake::pack::encode_reach_sidecar(origin, 2, 1, &[0u64]);
        std::fs::write(resources.join(&layout.relative_reach), &reach).unwrap();
        let canlight = nav_bake::pack::encode_canlight_sidecar(origin, 2, 1, &[0u64]);
        std::fs::write(resources.join(&layout.relative_canlight), &canlight).unwrap();
        Self {
            root,
            old_pack,
            resources,
        }
    }

    /// Bind revision 289 against the upgraded home. `nav_pack` is the
    /// `--nav-pack` / `NAV_PACK` override; `None` uses the staged bundle.
    pub fn bind(&self, nav_pack: Option<PathBuf>) -> Arc<ServerProfile> {
        ProfileOptions {
            profile: Some("local-289".into()),
            cache_dir: Some(self.root.join("cache")),
            host: Some("127.0.0.1".into()),
            port: Some(9),
            asset_host: Some("127.0.0.1".into()),
            http_port: Some(9),
            nav_pack,
            ..Default::default()
        }
        .resolve_with_env(
            None,
            &ProfileEnvironment {
                home: Some(self.root.join("home")),
                engine_dir: Some(self.root.join("engine")),
                rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
                rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
                ..Default::default()
            },
        )
        .unwrap()
        .bind_with_resource_root(
            &ProfileProgressObserver::default(),
            Some(self.resources.as_path()),
        )
        .unwrap()
    }

    /// The old pack still holds exactly the bytes 0.1.9.x wrote.
    pub fn old_pack_untouched(&self) -> bool {
        std::fs::read(&self.old_pack).unwrap() == V10_PACK
    }
}

impl Drop for UpgradeHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
