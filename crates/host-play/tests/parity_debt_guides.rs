//! Content-backed route witness for the released Tourist Trap crossing.
use api::snapshot::WorldTile;
use nav::router::{find_with, FindOptions, Leg, RouteError};
use nav::zones::ZoneExempt;

#[test]
#[ignore = "requires WORLD_NAV_PACK pointing at the selected 289 content pack"]
fn desert_captain_cross_grants_only_required_ugthanki() {
    let document: serde_json::Value =
        serde_json::from_str(include_str!("../../script/paths/289/desertrescue.json")).unwrap();
    fn crossing(value: &serde_json::Value) -> Option<&serde_json::Value> {
        if value.get("id").and_then(serde_json::Value::as_str) == Some("walk-to-captain") {
            return Some(value);
        }
        match value {
            serde_json::Value::Array(rows) => rows.iter().find_map(crossing),
            serde_json::Value::Object(rows) => rows.values().find_map(crossing),
            _ => None,
        }
    }
    let args = &crossing(&document).expect("released captain crossing")["args"];
    assert!(args.get("allow_danger_zones").is_none());
    let names: Vec<_> = args["cross"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap())
        .collect();
    // content/maps/m51_47.jm2:5027,5034; maps/m51_48.jm2:4425.
    assert_eq!(
        names,
        [
            "ugthanki@3268,3052,0",
            "ugthanki@3281,3056,0",
            "ugthanki@3305,3089,0"
        ]
    );
    let pack = std::fs::read(std::env::var_os("WORLD_NAV_PACK").unwrap()).unwrap();
    let (collision, graph, _) = nav::pack::decode(&pack).unwrap();
    let table = graph.zones.as_ref().unwrap();
    let keys: Vec<_> = names
        .iter()
        .map(|name| table.resolve(name).unwrap())
        .collect();
    let origin = WorldTile {
        x: 3302,
        z: 3114,
        level: 0,
    };
    let destination = WorldTile {
        x: 3270,
        z: 3029,
        level: 0,
    };
    let route = |keys: &[_], combat| {
        find_with(
            &collision,
            &graph,
            origin,
            destination,
            FindOptions {
                zones: ZoneExempt::named(keys).unwrap(),
                ..Default::default()
            },
            &nav::WorldState {
                combat_level: Some(combat),
                map_members: true,
                ..Default::default()
            },
        )
    };
    // desert.npc:20,32-34,51,57,64-66 and all.hunt:37-45: both hunters
    // remain aggressive at combat 40; above 84 neither level-rule zone applies.
    for combat in [3, 40] {
        assert_eq!(route(&[], combat), Err(RouteError::NoPath));
        let found = route(&keys, combat).expect("three spawn-local grants permit the route");
        let mut crossed = std::collections::BTreeSet::new();
        for leg in &found.legs {
            let Leg::Walk { tiles } = leg else {
                panic!("crossing must walk real geometry")
            };
            for &tile in tiles {
                for index in table.at(tile) {
                    let zone = &table.zones()[usize::from(index)];
                    let kind = &table.kinds()[usize::from(zone.kind)];
                    crossed.insert(format!(
                        "{}@{},{},{}",
                        kind.id, zone.spawn_x, zone.spawn_z, zone.level
                    ));
                }
            }
        }
        assert_eq!(crossed, names.iter().map(|name| name.to_string()).collect());
        for (omitted, name) in names.iter().enumerate() {
            let reduced: Vec<_> = keys
                .iter()
                .enumerate()
                .filter_map(|(index, key)| (index != omitted).then_some(*key))
                .collect();
            assert_eq!(
                route(&reduced, combat),
                Err(RouteError::NoPath),
                "removing required grant {name} must refuse the route"
            );
        }
        println!(
            "combat={combat}: crossed exactly {crossed:?}; every proper two-grant subset refused"
        );
    }
    assert!(route(&[], 85).is_ok());
}
