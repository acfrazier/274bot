//! Consent before a local WalkTo terrain bake, against a real map cache and
//! worker: a cold images open asks and bakes nothing, Not now stays
//! catalogue-only, accepting bakes, a ready (pre-installed or earlier) cache
//! opens without asking, and the remembered choice skips the question.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use frontend_core::{
    load_map_bake_choice, persist_map_bake_choice, MapBakeChoice, MapBakeGate, MapBakePrompt,
};
use host_play::map_cache::fixture::{fixture_descriptor, CountingMapProducer};
use host_play::map_cache::{
    MapCacheRoot, MapDemand, MapDemandHandle, MapDemandManager, MapJobStatus,
};

/// Scratch map-cache root, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("274bot-map-bake-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }

    fn root(&self) -> MapCacheRoot {
        MapCacheRoot::from_root(&self.0)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn manager(scratch: &Scratch) -> (MapDemandManager, Arc<CountingMapProducer>) {
    let producer = Arc::new(CountingMapProducer::default());
    (
        MapDemandManager::new(scratch.root(), producer.clone()),
        producer,
    )
}

fn wait_ready(handle: &MapDemandHandle) {
    let start = Instant::now();
    loop {
        match handle.status() {
            MapJobStatus::Ready => return,
            MapJobStatus::Queued | MapJobStatus::Running(_) => {}
            other => panic!("map demand ended {other:?}"),
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "map demand never ready"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn has_terrain(handle: &MapDemandHandle) -> bool {
    handle.ready().unwrap().images.is_some()
}

#[test]
fn a_cold_images_open_asks_and_bakes_only_the_catalogue() {
    let scratch = Scratch::new("cold");
    let (manager, producer) = manager(&scratch);
    let mut gate = MapBakeGate::new(MapBakeChoice::Ask);
    let handle = gate
        .open(&manager, fixture_descriptor(), MapDemand::Images)
        .unwrap();
    assert_eq!(gate.prompt(), MapBakePrompt::Asking);
    assert_eq!(handle.demand(), MapDemand::CatalogueOnly);
    wait_ready(&handle);
    assert!(!has_terrain(&handle));
    assert_eq!(producer.catalogue_runs(), 1);
    assert_eq!(producer.image_runs(), 0, "no terrain bake");
}

#[test]
fn not_now_keeps_the_map_catalogue_only_across_reopens() {
    let scratch = Scratch::new("decline");
    let (manager, producer) = manager(&scratch);
    let mut gate = MapBakeGate::new(MapBakeChoice::Ask);
    let first = gate
        .open(&manager, fixture_descriptor(), MapDemand::Images)
        .unwrap();
    wait_ready(&first);
    gate.decline();
    assert_eq!(gate.prompt(), MapBakePrompt::Declined);
    drop(first);
    manager.reap();
    let reopened = gate
        .open(&manager, fixture_descriptor(), MapDemand::Images)
        .unwrap();
    wait_ready(&reopened);
    assert_eq!(gate.prompt(), MapBakePrompt::Declined, "no second question");
    assert!(!has_terrain(&reopened));
    assert_eq!(producer.image_runs(), 0, "no terrain bake");
}

#[test]
fn accepting_bakes_the_terrain_and_joins_the_open_catalogue_demand() {
    let scratch = Scratch::new("accept");
    let (manager, producer) = manager(&scratch);
    let mut gate = MapBakeGate::new(MapBakeChoice::Ask);
    let asking = gate
        .open(&manager, fixture_descriptor(), MapDemand::Images)
        .unwrap();
    gate.accept();
    let baking = gate
        .open(&manager, fixture_descriptor(), MapDemand::Images)
        .unwrap();
    drop(asking);
    assert_eq!(gate.prompt(), MapBakePrompt::None);
    assert_eq!(baking.demand(), MapDemand::Images);
    wait_ready(&baking);
    assert!(has_terrain(&baking));
    assert_eq!(producer.image_runs(), 1);
    assert_eq!(producer.catalogue_runs(), 1);
}

#[test]
fn a_ready_cache_opens_without_asking_or_baking() {
    let scratch = Scratch::new("warm");
    // Publish once, as an earlier run (or a pre-installed cache) left it.
    {
        let (installer, _) = manager(&scratch);
        let handle = MapBakeGate::new(MapBakeChoice::Always)
            .open(&installer, fixture_descriptor(), MapDemand::Images)
            .unwrap();
        wait_ready(&handle);
    }
    let (manager, producer) = manager(&scratch);
    let mut gate = MapBakeGate::new(MapBakeChoice::Ask);
    let handle = gate
        .open(&manager, fixture_descriptor(), MapDemand::Images)
        .unwrap();
    assert_eq!(gate.prompt(), MapBakePrompt::None);
    assert_eq!(handle.status(), MapJobStatus::Ready, "served, not queued");
    assert!(has_terrain(&handle));
    assert_eq!(producer.catalogue_runs(), 0);
    assert_eq!(producer.image_runs(), 0);
}

#[test]
fn catalogue_only_demand_never_asks_even_when_cold() {
    let scratch = Scratch::new("catalogue");
    let (manager, producer) = manager(&scratch);
    let mut gate = MapBakeGate::new(MapBakeChoice::Ask);
    let handle = gate
        .open(&manager, fixture_descriptor(), MapDemand::CatalogueOnly)
        .unwrap();
    assert_eq!(gate.prompt(), MapBakePrompt::None);
    wait_ready(&handle);
    assert_eq!(producer.catalogue_runs(), 1);
    assert_eq!(producer.image_runs(), 0);
}

#[test]
fn the_remembered_always_choice_bakes_without_asking() {
    let _iso = script::IsolatedEnv::enter("map-bake-always");
    persist_map_bake_choice(MapBakeChoice::Always).unwrap();
    let scratch = Scratch::new("always");
    let (manager, producer) = manager(&scratch);
    let mut gate = MapBakeGate::new(load_map_bake_choice());
    let handle = gate
        .open(&manager, fixture_descriptor(), MapDemand::Images)
        .unwrap();
    assert_eq!(gate.prompt(), MapBakePrompt::None);
    wait_ready(&handle);
    assert!(has_terrain(&handle));
    assert_eq!(producer.image_runs(), 1);
}

#[test]
fn the_choice_is_ask_when_absent_or_unknown_and_persists_beside_other_prefs() {
    let _iso = script::IsolatedEnv::enter("map-bake-choice");
    assert_eq!(load_map_bake_choice(), MapBakeChoice::Ask, "no prefs file");
    let path = host_play::panel_ui_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    // A 0.1.8.1 prefs file has no `map_bake` key.
    std::fs::write(
        &path,
        r#"{"last_focus":"alice","background_bots_ack":true}"#,
    )
    .unwrap();
    assert_eq!(load_map_bake_choice(), MapBakeChoice::Ask);
    persist_map_bake_choice(MapBakeChoice::Always).unwrap();
    assert_eq!(load_map_bake_choice(), MapBakeChoice::Always);
    let prefs: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(prefs["last_focus"], "alice");
    assert_eq!(prefs["background_bots_ack"], true);
    assert_eq!(prefs["map_bake"], "always");
    std::fs::write(&path, r#"{"map_bake":"sometimes"}"#).unwrap();
    assert_eq!(load_map_bake_choice(), MapBakeChoice::Ask);
}
