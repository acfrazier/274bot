use api::snapshot::{QuestListStatus, QuestStatusView};

fn row(colour: i32) -> QuestStatusView {
    QuestStatusView {
        component_id: 7350,
        name: "Waterfall Quest".into(),
        colour,
    }
}

#[test]
fn quest_list_status_maps_only_documented_display_colours() {
    assert_eq!(row(0xF80000).status(), QuestListStatus::NotStarted);
    assert_eq!(row(0xF8F800).status(), QuestListStatus::InProgress);
    assert_eq!(row(0x00F800).status(), QuestListStatus::Complete);
    assert_eq!(row(0xFFFF00).status(), QuestListStatus::Unknown);
}

#[test]
fn quest_list_status_uses_the_frozen_declared_strings() {
    assert_eq!(QuestListStatus::NotStarted.as_str(), "notStarted");
    assert_eq!(QuestListStatus::InProgress.as_str(), "inProgress");
    assert_eq!(QuestListStatus::Complete.as_str(), "complete");
    assert_eq!(QuestListStatus::Unknown.as_str(), "unknown");
}
