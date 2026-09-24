use super::*;

#[test]
fn session_latches_the_entry_wizards_return_anchor() {
    let aubury = essence_session_for_wizard(553).expect("aubury is a wizard");
    assert_eq!(aubury.wizard_npc, 553);
    assert_eq!(
        aubury.return_tile,
        WorldTile {
            x: 3253,
            z: 3401,
            level: 0
        },
        "aubury's `^essence_mine_to_aubury` anchor"
    );
    let sedridor = essence_session_for_wizard(300).expect("sedridor is a wizard");
    assert_eq!(
        sedridor.return_tile,
        WorldTile {
            x: 3106,
            z: 9572,
            level: 0
        }
    );
    assert_eq!(
        essence_session_for_wizard(7),
        None,
        "a cart driver latches nothing"
    );
}

#[test]
fn entry_edges_are_the_wizard_npc_hops_only() {
    let mut entry = essence_return_edge(
        ESSENCE_MINE_PORTALS[0],
        &essence_session_for_wizard(553).unwrap(),
    );
    entry.kind = TransportKind::Npc;
    entry.loc_id = 553; // the wizard npc, like `essence_mine_edges`
    entry.to = WorldTile {
        x: 2912,
        z: 4833,
        level: 0,
    };
    assert!(is_essence_entry_edge(&entry));
    let cart = TransportEdge {
        kind: TransportKind::Npc,
        at: WorldTile {
            x: 2834,
            z: 2954,
            level: 0,
        },
        to: WorldTile {
            x: 2776,
            z: 3214,
            level: 0,
        },
        loc_id: 511,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
    };
    assert!(
        !is_essence_entry_edge(&cart),
        "a cart driver is not an entry hop"
    );
}

#[test]
fn the_return_edge_targets_only_the_session_wizard() {
    let aubury = essence_session_for_wizard(553).unwrap();
    for &portal in ESSENCE_MINE_PORTALS {
        let edge = essence_return_edge(portal, &aubury);
        assert_eq!(edge.kind, TransportKind::EssenceExit);
        assert_eq!(edge.loc_id, ESSENCE_MINE_PORTAL_LOC_ID);
        assert_eq!(edge.option, 1, "the exit is `oploc1`");
        assert_eq!(
            edge.to, aubury.return_tile,
            "returns to the entry wizard only"
        );
    }
    let sedridor = essence_session_for_wizard(300).unwrap();
    let edge = essence_return_edge(ESSENCE_MINE_PORTALS[0], &sedridor);
    assert_eq!(edge.to, sedridor.return_tile);
    assert_ne!(
        edge.to, aubury.return_tile,
        "a different wizard returns elsewhere"
    );
}

#[test]
fn mine_enclosure_covers_the_landings_and_portals() {
    // The four portal placements and the pad sit inside the enclosure;
    // a tile outside the cave walls does not.
    for &p in ESSENCE_MINE_PORTALS {
        assert!(in_essence_mine(p), "portal at {p:?} inside the mine");
    }
    assert!(in_essence_mine(WorldTile {
        x: 2912,
        z: 4833,
        level: 0
    }));
    assert!(in_essence_mine(WorldTile {
        x: 2896,
        z: 4809,
        level: 0
    }));
    assert!(!in_essence_mine(WorldTile {
        x: 3253,
        z: 3401,
        level: 0
    }));
    assert!(!in_essence_mine(WorldTile {
        x: 2879,
        z: 4833,
        level: 0
    }));
    assert!(!in_essence_mine(WorldTile {
        x: 2912,
        z: 4833,
        level: 1
    }));
}
