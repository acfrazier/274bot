//! Catalog ranking uses captured account facts, not scene-booth proximity.

use std::sync::Arc;
#[path = "common/bank_slot.rs"]
mod bank_slot;
use bank_slot::BankSlot;

use api::named_banks::{NamedBank, NamedBankFacts, BANK_CATALOG};
use api::quest_progress::EvidenceStamp;
use api::snapshot::WorldTile;
use nav::named_banks::resolve;
use script::bank::{AccessKind, BankPickReceipt, BankStandAccess, PickKind, SelectedBank};
use script::isolate_fb::{
    NativeFactsInput, QuestStatusInput, ReachViewInput, SnapshotInput, StatInput, TileInput,
};
use script::native::{HostEffect, InteractionReceipt, WalkEnd, WalkReceipt};
use script::shim::InteractReq;
use script::LoadShape;

fn base_snapshot<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
        here: Some(TileInput {
            x: 3200,
            z: 3200,
            level: 0,
        }),
        ingame: true,
        inv: &[],
        inv_size: 28,
        stats: &[],
        booths: &[],
        banks: &[],
        bank: &[],
        bank_side: &[],
        bank_open: false,
        bank_loaded: false,
        bank_generation: 0,
        count_dialog_open: false,
        withdraw_x_result_seq: 0,
        withdraw_x_result: false,
        bank_op_result_seq: 0,
        bank_op_result: false,
        hold: false,
        ours: false,
        npcs: &[],
        locs: &[],
        players: &[],
        ground: &[],
        equipment: &[],
        chat_open: false,
        chat_continue: false,
        chat_text: None,
        chat_options: &[],
        side_tab: 0,
        varps: &[],
        combat_styles: &[],
        run_energy: 0,
        run_enabled: false,
        retaliate_enabled: false,
        my_name: Some("bot"),
        in_combat: false,
        animating: false,
        main_modal_id: -1,
        chat_modal_id: -1,
        make_products: &[],
        side_tab_ifaces: &[],
        spell_buttons: &[],
        chat_lines: &[],
        nearest_booth: None,
        bank_note_on: -1,
        bank_note_off: -1,
        scene_state: 2,
        weight: 0,
        combat_level: 0,
        camera_yaw: 0,
        camera_pitch: 0,
        teleports_enabled: false,
        self_slot: 0,
        trade_offer_open: false,
        trade_confirm_open: false,
        trade_partner: None,
        trade_mine: &[],
        trade_theirs: &[],
        trade_side: &[],
        trade_accept_id: -1,
        trade_decline_id: -1,
        shop_open: false,
        shop_stock: &[],
        reach: ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
        user_move_intent_seq: 0,
        walk_outcome_cancel_reason: Default::default(),
    }
}

fn spawn(src: &str, facts: NamedBankFacts) -> BankSlot {
    BankSlot::new(src, LoadShape::CompatClass, None, Arc::new(facts))
}

const PROBE: &str = r#"
import { nearestBank, nearestBanks, nearestUsableBank } from '../../api/bank/BankLocations.js';
globalThis.pick = (x, z, level = 0) => nearestBank({x, z, level})?.name ?? null;
globalThis.ranked = (x, z, level = 0) => nearestBanks({x, z, level}).map(bank => bank.name);
globalThis.custom = (x, z, name) => nearestUsableBank({x, z, level: 0}, bank => bank.name === name)?.name ?? null;
export default class T extends LoopingBot { loop() {} }
"#;

fn catalog() -> NamedBankFacts {
    let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    resolve(BANK_CATALOG, &data.bank_placements().unwrap().rows, |_| {
        true
    })
}

fn post(iso: &mut BankSlot, fishing_base: i32, quests: &[QuestStatusInput<'_>]) {
    let stats = [StatInput {
        index: 10,
        name: "Fishing",
        xp: 0,
        base: fishing_base,
        effective: 99,
    }];
    let mut snapshot = base_snapshot();
    snapshot.stats = &stats;
    iso.post_snapshot(
        &snapshot,
        NativeFactsInput {
            quest_statuses: Some(quests),
            ..Default::default()
        },
    );
    iso.probe("true").unwrap();
}

#[test]
fn account_gates_use_base_level_and_require_both_quest_and_opt_in() {
    let mut iso = spawn(PROBE, catalog());
    post(&mut iso, 67, &[]);
    assert_eq!(
        iso.probe("ranked(2586,3420).includes('Fishing Guild')")
            .unwrap(),
        false
    );
    assert_eq!(
        iso.probe("ranked(2852,2954).includes('Shilo Village')")
            .unwrap(),
        false
    );
    assert_ne!(iso.probe("pick(3153,9576)").unwrap(), "Zanaris");
    let quests = [
        QuestStatusInput {
            name: "Shilo Village",
            status: "complete",
            component_id: None,
        },
        QuestStatusInput {
            name: "Lost City",
            status: "complete",
            component_id: None,
        },
    ];
    post(&mut iso, 68, &quests);
    assert_eq!(iso.probe("pick(2586,3420)").unwrap(), "Fishing Guild");
    assert_eq!(iso.probe("pick(2852,2954)").unwrap(), "Shilo Village");
    assert_ne!(iso.probe("pick(3153,9576)").unwrap(), "Zanaris");
    iso.slot.post_settings_bag(
        serde_json::json!({"useZanarisBank":true})
            .as_object()
            .unwrap(),
    );
    assert_eq!(iso.probe("pick(3153,9576)").unwrap(), "Zanaris");
    post(&mut iso, 68, &[]);
    assert_ne!(iso.probe("pick(3153,9576)").unwrap(), "Zanaris");
    iso.stop();
}

#[test]
fn air_ranking_uses_mage_approach_and_caller_predicate_without_builtin_gates() {
    let mut iso = spawn(PROBE, catalog());
    post(&mut iso, 1, &[]);
    assert_ne!(iso.probe("pick(3091,3958)").unwrap(), "Mage Arena");
    assert_eq!(
        iso.probe("custom(3091,3958,'Mage Arena')").unwrap(),
        "Mage Arena"
    );
    iso.slot
        .post_settings_bag(serde_json::json!({"useMageBank":true}).as_object().unwrap());
    assert_eq!(iso.probe("pick(3091,3958)").unwrap(), "Mage Arena");
    iso.stop();
}

#[test]
fn air_ties_keep_catalog_order_across_planes_without_isolate_leakage() {
    let mut a = spawn(
        PROBE,
        NamedBankFacts::from_banks(vec![
            NamedBank::new(
                "Upstairs",
                WorldTile {
                    x: 10,
                    z: 11,
                    level: 1,
                },
            ),
            NamedBank::new(
                "Ground",
                WorldTile {
                    x: 11,
                    z: 10,
                    level: 0,
                },
            ),
        ]),
    );
    let mut b = spawn(
        PROBE,
        NamedBankFacts::from_banks(vec![NamedBank::new(
            "Other world",
            WorldTile {
                x: 10,
                z: 10,
                level: 0,
            },
        )]),
    );
    post(&mut a, 1, &[]);
    post(&mut b, 1, &[]);
    assert_eq!(a.probe("pick(10,10)").unwrap(), "Upstairs");
    assert_eq!(
        a.probe("ranked(10,10)").unwrap(),
        serde_json::json!(["Upstairs", "Ground"])
    );
    assert_eq!(b.probe("pick(10,10)").unwrap(), "Other world");
    a.stop();
    b.stop();
}

/// A selected identity controls opening until native facts prove loaded stock.
#[test]
fn reachable_bank_continuations_open_npc_access_and_wait_for_loaded_stock() {
    use script::isolate_fb::{ChatOptionInput, ItemRowInput, SceneEntityInput};

    for (call, after) in [
        ("Banking.open()", 0),
        ("Bank.openNearestWorld()", 0),
        (
            "Banking.bankNearest({deposit: () => false, afterDeposit() { globalThis.__after++; }})",
            1,
        ),
    ] {
        let source = format!(
            r#"
import {{ Bank }} from '../../api/bank/Bank.js';
import {{ Banking }} from '../../api/bank/Banking.js';
globalThis.__after = 0; globalThis.__ok = null;
export default class T extends LoopingBot {{
    async loop() {{
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__ok = await {call};
    }}
}}"#
        );
        let facts = catalog();
        let index = facts
            .banks()
            .iter()
            .position(|bank| bank.name == "Shilo Village")
            .unwrap();
        let index_u16 = u16::try_from(index).unwrap();
        let bank = facts.banks()[index];
        let stand = bank.tile;
        let teller = WorldTile {
            x: stand.x + 1,
            z: stand.z,
            level: stand.level,
        };
        let mut slot = spawn(&source, facts);
        let quests = [QuestStatusInput {
            name: "Shilo Village",
            status: "complete",
            component_id: None,
        }];
        let stats = [StatInput {
            index: 10,
            name: "Fishing",
            xp: 0,
            base: 68,
            effective: 68,
        }];
        let mut snapshot = base_snapshot();
        snapshot.stats = &stats;
        let mut native = NativeFactsInput {
            quest_statuses: Some(&quests),
            ..Default::default()
        };

        slot.tick_from_input(1, &snapshot, native);
        let mut actions = slot.take_actions();
        assert_eq!(actions.len(), 1, "{call}: expected native bank pick");
        let pick = actions.pop().unwrap();
        let HostEffect::BankPick(request) = &pick.effect else {
            panic!("{call}: expected native bank pick");
        };
        assert!(
            request.eligible.contains(&index_u16),
            "{call}: Shilo must be eligible from native quest and level facts"
        );
        let authority = pick.authority();
        let request_id = pick.request_id.get();
        let selected = SelectedBank {
            bank_index: index_u16,
            access_tile: stand,
            kind: PickKind::Reachable,
            access: Some(Arc::new(BankStandAccess {
                bank,
                stand_tile: teller,
                kind: AccessKind::Teller,
                stand_op: 0,
                name: Some(Arc::from("Banker")),
                choose: None,
            })),
        };
        slot.slot.complete_native_bank_pick(
            &authority,
            BankPickReceipt {
                request_id: request_id + 1,
                evidence: EvidenceStamp {
                    run: authority.run(),
                    tick: 2,
                    sequence: 2,
                },
                selected: selected.clone(),
            },
        );
        snapshot.tick = 2;
        slot.tick_from_input(2, &snapshot, native);
        assert!(
            slot.take_actions().is_empty(),
            "{call}: a stale native selection must not start a walk"
        );

        slot.slot.complete_native_bank_pick(
            &authority,
            BankPickReceipt {
                request_id,
                evidence: EvidenceStamp {
                    run: authority.run(),
                    tick: 3,
                    sequence: 3,
                },
                selected,
            },
        );
        snapshot.tick = 3;
        slot.tick_from_input(3, &snapshot, native);
        let mut actions = slot.take_actions();
        assert_eq!(actions.len(), 1, "{call}: expected walk to selected access");
        let walk = actions.pop().unwrap();
        let HostEffect::Walk(request) = &walk.effect else {
            panic!("{call}: expected native walk");
        };
        assert_eq!((request.target, request.radius), (stand, 4));
        let walk_authority = walk.authority();
        slot.slot.complete_native_walk(
            &walk_authority,
            WalkReceipt {
                goal: None,
                request_id: walk.request_id.get(),
                evidence: EvidenceStamp {
                    run: walk_authority.run(),
                    tick: 3,
                    sequence: 3,
                },
                end: WalkEnd::Arrived,
                blocked: None,
                detail: None,
                refusal: None,
                assessment: None,
                escape: None,
            },
        );

        snapshot.tick = 4;
        snapshot.here = Some(TileInput {
            x: stand.x,
            z: stand.z,
            level: stand.level,
        });
        snapshot.reach = ReachViewInput {
            available: true,
            base_x: stand.x,
            base_z: stand.z,
            level: stand.level,
            width: 1,
            height: 1,
            walkable: &[1],
            reachable: &[1],
            reachable_adj: &[1],
            exact_rank: &[0],
            adjacent_rank: &[0],
            step: &[0],
            canlight: &[0],
            stamp: 1,
        };
        let actions = ["Bank".to_string()];
        let npcs = [SceneEntityInput {
            index: 42,
            id: 499,
            name: Some("Banker"),
            x: teller.x,
            z: teller.z,
            level: teller.level,
            distance: 1,
            health: 1,
            max_health: 1,
            in_combat: false,
            animating: false,
            actions: &actions,
            reachable: true,
            reachable_adj: true,
            combat_level: 0,
            target_kind: 0,
            target_index: -1,
            size: 1,
            nx: teller.x,
            nz: teller.z,
            shape: 0,
            angle: 0,
        }];
        snapshot.npcs = &npcs;
        slot.tick_from_input(4, &snapshot, native);
        let mut actions = slot.take_actions();
        assert_eq!(actions.len(), 1, "{call}: expected teller interaction");
        let interaction = actions.pop().unwrap();
        assert!(
            matches!(
                &interaction.effect,
                HostEffect::Interaction(InteractReq::Npc { name, action, index })
                    if name == "Banker" && action == "Bank" && *index == Some(42)
            ),
            "{call}: NPC access must not wait for a booth"
        );
        let interaction_authority = interaction.authority();
        slot.slot.complete_native_interaction(
            &interaction_authority,
            InteractionReceipt {
                request_id: interaction.request_id.get(),
                evidence: EvidenceStamp {
                    run: interaction_authority.run(),
                    tick: 4,
                    sequence: 4,
                },
                accepted: true,
                chat_since: 0,
            },
        );

        snapshot.tick = 5;
        let choices = [ChatOptionInput {
            text: "Tell me about this village.",
            com_id: 2493,
        }];
        snapshot.chat_open = true;
        snapshot.chat_options = &choices;
        slot.tick_from_input(5, &snapshot, native);
        assert!(
            slot.take_actions().is_empty(),
            "absent npcAccess.choose must not select an unrelated first option"
        );
        assert_eq!(
            slot.probe("[__ok,__after]").unwrap(),
            serde_json::json!([null, 0])
        );

        snapshot.tick = 6;
        snapshot.chat_open = false;
        snapshot.chat_options = &[];
        snapshot.bank_open = true;
        snapshot.bank_generation = 1;
        native.side_modal_id = Some(700);
        native.bank_snapshot_generation = Some(0);
        slot.tick_from_input(6, &snapshot, native);
        assert!(
            slot.take_actions().is_empty(),
            "{call}: open bank is not loaded yet"
        );
        assert_eq!(
            slot.probe("[__ok,__after]").unwrap(),
            serde_json::json!([null, 0])
        );

        snapshot.bank_loaded = true;
        let side = [ItemRowInput {
            name: Some("Coins"),
            count: 1,
            id: 995,
            ops: &[],
            noted: false,
            cert: -1,
            component_id: 1,
            slot: 0,
        }];
        snapshot.bank_side = &side;
        native.bank_snapshot_generation = Some(1);
        for tick in 7..=9 {
            snapshot.tick = tick;
            slot.tick_from_input(tick, &snapshot, native);
            assert!(
                slot.take_actions().is_empty(),
                "{call}: empty held inventory must not create a transfer action"
            );
        }
        assert_eq!(
            slot.probe("[__ok,__after]").unwrap(),
            serde_json::json!([true, after]),
            "{call}"
        );
        slot.stop();
    }
}
