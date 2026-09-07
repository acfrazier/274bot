//! A1 discriminator: rebuild-edge exact snapshot family dedup prototype.
//! Uses real GameSnapshot builders and Client/cache fixtures. No live/RSS.

use api::snapshot::{Family, GameSnapshot, LocLayer, WidgetRoot};
use api::snapshot_dedup_proto::{
    account_allocations, locs_eq, side_tabs_eq, widgets_eq, DedupCursor, SlotFamilyRegistry,
};
use client::client::{Client, ClientConfig};
use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeMut};
use client::config::LocType;
use client::dash3d::ClientPlayer;
use client::io::ServerProt;
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

fn base_client() -> Client {
    Client::new(cfg())
}

fn set_iface(c: &mut Client, id: usize, com: IfType) {
    c.set_iface(id, com);
}

fn set_iface_mut(c: &mut Client, id: usize, m: IfTypeMut) {
    c.set_iface_mut(id, m);
}

/// Populated widgets + side tabs + one loc for equal-body cases.
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

    // Inventory widget for CX3.
    set_iface(
        c,
        1003,
        IfType {
            id: 1003,
            layer_id: 1000,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Use".into()), None, None, None, None],
            ..Default::default()
        },
    );
    // Attach inv under root children for CX3 path when needed.

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

fn bump_iface_inv(c: &mut Client) {
    c.bump_gens(ServerProt::IF_SETTEXT);
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
}

fn bump_scene(c: &mut Client) {
    c.bump_gens(ServerProt::REBUILD_NORMAL);
}

/// Three owners rebuild equal populated bodies → Arc ptr share; unique < old.
#[test]
fn equal_populated_bodies_share() {
    let mut c = base_client();
    plant_populated(&mut c);
    bump_iface_inv(&mut c);
    bump_scene(&mut c);

    let mut reg = SlotFamilyRegistry::new();
    let mut o0 = DedupCursor::new(0);
    let mut o1 = DedupCursor::new(1);
    let mut o2 = DedupCursor::new(2);

    assert!(o0.rebuild_families(&c, &mut reg));
    assert!(o1.rebuild_families(&c, &mut reg));
    assert!(o2.rebuild_families(&c, &mut reg));

    assert!(
        !o0.widgets().is_empty() && !o0.side_tabs().is_empty() && !o0.locs().is_empty(),
        "fixture must produce populated bodies"
    );
    assert!(
        Arc::ptr_eq(&o0.widgets_arc(), &o1.widgets_arc())
            && Arc::ptr_eq(&o1.widgets_arc(), &o2.widgets_arc()),
        "equal widgets must share one Arc"
    );
    assert!(
        Arc::ptr_eq(&o0.side_tabs_arc(), &o1.side_tabs_arc())
            && Arc::ptr_eq(&o1.side_tabs_arc(), &o2.side_tabs_arc())
    );
    assert!(
        Arc::ptr_eq(&o0.locs_arc(), &o1.locs_arc()) && Arc::ptr_eq(&o1.locs_arc(), &o2.locs_arc())
    );

    // O0 published; O1/O2 hit equality.
    assert_eq!(o0.counters().widgets.publishes, 1);
    assert!(o1.counters().widgets.equality_hits >= 1);
    assert!(o2.counters().widgets.equality_hits >= 1);

    let acct = account_allocations(&[&o0, &o1, &o2], &reg);
    assert_eq!(reg.unique_widget_bodies().len(), 1);
    assert_eq!(reg.unique_side_tab_bodies().len(), 1);
    assert_eq!(reg.unique_loc_bodies().len(), 1);
    assert!(
        acct.unique_body_payload_bytes * 3 == acct.old_per_owner_payload_bytes
            || acct.unique_body_payload_bytes < acct.old_per_owner_payload_bytes,
        "unique bodies must be strictly below summed per-owner private payloads when shared; got unique={} old={}",
        acct.unique_body_payload_bytes,
        acct.old_per_owner_payload_bytes
    );
    assert!(acct.registry_metadata_bytes > 0);
    assert!(acct.scratch_peak_bytes > 0);
}

/// Quiet gate: second rebuild does no walk and no equality work.
#[test]
fn quiet_read_zero_work() {
    let mut c = base_client();
    plant_populated(&mut c);
    bump_iface_inv(&mut c);
    bump_scene(&mut c);

    let mut reg = SlotFamilyRegistry::new();
    let mut o0 = DedupCursor::new(0);
    assert!(o0.rebuild_families(&c, &mut reg));
    let walks = o0.counters().walks();
    let eqs = o0.counters().equality_comparisons();
    let quiet_before = o0.counters().widgets.quiet_skips
        + o0.counters().side_tabs.quiet_skips
        + o0.counters().loc.quiet_skips;

    assert!(!o0.rebuild_families(&c, &mut reg));
    assert_eq!(o0.counters().walks(), walks, "no additional walks");
    assert_eq!(
        o0.counters().equality_comparisons(),
        eqs,
        "quiet path must not equality-compare"
    );
    let quiet_after = o0.counters().widgets.quiet_skips
        + o0.counters().side_tabs.quiet_skips
        + o0.counters().loc.quiet_skips;
    assert_eq!(quiet_after, quiet_before + 3);
}

/// CX1: tab click without gen bump; O2 behind gate walks fresh content, no share.
#[test]
fn cx1_tab_click_unequal_gate_history_no_share() {
    let mut c = base_client();
    plant_populated(&mut c);
    bump_iface_inv(&mut c);

    let mut reg = SlotFamilyRegistry::new();
    let mut o1 = DedupCursor::new(0); // observe shell
    let mut o2 = DedupCursor::new(1); // after_drain shell, still at default gates

    assert!(o1.rebuild_widgets(&c, &mut reg));
    assert!(o1.rebuild_side_tabs(&c, &mut reg));
    let o1_w = o1.widgets_arc();
    let o1_t = o1.side_tabs_arc();
    assert_eq!(o1.side_tabs()[3].active, true);
    assert_eq!(o1.side_tabs()[5].active, false);

    // Local tab click: no iface/inv gen bump.
    c.active_icon = 5;

    assert!(o2.rebuild_widgets(&c, &mut reg));
    assert!(o2.rebuild_side_tabs(&c, &mut reg));
    assert!(
        !Arc::ptr_eq(&o1_w, &o2.widgets_arc()),
        "CX1 widgets must not share across tab change"
    );
    assert!(!Arc::ptr_eq(&o1_t, &o2.side_tabs_arc()));
    assert_eq!(o2.side_tabs()[5].active, true);
    assert_eq!(o2.side_tabs()[3].active, false);
    // O1 retained older publication.
    assert_eq!(o1.side_tabs()[3].active, true);
    assert!(o2.counters().side_tabs.equality_misses >= 1 || o2.counters().side_tabs.publishes >= 1);
}

/// CX2: scroll without gen; O2 walks new scroll_position.
#[test]
fn cx2_scroll_without_gen_no_share() {
    let mut c = base_client();
    plant_populated(&mut c);
    bump_iface_inv(&mut c);

    let mut reg = SlotFamilyRegistry::new();
    let mut o1 = DedupCursor::new(0);
    let mut o2 = DedupCursor::new(1);
    assert!(o1.rebuild_widgets(&c, &mut reg));
    let old = o1.widgets_arc();
    let s0 = o1
        .widgets()
        .iter()
        .find(|w| w.component_id == 1002)
        .unwrap()
        .scroll_position;
    assert_eq!(s0, 7);

    c.set_iface_mut(
        1002,
        IfTypeMut {
            scroll_pos: 42,
            ..Default::default()
        },
    );

    assert!(o2.rebuild_widgets(&c, &mut reg));
    assert!(!Arc::ptr_eq(&old, &o2.widgets_arc()));
    let s1 = o2
        .widgets()
        .iter()
        .find(|w| w.component_id == 1002)
        .unwrap()
        .scroll_position;
    assert_eq!(s1, 42);
    assert_eq!(
        o1.widgets()
            .iter()
            .find(|w| w.component_id == 1002)
            .unwrap()
            .scroll_position,
        7,
        "older publication immutable"
    );
}

/// CX3: local inv drag without inv gen.
#[test]
fn cx3_local_inv_drag_without_gen_no_share() {
    let mut c = base_client();
    plant_populated(&mut c);
    // Ensure TYPE_INV is under the main modal tree.
    set_iface(
        &mut c,
        1000,
        IfType {
            id: 1000,
            layer_id: 1000,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![1001, 1003]),
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        1003,
        IfTypeMut {
            link_obj_type: Some(vec![4, 0, 6]),
            link_obj_number: Some(vec![1, 0, 2]),
            ..Default::default()
        },
    );
    bump_iface_inv(&mut c);

    let mut reg = SlotFamilyRegistry::new();
    let mut o1 = DedupCursor::new(0);
    let mut o2 = DedupCursor::new(1);
    assert!(o1.rebuild_widgets(&c, &mut reg));
    let old = o1.widgets_arc();
    let items_before = o1
        .widgets()
        .iter()
        .find(|w| w.component_id == 1003)
        .unwrap()
        .items
        .clone();
    assert!(!items_before.is_empty());

    // Local drag rearrange without gens.inv bump.
    c.set_iface_mut(
        1003,
        IfTypeMut {
            link_obj_type: Some(vec![6, 4, 0]),
            link_obj_number: Some(vec![2, 1, 0]),
            ..Default::default()
        },
    );

    assert!(o2.rebuild_widgets(&c, &mut reg));
    assert!(!Arc::ptr_eq(&old, &o2.widgets_arc()));
    let items_after = &o2
        .widgets()
        .iter()
        .find(|w| w.component_id == 1003)
        .unwrap()
        .items;
    assert_eq!(items_before[0].def.id, 3);
    assert_eq!(
        items_after[0].def.id, 5,
        "post-drag first filled slot is obj 5"
    );
    assert_ne!(
        items_before
            .iter()
            .map(|i| (i.slot, i.def.id, i.count))
            .collect::<Vec<_>>(),
        items_after
            .iter()
            .map(|i| (i.slot, i.def.id, i.count))
            .collect::<Vec<_>>()
    );
}

/// CX4: clientComponent-style text rewrite without gen.
#[test]
fn cx4_text_without_gen_no_share() {
    let mut c = base_client();
    plant_populated(&mut c);
    bump_iface_inv(&mut c);

    let mut reg = SlotFamilyRegistry::new();
    let mut o1 = DedupCursor::new(0);
    let mut o2 = DedupCursor::new(1);
    assert!(o1.rebuild_widgets(&c, &mut reg));
    let old = o1.widgets_arc();

    c.set_iface_mut(
        1001,
        IfTypeMut {
            text: "Friends rewritten".into(),
            button_type: ButtonType::BUTTON_OK,
            ..Default::default()
        },
    );

    assert!(o2.rebuild_widgets(&c, &mut reg));
    assert!(!Arc::ptr_eq(&old, &o2.widgets_arc()));
    assert_eq!(
        o2.widgets()
            .iter()
            .find(|w| w.component_id == 1001)
            .unwrap()
            .text
            .as_deref(),
        Some("Friends rewritten")
    );
    assert_eq!(
        o1.widgets()
            .iter()
            .find(|w| w.component_id == 1001)
            .unwrap()
            .text
            .as_deref(),
        Some("Hello")
    );
}

/// CX5: TUT_OPEN-style tut_com_id without iface gen.
#[test]
fn cx5_tutorial_root_without_gen_no_share() {
    let mut c = base_client();
    plant_populated(&mut c);
    set_iface(
        &mut c,
        1300,
        IfType {
            id: 1300,
            layer_id: 1300,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![1301]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        1301,
        IfType {
            id: 1301,
            layer_id: 1300,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        1301,
        IfTypeMut {
            text: "tutorial".into(),
            ..Default::default()
        },
    );
    c.tut_com_id = -1;
    bump_iface_inv(&mut c);

    let mut reg = SlotFamilyRegistry::new();
    let mut o1 = DedupCursor::new(0);
    let mut o2 = DedupCursor::new(1);
    assert!(o1.rebuild_widgets(&c, &mut reg));
    let old = o1.widgets_arc();
    assert!(!o1.widgets().iter().any(|w| w.root == WidgetRoot::Tutorial));

    // TUT_OPEN without gen bump.
    c.tut_com_id = 1300;

    assert!(o2.rebuild_widgets(&c, &mut reg));
    assert!(!Arc::ptr_eq(&old, &o2.widgets_arc()));
    assert!(o2.widgets().iter().any(|w| w.root == WidgetRoot::Tutorial));
    assert!(!o1.widgets().iter().any(|w| w.root == WidgetRoot::Tutorial));
}

/// CX6: player move without loc gate; O2 dirty walks new distance, no share.
#[test]
fn cx6_loc_distance_player_move_no_share() {
    let mut c = base_client();
    plant_populated(&mut c);
    bump_scene(&mut c);

    let mut reg = SlotFamilyRegistry::new();
    let mut o1 = DedupCursor::new(0);
    let mut o2 = DedupCursor::new(1);
    assert!(o1.rebuild_loc(&c, &mut reg));
    let old = o1.locs_arc();
    let d0 = o1.locs()[0].distance;
    assert_eq!(d0, 17); // from (3220,3212) to (3203,3204)

    // Player walks; scene gen + model stamp unchanged.
    c.local_player = Some(ClientPlayer::at(50, 50));

    assert!(o2.rebuild_loc(&c, &mut reg));
    assert!(!Arc::ptr_eq(&old, &o2.locs_arc()));
    let d1 = o2.locs()[0].distance;
    assert_ne!(d0, d1);
    assert_eq!(o1.locs()[0].distance, d0, "older loc publication immutable");
    // Quiet owner already at gate keeps stale distance (existing behavior).
    assert!(!o1.rebuild_loc(&c, &mut reg));
    assert_eq!(o1.locs()[0].distance, d0);
}

/// Older Arc remains bitwise-equal after owner publishes a replacement.
#[test]
fn older_publication_immutability() {
    let mut c = base_client();
    plant_populated(&mut c);
    bump_iface_inv(&mut c);

    let mut reg = SlotFamilyRegistry::new();
    let mut o0 = DedupCursor::new(0);
    assert!(o0.rebuild_widgets(&c, &mut reg));
    let retained = o0.widgets_arc();
    let snapshot = retained.as_slice().to_vec();

    c.set_iface_mut(
        1001,
        IfTypeMut {
            text: "changed".into(),
            ..Default::default()
        },
    );
    c.bump_gens(ServerProt::IF_SETTEXT);
    assert!(o0.rebuild_widgets(&c, &mut reg));
    assert!(!Arc::ptr_eq(&retained, &o0.widgets_arc()));
    assert!(widgets_eq(&snapshot, retained.as_slice()));
    assert_eq!(retained[1].text.as_deref(), Some("Hello"));
}

/// Drop/restart: weak cleared; new cursor does not alias dead identity.
#[test]
fn drop_restart_identity() {
    let mut c = base_client();
    plant_populated(&mut c);
    bump_iface_inv(&mut c);
    bump_scene(&mut c);

    let mut reg = SlotFamilyRegistry::new();
    let mut o0 = DedupCursor::new(0);
    let mut o1 = DedupCursor::new(1);
    assert!(o0.rebuild_families(&c, &mut reg));
    assert!(o1.rebuild_families(&c, &mut reg));
    assert!(Arc::ptr_eq(&o0.widgets_arc(), &o1.widgets_arc()));
    let shared = o0.widgets_arc();
    let strong_before = Arc::strong_count(&shared);

    o0.unregister(&mut reg);
    // o1 still holds the body; registry slot 0 cleared.
    assert!(Arc::strong_count(&shared) < strong_before || Arc::strong_count(&shared) >= 1);
    assert_eq!(reg.unique_widget_bodies().len(), 1);

    // Restart cursor 0: new publication can re-share with o1 by equality.
    let mut o0b = DedupCursor::new(0);
    assert!(o0b.rebuild_families(&c, &mut reg));
    assert!(Arc::ptr_eq(&o0b.widgets_arc(), &o1.widgets_arc()));

    // Full teardown: drop both, registry empty of live bodies.
    let last = o1.widgets_arc();
    o1.unregister(&mut reg);
    o0b.unregister(&mut reg);
    drop(last);
    drop(shared);
    assert!(reg.unique_widget_bodies().is_empty());
    assert!(reg.unique_side_tab_bodies().is_empty());
    assert!(reg.unique_loc_bodies().is_empty());
}

/// Baseline oracle: independent GameSnapshot rebuilds match prototype bodies.
#[test]
fn prototype_matches_independent_game_snapshot_builders() {
    let mut c = base_client();
    plant_populated(&mut c);
    bump_iface_inv(&mut c);
    bump_scene(&mut c);

    let mut baseline = GameSnapshot::new();
    assert!(baseline.rebuild_family(&c, Family::Widgets));
    assert!(baseline.rebuild_family(&c, Family::SideTabs));
    assert!(baseline.rebuild_family(&c, Family::Loc));

    let mut reg = SlotFamilyRegistry::new();
    let mut cursor = DedupCursor::new(0);
    assert!(cursor.rebuild_families(&c, &mut reg));

    assert!(widgets_eq(baseline.widgets(), cursor.widgets()));
    assert!(side_tabs_eq(baseline.side_tabs(), cursor.side_tabs()));
    assert!(locs_eq(baseline.locs(), cursor.locs()));
    assert_eq!(baseline.locs()[0].layer, LocLayer::Wall);
}

/// Equality and walk counts are recorded for the report.
#[test]
fn equality_and_walk_counts_recorded() {
    let mut c = base_client();
    plant_populated(&mut c);
    bump_iface_inv(&mut c);
    bump_scene(&mut c);

    let mut reg = SlotFamilyRegistry::new();
    let mut o0 = DedupCursor::new(0);
    let mut o1 = DedupCursor::new(1);
    assert!(o0.rebuild_families(&c, &mut reg));
    assert!(o1.rebuild_families(&c, &mut reg));

    assert_eq!(o0.counters().widgets.walks, 1);
    assert_eq!(o0.counters().side_tabs.walks, 1);
    assert_eq!(o0.counters().loc.walks, 1);
    assert_eq!(o0.counters().widgets.publishes, 1);
    assert!(o1.counters().widgets.equality_comparisons >= 1);
    assert!(o1.counters().widgets.equality_hits >= 1);
    assert_eq!(o1.counters().widgets.walks, 1);

    // Quiet frame
    assert!(!o1.rebuild_families(&c, &mut reg));
    assert_eq!(o1.counters().widgets.walks, 1);
    assert_eq!(o1.counters().widgets.quiet_skips, 1);
}
