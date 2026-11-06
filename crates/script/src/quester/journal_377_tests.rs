//! Canon 377 journal recovery: quest_{sheep,runemysteries,romeojuliet}/scripts/*_journal.rs2.
use super::super::{compile, progress};
use super::super::probe::{known_empty_bank, Choice, Probe};
use super::bundled_path_for;
use api::quest_facts::QuestCatalog;
use api::quest_progress::{EvidenceStamp, JournalRead};
use api::selected::{ClientRevision, FactKey, Knowledge, RunKey, Truth};
use api::snapshot::GameSnapshot;
use std::sync::Arc;

#[test]
fn selected_377_journal_paths_compile_and_recover_source_states() {
    api::selected::FamilyPreparation::run(|worker| {
        let selected = api::game_data::for_revision(ClientRevision::R377).unwrap();
        let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
        let pin = selected.selected_pin().unwrap();
        let stamp = EvidenceStamp {
            run: RunKey { slot: 1, run: 1, session: 1 },
            tick: 10,
            sequence: 10,
        };
        for (id, cases) in [
            ("sheep", vec![
                ("sheep:0", "I can start this quest by speaking to Farmer Fred at his|farm just a little way North West of Lumbridge"),
                ("sheep:1", "I asked Farmer Fred, near Lumbridge, for a quest. Fred|said he'd pay me for shearing his sheep for him!|I need to collect 20 more balls of wool."),
                ("sheep:1", "I asked Farmer Fred, near Lumbridge, for a quest. Fred|said he'd pay me for shearing his sheep for him!|I need to collect 1 more ball of wool."),
                ("sheep:enough", "I asked Farmer Fred, near Lumbridge, for a quest. Fred|said he'd pay me for shearing his sheep for him!|I have enough balls of wool to give Fred and get my reward|money!"),
                ("sheep:22", "I brought Farmer Fred 20 balls of wool, and he paid me for|it!|QUEST COMPLETE!"),
            ]),
            ("runemysteries", vec![
                ("runemysteries:0", "I can start this quest by speaking to Duke Horacio of|Lumbridge, upstairs in Lumbridge Castle"),
                ("runemysteries:1", "I spoke to Duke Horacio and he showed me a strange|talisman that had been found by one of his subjects.|I need to find the Head Wizard and give him the Talisman"),
                ("runemysteries:2", "I spoke to Duke Horacio and he showed me a strange|talisman that had been found by one of his subjects.|I gave the Talisman to the Head Wizard of the Tower and|agreed to help him with his research into rune stones.|I should talk to the Head Wizard again"),
                ("runemysteries:3", "I gave the Talisman to the Head Wizard of the Tower and|agreed to help him with his research into rune stones.|I should take this Research Package to Aubury in Varrock"),
                ("runemysteries:4", "I took the research package to Varrock and delivered it.|I should speak to Aubury again when he has finished|examining the research package I have delivered him."),
                ("runemysteries:5", "I took the research package to Varrock and delivered it.|Aubury was interested in the research package and gave|me his own research notes to deliver to Sedridor.|I should take the notes to Sedridor and see what he says"),
                ("runemysteries:6", "Aubury was interested in the research package and gave|me his own research notes to deliver to Sedridor.|I delivered the notes to Sedridor. As a thank you|for my help, he granted me permission to use the Rune|Essence Mine whenever I please.|QUEST COMPLETE!"),
            ]),
            ("romeojuliet", vec![
                ("romeojuliet:0", "I can start this quest by talking to Romeo in Varrock|Square."),
                ("romeojuliet:10", "I have agreed to find Juliet for Romeo and tell her how he|feels. For some reason he can't just do this himself.|I should go and speak to Juliet. I can find her west of|Varrock."),
                ("romeojuliet:20", "I found Juliet on the Western edge of Varrock, and told her|about Romeo. She gave me a message to take back.|I should take the message from Juliet to Romeo in Varrock|central square."),
                ("romeojuliet:30", "I delivered the message to Romeo, and he was sad to hear|that Juliet's father opposed their marriage. However, he|said that Father Lawrence might be able to overcome this.|I should find Father Lawrence and see how we can help"),
                ("romeojuliet:40", "I found Father Lawrence and he suggested the use of a|potion to fool Juliet's father that she is dead so that|Romeo and Juliet can be together in peace.|I need to find the Apothecary to make a cadava potion."),
                ("romeojuliet:50", "I went to the Apothecary regarding making this cadava|potion, and he told me to bring him some cadava berries.|I will have to find some cadava berries."),
                ("romeojuliet:50", "I went to the Apothecary regarding making this cadava|potion, and he told me to bring him some cadava berries.|I should take these cadava berries to the Apothecary."),
                ("romeojuliet:50", "I went to the Apothecary regarding making this cadava|potion, and he told me to bring him some cadava berries.|I should take this cadava potion to Juliet."),
                ("romeojuliet:60", "I went to the Apothecary regarding making this cadava|potion, and he told me to bring him some cadava berries.|After the Apothecary made me the potion, I delivered it to|Juliet. She asked me to tell Romeo the plan.|I have to find Romeo and tell him what's happened."),
                ("romeojuliet:100", "I went to the Apothecary regarding making this cadava|potion, and he told me to bring him some cadava berries.|I told Romeo what was going to happen, but I'm not exactly|sure he understood what was happening. Ah well, I was|rewarded for all of my help regardless.|QUEST COMPLETE!"),
            ]),
        ] {
            let bytes = bundled_path_for(ClientRevision::R377, id).unwrap();
            let path = compile::compile_path(bytes, &selected, &quests, worker, &crate::local_state::OperatorLoadouts::absent())
                .unwrap_or_else(|error| panic!("{id}: {error:?}"));
            for (stage, text) in cases {
                let read = JournalRead {
                    quest: path.id.clone(), root: 8134,
                    lines: text.split('|').map(Arc::<str>::from).collect::<Vec<_>>().into(),
                    colour: None, acquired: stamp, closed: stamp, pin: Arc::clone(&pin),
                };
                let recovered = progress::resolve_journal(&path, &read, None);
                assert!(matches!(&recovered.stage, Knowledge::Known(value) if value.0.as_ref() == stage), "{id} expected {stage}, got {:?}", recovered.stage);
                if id == "sheep" && stage == "sheep:1" {
                    let flag = recovered.flags.iter().find(|flag| flag.flag.0.as_ref() == "sheep:balls_to_go").unwrap();
                    assert_eq!(flag.truth, Truth::True);
                    assert_eq!(flag.count, Some(if text.contains("20 more") { 20 } else { 1 }));
                }
            }
            if id == "sheep" {
                // sheep_journal.rs2:15-25 prints the additional count, not
                // the total obligation. Every possible partial hand-in
                // must remain numeric, including its singular branch.
                for remaining in 1..=20 {
                    let text = format!("I asked Farmer Fred, near Lumbridge, for a quest.|I need to collect {remaining} more {} of wool.", if remaining == 1 { "ball" } else { "balls" });
                    let read = JournalRead {
                        quest: path.id.clone(), root: 8134,
                        lines: text.split('|').map(Arc::<str>::from).collect::<Vec<_>>().into(),
                        colour: None, acquired: stamp, closed: stamp, pin: Arc::clone(&pin),
                    };
                    let recovered = progress::resolve_journal(&path, &read, None);
                    assert!(matches!(&recovered.stage, Knowledge::Known(value) if value.0.as_ref() == "sheep:1"));
                    assert_eq!(recovered.flags[0].count, Some(remaining));
                }
                let document: serde_json::Value = serde_json::from_slice(bytes).unwrap();
                assert!(!document.to_string().contains("minus_item"), "377's count already excludes held balls");
                let sequences = document["roles"][0]["sequences"].as_array().unwrap();
                let collecting = sequences.iter().find(|sequence| sequence["stage"] == "sheep:1").unwrap();
                assert_eq!(collecting["steps"][0]["id"], "hand-held");
                assert_eq!(collecting["steps"][0]["advances"], true, "partial hand-in must reread the journal before producing");
                let enough = sequences.iter().find(|sequence| sequence["stage"] == "sheep:enough").unwrap();
                assert_eq!(enough["steps"].as_array().unwrap().len(), 1, "enough balls must go straight to Fred");
                assert_eq!(enough["steps"][0]["kind"], "talk");
                assert_eq!(document["quest"]["items"].as_array().unwrap().len(), 1, "do not pre-provision fixed twenty-item needs after a partial hand-in");
            }
        }
    }).unwrap().join().unwrap();
}

#[test]
fn hetty_377_journal_state_skips_consumed_items_and_selects_cauldron() {
    api::selected::FamilyPreparation::run(|worker| {
        let selected = api::game_data::for_revision(ClientRevision::R377).unwrap();
        let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
        let pin = selected.selected_pin().unwrap();
        let stamp = EvidenceStamp {
            run: RunKey { slot: 1, run: 1, session: 1 },
            tick: 10,
            sequence: 10,
        };
        let bytes = bundled_path_for(ClientRevision::R377, "hetty").unwrap();
        let path = compile::compile_path(
            bytes,
            &selected,
            &quests,
            worker,
            &crate::local_state::OperatorLoadouts::absent(),
        )
        .unwrap_or_else(|error| panic!("hetty: {error:?}"));
        super::super::probe::assert_no_shadowed_rules(&path);

        let read = |text: &str| JournalRead {
            quest: path.id.clone(),
            root: 8134,
            lines: text.split('|').map(Arc::<str>::from).collect::<Vec<_>>().into(),
            colour: None,
            acquired: stamp,
            closed: stamp,
            pin: Arc::clone(&pin),
        };
        let started = progress::resolve_journal(
            &path,
            &read("I spoke to Hetty in her house in Rimmington. Hetty told me|she could increase my magic power if I can bring her|certain ingredients for a potion."),
            None,
        );
        assert!(matches!(&started.stage, Knowledge::Known(stage) if stage.0.as_ref() == "hetty:1"));
        let objects_given = progress::resolve_journal(
            &path,
            &read("I brought her an onion, a rat's tail, a piece of burnt meat|and an eye of newt which she used to make a potion.||I should drink from the cauldron and improve my magic!"),
            None,
        );
        assert!(matches!(&objects_given.stage, Knowledge::Known(stage) if stage.0.as_ref() == "hetty:2"));
        let completed = progress::resolve_journal(
            &path,
            &read("I brought her an onion, a rat's tail, a piece of burnt meat|and an eye of newt which she used to make a potion.||I drank from the cauldron and my magic power increased!||QUEST COMPLETE!"),
            None,
        );
        assert!(matches!(&completed.stage, Knowledge::Known(stage) if stage.0.as_ref() == "hetty:3"));

        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(Vec::new(), 28);
        let bank = known_empty_bank();
        let started_probe = Probe {
            path: &path,
            selected: &selected,
            quests: &quests,
            progress: std::slice::from_ref(&started),
            bank: &bank,
        };
        assert_eq!(
            started_probe.choice("hetty:1", 0, &snapshot),
            Choice::Step(FactKey::new("get-eye-of-newt")),
            "missing chat lines must not block authored ingredient acquisition"
        );
        let objects_given_probe = Probe {
            path: &path,
            selected: &selected,
            quests: &quests,
            progress: std::slice::from_ref(&objects_given),
            bank: &bank,
        };
        assert_eq!(
            objects_given_probe.choice("hetty:2", 0, &snapshot),
            Choice::Step(FactKey::new("drink-hetty-cauldron")),
            "the journal's objects-given state must resume at the cauldron"
        );
        let ingredients: Vec<_> = path
            .provisioning
            .items
            .iter()
            .filter(|item| item.kind == compile::CompiledItemKind::Acquirable)
            .collect();
        assert_eq!(ingredients.len(), 4);
        assert!(
            ingredients.iter().all(|item| item.acquire.is_none()),
            "stage-authored acquisition steps own these goals; provisioner must not restore ingredients consumed by hand-in"
        );
    })
    .unwrap()
    .join()
    .unwrap();
}


#[test]
fn rune_377_lost_talisman_path_selects_dukes_quest_branch() {
    let document: serde_json::Value =
        serde_json::from_slice(bundled_path_for(ClientRevision::R377, "runemysteries").unwrap())
            .unwrap();
    let sequence = document["roles"][0]["sequences"]
        .as_array()
        .unwrap()
        .iter()
        .find(|sequence| sequence["stage"] == "runemysteries:1")
        .unwrap();
    let recover = sequence["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["id"] == "recover-duke")
        .unwrap();
    let prefer: Vec<Arc<str>> = recover["args"]["prefer"]
        .as_array()
        .unwrap()
        .iter()
        .map(|text| Arc::from(text.as_str().unwrap()))
        .collect();
    // duke_horacio.rs2:5,7 offers both forms of the menu. The lost-item
    // branch is reached only through the quest option (:12-15,40-47).
    for choices in [
        vec!["Have you any quests for me?", "Where can I find money?"],
        vec![
            "I seek a shield that will protect me from the dragon's breath.",
            "Have you any quests for me?",
            "Where can I find money?",
        ],
    ] {
        let options: Vec<_> = choices
            .iter()
            .enumerate()
            .map(|(index, text)| api::snapshot::ChatOptionView {
                component_id: index as i32,
                text: (*text).into(),
            })
            .collect();
        let selected = super::super::families::dialogue::pick_preferred(&options, &prefer);
        assert_eq!(
            selected.map(|index| options[index].text.as_str()),
            Some("Have you any quests for me?"),
            "recover-duke must not fall back to the final money option"
        );
    }
}

#[test]
fn sheep_377_path_targets_shearable_forms_and_prefers_freds_hand_in() {
    let document: serde_json::Value =
        serde_json::from_slice(bundled_path_for(ClientRevision::R377, "sheep").unwrap()).unwrap();
    let shear = document["quest"]["acquire"]["acquire:wool"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["id"] == "shear")
        .unwrap();
    assert_eq!(
        shear["args"]["target"]["npc"],
        serde_json::json!(["sheepunsheered", "sheepunsheered3"]),
        "the Path must allow both placed shearable NPC forms, not The Thing"
    );

    // fred_the_farmer.rs2:163-166 offers these exact choices once
    // %the_thing_sheared is 1.
    let menu = ["I'm back!", "Fred! Fred! I've seen The Thing!"];
    let options: Vec<_> = menu
        .iter()
        .enumerate()
        .map(|(index, text)| api::snapshot::ChatOptionView {
            component_id: index as i32,
            text: (*text).into(),
        })
        .collect();
    for (stage, step_id) in [
        ("sheep:1", "hand-held"),
        ("sheep:1", "hand-in"),
        ("sheep:enough", "hand-in-enough"),
    ] {
        let sequence = document["roles"][0]["sequences"]
            .as_array()
            .unwrap()
            .iter()
            .find(|sequence| sequence["stage"] == stage)
            .unwrap();
        let step = sequence["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|step| step["id"] == step_id)
            .unwrap();
        let prefer: Vec<Arc<str>> = step["args"]["prefer"]
            .as_array()
            .unwrap()
            .iter()
            .map(|text| Arc::from(text.as_str().unwrap()))
            .collect();
        let selected = super::super::families::dialogue::pick_preferred(&options, &prefer);
        assert_eq!(
            selected.map(|index| options[index].text.as_str()),
            Some("I'm back!"),
            "{step_id} must choose Fred's wool hand-in branch"
        );
    }
}

#[test]
fn romeo_377_missing_message_path_skips_juliet_detour() {
    let document: serde_json::Value =
        serde_json::from_slice(bundled_path_for(ClientRevision::R377, "romeojuliet").unwrap())
            .unwrap();
    let sequence = document["roles"][0]["sequences"]
        .as_array()
        .unwrap()
        .iter()
        .find(|sequence| sequence["stage"] == "romeojuliet:20")
        .unwrap();
    let steps = sequence["steps"].as_array().unwrap();
    assert!(
        steps.iter().any(|step| step["id"] == "deliver-message"),
        "Romeo's state-20 talk advances even if the message is absent"
    );
    assert!(
        steps.iter().all(|step| step["id"] != "replace-message"),
        "the Path must not walk back to Juliet for an item Romeo does not check"
    );
}

#[test]
fn hetty_377_path_compiles_and_prefers_the_quest_branch() {
    let document: serde_json::Value =
        serde_json::from_slice(bundled_path_for(ClientRevision::R377, "hetty").unwrap()).unwrap();
    assert!(
        document["roles"][0]["progress"]["rules"]
            .as_array()
            .unwrap()
            .is_empty(),
        "hetty is not in the selected 377 journal button table"
    );
    let start = document["roles"][0]["sequences"][0]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["id"] == "start-hetty")
        .unwrap()
        .clone();
    let prefer: Vec<Arc<str>> = start["args"]["prefer"]
        .as_array()
        .unwrap()
        .iter()
        .map(|text| Arc::from(text.as_str().unwrap()))
        .collect();
    // hetty.rs2:5 last option does not start the quest. :43 last option does not set the varp.
    let menus: [&[&str]; 2] = [
        &[
            "I am in search of a quest.",
            "I've heard that you are a witch.",
        ],
        &[
            "Yes help me become one with my darker side.",
            "No I have my principles and honour.",
            "What, you mean improve my magic?",
        ],
    ];
    for choices in menus {
        let options: Vec<api::snapshot::ChatOptionView> = choices
            .iter()
            .enumerate()
            .map(|(index, text)| api::snapshot::ChatOptionView {
                component_id: index as i32 + 1,
                text: (*text).into(),
            })
            .collect();
        let selected = super::super::families::dialogue::pick_preferred(&options, &prefer);
        assert_eq!(selected, Some(0), "{choices:?}");
    }
    assert_eq!(
        document["quest"]["acquire"]["acquire:burnt-meat"]
            .as_array()
            .unwrap()
            .iter()
            .find(|step| step["id"] == "cook-beef")
            .unwrap()["args"]["target"]["loc"],
        "rimmington_poor_range",
        "377 places the poor range, not the generic range loc"
    );
    api::selected::FamilyPreparation::run(|worker| {
        let selected = api::game_data::for_revision(ClientRevision::R377).unwrap();
        let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
        let bytes = bundled_path_for(ClientRevision::R377, "hetty").unwrap();
        compile::compile_path(
            bytes,
            &selected,
            &quests,
            worker,
            &crate::local_state::OperatorLoadouts::absent(),
        )
        .unwrap_or_else(|error| panic!("377 Witch's Potion failed to compile: {error:?}"));
        let gobdip = bundled_path_for(ClientRevision::R377, "gobdip").unwrap();
        compile::compile_path(
            gobdip,
            &selected,
            &quests,
            worker,
            &crate::local_state::OperatorLoadouts::absent(),
        )
        .unwrap_or_else(|error| panic!("377 Goblin Diplomacy failed to compile: {error:?}"));
        let vampire = bundled_path_for(ClientRevision::R377, "vampire").unwrap();
        compile::compile_path(
            vampire,
            &selected,
            &quests,
            worker,
            &crate::local_state::OperatorLoadouts::absent(),
        )
        .unwrap_or_else(|error| panic!("377 Vampire Slayer failed to compile: {error:?}"));
        let demon = bundled_path_for(ClientRevision::R377, "demon").unwrap();
        compile::compile_path(
            demon,
            &selected,
            &quests,
            worker,
            &crate::local_state::OperatorLoadouts::absent(),
        )
        .unwrap_or_else(|error| panic!("377 Demon Slayer failed to compile: {error:?}"));
        assert_eq!(
            selected.loc_by_config("rimmington_poor_range").unwrap().id,
            9682
        );
    })
    .unwrap()
    .join()
    .unwrap();
}
