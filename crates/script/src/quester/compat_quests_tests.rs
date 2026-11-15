use super::*;
use api::selected::FactKey;

fn row(id: &str, kind: QuestRequirementKindDocument, at: &str) -> QuestRequirementDocument {
    QuestRequirementDocument {
        id: FactKey::new(id),
        kind,
        at: at.to_owned(),
        source: "test".to_owned(),
    }
}

#[test]
fn only_start_requirements_are_quest_requirements() {
    let rows = [
        row("qp", QuestRequirementKindDocument::QuestPoints(12), "start"),
        row(
            "qp-more",
            QuestRequirementKindDocument::QuestPoints(32),
            "start",
        ),
        row(
            "cooking",
            QuestRequirementKindDocument::Skill {
                skill: "cooking".into(),
                level: 10,
            },
            "start",
        ),
        row(
            "mining",
            QuestRequirementKindDocument::Skill {
                skill: "mining".into(),
                level: 15,
            },
            "materials",
        ),
        row(
            "elena",
            QuestRequirementKindDocument::Quest("elena".into()),
            "start",
        ),
        row(
            "gone",
            QuestRequirementKindDocument::Quest("unknown".into()),
            "start",
        ),
        row(
            "bucket",
            QuestRequirementKindDocument::Item {
                obj: "bucket".into(),
                qty: 1,
            },
            "start",
        ),
    ];
    let found = start_requirements(&rows, |id| {
        (id == "elena").then(|| "Plague City".to_owned())
    });
    assert_eq!(
        found,
        CompatRequirements {
            min_quest_points: Some(32),
            skills: vec![("cooking".into(), 10)],
            quests: vec!["Plague City".into()],
        }
    );
}

#[test]
fn selected_content_builds_the_records_the_catalog_reads() {
    if !api::test_support::game_data_available() {
        return;
    }
    let selected = api::game_data::for_revision(ClientRevision::R289).expect("289 game data");
    let quests = compat_quests(&selected).expect("quest identity");
    let by_id = |id: &str| {
        quests
            .iter()
            .find(|quest| quest.id == id)
            .unwrap_or_else(|| panic!("no quest {id}"))
    };

    // `champions_guild.rs2:1-5` and `sir_amik_varze.rs2:8-19`, through the Paths.
    let dragon = by_id("dragon");
    assert_eq!(
        (dragon.name.as_str(), dragon.quest_points),
        ("Dragon Slayer", 2)
    );
    assert_eq!(
        dragon
            .requirements
            .as_ref()
            .and_then(|req| req.min_quest_points),
        Some(32)
    );
    let fortress = by_id("blackknight");
    assert_eq!(fortress.name, "Black Knight's Fortress");
    assert_eq!(
        fortress
            .requirements
            .as_ref()
            .and_then(|req| req.min_quest_points),
        Some(12)
    );
    assert!(
        quests.iter().all(|quest| quest.id != "fortress"),
        "the content id is replaced, not duplicated"
    );
    assert_eq!(
        by_id("squire")
            .requirements
            .as_ref()
            .map(|req| req.skills.clone()),
        Some(vec![("cooking".to_owned(), 10)])
    );
    assert_eq!(
        by_id("cook").requirements,
        Some(CompatRequirements::default())
    );

    // The 16 quests `AccountLeveler/dragons.ts:8,27` looks up by id all exist.
    for id in [
        "runemysteries",
        "cook",
        "doric",
        "sheep",
        "priest",
        "haunted",
        "hetty",
        "romeojuliet",
        "prince",
        "hunt",
        "gobdip",
        "blackknight",
        "vampire",
        "squire",
        "arthur",
        "dragon",
    ] {
        assert!(by_id(id).requirements.is_some(), "{id} has a Path");
    }

    // Miniquests are not quests, and every name is a quest-list name.
    let identity = selected.quest_identity().expect("identity");
    let names: Vec<_> = identity
        .rows
        .iter()
        .filter(|row| row.kind == "quest")
        .map(|row| row.display.as_str())
        .collect();
    assert_eq!(quests.len(), names.len());
    assert!(quests
        .iter()
        .all(|quest| names.contains(&quest.name.as_str())));
}
