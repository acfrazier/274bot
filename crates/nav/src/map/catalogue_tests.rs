use super::*;
use crate::map::identity::Digest;
use crate::map::producer::{derive_catalogue, ClientMapInput};
use client::io::cache_289::synthetic_jag;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// Anvil marker (Key row 10), minigame marker (Jagex's `???` row 38), a
/// nameless Trade loc without a mapfunction, and a nameless marker for 49,
/// which lies past the Key legend.
const LOCS: [&[u8]; 4] = [
    &[60, 0, 10, 0],
    &[60, 0, 38, 0],
    &[30, b'T', b'r', b'a', b'd', b'e', b'\n', 0],
    &[60, 0, 49, 0],
];

struct Snapshot(PathBuf);
impl Snapshot {
    /// One mapsquare (50, 50) with flat land and loc `id` placed at local (0, id).
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "catalogue-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // `LocType::unpack` starts reading loc.dat at byte 2, after the count.
        let mut dat = vec![0, LOCS.len() as u8];
        let mut idx = vec![0, LOCS.len() as u8];
        for loc in LOCS {
            dat.extend(loc);
            idx.extend((loc.len() as u16).to_be_bytes());
        }
        std::fs::write(
            dir.join("config"),
            synthetic_jag(&[("loc.dat", &dat), ("loc.idx", &idx)]),
        )
        .unwrap();
        let mut index = (50u16 << 8 | 50).to_be_bytes().to_vec();
        index.extend(0u16.to_be_bytes());
        index.extend(1u16.to_be_bytes());
        index.push(0);
        std::fs::write(
            dir.join("versionlist"),
            synthetic_jag(&[("map_index", &index)]),
        )
        .unwrap();
        // Every id delta is 1; each position delta (x<<6|z) + 1 is a one-byte smart.
        let mut locs = Vec::new();
        for id in 0..LOCS.len() as u8 {
            locs.extend([1, id + 1, 22 << 2, 0]);
        }
        locs.push(0);
        let land = vec![0; 4 * 64 * 64];
        let mut maps = Vec::new();
        for (file, payload) in [land, locs].into_iter().enumerate() {
            maps.extend((file as u32).to_le_bytes());
            maps.extend((payload.len() as u32).to_le_bytes());
            maps.extend(payload);
        }
        std::fs::write(dir.join("maps.bin"), maps).unwrap();
        Self(dir)
    }

    fn records(&self) -> Vec<PoiRecord> {
        let input = ClientMapInput::new(289, Digest([7; 32]), &self.0, &self.0).unwrap();
        let (pois, _) = derive_catalogue(input).unwrap();
        pois.records.as_slice().to_vec()
    }
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn nameless_mapfunction_loc_takes_its_key_legend_name() {
    let records = Snapshot::new().records();
    let named: Vec<_> = records
        .iter()
        .filter(|record| record.key.id < 2)
        .map(|record| (record.key.id, record.key.entity, record.name.as_str()))
        .collect();
    assert_eq!(
        named,
        [
            (0, EntityKind::MapFunction, "Anvil"),
            (1, EntityKind::MapFunction, "Minigames"),
        ]
    );
}

#[test]
fn nameless_loc_without_a_key_legend_name_is_not_a_poi() {
    let records = Snapshot::new().records();
    assert!(
        records.iter().all(|record| record.key.id < 2),
        "unlabelled places leaked into the catalogue: {records:?}"
    );
}

#[test]
fn quest_start_names_aggregate_nearby_targets_and_keep_far_markers_generic() {
    let catalog = QuestStartCatalog {
        entries: vec![
            QuestStartEntry {
                display: "Cook's Assistant".to_string(),
                kind: QuestStartTargetKind::Npc,
                id: 10,
            },
            QuestStartEntry {
                display: "Rune Mysteries Quest".to_string(),
                kind: QuestStartTargetKind::Npc,
                id: 11,
            },
            QuestStartEntry {
                display: "Arena Quest".to_string(),
                kind: QuestStartTargetKind::Loc,
                id: 20,
            },
        ],
        npc_placements: vec![
            QuestStartPlacement {
                id: 10,
                x: 100,
                z: 200,
                plane: 0,
            },
            QuestStartPlacement {
                id: 11,
                x: 101,
                z: 200,
                plane: 0,
            },
        ],
        loc_ids: HashSet::from([20]),
    };
    let loc_placements = [QuestStartPlacement {
        id: 20,
        x: 300,
        z: 300,
        plane: 0,
    }];
    let name = quest_start_name_at(&catalog, 0, 100.5, 200.5, &loc_placements)
        .expect("nearby quest starts are named");
    assert_eq!(name.as_str(), "Cook's Assistant, Rune Mysteries Quest");
    assert!(quest_start_name_at(&catalog, 0, 500.5, 500.5, &loc_placements).is_none());
}

/// Rare-Trees markers (mapfunction 34, empty name) take the nearest visited
/// woodcutting tree's client-cache loc name on the same plane. Same-tile
/// markers match exactly; an isolated marker with no tree within range on its
/// own plane (a tree directly above it on plane 1 doesn't count) stays an
/// honest generic label.
#[test]
fn rare_trees_markers_take_nearby_tree_names() {
    fn tree_loc(name: &str) -> Vec<u8> {
        let mut loc = vec![2];
        loc.extend(name.bytes());
        loc.push(10);
        loc.push(30);
        loc.extend("Chop down".bytes());
        loc.push(10);
        loc.push(0);
        loc
    }
    let yew = tree_loc("Yew tree");
    let magic = tree_loc("Magic tree");
    let maple = tree_loc("Maple tree");
    let willow = tree_loc("Willow");
    let marker: &[u8] = &[60, 0, 34, 0];
    // id0 Yew, id1 Magic, id2 marker=Yew tile, id3 marker=Magic tile,
    // id4 Maple, id5 marker=Maple tile, id6 isolated marker, id7 Willow on
    // plane 1 directly above the isolated marker.
    let locs: Vec<&[u8]> = vec![
        &yew, &magic, marker, marker, &maple, marker, marker, &willow,
    ];
    // Local placements (plane, x=0, z): Yew 0, Magic 5, marker 0, marker 5,
    // Maple 10, marker 10, isolated marker 20, Willow plane 1 z 20.
    let tiles: [(u16, u16); 8] = [
        (0, 0),
        (0, 5),
        (0, 0),
        (0, 5),
        (0, 10),
        (0, 10),
        (0, 20),
        (1, 20),
    ];
    let dir = std::env::temp_dir().join(format!(
        "catalogue-rare-trees-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut dat = vec![0, locs.len() as u8];
    let mut idx = vec![0, locs.len() as u8];
    for loc in &locs {
        dat.extend(*loc);
        idx.extend((loc.len() as u16).to_be_bytes());
    }
    std::fs::write(
        dir.join("config"),
        synthetic_jag(&[("loc.dat", &dat), ("loc.idx", &idx)]),
    )
    .unwrap();
    let mut index = (50u16 << 8 | 50).to_be_bytes().to_vec();
    index.extend(0u16.to_be_bytes());
    index.extend(1u16.to_be_bytes());
    index.push(0);
    std::fs::write(
        dir.join("versionlist"),
        synthetic_jag(&[("map_index", &index)]),
    )
    .unwrap();
    let mut placements = Vec::new();
    for (plane, z) in tiles {
        // Position delta is packed (plane << 12 | x << 6 | z) + 1 as a smart.
        let delta = (plane << 12 | z) + 1;
        placements.push(1);
        if delta < 128 {
            placements.push(delta as u8);
        } else {
            placements.extend((delta + 32768).to_be_bytes());
        }
        placements.extend([22 << 2, 0]);
    }
    placements.push(0);
    let land = vec![0; 4 * 64 * 64];
    let mut maps = Vec::new();
    for (file, payload) in [land, placements].into_iter().enumerate() {
        maps.extend((file as u32).to_le_bytes());
        maps.extend((payload.len() as u32).to_le_bytes());
        maps.extend(payload);
    }
    std::fs::write(dir.join("maps.bin"), maps).unwrap();
    let input = ClientMapInput::new(289, Digest([7; 32]), &dir, &dir).unwrap();
    let (pois, _) = derive_catalogue(input).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let records = pois.records.as_slice();
    // Trees carry no service or mapfunction, so only the four markers remain.
    assert_eq!(records.len(), 4, "{records:?}");
    let mut names: Vec<(u32, &str)> = records
        .iter()
        .map(|record| (record.key.id, record.name.as_str()))
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            (2, "Yew tree"),
            (3, "Magic tree"),
            (5, "Maple tree"),
            (6, "Rare Trees"),
        ],
        "{records:?}"
    );
    // Only the isolated marker remains generic.
    let unknown = pois
        .coverage
        .unresolved
        .as_slice()
        .iter()
        .find(|issue| {
            matches!(
                issue.reason,
                crate::map::formats::CoverageReason::UnknownMapFunction
            )
        })
        .map(|issue| issue.count);
    assert_eq!(
        unknown,
        Some(1),
        "{:?}",
        pois.coverage.unresolved.as_slice()
    );
}
