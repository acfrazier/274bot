//! Real-frame snapshot equivalence through Host / host-play publication paths.
//!
//! Offline Client fixtures only. Exercises production builders:
//! - `host::drain_and_rebuild_snapshot` (SlotLoop::after_drain body)
//! - `host_play::observe_rebuild_snapshot` (nav/script tick-edge rebuild)
//!
//! Feature-off and feature-on builds must share the same observable values and
//! gate histories via one committed golden transcript produced by the real
//! paths. Feature-on additionally proves Arc identity among coexisting owners
//! when bodies match, and isolation across slots/Plays when they must not.
//!
//! Panel `publish_nav_snapshot` is covered in `panel` crate unit tests (same
//! attach + `GameSnapshot::rebuild` seam as host-play observe).
//!
//! Golden: `tests/fixtures/snapshot_frame_transcript_golden.json`
//! Regenerate: `UPDATE_SNAPSHOT_FRAME_GOLDEN=1 cargo test -p host-play --test snapshot_frame_equivalence transcript_matches_golden --offline`

use api::snapshot::{GameSnapshot, WidgetView};
use client::client::{Client, ClientConfig};
use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeMut};
use client::config::LocType;
use client::dash3d::ClientPlayer;
use client::io::ServerProt;
use host::{drain_and_rebuild_snapshot, Pump};
use host_play::observe_rebuild_snapshot;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

fn cfg() -> ClientConfig {
    ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    }
}

fn set_iface(c: &mut Client, id: usize, com: IfType) {
    c.set_iface(id, com);
}

fn set_iface_mut(c: &mut Client, id: usize, m: IfTypeMut) {
    c.set_iface_mut(id, m);
}

/// Deterministic client: widgets + side tabs + one loc (matches api snapshot_dedup_proto plant).
fn frame_client() -> Client {
    let mut c = Client::new(cfg());
    plant_populated(&mut c);
    c
}

fn plant_populated(c: &mut Client) {
    set_iface(
        c,
        1000,
        IfType {
            id: 1000,
            layer_id: 1000,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![1001, 1002]),
            ..Default::default()
        },
    );
    set_iface(
        c,
        1001,
        IfType {
            id: 1001,
            layer_id: 1000,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        c,
        1001,
        IfTypeMut {
            text: "Hello".into(),
            colour: 0x00FF00,
            ..Default::default()
        },
    );
    set_iface(
        c,
        1002,
        IfType {
            id: 1002,
            layer_id: 1000,
            r#type: ComponentType::TYPE_GRAPHIC,
            button_text: "Select".into(),
            ..Default::default()
        },
    );
    set_iface_mut(
        c,
        1002,
        IfTypeMut {
            button_type: ButtonType::BUTTON_SELECT,
            scroll_pos: 7,
            ..Default::default()
        },
    );
    c.main_modal_id = 1000;

    set_iface(
        c,
        1100,
        IfType {
            id: 1100,
            layer_id: 1100,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![1101]),
            ..Default::default()
        },
    );
    set_iface(
        c,
        1101,
        IfType {
            id: 1101,
            layer_id: 1100,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        c,
        1101,
        IfTypeMut {
            text: "tab3".into(),
            ..Default::default()
        },
    );
    set_iface(
        c,
        1500,
        IfType {
            id: 1500,
            layer_id: 1500,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![1501]),
            ..Default::default()
        },
    );
    set_iface(
        c,
        1501,
        IfType {
            id: 1501,
            layer_id: 1500,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        c,
        1501,
        IfTypeMut {
            text: "tab5".into(),
            ..Default::default()
        },
    );
    c.side_icon[3] = 1100;
    c.side_icon[5] = 1500;
    c.active_icon = 3;

    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.local_player = Some(ClientPlayer::at(20, 12));
    let id = {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        let id = cache.locs.len() as i32;
        cache.locs.push(LocType {
            id,
            name: "Large door".into(),
            desc: "A sturdy wooden door.".into(),
            op: vec![Some("Open".into()), None],
            width: 2,
            length: 3,
            blockwalk: false,
            blockrange: false,
            active: true,
            ..Default::default()
        });
        id
    };
    let typecode = 0x4000_0000 + (id << 14) + 3 + (4 << 7);
    c.world
        .set_wall(0, 3, 4, 0, 0, 0, typecode, 1 << 6, 0, 0, 0, 0);
}

fn bump_all(c: &mut Client) {
    c.bump_gens(ServerProt::IF_SETTEXT);
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
    c.bump_gens(ServerProt::REBUILD_NORMAL);
}

fn bump_iface(c: &mut Client) {
    c.bump_gens(ServerProt::IF_SETTEXT);
}

fn bump_scene(c: &mut Client) {
    c.bump_gens(ServerProt::REBUILD_NORMAL);
}

fn mutate_widget_text(c: &mut Client, text: &str) {
    set_iface_mut(
        c,
        1001,
        IfTypeMut {
            text: text.into(),
            colour: 0x00FF00,
            ..Default::default()
        },
    );
    bump_iface(c);
}

fn mutate_side_tab_text(c: &mut Client, text: &str) {
    set_iface_mut(
        c,
        1101,
        IfTypeMut {
            text: text.into(),
            ..Default::default()
        },
    );
    bump_iface(c);
}

/// Replace the planted wall with a second loc type ("Open door") and bump scene.
fn mutate_loc_to_open_door(c: &mut Client) {
    let id = {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        let id = cache.locs.len() as i32;
        cache.locs.push(LocType {
            id,
            name: "Open door".into(),
            desc: "The door stands open.".into(),
            op: vec![Some("Close".into()), None],
            width: 2,
            length: 3,
            blockwalk: false,
            blockrange: false,
            active: true,
            ..Default::default()
        });
        id
    };
    let typecode = 0x4000_0000 + (id << 14) + 3 + (4 << 7);
    c.world
        .set_wall(0, 3, 4, 0, 0, 0, typecode, 1 << 6, 0, 0, 0, 0);
    bump_scene(c);
}

/// Observable transcript of the three A1 families + gen gates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FamilyObs {
    widget_ids: Vec<i32>,
    widget_texts: Vec<Option<String>>,
    side_tab_roots: Vec<i32>,
    side_tab_available: Vec<bool>,
    side_tab_active: Vec<bool>,
    /// Nested widget texts for each side-tab slot (empty when unavailable).
    side_tab_texts: Vec<Vec<Option<String>>>,
    loc_ids: Vec<i32>,
    loc_names: Vec<Option<String>>,
    gens_iface: u64,
    gens_inv: u64,
    gens_scene: u64,
}

fn observe(snap: &GameSnapshot) -> FamilyObs {
    FamilyObs {
        widget_ids: snap.widgets().iter().map(|w| w.component_id).collect(),
        widget_texts: snap.widgets().iter().map(|w| w.text.clone()).collect(),
        side_tab_roots: snap
            .side_tabs()
            .iter()
            .map(|t| t.root_component_id)
            .collect(),
        side_tab_available: snap.side_tabs().iter().map(|t| t.available).collect(),
        side_tab_active: snap.side_tabs().iter().map(|t| t.active).collect(),
        side_tab_texts: snap
            .side_tabs()
            .iter()
            .map(|t| t.widgets.iter().map(|w| w.text.clone()).collect())
            .collect(),
        loc_ids: snap.locs().iter().map(|l| l.id).collect(),
        loc_names: snap.locs().iter().map(|l| l.name.clone()).collect(),
        gens_iface: snap.gens().iface,
        gens_inv: snap.gens().inv,
        gens_scene: snap.gens().scene,
    }
}

/// One publication step: path + gate bits + family observables.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StepObs {
    name: String,
    path: String,
    dirty_any: Option<bool>,
    dirty_iface: Option<bool>,
    dirty_scene: Option<bool>,
    dirty_inv: Option<bool>,
    rebuilt: Option<bool>,
    obs: FamilyObs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Transcript {
    /// Schema tag so mismatched goldens fail loudly.
    schema: String,
    steps: Vec<StepObs>,
}

const TRANSCRIPT_SCHEMA: &str = "snapshot-frame-equivalence-v2";

fn host_publish(pump: &mut Pump, snap: &mut GameSnapshot, client: &Client) -> host::DrainResult {
    drain_and_rebuild_snapshot(pump, snap, client)
}

fn nav_publish(snap: &mut GameSnapshot, client: &Client, tick_edge: bool) -> bool {
    observe_rebuild_snapshot(snap, client, tick_edge)
}

fn host_step(name: &str, pump: &mut Pump, snap: &mut GameSnapshot, client: &Client) -> StepObs {
    let result = host_publish(pump, snap, client);
    StepObs {
        name: name.into(),
        path: "host".into(),
        dirty_any: Some(result.dirty.any()),
        dirty_iface: Some(result.dirty.iface),
        dirty_scene: Some(result.dirty.scene),
        dirty_inv: Some(result.dirty.inv),
        rebuilt: None,
        obs: observe(snap),
    }
}

fn nav_step(name: &str, snap: &mut GameSnapshot, client: &Client, tick_edge: bool) -> StepObs {
    let rebuilt = nav_publish(snap, client, tick_edge);
    StepObs {
        name: name.into(),
        path: "host_play".into(),
        dirty_any: None,
        dirty_iface: None,
        dirty_scene: None,
        dirty_inv: None,
        rebuilt: Some(rebuilt),
        obs: observe(snap),
    }
}

/// Deterministic multi-frame sequence through real Host + host-play paths.
/// Covers first plant, quiet retain, widget/side-tab/loc change, and quiet
/// after each mutation (stale-gate retain).
fn run_cross_path_transcript() -> Transcript {
    let mut steps = Vec::new();

    // --- Host path ---
    let mut client = frame_client();
    bump_all(&mut client);
    let mut pump = Pump::new();
    let mut host_snap = GameSnapshot::new();

    steps.push(host_step(
        "first_publication",
        &mut pump,
        &mut host_snap,
        &client,
    ));
    steps.push(host_step(
        "quiet_after_first",
        &mut pump,
        &mut host_snap,
        &client,
    ));

    mutate_widget_text(&mut client, "Changed");
    steps.push(host_step(
        "widget_changed",
        &mut pump,
        &mut host_snap,
        &client,
    ));
    steps.push(host_step(
        "quiet_after_widget",
        &mut pump,
        &mut host_snap,
        &client,
    ));

    mutate_side_tab_text(&mut client, "tab3-mut");
    steps.push(host_step(
        "side_tab_changed",
        &mut pump,
        &mut host_snap,
        &client,
    ));
    steps.push(host_step(
        "quiet_after_side_tab",
        &mut pump,
        &mut host_snap,
        &client,
    ));

    mutate_loc_to_open_door(&mut client);
    steps.push(host_step("loc_changed", &mut pump, &mut host_snap, &client));
    steps.push(host_step(
        "quiet_after_loc",
        &mut pump,
        &mut host_snap,
        &client,
    ));

    // --- host-play path (fresh client, same mutation schedule) ---
    let mut client = frame_client();
    bump_all(&mut client);
    let mut nav_snap = GameSnapshot::new();

    steps.push(nav_step(
        "first_publication",
        &mut nav_snap,
        &client,
        true,
    ));
    steps.push(nav_step(
        "quiet_off_edge_after_first",
        &mut nav_snap,
        &client,
        false,
    ));
    steps.push(nav_step(
        "equal_repeated_edge",
        &mut nav_snap,
        &client,
        true,
    ));

    mutate_widget_text(&mut client, "Changed");
    steps.push(nav_step("widget_changed", &mut nav_snap, &client, true));
    steps.push(nav_step(
        "quiet_off_edge_after_widget",
        &mut nav_snap,
        &client,
        false,
    ));

    mutate_side_tab_text(&mut client, "tab3-mut");
    steps.push(nav_step("side_tab_changed", &mut nav_snap, &client, true));
    steps.push(nav_step(
        "quiet_off_edge_after_side_tab",
        &mut nav_snap,
        &client,
        false,
    ));

    mutate_loc_to_open_door(&mut client);
    steps.push(nav_step("loc_changed", &mut nav_snap, &client, true));
    steps.push(nav_step(
        "quiet_off_edge_after_loc",
        &mut nav_snap,
        &client,
        false,
    ));

    Transcript {
        schema: TRANSCRIPT_SCHEMA.into(),
        steps,
    }
}

fn golden_path() -> PathBuf {
    if let Ok(p) = std::env::var("SNAPSHOT_FRAME_GOLDEN") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/snapshot_frame_transcript_golden.json")
}

fn feature_label() -> &'static str {
    if cfg!(feature = "snapshot-dedup") {
        "on"
    } else {
        "off"
    }
}

/// Live transcript dump under CARGO_TARGET_DIR so a peer feature build can
/// load the other config's file for exact cross-build comparison.
fn target_transcript_path(label: &str) -> PathBuf {
    let root = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target-frame-eq")
            .display()
            .to_string()
    });
    PathBuf::from(root).join(format!("snapshot_frame_transcript_feature_{label}.json"))
}

fn write_json(path: &std::path::Path, value: &impl Serialize) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = serde_json::to_string_pretty(value).expect("serialize transcript");
    std::fs::write(path, body + "\n").unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

fn read_transcript(path: &std::path::Path) -> Transcript {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// First publication through Host drain path yields populated A1 families.
#[test]
fn host_first_publication_populates_widgets_side_tabs_locs() {
    let mut client = frame_client();
    bump_all(&mut client);
    let mut pump = Pump::new();
    let mut snap = GameSnapshot::new();
    let result = host_publish(&mut pump, &mut snap, &client);
    assert!(
        result.dirty.iface || result.dirty.scene || result.dirty.inv,
        "first drain must dirty at least one A1-related family"
    );
    let obs = observe(&snap);
    assert!(
        obs.widget_ids.contains(&1000) && obs.widget_ids.contains(&1001),
        "host drain must publish widget tree: {obs:?}"
    );
    assert!(
        obs.widget_texts
            .iter()
            .any(|t| t.as_deref() == Some("Hello")),
        "Hello text missing: {obs:?}"
    );
    assert!(
        obs.side_tab_roots.iter().any(|&r| r == 1100),
        "side tab root 1100 missing: {obs:?}"
    );
    assert!(
        obs.side_tab_texts
            .iter()
            .flatten()
            .any(|t| t.as_deref() == Some("tab3")),
        "side tab nested text missing: {obs:?}"
    );
    assert_eq!(obs.loc_ids.len(), 1, "one loc expected: {obs:?}");
    assert_eq!(obs.loc_names[0].as_deref(), Some("Large door"));
}

/// Quiet host drain retains prior bodies and reports no dirty families.
#[test]
fn host_quiet_frame_retains_stale_bodies_and_empty_dirty() {
    let mut client = frame_client();
    bump_all(&mut client);
    let mut pump = Pump::new();
    let mut snap = GameSnapshot::new();
    host_publish(&mut pump, &mut snap, &client);
    let first = observe(&snap);
    let ptr_w = snap.widgets().as_ptr();
    let ptr_s = snap.side_tabs().as_ptr();
    let ptr_l = snap.locs().as_ptr();

    let result = host_publish(&mut pump, &mut snap, &client);
    assert!(!result.dirty.any(), "quiet drain must not mark dirty");
    assert_eq!(observe(&snap), first);
    assert_eq!(
        snap.widgets().as_ptr(),
        ptr_w,
        "quiet must keep widget storage"
    );
    assert_eq!(snap.side_tabs().as_ptr(), ptr_s);
    assert_eq!(snap.locs().as_ptr(), ptr_l);
}

/// host-play observe rebuilds on tick edge and skips off-edge (keeps last).
#[test]
fn host_play_observe_tick_edge_and_quiet_off_edge() {
    let mut client = frame_client();
    bump_all(&mut client);
    let mut snap = GameSnapshot::new();
    assert!(
        nav_publish(&mut snap, &client, true),
        "tick edge must rebuild"
    );
    let first = observe(&snap);
    assert!(!first.widget_ids.is_empty());
    assert_eq!(first.loc_ids.len(), 1);

    let ptr = snap.widgets().as_ptr();
    assert!(
        !nav_publish(&mut snap, &client, false),
        "off-edge must not rebuild"
    );
    assert_eq!(observe(&snap), first);
    assert_eq!(snap.widgets().as_ptr(), ptr);

    // Equal repeated rebuild on edge with unchanged gens: gate history quiet.
    assert!(nav_publish(&mut snap, &client, true));
    assert_eq!(observe(&snap), first);
}

/// Host drain and host-play observe agree on A1 observable transcript.
#[test]
fn host_and_host_play_paths_agree_on_family_obs() {
    let mut client = frame_client();
    bump_all(&mut client);

    let mut pump = Pump::new();
    let mut host_snap = GameSnapshot::new();
    host_publish(&mut pump, &mut host_snap, &client);
    let host_obs = observe(&host_snap);

    let mut nav_snap = GameSnapshot::new();
    nav_publish(&mut nav_snap, &client, true);
    let nav_obs = observe(&nav_snap);

    assert_eq!(
        host_obs, nav_obs,
        "Host drain_and_rebuild vs observe_rebuild must match A1 observables"
    );
}

/// Changed widgets republish; retained prior observation stays stable.
#[test]
fn retained_old_observation_stable_after_widget_change() {
    let mut client = frame_client();
    bump_all(&mut client);
    let mut pump = Pump::new();
    let mut snap = GameSnapshot::new();
    host_publish(&mut pump, &mut snap, &client);

    let old_widgets: Vec<WidgetView> = snap.widgets().to_vec();
    let old_text = old_widgets
        .iter()
        .find(|w| w.component_id == 1001)
        .and_then(|w| w.text.clone());
    assert_eq!(old_text.as_deref(), Some("Hello"));

    mutate_widget_text(&mut client, "Changed");
    host_publish(&mut pump, &mut snap, &client);

    let new_text = snap
        .widgets()
        .iter()
        .find(|w| w.component_id == 1001)
        .and_then(|w| w.text.clone());
    assert_eq!(new_text.as_deref(), Some("Changed"));
    assert_eq!(
        old_text.as_deref(),
        Some("Hello"),
        "retained old observation must not mutate under republish"
    );
    assert_eq!(
        old_widgets
            .iter()
            .find(|w| w.component_id == 1001)
            .and_then(|w| w.text.as_deref()),
        Some("Hello")
    );
}

/// Side-tab text mutation through Host: dirty.iface, new nested text, then quiet retain.
#[test]
fn host_side_tab_change_then_quiet_retain() {
    let mut client = frame_client();
    bump_all(&mut client);
    let mut pump = Pump::new();
    let mut snap = GameSnapshot::new();
    host_publish(&mut pump, &mut snap, &client);

    let old_tabs = snap.side_tabs().to_vec();
    let old_text = old_tabs
        .iter()
        .find(|t| t.root_component_id == 1100)
        .and_then(|t| t.widgets.iter().find_map(|w| w.text.clone()));
    assert_eq!(old_text.as_deref(), Some("tab3"));

    mutate_side_tab_text(&mut client, "tab3-mut");
    let result = host_publish(&mut pump, &mut snap, &client);
    assert!(result.dirty.iface, "side-tab text change must dirty iface");
    let new_text = snap
        .side_tabs()
        .iter()
        .find(|t| t.root_component_id == 1100)
        .and_then(|t| t.widgets.iter().find_map(|w| w.text.clone()));
    assert_eq!(new_text.as_deref(), Some("tab3-mut"));
    assert_eq!(
        old_text.as_deref(),
        Some("tab3"),
        "retained side-tab observation must stay tab3"
    );

    let after = observe(&snap);
    let ptr_s2 = snap.side_tabs().as_ptr();
    let quiet = host_publish(&mut pump, &mut snap, &client);
    assert!(
        !quiet.dirty.any(),
        "quiet after side-tab must be empty dirty"
    );
    assert_eq!(observe(&snap), after);
    assert_eq!(snap.side_tabs().as_ptr(), ptr_s2);
}

/// Loc typecode mutation through Host: dirty.scene, new name, then quiet retain.
#[test]
fn host_loc_change_then_quiet_retain() {
    let mut client = frame_client();
    bump_all(&mut client);
    let mut pump = Pump::new();
    let mut snap = GameSnapshot::new();
    host_publish(&mut pump, &mut snap, &client);

    let old_locs = snap.locs().to_vec();
    assert_eq!(old_locs[0].name.as_deref(), Some("Large door"));

    mutate_loc_to_open_door(&mut client);
    let result = host_publish(&mut pump, &mut snap, &client);
    assert!(result.dirty.scene, "loc typecode change must dirty scene");
    assert_eq!(snap.locs().len(), 1);
    assert_eq!(snap.locs()[0].name.as_deref(), Some("Open door"));
    assert_eq!(
        old_locs[0].name.as_deref(),
        Some("Large door"),
        "retained loc observation must stay Large door"
    );

    let after = observe(&snap);
    let ptr_l2 = snap.locs().as_ptr();
    let quiet = host_publish(&mut pump, &mut snap, &client);
    assert!(!quiet.dirty.any(), "quiet after loc must be empty dirty");
    assert_eq!(observe(&snap), after);
    assert_eq!(snap.locs().as_ptr(), ptr_l2);
}

/// host-play side-tab + loc change on tick edge; off-edge retains stale bodies.
#[test]
fn host_play_side_tab_and_loc_change_then_off_edge_retain() {
    let mut client = frame_client();
    bump_all(&mut client);
    let mut snap = GameSnapshot::new();
    assert!(nav_publish(&mut snap, &client, true));

    let old_tabs = snap.side_tabs().to_vec();
    mutate_side_tab_text(&mut client, "tab3-mut");
    assert!(nav_publish(&mut snap, &client, true));
    let mut_text = snap
        .side_tabs()
        .iter()
        .find(|t| t.root_component_id == 1100)
        .and_then(|t| t.widgets.iter().find_map(|w| w.text.clone()));
    assert_eq!(mut_text.as_deref(), Some("tab3-mut"));
    assert_eq!(
        old_tabs
            .iter()
            .find(|t| t.root_component_id == 1100)
            .and_then(|t| t.widgets.iter().find_map(|w| w.text.as_deref())),
        Some("tab3")
    );
    let after_tab = observe(&snap);
    let ptr_s = snap.side_tabs().as_ptr();
    assert!(!nav_publish(&mut snap, &client, false));
    assert_eq!(observe(&snap), after_tab);
    assert_eq!(snap.side_tabs().as_ptr(), ptr_s);

    let old_locs = snap.locs().to_vec();
    mutate_loc_to_open_door(&mut client);
    assert!(nav_publish(&mut snap, &client, true));
    assert_eq!(snap.locs()[0].name.as_deref(), Some("Open door"));
    assert_eq!(old_locs[0].name.as_deref(), Some("Large door"));
    let after_loc = observe(&snap);
    let ptr_l = snap.locs().as_ptr();
    assert!(!nav_publish(&mut snap, &client, false));
    assert_eq!(observe(&snap), after_loc);
    assert_eq!(snap.locs().as_ptr(), ptr_l);
}

/// Committed golden + peer feature dump: exact observable/gate history equality
/// across feature-off and feature-on builds (not same-build self-asserts alone).
#[test]
fn transcript_matches_golden_and_peer_feature_build() {
    let live = run_cross_path_transcript();
    assert_eq!(live.schema, TRANSCRIPT_SCHEMA);
    assert!(
        live.steps.iter().any(|s| s.name == "side_tab_changed"),
        "transcript must include side_tab_changed"
    );
    assert!(
        live.steps.iter().any(|s| s.name == "loc_changed"),
        "transcript must include loc_changed"
    );

    // Sanity on gate histories (feature-agnostic observables).
    let host_first = live
        .steps
        .iter()
        .find(|s| s.path == "host" && s.name == "first_publication")
        .expect("host first");
    assert_eq!(host_first.dirty_any, Some(true));
    assert!(
        host_first.obs.widget_texts.iter().any(|t| t.as_deref() == Some("Hello"))
    );
    assert!(host_first
        .obs
        .side_tab_texts
        .iter()
        .flatten()
        .any(|t| t.as_deref() == Some("tab3")));
    assert_eq!(host_first.obs.loc_names[0].as_deref(), Some("Large door"));

    let host_quiet = live
        .steps
        .iter()
        .find(|s| s.path == "host" && s.name == "quiet_after_first")
        .expect("host quiet");
    assert_eq!(host_quiet.dirty_any, Some(false));
    assert_eq!(host_quiet.obs, host_first.obs);

    let host_side = live
        .steps
        .iter()
        .find(|s| s.path == "host" && s.name == "side_tab_changed")
        .expect("host side");
    assert_eq!(host_side.dirty_iface, Some(true));
    assert!(host_side
        .obs
        .side_tab_texts
        .iter()
        .flatten()
        .any(|t| t.as_deref() == Some("tab3-mut")));

    let host_loc = live
        .steps
        .iter()
        .find(|s| s.path == "host" && s.name == "loc_changed")
        .expect("host loc");
    assert_eq!(host_loc.dirty_scene, Some(true));
    assert_eq!(host_loc.obs.loc_names[0].as_deref(), Some("Open door"));

    let nav_off = live
        .steps
        .iter()
        .find(|s| s.path == "host_play" && s.name == "quiet_off_edge_after_side_tab")
        .expect("nav quiet side");
    assert_eq!(nav_off.rebuilt, Some(false));

    // Host vs host-play FamilyObs agreement at shared content steps.
    for name in [
        "first_publication",
        "widget_changed",
        "side_tab_changed",
        "loc_changed",
    ] {
        let h = live
            .steps
            .iter()
            .find(|s| s.path == "host" && s.name == name)
            .unwrap_or_else(|| panic!("missing host {name}"));
        let n = live
            .steps
            .iter()
            .find(|s| s.path == "host_play" && s.name == name)
            .unwrap_or_else(|| panic!("missing host_play {name}"));
        assert_eq!(
            h.obs, n.obs,
            "host vs host_play FamilyObs diverge at {name}"
        );
    }

    let golden_file = golden_path();
    if std::env::var("UPDATE_SNAPSHOT_FRAME_GOLDEN").ok().as_deref() == Some("1") {
        write_json(&golden_file, &live);
    }
    assert!(
        golden_file.is_file(),
        "missing golden at {} — run with UPDATE_SNAPSHOT_FRAME_GOLDEN=1 once under feature-off",
        golden_file.display()
    );
    let golden = read_transcript(&golden_file);
    assert_eq!(
        live, golden,
        "live transcript (feature-{}) must exactly equal committed golden",
        feature_label()
    );

    // Dump this feature's transcript for the peer build to load.
    let mine = target_transcript_path(feature_label());
    write_json(&mine, &live);

    let peer_label = if feature_label() == "on" { "off" } else { "on" };
    let peer = target_transcript_path(peer_label);
    if peer.is_file() {
        let peer_tx = read_transcript(&peer);
        assert_eq!(
            live, peer_tx,
            "cross-feature exact compare failed: feature-{} vs feature-{} (paths {} vs {})",
            feature_label(),
            peer_label,
            mine.display(),
            peer.display()
        );
    } else {
        // First of the two cargo invocations: peer dump not present yet.
        // Committed golden already proved equality for this config; report
        // documents that the second invocation performs the peer load.
        eprintln!(
            "note: peer transcript {} not found yet; golden match is the cross-build oracle for this run",
            peer.display()
        );
    }
}

#[cfg(feature = "snapshot-dedup")]
mod with_dedup {
    use super::*;
    use api::snapshot_dedup::{SlotDedupDirectory, SlotDedupInstance};

    fn attach(inst: &SlotDedupInstance) -> GameSnapshot {
        let mut s = GameSnapshot::new();
        s.attach_dedup(inst.attach());
        s
    }

    /// Host + host-play owners on one slot share Arc bodies when equal.
    #[test]
    fn host_and_nav_owners_share_arc_on_equal_bodies() {
        let mut client = frame_client();
        bump_all(&mut client);
        let inst = SlotDedupInstance::new();

        let mut pump = Pump::new();
        let mut host_snap = attach(&inst);
        host_publish(&mut pump, &mut host_snap, &client);

        let mut nav_snap = attach(&inst);
        nav_publish(&mut nav_snap, &client, true);

        assert!(
            Arc::ptr_eq(&host_snap.widgets_arc(), &nav_snap.widgets_arc()),
            "equal widgets must share Arc across host+nav owners"
        );
        assert!(Arc::ptr_eq(
            &host_snap.side_tabs_arc(),
            &nav_snap.side_tabs_arc()
        ));
        assert!(Arc::ptr_eq(&host_snap.locs_arc(), &nav_snap.locs_arc()));
        assert!(
            nav_snap.dedup_counters().equality_hits() >= 1
                || host_snap.dedup_counters().equality_hits() >= 1,
            "at least one family must record an equality hit: host={:?} nav={:?}",
            host_snap.dedup_counters(),
            nav_snap.dedup_counters()
        );
        assert_eq!(observe(&host_snap), observe(&nav_snap));
    }

    /// Three coexisting owners share when equal; unequal body does not share.
    #[test]
    fn three_owners_share_when_equal_and_miss_when_unequal() {
        let mut client = frame_client();
        bump_all(&mut client);
        let inst = SlotDedupInstance::new();

        let mut pump = Pump::new();
        let mut host_snap = attach(&inst);
        host_publish(&mut pump, &mut host_snap, &client);

        let mut nav_a = attach(&inst);
        nav_publish(&mut nav_a, &client, true);
        let mut nav_b = attach(&inst);
        nav_publish(&mut nav_b, &client, true);

        assert!(Arc::ptr_eq(&host_snap.widgets_arc(), &nav_a.widgets_arc()));
        assert!(Arc::ptr_eq(&nav_a.widgets_arc(), &nav_b.widgets_arc()));

        let mut client2 = frame_client();
        set_iface_mut(
            &mut client2,
            1001,
            IfTypeMut {
                text: "Other".into(),
                colour: 0x00FF00,
                ..Default::default()
            },
        );
        bump_all(&mut client2);
        drop(nav_b);
        let mut nav_miss = attach(&inst);
        nav_publish(&mut nav_miss, &client2, true);
        assert!(
            !Arc::ptr_eq(&host_snap.widgets_arc(), &nav_miss.widgets_arc()),
            "unequal widgets must not share"
        );
        assert!(
            nav_miss.dedup_counters().widgets.equality_misses >= 1,
            "miss counter must advance"
        );
    }

    /// Quiet frame under dedup: quiet_skips advance; no new equality work.
    #[test]
    fn quiet_observe_advances_quiet_skips_without_equality() {
        let mut client = frame_client();
        bump_all(&mut client);
        let inst = SlotDedupInstance::new();
        let mut snap = attach(&inst);
        nav_publish(&mut snap, &client, true);
        let hits_before = snap.dedup_counters().equality_hits();
        let comps_before = snap.dedup_counters().equality_comparisons();
        let quiet_before = snap.dedup_counters().widgets.quiet_skips
            + snap.dedup_counters().side_tabs.quiet_skips
            + snap.dedup_counters().loc.quiet_skips;

        nav_publish(&mut snap, &client, true);
        let quiet_after = snap.dedup_counters().widgets.quiet_skips
            + snap.dedup_counters().side_tabs.quiet_skips
            + snap.dedup_counters().loc.quiet_skips;
        assert!(
            quiet_after > quiet_before,
            "quiet gate must bump quiet_skips ({quiet_before} -> {quiet_after})"
        );
        assert_eq!(
            snap.dedup_counters().equality_comparisons(),
            comps_before,
            "quiet must not run equality"
        );
        assert_eq!(snap.dedup_counters().equality_hits(), hits_before);
    }

    /// Slot stop/restart: directory remove + new instance does not share with old.
    #[test]
    fn slot_restart_via_directory_does_not_share_with_prior_instance() {
        let dir = SlotDedupDirectory::new_shared();
        let first = SlotDedupInstance::new();
        dir.install("alice", first.clone());

        let mut client = frame_client();
        bump_all(&mut client);
        let mut snap1 = attach(&first);
        nav_publish(&mut snap1, &client, true);
        let old_widgets = snap1.widgets_arc();
        drop(snap1);
        dir.remove("alice");
        assert!(dir.get("alice").is_none());

        let second = SlotDedupInstance::new();
        dir.install("alice", second.clone());
        assert!(!Arc::ptr_eq(&first.registry(), &second.registry()));
        let mut snap2 = attach(&second);
        nav_publish(&mut snap2, &client, true);
        assert!(
            !Arc::ptr_eq(&old_widgets, &snap2.widgets_arc()),
            "restarted slot must not share Arc with prior instance body"
        );
        assert_eq!(
            snap2.dedup_counters().widgets.equality_hits,
            0,
            "fresh instance has no prior owner to hit"
        );
    }

    /// Two Play-local directories: same username never shares across directories.
    #[test]
    fn two_play_directories_isolate_same_username() {
        let d1 = SlotDedupDirectory::new_shared();
        let d2 = SlotDedupDirectory::new_shared();
        let i1 = SlotDedupInstance::new();
        let i2 = SlotDedupInstance::new();
        d1.install("alice", i1.clone());
        d2.install("alice", i2.clone());

        let mut client = frame_client();
        bump_all(&mut client);
        let mut s1 = attach(&i1);
        let mut s2 = attach(&i2);
        nav_publish(&mut s1, &client, true);
        nav_publish(&mut s2, &client, true);
        assert!(
            !Arc::ptr_eq(&s1.widgets_arc(), &s2.widgets_arc()),
            "identical username on two Plays must not share directory bodies"
        );
        assert_eq!(observe(&s1), observe(&s2));
    }

    /// Old retained Arc observation survives new equal rebuild by another owner.
    #[test]
    fn retained_arc_observation_stable_while_peer_republishes_equal() {
        let mut client = frame_client();
        bump_all(&mut client);
        let inst = SlotDedupInstance::new();
        let mut a = attach(&inst);
        nav_publish(&mut a, &client, true);
        let retained = a.widgets_arc();
        let retained_text = retained
            .iter()
            .find(|w| w.component_id == 1001)
            .and_then(|w| w.text.clone());

        let mut b = attach(&inst);
        nav_publish(&mut b, &client, true);
        assert!(Arc::ptr_eq(&retained, &b.widgets_arc()));
        assert_eq!(retained_text.as_deref(), Some("Hello"));
        assert_eq!(
            retained
                .iter()
                .find(|w| w.component_id == 1001)
                .and_then(|w| w.text.as_deref()),
            Some("Hello")
        );
    }

    /// Side-tab / loc change under dedup: unequal bodies do not share; quiet retains.
    #[test]
    fn side_tab_and_loc_change_break_arc_share_then_quiet() {
        let mut client = frame_client();
        bump_all(&mut client);
        let inst = SlotDedupInstance::new();
        let mut a = attach(&inst);
        nav_publish(&mut a, &client, true);
        let tabs_before = a.side_tabs_arc();
        let locs_before = a.locs_arc();

        mutate_side_tab_text(&mut client, "tab3-mut");
        let mut b = attach(&inst);
        nav_publish(&mut b, &client, true);
        assert!(
            !Arc::ptr_eq(&tabs_before, &b.side_tabs_arc()),
            "changed side tabs must not share prior Arc"
        );
        assert!(
            b.side_tabs()
                .iter()
                .find(|t| t.root_component_id == 1100)
                .and_then(|t| t.widgets.iter().find_map(|w| w.text.as_deref()))
                == Some("tab3-mut")
        );

        mutate_loc_to_open_door(&mut client);
        nav_publish(&mut b, &client, true);
        assert!(!Arc::ptr_eq(&locs_before, &b.locs_arc()));
        assert_eq!(b.locs()[0].name.as_deref(), Some("Open door"));

        let quiet_before = b.dedup_counters().side_tabs.quiet_skips
            + b.dedup_counters().loc.quiet_skips;
        nav_publish(&mut b, &client, true);
        let quiet_after = b.dedup_counters().side_tabs.quiet_skips
            + b.dedup_counters().loc.quiet_skips;
        assert!(quiet_after > quiet_before);
    }
}
