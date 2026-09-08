//! Real-frame snapshot equivalence through Host / host-play publication paths.
//!
//! Offline Client fixtures only. Exercises production builders:
//! - `host::drain_and_rebuild_snapshot` (SlotLoop::after_drain body)
//! - `host_play::observe_rebuild_snapshot` (nav/script tick-edge rebuild)
//!
//! Feature-off and feature-on builds must share the same observable values and
//! gate histories. Feature-on additionally proves Arc identity among coexisting
//! owners when bodies match, and isolation across slots/Plays when they must not.
//!
//! Panel `publish_nav_snapshot` is covered in `panel` crate unit tests (same
//! attach + `GameSnapshot::rebuild` seam as host-play observe).

use api::snapshot::{GameSnapshot, WidgetView};
use client::client::{Client, ClientConfig};
use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeMut};
use client::config::LocType;
use client::dash3d::ClientPlayer;
use client::io::ServerProt;
use host::{drain_and_rebuild_snapshot, Pump};
use host_play::observe_rebuild_snapshot;
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

/// Observable transcript of the three A1 families + gen gates.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FamilyObs {
    widget_ids: Vec<i32>,
    widget_texts: Vec<Option<String>>,
    side_tab_roots: Vec<i32>,
    side_tab_available: Vec<bool>,
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
        loc_ids: snap.locs().iter().map(|l| l.id).collect(),
        loc_names: snap.locs().iter().map(|l| l.name.clone()).collect(),
        gens_iface: snap.gens().iface,
        gens_inv: snap.gens().inv,
        gens_scene: snap.gens().scene,
    }
}

fn host_publish(pump: &mut Pump, snap: &mut GameSnapshot, client: &Client) -> host::DrainResult {
    drain_and_rebuild_snapshot(pump, snap, client)
}

fn nav_publish(snap: &mut GameSnapshot, client: &Client, tick_edge: bool) -> bool {
    observe_rebuild_snapshot(snap, client, tick_edge)
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

    set_iface_mut(
        &mut client,
        1001,
        IfTypeMut {
            text: "Changed".into(),
            colour: 0x00FF00,
            ..Default::default()
        },
    );
    bump_iface(&mut client);
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
}
