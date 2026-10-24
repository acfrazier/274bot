//! Shipped map data against the real map cache and worker: release packaging
//! ships exactly what a local bake or derive publishes, and a matching home
//! installs it byte for byte without baking or deriving; foreign, partial or
//! damaged shipped data is never installed or modified (the local derive or
//! bake runs instead); a published user artifact is never replaced.

use super::super::fixture::{
    fixture_descriptor, fixture_png, other_cache_descriptor, ship_fixture_catalogue,
    ship_fixture_terrain, synthetic_client_snapshot, wait_for_status, CountingMapProducer,
};
use super::super::{
    map_bake_test_step_snapshot, ArtifactKind, BakeRequest, MapBakeCursor, MapBakeProducer,
    MapCacheError, MapCacheRoot, MapDemand, MapDemandHandle, MapDemandManager, MapJobStatus,
    MapProfileDescriptor, MapStage, PreparedMapInput,
};
use super::ShippedMap;
use crate::map_producer::{map_artifact_policies, NativeMapProducer};
use nav_planner::map::formats::{ImageManifest, PayloadReceipt, ShippedImages, TileReceipt};
use nav_planner::map::identity::Digest;
use nav_planner::map::spatial::TileKey;
use nav_planner::map::Rows;
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
/// The production producer, counting the catalogue derives and terrain
/// bakes it runs.
#[derive(Default)]
struct CountingNative {
    catalogues: AtomicUsize,
    images: AtomicUsize,
}

impl CountingNative {
    fn catalogue_runs(&self) -> usize {
        self.catalogues.load(Ordering::SeqCst)
    }

    fn image_runs(&self) -> usize {
        self.images.load(Ordering::SeqCst)
    }
}

impl MapBakeProducer for CountingNative {
    fn charge(&self, request: &BakeRequest) -> Result<api::work::WorkCharge, MapCacheError> {
        NativeMapProducer::new().charge(request)
    }

    fn cursor(
        &self,
        request: &BakeRequest,
        partial_dir: Arc<PathBuf>,
    ) -> Result<Box<dyn MapBakeCursor>, MapCacheError> {
        match request.artifact() {
            ArtifactKind::Catalogue => {
                self.catalogues.fetch_add(1, Ordering::SeqCst);
            }
            ArtifactKind::Images => {
                self.images.fetch_add(1, Ordering::SeqCst);
            }
        }
        NativeMapProducer::new().cursor(request, partial_dir)
    }
}

fn wait_ready(handle: &MapDemandHandle) {
    wait_for_status(handle, MapJobStatus::Ready);
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
fn fixture_tiles(shipped: &ShippedMap) -> [TileKey; 2] {
    let identity = fixture_descriptor().image_identity();
    let bytes = fs::read(shipped.image_dir(identity).unwrap().join("manifest.json")).unwrap();
    let manifest = ImageManifest::decode(&bytes, identity).unwrap();
    let tiles = manifest.tiles.as_slice();
    [tiles[0].key, tiles[1].key]
}

fn shipped_tile(shipped: &ShippedMap, key: TileKey) -> PathBuf {
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
    let described = crate::map_bind::ship_map_bundle(input(), &package, |_| {}).unwrap();
    assert_eq!(described.images.identity, identity);
    assert_eq!(
        described.catalogue.identity,
        descriptor.catalogue_identity()
    );
    assert!(
        !package
            .read_dir()
            .unwrap()
            .any(|entry| entry.unwrap().file_name() != "map"),
        "packaging leaves only the shipped map directory"
    );
    let shipped = ShippedMap::at(package.join("map"));

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
    let catalogue_identity = descriptor.catalogue_identity();
    let local_catalogue_tree = tree(&local.catalogue_dir(catalogue_identity).unwrap());
    assert_eq!(
        local_catalogue_tree.len(),
        2,
        "manifest plus the catalogue payload"
    );
    assert_eq!(
        tree(&shipped.catalogue_dir(catalogue_identity).unwrap()),
        local_catalogue_tree,
        "shipped catalogue bytes are a local derive's"
    );

    // A fresh home: the adopt-only demand installs the shipped terrain and
    // the shipped catalogue.
    let home = MapCacheRoot::from_root(scratch.0.join("home"));
    let producer = Arc::new(CountingNative::default());
    let manager = MapDemandManager::with_shipped_map(home.clone(), producer.clone(), shipped);
    assert!(manager.shipped_images_available(&descriptor));
    let handle = manager
        .request(descriptor.clone(), MapDemand::ReadyImages)
        .unwrap();
    wait_ready(&handle);
    assert!(has_terrain(&handle), "installed terrain is served");
    assert_eq!(producer.image_runs(), 0, "installed, never baked");
    assert_eq!(producer.catalogue_runs(), 0, "installed, never derived");
    assert_eq!(
        tree(&home.image_dir(identity).unwrap()),
        local_tree,
        "installed bytes are a local bake's"
    );
    assert_eq!(
        tree(&home.catalogue_dir(catalogue_identity).unwrap()),
        local_catalogue_tree,
        "installed catalogue bytes are a local derive's"
    );
}

#[test]
fn missing_terrain_probe_leaves_the_cached_catalogue_tree_unchanged() {
    let scratch = Scratch::new("read-only-probe");
    let root = MapCacheRoot::from_root(scratch.0.join("home"));
    let producer = Arc::new(CountingMapProducer::default());
    let manager = MapDemandManager::new(root.clone(), producer.clone());
    let descriptor = fixture_descriptor();
    let catalogue = manager
        .request(descriptor.clone(), MapDemand::CatalogueOnly)
        .unwrap();
    wait_ready(&catalogue);
    let before = tree(root.path());
    let probe = manager
        .request(descriptor.clone(), MapDemand::ReadyImages)
        .unwrap();
    wait_ready(&probe);
    assert!(!has_terrain(&probe));
    assert_eq!(producer.image_runs(), 0);
    assert_eq!(
        tree(root.path()),
        before,
        "the probe must not mutate the cache"
    );
    assert!(!root
        .image_dir(descriptor.image_identity())
        .unwrap()
        .exists());
}

#[test]
fn terrain_shipped_for_another_client_cache_or_policy_is_ignored_and_left_as_shipped() {
    let scratch = Scratch::new("foreign");
    let shipped = ship_fixture_terrain(&scratch.0);
    let before = tree(shipped.path());
    let producer = Arc::new(CountingMapProducer::default());
    let manager = MapDemandManager::with_shipped_map(
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
        MapDemandManager::with_shipped_map(root.clone(), producer.clone(), shipped.clone());
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
    type Damage = fn(&ShippedMap);
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
            fs::write(shipped.images_description_path(289), b"{\"schema\":").unwrap();
        }),
    ];
    for (name, damage) in cases {
        let scratch = Scratch::new(name);
        let shipped = ship_fixture_terrain(&scratch.0);
        damage(&shipped);
        let root = MapCacheRoot::from_root(scratch.0.join("home"));
        let producer = Arc::new(CountingMapProducer::default());
        let manager = MapDemandManager::with_shipped_map(root.clone(), producer.clone(), shipped);
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
fn reship_with_other_tiles(shipped: &ShippedMap) {
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
    fs::write(
        shipped.images_description_path(289),
        description.encode().unwrap(),
    )
    .unwrap();
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
    let fresh_manager = MapDemandManager::with_shipped_map(
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
    let manager = MapDemandManager::with_shipped_map(root.clone(), producer.clone(), shipped);
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

fn shipped_catalogue_file(shipped: &ShippedMap, name: &str) -> PathBuf {
    shipped
        .catalogue_dir(fixture_descriptor().catalogue_identity())
        .unwrap()
        .join(name)
}

#[test]
fn shipped_catalogue_installs_byte_identically_without_deriving() {
    let scratch = Scratch::new("catalogue-identical");
    let shipped = ship_fixture_catalogue(&scratch.0);
    let descriptor = fixture_descriptor();
    let identity = descriptor.catalogue_identity();

    // A local derive of the same client cache.
    let local = MapCacheRoot::from_root(scratch.0.join("local"));
    let deriver = MapDemandManager::new(local.clone(), Arc::new(CountingMapProducer::default()));
    let derived = deriver
        .request(descriptor.clone(), MapDemand::CatalogueOnly)
        .unwrap();
    wait_ready(&derived);
    let local_tree = tree(&local.catalogue_dir(identity).unwrap());
    assert_eq!(
        tree(&shipped.catalogue_dir(identity).unwrap()),
        local_tree,
        "shipped bytes are a local derive's"
    );

    // A fresh home installs the shipped catalogue instead of deriving it.
    let home = MapCacheRoot::from_root(scratch.0.join("home"));
    let producer = Arc::new(CountingMapProducer::default());
    let manager = MapDemandManager::with_shipped_map(home.clone(), producer.clone(), shipped);
    let handle = manager
        .request(descriptor.clone(), MapDemand::CatalogueOnly)
        .unwrap();
    wait_ready(&handle);
    assert_eq!(
        handle.ready().unwrap().catalogue.manifest().identity,
        identity,
        "installed catalogue is served"
    );
    assert_eq!(producer.catalogue_runs(), 0, "installed, never derived");
    assert_eq!(
        tree(&home.catalogue_dir(identity).unwrap()),
        local_tree,
        "installed bytes are a local derive's"
    );
}

#[test]
fn catalogue_shipped_for_another_client_cache_or_policy_is_ignored_and_left_as_shipped() {
    let scratch = Scratch::new("catalogue-foreign");
    let shipped = ship_fixture_catalogue(&scratch.0);
    let before = tree(shipped.path());
    let other_policy =
        MapProfileDescriptor::new(289, Digest([7; 32]), Digest([8; 32]), Digest([5; 32])).unwrap();
    for (name, descriptor) in [
        ("other-cache", other_cache_descriptor()),
        ("other-policy", other_policy),
    ] {
        let producer = Arc::new(CountingMapProducer::default());
        let manager = MapDemandManager::with_shipped_map(
            MapCacheRoot::from_root(scratch.0.join(name)),
            producer.clone(),
            shipped.clone(),
        );
        let handle = manager
            .request(descriptor.clone(), MapDemand::CatalogueOnly)
            .unwrap();
        wait_ready(&handle);
        assert_eq!(
            handle.ready().unwrap().catalogue.manifest().identity,
            descriptor.catalogue_identity(),
            "{name}: the local derive serves its own identity"
        );
        assert_eq!(producer.catalogue_runs(), 1, "{name}: derived locally");
    }
    assert_eq!(
        tree(shipped.path()),
        before,
        "shipped catalogue stays as shipped"
    );
}

#[test]
fn a_damaged_shipped_catalogue_falls_back_to_the_local_derive() {
    type Damage = fn(&ShippedMap);
    let cases: [(&str, Damage); 6] = [
        ("catalogue-flipped-payload", |shipped| {
            let path = shipped_catalogue_file(shipped, "client-pois.json");
            let mut bytes = fs::read(&path).unwrap();
            let middle = bytes.len() / 2;
            bytes[middle] ^= 0x5a;
            fs::write(path, bytes).unwrap();
        }),
        ("catalogue-truncated-payload", |shipped| {
            let path = shipped_catalogue_file(shipped, "client-pois.json");
            let bytes = fs::read(&path).unwrap();
            fs::write(path, &bytes[..bytes.len() / 2]).unwrap();
        }),
        ("catalogue-missing-payload", |shipped| {
            fs::remove_file(shipped_catalogue_file(shipped, "client-pois.json")).unwrap();
        }),
        ("catalogue-edited-manifest", |shipped| {
            let path = shipped_catalogue_file(shipped, "manifest.json");
            let mut bytes = fs::read(&path).unwrap();
            bytes.push(b'\n');
            fs::write(path, bytes).unwrap();
        }),
        ("catalogue-missing-manifest", |shipped| {
            fs::remove_file(shipped_catalogue_file(shipped, "manifest.json")).unwrap();
        }),
        ("catalogue-garbled-description", |shipped| {
            fs::write(shipped.catalogue_description_path(289), b"{\"schema\":").unwrap();
        }),
    ];
    for (name, damage) in cases {
        let scratch = Scratch::new(name);
        let shipped = ship_fixture_catalogue(&scratch.0);
        let descriptor = fixture_descriptor();
        let identity = descriptor.catalogue_identity();
        let expected = tree(&shipped.catalogue_dir(identity).unwrap());
        damage(&shipped);
        let damaged = tree(shipped.path());
        let root = MapCacheRoot::from_root(scratch.0.join("home"));
        let producer = Arc::new(CountingMapProducer::default());
        let manager =
            MapDemandManager::with_shipped_map(root.clone(), producer.clone(), shipped.clone());
        let handle = manager
            .request(descriptor, MapDemand::CatalogueOnly)
            .unwrap();
        wait_ready(&handle);
        assert_eq!(
            producer.catalogue_runs(),
            1,
            "{name}: the local derive runs instead"
        );
        assert_eq!(
            tree(&root.catalogue_dir(identity).unwrap()),
            expected,
            "{name}: the local derive publishes the same catalogue"
        );
        assert_eq!(
            tree(shipped.path()),
            damaged,
            "{name}: shipped files are only read"
        );
    }
}

#[test]
fn a_manager_without_a_shipped_source_ignores_shipped_files() {
    // A dev build wires no shipped source into its manager (nothing ships
    // beside a worktree binary), so shipped files on disk change nothing:
    // the catalogue is derived locally as before.
    let scratch = Scratch::new("catalogue-dev-build");
    let shipped = ship_fixture_catalogue(&scratch.0);
    let before = tree(shipped.path());
    let producer = Arc::new(CountingMapProducer::default());
    let manager = MapDemandManager::new(
        MapCacheRoot::from_root(scratch.0.join("home")),
        producer.clone(),
    );
    let handle = manager
        .request(fixture_descriptor(), MapDemand::CatalogueOnly)
        .unwrap();
    wait_ready(&handle);
    assert_eq!(producer.catalogue_runs(), 1, "derived locally");
    assert_eq!(tree(shipped.path()), before, "shipped files untouched");
}
/// Catalogue-only real-content capture through the host API.
/// No shipped terrain is supplied by this fixture, so a `ReadyImages` demand
/// against the fresh cache root settles catalogue-only without baking terrain.
/// The result isolates catalogue generation, not shipped-terrain installation.
fn catalogue_only_capture(
    capture: &Path,
    cache_root: &MapCacheRoot,
    descriptor: &MapProfileDescriptor,
    revision: u16,
    cache_id: &str,
    content_id: &str,
    navpack_sha256: &str,
) {
    let manager = MapDemandManager::new(cache_root.clone(), Arc::new(NativeMapProducer::new()));
    let metrics_start = map_bake_test_step_snapshot();
    let request_started = Instant::now();
    let handle = manager
        .request(descriptor.clone(), MapDemand::ReadyImages)
        .unwrap();
    loop {
        match handle.status() {
            MapJobStatus::Ready => break,
            MapJobStatus::Queued | MapJobStatus::Running(_) => {
                assert!(
                    request_started.elapsed() < Duration::from_secs(60 * 60),
                    "real-content catalogue demand never settled"
                );
                handle.wait_for_change(Duration::from_millis(1));
            }
            other => panic!("real-content catalogue demand ended {other:?}"),
        }
    }
    let catalogue_request_seconds = request_started.elapsed().as_secs_f64();
    let metrics_end = map_bake_test_step_snapshot();
    let ready = handle.ready().unwrap();
    assert!(
        ready.images.is_none(),
        "catalogue-only demand must not publish terrain"
    );
    let catalogue_dir = cache_root
        .catalogue_dir(descriptor.catalogue_identity())
        .unwrap();
    let capture_catalogue = capture.join("catalogue");
    fs::create_dir_all(&capture_catalogue).unwrap();
    fs::copy(
        catalogue_dir.join("manifest.json"),
        capture_catalogue.join("manifest.json"),
    )
    .unwrap();
    fs::copy(
        catalogue_dir.join("client-pois.json"),
        capture_catalogue.join("client-pois.json"),
    )
    .unwrap();
    let catalogue_manifest = fs::read(capture_catalogue.join("manifest.json")).unwrap();
    let catalogue_payload = fs::read(capture_catalogue.join("client-pois.json")).unwrap();
    let outcome = serde_json::json!({
        "schema": 1,
        "status": "ready",
        "revision": revision,
        "cache_id": cache_id,
        "content_id": content_id,
        "navpack_sha256": navpack_sha256,
        "demand": "ReadyImages",
        "images": null,
        "note": "No shipped terrain is supplied by this fixture; this path isolates catalogue generation, not shipped-terrain installation.",
        "catalogue": {
            "identity": descriptor.catalogue_identity(),
            "manifest_sha256": Digest::of(&catalogue_manifest),
            "payload_sha256": Digest::of(&catalogue_payload),
            "payload_bytes": catalogue_payload.len(),
            "record_count": ready.catalogue.pois().records.as_slice().len(),
        },
        "catalogue_request_seconds": catalogue_request_seconds,
        "cooperative": {
            "dispatches": metrics_end.dispatches.saturating_sub(metrics_start.dispatches),
            "units": metrics_end.units.saturating_sub(metrics_start.units),
            "active_nanos": metrics_end.dispatch_nanos.saturating_sub(metrics_start.dispatch_nanos),
            "max_dispatch_nanos": metrics_end.max_dispatch_nanos,
        },
    });
    fs::write(
        capture.join("capture.json"),
        serde_json::to_vec(&outcome).unwrap(),
    )
    .unwrap();
}

/// Capture one complete real-content demand through the host API. Run once
/// against each source snapshot with the same canonical cache, navpack, and
/// output contract; compare the resulting full capture trees byte for byte.
///
/// Required: `BOT_NAV_BUILD=skip`, `BOT_MAP_BAKE_JAG_DIR`,
/// `BOT_MAP_BAKE_SNAPSHOT_DIR`, `BOT_MAP_BAKE_NAVPACK`, and a fresh
/// `BOT_MAP_BAKE_CAPTURE_DIR`. `BOT_MAP_BAKE_REVISION` defaults to 289.
#[test]
#[ignore = "real-content old/new map-bake equivalence capture"]
fn real_content_map_demand_capture() {
    assert_eq!(
        std::env::var("BOT_NAV_BUILD").unwrap_or_default(),
        "skip",
        "real-content capture must use the staged navpack, not rebuild it"
    );
    let env_path = |name: &str| {
        PathBuf::from(
            std::env::var_os(name).unwrap_or_else(|| panic!("set {name} for real-content capture")),
        )
    };
    let jag_dir = env_path("BOT_MAP_BAKE_JAG_DIR");
    let snapshot_root = env_path("BOT_MAP_BAKE_SNAPSHOT_DIR");
    let navpack = env_path("BOT_MAP_BAKE_NAVPACK");
    let capture = env_path("BOT_MAP_BAKE_CAPTURE_DIR");
    let revision = std::env::var("BOT_MAP_BAKE_REVISION")
        .ok()
        .map(|value| value.parse::<u16>().expect("numeric map-bake revision"))
        .unwrap_or(289);
    assert!(
        !capture.exists(),
        "capture output must be fresh: {}",
        capture.display()
    );
    fs::create_dir_all(&capture).unwrap();
    let cache_root = MapCacheRoot::from_existing_root(capture.join(".cache"));

    // Bind both identities to the same complete source: offline() verifies
    // the decoded snapshot's versionlist and computes its content identity;
    // the navpack sidecar binds that identity to the canonical source cache.
    let input = PreparedMapInput::offline(revision, &jag_dir, &snapshot_root).unwrap();
    let cache = nav_planner::manifest::CacheManifest::capture(revision, &jag_dir).unwrap();
    let nav_manifest_path = nav_planner::manifest::nav_manifest_path(&navpack);
    let nav_manifest: nav_planner::manifest::NavManifest =
        serde_json::from_slice(&fs::read(nav_manifest_path).unwrap()).unwrap();
    let navpack_sha256 = nav_planner::manifest::hash_file(&navpack).unwrap();
    let content_id = input.content().to_string();
    nav_manifest
        .verify_pack(revision, &cache, &navpack_sha256, Some(content_id.as_str()))
        .unwrap();
    for (extension, expected) in [
        ("navflags", nav_manifest.flags_sha256.as_deref()),
        ("navreach", nav_manifest.reach_sha256.as_deref()),
        ("navcanlight", nav_manifest.canlight_sha256.as_deref()),
        ("navpois", nav_manifest.pois_sha256.as_deref()),
    ] {
        let sidecar = navpack.with_extension(extension);
        match expected {
            Some(expected) => {
                assert_eq!(
                    nav_planner::manifest::hash_file(&sidecar).unwrap(),
                    expected,
                    "reference navpack sidecar {}",
                    sidecar.display()
                );
            }
            None => assert!(
                !sidecar.exists(),
                "undeclared navpack sidecar exists: {}",
                sidecar.display()
            ),
        }
    }

    let (image_policy, catalogue_policy) = map_artifact_policies().unwrap();
    let descriptor =
        MapProfileDescriptor::new(revision, input.content(), image_policy, catalogue_policy)
            .unwrap()
            .with_input(input)
            .unwrap();
    if std::env::var("BOT_MAP_BAKE_CATALOGUE_ONLY").as_deref() == Ok("1") {
        catalogue_only_capture(
            &capture,
            &cache_root,
            &descriptor,
            revision,
            &cache.identity(),
            &content_id,
            &navpack_sha256,
        );
        return;
    }
    let work_service =
        script::work_admission::WorkService::shared().expect("WorkAdmission service");
    let (capacity_start, progress_drops_start) = {
        let service = work_service.lock();
        (service.capacity(), service.dropped_progress_reports())
    };
    let bake_metrics_start = map_bake_test_step_snapshot();
    let manager = MapDemandManager::new(cache_root.clone(), Arc::new(NativeMapProducer::new()));
    let handle = manager
        .request(descriptor.clone(), MapDemand::Images)
        .unwrap();
    let started = Instant::now();
    let mut last_status: Option<MapJobStatus> = None;
    let mut record_status = |status: &MapJobStatus| {
        if last_status.as_ref() != Some(status) {
            eprintln!(
                "map-bake visible status: elapsed_ms={} {status:?}",
                started.elapsed().as_millis()
            );
            last_status = Some(status.clone());
        }
        if let MapJobStatus::Running(progress) = status {
            // Publishing also occurs for the catalogue. Only the image job
            // has a terrain-unit denominator.
            let publishing_images = progress.stage == MapStage::Publishing
                && manager
                    .inner
                    .jobs
                    .lock()
                    .active
                    .as_ref()
                    .is_some_and(|active| active.artifact == ArtifactKind::Images);
            if matches!(
                progress.stage,
                MapStage::BakingPlane | MapStage::BuildingZoomLevels
            ) || publishing_images
            {
                assert!(
                    progress.total > 0 && !progress.message.contains("0/0"),
                    "terrain progress must have a nonzero denominator: {progress:?}"
                );
            }
        }
    };
    loop {
        let status = handle.status();
        record_status(&status);
        match &status {
            MapJobStatus::Running(progress) if progress.stage == MapStage::DerivingPois => break,
            MapJobStatus::Queued | MapJobStatus::Running(_) => {
                assert!(
                    started.elapsed() < Duration::from_secs(60 * 60),
                    "real-content catalogue phase never started"
                );
                handle.wait_for_change(Duration::from_millis(100));
            }
            MapJobStatus::Ready => panic!("real-content map demand skipped its catalogue phase"),
            other => panic!("real-content map demand ended before catalogue phase: {other:?}"),
        }
    }

    let capacity_during_catalogue = work_service.lock().capacity();
    assert!(
        capacity_during_catalogue.jobs() > capacity_start.jobs(),
        "the real catalogue bake must remain admitted while profile work is submitted"
    );
    // Match the production panel's 64/32/32 MiB ProfilePreparation bounds,
    // and submit through WorkAdmission rather than reserving a test charge.
    const MIB: usize = 1024 * 1024;
    let profile_input = script::preparation::PreparationInput::new(
        u64::from(revision),
        64 * MIB,
        32 * MIB,
        32 * MIB,
    )
    .expect("production profile preparation bounds");
    let profile_charge = profile_input.charge().total_bytes().unwrap();
    assert_eq!(profile_charge, 128 * MIB);
    let profile_task = script::preparation::submit(
        script::preparation::PreparationKind::Profile,
        api::identity::Scope::Process,
        profile_input,
        |_| (),
    )
    .expect("128 MiB ProfilePreparation admission during catalogue bake");
    let capacity_with_profile = work_service.lock().capacity();
    assert_eq!(
        capacity_with_profile.jobs(),
        capacity_during_catalogue.jobs() + 1,
        "the real ProfilePreparation ticket must coexist with the catalogue ticket"
    );
    let profile_reservation = capacity_with_profile
        .bytes()
        .checked_sub(capacity_during_catalogue.bytes())
        .expect("profile admission must not release catalogue capacity");
    assert!(
        profile_reservation > profile_charge,
        "the admitted profile must reserve progress custody in addition to its {profile_charge}-byte charge"
    );
    let profile_progress_custody = profile_reservation - profile_charge;

    loop {
        let status = handle.status();
        record_status(&status);
        match status {
            MapJobStatus::Ready => break,
            MapJobStatus::Queued | MapJobStatus::Running(_) => {
                assert!(
                    started.elapsed() < Duration::from_secs(60 * 60),
                    "real-content map demand never settled"
                );
                handle.wait_for_change(Duration::from_millis(100));
            }
            other => panic!("real-content map demand ended {other:?}"),
        }
    }
    let bake_elapsed = started.elapsed();
    let ready = handle.ready().unwrap();
    let images = ready
        .images
        .as_ref()
        .expect("full image demand publishes terrain");
    let catalogue_dir = cache_root
        .catalogue_dir(descriptor.catalogue_identity())
        .unwrap();
    let image_dir = cache_root.image_dir(descriptor.image_identity()).unwrap();
    let capture_catalogue = capture.join("catalogue");
    let capture_images = capture.join("images");
    fs::create_dir_all(&capture_catalogue).unwrap();
    fs::create_dir_all(&capture_images).unwrap();
    fs::copy(
        catalogue_dir.join("manifest.json"),
        capture_catalogue.join("manifest.json"),
    )
    .unwrap();
    fs::copy(
        catalogue_dir.join("client-pois.json"),
        capture_catalogue.join("client-pois.json"),
    )
    .unwrap();
    fs::copy(
        image_dir.join("manifest.json"),
        capture_images.join("manifest.json"),
    )
    .unwrap();

    let image_manifest_bytes = fs::read(capture_images.join("manifest.json")).unwrap();
    let image_manifest =
        ImageManifest::decode(&image_manifest_bytes, descriptor.image_identity()).unwrap();
    let mut captured_tiles = Vec::new();
    for tile in image_manifest.tiles.as_slice() {
        let mut bytes = Vec::new();
        images
            .read_tile_into(tile.key, &mut bytes)
            .unwrap()
            .expect("manifest-listed native tile");
        let relative = tile.key.relative_path().unwrap();
        let destination = capture_images.join("tiles").join(&relative);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(&destination, &bytes).unwrap();
        captured_tiles.push(serde_json::json!({
            "key": tile.key,
            "path": relative,
            "bytes": tile.payload.bytes,
            "sha256": tile.payload.sha256,
        }));
    }
    let catalogue_manifest = fs::read(capture_catalogue.join("manifest.json")).unwrap();
    let catalogue_payload = fs::read(capture_catalogue.join("client-pois.json")).unwrap();
    let outcome = serde_json::json!({
        "schema": 1,
        "status": "ready",
        "revision": revision,
        "cache_id": cache.identity(),
        "content_id": content_id,
        "navpack_sha256": navpack_sha256,
        "catalogue": {
            "identity": descriptor.catalogue_identity(),
            "manifest_sha256": Digest::of(&catalogue_manifest),
            "payload_sha256": Digest::of(&catalogue_payload),
            "payload_bytes": catalogue_payload.len(),
            "record_count": ready.catalogue.pois().records.as_slice().len(),
        },
        "images": {
            "identity": descriptor.image_identity(),
            "manifest_sha256": Digest::of(&image_manifest_bytes),
            "tile_count": image_manifest.tiles.as_slice().len(),
            "captured_native_tiles": captured_tiles,
        },
    });
    fs::write(
        capture.join("capture.json"),
        serde_json::to_vec(&outcome).unwrap(),
    )
    .unwrap();
    profile_task
        .join()
        .expect("ProfilePreparation terminal after map bake");
    let (capacity_settled, progress_reports_dropped) = {
        let service = work_service.lock();
        (
            service.capacity(),
            service
                .dropped_progress_reports()
                .saturating_sub(progress_drops_start),
        )
    };
    assert_eq!(
        progress_reports_dropped, 0,
        "real map progress must stay within immutable progress custody"
    );
    assert_eq!(
        capacity_settled, capacity_start,
        "map and profile terminals must release all WorkService custody"
    );
    let released_jobs = capacity_with_profile
        .jobs()
        .checked_sub(capacity_settled.jobs())
        .expect("completed map/profile work cannot increase the job ledger");
    let released_bytes = capacity_with_profile
        .bytes()
        .checked_sub(capacity_settled.bytes())
        .expect("completed map/profile work cannot increase the byte ledger");
    assert!(released_jobs >= 2);
    assert!(released_bytes >= profile_charge + profile_progress_custody);

    let bake_metrics_end = map_bake_test_step_snapshot();
    let dispatches = bake_metrics_end
        .dispatches
        .saturating_sub(bake_metrics_start.dispatches);
    let units = bake_metrics_end
        .units
        .saturating_sub(bake_metrics_start.units);
    let dispatch_nanos = bake_metrics_end
        .dispatch_nanos
        .saturating_sub(bake_metrics_start.dispatch_nanos);
    eprintln!(
        "map-bake WorkService capacity: baseline_jobs={} baseline_bytes={} catalogue_jobs={} catalogue_bytes={} profile_jobs={} profile_bytes={} profile_charge_bytes={} profile_progress_custody_bytes={} released_jobs={} released_bytes={} settled_jobs={} settled_bytes={}",
        capacity_start.jobs(),
        capacity_start.bytes(),
        capacity_during_catalogue.jobs(),
        capacity_during_catalogue.bytes(),
        capacity_with_profile.jobs(),
        capacity_with_profile.bytes(),
        profile_charge,
        profile_progress_custody,
        released_jobs,
        released_bytes,
        capacity_settled.jobs(),
        capacity_settled.bytes(),
    );
    eprintln!(
        "map-bake host step metrics: dispatches={} units={} cumulative_dispatch_ms={} max_dispatch_ns_observed={} bake_wall_ms={} outside_job_steps_ms={} progress_reports_dropped={}",
        dispatches,
        units,
        dispatch_nanos / 1_000_000,
        bake_metrics_end.max_dispatch_nanos,
        bake_elapsed.as_millis(),
        bake_elapsed.as_nanos().saturating_sub(u128::from(dispatch_nanos)) / 1_000_000,
        progress_reports_dropped,
    );
}
