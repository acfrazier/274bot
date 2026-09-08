//! Slot-instance family body sharing tests (feature `snapshot-dedup`).
//!
//! Three corrective roots: true slot-instance ownership, exclusive
//! capacity/header/scratch accounting, multi-owner CX/lifecycle coverage.

#![cfg(feature = "snapshot-dedup")]

use api::snapshot::{LocLayer, LocView, SideTabView, WidgetKind, WidgetRoot, WidgetView, WorldTile};
use api::snapshot_dedup::{
    account_allocations_arcs, account_registry_live, widgets_eq, widgets_payload_bytes_vec,
    AllocationAccount, DedupHandle, FamilyCounters, SlotDedupDirectory, SlotDedupInstance,
    SlotFamilyRegistry,
};
use api::snapshot::GameSnapshot;
use std::sync::{Arc, Mutex};

fn w(component_id: i32) -> WidgetView {
    WidgetView {
        kind: WidgetKind::Widget,
        component_id,
        layer_id: 0,
        parent_id: -1,
        root_component_id: component_id,
        root: WidgetRoot::Main,
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
        text: None,
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
        varp_bindings: Vec::new(),
        colour: 0,
        actions: Vec::new(),
        items: Vec::new(),
    }
}

fn st(index: i32) -> SideTabView {
    SideTabView {
        index,
        root_component_id: index,
        available: true,
        active: false,
        visible: true,
        widgets: vec![w(index)],
    }
}

fn loc(id: i32) -> LocView {
    LocView {
        typecode: 0,
        info: 0,
        id,
        name: Some(format!("loc-{id}")),
        description: None,
        actions: Vec::new(),
        tile: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        distance: 0,
        layer: LocLayer::Ground,
        shape: 0,
        angle: 0,
        width: 1,
        length: 1,
        footprint_width: 1,
        footprint_length: 1,
        block_walk: false,
        block_range: false,
        active: true,
        animation: -1,
        map_function: -1,
        map_scene: -1,
        force_approach: -1,
    }
}

fn attach_pair(inst: &SlotDedupInstance) -> (GameSnapshot, GameSnapshot) {
    let mut a = GameSnapshot::new();
    let mut b = GameSnapshot::new();
    a.attach_dedup(inst.attach());
    b.attach_dedup(inst.attach());
    (a, b)
}

fn put_widgets(s: &mut GameSnapshot, body: Vec<WidgetView>) {
    s.proto_put_widgets(body);
}

fn put_side_tabs(s: &mut GameSnapshot, body: Vec<SideTabView>) {
    s.proto_put_side_tabs(body);
}

fn put_locs(s: &mut GameSnapshot, body: Vec<LocView>) {
    s.proto_put_locs(body);
}

#[test]
fn widgets_eq_is_byte_exact() {
    assert!(widgets_eq(&[w(1), w(2)], &[w(1), w(2)]));
    let mut x = w(1);
    x.x = 9;
    assert!(!widgets_eq(&[x], &[w(1)]));
}

#[test]
fn two_owners_share_identical_widgets_body() {
    let inst = SlotDedupInstance::new();
    let (mut a, mut b) = attach_pair(&inst);
    let body = vec![w(1), w(2), w(3)];
    put_widgets(&mut a, body.clone());
    put_widgets(&mut b, body);
    a.intern_widgets_for_test();
    b.intern_widgets_for_test();
    assert!(
        Arc::ptr_eq(&a.widgets_arc(), &b.widgets_arc()),
        "identical widgets must share one Arc body"
    );
    assert_eq!(a.dedup_counters().widgets.equality_hits, 0);
    assert_eq!(b.dedup_counters().widgets.equality_hits, 1);
    assert_eq!(b.dedup_counters().widgets.equality_comparisons, 1);
}

#[test]
fn two_owners_keep_distinct_bodies_on_miss() {
    let inst = SlotDedupInstance::new();
    let (mut a, mut b) = attach_pair(&inst);
    put_widgets(&mut a, vec![w(1)]);
    put_widgets(&mut b, vec![w(2)]);
    a.intern_widgets_for_test();
    b.intern_widgets_for_test();
    assert!(!Arc::ptr_eq(&a.widgets_arc(), &b.widgets_arc()));
    assert_eq!(b.dedup_counters().widgets.equality_misses, 1);
    assert_eq!(b.dedup_counters().widgets.publishes, 1);
}

#[test]
fn side_tabs_and_loc_share_on_hit() {
    let inst = SlotDedupInstance::new();
    let (mut a, mut b) = attach_pair(&inst);
    put_side_tabs(&mut a, vec![st(10), st(11)]);
    put_side_tabs(&mut b, vec![st(10), st(11)]);
    put_locs(&mut a, vec![loc(100)]);
    put_locs(&mut b, vec![loc(100)]);
    a.intern_side_tabs_for_test();
    b.intern_side_tabs_for_test();
    a.intern_loc_for_test();
    b.intern_loc_for_test();
    assert!(Arc::ptr_eq(&a.side_tabs_arc(), &b.side_tabs_arc()));
    assert!(Arc::ptr_eq(&a.locs_arc(), &b.locs_arc()));
}

#[test]
fn cursor_unregister_releases_weak_and_allows_reclaim() {
    let inst = SlotDedupInstance::new();
    let mut a = GameSnapshot::new();
    a.attach_dedup(inst.attach());
    put_widgets(&mut a, vec![w(1); 8]);
    a.intern_widgets_for_test();
    let body = a.widgets_arc();
    assert_eq!(Arc::strong_count(&body), 2);
    drop(a);
    assert_eq!(Arc::strong_count(&body), 1);
    let reg = inst.registry();
    let g = reg.lock().unwrap();
    assert_eq!(g.live_owner_count(), 0);
    assert!(g.unique_widget_bodies().is_empty());
}

#[test]
fn allocation_account_counts_headers_and_duplicate_savings() {
    let reg = Arc::new(Mutex::new(SlotFamilyRegistry::new()));
    let ha = DedupHandle::additional_owner(&reg);
    let hb = DedupHandle::additional_owner(&reg);
    let (shared, hit) = {
        let mut g = reg.lock().unwrap();
        let mut c = FamilyCounters::default();
        let shared = g.intern_widgets(ha.cursor, vec![w(1), w(2), w(3)], &mut c);
        let hit = g.intern_widgets(hb.cursor, vec![w(1), w(2), w(3)], &mut c);
        assert!(Arc::ptr_eq(&shared, &hit));
        (shared, hit)
    };
    let empty_s = Arc::new(Vec::new());
    let empty_l = Arc::new(Vec::new());
    let holders = vec![
        (
            Arc::clone(&shared),
            Arc::clone(&empty_s),
            Arc::clone(&empty_l),
        ),
        (hit, empty_s, empty_l),
    ];
    let g = reg.lock().unwrap();
    let acct = account_allocations_arcs(&holders, &g, None);
    assert!(acct.old_per_owner_payload_bytes > 0);
    assert_eq!(acct.unique_body_count, 1);
    assert!(acct.unique_body_payload_bytes > 0);
    assert!(acct.arc_header_bytes > 0);
    assert!(acct.registry_metadata_bytes > 0);
    assert!(
        acct.duplicate_nested_payload_bytes > 0,
        "expected duplicate savings, got {acct:?}"
    );
    assert_eq!(acct.scratch_peak_bytes, None);
}

#[test]
fn directory_isolates_same_username_across_plays() {
    let d1 = SlotDedupDirectory::new_shared();
    let d2 = SlotDedupDirectory::new_shared();
    let i1 = SlotDedupInstance::new();
    let i2 = SlotDedupInstance::new();
    d1.install("alice", i1.clone());
    d2.install("alice", i2.clone());
    assert!(!Arc::ptr_eq(&i1.registry(), &i2.registry()));
    assert!(Arc::ptr_eq(
        &d1.get("alice").unwrap().registry(),
        &i1.registry()
    ));
    d1.remove("alice");
    assert!(d1.get("alice").is_none());
    assert!(d2.get("alice").is_some());
}

#[test]
fn slot_restart_replaces_instance_without_process_global_leak() {
    let dir = SlotDedupDirectory::new_shared();
    let first = SlotDedupInstance::new();
    dir.install("bob", first.clone());
    let mut snap = GameSnapshot::new();
    snap.attach_dedup(first.attach());
    put_widgets(&mut snap, vec![w(42)]);
    snap.intern_widgets_for_test();
    drop(snap);
    dir.remove("bob");
    assert!(dir.get("bob").is_none());

    let second = SlotDedupInstance::new();
    dir.install("bob", second.clone());
    assert!(!Arc::ptr_eq(&first.registry(), &second.registry()));
    let mut snap2 = GameSnapshot::new();
    snap2.attach_dedup(second.attach());
    put_widgets(&mut snap2, vec![w(42)]);
    snap2.intern_widgets_for_test();
    assert_eq!(snap2.dedup_counters().widgets.equality_hits, 0);
    assert_eq!(snap2.dedup_counters().widgets.publishes, 1);
}

#[test]
fn diagnostic_json_reports_cursors_and_accounting_fields() {
    let inst = SlotDedupInstance::new();
    let mut a = GameSnapshot::new();
    let mut b = GameSnapshot::new();
    a.attach_dedup(inst.attach());
    b.attach_dedup(inst.attach());
    put_widgets(&mut a, vec![w(1); 4]);
    put_widgets(&mut b, vec![w(1); 4]);
    a.intern_widgets_for_test();
    b.intern_widgets_for_test();

    let dir = SlotDedupDirectory::new_shared();
    dir.install("cx", inst);
    // Unmeasured scratch: one census for slots + aggregate (not two samples).
    let (rows, agg_unmeasured) = dir.census_sample(None);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].slot_name, "cx");
    assert_eq!(rows[0].live_owner_count, 2);
    assert_eq!(rows[0].unique_body_count, 1);
    assert!(rows[0].duplicate_nested_payload_bytes > 0);
    assert!(rows[0].widgets.equality_hits >= 1);
    assert_eq!(rows[0].scratch_peak_bytes, None);
    assert_eq!(agg_unmeasured.scratch_peak_bytes, None);
    let v = serde_json::to_value(&rows).expect("serialize");
    assert!(v[0].get("slot_name").is_some());
    assert!(v[0].get("arc_header_bytes").is_some());
    assert!(v[0].get("unique_body_payload_bytes").is_some());
    assert!(v[0]["widgets"].get("equality_hits").is_some());
    // Honest null, not silent 0.
    assert!(v[0].get("scratch_peak_bytes").unwrap().is_null());
    let (rows_m, agg) = dir.census_sample(Some(64));
    assert_eq!(rows_m[0].scratch_peak_bytes, Some(64));
    assert_eq!(agg.scratch_peak_bytes, Some(64));
    assert!(agg.duplicate_nested_payload_bytes > 0);
    assert_eq!(
        agg.duplicate_nested_payload_bytes,
        rows_m
            .iter()
            .map(|r| r.duplicate_nested_payload_bytes)
            .sum::<usize>()
    );
}

/// CX1: two owners, identical widgets → one unique body, positive savings.
#[test]
fn cx1_two_owners_identical_widgets_savings() {
    let inst = SlotDedupInstance::new();
    let (mut a, mut b) = attach_pair(&inst);
    let body = vec![w(7); 16];
    put_widgets(&mut a, body.clone());
    put_widgets(&mut b, body);
    a.intern_widgets_for_test();
    b.intern_widgets_for_test();
    let empty_s = Arc::new(Vec::new());
    let empty_l = Arc::new(Vec::new());
    let holders = vec![
        (a.widgets_arc(), Arc::clone(&empty_s), Arc::clone(&empty_l)),
        (b.widgets_arc(), empty_s, empty_l),
    ];
    let g = inst.registry().lock().unwrap();
    let acct = account_allocations_arcs(&holders, &g, None);
    assert_eq!(acct.unique_body_count, 1);
    assert!(acct.duplicate_nested_payload_bytes > 0);
}

/// CX2: three owners, two share / one diverges.
#[test]
fn cx2_three_owners_partial_share() {
    let inst = SlotDedupInstance::new();
    let mut a = GameSnapshot::new();
    let mut b = GameSnapshot::new();
    let mut c = GameSnapshot::new();
    a.attach_dedup(inst.attach());
    b.attach_dedup(inst.attach());
    c.attach_dedup(inst.attach());
    put_widgets(&mut a, vec![w(1), w(2)]);
    put_widgets(&mut b, vec![w(1), w(2)]);
    put_widgets(&mut c, vec![w(9)]);
    a.intern_widgets_for_test();
    b.intern_widgets_for_test();
    c.intern_widgets_for_test();
    assert!(Arc::ptr_eq(&a.widgets_arc(), &b.widgets_arc()));
    assert!(!Arc::ptr_eq(&a.widgets_arc(), &c.widgets_arc()));
    let g = inst.registry().lock().unwrap();
    assert_eq!(g.unique_widget_bodies().len(), 2);
}

/// CX3: multi-family concurrent share (widgets + side_tabs + loc).
#[test]
fn cx3_multi_family_share() {
    let inst = SlotDedupInstance::new();
    let (mut a, mut b) = attach_pair(&inst);
    put_widgets(&mut a, vec![w(1)]);
    put_widgets(&mut b, vec![w(1)]);
    put_side_tabs(&mut a, vec![st(2)]);
    put_side_tabs(&mut b, vec![st(2)]);
    put_locs(&mut a, vec![loc(3)]);
    put_locs(&mut b, vec![loc(3)]);
    a.intern_widgets_for_test();
    b.intern_widgets_for_test();
    a.intern_side_tabs_for_test();
    b.intern_side_tabs_for_test();
    a.intern_loc_for_test();
    b.intern_loc_for_test();
    assert!(Arc::ptr_eq(&a.widgets_arc(), &b.widgets_arc()));
    assert!(Arc::ptr_eq(&a.side_tabs_arc(), &b.side_tabs_arc()));
    assert!(Arc::ptr_eq(&a.locs_arc(), &b.locs_arc()));
}

/// CX4: miss path keeps independent bodies + miss counters.
#[test]
fn cx4_miss_keeps_independent_bodies() {
    let inst = SlotDedupInstance::new();
    let (mut a, mut b) = attach_pair(&inst);
    put_widgets(&mut a, vec![w(1)]);
    put_widgets(&mut b, vec![w(2)]);
    a.intern_widgets_for_test();
    b.intern_widgets_for_test();
    assert_eq!(b.dedup_counters().widgets.equality_misses, 1);
    assert!(!Arc::ptr_eq(&a.widgets_arc(), &b.widgets_arc()));
}

/// CX5: directory remove + reinstall is a clean lifetime boundary.
#[test]
fn cx5_directory_lifetime_boundary() {
    let dir = SlotDedupDirectory::new_shared();
    let a = SlotDedupInstance::new();
    dir.install("s", a.clone());
    dir.remove("s");
    let b = SlotDedupInstance::new();
    dir.install("s", b.clone());
    assert!(!Arc::ptr_eq(&a.registry(), &b.registry()));
}

/// CX6: allocation account includes capacity-aware payload + arc headers (not RSS).
#[test]
fn cx6_account_has_capacity_headers_not_rss_claims() {
    let inst = SlotDedupInstance::new();
    let (mut a, mut b) = attach_pair(&inst);
    let mut body = Vec::with_capacity(64);
    body.extend([w(1), w(2), w(3)]);
    put_widgets(&mut a, body.clone());
    put_widgets(&mut b, body);
    a.intern_widgets_for_test();
    b.intern_widgets_for_test();
    let empty_s = Arc::new(Vec::new());
    let empty_l = Arc::new(Vec::new());
    let holders = vec![
        (a.widgets_arc(), Arc::clone(&empty_s), Arc::clone(&empty_l)),
        (b.widgets_arc(), empty_s, empty_l),
    ];
    let g = inst.registry().lock().unwrap();
    let acct: AllocationAccount = account_allocations_arcs(&holders, &g, Some(128));
    // Capacity-aware unique payload (widgets_payload_bytes_vec uses capacity).
    assert!(acct.unique_body_payload_bytes >= 3 * std::mem::size_of::<WidgetView>());
    assert!(acct.arc_header_bytes > 0);
    assert_eq!(acct.scratch_peak_bytes, Some(128));
    let json = serde_json::to_value(&acct).unwrap();
    assert!(json.get("rss").is_none());
    assert!(json.get("unique_body_payload_bytes").is_some());
    assert!(json.get("arc_header_bytes").is_some());
    assert!(json.get("weak_slot_bytes").is_some());
    assert_eq!(json.get("scratch_peak_bytes").unwrap().as_u64(), Some(128));
}

/// Two distinct bodies with different capacities → zero duplicate payload.
#[test]
fn census_two_distinct_bodies_zero_duplicate_payload() {
    let inst = SlotDedupInstance::new();
    let (mut a, mut b) = attach_pair(&inst);
    let mut body_a = Vec::with_capacity(8);
    body_a.push(w(1));
    let mut body_b = Vec::with_capacity(64);
    body_b.push(w(2));
    put_widgets(&mut a, body_a);
    put_widgets(&mut b, body_b);
    a.intern_widgets_for_test();
    b.intern_widgets_for_test();
    assert!(!Arc::ptr_eq(&a.widgets_arc(), &b.widgets_arc()));

    let g = inst.registry().lock().unwrap();
    let acct = account_registry_live(&g, None, Some(g.counters()));
    let p_a = widgets_payload_bytes_vec(a.widgets_arc().as_ref());
    let p_b = widgets_payload_bytes_vec(b.widgets_arc().as_ref());
    assert_ne!(p_a, p_b, "fixture requires asymmetric capacities");
    // Only widgets published: two unique widget bodies.
    assert_eq!(g.unique_widget_bodies().len(), 2);
    assert_eq!(acct.unique_body_count, 2);
    assert_eq!(acct.unique_body_payload_bytes, p_a + p_b);
    assert_eq!(acct.old_per_owner_payload_bytes, p_a + p_b);
    assert_eq!(acct.duplicate_nested_payload_bytes, 0);
    assert_eq!(acct.live_owner_count, 2);
    assert_eq!(acct.scratch_peak_bytes, None);
}

/// Shared + distinct three-owner case: exact private / unique / duplicate math.
#[test]
fn census_three_owners_shared_plus_distinct_exact() {
    let inst = SlotDedupInstance::new();
    let mut a = GameSnapshot::new();
    let mut b = GameSnapshot::new();
    let mut c = GameSnapshot::new();
    a.attach_dedup(inst.attach());
    b.attach_dedup(inst.attach());
    c.attach_dedup(inst.attach());
    let shared = vec![w(1), w(2)];
    put_widgets(&mut a, shared.clone());
    put_widgets(&mut b, shared);
    put_widgets(&mut c, vec![w(9)]);
    a.intern_widgets_for_test();
    b.intern_widgets_for_test();
    c.intern_widgets_for_test();
    assert!(Arc::ptr_eq(&a.widgets_arc(), &b.widgets_arc()));
    assert!(!Arc::ptr_eq(&a.widgets_arc(), &c.widgets_arc()));

    let p_shared = widgets_payload_bytes_vec(a.widgets_arc().as_ref());
    let p_div = widgets_payload_bytes_vec(c.widgets_arc().as_ref());
    let g = inst.registry().lock().unwrap();
    let acct = account_registry_live(&g, Some(32), Some(g.counters()));
    assert_eq!(acct.live_owner_count, 3);
    assert_eq!(acct.unique_body_count, 2);
    assert_eq!(acct.unique_body_payload_bytes, p_shared + p_div);
    // Private counterfactual: shared body × 2 owners + distinct × 1.
    assert_eq!(acct.old_per_owner_payload_bytes, p_shared * 2 + p_div);
    assert_eq!(acct.duplicate_nested_payload_bytes, p_shared);
    assert_eq!(acct.scratch_peak_bytes, Some(32));
}

/// Dropped cursor / dead weak cleanup: census follows real remaining owners only.
#[test]
fn census_dropped_cursor_and_dead_weak_cleanup() {
    let inst = SlotDedupInstance::new();
    let mut a = GameSnapshot::new();
    let mut b = GameSnapshot::new();
    a.attach_dedup(inst.attach());
    b.attach_dedup(inst.attach());
    put_widgets(&mut a, vec![w(1); 4]);
    put_widgets(&mut b, vec![w(1); 4]);
    a.intern_widgets_for_test();
    b.intern_widgets_for_test();
    assert!(Arc::ptr_eq(&a.widgets_arc(), &b.widgets_arc()));
    let p = widgets_payload_bytes_vec(a.widgets_arc().as_ref());
    {
        let g = inst.registry().lock().unwrap();
        let before = account_registry_live(&g, None, None);
        assert_eq!(before.live_owner_count, 2);
        assert_eq!(before.unique_body_count, 1);
        assert_eq!(before.old_per_owner_payload_bytes, p * 2);
        assert_eq!(before.duplicate_nested_payload_bytes, p);
    }
    // Drop owner a (cursor unregister on GameSnapshot drop).
    drop(a);
    {
        let g = inst.registry().lock().unwrap();
        let after = account_registry_live(&g, None, None);
        assert_eq!(after.live_owner_count, 1);
        assert_eq!(after.unique_body_count, 1);
        assert_eq!(after.unique_body_payload_bytes, p);
        assert_eq!(after.old_per_owner_payload_bytes, p);
        assert_eq!(after.duplicate_nested_payload_bytes, 0);
    }
    // Drop last owner body: dead weaks must not invent owners or payload.
    drop(b);
    {
        let g = inst.registry().lock().unwrap();
        let empty = account_registry_live(&g, None, None);
        assert_eq!(empty.live_owner_count, 0);
        assert_eq!(empty.unique_body_count, 0);
        assert_eq!(empty.unique_body_payload_bytes, 0);
        assert_eq!(empty.old_per_owner_payload_bytes, 0);
        assert_eq!(empty.duplicate_nested_payload_bytes, 0);
        assert!(g.unique_widget_bodies().is_empty());
    }
}
