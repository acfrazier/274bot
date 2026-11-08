//! Canon 377 journal recovery and observable batch-two Path step selection.
use super::super::{compile, progress};
use super::bundled_path_for;
use crate::quester::probe::{known_empty_bank, progress_for_stage, Choice, Probe};
use api::game_data::SelectedGameData;
use api::quest_facts::QuestCatalog;
use api::quest_progress::{EvidenceStamp, JournalRead};
use api::selected::{ClientRevision, Knowledge, RunKey};
use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView};
use serde_json::Value;
use std::sync::Arc;

fn selection_snapshot(
    data: &SelectedGameData,
    tile: api::WorldTile,
    items: &[(&str, i32)],
) -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_tile(tile);
    snapshot.seed_local_player(crate::quester::families::tests::local_player(tile));
    snapshot.seed_inventory(
        items
            .iter()
            .enumerate()
            .map(|(slot, (alias, count))| ItemView {
                def: crate::quester::families::tests::def(
                    data.item_by_alias(alias).unwrap().id,
                    alias,
                ),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: slot as i32,
                count: *count,
                actions: Vec::new(),
                component_id: 0,
            })
            .collect(),
        28,
    );
    snapshot.seed_equipment(Vec::new());
    snapshot.seed_locs(Vec::new());
    snapshot
}

#[test]
fn batch2_377_journals_recover_every_action_boundary() {
    crate::require_local_game_data_for_test!(ClientRevision::R377);
    let Some(nav) = compile::real_nav_lease(ClientRevision::R377) else {
        return;
    };
    api::selected::FamilyPreparation::run(|worker| {
        let selected = api::game_data::for_revision(ClientRevision::R377).unwrap();
        let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
        let stamp = EvidenceStamp {
            run: RunKey { slot: 1, run: 1, session: 1 },
            tick: 10,
            sequence: 10,
        };
        // quests/quest_{gobdip,vampire,demon}/scripts/*_journal.rs2.
        // Include repeated history: later pages retain earlier milestone text.
        for (id, cases) in [
            ("gobdip", vec![
                ("gobdip:0", "I can start this quest by talking to the Bartender in|Port Sarim."),
                ("gobdip:1", "I spoke to the bartender in Port Sarim who heard a|rumour that the goblins up by the mountain are arguing|about the colour of their armour.|I should investigate before it turns into a full blown war."),
                ("gobdip:2", "I offered to help the generals by finding another colour that they both like.|I should bring the goblins some Orange Goblin Armour."),
                ("gobdip:3", "I gave some Orange Goblin Armour to General Wartface|and General Bentnoze, but they didn't like it.|They asked me to bring them some Blue Goblin Armour|to try out instead."),
                ("gobdip:4", "I gave some Orange Goblin Armour to General Wartface|and General Bentnoze, but they didn't like it.|I gave some Blue Goblin Armour to General Wartface|and General Bentnoze, but they didn't like it.|They asked me to bring them some of their original Brown|Goblin Armour to try out instead."),
                ("gobdip:5", "I gave some Orange Goblin Armour to General Wartface|and General Bentnoze, but they didn't like it.|I gave some Blue Goblin Armour to General Wartface|and General Bentnoze, but they didn't like it.|I gave General Wartface and General Bentnoze some of|their original Brown Goblin Armour.|I should see if they liked the Brown Goblin Armour."),
                ("gobdip:6", "I gave General Wartface and General Bentnoze some of|their original Brown Goblin Armour, and it turned out to be|their favourite.|QUEST COMPLETE!"),
            ]),
            ("vampire", vec![
                ("vampire:0", "I can start this quest by speaking to Morgan who is in|Draynor Village."),
                ("vampire:1", "I spoke to Morgan in Draynor Village.|I need to speak to Dr Harlow who can normally be found in|the Jolly Boar Inn North East of Varrock."),
                ("vampire:2", "I have spoken to Dr Harlow. He seemed terribly drunk, and|he kept asking me to buy him drinks.|I should see what advice Dr Harlow can give me about killing|Vampires."),
                ("vampire:2", "I have spoken to Dr Harlow. He seemed terribly drunk, and|he kept asking me to buy him drinks.|Dr Harlow gave me a stake to finish off the Vampire when|I'm fighting it. I've got a hammer to drive the stake deep|into the Vampire's chest, and I have some garlic which|should weaken the Vampire."),
                ("vampire:3", "I have spoken to Dr Harlow. He seemed terribly drunk, and|he kept asking me to buy him drinks.|I have killed the Vampire, Count Draynor.|QUEST COMPLETE!"),
            ]),
            ("demon", vec![
                ("demon:0", "I can start this quest by speaking to the Gypsy in Varrock."),
                ("demon:1", "I should ask Sir Prysin about Silverlight."),
                ("demon:2", "I need to find 3 keys to open the box containing Silverlight."),
                ("demon:29", "I found all 3 keys and received Silverlight.|Now I should go to the stone circle."),
                ("demon:30", "I found all 3 keys and received Silverlight.|QUEST COMPLETE!"),
            ]),
        ] {
            let path = compile::compile_path(
                bundled_path_for(ClientRevision::R377, id).unwrap(),
                &selected,
                &quests,
                worker,
                &crate::local_state::OperatorLoadouts::absent(),
                nav.zones(),
            ).unwrap_or_else(|error| panic!("{id}: {error:?}"));
            for (stage, text) in cases {
                let read = JournalRead {
                    quest: path.id.clone(),
                    root: 8134,
                    lines: text.split('|').map(Arc::<str>::from).collect::<Vec<_>>().into(),
                    colour: None,
                    acquired: stamp,
                    closed: stamp,
                };
                let recovered = progress::resolve_journal(&path, &read, None);
                assert!(
                    matches!(&recovered.stage, Knowledge::Known(value) if value.0.as_ref() == stage),
                    "{id} expected {stage}, got {:?}", recovered.stage,
                );
            }
        }
    }).unwrap().join().unwrap();
}

fn document(id: &str) -> Value {
    serde_json::from_slice(bundled_path_for(ClientRevision::R377, id).unwrap()).unwrap()
}

fn step<'a>(document: &'a Value, stage: &str, id: &str) -> &'a Value {
    document["roles"][0]["sequences"]
        .as_array()
        .unwrap()
        .iter()
        .find(|sequence| sequence["stage"] == stage)
        .unwrap()["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["id"] == id)
        .unwrap()
}

#[test]
fn batch2_377_dialogue_transitions_settle_on_native_journal_stages() {
    for (id, stage, action, next) in [
        ("gobdip", "gobdip:1", "offer-colour", "gobdip:2"),
        ("gobdip", "gobdip:2", "hand-orange", "gobdip:3"),
        ("gobdip", "gobdip:3", "hand-blue", "gobdip:4"),
        ("vampire", "vampire:1", "ask-harlow", "vampire:2"),
        ("demon", "demon:1", "ask-prysin", "demon:2"),
        ("demon", "demon:2", "receive-silverlight", "demon:29"),
    ] {
        let document = document(id);
        let action = step(&document, stage, action);
        assert_eq!(action["advances"], true, "{id} must reread after {action}");
        assert_eq!(action["settle"]["Fact"]["kind"], "stage_in");
        assert_eq!(
            action["settle"]["Fact"]["args"]["any"],
            serde_json::json!([next])
        );
    }
    let demon = document("demon");
    let ritual = step(&demon, "demon:2", "ask-traiborn");
    assert_eq!(
        ritual["settle"],
        serde_json::json!({"All": []}),
        "the journal does not distinguish the bone counter; only the completed talk is claimed"
    );
    assert_eq!(ritual["args"]["gap_ticks"], 3);
    let hand_in = step(&demon, "demon:2", "give-traiborn-bones");
    assert_eq!(hand_in["args"]["gap_ticks"], 3);
    assert_eq!(
        hand_in["settle"]["Fact"]["args"]["obj"],
        "silverlight_key_1"
    );
    assert_eq!(
        step(&demon, "demon:2", "bank-for-bones")["args"]["op"],
        "deposit_all"
    );
}

#[test]
fn batch2_377_coin_acquisition_exercises_real_palace_spawns() {
    let gobdip = document("gobdip");
    let vampire = document("vampire");
    let demon = document("demon");
    for (action, qty) in [
        (&gobdip["quest"]["acquire"]["acquire:coins"][0], 35),
        (step(&vampire, "vampire:2", "take-beer-coins"), 2),
        (step(&demon, "demon:0", "take-gypsy-coin"), 1),
    ] {
        assert_eq!(action["kind"], "interact");
        assert_eq!(action["args"]["target"]["ground"], "coins");
        assert_eq!(
            action["args"]["anchor"]["tile"],
            serde_json::json!([3232, 3500, 0])
        );
        assert_eq!(
            action["args"]["until"],
            serde_json::json!({"obj": "coins", "qty": qty})
        );
        assert_eq!(action["args"]["wait_if_missing"], true);
    }
}

#[test]
fn vampire_377_authored_stairs_advance_at_the_canonical_landing() {
    crate::require_local_game_data_for_test!(ClientRevision::R377);
    let Some(nav) = compile::real_nav_lease(ClientRevision::R377) else {
        return;
    };
    let selected = api::game_data::for_revision(ClientRevision::R377).unwrap();
    let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
    let path = compile::compile_uncached_for_test_with_zones(
        &serde_json::from_value(document("vampire")).unwrap(),
        &selected,
        &quests,
        nav.zones().expect("real 377 zone catalog"),
    )
    .unwrap();
    let progress = [progress_for_stage(&path, "vampire:2", &[])];
    let bank = known_empty_bank();
    let probe = Probe {
        path: &path,
        selected: &selected,
        quests: &quests,
        progress: &progress,
        bank: &bank,
    };
    for (x, z, level, items, expected) in [
        (3099, 3266, 0, &[][..], "climb-morgan-stairs"),
        (3102, 3266, 1, &[][..], "open-garlic-cupboard"),
        (3101, 3266, 1, &[][..], "open-garlic-cupboard"),
        (3097, 3269, 1, &[][..], "open-garlic-cupboard"),
        (3102, 3266, 1, &[("garlic", 1)][..], "take-hammer-coins"),
        (
            3102,
            3266,
            1,
            &[("garlic", 1), ("beer", 1)][..],
            "take-hammer-coins",
        ),
        (
            3102,
            3266,
            1,
            &[("garlic", 1), ("coins", 1)][..],
            "buy-hammer",
        ),
        (
            3102,
            3266,
            1,
            &[("garlic", 1), ("stake", 1), ("coins", 1)][..],
            "buy-hammer",
        ),
        (
            3102,
            3266,
            1,
            &[("garlic", 1), ("hammer", 1)][..],
            "take-beer-coins",
        ),
    ] {
        let snapshot = selection_snapshot(&selected, api::WorldTile { x, z, level }, items);
        assert_eq!(
            probe.choice("vampire:2", 0, &snapshot),
            Choice::Step(api::selected::FactKey::new(expected)),
            "the authored selector must not repeat completed stairs at {x},{z},{level}"
        );
    }
}

#[test]
fn demon_377_authored_bone_preparation_advances_after_deposit() {
    crate::require_local_game_data_for_test!(ClientRevision::R377);
    let Some(nav) = compile::real_nav_lease(ClientRevision::R377) else {
        return;
    };
    let selected = api::game_data::for_revision(ClientRevision::R377).unwrap();
    let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
    let path = compile::compile_uncached_for_test_with_zones(
        &serde_json::from_value(document("demon")).unwrap(),
        &selected,
        &quests,
        nav.zones().expect("real 377 zone catalog"),
    )
    .unwrap();
    let progress = [progress_for_stage(&path, "demon:2", &[])];
    let bank = known_empty_bank();
    let probe = Probe {
        path: &path,
        selected: &selected,
        quests: &quests,
        progress: &progress,
        bank: &bank,
    };
    let tile = api::WorldTile {
        x: 3185,
        z: 3436,
        level: 0,
    };
    for (items, expected) in [
        (&[("coins", 3), ("trout", 5)][..], "bank-for-bones"),
        (&[("coins", 3)][..], "collect-25-bones"),
        (&[("coins", 3), ("bones", 24)][..], "collect-25-bones"),
        (&[("coins", 3), ("bones", 25)][..], "ask-traiborn"),
    ] {
        let snapshot = selection_snapshot(&selected, tile, items);
        assert_eq!(
            probe.choice("demon:2", 0, &snapshot),
            Choice::Step(api::selected::FactKey::new(expected)),
            "the authored selector must preserve the post-bank bone collection boundary: {items:?}"
        );
    }
}

#[test]
fn demon_377_authored_drain_sequence_advances_after_the_observed_pour() {
    crate::require_local_game_data_for_test!(ClientRevision::R377);
    let Some(nav) = compile::real_nav_lease(ClientRevision::R377) else {
        return;
    };
    let selected = api::game_data::for_revision(ClientRevision::R377).unwrap();
    let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
    let path = compile::compile_uncached_for_test_with_zones(
        &serde_json::from_value(document("demon")).unwrap(),
        &selected,
        &quests,
        nav.zones().expect("real 377 zone catalog"),
    )
    .unwrap();
    let progress = [progress_for_stage(&path, "demon:2", &[])];
    let bank = known_empty_bank();
    let probe = Probe {
        path: &path,
        selected: &selected,
        quests: &quests,
        progress: &progress,
        bank: &bank,
    };
    for (tile, bucket, poured, expected) in [
        ((3223, 3494), "bucket_empty", false, "fill-bucket"),
        ((3223, 3494), "bucket_water", false, "wash-key-down-drain"),
        ((3223, 3494), "bucket_empty", true, "climb-down-manhole"),
        ((3237, 9858), "bucket_empty", true, "walk-to-sewer-key"),
        ((3237, 9858), "bucket_empty", false, "walk-to-sewer-key"),
    ] {
        let mut snapshot = selection_snapshot(
            &selected,
            api::WorldTile {
                x: tile.0,
                z: tile.1,
                level: 0,
            },
            &[
                ("silverlight_key_1", 1),
                ("silverlight_key_2", 1),
                (bucket, 1),
            ],
        );
        snapshot.seed_chat_lines(
            poured
                .then(|| api::snapshot::ChatLineView {
                    sequence: 1,
                    type_: 0,
                    username: None,
                    text: "Ok, I think I've washed the key down to the sewer.".into(),
                })
                .into_iter()
                .collect(),
        );
        assert_eq!(
            probe.choice("demon:2", 0, &snapshot),
            Choice::Step(api::selected::FactKey::new(expected)),
            "completed pouring must not reselect bucket preparation at {tile:?}"
        );
    }
}

#[test]
fn gobdip_377_authored_funding_tracks_the_remaining_dye_cost() {
    crate::require_local_game_data_for_test!(ClientRevision::R377);
    let Some(nav) = compile::real_nav_lease(ClientRevision::R377) else {
        return;
    };
    let selected = api::game_data::for_revision(ClientRevision::R377).unwrap();
    let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
    let path = compile::compile_uncached_for_test_with_zones(
        &serde_json::from_value(document("gobdip")).unwrap(),
        &selected,
        &quests,
        nav.zones().expect("real 377 zone catalog"),
    )
    .unwrap();
    let progress = [progress_for_stage(&path, "gobdip:1", &[])];
    let bank = known_empty_bank();
    let probe = Probe {
        path: &path,
        selected: &selected,
        quests: &quests,
        progress: &progress,
        bank: &bank,
    };
    for (items, expected) in [
        (
            &[
                ("goblin_armour", 3),
                ("redberries", 3),
                ("onion", 2),
                ("coins", 34),
            ][..],
            "get-coins",
        ),
        (
            &[
                ("goblin_armour", 3),
                ("redberries", 3),
                ("onion", 2),
                ("coins", 35),
            ][..],
            "get-dyes",
        ),
        (
            &[("goblin_armour", 3), ("orangedye", 1), ("coins", 24)][..],
            "get-coins",
        ),
        (
            &[("goblin_armour", 3), ("orangedye", 1), ("coins", 25)][..],
            "get-woad",
        ),
        (
            &[
                ("goblin_armour", 3),
                ("orangedye", 1),
                ("woadleaf", 2),
                ("coins", 4),
            ][..],
            "get-coins",
        ),
        (
            &[
                ("goblin_armour", 3),
                ("orangedye", 1),
                ("woadleaf", 2),
                ("coins", 5),
            ][..],
            "buy-blue-dye",
        ),
        (
            &[
                ("goblin_armour", 3),
                ("orangedye", 1),
                ("woadleaf", 1),
                ("coins", 5),
            ][..],
            "get-coins",
        ),
        (
            &[("goblin_armour", 3), ("orangedye", 1), ("bluedye", 1)][..],
            "offer-colour",
        ),
    ] {
        let snapshot = selection_snapshot(
            &selected,
            api::WorldTile {
                x: 3086,
                z: 3259,
                level: 0,
            },
            items,
        );
        assert_eq!(
            probe.choice("gobdip:1", 0, &snapshot),
            Choice::Step(api::selected::FactKey::new(expected)),
            "spent dye coins must not restart fully funded acquisition, but short funds must: {items:?}"
        );
    }
}
