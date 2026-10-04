//! An operator home upgraded from 0.1.9.x: the old `274V10` pack (and its
//! sidecar) still sits at the default `~/.274bot/289/274bot.navpack` while the
//! installed app ships a packaged `274V15` bundle for the same cache. Shared by
//! host-play profile tests and the panel and TUI adapters.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use api::snapshot::WorldTile;
use host_play::profile::{CacheManifest, NavManifest, ProfileEnvironment};
use host_play::progress::ProfileProgressObserver;
use host_play::{BundledNavIdentity, ProfileOptions, ServerProfile};
use nav::collision::{pack_walk, WorldCollision};
use nav::map::identity::Digest;
use nav::transport::TransportGraph;

static NEXT: AtomicU64 = AtomicU64::new(0);

/// The real 0.1.9.x pack: written by the `274V10` encoder (see
/// `fixtures/nav-v10/README.md`).
pub const V10_PACK: &[u8] = include_bytes!("../fixtures/nav-v10/tiny-274V10.bin");

pub struct UpgradeHome {
    pub root: PathBuf,
    /// The stale user pack at the default home path.
    pub old_pack: PathBuf,
    /// The install resource root holding the packaged v15 bundle.
    pub resources: PathBuf,
    /// The compiled bundled identity row for that bundle.
    pub table: [BundledNavIdentity; 1],
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
            "manifest-289.json",
        ] {
            std::fs::copy(source.join(name), cache.join(name)).unwrap();
        }
        let cache_id = CacheManifest::capture(289, &cache).unwrap().identity();

        // What 0.1.9.x left behind: the v10 pack and its matching sidecar.
        let old_pack = root.join("home/.274bot/289/274bot.navpack");
        std::fs::create_dir_all(old_pack.parent().unwrap()).unwrap();
        std::fs::write(&old_pack, V10_PACK).unwrap();
        std::fs::write(
            host_play::profile::nav_manifest_path(&old_pack),
            serde_json::to_vec(&NavManifest {
                revision: 289,
                cache_id: cache_id.clone(),
                content_id: None,
                source_sha256: None,
                nav_sha256: Digest::of(V10_PACK).to_string(),
                flags_sha256: None,
                reach_sha256: None,
                canlight_sha256: None,
                pois_sha256: None,
                zone_count: 0,
                zone_npc_count: 0,
            })
            .unwrap(),
        )
        .unwrap();

        // What the new install ships: the same world as a v15 bundle.
        let resources = root.join("Resources");
        std::fs::create_dir_all(&resources).unwrap();
        let origin = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let (walk, blocked) = pack_walk(&[0; 8]);
        let bundle = nav::pack::encode(
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
        std::fs::write(resources.join("274bot.navpack"), &bundle).unwrap();
        let bundle_digest = Digest::of(&bundle).0;
        let reach = nav::pack::encode_reach_sidecar(origin, 2, 1, &[0u64], &bundle_digest);
        std::fs::write(resources.join("274bot.navreach"), &reach).unwrap();
        let policy = nav::canlight::identity_digest(289, b"fixture-bank-zones", &[0u64]);
        let canlight = nav::pack::encode_canlight_sidecar(
            origin,
            2,
            1,
            &[0u64],
            &nav::canlight::header_binding(&bundle_digest, &policy),
        );
        std::fs::write(resources.join("274bot.navcanlight"), &canlight).unwrap();
        let table = [BundledNavIdentity {
            revision: 289,
            cache_id,
            content_id: None,
            source_sha256: None,
            format: nav::pack::FORMAT_ID.into(),
            nav_sha256: nav::manifest::hash_bytes(&bundle),
            flags_sha256: None,
            reach_sha256: Some(nav::manifest::hash_bytes(&reach)),
            canlight_sha256: Some(nav::manifest::hash_bytes(&canlight)),
            canlight_identity: Some(nav::pack::sha256_hex(&policy)),
            pois_sha256: None,
            relative_path: "274bot.navpack".into(),
        }];
        Self {
            root,
            old_pack,
            resources,
            table,
        }
    }

    /// Bind revision 289 against the upgraded home. `nav_pack` is the
    /// `--nav-pack` / `NAV_PACK` override; `None` is the default launch.
    pub fn bind(&self, nav_pack: Option<PathBuf>) -> Arc<ServerProfile> {
        ProfileOptions {
            profile: Some("local-289".into()),
            cache_dir: Some(self.root.join("cache")),
            cache_manifest: Some(self.root.join("cache/manifest-289.json")),
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
        .bind_with_nav_identities(
            &ProfileProgressObserver::default(),
            &self.table,
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
