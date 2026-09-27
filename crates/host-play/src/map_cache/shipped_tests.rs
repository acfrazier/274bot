//! Shipped terrain against the real map cache and worker: release packaging
//! ships exactly what a local bake publishes, and a matching home installs it
//! byte for byte without baking; foreign, partial or damaged shipped terrain
//! is never installed or modified; a published user terrain is never replaced.

use super::super::fixture::{
    fixture_descriptor, fixture_png, other_cache_descriptor, ship_fixture_terrain,
    synthetic_client_snapshot, CountingMapProducer,
};
use super::super::{
    ArtifactKind, BakeOutput, BakePlan, BakeRequest, BakeWriter, MapBakeProducer, MapCacheError,
    MapCacheRoot, MapDemand, MapDemandHandle, MapDemandManager, MapJobStatus, MapProfileDescriptor,
    PreparedMapInput,
};
use super::ShippedMapImages;
use crate::map_producer::{map_artifact_policies, NativeMapProducer};
use nav::map::formats::{ImageManifest, PayloadReceipt, ShippedImages, TileReceipt};
use nav::map::identity::Digest;
use nav::map::spatial::TileKey;
use nav::map::Rows;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Scratch directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("274bot-map-shipped-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The production producer, counting the terrain bakes it runs.
#[derive(Default)]
struct CountingNative {
    images: AtomicUsize,
}

impl CountingNative {
    fn image_runs(&self) -> usize {
        self.images.load(Ordering::SeqCst)
    }
}

impl MapBakeProducer for CountingNative {
    fn plan(&self, request: &BakeRequest) -> Result<BakePlan, MapCacheError> {
        NativeMapProducer::new().plan(request)
    }

    fn run(
        &self,
        request: &BakeRequest,
        writer: &mut BakeWriter,
    ) -> Result<BakeOutput, MapCacheError> {
        if request.artifact() == ArtifactKind::Images {
            self.images.fetch_add(1, Ordering::SeqCst);
        }
        NativeMapProducer::new().run(request, writer)
    }
}

fn wait_ready(handle: &MapDemandHandle) {
    let started = Instant::now();
    loop {
        match handle.status() {
            MapJobStatus::Ready => return,
            MapJobStatus::Queued | MapJobStatus::Running(_) => {}
            other => panic!("map demand ended {other:?}"),
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "map demand never settled"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn has_terrain(handle: &MapDemandHandle) -> bool {
    handle.ready().unwrap().images.is_some()
}

/// Every file under `dir` by relative path.
fn tree(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(base).unwrap().to_path_buf(),
                    fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

#[cfg(unix)]
fn file_id(path: &Path) -> u64 {
    std::os::unix::fs::MetadataExt::ino(&fs::metadata(path).unwrap())
}

#[cfg(not(unix))]
fn file_id(path: &Path) -> u64 {
    fs::metadata(path).unwrap().len()
}

/// The fixture terrain's two tile keys, in manifest order.
fn fixture_tiles(shipped: &ShippedMapImages) -> [TileKey; 2] {
    let identity = fixture_descriptor().image_identity();
    let bytes = fs::read(shipped.image_dir(identity).unwrap().join("manifest.json")).unwrap();
    let manifest = ImageManifest::decode(&bytes, identity).unwrap();
    let tiles = manifest.tiles.as_slice();
    [tiles[0].key, tiles[1].key]
}

fn shipped_tile(shipped: &ShippedMapImages, key: TileKey) -> PathBuf {
    shipped
        .image_dir(fixture_descriptor().image_identity())
        .unwrap()
        .join(key.relative_path().unwrap())
}

#[test]
fn shipped_terrain_is_a_local_bake_and_installs_byte_identically_without_baking() {
    let scratch = Scratch::new("identical");
    let (jag, snapshot) = synthetic_client_snapshot(&scratch.0.join("client"));
    let content = Digest([9; 32]);
    let input = || PreparedMapInput::with_snapshot(289, content, jag.clone(), snapshot.clone());
    let (image_policy, catalogue_policy) = map_artifact_policies().unwrap();
    let descriptor = MapProfileDescriptor::new(289, content, image_policy, catalogue_policy)
        .unwrap()
        .with_input(input())
        .unwrap();
    let identity = descriptor.image_identity();

    // Release packaging, from the same client cache.
    let package = scratch.0.join("package");
    let described = crate::map_bind::ship_map_images(input(), &package, |_| {}).unwrap();
    assert_eq!(described.identity, identity);
    assert!(
        !package
            .read_dir()
            .unwrap()
            .any(|entry| entry.unwrap().file_name() != "map"),
        "packaging leaves only the shipped map directory"
    );
    let shipped = ShippedMapImages::at(package.join("map"));

    // A local bake of that client cache, as the app runs it with consent.
    let local = MapCacheRoot::from_root(scratch.0.join("local"));
    let baker = MapDemandManager::new(local.clone(), Arc::new(NativeMapProducer::new()));
    let baked = baker
        .request(descriptor.clone(), MapDemand::Images)
        .unwrap();
    wait_ready(&baked);
    let local_tree = tree(&local.image_dir(identity).unwrap());
    assert!(local_tree.len() > 2, "manifest plus several tiles");
    assert_eq!(
        tree(&shipped.image_dir(identity).unwrap()),
        local_tree,
        "shipped bytes are a local bake's"
    );

    // A fresh home: the adopt-only demand installs the shipped terrain.
    let home = MapCacheRoot::from_root(scratch.0.join("home"));
    let producer = Arc::new(CountingNative::default());
    let manager = MapDemandManager::with_shipped_images(home.clone(), producer.clone(), shipped);
    assert!(manager.shipped_images_available(&descriptor));
    let handle = manager
        .request(descriptor.clone(), MapDemand::ReadyImages)
        .unwrap();
    wait_ready(&handle);
    assert!(has_terrain(&handle), "installed terrain is served");
    assert_eq!(producer.image_runs(), 0, "installed, never baked");
    assert_eq!(
        tree(&home.image_dir(identity).unwrap()),
        local_tree,
        "installed bytes are a local bake's"
    );
}

#[test]
fn terrain_shipped_for_another_client_cache_or_policy_is_ignored_and_left_as_shipped() {
    let scratch = Scratch::new("foreign");
    let shipped = ship_fixture_terrain(&scratch.0);
    let before = tree(shipped.path());
    let producer = Arc::new(CountingMapProducer::default());
    let manager = MapDemandManager::with_shipped_images(
        MapCacheRoot::from_root(scratch.0.join("home")),
        producer.clone(),
        shipped.clone(),
    );
    let fixture = fixture_descriptor();
    let other_cache = other_cache_descriptor();
    let other_policy =
        MapProfileDescriptor::new(289, Digest([7; 32]), Digest([5; 32]), Digest([9; 32])).unwrap();
    assert!(manager.shipped_images_available(&fixture), "control");
    assert!(!manager.shipped_images_available(&other_cache));
    assert!(!manager.shipped_images_available(&other_policy));

    let adopt = manager
        .request(other_cache.clone(), MapDemand::ReadyImages)
        .unwrap();
    wait_ready(&adopt);
    assert!(!has_terrain(&adopt), "nothing to adopt: catalogue-only");
    assert_eq!(producer.image_runs(), 0);
    drop(adopt);
    manager.reap();

    // With consent the local bake runs as before.
    let bake = manager.request(other_cache, MapDemand::Images).unwrap();
    wait_ready(&bake);
    assert!(has_terrain(&bake));
    assert_eq!(producer.image_runs(), 1);
    assert_eq!(
        tree(shipped.path()),
        before,
        "shipped terrain stays as shipped"
    );
}

#[test]
fn a_damaged_shipped_tile_fails_closed_and_a_consented_bake_adopts_the_verified_ones() {
    let scratch = Scratch::new("damaged");
    let shipped = ship_fixture_terrain(&scratch.0);
    let [first, second] = fixture_tiles(&shipped);
    let mut bytes = fs::read(shipped_tile(&shipped, second)).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0x5a;
    fs::write(shipped_tile(&shipped, second), &bytes).unwrap();
    let before = tree(shipped.path());

    let root = MapCacheRoot::from_root(scratch.0.join("home"));
    let producer = Arc::new(CountingMapProducer::default());
    let manager =
        MapDemandManager::with_shipped_images(root.clone(), producer.clone(), shipped.clone());
    let descriptor = fixture_descriptor();
    assert!(
        manager.shipped_images_available(&descriptor),
        "only the description is read up front"
    );
    let adopt = manager
        .request(descriptor.clone(), MapDemand::ReadyImages)
        .unwrap();
    wait_ready(&adopt);
    assert!(!has_terrain(&adopt), "fails closed to catalogue-only");
    assert_eq!(producer.image_runs(), 0, "no bake without consent");
    assert!(
        !manager.shipped_images_available(&descriptor),
        "remembered as unusable"
    );
    assert_eq!(tree(shipped.path()), before, "shipped files are only read");
    let identity = descriptor.image_identity();
    let partial = root.partial_dir(289, ArtifactKind::Images, identity.key().unwrap().0);
    let staged = partial.join(first.relative_path().unwrap());
    let staged_id = file_id(&staged);
    assert!(
        !partial.join(second.relative_path().unwrap()).exists(),
        "the damaged tile is never staged"
    );
    drop(adopt);
    manager.reap();

    // Consent: the local bake adopts the verified shipped tile in place and
    // bakes the rest.
    let bake = manager.request(descriptor, MapDemand::Images).unwrap();
    wait_ready(&bake);
    assert!(has_terrain(&bake));
    assert_eq!(producer.image_runs(), 1);
    let published = root.image_dir(identity).unwrap();
    assert_eq!(
        file_id(&published.join(first.relative_path().unwrap())),
        staged_id,
        "the verified shipped tile was adopted, not rewritten"
    );
    assert_eq!(
        fs::read(published.join(second.relative_path().unwrap())).unwrap(),
        fixture_png(2)
    );
}

#[test]
fn partial_or_corrupt_shipped_files_are_never_installed() {
    type Damage = fn(&ShippedMapImages);
    let cases: [(&str, Damage); 5] = [
        ("truncated-tile", |shipped| {
            let [first, _] = fixture_tiles(shipped);
            let bytes = fs::read(shipped_tile(shipped, first)).unwrap();
            fs::write(shipped_tile(shipped, first), &bytes[..bytes.len() / 2]).unwrap();
        }),
        ("missing-tile", |shipped| {
            let [_, second] = fixture_tiles(shipped);
            fs::remove_file(shipped_tile(shipped, second)).unwrap();
        }),
        ("edited-manifest", |shipped| {
            let path = shipped
                .image_dir(fixture_descriptor().image_identity())
                .unwrap()
                .join("manifest.json");
            let mut bytes = fs::read(&path).unwrap();
            bytes.push(b'\n');
            fs::write(path, bytes).unwrap();
        }),
        ("missing-manifest", |shipped| {
            fs::remove_file(
                shipped
                    .image_dir(fixture_descriptor().image_identity())
                    .unwrap()
                    .join("manifest.json"),
            )
            .unwrap();
        }),
        ("garbled-description", |shipped| {
            fs::write(shipped.description_path(289), b"{\"schema\":").unwrap();
        }),
    ];
    for (name, damage) in cases {
        let scratch = Scratch::new(name);
        let shipped = ship_fixture_terrain(&scratch.0);
        damage(&shipped);
        let root = MapCacheRoot::from_root(scratch.0.join("home"));
        let producer = Arc::new(CountingMapProducer::default());
        let manager =
            MapDemandManager::with_shipped_images(root.clone(), producer.clone(), shipped);
        let handle = manager
            .request(fixture_descriptor(), MapDemand::ReadyImages)
            .unwrap();
        wait_ready(&handle);
        assert!(!has_terrain(&handle), "{name}: catalogue-only");
        assert_eq!(producer.image_runs(), 0, "{name}: no bake");
        assert!(
            !root
                .image_dir(fixture_descriptor().image_identity())
                .unwrap()
                .exists(),
            "{name}: nothing published"
        );
        assert!(
            !manager.shipped_images_available(&fixture_descriptor()),
            "{name}: remembered as unusable"
        );
    }
}

/// Replace the shipped fixture terrain with other valid tiles for the same
/// identity, updating every receipt: a consistent bundle a home would install.
fn reship_with_other_tiles(shipped: &ShippedMapImages) {
    let identity = fixture_descriptor().image_identity();
    let dir = shipped.image_dir(identity).unwrap();
    let mut manifest =
        ImageManifest::decode(&fs::read(dir.join("manifest.json")).unwrap(), identity).unwrap();
    let tiles = manifest
        .tiles
        .as_slice()
        .iter()
        .enumerate()
        .map(|(index, tile)| {
            let png = fixture_png(index as u8 + 5);
            fs::write(dir.join(tile.key.relative_path().unwrap()), &png).unwrap();
            TileReceipt {
                key: tile.key,
                payload: PayloadReceipt {
                    bytes: png.len() as u32,
                    sha256: Digest::of(&png),
                },
            }
        })
        .collect();
    manifest.tiles = Rows::new(tiles).unwrap();
    let bytes = manifest.encode().unwrap();
    fs::write(dir.join("manifest.json"), &bytes).unwrap();
    let description = ShippedImages::describe(&manifest, &bytes).unwrap();
    fs::write(shipped.description_path(289), description.encode().unwrap()).unwrap();
}

#[test]
fn a_published_user_terrain_is_never_replaced_by_shipped_terrain() {
    let scratch = Scratch::new("no-clobber");
    let shipped = ship_fixture_terrain(&scratch.0);
    reship_with_other_tiles(&shipped);
    let [first, _] = fixture_tiles(&shipped);
    let descriptor = fixture_descriptor();
    let identity = descriptor.image_identity();
    let mut buffer = Vec::new();

    // Control: that shipped terrain installs into a fresh home.
    let fresh = MapCacheRoot::from_root(scratch.0.join("fresh"));
    let fresh_manager = MapDemandManager::with_shipped_images(
        fresh,
        Arc::new(CountingMapProducer::default()),
        shipped.clone(),
    );
    let installed = fresh_manager
        .request(descriptor.clone(), MapDemand::ReadyImages)
        .unwrap();
    wait_ready(&installed);
    let images = installed.ready().unwrap().images.unwrap();
    images.read_tile_into(first, &mut buffer).unwrap();
    assert_eq!(buffer, fixture_png(5));

    // The operator's home already holds terrain baked for this identity.
    let root = MapCacheRoot::from_root(scratch.0.join("home"));
    {
        let baker = MapDemandManager::new(root.clone(), Arc::new(CountingMapProducer::default()));
        let baked = baker
            .request(descriptor.clone(), MapDemand::Images)
            .unwrap();
        wait_ready(&baked);
    }
    let user_dir = root.image_dir(identity).unwrap();
    let before = tree(&user_dir);
    let before_ids: Vec<u64> = before
        .keys()
        .map(|path| file_id(&user_dir.join(path)))
        .collect();

    let producer = Arc::new(CountingMapProducer::default());
    let manager = MapDemandManager::with_shipped_images(root.clone(), producer.clone(), shipped);
    let handle = crate::map_bind::open_map_ready_terrain(&manager, descriptor)
        .unwrap()
        .expect("ready terrain opens");
    wait_ready(&handle);
    let images = handle.ready().unwrap().images.unwrap();
    images.read_tile_into(first, &mut buffer).unwrap();
    assert_eq!(
        buffer,
        fixture_png(1),
        "the operator's own terrain is served"
    );
    assert_eq!(tree(&user_dir), before, "user terrain bytes unchanged");
    assert_eq!(
        before
            .keys()
            .map(|path| file_id(&user_dir.join(path)))
            .collect::<Vec<_>>(),
        before_ids,
        "user terrain files never replaced"
    );
    assert!(
        !root
            .partial_dir(289, ArtifactKind::Images, identity.key().unwrap().0)
            .exists(),
        "nothing was staged for install"
    );
    assert_eq!(producer.image_runs(), 0);
}
