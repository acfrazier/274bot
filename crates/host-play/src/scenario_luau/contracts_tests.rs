//! Failure modes: wrong heads admit a contract; a non-tinderbox or non-light
//! use-on admits one; the SC-01 level guard ignores the observed level; a
//! canon held Extinguish closes on any message instead of the exact default
//! equip script's no-wearpos text; a SC-01-line held op borrows the canon
//! contract; the dark hole contract admits another op or another loc;
//! `equipment.wear` (`InteractReq::Wear`) does not reach the equip contract,
//! picks an option the native wear send would not take, or guesses between
//! two carried item types of one name.

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
fn canon_extinguish_closes_only_by_the_default_equip_no_wearpos_text() {
    let extinguish = FileAction::HeldOp {
        item: CANON_IDS.oil_lamp_lit,
        op: 2,
    };
    let contract = select(CANON, extinguish, &snapshot(1)).expect("canon held extinguish");
    let Closure::Terminal(terminal) = contract.closure else {
        panic!("held extinguish is a terminal message");
    };
    assert_eq!(
        terminal.message.as_deref(),
        Some("[DEBUG]: No wearpos set on oil_lamp_lit")
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
            diagnostic: "No trigger for [oploc1,goblin_cave_entrance]".into()
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

fn carried(id: i32, slot: i32, name: &str, actions: &[Option<&str>]) -> api::snapshot::ItemView {
    api::snapshot::ItemView {
        def: api::obj_names::ItemDefView {
            id,
            name: Some(name.into()),
            stackable: false,
            members: false,
            base_value: 0,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        container: api::snapshot::ItemContainer::Inventory,
        action_family: api::snapshot::ItemActionFamily::Held,
        slot,
        count: 1,
        actions: actions
            .iter()
            .map(|action| action.map(str::to_owned))
            .collect(),
        component_id: -1,
    }
}

fn wear(name: &str) -> script::interact::InteractReq {
    script::interact::InteractReq::Wear { name: name.into() }
}

#[test]
fn equipment_wear_resolves_to_the_committed_equip_contract() {
    let mut snapshot = snapshot(1);
    snapshot.seed_inventory(
        vec![carried(
            CANON_IDS.helmet_unlit,
            3,
            "Mining helmet",
            &[None, Some("Wear"), None, None, Some("Drop")],
        )],
        28,
    );
    let action = super::super::actions::resolve(&wear("mining helmet"), &snapshot)
        .expect("carried helmet resolves");
    assert_eq!(
        action,
        FileAction::HeldOp {
            item: CANON_IDS.helmet_unlit,
            op: 2
        }
    );
    assert_eq!(
        select(CANON, action, &snapshot).expect("equip contract").id,
        "r377-equip"
    );
}

#[test]
fn equipment_wear_takes_the_native_option_order_and_refuses_guesses() {
    let mut wield = snapshot(1);
    wield.seed_inventory(
        vec![carried(
            1277,
            0,
            "Bronze sword",
            &[Some("Equip"), Some("Wield"), None, None, Some("Drop")],
        )],
        28,
    );
    assert_eq!(
        super::super::actions::resolve(&wear("Bronze sword"), &wield),
        Ok(FileAction::HeldOp { item: 1277, op: 2 })
    );

    let mut none = snapshot(1);
    none.seed_inventory(
        vec![carried(
            590,
            0,
            "Tinderbox",
            &[Some("Use"), None, None, None, Some("Drop")],
        )],
        28,
    );
    assert!(super::super::actions::resolve(&wear("Tinderbox"), &none).is_err());

    let mut two = snapshot(1);
    two.seed_inventory(
        vec![
            carried(5014, 0, "Mining helmet", &[None, Some("Wear")]),
            carried(5013, 1, "Mining helmet", &[None, Some("Wear")]),
        ],
        28,
    );
    assert!(super::super::actions::resolve(&wear("Mining helmet"), &two).is_err());
    assert!(super::super::actions::resolve(&wear("Iron helmet"), &two).is_err());
}

// Revision-289 failure modes: an R289 action borrows a revision-377 contract,
// or a 377 or unpinned head admits an R289 contract; Bury admits another item
// or another op, or closes on the "dig a hole" line that precedes the delete;
// the booth's Use (op1) is admitted although a nearby teller turns it into
// dialogue; Use-quickly is admitted while a bank is already open, so its
// "opened" record is not fresh; Withdraw-X (op5, a suspended count dialog) or
// a withdraw from a closed bank is admitted; close is admitted with no bank
// open, or closes before the cycle that runs its queued reorganize; the
// native OpenBooth, Withdraw, Close and Bury requests resolve to a different
// op than the one the normal owner sends.

const R289: Heads<'static> = Heads {
    revision: 289,
    engine: R289_ENGINE,
    content: R289_CONTENT,
};
const BOOTH_TILE: WorldTile = WorldTile {
    x: 3253,
    z: 3420,
    level: 0,
};

fn r289_snapshot(bank: Option<Vec<api::snapshot::ItemView>>) -> GameSnapshot {
    let mut snapshot = snapshot(1);
    snapshot.seed_inventory(
        vec![carried(
            R289_BONES,
            0,
            "Bones",
            &[Some("Bury"), None, None, None, Some("Drop")],
        )],
        28,
    );
    if let Some(rows) = bank {
        snapshot.seed_bank_observation(5382, 1, Some(rows), Vec::new());
    }
    snapshot
}

fn banked(id: i32, slot: i32, name: &str) -> api::snapshot::ItemView {
    api::snapshot::ItemView {
        container: api::snapshot::ItemContainer::Bank,
        count: 28,
        ..carried(
            id,
            slot,
            name,
            &[
                Some("Withdraw-1"),
                Some("Withdraw-5"),
                Some("Withdraw-10"),
                Some("Withdraw-All"),
                Some("Withdraw-X"),
            ],
        )
    }
}

fn booth() -> api::snapshot::LocView {
    api::snapshot::LocView {
        typecode: 10,
        info: 0,
        id: R289_BANK_BOOTH,
        name: Some("Bank booth".into()),
        description: None,
        actions: vec![Some("Use".into()), Some("Use-quickly".into())],
        tile: BOOTH_TILE,
        distance: 1,
        layer: api::snapshot::LocLayer::Ground,
        shape: 10,
        angle: 0,
        width: 1,
        length: 1,
        footprint_width: 1,
        footprint_length: 1,
        block_walk: true,
        block_range: false,
        active: true,
        animation: -1,
        map_function: -1,
        map_scene: -1,
        force_approach: 0,
    }
}

#[test]
fn revision_289_contracts_bind_only_to_the_pinned_289_heads() {
    let bury = FileAction::HeldOp {
        item: R289_BONES,
        op: 1,
    };
    let closed = r289_snapshot(None);
    assert_eq!(select(R289, bury, &closed).unwrap().id, "r289-bury-bones");
    for heads in [
        CANON,
        SC01,
        Heads {
            engine: "0000000000000000000000000000000000000000",
            ..R289
        },
        Heads {
            content: R377_CANON_CONTENT,
            ..R289
        },
        Heads {
            revision: 377,
            ..R289
        },
    ] {
        assert!(select(heads, bury, &closed).is_err());
    }
    let light = FileAction::UseOnHeld {
        source: CANON_IDS.tinderbox,
        target: CANON_IDS.oil_lamp_unlit,
    };
    assert!(select(R289, light, &closed).is_err());
}

#[test]
fn bury_closes_on_the_deleting_message_and_only_for_normal_bones_op1() {
    let contract = select(
        R289,
        FileAction::HeldOp {
            item: R289_BONES,
            op: 1,
        },
        &r289_snapshot(None),
    )
    .unwrap();
    let Closure::Terminal(terminal) = contract.closure else {
        panic!("Bury closes by its terminal record");
    };
    assert_eq!(terminal.message.as_deref(), Some("You bury the bones."));
    assert_eq!(terminal.items, vec![R289_BONES]);
    assert_eq!(terminal.inventories, &[Domain::Carried]);
    // The random-event branch runs after the message: closure waits one more
    // PLAYER_INFO and watches for every prayer event npc type.
    assert!(terminal.settle);
    for npc in [407, 409, 410, 411, 419, 424, 425, 430, 956] {
        assert!(terminal.descendants.contains(&npc), "{npc}");
    }
    for action in [
        FileAction::HeldOp {
            item: R289_BONES,
            op: 2,
        },
        FileAction::HeldOp {
            item: R289_BONES + 2,
            op: 1,
        },
    ] {
        assert!(select(R289, action, &r289_snapshot(None)).is_err());
    }
}

#[test]
fn bank_open_needs_use_quickly_on_the_pinned_booth_with_the_bank_closed() {
    let quickly = FileAction::LocOp {
        loc: R289_BANK_BOOTH,
        tile: BOOTH_TILE,
        op: 2,
    };
    let contract = select(R289, quickly, &r289_snapshot(None)).unwrap();
    assert_eq!(contract.id, "r289-bank-open-quickly");
    let Closure::Terminal(terminal) = contract.closure else {
        panic!("bank open closes by its terminal record");
    };
    assert_eq!(terminal.bank, Some(BankEdge::Opened));
    let open = r289_snapshot(Some(vec![banked(R289_BONES, 0, "Bones")]));
    assert!(select(R289, quickly, &open).is_err());
    for action in [
        FileAction::LocOp {
            loc: R289_BANK_BOOTH,
            tile: BOOTH_TILE,
            op: 1,
        },
        FileAction::LocOp {
            loc: R289_BANK_BOOTH + 2,
            tile: BOOTH_TILE,
            op: 2,
        },
    ] {
        assert!(select(R289, action, &r289_snapshot(None)).is_err());
    }
}

#[test]
fn withdraw_and_close_need_an_open_bank_and_close_settles_one_more_cycle() {
    let open = r289_snapshot(Some(vec![banked(R289_BONES, 0, "Bones")]));
    let closed = r289_snapshot(None);
    let all = FileAction::BankWithdraw {
        item: R289_BONES,
        op: 4,
    };
    let contract = select(R289, all, &open).unwrap();
    assert_eq!(contract.id, "r289-bank-withdraw");
    let Closure::Terminal(terminal) = contract.closure else {
        panic!("withdraw closes by its terminal record");
    };
    assert_eq!(terminal.inventories, &[Domain::Carried, Domain::Bank]);
    assert!(select(R289, all, &closed).is_err());
    assert!(select(
        R289,
        FileAction::BankWithdraw {
            item: R289_BONES,
            op: 5
        },
        &open
    )
    .is_err());

    let close = select(R289, FileAction::CloseModal, &open).unwrap();
    let Closure::Terminal(terminal) = close.closure else {
        panic!("close closes by its terminal record");
    };
    assert_eq!(terminal.bank, Some(BankEdge::Closed));
    assert!(
        terminal.settle,
        "the queued reorganize runs after the close"
    );
    assert!(select(R289, FileAction::CloseModal, &closed).is_err());
}

#[test]
fn native_bank_and_bury_requests_resolve_to_the_ops_the_owner_sends() {
    use script::interact::InteractReq;
    let mut closed = r289_snapshot(None);
    closed.seed_locs(vec![booth()]);
    let resolve = super::super::actions::resolve;
    assert_eq!(
        resolve(
            &InteractReq::OpenBooth {
                x: BOOTH_TILE.x,
                z: BOOTH_TILE.z,
                level: BOOTH_TILE.level,
                id: R289_BANK_BOOTH,
                name: None,
                action: None,
            },
            &closed
        ),
        Ok(FileAction::LocOp {
            loc: R289_BANK_BOOTH,
            tile: BOOTH_TILE,
            op: 2
        })
    );
    assert_eq!(
        resolve(
            &InteractReq::Held {
                name: "Bones".into(),
                action: "Bury".into(),
                slot: None,
                target_item_id: None,
            },
            &closed
        ),
        Ok(FileAction::HeldOp {
            item: R289_BONES,
            op: 1
        })
    );
    let open = r289_snapshot(Some(vec![banked(R289_BONES, 0, "Bones")]));
    assert_eq!(
        resolve(
            &InteractReq::Withdraw {
                name: "bones".into(),
                action: "Withdraw-All".into(),
            },
            &open
        ),
        Ok(FileAction::BankWithdraw {
            item: R289_BONES,
            op: 4
        })
    );
    assert_eq!(
        resolve(&InteractReq::Close, &open),
        Ok(FileAction::CloseModal)
    );
    assert!(resolve(
        &InteractReq::Withdraw {
            name: "Coins".into(),
            action: "Withdraw-All".into(),
        },
        &open
    )
    .is_err());
}

// Direct-root correlation failure modes: an already-true check from another
// domain (the bank modal, another item, a stat) closes an open Bury root; the
// family's own item or exact message check is refused; a bank close closes
// from an item check instead of the closed bank.
#[test]
fn a_direct_root_closes_only_from_a_check_of_its_own_terminal_domain() {
    use scenario::file_scenario::FileCheck;
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let parse = |name: &str, args: serde_json::Value| {
        FileCheck::parse(name, args.as_object().unwrap().clone()).unwrap()
    };
    let bury = select(
        R289,
        FileAction::HeldOp {
            item: R289_BONES,
            op: 1,
        },
        &r289_snapshot(None),
    )
    .unwrap();
    let closes = |contract: &Contract, name: &str, check: &FileCheck| {
        closes_direct_root(contract, name, check, &selected)
    };
    let bank_closed = parse("bank_closed", serde_json::json!({}));
    assert!(!closes(&bury, "bank_closed", &bank_closed));
    assert!(!closes(
        &bury,
        "item_count",
        &parse("item_count", serde_json::json!({"item": 995, "eq": 0}))
    ));
    assert!(!closes(
        &bury,
        "stat",
        &parse("stat", serde_json::json!({"id": 5, "min": 1}))
    ));
    assert!(closes(
        &bury,
        "item_at_most",
        &parse(
            "item_at_most",
            serde_json::json!({"name": "bones", "count": 0})
        )
    ));
    assert!(closes(
        &bury,
        "received",
        &parse(
            "received",
            serde_json::json!({"after": 1, "messages": [{"type": 0, "text": "You bury the bones."}]})
        )
    ));

    let open = r289_snapshot(Some(vec![banked(R289_BONES, 0, "Bones")]));
    let close = select(R289, FileAction::CloseModal, &open).unwrap();
    assert!(closes(&close, "bank_closed", &bank_closed));
    assert!(!closes(
        &close,
        "item_at_most",
        &parse(
            "item_at_most",
            serde_json::json!({"name": "bones", "count": 0})
        )
    ));
}
