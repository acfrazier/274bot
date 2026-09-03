// Task 7: typed query filters — the extension traits over `Query<T>`
// (one per entity family), plus `SceneQuery`, `WidgetSearch` and
// `LocApproach`. Views are built directly here (fixtures); the
// widget-search tests rebuild a real `GameSnapshot` from a client iface
// tree (the `snapshot` test pattern).

use api::query::*;
use api::snapshot::{
    ActorKind, ActorTargetView, ActorView, ChatLineView, Family, GameSnapshot, GroundItemView,
    ItemActionFamily, ItemContainer, ItemView, LocLayer, LocView, LocalTile, NpcView, PlayerView,
    SceneView, SideTabView, StatView, VarpView, WidgetKind, WidgetRoot, WidgetView, WorldTile,
};
use api::ItemDefView;
use client::client::{Client, ClientConfig};
use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeMut};
use client::dash3d::CollisionFlag;
use client::io::ServerProt;

fn fixture_item(id: i32, count: i32) -> ItemView {
    ItemView {
        def: ItemDefView {
            id,
            name: Some(format!("item {id}")),
            stackable: count > 1,
            members: false,
            base_value: id * 10,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        container: ItemContainer::Inventory,
        action_family: ItemActionFamily::Held,
        slot: 0,
        count,
        actions: vec![Some("Take".into())],
        component_id: -1,
    }
}

#[allow(clippy::too_many_arguments)]
fn fixture_npc(
    index: usize,
    id: i32,
    name: &str,
    x: i32,
    z: i32,
    distance: i32,
    health: i32,
) -> NpcView {
    NpcView {
        index,
        r#type: Some(id as usize),
        name: Some(name.to_string()),
        actions: vec![Some("Attack".into())],
        tile: WorldTile { x, z, level: 0 },
        distance,
        animation: 0,
        pose_animation: 0,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        health,
        total_health: 5,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
        level: 2,
        size: 1,
        x,
        z,
        yaw: 0,
    }
}

fn fixture_player(index: usize, combat_level: i32, skill_level: i32) -> PlayerView {
    PlayerView {
        index,
        actor: ActorView {
            name: Some("Player".into()),
            actions: vec![Some("Attack".into())],
            tile: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            distance: 0,
            animation: 0,
            pose_animation: 0,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            health: 10,
            total_health: 10,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
        },
        combat_level,
        skill_level,
    }
}

fn fixture_ground_item(
    id: i32,
    name: &str,
    count: i32,
    stackable: bool,
    members: bool,
    x: i32,
    z: i32,
) -> GroundItemView {
    GroundItemView {
        def: ItemDefView {
            id,
            name: Some(name.into()),
            stackable,
            members,
            base_value: id * 10,
            noted: id == 4,
            certificate_link: -1,
            certificate_template: -1,
        },
        count,
        actions: vec![Some("Take".into())],
        tile: WorldTile { x, z, level: 0 },
        distance: 0,
    }
}

fn fixture_loc(shape: i32, angle: i32, x: i32, z: i32, w: i32, l: i32) -> LocView {
    LocView {
        typecode: 0,
        info: 0,
        id: 1,
        name: Some("Tree".into()),
        description: None,
        actions: vec![Some("Chop down".into())],
        tile: WorldTile {
            x: 3200 + x,
            z: 3200 + z,
            level: 0,
        },
        distance: 0,
        layer: LocLayer::Ground,
        shape,
        angle,
        width: w,
        length: l,
        footprint_width: w,
        footprint_length: l,
        block_walk: true,
        block_range: true,
        active: true,
        animation: -1,
        map_function: -1,
        map_scene: -1,
        force_approach: 0,
    }
}

fn fixture_stat(
    index: i32,
    name: &str,
    effective: i32,
    base: i32,
    xp: i32,
    used: bool,
) -> StatView {
    StatView {
        index,
        name: name.into(),
        effective,
        base,
        xp,
        used,
    }
}

fn fixture_varp(index: i32, value: i32) -> VarpView {
    VarpView { index, value }
}

fn fixture_widget(component_id: i32, button_type: i32, text: Option<&str>, y: i32) -> WidgetView {
    WidgetView {
        kind: WidgetKind::Widget,
        component_id,
        layer_id: 1000,
        parent_id: -1,
        root_component_id: 1000,
        root: WidgetRoot::Main,
        type_: 0,
        button_type,
        client_code: 0,
        x: 0,
        y,
        width: 0,
        height: 0,
        scroll_height: 0,
        scroll_position: 0,
        hidden: false,
        text: text.map(String::from),
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

fn fixture_side_tab(
    index: i32,
    root: i32,
    available: bool,
    active: bool,
    visible: bool,
    widgets: Vec<WidgetView>,
) -> SideTabView {
    SideTabView {
        index,
        root_component_id: root,
        available,
        active,
        visible,
        widgets,
    }
}

fn fixture_chat(type_: i32, username: Option<&str>, text: &str, sequence: i32) -> ChatLineView {
    ChatLineView {
        type_,
        username: username.map(String::from),
        text: text.into(),
        sequence,
    }
}

fn open_scene() -> SceneView {
    SceneView {
        available: true,
        base_x: 3200,
        base_z: 3200,
        level: 0,
        width: 104,
        height: 104,
        collision_flags: vec![0; 104 * 104],
    }
}

#[test]
fn item_and_world_filters_compose() {
    let items = [fixture_item(1, 5), fixture_item(2, 1)]; // (id, count)
    let mut q = api::query::Query::new(&items);
    // ItemQueryExt::count_at_least + EntityQueryExt::with_id
    assert_eq!(q.count_at_least(2).first().map(|i| i.def.id), Some(1));
}

#[test]
fn entity_filters_match_names_ids_and_actions() {
    let items = [fixture_item(1, 5), fixture_item(2, 1), fixture_item(3, 2)];
    let mut q = Query::new(&items);
    q.with_id(&[1, 3]);
    assert_eq!(q.results().len(), 2);
    let mut q = Query::new(&items);
    q.with_name(&["ITEM 2"]);
    assert_eq!(q.first().map(|i| i.def.id), Some(2));
    let mut q = Query::new(&items);
    q.with_action(&["take"]);
    assert_eq!(q.count(), 3);
    let mut q = Query::new(&items);
    q.with_name_or_id(&[NameOrId::Name("item 1".into()), NameOrId::Id(3)]);
    assert_eq!(q.count(), 2);
}

#[test]
fn entity_filters_match_name_terms_and_wildcards() {
    let npcs = [
        fixture_npc(0, 1, "Goblin", 0, 0, 0, 5),
        fixture_npc(1, 2, "Goblin Guard", 0, 0, 0, 5),
        fixture_npc(2, 3, "Chicken", 0, 0, 0, 5),
    ];
    let mut q = Query::new(&npcs);
    q.name_contains(&["guard"]);
    assert_eq!(q.first().map(|n| n.index), Some(1));
    let mut q = Query::new(&npcs);
    q.matches_wildcard(&["Goblin*"]);
    assert_eq!(q.count(), 2);
    let mut q = Query::new(&npcs);
    q.matches_regex(|name| name.starts_with("Chic"));
    assert_eq!(q.first().map(|n| n.index), Some(2));
    // `with_index` never matches an entity without an index (items).
    let items = [fixture_item(1, 5)];
    let mut q = Query::new(&items);
    q.with_index(&[0]);
    assert_eq!(q.count(), 0);
}

#[test]
fn item_filters_by_slot_and_total_sum() {
    let mut items = [fixture_item(1, 5), fixture_item(2, 1), fixture_item(3, 2)];
    items[0].slot = 3;
    let mut q = Query::new(&items);
    q.with_slot(&[3]);
    assert_eq!(q.first().map(|i| i.def.id), Some(1));
    let mut q = Query::new(&items);
    q.with_id(&[1, 2, 3]).count_at_least(2);
    assert_eq!(q.total(), 7);
    let mut q = Query::new(&items);
    q.unstackable();
    assert_eq!(q.first().map(|i| i.def.id), Some(2));
}

#[test]
fn world_filters_use_distance_and_tiles() {
    let npcs = [
        fixture_npc(0, 1, "a", 10, 10, 0, 5),
        fixture_npc(1, 2, "b", 20, 20, 10, 5),
        fixture_npc(2, 3, "c", 12, 10, 2, 5),
    ];
    let mut q = Query::new(&npcs);
    q.within_distance(3);
    assert_eq!(q.count(), 2, "npc0 at d0 and npc2 at d2");
    let mut q = Query::new(&npcs);
    q.within_distance_to(
        WorldTile {
            x: 21,
            z: 20,
            level: 0,
        },
        2,
    );
    assert_eq!(q.first().map(|n| n.index), Some(1));
    let mut q = Query::new(&npcs);
    q.on_level(1);
    assert_eq!(q.count(), 0);
    let mut q = Query::new(&npcs);
    q.on_tile(WorldTile {
        x: 12,
        z: 10,
        level: 0,
    });
    assert_eq!(q.first().map(|n| n.index), Some(2));
    let mut q = Query::new(&npcs);
    q.inside(WorldArea {
        min_x: 10,
        max_x: 15,
        min_z: 10,
        max_z: 10,
        level: 0,
    });
    assert_eq!(q.count(), 2);
    // `nearest` is min distance, first on ties.
    assert_eq!(q.nearest().map(|n| n.index), Some(0));
    let q = Query::new(&npcs);
    assert_eq!(
        q.nearest_to(WorldTile {
            x: 21,
            z: 20,
            level: 0
        })
        .map(|n| n.index),
        Some(1)
    );
    // Level-mismatched tiles are infinitely far.
    let q = Query::new(&npcs);
    assert_eq!(
        q.nearest_to(WorldTile {
            x: 21,
            z: 20,
            level: 1
        })
        .map(|n| n.index),
        Some(0)
    );
}

#[test]
fn actor_filters_read_actor_state() {
    let mut npcs = [
        fixture_npc(0, 1, "a", 0, 0, 0, 5),
        fixture_npc(1, 2, "b", 0, 0, 0, 5),
        fixture_npc(2, 3, "c", 0, 0, 0, 5),
    ];
    npcs[0].animation = 12;
    npcs[0].pose_animation = 5;
    npcs[0].in_combat = true;
    npcs[0].target = Some(ActorTargetView {
        kind: ActorKind::Npc,
        index: 1,
    });
    npcs[1].moving = true;
    npcs[1].running = true;
    npcs[2].moving = true;
    npcs[2].running = false;
    npcs[2].target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: 7,
    });
    // dead = total > 0 and health 0; total 0 reads as alive-by-default.
    npcs[1].health = 0;

    let mut q = Query::new(&npcs);
    q.with_animation(&[12]);
    assert_eq!(q.count(), 1);
    let mut q = Query::new(&npcs);
    q.with_pose_animation(&[5]);
    assert_eq!(q.count(), 1);
    let mut q = Query::new(&npcs);
    q.in_combat().interacting();
    assert_eq!(q.first().map(|n| n.index), Some(0));
    let mut q = Query::new(&npcs);
    q.not_in_combat();
    assert_eq!(q.count(), 2);
    let mut q = Query::new(&npcs);
    q.targeting_npc(&[1]);
    assert_eq!(q.first().map(|n| n.index), Some(0));
    let mut q = Query::new(&npcs);
    q.targeting_player(&[7]);
    assert_eq!(q.first().map(|n| n.index), Some(2));
    let mut q = Query::new(&npcs);
    q.running();
    assert_eq!(q.first().map(|n| n.index), Some(1));
    let mut q = Query::new(&npcs);
    q.walking();
    assert_eq!(q.first().map(|n| n.index), Some(2));
    let mut q = Query::new(&npcs);
    q.not_interacting();
    assert_eq!(q.first().map(|n| n.index), Some(1));
    let mut q = Query::new(&npcs);
    q.moving();
    assert_eq!(q.count(), 2);
    let mut q = Query::new(&npcs);
    q.stationary();
    assert_eq!(q.first().map(|n| n.index), Some(0));

    // dead = total > 0 and health 0; total 0 reads as alive-by-default.
    let mut q = Query::new(&npcs);
    q.dead();
    assert_eq!(q.first().map(|n| n.index), Some(1));
    let mut q = Query::new(&npcs);
    q.alive();
    assert_eq!(q.count(), 2);
    let mut q = Query::new(&npcs);
    q.health_at_least(50);
    assert_eq!(q.count(), 2);
    let mut q = Query::new(&npcs);
    q.health_at_most(50);
    assert_eq!(q.count(), 1);
}

#[test]
fn npc_filters_read_level_size_and_local_target() {
    let mut npcs = [
        fixture_npc(0, 1, "a", 0, 0, 0, 5),
        fixture_npc(1, 2, "b", 0, 0, 0, 5),
    ];
    npcs[0].level = 2;
    npcs[0].size = 2;
    npcs[0].target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: 7,
    });
    npcs[1].level = 3;

    let mut q = Query::new(&npcs);
    q.with_level(&[3]);
    assert_eq!(q.first().map(|n| n.index), Some(1));
    let mut q = Query::new(&npcs);
    q.level_at_least(3);
    assert_eq!(q.count(), 1);
    let mut q = Query::new(&npcs);
    q.level_at_most(2);
    assert_eq!(q.first().map(|n| n.index), Some(0));
    let mut q = Query::new(&npcs);
    q.with_size(&[2]);
    assert_eq!(q.first().map(|n| n.index), Some(0));
    let mut q = Query::new(&npcs);
    q.interacting_with_local(7);
    assert_eq!(q.first().map(|n| n.index), Some(0));
}

#[test]
fn player_filters_read_combat_and_skill_levels() {
    let players = [
        fixture_player(0, 3, 5),
        fixture_player(1, 10, 5),
        fixture_player(2, 10, 7),
    ];
    let mut q = Query::new(&players);
    q.with_combat_level(&[10]);
    assert_eq!(q.count(), 2);
    let mut q = Query::new(&players);
    q.combat_level_at_most(3);
    assert_eq!(q.first().map(|p| p.index), Some(0));
    let mut q = Query::new(&players);
    q.combat_level_at_least(10).with_skill_level(&[7]);
    assert_eq!(q.first().map(|p| p.index), Some(2));
}

#[test]
fn ground_item_filters_read_item_def() {
    let items = [
        fixture_ground_item(1, "Bones", 1, false, false, 10, 10),
        fixture_ground_item(2, "Coins", 25, true, false, 12, 10),
        fixture_ground_item(3, "Rune", 1, true, true, 50, 50),
    ];
    let mut q = Query::new(&items);
    q.stackable().free_to_play();
    assert_eq!(q.first().map(|i| i.def.id), Some(2));
    let mut q = Query::new(&items);
    q.noted().unstackable();
    assert_eq!(q.count(), 0);
    let mut q = Query::new(&items);
    q.members();
    assert_eq!(q.first().map(|i| i.def.id), Some(3));
    let mut q = Query::new(&items);
    q.value_at_least(30).value_at_most(30);
    assert_eq!(q.first().map(|i| i.def.id), Some(3));
    let q = Query::new(&items);
    assert_eq!(
        q.nearest_to(WorldTile {
            x: 12,
            z: 11,
            level: 0
        })
        .map(|i| i.def.id),
        Some(2)
    );
}

#[test]
fn local_filters_read_loc_shape_and_state() {
    let locs = [
        fixture_loc(10, 0, 0, 0, 1, 1),
        fixture_loc(11, 1, 0, 0, 1, 1),
        fixture_loc(22, 0, 0, 0, 2, 1),
    ];
    let mut locs = locs;
    locs[1].layer = LocLayer::Wall;
    locs[1].animation = 12;
    locs[2].block_walk = false;
    locs[2].footprint_width = 2;
    locs[2].footprint_length = 1;

    let mut q = Query::new(&locs);
    q.with_shape(&[10]).blocking_walk();
    assert_eq!(q.count(), 1);
    let mut q = Query::new(&locs);
    q.with_layer(&[LocLayer::Wall]);
    assert_eq!(q.first().map(|l| l.shape), Some(11));
    let mut q = Query::new(&locs);
    q.with_footprint(2, 1);
    assert_eq!(q.first().map(|l| l.shape), Some(22));
    let mut q = Query::new(&locs);
    q.animated();
    assert_eq!(q.first().map(|l| l.shape), Some(11));
    let mut q = Query::new(&locs);
    q.static_().active();
    assert_eq!(q.count(), 2);
    let mut q = Query::new(&locs);
    q.with_angle(&[1]).with_animation(&[12]);
    assert_eq!(q.first().map(|l| l.shape), Some(11));
    let mut q = Query::new(&locs);
    q.not_blocking_walk().not_blocking_range();
    assert_eq!(q.count(), 0, "loc 2 blocks range");
    let mut q = Query::new(&locs);
    q.inactive();
    assert_eq!(q.count(), 0);
}

#[test]
fn stat_filters_read_skill_state() {
    let stats = [
        fixture_stat(0, "Attack", 7, 7, 100, true),
        fixture_stat(1, "Strength", 9, 8, 200, true),
        fixture_stat(2, "Defence", 7, 7, 100, false),
    ];
    let mut q = Query::new(&stats);
    q.boosted();
    assert_eq!(q.first().map(|s| s.index), Some(1));
    let mut q = Query::new(&stats);
    q.with_effective(&[7]).used();
    assert_eq!(q.count(), 1);
    let mut q = Query::new(&stats);
    q.with_base(&[8]).base_at_least(8);
    assert_eq!(q.first().map(|s| s.index), Some(1));
    let mut q = Query::new(&stats);
    q.experience_at_least(150).experience_at_most(200);
    assert_eq!(q.first().map(|s| s.index), Some(1));
    let mut q = Query::new(&stats);
    q.with_name(&["attack"]).with_index(&[0]);
    assert_eq!(q.count(), 1);
    let mut q = Query::new(&stats);
    q.drained();
    assert_eq!(q.count(), 0);
    let mut q = Query::new(&stats);
    q.unchanged();
    assert_eq!(q.count(), 2);
}

#[test]
fn varp_filters_read_values() {
    let varps = [fixture_varp(0, 0), fixture_varp(1, 5), fixture_varp(2, 5)];
    let mut q = Query::new(&varps);
    q.with_index(&[1, 2]).value_at_least(5);
    assert_eq!(q.count(), 2);
    let mut q = Query::new(&varps);
    q.zero();
    assert_eq!(q.first().map(|v| v.index), Some(0));
    let mut q = Query::new(&varps);
    q.non_zero().with_value(&[5]);
    assert_eq!(q.count(), 2);
    let mut q = Query::new(&varps);
    q.value_at_most(4);
    assert_eq!(q.first().map(|v| v.index), Some(0));
}

#[test]
fn widget_filters_read_component_state() {
    let mut widgets = [
        fixture_widget(1001, 3, None, 0),
        fixture_widget(1002, 1, Some("Attack"), 10),
        fixture_widget(1003, 5, Some("Defence"), 20),
    ];
    widgets[0].hidden = true;
    widgets[2].items = vec![fixture_item(7, 1)];
    widgets[2].actions = vec![Some("Select".into())];
    widgets[2].varp_bindings = vec![api::snapshot::WidgetVarpBindingView {
        script_index: 0,
        varp: 43,
        value: Some(2),
        comparator: Some(0),
    }];

    let mut q = Query::new(&widgets);
    q.with_component_id(&[1001, 1003]).with_button_type(&[3]);
    assert_eq!(q.first().map(|w| w.component_id), Some(1001));
    let mut q = Query::new(&widgets);
    q.with_text(&["attack"]);
    assert_eq!(q.first().map(|w| w.component_id), Some(1002));
    let mut q = Query::new(&widgets);
    q.text_contains(&["fence"]);
    assert_eq!(q.first().map(|w| w.component_id), Some(1003));
    let mut q = Query::new(&widgets);
    q.text_matches(|t| t.contains("efence"));
    assert_eq!(q.first().map(|w| w.component_id), Some(1003));
    let mut q = Query::new(&widgets);
    q.hidden();
    assert_eq!(q.first().map(|w| w.component_id), Some(1001));
    let mut q = Query::new(&widgets);
    q.not_hidden().with_action(&["select"]);
    assert_eq!(q.first().map(|w| w.component_id), Some(1003));
    let mut q = Query::new(&widgets);
    q.bound_to_varp(43, Some(2));
    assert_eq!(q.first().map(|w| w.component_id), Some(1003));
    let mut q = Query::new(&widgets);
    q.bound_to_varp(43, None);
    assert_eq!(q.count(), 1);
    let mut q = Query::new(&widgets);
    q.with_item_id(&[7]).with_any_item();
    assert_eq!(q.first().map(|w| w.component_id), Some(1003));
    let mut q = Query::new(&widgets);
    q.with_layer_id(&[1000]);
    assert_eq!(q.count(), 3);
}

#[test]
fn widget_items_subquery_filters_component_slots() {
    let mut w = fixture_widget(500, 1, None, 0);
    w.items = vec![fixture_item(1, 5), fixture_item(2, 1)];
    let widgets = [w];

    let mut q = Query::new(&widgets);
    q.with_item_id(&[2]);
    let items = q.items();
    assert_eq!(items.count(), 2, "both slots of the matching widget");
    assert_eq!(items.total(), 6);
    let q = Query::new(&widgets);
    let mut items = q.items();
    items.with_id(&[2]);
    assert_eq!(items.first().map(|i| i.def.id), Some(2));
}

#[test]
fn side_tab_filters_read_tab_state() {
    let tabs = [
        fixture_side_tab(
            3,
            500,
            true,
            false,
            false,
            vec![fixture_widget(501, 1, None, 0)],
        ),
        fixture_side_tab(4, 600, true, true, true, vec![]),
    ];
    let mut q = Query::new(&tabs);
    q.with_index(&[4]).active().visible();
    assert_eq!(q.first().map(|t| t.root_component_id), Some(600));
    let mut q = Query::new(&tabs);
    q.unavailable();
    assert_eq!(q.count(), 0);
    let mut q = Query::new(&tabs);
    let widgets = q.available().widgets();
    assert_eq!(widgets.count(), 1);
    let mut q = Query::new(&tabs);
    q.not_visible();
    assert_eq!(q.first().map(|t| t.index), Some(3));
}

#[test]
fn chat_filters_read_lines_and_sequences() {
    let lines = [
        fixture_chat(0, None, "welcome", 0),
        fixture_chat(1, Some("alice"), "hello", 1),
        fixture_chat(1, Some("bob"), "hi there", 2),
        fixture_chat(2, None, "spam", 3),
    ];
    let mut q = Query::new(&lines);
    q.with_type_(&[1]);
    assert_eq!(q.count(), 2);
    let mut q = Query::new(&lines);
    q.sent_by(&["ALICE"]);
    assert_eq!(q.first().map(|l| l.sequence), Some(1));
    let mut q = Query::new(&lines);
    q.with_sender();
    assert_eq!(q.count(), 2);
    let mut q = Query::new(&lines);
    q.without_sender();
    assert_eq!(q.count(), 2);
    let mut q = Query::new(&lines);
    q.text_contains(&["there"]);
    assert_eq!(q.first().map(|l| l.sequence), Some(2));
    let mut q = Query::new(&lines);
    q.since(1);
    assert_eq!(q.count(), 2);
    assert_eq!(q.latest_sequence(), 3);
    let q = Query::new(&lines);
    assert_eq!(q.latest_sequence(), 3);
    let empty: [ChatLineView; 0] = [];
    let q = Query::new(&empty);
    assert_eq!(q.latest_sequence(), 0);
}

#[test]
fn scene_query_collision_and_reach() {
    let mut scene = open_scene();
    scene.collision_flags[5 * 104 + 6] = CollisionFlag::SQ_BLOCKED;
    scene.collision_flags[10 * 104 + 10] = CollisionFlag::W_W;

    let sq = SceneQuery::new(
        &scene,
        Some(WorldTile {
            x: 3205,
            z: 3205,
            level: 0,
        }),
    );
    assert!(sq.contains(WorldTile {
        x: 3205,
        z: 3205,
        level: 0
    }));
    assert!(!sq.contains(WorldTile {
        x: 3304,
        z: 3200,
        level: 0
    }));
    assert_eq!(
        sq.base(),
        WorldTile {
            x: 3200,
            z: 3200,
            level: 0
        }
    );
    assert_eq!(
        sq.to_local(WorldTile {
            x: 3205,
            z: 3206,
            level: 0
        }),
        Some(LocalTile { lx: 5, lz: 6 })
    );
    assert_eq!(
        sq.to_local(WorldTile {
            x: 3304,
            z: 3200,
            level: 0
        }),
        None
    );
    assert_eq!(
        sq.to_world(LocalTile { lx: 5, lz: 6 }),
        Some(WorldTile {
            x: 3205,
            z: 3206,
            level: 0
        })
    );
    assert_eq!(sq.to_world(LocalTile { lx: 104, lz: 0 }), None);

    assert_eq!(
        sq.collision_at(WorldTile {
            x: 3205,
            z: 3206,
            level: 0
        }),
        Some(CollisionFlag::SQ_BLOCKED)
    );
    assert_eq!(
        sq.collision_at(WorldTile {
            x: 3210,
            z: 3210,
            level: 0
        }),
        Some(CollisionFlag::W_W)
    );
    assert_eq!(
        sq.collision_at(WorldTile {
            x: 3304,
            z: 3200,
            level: 0
        }),
        None
    );
    assert!(sq.probeable(WorldTile {
        x: 3210,
        z: 3210,
        level: 0
    }));
    assert!(!sq.probeable(WorldTile {
        x: 3304,
        z: 3200,
        level: 0
    }));
    assert!(!sq.walkable(WorldTile {
        x: 3205,
        z: 3206,
        level: 0
    }));
    assert!(sq.walkable(WorldTile {
        x: 3206,
        z: 3206,
        level: 0
    }));

    // Orthogonal steps across open edges; the blocked tile and the wall
    // tile refuse the step.
    assert!(sq.can_step(
        WorldTile {
            x: 3205,
            z: 3207,
            level: 0
        },
        WorldTile {
            x: 3206,
            z: 3207,
            level: 0
        }
    ));
    assert!(!sq.can_step(
        WorldTile {
            x: 3205,
            z: 3205,
            level: 0
        },
        WorldTile {
            x: 3205,
            z: 3206,
            level: 0
        }
    ));
    assert!(!sq.can_step(
        WorldTile {
            x: 3209,
            z: 3210,
            level: 0
        },
        WorldTile {
            x: 3210,
            z: 3210,
            level: 0
        }
    ));
    assert!(!sq.can_step(
        WorldTile {
            x: 3205,
            z: 3205,
            level: 0
        },
        WorldTile {
            x: 3205,
            z: 3205,
            level: 0
        }
    ));
    assert!(!sq.can_step(
        WorldTile {
            x: 3205,
            z: 3205,
            level: 0
        },
        WorldTile {
            x: 3207,
            z: 3205,
            level: 0
        }
    ));
    assert!(!sq.can_step(
        WorldTile {
            x: 3205,
            z: 3205,
            level: 0
        },
        WorldTile {
            x: 3206,
            z: 3206,
            level: 1
        }
    ));

    // BFS reach: the blocked tile is not reachable, but a walk around
    // it is; `adjacent_ok` allows interacting across a blocked tile.
    assert!(!sq.can_reach(
        WorldTile {
            x: 3205,
            z: 3206,
            level: 0
        },
        &SceneReachOptions::default()
    ));
    assert!(sq.can_reach(
        WorldTile {
            x: 3206,
            z: 3206,
            level: 0
        },
        &SceneReachOptions::default()
    ));
    assert!(sq.can_reach(
        WorldTile {
            x: 3205,
            z: 3206,
            level: 0
        },
        &SceneReachOptions {
            max_steps: None,
            adjacent_ok: true,
        }
    ));
    let no_player = SceneQuery::new(&scene, None);
    assert!(!no_player.can_reach(
        WorldTile {
            x: 3206,
            z: 3206,
            level: 0
        },
        &SceneReachOptions::default()
    ));
}

#[test]
fn loc_approach_operability() {
    let scene = open_scene();
    let tree = fixture_loc(10, 0, 5, 5, 1, 1);
    // From the west tile: the edge is open and no force-approach applies.
    assert_eq!(
        loc_approach::can_operate_from(
            &tree,
            &scene,
            WorldTile {
                x: 3204,
                z: 3205,
                level: 0
            }
        ),
        Some(true)
    );
    // On the loc's own tile: always operable.
    assert_eq!(
        loc_approach::can_operate_from(
            &tree,
            &scene,
            WorldTile {
                x: 3205,
                z: 3205,
                level: 0
            }
        ),
        Some(true)
    );
    // Non-footprint shapes (and level mismatches) read as "don't know".
    let rock = fixture_loc(0, 0, 5, 5, 1, 1);
    assert_eq!(
        loc_approach::can_operate_from(
            &rock,
            &scene,
            WorldTile {
                x: 3204,
                z: 3205,
                level: 0
            }
        ),
        None
    );
    assert_eq!(
        loc_approach::can_operate_from(
            &tree,
            &scene,
            WorldTile {
                x: 3204,
                z: 3205,
                level: 1
            }
        ),
        None
    );
    // Force-approach west refuses the west side, allows the east.
    let mut forced = tree.clone();
    forced.force_approach = 0x8;
    assert_eq!(
        loc_approach::can_operate_from(
            &forced,
            &scene,
            WorldTile {
                x: 3204,
                z: 3205,
                level: 0
            }
        ),
        Some(false)
    );
    assert_eq!(
        loc_approach::can_operate_from(
            &forced,
            &scene,
            WorldTile {
                x: 3206,
                z: 3205,
                level: 0
            }
        ),
        Some(true)
    );
    // A blocked adjacent tile drops out of the operable set.
    let tiles = loc_approach::operable_tiles(&tree, &scene).unwrap();
    assert!(tiles.contains(&WorldTile {
        x: 3204,
        z: 3205,
        level: 0
    }));
    assert!(tiles.contains(&WorldTile {
        x: 3206,
        z: 3205,
        level: 0
    }));
    let mut scene = open_scene();
    scene.collision_flags[4 * 104 + 5] = CollisionFlag::SQ_BLOCKED;
    let tiles = loc_approach::operable_tiles(&tree, &scene).unwrap();
    assert!(!tiles.contains(&WorldTile {
        x: 3204,
        z: 3205,
        level: 0
    }));
    // The SceneQuery delegates both reads.
    let sq = SceneQuery::new(
        &scene,
        Some(WorldTile {
            x: 3205,
            z: 3205,
            level: 0,
        }),
    );
    assert_eq!(
        sq.can_operate_from(
            &tree,
            WorldTile {
                x: 3206,
                z: 3205,
                level: 0
            }
        ),
        Some(true)
    );
    assert!(sq.operable_tiles(&tree).unwrap().len() >= 4);
}

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

#[test]
fn widget_search_finds_buttons_and_styles() {
    let mut c = Client::new(cfg());
    set_iface(
        &mut c,
        1000,
        IfType {
            id: 1000,
            layer_id: 1000,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![1001, 1002, 1003, 1004, 1005, 1006, 1007, 1008]),
            child_y: Some(vec![0, 10, 20, 30, 50, 70, 60, 70]),
            ..Default::default()
        },
    );
    // Close button (type 3).
    set_iface(
        &mut c,
        1001,
        IfType {
            id: 1001,
            layer_id: 1000,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        1001,
        IfTypeMut {
            button_type: ButtonType::BUTTON_CLOSE,
            ..Default::default()
        },
    );

    // Plain OK button labeled "Attack".
    set_iface(
        &mut c,
        1002,
        IfType {
            id: 1002,
            layer_id: 1000,
            button_text: "Attack".into(),
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        1002,
        IfTypeMut {
            button_type: ButtonType::BUTTON_OK,
            ..Default::default()
        },
    );

    // Target button with base "Chop down".
    set_iface(
        &mut c,
        1003,
        IfType {
            id: 1003,
            layer_id: 1000,
            target_base: "Chop down".into(),
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        1003,
        IfTypeMut {
            button_type: ButtonType::BUTTON_TARGET,
            ..Default::default()
        },
    );

    // Select button bound to varp 43, value 2.
    set_iface(
        &mut c,
        1004,
        IfType {
            id: 1004,
            layer_id: 1000,
            scripts: Some(vec![vec![5, 43]]),
            script_operand: Some(vec![2]),
            script_comparator: Some(vec![0]),
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        1004,
        IfTypeMut {
            button_type: ButtonType::BUTTON_SELECT,
            ..Default::default()
        },
    );

    // Combat style buttons (varp 43, values 0 and 1) with text labels.
    set_iface(
        &mut c,
        1005,
        IfType {
            id: 1005,
            layer_id: 1000,
            scripts: Some(vec![vec![5, 43]]),
            script_operand: Some(vec![0]),
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        1005,
        IfTypeMut {
            button_type: ButtonType::BUTTON_SELECT,
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        1006,
        IfType {
            id: 1006,
            layer_id: 1000,
            scripts: Some(vec![vec![5, 43]]),
            script_operand: Some(vec![1]),
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        1006,
        IfTypeMut {
            button_type: ButtonType::BUTTON_SELECT,
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        1007,
        IfType {
            id: 1007,
            layer_id: 1000,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        1007,
        IfTypeMut {
            text: "Punch".into(),
            ..Default::default()
        },
    );

    set_iface(
        &mut c,
        1008,
        IfType {
            id: 1008,
            layer_id: 1000,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    set_iface_mut(
        &mut c,
        1008,
        IfTypeMut {
            text: "Kick".into(),
            ..Default::default()
        },
    );

    c.main_modal_id = 1000;

    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
    assert!(snap.rebuild_family(&c, Family::Widgets));
    assert_eq!(snap.widgets().len(), 9, "the root plus its eight children");

    assert_eq!(widget_search::close_button_com_id(&snap, 1000), 1001);
    assert_eq!(widget_search::button_by_text(&snap, 1000, "attack"), 1002);
    assert_eq!(
        widget_search::button_by_text(&snap, 1000, "  Attack "),
        1002
    );
    assert_eq!(widget_search::button_by_text(&snap, 1000, "defence"), -1);
    assert_eq!(
        widget_search::target_button_by_base(&snap, 1000, "chop down"),
        1003
    );
    assert_eq!(
        widget_search::target_button_by_base(&snap, 1000, "nothing"),
        -1
    );
    assert_eq!(
        widget_search::select_button_by_varp(&snap, 1000, 43, 2),
        1004
    );
    assert_eq!(
        widget_search::select_button_by_varp(&snap, 1000, 43, 0),
        1005
    );
    assert_eq!(widget_search::select_button_by_varp(&snap, 1000, 7, 2), -1);

    // The three varp-43 buttons (1004/1005/1006) pair with the nearest
    // text by y and sort by mode.
    let labels = widget_search::combat_style_labels(&snap, 1000, 43);
    assert_eq!(labels.len(), 3);
    assert_eq!(labels[0].mode, 0);
    assert_eq!(labels[0].label, "Punch");
    assert_eq!(labels[0].component_id, 1005);
    assert_eq!(labels[1].mode, 1);
    assert_eq!(labels[1].label, "Kick");
    assert_eq!(labels[2].mode, 2);
    assert_eq!(labels[2].component_id, 1004);
}

/// 274 `combat_unarmed.if`: each SELECT sits on the same row as Punch/Kick/Block,
/// with `(Accurate)` / `(Aggressive)` / `(Defensive)` a few pixels below. Nearest
/// text by y is the action name; `Game.setCombatStyle('strength')` matches the
/// style name. Posted labels must be the style names already on the IF.
#[test]
fn combat_style_labels_prefers_parenthetical_style_name() {
    let mut c = Client::new(cfg());
    // Overlay root + style layer (unarmed101) + three SELECT boxes + six texts.
    set_iface(
        &mut c,
        2000,
        IfType {
            id: 2000,
            layer_id: 2000,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![2001]),
            child_x: Some(vec![4]),
            child_y: Some(vec![60]),
            ..Default::default()
        },
    );
    set_iface(
        &mut c,
        2001,
        IfType {
            id: 2001,
            layer_id: 2000,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![2010, 2011, 2012, 2020, 2021, 2022, 2023, 2024, 2025]),
            child_x: Some(vec![5, 5, 5, 78, 78, 78, 78, 78, 78]),
            child_y: Some(vec![5, 51, 97, 5, 51, 97, 18, 64, 110]),
            ..Default::default()
        },
    );
    for (id, mode) in [(2010, 0), (2011, 1), (2012, 2)] {
        set_iface(
            &mut c,
            id,
            IfType {
                id: id as i32,
                layer_id: 2001,
                r#type: ComponentType::TYPE_GRAPHIC,
                width: 72,
                height: 36,
                scripts: Some(vec![vec![5, 43, 0]]),
                script_operand: Some(vec![mode]),
                script_comparator: Some(vec![0]),
                ..Default::default()
            },
        );
        set_iface_mut(
            &mut c,
            id,
            IfTypeMut {
                button_type: ButtonType::BUTTON_SELECT,
                ..Default::default()
            },
        );
    }
    for (id, text) in [
        (2020, "Punch"),
        (2021, "Kick"),
        (2022, "Block"),
        (2023, "(Accurate)"),
        (2024, "(Aggressive)"),
        (2025, "(Defensive)"),
    ] {
        set_iface(
            &mut c,
            id,
            IfType {
                id: id as i32,
                layer_id: 2001,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        set_iface_mut(
            &mut c,
            id,
            IfTypeMut {
                text: text.into(),
                ..Default::default()
            },
        );
    }

    c.side_icon[0] = 2000;
    c.bump_gens(ServerProt::IF_SETICON);
    let mut snap = GameSnapshot::new();
    assert!(snap.rebuild_family(&c, Family::SideTabs));
    let labels = widget_search::combat_style_labels(&snap, 2000, 43);
    assert_eq!(labels.len(), 3, "three varp-43 SELECT boxes");
    assert_eq!(labels[0].mode, 0);
    assert!(
        labels[0].label.to_ascii_lowercase().contains("accurate"),
        "mode 0 must post the style name, not Punch: {:?}",
        labels[0].label
    );
    assert_eq!(labels[1].mode, 1);
    assert!(
        labels[1].label.to_ascii_lowercase().contains("aggressive"),
        "mode 1 must post the style name, not Kick: {:?}",
        labels[1].label
    );
    assert_eq!(labels[2].mode, 2);
    assert!(
        labels[2].label.to_ascii_lowercase().contains("defensive"),
        "mode 2 must post the style name, not Block: {:?}",
        labels[2].label
    );
}

/// Packed `combat_unarmed` from the local client jag: SELECT + `pushvar com_mode`
/// must survive unpack so live `combat_styles` is not an empty keyframe.
#[test]
fn packed_combat_unarmed_posts_aggressive_from_tab_0() {
    let cache = client::cache_dir();
    if !cache.join("interface").is_file() {
        return;
    }
    let mut c = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: cache.display().to_string(),
        members: true,
        lowmem: false,
    });
    assert!(
        c.ifaces_len() > 0,
        "cache interface jag unpacked no components"
    );

    let mut aggressive_id = None;
    for id in 0..c.ifaces_len() {
        let Some(com) = c.if_(id) else {
            continue;
        };
        if com.text.to_ascii_lowercase().contains("aggressive") {
            aggressive_id = Some(id);
            break;
        }
    }
    let aggressive_id = aggressive_id.expect("packed IF must contain an Aggressive label");
    let style_layer = c.if_(aggressive_id).expect("aggressive component").layer_id;
    let overlay = c
        .if_(style_layer as usize)
        .map(|com| com.layer_id)
        .filter(|&id| id >= 0)
        .unwrap_or(style_layer);

    c.side_icon[0] = overlay;
    c.bump_gens(ServerProt::IF_SETICON);
    let mut snap = GameSnapshot::new();
    assert!(snap.rebuild_family(&c, Family::SideTabs));
    let tab = snap
        .side_tabs()
        .iter()
        .find(|t| t.index == 0)
        .expect("tab 0 row");
    assert!(
        !tab.widgets.is_empty(),
        "tab 0 walk from overlay {overlay} produced no widgets (aggressive id {aggressive_id}, style layer {style_layer})"
    );

    let varps: Vec<i32> = tab
        .widgets
        .iter()
        .filter(|w| w.button_type == ButtonType::BUTTON_SELECT)
        .flat_map(|w| w.varp_bindings.iter().map(|b| b.varp))
        .collect();
    assert!(
        !varps.is_empty(),
        "packed combat SELECT buttons have no opcode-5 varp bindings; widgets={} selects={}",
        tab.widgets.len(),
        tab.widgets
            .iter()
            .filter(|w| w.button_type == ButtonType::BUTTON_SELECT)
            .count()
    );
    let varp = varps[0];
    let labels = widget_search::combat_style_labels(&snap, overlay, varp);
    assert!(
        labels
            .iter()
            .any(|l| l.label.to_ascii_lowercase().contains("aggressive")),
        "packed combat IF varp={varp} labels={labels:?}"
    );
}
