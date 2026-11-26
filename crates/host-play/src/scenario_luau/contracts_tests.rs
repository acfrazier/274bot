//! Failure modes: wrong heads admit a contract; a non-tinderbox or non-light
//! use-on admits one; the SC-01 level guard ignores the observed level; a
//! canon held op closes from any message instead of its exact NoTrigger text;
//! a SC-01-line held op borrows the canon NoTrigger contract; the dark hole
//! contract admits another op or another loc.

use super::*;
use client::client::{Client, ClientConfig};
use client::io::ClientRevision;

fn snapshot(firemaking: i32) -> GameSnapshot {
    let mut client = Client::new_with_revision(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 0,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        ClientRevision::R377,
    );
    client.ingame = true;
    client.stat_effective_level[FIREMAKING as usize] = firemaking;
    client.stat_base_level[FIREMAKING as usize] = firemaking;
    client.gens.stat += 1;
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(&client);
    snapshot
}

const CANON: Heads<'static> = Heads {
    revision: 377,
    engine: R377_CANON_ENGINE,
    content: R377_CANON_CONTENT,
};
const SC01: Heads<'static> = Heads {
    revision: 377,
    engine: R377_SC01_ENGINE,
    content: R377_SC01_CONTENT,
};
const TILE: WorldTile = WorldTile {
    x: 3169,
    z: 3172,
    level: 0,
};

#[test]
fn unknown_heads_admit_no_contract() {
    let light = FileAction::UseOnHeld {
        source: CANON_IDS.tinderbox,
        target: CANON_IDS.oil_lamp_unlit,
    };
    for heads in [
        Heads {
            revision: 289,
            ..CANON
        },
        Heads {
            content: R377_SC01_CONTENT,
            ..CANON
        },
        Heads {
            engine: "0000000000000000000000000000000000000000",
            ..CANON
        },
    ] {
        assert!(select(heads, light, &snapshot(1)).is_err());
    }
}

#[test]
fn light_source_contract_needs_the_tinderbox_and_a_pinned_unlit_source() {
    let contract = select(
        CANON,
        FileAction::UseOnHeld {
            source: CANON_IDS.oil_lamp_unlit,
            target: CANON_IDS.tinderbox,
        },
        &snapshot(1),
    )
    .expect("tinderbox on oil lamp in either order");
    let Closure::Terminal(terminal) = contract.closure else {
        panic!("light closes by its terminal inventory record");
    };
    assert_eq!(
        terminal.items,
        vec![CANON_IDS.oil_lamp_unlit, CANON_IDS.oil_lamp_lit]
    );
    assert_eq!(terminal.inventories, &[Domain::Carried]);
    for (source, target) in [
        (CANON_IDS.oil_lamp_unlit, CANON_IDS.oil_lantern_unlit),
        (CANON_IDS.tinderbox, CANON_IDS.oil_lamp_lit),
    ] {
        assert!(select(
            CANON,
            FileAction::UseOnHeld { source, target },
            &snapshot(1)
        )
        .is_err());
    }
}

#[test]
fn sc01_line_guard_follows_the_observed_firemaking_level() {
    let lantern = FileAction::UseOnHeld {
        source: CANON_IDS.tinderbox,
        target: CANON_IDS.oil_lantern_unlit,
    };
    let below = select(SC01, lantern, &snapshot(25)).expect("guarded lantern");
    assert_eq!(
        below.closure,
        Closure::SilentGuard {
            skill: FIREMAKING,
            level: 26,
            item: CANON_IDS.oil_lantern_unlit
        }
    );
    let at = select(SC01, lantern, &snapshot(26)).expect("lantern at level");
    assert!(matches!(at.closure, Closure::Terminal(_)));
    // The canon content has no level guard, so level 25 still lights there.
    assert!(matches!(
        select(CANON, lantern, &snapshot(25))
            .expect("canon lantern")
            .closure,
        Closure::Terminal(_)
    ));
}

#[test]
fn canon_extinguish_closes_only_by_its_exact_no_trigger_text() {
    let extinguish = FileAction::HeldOp {
        item: CANON_IDS.oil_lamp_lit,
        op: 2,
    };
    let contract = select(CANON, extinguish, &snapshot(1)).expect("canon no-trigger held op");
    let Closure::Terminal(terminal) = contract.closure else {
        panic!("held no-trigger is a terminal message");
    };
    assert_eq!(
        terminal.message.as_deref(),
        Some("No trigger for [opheld2,oil_lamp_lit]")
    );
    assert!(terminal.items.is_empty());
    assert!(select(SC01, extinguish, &snapshot(1)).is_err());
    assert!(select(
        CANON,
        FileAction::HeldOp {
            item: CANON_IDS.oil_lamp_lit,
            op: 1
        },
        &snapshot(1)
    )
    .is_err());
}

#[test]
fn helmet_wear_and_remove_close_on_carried_or_worn_records() {
    for action in [
        FileAction::HeldOp {
            item: CANON_IDS.helmet_unlit,
            op: 2,
        },
        FileAction::Unequip {
            item: CANON_IDS.helmet_lit,
        },
    ] {
        let Closure::Terminal(terminal) = select(CANON, action, &snapshot(1))
            .expect("helmet contract")
            .closure
        else {
            panic!("equip family is terminal");
        };
        assert_eq!(terminal.inventories, &[Domain::Carried, Domain::Worn]);
    }
    assert!(select(
        CANON,
        FileAction::Unequip {
            item: CANON_IDS.tinderbox
        },
        &snapshot(1)
    )
    .is_err());
}

#[test]
fn dark_hole_contract_is_op1_on_the_exact_loc_type() {
    let contract = select(
        CANON,
        FileAction::LocOp {
            loc: CANON_IDS.goblin_cave_entrance,
            tile: TILE,
            op: 1,
        },
        &snapshot(1),
    )
    .expect("dark hole");
    assert_eq!(
        contract.closure,
        Closure::NoTrigger {
            message: "No trigger for [oploc1,goblin_cave_entrance]".into()
        }
    );
    for (loc, op) in [
        (CANON_IDS.goblin_cave_entrance, 2),
        (CANON_IDS.goblin_cave_entrance + 1, 1),
    ] {
        assert!(select(
            CANON,
            FileAction::LocOp {
                loc,
                tile: TILE,
                op
            },
            &snapshot(1)
        )
        .is_err());
    }
}
