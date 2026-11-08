use super::*;

#[test]
fn explicit_289_inputs_select_game_data_and_config() {
    let inputs = parse_args([
        "--revision",
        "289",
        "--content",
        "/fixture/289/content",
        "--cache",
        "/fixture/289/engine/data/pack/client",
        "--game-data-dir",
        "/fixture/game-data",
        "--out",
        "/tmp/289.navpack",
    ])
    .unwrap();
    assert_eq!(inputs.revision, 289);
    assert_eq!(inputs.maps_dir, PathBuf::from("/fixture/289/content/maps"));
    assert_eq!(
        inputs.doors_dir,
        PathBuf::from("/fixture/289/content/scripts/doors/configs")
    );
    assert_eq!(
        inputs.config_jag,
        PathBuf::from("/fixture/289/engine/data/pack/client/config")
    );
    assert_eq!(inputs.game_data_dir, PathBuf::from("/fixture/game-data"));
    assert_eq!(inputs.out, PathBuf::from("/tmp/289.navpack"));
    assert_eq!(inputs.flags_out, PathBuf::from("/tmp/289.navflags"));
    assert_eq!(inputs.reach_out, PathBuf::from("/tmp/289.navreach"));
    assert_eq!(inputs.canlight_out, PathBuf::from("/tmp/289.navcanlight"));
    assert_eq!(inputs.pois_out, PathBuf::from("/tmp/289.navpois"));
}

#[test]
fn explicit_274_uses_server_config_next_to_client_cache() {
    let inputs = parse_args([
        "--revision",
        "274",
        "--content",
        "/fixture/274/content",
        "--cache",
        "/fixture/274/engine/data/pack/client",
        "--game-data-dir",
        "/fixture/game-data",
        "--out",
        "/tmp/274.navpack",
    ])
    .unwrap();
    assert_eq!(
        inputs.config_jag,
        PathBuf::from("/fixture/274/engine/data/pack/config")
    );
}

#[test]
fn explicit_mode_rejects_ambiguous_or_incomplete_inputs() {
    let complete = [
        "--revision",
        "289",
        "--content",
        "/content",
        "--cache",
        "/cache",
        "--game-data-dir",
        "/game-data",
        "--out",
        "/tmp/x",
    ];
    let mut positional = complete.to_vec();
    positional.push("positional");
    for args in [
        vec!["--revision", "289"],
        vec![
            "--revision",
            "377",
            "--content",
            "/content",
            "--cache",
            "/cache",
            "--game-data-dir",
            "/game-data",
            "--out",
            "/tmp/x",
        ],
        positional,
        vec![
            "--revision",
            "274",
            "--revision",
            "289",
            "--content",
            "/content",
            "--cache",
            "/cache",
            "--game-data-dir",
            "/game-data",
            "--out",
            "/tmp/x",
        ],
    ] {
        assert!(parse_args(args).is_err());
    }
}

#[test]
fn flags_path_for_swaps_pack_extension() {
    assert_eq!(
        flags_path_for(&PathBuf::from("/tmp/x/274bot.navpack")),
        PathBuf::from("/tmp/x/274bot.navflags")
    );
}

#[test]
fn cli_bake_rejects_a_missing_door_config_before_baking() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "274bot-nav-pack-strict-{}-{unique}",
        std::process::id()
    ));
    let maps = root.join("content/maps");
    let doors = root.join("content/scripts/doors/configs");
    std::fs::create_dir_all(&maps).unwrap();
    std::fs::create_dir_all(&doors).unwrap();
    std::fs::write(doors.join("doors.loc"), "[loc_100]\nop1=Open\n").unwrap();

    let inputs = parse_args(vec![
        "--revision".to_owned(),
        "274".to_owned(),
        "--content".to_owned(),
        root.join("content").to_string_lossy().into_owned(),
        "--cache".to_owned(),
        root.join("engine/data/pack/client")
            .to_string_lossy()
            .into_owned(),
        "--game-data-dir".to_owned(),
        root.join("game-data").to_string_lossy().into_owned(),
        "--out".to_owned(),
        root.join("274.navpack").to_string_lossy().into_owned(),
    ])
    .unwrap();
    let gates = root.join("content/scripts/general_use/configs/gates.loc");
    let error = bake_world(&inputs, &gates)
        .err()
        .expect("missing door config must fail");
    assert!(error.to_string().contains("doubledoors.loc"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}

fn crossing_tile(x: i32, z: i32, level: i32) -> api::snapshot::WorldTile {
    api::snapshot::WorldTile { x, z, level }
}

fn crossing_collision(
    blocked_tiles: &[api::snapshot::WorldTile],
) -> nav_planner::collision::WorldCollision {
    let origin = crossing_tile(10, 20, 0);
    let width = 5;
    let height = 5;
    let plane_cells = width * height;
    let cells = plane_cells * nav_planner::collision::LEVELS;
    let mut blocked = vec![0u64; cells.div_ceil(64)];
    for tile in blocked_tiles {
        let index = tile.level as usize * plane_cells
            + (tile.z - origin.z) as usize * width
            + (tile.x - origin.x) as usize;
        blocked[index >> 6] |= 1u64 << (index & 63);
    }
    nav_planner::collision::WorldCollision {
        origin,
        width,
        height,
        walk: vec![0; cells],
        blocked,
        flags: None,
    }
}

fn crossing_edge(
    kind: nav_planner::transport::TransportKind,
    at: api::snapshot::WorldTile,
    to: api::snapshot::WorldTile,
    takeoff: Option<api::snapshot::WorldTile>,
    loc_id: i32,
    option: i32,
    dir: Option<nav_planner::transport::DoorDir>,
) -> nav_planner::transport::TransportEdge {
    nav_planner::transport::TransportEdge {
        kind,
        at,
        to,
        takeoff,
        player_delta: None,
        loc_id,
        option,
        ticks: 1,
        dir,
        open_loc_id: None,
        skill_req: Vec::new(),
        item_req: Vec::new(),
        consumed_req: Vec::new(),
        item_returns: Vec::new(),
        quest_req: Vec::new(),
        varp_req: Vec::new(),
        worn_req: Vec::new(),
        worn_all_req: Vec::new(),
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

#[test]
fn crossing_export_options_require_pack_and_output_paths() {
    let expected = ExportInputs {
        pack: PathBuf::from("/tmp/world.navpack"),
        out: PathBuf::from("/tmp/crossings.json"),
    };
    assert_eq!(
        parse_export_args([
            "--export-crossings",
            "/tmp/world.navpack",
            "--out",
            "/tmp/crossings.json"
        ])
        .unwrap(),
        expected
    );
    assert_eq!(
        parse_export_args([
            "--out",
            "/tmp/crossings.json",
            "--export-crossings",
            "/tmp/world.navpack"
        ])
        .unwrap(),
        expected
    );
    for args in [
        vec![],
        vec!["--export-crossings", "/tmp/world.navpack"],
        vec![
            "--export-crossings",
            "/tmp/world.navpack",
            "--out",
            "/tmp/crossings.json",
            "--out",
            "/tmp/other.json",
        ],
        vec![
            "--export-crossings",
            "/tmp/world.navpack",
            "--out",
            "/tmp/crossings.json",
            "--extra",
            "value",
        ],
    ] {
        assert!(parse_export_args(args).is_err());
    }
}

#[test]
fn crossing_export_has_exact_schema_and_serde_quest_gate_values() {
    let collision = crossing_collision(&[]);
    let at = crossing_tile(12, 22, 0);
    let takeoff = crossing_tile(11, 22, 0);
    let to = crossing_tile(13, 22, 0);
    let mut graph = nav_planner::transport::TransportGraph::default();
    let mut edge = crossing_edge(
        nav_planner::transport::TransportKind::Ladder,
        at,
        to,
        Some(takeoff),
        500,
        1,
        None,
    );
    edge.quest_gates = Some(
        nav_planner::quest_gates::QuestGates::new([api::selected::QuestGate::Complete(
            api::selected::FactKey::new("quest"),
        )])
        .unwrap(),
    );
    graph.edges.push(edge);

    let json = crossing_export_json(&collision, &graph).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "schema": 3,
            "crossings": [{
                "crossing": {
                    "loc_id": 500,
                    "option": 1,
                    "at": {"x": 12, "z": 22, "level": 0},
                    "takeoff": {"x": 11, "z": 22, "level": 0},
                    "to": {"x": 13, "z": 22, "level": 0},
                    "kind": "ladder"
                },
                "standable": true,
                "admissible": true,
                "quest_gate_alternatives": [[{"Complete": "quest"}]]
            }],
            "identical_edge_groups": []
        })
    );
}

#[test]
fn crossing_export_distinguishes_both_a2_wall_directions() {
    let collision = crossing_collision(&[]);
    let at = crossing_tile(12, 22, 0);
    let forward = crossing_tile(13, 22, 0);
    let mut graph = nav_planner::transport::TransportGraph::default();
    graph.edges.push(crossing_edge(
        nav_planner::transport::TransportKind::Door,
        at,
        at,
        None,
        1512,
        1,
        Some(nav_planner::transport::DoorDir::E),
    ));
    graph.edges.push(crossing_edge(
        nav_planner::transport::TransportKind::Door,
        at,
        forward,
        None,
        1512,
        1,
        Some(nav_planner::transport::DoorDir::E),
    ));

    let json = crossing_export_json(&collision, &graph).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let rows = value["crossings"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0]["crossing"]["at"],
        serde_json::json!({"x": 12, "z": 22, "level": 0})
    );
    assert_eq!(
        rows[0]["crossing"]["takeoff"],
        serde_json::json!({"x": 11, "z": 22, "level": 0})
    );
    assert_eq!(
        rows[0]["crossing"]["to"],
        serde_json::json!({"x": 12, "z": 22, "level": 0})
    );
    assert_eq!(
        rows[1]["crossing"]["at"],
        serde_json::json!({"x": 12, "z": 22, "level": 0})
    );
    assert_eq!(
        rows[1]["crossing"]["takeoff"],
        serde_json::json!({"x": 12, "z": 22, "level": 0})
    );
    assert_eq!(
        rows[1]["crossing"]["to"],
        serde_json::json!({"x": 13, "z": 22, "level": 0})
    );
    assert_ne!(rows[0]["crossing"], rows[1]["crossing"]);
    assert!(rows
        .iter()
        .all(|row| row["admissible"].as_bool() == Some(true)));
}

#[test]
fn crossing_export_distinguishes_two_a1_takeoffs_at_one_anchor() {
    let collision = crossing_collision(&[]);
    let at = crossing_tile(12, 22, 0);
    let to = crossing_tile(13, 22, 0);
    let mut graph = nav_planner::transport::TransportGraph::default();
    graph.edges.push(crossing_edge(
        nav_planner::transport::TransportKind::Ladder,
        at,
        to,
        Some(crossing_tile(11, 22, 0)),
        3201,
        1,
        None,
    ));
    graph.edges.push(crossing_edge(
        nav_planner::transport::TransportKind::Ladder,
        at,
        to,
        Some(crossing_tile(12, 21, 0)),
        3201,
        1,
        None,
    ));

    let json = crossing_export_json(&collision, &graph).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let rows = value["crossings"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| {
        row["crossing"]["at"] == serde_json::json!({"x": 12, "z": 22, "level": 0})
            && row["admissible"].as_bool() == Some(true)
    }));
    assert_eq!(
        rows[0]["crossing"]["takeoff"],
        serde_json::json!({"x": 11, "z": 22, "level": 0})
    );
    assert_eq!(
        rows[1]["crossing"]["takeoff"],
        serde_json::json!({"x": 12, "z": 21, "level": 0})
    );
    assert_ne!(rows[0]["crossing"], rows[1]["crossing"]);
}

#[test]
fn crossing_export_keeps_each_stage_window_alternative_for_one_crossing() {
    let collision = crossing_collision(&[]);
    let at = crossing_tile(12, 22, 0);
    let takeoff = crossing_tile(11, 22, 0);
    let to = crossing_tile(13, 22, 0);
    let mut graph = nav_planner::transport::TransportGraph::default();
    for (min, max) in [(1, 3), (5, 7)] {
        let mut edge = crossing_edge(
            nav_planner::transport::TransportKind::Ladder,
            at,
            to,
            Some(takeoff),
            510,
            1,
            None,
        );
        edge.quest_gates = Some(
            nav_planner::quest_gates::QuestGates::new([api::selected::QuestGate::Window(
                api::selected::StageWindow {
                    quest: api::selected::FactKey::new("quest"),
                    signal: api::selected::FactKey::new("stage"),
                    values: api::selected::InclusiveRange {
                        min: Some(min),
                        max: Some(max),
                    },
                },
            )])
            .unwrap(),
        );
        graph.edges.push(edge);
    }

    let json = crossing_export_json(&collision, &graph).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let rows = value["crossings"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]["quest_gate_alternatives"],
        serde_json::json!([
            [{
                "Window": {
                    "quest": "quest",
                    "signal": "stage",
                    "values": {"min": 1, "max": 3}
                }
            }],
            [{
                "Window": {
                    "quest": "quest",
                    "signal": "stage",
                    "values": {"min": 5, "max": 7}
                }
            }]
        ])
    );
}

#[test]
fn crossing_export_collapses_duplicate_physical_crossings() {
    let collision = crossing_collision(&[]);
    let at = crossing_tile(12, 22, 0);
    let to = crossing_tile(13, 22, 0);
    let mut graph = nav_planner::transport::TransportGraph::default();
    let duplicate_edge = crossing_edge(
        nav_planner::transport::TransportKind::Door,
        at,
        to,
        None,
        4200,
        1,
        None,
    );
    graph.edges.push(duplicate_edge.clone());
    graph.edges.push(duplicate_edge);
    let duplicate_teleport = crossing_edge(
        nav_planner::transport::TransportKind::Teleport,
        at,
        to,
        None,
        0,
        0,
        None,
    );
    graph.teleports.push(duplicate_teleport.clone());
    graph.teleports.push(duplicate_teleport);

    let json = crossing_export_json(&collision, &graph).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["crossings"].as_array().unwrap().len(), 2);
    assert_eq!(
        value["identical_edge_groups"],
        serde_json::json!([
            ["graph.edges[0]", "graph.edges[1]"],
            ["graph.teleports[0]", "graph.teleports[1]"]
        ])
    );
}

#[test]
fn crossing_export_uses_a_legal_takeoff_when_the_anchor_is_blocked() {
    let at = crossing_tile(12, 22, 0);
    let takeoff = crossing_tile(11, 22, 0);
    let collision = crossing_collision(&[at]);
    assert!(!collision.standable(at));
    assert!(collision.standable(takeoff));
    let mut graph = nav_planner::transport::TransportGraph::default();
    graph.edges.push(crossing_edge(
        nav_planner::transport::TransportKind::Ladder,
        at,
        crossing_tile(13, 22, 0),
        Some(takeoff),
        777,
        1,
        None,
    ));

    let json = crossing_export_json(&collision, &graph).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let row = &value["crossings"][0];
    assert_eq!(
        row["crossing"]["at"],
        serde_json::json!({"x": 12, "z": 22, "level": 0})
    );
    assert_eq!(
        row["crossing"]["takeoff"],
        serde_json::json!({"x": 11, "z": 22, "level": 0})
    );
    assert!(row["standable"].as_bool() == Some(true));
    assert!(row["admissible"].as_bool() == Some(true));
}
