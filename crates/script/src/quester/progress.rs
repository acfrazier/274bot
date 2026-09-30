//! Colour-only stage read. Journal and quest-gate evidence answering are later slices.
use super::compile::CompiledPath;
use api::quest_facts::QuestCatalog;
use api::selected::FactKey;
use api::snapshot::{QuestListStatus, SnapshotView};

pub fn colour_stage(
    path: &CompiledPath,
    quests: &QuestCatalog,
    snapshot: SnapshotView<'_>,
) -> Option<FactKey> {
    let facts = quests.quest(path.id.0.as_ref()).ok()?;
    let rows = snapshot.quest_statuses()?;
    let row = rows
        .value
        .iter()
        .find(|row| row.name.eq_ignore_ascii_case(facts.display.as_ref()))?;
    match row.status() {
        QuestListStatus::NotStarted => Some(path.colour_not_started.clone()),
        QuestListStatus::Complete => Some(path.colour_complete.clone()),
        QuestListStatus::InProgress => Some(path.colour_in_progress.clone()),
        QuestListStatus::Unknown => None,
    }
}
