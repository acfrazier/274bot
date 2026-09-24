//! v2 scene projections and quest-tab status over the isolate scene.
//!
//! `api.sceneLocs` / `sceneNpcs` / `questStatus` are typed local helpers:
//! they read [`crate::observed`] and return the posted copy. They never
//! enqueue a game op.

use crate::observed::{self, EntityRow, QuestStatusRow, QuestTab, SceneRow};
use api::line_of_sight::CollisionQuery;

pub const SCENE_LIMIT_MAX: i32 = 64;

const QUEST_STATUS_VALUES: [&str; 4] = ["notStarted", "inProgress", "complete", "unknown"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub min_x: i32,
    pub min_z: i32,
    pub max_x: i32,
    pub max_z: i32,
    pub level: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneRowOut {
    pub id: i32,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub actions: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneBounds {
    pub available: bool,
    pub base_x: i32,
    pub base_z: i32,
    pub level: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneProjection {
    pub as_of_sequence: u64,
    pub scene: SceneBounds,
    pub rows: Vec<SceneRowOut>,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneQueryError {
    InvalidArgs,
    MissingIds,
    SnapshotUnavailable,
}

impl SceneQueryError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArgs => "invalid-args",
            Self::MissingIds => "missing-ids",
            Self::SnapshotUnavailable => "snapshot-unavailable",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestStatusHit {
    pub status: String,
    pub as_of_sequence: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestStatusError {
    InvalidArgs,
    SnapshotUnavailable,
    QuestTabUnbound,
    NotOnTab,
}

impl QuestStatusError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArgs => "invalid-args",
            Self::SnapshotUnavailable => "snapshot-unavailable",
            Self::QuestTabUnbound => "quest-tab-unbound",
            Self::NotOnTab => "not-on-tab",
        }
    }
}

/// A–Z fold only. Not Unicode lowercasing.
pub fn fold_ascii(text: &str) -> String {
    text.chars()
        .map(|c| {
            let n = c as u32;
            if (97..=122).contains(&n) {
                char::from_u32(n - 32).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

pub fn scene_locs(
    ids: &[i32],
    limit: i32,
    region: Option<Region>,
) -> Result<SceneProjection, SceneQueryError> {
    if ids.is_empty() {
        return Err(SceneQueryError::MissingIds);
    }
    if !(1..=SCENE_LIMIT_MAX).contains(&limit) {
        return Err(SceneQueryError::InvalidArgs);
    }
    project(Page::Locs, ids, None, limit, region)
}

pub fn scene_npcs(
    types: &[i32],
    actions: &[String],
    limit: i32,
    region: Option<Region>,
) -> Result<SceneProjection, SceneQueryError> {
    if types.is_empty() || actions.is_empty() {
        return Err(SceneQueryError::InvalidArgs);
    }
    if actions.iter().any(|action| action.is_empty()) {
        return Err(SceneQueryError::InvalidArgs);
    }
    if !(1..=SCENE_LIMIT_MAX).contains(&limit) {
        return Err(SceneQueryError::InvalidArgs);
    }
    project(Page::Npcs, types, Some(actions), limit, region)
}

pub fn quest_status(name: &str) -> Result<QuestStatusHit, QuestStatusError> {
    let wanted = fold_ascii(name.trim());
    if wanted.is_empty() {
        return Err(QuestStatusError::InvalidArgs);
    }
    observed::with(|scene| {
        let Some(tab) = scene.latest().quest_statuses() else {
            return Err(QuestStatusError::SnapshotUnavailable);
        };
        match tab {
            QuestTab::Unbound => Err(QuestStatusError::QuestTabUnbound),
            QuestTab::Bound(rows) => {
                let Some(sequence) = scene.tick() else {
                    return Err(QuestStatusError::SnapshotUnavailable);
                };
                match first_quest_row(rows, &wanted) {
                    Some(row) => Ok(QuestStatusHit {
                        status: row.status.to_string(),
                        as_of_sequence: sequence,
                    }),
                    None => Err(QuestStatusError::NotOnTab),
                }
            }
        }
    })
}

pub fn first_quest_row<'a>(
    rows: &'a [QuestStatusRow],
    wanted_folded: &str,
) -> Option<&'a QuestStatusRow> {
    rows.iter()
        .find(|row| quest_row_matches(row, wanted_folded))
}

fn quest_row_matches(row: &QuestStatusRow, wanted_folded: &str) -> bool {
    if !QUEST_STATUS_VALUES
        .iter()
        .any(|status| *status == &*row.status)
    {
        return false;
    }
    fold_ascii(row.name.trim()) == wanted_folded
}

enum Page {
    Locs,
    Npcs,
}

fn project(
    page: Page,
    ids: &[i32],
    actions: Option<&[String]>,
    limit: i32,
    region: Option<Region>,
) -> Result<SceneProjection, SceneQueryError> {
    observed::with(|scene| {
        let lens = scene.latest();
        let Some(collision) = lens.collision() else {
            return Err(SceneQueryError::SnapshotUnavailable);
        };
        if !collision.available {
            return Err(SceneQueryError::SnapshotUnavailable);
        }
        let Some(sequence) = scene.tick() else {
            return Err(SceneQueryError::SnapshotUnavailable);
        };
        let limit = limit as usize;
        let (rows, truncated) = match page {
            Page::Locs => {
                let Some(posted) = lens.locs() else {
                    return Err(SceneQueryError::SnapshotUnavailable);
                };
                take_locs(posted, ids, region, limit)
            }
            Page::Npcs => {
                let Some(posted) = lens.npcs() else {
                    return Err(SceneQueryError::SnapshotUnavailable);
                };
                take_npcs(posted, ids, actions.unwrap_or(&[]), region, limit)
            }
        };
        Ok(SceneProjection {
            as_of_sequence: sequence,
            scene: bounds(collision),
            rows,
            truncated,
        })
    })
}

fn bounds(collision: &CollisionQuery) -> SceneBounds {
    SceneBounds {
        available: collision.available,
        base_x: collision.base_x,
        base_z: collision.base_z,
        level: collision.level,
        width: collision.width,
        height: collision.height,
    }
}

fn in_region(x: i32, z: i32, level: i32, region: Option<Region>) -> bool {
    let Some(region) = region else {
        return true;
    };
    level == region.level
        && x >= region.min_x
        && x <= region.max_x
        && z >= region.min_z
        && z <= region.max_z
}

fn take_locs(
    posted: &[SceneRow],
    ids: &[i32],
    region: Option<Region>,
    limit: usize,
) -> (Vec<SceneRowOut>, bool) {
    let mut rows = Vec::new();
    for entity in posted {
        if !ids.contains(&entity.id) {
            continue;
        }
        if !in_region(entity.x, entity.z, entity.level, region) {
            continue;
        }
        if rows.len() >= limit {
            return (rows, true);
        }
        rows.push(copy_place(
            entity.id,
            entity.x,
            entity.z,
            entity.level,
            &entity.actions,
        ));
    }
    (rows, false)
}

fn take_npcs(
    posted: &[EntityRow],
    ids: &[i32],
    actions: &[String],
    region: Option<Region>,
    limit: usize,
) -> (Vec<SceneRowOut>, bool) {
    let mut rows = Vec::new();
    for entity in posted {
        if !ids.contains(&entity.id) {
            continue;
        }
        if !in_region(entity.x, entity.z, entity.level, region) {
            continue;
        }
        if !entity
            .actions
            .iter()
            .any(|posted| actions.iter().any(|want| want == &**posted))
        {
            continue;
        }
        if rows.len() >= limit {
            return (rows, true);
        }
        rows.push(copy_place(
            entity.id,
            entity.x,
            entity.z,
            entity.level,
            &entity.actions,
        ));
    }
    (rows, false)
}

fn copy_place(
    id: i32,
    x: i32,
    z: i32,
    level: i32,
    actions: &[crate::observed::Text],
) -> SceneRowOut {
    SceneRowOut {
        id,
        x,
        z,
        level,
        actions: observed::strings(actions),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observed::{self, EntityRow, QuestStatusRow, QuestTab, SceneRow};
    use api::line_of_sight::CollisionQuery;
    use std::sync::Arc;

    fn open_collision() -> CollisionQuery {
        CollisionQuery {
            available: true,
            base_x: 3200,
            base_z: 3200,
            level: 0,
            width: 16,
            height: 16,
            flags: Arc::from([]),
        }
    }

    fn loc(id: i32, x: i32, z: i32, actions: &[&str]) -> SceneRow {
        SceneRow {
            id,
            name: None,
            x,
            z,
            level: 0,
            distance: 0,
            actions: observed::ops_of(
                &actions.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(),
            ),
        }
    }

    fn npc(id: i32, x: i32, z: i32, actions: &[&str]) -> EntityRow {
        EntityRow {
            id,
            x,
            z,
            actions: observed::ops_of(
                &actions.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(),
            ),
            ..EntityRow::default()
        }
    }

    fn quest(name: &str, status: &str) -> QuestStatusRow {
        QuestStatusRow {
            name: name.into(),
            status: status.into(),
            component_id: None,
        }
    }

    #[test]
    fn locs_keep_posted_order_filter_and_truncate() {
        observed::on_reset();
        observed::post(9, |post| {
            post.collision(open_collision());
            post.locs(vec![
                loc(1, 3200, 3200, &["Mine"]),
                loc(2092, 3201, 3200, &["Mine"]),
                loc(2092, 3202, 3200, &["Prospect"]),
                loc(3, 3203, 3200, &[]),
                loc(2092, 3204, 3200, &["Mine"]),
            ]);
        });
        let hit = scene_locs(&[2092], 2, None).expect("locs");
        assert_eq!(hit.as_of_sequence, 9);
        assert!(hit.scene.available);
        assert_eq!(hit.scene.base_x, 3200);
        assert_eq!(
            hit.rows
                .iter()
                .map(|row| (row.id, row.x, row.actions.clone()))
                .collect::<Vec<_>>(),
            vec![
                (2092, 3201, vec!["Mine".to_string()]),
                (2092, 3202, vec!["Prospect".to_string()]),
            ]
        );
        assert!(hit.truncated);
        let one = scene_locs(&[2092], 8, None).expect("all");
        assert_eq!(one.rows.len(), 3);
        assert!(!one.truncated);
    }

    #[test]
    fn locs_region_is_an_inclusive_box_on_one_level() {
        observed::on_reset();
        observed::post(1, |post| {
            post.collision(open_collision());
            post.locs(vec![
                loc(2092, 10, 10, &[]),
                loc(2092, 12, 10, &[]),
                SceneRow {
                    level: 1,
                    ..loc(2092, 11, 10, &[])
                },
            ]);
        });
        let hit = scene_locs(
            &[2092],
            8,
            Some(Region {
                min_x: 10,
                min_z: 10,
                max_x: 11,
                max_z: 10,
                level: 0,
            }),
        )
        .expect("region");
        assert_eq!(
            hit.rows.iter().map(|row| row.x).collect::<Vec<_>>(),
            vec![10]
        );
        assert!(!hit.truncated);
    }

    #[test]
    fn npcs_require_an_action_hit_and_keep_posted_order() {
        observed::on_reset();
        observed::post(4, |post| {
            post.collision(open_collision());
            post.npcs(vec![
                npc(-1, 1, 1, &["Talk-to"]),
                npc(-1, 2, 2, &["Attack", "Talk-to"]),
                npc(9, 3, 3, &["Attack"]),
                npc(-1, 4, 4, &["Attack"]),
            ]);
        });
        let hit = scene_npcs(&[-1], &["Attack".to_string()], 8, None).expect("npcs");
        assert_eq!(
            hit.rows.iter().map(|row| row.x).collect::<Vec<_>>(),
            vec![2, 4]
        );
        assert!(!hit.truncated);
        assert_eq!(
            scene_npcs(&[-1], &["Attack".to_string()], 1, None)
                .expect("cap")
                .truncated,
            true
        );
    }

    #[test]
    fn unavailable_collision_wins_over_empty_rows() {
        observed::on_reset();
        observed::post(1, |post| {
            post.locs(vec![]);
            post.npcs(vec![]);
        });
        assert_eq!(
            scene_locs(&[2092], 8, None).unwrap_err(),
            SceneQueryError::SnapshotUnavailable
        );
        observed::on_reset();
        observed::post(1, |post| {
            post.collision(CollisionQuery {
                available: false,
                ..open_collision()
            });
            post.locs(vec![loc(2092, 0, 0, &[])]);
        });
        assert_eq!(
            scene_locs(&[2092], 8, None).unwrap_err(),
            SceneQueryError::SnapshotUnavailable
        );
    }

    #[test]
    fn quest_status_folds_ascii_and_keeps_the_first_legal_row() {
        observed::on_reset();
        assert_eq!(
            quest_status("Death Plateau").unwrap_err(),
            QuestStatusError::SnapshotUnavailable
        );
        observed::post(3, |post| {
            post.quest_statuses(QuestTab::Unbound);
        });
        assert_eq!(
            quest_status("Death Plateau").unwrap_err(),
            QuestStatusError::QuestTabUnbound
        );
        observed::post(4, |post| {
            post.quest_statuses(QuestTab::Bound(vec![
                quest("skip", "bogus"),
                quest("  death plateau  ", "notStarted"),
                quest("Death Plateau", "complete"),
            ]));
        });
        let hit = quest_status("Death Plateau").expect("hit");
        assert_eq!(hit.status, "notStarted");
        assert_eq!(hit.as_of_sequence, 4);
        assert_eq!(
            quest_status("Missing").unwrap_err(),
            QuestStatusError::NotOnTab
        );
    }

    #[test]
    fn fold_ascii_is_not_unicode_lowercasing() {
        assert_eq!(fold_ascii("Death Plateau"), "DEATH PLATEAU");
        assert_eq!(fold_ascii("cook's assistant"), "COOK'S ASSISTANT");
        assert_eq!(fold_ascii("İ"), "İ");
    }
}
