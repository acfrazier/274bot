use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use nav::collision::WorldCollision;
use nav::map::identity::Digest;
use nav::tile::Tile;
use nav::world::NavWorld;

use super::map_host;
use map_host::profile::{CacheManifest, NavManifest, ProfileEnvironment};
use map_host::{Play, ProfileOptions, SharedClientTemplate, SlotArm, SlotStatus};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A real checked profile/nav binding, without fetching assets or starting slots.
/// Shared by host command and panel adapter tests.
pub struct MapFixture {
    pub root: PathBuf,
    pub template: Arc<SharedClientTemplate>,
}

impl MapFixture {
    pub fn new(world: &NavWorld, profile: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "274bot-map-actions-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
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
            std::fs::copy(source.join(name), root.join(name)).unwrap();
        }
        // Older routing fixtures contain only their exercised plane. The wire
        // requires four; absent fixture cells are blocked, not fabricated land.
        let cells = 4 * world.collision.width * world.collision.height;
        let supplied = world.collision.walk.len();
        let mut padded;
        let collision = if supplied < cells {
            padded = WorldCollision {
                origin: world.collision.origin,
                width: world.collision.width,
                height: world.collision.height,
                walk: world.collision.walk.clone(),
                blocked: world.collision.blocked.clone(),
                flags: None,
            };
            padded.walk.resize(cells, u8::MAX);
            padded.blocked.resize(cells.div_ceil(64), u64::MAX);
            for i in supplied..cells {
                padded.blocked[i / 64] |= 1 << (i % 64);
            }
            &padded
        } else {
            &world.collision
        };
        let bytes = nav::pack::encode(collision, &world.graph, world.banks());
        let pack = root.join("selected.navpack");
        std::fs::write(&pack, &bytes).unwrap();
        let manifest = NavManifest {
            revision: 289,
            cache_id: CacheManifest::capture(289, &root).unwrap().identity(),
            content_id: None,
            source_sha256: None,
            nav_sha256: Digest::of(&bytes).to_string(),
            flags_sha256: None,
            reach_sha256: None,
            canlight_sha256: None,
        };
        std::fs::write(
            map_host::profile::nav_manifest_path(&pack),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let profile = ProfileOptions {
            profile: Some(profile.into()),
            revision: Some("289".into()),
            cache_dir: Some(root.clone()),
            cache_manifest: Some(root.join("manifest-289.json")),
            nav_pack: Some(pack),
            ..Default::default()
        }
        .resolve_with_env(
            None,
            &ProfileEnvironment {
                home: Some(root.clone()),
                rsa_modulus: Some(client::JAVA_LOGIN_RSAN.into()),
                rsa_exponent: Some(client::JAVA_LOGIN_RSAE.into()),
                ..Default::default()
            },
        )
        .unwrap()
        .bind()
        .unwrap();
        let template = SharedClientTemplate::load(profile).unwrap();
        Self { root, template }
    }

    pub fn play(&self, origin: Tile) -> Play {
        let mut play = map_host::run_prepared_template(
            self.template.validate_for_play().unwrap(),
            false,
            vec![],
            |_| (None, None),
            |_, _, _| {},
        )
        .unwrap();
        play.attach_arm("alice", SlotArm::new(1, false));
        play.focus("alice");
        play.statuses.lock().unwrap().push(SlotStatus {
            username: "alice".into(),
            ingame: true,
            tile_x: origin.x,
            tile_z: origin.z,
            tile_level: origin.level,
            ..Default::default()
        });
        play
    }
}

impl Drop for MapFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
