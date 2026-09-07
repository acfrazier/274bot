//! Production snapshot-dedup candidate tests (feature `snapshot-dedup`).
//! Empty under feature-off so default `cargo test -p api` stays green.

#![cfg(feature = "snapshot-dedup")]

use api::snapshot::{Family, GameSnapshot, WidgetView};
use api::snapshot_dedup::{
    account_allocations_arcs, attach_owner_for_slot, process_slot_table, widgets_eq,
    SlotFamilyRegistry,
};
use client::client::{Client, ClientConfig};
use client::config::if_type::{ComponentType, IfType, IfTypeMut};
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

/// Populated widgets + side tabs + one loc (from approved prototype fixture).
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

fn bump_iface_inv(c: &mut Client) {
    c.bump_gens(ServerProt::IF_SETTEXT);
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
}

fn bump_scene(c: &mut Client) {
    c.bump_gens(ServerProt::REBUILD_NORMAL);
}

fn owner_pair(slot: &str) -> (GameSnapshot, GameSnapshot) {
    let mut a = GameSnapshot::new();
    let mut b = GameSnapshot::new();
    a.attach_dedup(attach_owner_for_slot(slot));
    b.attach_dedup(attach_owner_for_slot(slot));
    (a, b)
}

fn bare_widget(component_id: i32, text: &str) -> WidgetView {
    WidgetView {
        kind: api::snapshot::WidgetKind::Widget,
        component_id,
        layer_id: 0,
        parent_id: -1,
        root_component_id: 0,
        root: api::snapshot::WidgetRoot::Main,
        type_: 0,
        button_type: 0,
        client_code: 0,
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        scroll_height: 0,
        scroll_position: 0,
        hidden: false,
        text: Some(text.into()),
        alternate_text: None,
        button_text: None,
        target_verb: None,
        target_base: None,
        target_mask: 0,
        model_type: 0,
        model_id: 0,
        alternate_model_type: 0,
        alternate_model_id: 0,
        scripts: None,
        script_comparators: None,
        script_operands: None,
        varp_bindings: vec![],
        colour: 0,
        actions: vec![],
        items: vec![],
    }
}

#[test]
fn registry_equal_bodies_share_arc_and_unequal_do_not() {
    let mut reg = SlotFamilyRegistry::new();
    let c0 = reg.register();
    let c1 = reg.register();
    let mut counters = Default::default();

    let body = vec![bare_widget(1, "a")];
    let a = reg.intern_widgets(c0, body.clone(), &mut counters);
    let b = reg.intern_widgets(c1, body, &mut counters);
    assert!(Arc::ptr_eq(&a, &b), "equal content must share Arc");
    assert_eq!(counters.equality_hits, 1);
    assert_eq!(counters.publishes, 1);

    let other = vec![bare_widget(2, "b")];
    let c = reg.intern_widgets(c1, other, &mut counters);
    assert!(!Arc::ptr_eq(&a, &c), "unequal content must not share");
    assert_eq!(counters.equality_misses, 1);
    assert_eq!(counters.publishes, 2);
}

#[test]
fn quiet_rebuild_skips_equality_and_walks() {
    let mut c = base_client();
    plant_populated(&mut c);
    bump_iface_inv(&mut c);
    bump_scene(&mut c);

    let mut snap = GameSnapshot::new();
    snap.attach_dedup(attach_owner_for_slot("quiet-prod"));
    assert!(snap.rebuild_family(&c, Family::Widgets));
    assert!(snap.rebuild_family(&c, Family::SideTabs));
    assert!(snap.rebuild_family(&c, Family::Loc));
    let walks_after = snap.dedup_counters().walks();
    let eq_after = snap.dedup_counters().equality_comparisons();
    assert!(walks_after >= 3);

    assert!(!snap.rebuild_family(&c, Family::Widgets));
    assert!(!snap.rebuild_family(&c, Family::SideTabs));
    assert!(!snap.rebuild_family(&c, Family::Loc));
    assert_eq!(snap.dedup_counters().walks(), walks_after);
    assert_eq!(snap.dedup_counters().equality_comparisons(), eq_after);
    let quiet = snap.dedup_counters().widgets.quiet_skips
        + snap.dedup_counters().side_tabs.quiet_skips
        + snap.dedup_counters().loc.quiet_skips;
    assert!(quiet >= 3);
}

#[test]
fn two_owners_equal_rebuild_share_family_arcs() {
    let slot = "share-prod";
    process_slot_table().remove(slot);
    let mut c = base_client();
    plant_populated(&mut c);
    bump_iface_inv(&mut c);
    bump_scene(&mut c);

    let (mut a, mut b) = owner_pair(slot);
    assert!(a.rebuild_family(&c, Family::Widgets));
    assert!(a.rebuild_family(&c, Family::SideTabs));
    assert!(a.rebuild_family(&c, Family::Loc));
    assert!(b.rebuild_family(&c, Family::Widgets));
    assert!(b.rebuild_family(&c, Family::SideTabs));
    assert!(b.rebuild_family(&c, Family::Loc));

    assert!(!a.widgets().is_empty());
    assert!(!a.side_tabs().is_empty());
    assert!(!a.locs().is_empty());
    assert!(Arc::ptr_eq(&a.widgets_arc(), &b.widgets_arc()));
    assert!(Arc::ptr_eq(&a.side_tabs_arc(), &b.side_tabs_arc()));
    assert!(Arc::ptr_eq(&a.locs_arc(), &b.locs_arc()));
    process_slot_table().remove(slot);
}

#[test]
fn two_owners_unequal_widgets_do_not_share() {
    let slot = "unequal-prod";
    process_slot_table().remove(slot);
    let mut c = base_client();
    plant_populated(&mut c);
    bump_iface_inv(&mut c);

    let (mut a, mut b) = owner_pair(slot);
    assert!(a.rebuild_family(&c, Family::Widgets));

    set_iface_mut(
        &mut c,
        1001,
        IfTypeMut {
            text: "Changed".into(),
            colour: 0xFF0000,
            ..Default::default()
        },
    );
    bump_iface_inv(&mut c);
    assert!(b.rebuild_family(&c, Family::Widgets));
    assert!(!Arc::ptr_eq(&a.widgets_arc(), &b.widgets_arc()));
    assert!(!widgets_eq(a.widgets(), b.widgets()));
    process_slot_table().remove(slot);
}

#[test]
fn owner_teardown_unregisters_and_allows_reregister() {
    let slot = "teardown-prod";
    process_slot_table().remove(slot);
    {
        let mut a = GameSnapshot::new();
        a.attach_dedup(attach_owner_for_slot(slot));
        let reg = process_slot_table().get(slot).expect("installed");
        assert_eq!(reg.lock().unwrap().live_owner_count(), 1);
        drop(a);
        assert_eq!(reg.lock().unwrap().live_owner_count(), 0);
    }
    let mut b = GameSnapshot::new();
    b.attach_dedup(attach_owner_for_slot(slot));
    let reg = process_slot_table().get(slot).expect("reinstalled");
    assert_eq!(reg.lock().unwrap().live_owner_count(), 1);
    process_slot_table().remove(slot);
}

#[test]
fn allocation_account_counts_unique_arcs_not_per_owner_copies() {
    let mut reg = SlotFamilyRegistry::new();
    let c0 = reg.register();
    let c1 = reg.register();
    let c2 = reg.register();
    let mut counters = Default::default();
    let body: Vec<WidgetView> = (0..20).map(|i| bare_widget(i, &format!("w{i}"))).collect();
    let a = reg.intern_widgets(c0, body.clone(), &mut counters);
    let b = reg.intern_widgets(c1, body.clone(), &mut counters);
    let c = reg.intern_widgets(c2, body, &mut counters);
    assert!(Arc::ptr_eq(&a, &b) && Arc::ptr_eq(&b, &c));

    let empty_side = Arc::new(Vec::new());
    let empty_loc = Arc::new(Vec::new());
    let holders = [
        (
            Arc::clone(&a),
            Arc::clone(&empty_side),
            Arc::clone(&empty_loc),
        ),
        (
            Arc::clone(&b),
            Arc::clone(&empty_side),
            Arc::clone(&empty_loc),
        ),
        (
            Arc::clone(&c),
            Arc::clone(&empty_side),
            Arc::clone(&empty_loc),
        ),
    ];
    let acct = account_allocations_arcs(&holders, &reg, 0);
    assert_eq!(acct.live_owner_count, 3);
    assert!(
        acct.unique_body_payload_bytes < acct.old_per_owner_payload_bytes,
        "unique={} old={}",
        acct.unique_body_payload_bytes,
        acct.old_per_owner_payload_bytes
    );
    assert_eq!(reg.unique_widget_bodies().len(), 1);
}

#[test]
fn process_table_isolates_slots() {
    process_slot_table().remove("slot-a");
    process_slot_table().remove("slot-b");
    let ha = attach_owner_for_slot("slot-a");
    let hb = attach_owner_for_slot("slot-b");
    assert!(!Arc::ptr_eq(&ha.registry, &hb.registry));
    process_slot_table().remove("slot-a");
    process_slot_table().remove("slot-b");
}
