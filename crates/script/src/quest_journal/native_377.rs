//! Selected 377 journal facts, isolated from the production 289 layout.
//! Sources: canon content/pack/interface.pack:7219-7230,8020-8081,12060-12109;
//! scripts/player/scripts/quest_journal.rs2:1-105. Java 377 Client.java:
//! 9890-9893 (IF_BUTTON), 8540-8549 (IF_SETTEXT), 7973-8008 (IF_OPENMAIN),
//! 4567-4595 (CLOSE_MODAL). The existing host transaction owns those actions.
use super::{failure, ActionError};
use api::quest_facts::QuestFacts;
use api::selected::{ClientRevision, SelectedPin};
use api::snapshot::{SnapshotView, WidgetRoot};
use std::sync::Arc;

pub(super) const ROOT: i32 = 8134;
pub(super) const TITLE: i32 = 8144;

fn unavailable(reason: &str) -> ActionError {
    ActionError::Unavailable(Arc::from(reason))
}

pub(super) fn admit(pin: &SelectedPin, quest: &QuestFacts) -> Result<i32, ActionError> {
    let selected = api::game_data::for_revision(ClientRevision::R377)
        .map_err(|_| unavailable("selected 377 journal game data unavailable"))?;
    let expected = selected
        .selected_pin()
        .map_err(|_| unavailable("selected 377 journal pin unavailable"))?;
    if expected.as_ref() != pin {
        return Err(unavailable("selected 377 journal pin mismatch"));
    }
    let row = selected
        .quest_identity()
        .and_then(|facts| facts.rows.iter().find(|row| row.id == quest.id.0.as_ref()))
        .ok_or_else(|| unavailable("selected 377 journal quest identity unavailable"))?;
    if row.display != quest.display.as_ref()
        || row.journal_title.as_deref() != quest.journal_title.as_deref()
        || row.component != quest.component_name.as_ref()
    {
        return Err(unavailable("selected 377 journal quest identity mismatch"));
    }
    // Only source-mapped buttons are admitted. A shared root is not evidence
    // for an unqualified quest button, title or selected table.
    match row.component.as_str() {
        "cook" => Ok(7333),
        "runemysteries" => Ok(7335),
        "romeojuliet" => Ok(7343),
        "sheep" => Ok(7344),
        _ => Err(unavailable(
            "selected 377 journal quest button table unavailable",
        )),
    }
}

/// `split_get(0, 0..99)` order. com_5 is a static control, not journal text.
/// The cache assigns com_55 onwards a second, discontiguous ID range.
fn body_component(index: usize) -> i32 {
    match index {
        0 => 8145,
        1..=49 => 8146 + index as i32,
        _ => 12124 + index as i32,
    }
}

pub(super) fn body<'a>(snapshot: SnapshotView<'a>) -> Result<[&'a str; 100], ActionError> {
    let widgets = snapshot
        .widgets()
        .ok_or_else(|| failure("selected 377 journal widgets unavailable"))?;
    let mut lines = [""; 100];
    let mut seen = [false; 100];
    for widget in widgets.value {
        if widget.root != WidgetRoot::Main || widget.root_component_id != ROOT || widget.hidden {
            continue;
        }
        let index = match widget.component_id {
            8145 => 0,
            8147..=8195 => (widget.component_id - 8146) as usize,
            12174..=12223 => (widget.component_id - 12124) as usize,
            _ => continue,
        };
        if seen[index] {
            return Err(failure("selected 377 journal body component ambiguous"));
        }
        if widget.type_ != 4 {
            return Err(failure("selected 377 journal body is not a text component"));
        }
        seen[index] = true;
        // Snapshot decode.rs's non_empty maps a real empty text to None.
        // The observed type-4 component still proves the empty fragment.
        lines[index] = widget.text.as_deref().unwrap_or("");
    }
    if let Some(index) = seen.iter().position(|present| !present) {
        return Err(ActionError::Failed(Arc::from(format!(
            "selected 377 journal body component {} unavailable",
            body_component(index)
        ))));
    }
    Ok(lines)
}
