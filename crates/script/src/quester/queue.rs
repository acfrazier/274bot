//! Index-ordered per-account quest queue and its compact status projection.
use super::pair::{AccountKey, Gang};
use super::registry::{PathBytes, PathRegistry, PathSource};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseIndex {
    pub schema: u16,
    pub paths: Vec<ReleasePath>,
}

/// One release roster row. A released row names its Path `file`. An
/// unavailable row names the quest and an end-user reason instead; it is never
/// compiled or started, and may keep a `file` so the authored Path stays
/// validated until the server can run it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleasePath {
    pub id: String,
    #[serde(default)]
    pub file: Option<String>,
    /// Display name of an unavailable row; released rows take it from the Path.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub unavailable: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct QueueSettings {
    /// Empty selects every row in the release index.
    pub quests: Vec<String>,
    /// Explicit priority rows; remaining picked rows retain release-index order.
    pub order_override: Vec<String>,
    pub skip: Vec<String>,
    pub partner_account: Option<AccountKey>,
    pub gang: Option<Gang>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum QueueStatus {
    Done,
    Running,
    Ready,
    Parked,
    Blocked,
    Unknown,
}

impl QueueStatus {
    pub const fn letter(self) -> char {
        match self {
            Self::Done => 'D',
            Self::Running => 'R',
            Self::Ready => 'Y',
            Self::Parked => 'P',
            Self::Blocked => 'B',
            Self::Unknown => '?',
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueueRow {
    /// Original release-index ordinal, independent of vector position.
    pub index: u8,
    pub picked: bool,
    pub skipped: bool,
    pub status: QueueStatus,
}

#[derive(Debug, Clone)]
pub struct Queue {
    index: PathRegistry,
    rows: Vec<QueueRow>,
    /// Positions in `rows`, with the operator override ahead of index-order tail.
    order: Vec<u8>,
    reasons: Vec<Option<Arc<str>>>,
    pub partner_account: Option<AccountKey>,
    pub gang: Option<Gang>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueConfigError {
    pub field: &'static str,
    pub message: Arc<str>,
}

impl QueueConfigError {
    fn new(field: &'static str, message: impl Into<Arc<str>>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for QueueConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for QueueConfigError {}

impl Queue {
    #[cfg(test)]
    pub fn from_index(
        index: &ReleaseIndex,
        settings: QueueSettings,
    ) -> Result<Self, QueueConfigError> {
        Self::from_registry(PathRegistry::from_index(index.clone()), settings)
    }

    pub fn from_registry(
        source: PathRegistry,
        settings: QueueSettings,
    ) -> Result<Self, QueueConfigError> {
        let index = source.index();
        validate_index(index)?;
        let positions = index
            .paths
            .iter()
            .enumerate()
            .map(|(position, path)| (path.id.as_str(), position))
            .collect::<std::collections::HashMap<_, _>>();
        let picked_ids = unique_setting_ids("quests", &settings.quests, &positions)?;
        let skipped_ids = unique_setting_ids("skip", &settings.skip, &positions)?;
        let _order_ids =
            unique_setting_ids("order_override", &settings.order_override, &positions)?;

        let pick_all = settings.quests.is_empty();
        for id in &settings.order_override {
            if !pick_all && !picked_ids.contains(id.as_str()) {
                return Err(QueueConfigError::new(
                    "order_override",
                    format!("quest {id:?} is not in the selected quests"),
                ));
            }
        }

        let mut rows = Vec::with_capacity(index.paths.len());
        let mut reasons = Vec::with_capacity(index.paths.len());
        for (ordinal, path) in index.paths.iter().enumerate() {
            let explicit = picked_ids.contains(path.id.as_str());
            let skipped = skipped_ids.contains(path.id.as_str());
            // An empty selection means every quest this server can run.
            let picked = explicit || (pick_all && path.unavailable.is_none());
            let status = if path.unavailable.is_some() {
                QueueStatus::Blocked
            } else if !picked || skipped {
                QueueStatus::Parked
            } else {
                QueueStatus::Unknown
            };
            rows.push(QueueRow {
                index: u8::try_from(ordinal).map_err(|_| {
                    QueueConfigError::new("", "release index contains more than 256 paths")
                })?,
                picked,
                skipped,
                status,
            });
            reasons.push(path.unavailable.as_deref().map(Arc::from));
        }

        if !pick_all
            && rows
                .iter()
                .all(|row| !row.picked || row.skipped || row.status == QueueStatus::Blocked)
        {
            let refused = index
                .paths
                .iter()
                .zip(&rows)
                .filter(|(_, row)| row.picked && !row.skipped)
                .filter_map(|(path, _)| {
                    Some(format!(
                        "{}: {}",
                        path.name.as_deref().unwrap_or(&path.id),
                        path.unavailable.as_deref()?
                    ))
                })
                .collect::<Vec<_>>();
            if !refused.is_empty() {
                return Err(QueueConfigError::new("quests", refused.join("; ")));
            }
        }

        let mut order = Vec::with_capacity(rows.len());
        for id in &settings.order_override {
            if let Some(&position) = positions.get(id.as_str()) {
                order.push(u8::try_from(position).map_err(|_| {
                    QueueConfigError::new("", "release index contains more than 256 paths")
                })?);
            }
        }
        for row in &rows {
            if row.picked && !row.skipped && !order.contains(&row.index) {
                order.push(row.index);
            }
        }

        Ok(Self {
            index: source,
            rows,
            order,
            reasons,
            partner_account: settings.partner_account,
            gang: settings.gang,
        })
    }

    /// Vector position (not the release-index ordinal) of the next row to
    /// evaluate or run. Unknown rows are candidates because activation compiles
    /// the Path lazily before its eligibility can be evaluated.
    pub fn next_candidate(&self) -> Option<usize> {
        self.order
            .iter()
            .map(|&position| usize::from(position))
            .find(|&position| {
                matches!(
                    self.rows[position].status,
                    QueueStatus::Unknown | QueueStatus::Ready
                )
            })
    }

    pub fn rows(&self) -> &[QueueRow] {
        &self.rows
    }

    pub fn id(&self, position: usize) -> Option<&str> {
        let row = self.rows.get(position)?;
        self.index
            .paths
            .get(usize::from(row.index))
            .map(|path| path.id.as_str())
    }

    pub fn file(&self, position: usize) -> Option<&str> {
        let row = self.rows.get(position)?;
        self.index
            .paths
            .get(usize::from(row.index))
            .and_then(|path| path.file.as_deref())
    }

    pub fn path_bytes(&self, position: usize) -> Option<PathBytes> {
        self.index.bytes(self.id(position)?)
    }

    pub fn path_source(&self, position: usize) -> PathSource {
        self.id(position)
            .map_or(PathSource::Bundled, |id| self.index.path_source(id))
    }

    pub fn path_report(&self) -> Option<&Arc<str>> {
        self.index.report()
    }

    pub fn status(&self, position: usize) -> Option<QueueStatus> {
        self.rows.get(position).map(|row| row.status)
    }

    pub fn reason(&self, position: usize) -> Option<&str> {
        let row = self.rows.get(position)?;
        self.reasons.get(position).and_then(Option::as_deref).or({
            if row.skipped {
                Some("skipped by settings")
            } else if !row.picked {
                Some("not selected")
            } else {
                None
            }
        })
    }

    pub fn mark_ready(&mut self, position: usize) {
        self.set_state(position, QueueStatus::Ready, None);
    }

    pub fn mark_running(&mut self, position: usize) {
        self.set_state(position, QueueStatus::Running, None);
    }

    pub fn mark_done(&mut self, position: usize) {
        self.set_state(position, QueueStatus::Done, None);
    }

    pub fn mark_blocked(&mut self, position: usize, reason: Arc<str>) {
        self.set_state(position, QueueStatus::Blocked, Some(reason));
    }

    pub fn mark_parked(&mut self, position: usize, reason: Arc<str>) {
        self.set_state(position, QueueStatus::Parked, Some(reason));
    }

    /// A changed quest/status/equipment observation may make an earlier
    /// eligibility blocker pass. Preserve its row but require a fresh check.
    /// Rows the release index marks unavailable stay blocked.
    pub fn refresh_blocked(&mut self) {
        for (position, row) in self.rows.iter_mut().enumerate() {
            if row.picked
                && !row.skipped
                && row.status == QueueStatus::Blocked
                && self.index.paths[usize::from(row.index)]
                    .unavailable
                    .is_none()
            {
                row.status = QueueStatus::Unknown;
                self.reasons[position] = None;
            }
        }
    }

    pub fn all_done(&self) -> bool {
        self.rows
            .iter()
            .filter(|row| row.picked && !row.skipped)
            .all(|row| row.status == QueueStatus::Done)
    }

    pub fn any_blocked(&self) -> bool {
        self.rows
            .iter()
            .any(|row| row.picked && !row.skipped && row.status == QueueStatus::Blocked)
    }

    pub fn status_text(&self) -> String {
        self.rows.iter().map(|row| row.status.letter()).collect()
    }

    fn set_state(&mut self, position: usize, status: QueueStatus, reason: Option<Arc<str>>) {
        if let (Some(row), Some(reason_slot)) =
            (self.rows.get_mut(position), self.reasons.get_mut(position))
        {
            row.status = status;
            *reason_slot = reason;
        }
    }
}

pub(super) fn validate_index(index: &ReleaseIndex) -> Result<(), QueueConfigError> {
    if index.schema != 1 {
        return Err(QueueConfigError::new(
            "",
            format!("unsupported release index schema {}", index.schema),
        ));
    }
    if index.paths.is_empty() {
        return Err(QueueConfigError::new("", "release index contains no paths"));
    }
    if index.paths.len() > usize::from(u8::MAX) + 1 {
        return Err(QueueConfigError::new(
            "",
            "release index contains more than 256 paths",
        ));
    }
    let mut ids = std::collections::HashSet::with_capacity(index.paths.len());
    let mut files = std::collections::HashSet::with_capacity(index.paths.len());
    for path in &index.paths {
        let shape_ok = match (&path.unavailable, &path.name) {
            (None, None) => path.file.is_some(),
            (Some(reason), Some(name)) => !reason.is_empty() && !name.is_empty(),
            _ => false,
        };
        if path.id.is_empty() || path.file.as_deref() == Some("") || !shape_ok {
            return Err(QueueConfigError::new(
                "",
                format!(
                    "release index row {:?} needs a file, or a name and an unavailable reason",
                    path.id
                ),
            ));
        }
        if !ids.insert(path.id.as_str()) {
            return Err(QueueConfigError::new(
                "",
                format!("duplicate release path id {:?}", path.id),
            ));
        }
        if let Some(file) = path.file.as_deref() {
            if !files.insert(file) {
                return Err(QueueConfigError::new(
                    "",
                    format!("duplicate release path file {file:?}"),
                ));
            }
        }
    }
    Ok(())
}

fn unique_setting_ids<'a>(
    field: &'static str,
    ids: &'a [String],
    positions: &std::collections::HashMap<&str, usize>,
) -> Result<std::collections::HashSet<&'a str>, QueueConfigError> {
    let mut unique = std::collections::HashSet::with_capacity(ids.len());
    for id in ids {
        if !positions.contains_key(id.as_str()) {
            return Err(QueueConfigError::new(
                field,
                format!("unknown quest id {id:?}"),
            ));
        }
        if !unique.insert(id.as_str()) {
            return Err(QueueConfigError::new(
                field,
                format!("duplicate quest id {id:?}"),
            ));
        }
    }
    Ok(unique)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> ReleaseIndex {
        serde_json::from_str(
            r#"{"schema":1,"paths":[{"id":"cook","file":"cook.json"},{"id":"sheep","file":"sheep.json"},{"id":"runemysteries","file":"runemysteries.json"},{"id":"romeojuliet","file":"romeojuliet.json"}]}"#,
        )
        .unwrap()
    }

    fn settings(quests: &[&str], order: &[&str], skip: &[&str]) -> QueueSettings {
        QueueSettings {
            quests: quests.iter().map(|id| (*id).to_string()).collect(),
            order_override: order.iter().map(|id| (*id).to_string()).collect(),
            skip: skip.iter().map(|id| (*id).to_string()).collect(),
            ..QueueSettings::default()
        }
    }

    #[test]
    fn empty_selection_uses_release_index_order_and_tracks_all_rows() {
        let index = index();
        let queue = Queue::from_index(&index, QueueSettings::default()).unwrap();
        assert_eq!(queue.next_candidate(), Some(0));
        assert_eq!(queue.id(0), Some("cook"));
        assert_eq!(queue.id(1), Some("sheep"));
        assert_eq!(queue.id(2), Some("runemysteries"));
        assert_eq!(queue.file(0), Some("cook.json"));
        assert_eq!(std::mem::size_of::<QueueRow>(), 4);
        assert_eq!(queue.status_text(), "????");
        assert!(!queue.all_done());
    }

    #[test]
    fn explicit_subset_does_not_run_unselected_paths_and_override_prioritizes_with_index_tail() {
        let index = index();
        let mut queue = Queue::from_index(
            &index,
            settings(
                &["cook", "sheep", "romeojuliet"],
                &["romeojuliet", "cook"],
                &[],
            ),
        )
        .unwrap();
        assert_eq!(queue.rows()[2].status, QueueStatus::Parked);
        assert_eq!(queue.next_candidate(), Some(3));
        queue.mark_blocked(3, Arc::from("needs a key"));
        assert_eq!(queue.next_candidate(), Some(0));
        queue.mark_ready(0);
        assert_eq!(queue.next_candidate(), Some(0));
        queue.mark_running(0);
        assert_eq!(queue.next_candidate(), Some(1));
        queue.mark_done(0);
        queue.mark_done(1);
        assert!(!queue.all_done());
        queue.mark_done(3);
        assert!(queue.all_done());
        assert!(!queue.any_blocked());
    }

    #[test]
    fn skip_is_parked_and_blocked_rows_can_be_refreshed() {
        let index = index();
        let mut queue = Queue::from_index(&index, settings(&[], &[], &["cook"])).unwrap();
        assert_eq!(queue.status(0), Some(QueueStatus::Parked));
        assert_eq!(queue.reason(0), Some("skipped by settings"));
        assert_eq!(queue.next_candidate(), Some(1));
        queue.mark_blocked(1, Arc::from("need 20 quest points"));
        assert!(queue.any_blocked());
        assert_eq!(queue.reason(1), Some("need 20 quest points"));
        queue.refresh_blocked();
        assert_eq!(queue.status(1), Some(QueueStatus::Unknown));
        assert_eq!(queue.reason(1), None);
        assert_eq!(queue.next_candidate(), Some(1));
    }

    #[test]
    fn order_and_selection_reject_unknown_or_repeated_ids() {
        let index = index();
        let error = Queue::from_index(&index, settings(&["cook", "cook"], &[], &[])).unwrap_err();
        assert_eq!(error.field, "quests");
        let error = Queue::from_index(&index, settings(&["cook"], &["sheep"], &[])).unwrap_err();
        assert_eq!(error.field, "order_override");
        let error = Queue::from_index(&index, settings(&[], &[], &["missing"])).unwrap_err();
        assert_eq!(error.field, "skip");
        let error = Queue::from_index(&index, settings(&["missing"], &[], &[])).unwrap_err();
        assert_eq!(error.field, "quests");
        assert!(error.message.contains("unknown quest id \"missing\""));
    }

    #[test]
    fn status_projection_is_release_index_order_not_override_order() {
        let index = index();
        let mut queue =
            Queue::from_index(&index, settings(&[], &["romeojuliet", "cook"], &[])).unwrap();
        queue.mark_done(0);
        queue.mark_blocked(1, Arc::from("blocked"));
        queue.mark_parked(3, Arc::from("operator"));
        assert_eq!(queue.status_text(), "DB?P");
    }

    #[test]
    fn partner_account_and_gang_are_carried_in_queue_settings() {
        let index = index();
        let queue = Queue::from_index(
            &index,
            QueueSettings {
                partner_account: Some(AccountKey(Arc::from("secondary"))),
                gang: Some(Gang::BlackArm),
                ..QueueSettings::default()
            },
        )
        .unwrap();
        assert_eq!(
            queue.partner_account.as_ref().unwrap().0.as_ref(),
            "secondary"
        );
        assert_eq!(queue.gang, Some(Gang::BlackArm));
    }

    fn index_with_unavailable() -> ReleaseIndex {
        serde_json::from_str(
            r#"{"schema":1,"paths":[
                {"id":"cook","file":"cook.json"},
                {"id":"fortress","file":"fortress.json","name":"Black Knights' Fortress","unavailable":"Can't be completed on this server: the grill can't be reached"},
                {"id":"hauntedmine","name":"Haunted Mine","unavailable":"Not on this server"},
                {"id":"sheep","file":"sheep.json"}
            ]}"#,
        )
        .unwrap()
    }

    #[test]
    fn unavailable_rows_stay_blocked_with_their_reason_and_never_become_candidates() {
        let index = index_with_unavailable();
        let mut queue = Queue::from_index(&index, QueueSettings::default()).unwrap();
        assert_eq!(queue.status_text(), "?BB?");
        assert_eq!(
            queue.reason(1),
            Some("Can't be completed on this server: the grill can't be reached")
        );
        assert_eq!(queue.reason(2), Some("Not on this server"));
        assert_eq!(queue.file(2), None);
        assert!(
            !queue.rows()[1].picked,
            "select-all leaves unavailable rows out"
        );
        assert_eq!(queue.next_candidate(), Some(0));
        queue.mark_done(0);
        assert_eq!(queue.next_candidate(), Some(3));
        queue.mark_done(3);
        assert!(
            queue.all_done(),
            "select-all completes without unavailable rows"
        );
        assert!(!queue.any_blocked());

        let mut picked =
            Queue::from_index(&index, settings(&["fortress", "cook"], &["fortress"], &[])).unwrap();
        assert_eq!(picked.next_candidate(), Some(0));
        picked.refresh_blocked();
        assert_eq!(picked.status(1), Some(QueueStatus::Blocked));
        assert!(picked.reason(1).unwrap().contains("grill"));
        picked.mark_done(0);
        assert_eq!(picked.next_candidate(), None);
        assert!(
            picked.any_blocked(),
            "an explicit unavailable pick is reported"
        );
    }

    #[test]
    fn explicit_pick_of_only_unavailable_quests_is_refused_with_the_reason() {
        let index = index_with_unavailable();
        let error = Queue::from_index(&index, settings(&["fortress", "hauntedmine"], &[], &[]))
            .unwrap_err();
        assert_eq!(error.field, "quests");
        assert_eq!(
            error.message.as_ref(),
            "Black Knights' Fortress: Can't be completed on this server: the grill can't be reached; Haunted Mine: Not on this server"
        );
        let error =
            Queue::from_index(&index, settings(&["fortress", "cook"], &[], &["cook"])).unwrap_err();
        assert!(error.message.starts_with("Black Knights' Fortress: "));
        Queue::from_index(&index, settings(&[], &[], &["fortress"])).unwrap();
    }

    #[test]
    fn release_rows_need_a_file_or_a_name_and_reason() {
        for rejected in [
            r#"{"schema":1,"paths":[{"id":"cook"}]}"#,
            r#"{"schema":1,"paths":[{"id":"cook","file":"cook.json","name":"Cook"}]}"#,
            r#"{"schema":1,"paths":[{"id":"cook","unavailable":"why"}]}"#,
            r#"{"schema":1,"paths":[{"id":"cook","name":"Cook","unavailable":""}]}"#,
            r#"{"schema":1,"paths":[{"id":"cook","file":""}]}"#,
        ] {
            let index: ReleaseIndex = serde_json::from_str(rejected).unwrap();
            assert!(
                Queue::from_index(&index, QueueSettings::default()).is_err(),
                "accepted {rejected}"
            );
        }
    }
}
