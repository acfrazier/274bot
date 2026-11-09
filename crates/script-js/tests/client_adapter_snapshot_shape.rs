//! The native posted-snapshot builders of the ClientAdapter facts (widget rows with their item
//! rows, the facts object, the projection scene and its loc rows) must be born with fast
//! properties, and the posted objects must stay inside a stated used-heap bound after collection.
//!
//! This is a `harness = false` executable so `--allow-natives-syntax` can be set before any V8
//! platform exists. It posts through the public snapshot encoder and reads the objects back
//! without running a JavaScript reader, i.e. without warming any transition.

use script::native::snapshot_source::{
    ChatModalTextsInput, ClientAdapterFactsInput, ItemRowInput, NativeFactsInput, TileInput,
    WidgetFactInput,
};
use script_js::wire::encode_snapshot_with_native;
use script_js::{LoadIsolate, LoadShape};

mod common;

/// Posted widget rows, each with `ITEMS_PER_WIDGET` item rows.
const WIDGETS: usize = 64;
const ITEMS_PER_WIDGET: usize = 4;
/// Projection loc rows posted with the scene.
const SCENE_LOCS: usize = 256;
/// Retained V8 objects of the posted facts: the facts object and its `map_build_base`,
/// `server_tile` and `chat_modal_texts` objects, every widget row and its item rows, the
/// projection scene and its loc rows. The heap bound charges each 1 KiB, the same allowance
/// `cold_native_entity_rows_bound_collected_v8_heap_growth` uses, plus the scene's typed-array
/// views (one 105 x 105 height grid and one 104 x 104 flag grid).
const POSTED_OBJECTS: usize = 4 + WIDGETS * (1 + ITEMS_PER_WIDGET) + 1 + SCENE_LOCS;
const SCENE_ARRAY_BYTES: usize = 105 * 105 * 4 + 104 * 104;
const HEAP_LIMIT: usize = POSTED_OBJECTS * 1024 + SCENE_ARRAY_BYTES;

const BOT_SOURCE: &str = r#"
export default class SnapshotShapeProbe extends LoopingBot {
    loop() {}
}
"#;

const READ_FACTS: &str = r#"(() => {
    const fast = new Function('o', 'return %HasFastProperties(o)');
    const facts = globalThis.__rs2b0t_host.snapshot.client_adapter;
    const slow = [];
    const check = (label, object) => {
        if (object === null || typeof object !== 'object') {
            slow.push(label + ' is not an object');
        } else if (!fast(object)) {
            slow.push(label);
        }
    };
    check('client_adapter', facts);
    check('map_build_base', facts.map_build_base);
    check('server_tile', facts.server_tile);
    check('chat_modal_texts', facts.chat_modal_texts);
    facts.widgets.forEach((widget, i) => {
        check('widgets[' + i + ']', widget);
        widget.items.forEach((item, j) => check('widgets[' + i + '].items[' + j + ']', item));
    });
    const scene = facts.projection_scene;
    check('projection_scene', scene);
    scene.locs.forEach((loc, i) => check('projection_scene.locs[' + i + ']', loc));
    return {
        slow,
        widgets: facts.widgets.length,
        items: facts.widgets.reduce((sum, widget) => sum + widget.items.length, 0),
        locs: scene.locs.length,
        keys: Object.keys(facts),
    };
})()"#;

fn main() {
    // Must run before V8 exists in this process: the isolated executable keeps this flag off
    // every other test binary.
    v8::V8::set_flags_from_string("--allow-natives-syntax");
    script_js::bootstrap_v8_platform().expect("process-main V8 bootstrap");
    let iso = LoadIsolate::spawn(BOT_SOURCE.into(), LoadShape::CompatClass, vec![])
        .expect("spawn snapshot-shape isolate");
    iso.probe("true").expect("isolate starts");
    let baseline = iso.collected_heap_bytes_for_test().expect("baseline heap");

    let names: Vec<String> = (0..WIDGETS).map(|i| format!("Widget {i}")).collect();
    let items: Vec<Vec<ItemRowInput<'_>>> = (0..WIDGETS)
        .map(|i| {
            (0..ITEMS_PER_WIDGET)
                .map(|j| {
                    let mut row = ItemRowInput::nc(Some(names[i].as_str()), 1 + j as i32);
                    row.id = 1000 + (i * ITEMS_PER_WIDGET + j) as i32;
                    row.slot = j as i32;
                    row
                })
                .collect()
        })
        .collect();
    let widgets: Vec<WidgetFactInput<'_>> = (0..WIDGETS)
        .map(|i| WidgetFactInput {
            component_id: 5000 + i as i32,
            root_component_id: 5000,
            parent_id: 5000,
            type_: 2,
            button_type: 1,
            client_code: 0,
            x: i as i32,
            y: 2 * i as i32,
            width: 32,
            height: 32,
            hidden: false,
            text: Some(names[i].as_str()),
            alternate_text: None,
            button_text: Some("Ok"),
            target_base: None,
            model_type: 0,
            model_id: -1,
            script_opcode: 0,
            script_varp: -1,
            script_value: 0,
            colour: 0,
            inventory_ready: i % 2 == 0,
            items: &items[i],
        })
        .collect();
    let locs: Vec<api::snapshot::ProjectionLocView> = (0..SCENE_LOCS)
        .map(|i| api::snapshot::ProjectionLocView {
            id: 2000 + i as i32,
            name: Some(std::sync::Arc::from(format!("Loc {i}").as_str())),
            level: 0,
            rank: 1,
            scene_x: (i as i32 % 100) * 128 + 64,
            scene_z: (i as i32 / 100) * 128 + 64,
            min_tile_x: i as i32 % 100,
            max_tile_x: i as i32 % 100,
            min_tile_z: i as i32 / 100,
            max_tile_z: i as i32 / 100,
            half_width: 64,
            half_length: 64,
            top_height: 128,
        })
        .collect();
    let projection = api::snapshot::ProjectionSceneView {
        available: true,
        level: 0,
        camera_x: 0,
        camera_y: 0,
        camera_z: 0,
        sin_pitch: 0,
        cos_pitch: 65_536,
        sin_yaw: 0,
        cos_yaw: 65_536,
        heights: vec![0i32; 105 * 105].into(),
        heights_above: Vec::<i32>::new().into(),
        link_below: vec![0u8; 104 * 104].into(),
        locs: locs.into(),
    };
    let chat_texts = ["Continue".to_string()];
    let facts = ClientAdapterFactsInput {
        attached: true,
        map_build_base: (3200, 3200),
        server_tile: Some(TileInput {
            x: 3201,
            z: 3202,
            level: 0,
        }),
        hint_tile_level: Some(0),
        bank_com_id: 5382,
        chat_continue_component_id: 3012,
        chat_modal_texts: ChatModalTextsInput {
            root: 200,
            texts: &chat_texts,
        },
        widget_facts: &widgets,
        player_count: 0,
        npc_count: 0,
        run_controls: Some((91, 92)),
        login_message: "",
        menu_entries: &[],
        last_walk_path_local: &[],
        session_generation: 1,
        modal_close_generation: 0,
        projection_scene: Some(script::native::snapshot_source::ProjectionSceneInput {
            view: &projection,
        }),
    };
    let input = common::ingame_snapshot();
    let native = NativeFactsInput {
        client_adapter: Some(facts),
        ..Default::default()
    };
    assert!(
        iso.post_snapshot(encode_snapshot_with_native(&input, native)),
        "the ClientAdapter snapshot was refused"
    );

    let read = iso.probe(READ_FACTS).expect("read the posted facts");
    assert_eq!(read["widgets"], WIDGETS, "{read}");
    assert_eq!(read["items"], WIDGETS * ITEMS_PER_WIDGET, "{read}");
    assert_eq!(read["locs"], SCENE_LOCS, "{read}");
    assert_eq!(
        read["slow"],
        serde_json::json!([]),
        "every native-posted object must have fast properties"
    );

    let used = iso.collected_heap_bytes_for_test().expect("collected heap");
    let growth = used.saturating_sub(baseline);
    println!(
        "client adapter snapshot: {POSTED_OBJECTS} objects, {WIDGETS} widgets x {ITEMS_PER_WIDGET} items, {SCENE_LOCS} locs: baseline={baseline} used={used} growth={growth} limit={HEAP_LIMIT}"
    );
    assert!(
        growth <= HEAP_LIMIT,
        "the posted ClientAdapter snapshot retains {growth} B after collection, over {HEAP_LIMIT} B"
    );
    let _ = iso.join();
}
