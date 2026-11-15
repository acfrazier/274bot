//! The quest records rs2b0t's `api/ai/quests/data/quests.js` exposes to catalog
//! scripts, built from the selected revision's content.
//!
//! The only readers in the pinned catalog are `AccountLeveler/dragons.ts:26-27`
//! and `AccountLeveler/methods.ts:37-42`. They read `id`, `name`,
//! `questPoints`, `requirements.minQuestPoints`, `requirements.quests` and
//! `requirements.skills`, and nothing else, so nothing else is built.
//!
//! Identity (`id`, `name`, `questPoints`) is the selected quest identity.
//! A requirement is a quest Path's start requirement: the same rows
//! `eligibility::evaluate_path` checks before it starts a quest (`at_start`).
//! Requirements that a Path lists only for gathering the quest's materials are
//! not quest requirements and are left out. A quest without a bundled Path has
//! no requirements entry.

use super::path::{QuestRequirementDocument, QuestRequirementKindDocument};
use super::registry::bundled_path_for;
use api::game_data::SelectedGameData;
use api::selected::ClientRevision;
use serde::Deserialize;

/// Content ids that rs2b0t names differently. The catalog script looks these
/// quests up by its own id (`AccountLeveler/dragons.ts:8`).
const RS2B0T_IDS: [(&str, &str); 1] = [("fortress", "blackknight")];

/// One quest record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatQuest {
    pub id: String,
    pub name: String,
    pub quest_points: i32,
    /// `None` when no bundled Path states this quest's requirements.
    pub requirements: Option<CompatRequirements>,
}

/// The start requirements of one quest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompatRequirements {
    pub min_quest_points: Option<u16>,
    /// `(skill, level)`, the skill in the lower case the Path writes.
    pub skills: Vec<(String, u16)>,
    /// Display names of the quests that must be finished first.
    pub quests: Vec<String>,
}

#[derive(Deserialize)]
struct PathHeader {
    quest: Option<QuestHeader>,
}

#[derive(Deserialize)]
struct QuestHeader {
    #[serde(default)]
    requirements: Vec<QuestRequirementDocument>,
}

/// Start requirements from one Path's requirement rows. `display` names a
/// quest id.
pub fn start_requirements(
    rows: &[QuestRequirementDocument],
    display: impl Fn(&str) -> Option<String>,
) -> CompatRequirements {
    let mut out = CompatRequirements::default();
    for row in rows.iter().filter(|row| row.at == "start") {
        match &row.kind {
            QuestRequirementKindDocument::QuestPoints(points) => {
                out.min_quest_points = out.min_quest_points.max(Some(*points));
            }
            QuestRequirementKindDocument::Skill { skill, level } => {
                out.skills.push((skill.clone(), *level));
            }
            QuestRequirementKindDocument::Quest(id) => {
                if let Some(name) = display(id) {
                    out.quests.push(name);
                }
            }
            QuestRequirementKindDocument::Item { .. }
            | QuestRequirementKindDocument::MembersWorld => {}
        }
    }
    out
}

/// Every quest of the selected content, or `None` without quest identity.
pub fn compat_quests(selected: &SelectedGameData) -> Option<Vec<CompatQuest>> {
    let identity = selected.quest_identity()?;
    let revision = match selected.revision() {
        377 => ClientRevision::R377,
        _ => ClientRevision::R289,
    };
    let display = |id: &str| {
        identity
            .rows
            .iter()
            .find(|row| row.id == id)
            .map(|row| row.display.clone())
    };
    Some(
        identity
            .rows
            .iter()
            .filter(|row| row.kind == "quest")
            .map(|row| {
                let requirements = bundled_path_for(revision, &row.id)
                    .and_then(|bytes| serde_json::from_slice::<PathHeader>(bytes).ok())
                    .and_then(|header| header.quest)
                    .map(|quest| start_requirements(&quest.requirements, display));
                let id = RS2B0T_IDS
                    .iter()
                    .find(|(content, _)| *content == row.id)
                    .map_or(row.id.as_str(), |(_, rs2b0t)| rs2b0t);
                CompatQuest {
                    id: id.to_owned(),
                    name: row.display.clone(),
                    quest_points: row.quest_points,
                    requirements,
                }
            })
            .collect(),
    )
}

#[cfg(test)]
#[path = "compat_quests_tests.rs"]
mod tests;
