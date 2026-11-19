use std::{collections::HashMap, sync::Mutex};

use crate::SlotStatus;

use super::{script_slot, NavBot, ScriptWall};

/// `name`'s slot script's latest paint frame, `None` for a slot with no
/// script or a script that has not painted. Copied onto the status row
/// each observe so the TUI can show paint-as-chat without a probe
/// round-trip (the isolate forwards the frame after each tick).
pub(crate) fn script_paint_of(
    scripts: &ScriptWall,
    name: &str,
) -> Option<std::sync::Arc<script::paint::ScriptPaint>> {
    script_slot(scripts, name).and_then(|slot| slot.lock().ok().and_then(|slot| slot.paint()))
}

/// The active native script's typed survival report, overlaid with a current
/// walk-guard action when the script defers survival to navigation.
pub(crate) fn script_survival_of(
    scripts: &ScriptWall,
    navs: &Mutex<HashMap<String, NavBot>>,
    name: &str,
) -> script::survival::SurvivalReport {
    let current = script_slot(scripts, name)
        .and_then(|slot| slot.lock().ok().and_then(|slot| slot.native_status()))
        .map_or_else(script::survival::SurvivalReport::default, |status| {
            status.survival
        });
    let action = navs.lock().ok().and_then(|navs| {
        navs.get(name)
            .and_then(|bot| bot.walk_guard.as_ref())
            .and_then(|guard| guard.survival_report())
    });
    merge_survival_action(current, action)
}

fn merge_survival_action(
    current: script::survival::SurvivalReport,
    action: Option<script::survival::SurvivalReport>,
) -> script::survival::SurvivalReport {
    current.with_action(action)
}

/// Copy the latest runner report unless this frame starts a new session.
pub(crate) fn publish_script_survival(
    status: &mut SlotStatus,
    survival: script::survival::SurvivalReport,
    session_boundary: bool,
) {
    status.survival = if session_boundary {
        script::survival::SurvivalReport::default()
    } else {
        survival
    };
}

/// Publish `paint` onto `status.script_paint`, sharing the frame the isolate
/// recorded. The isolate forwards only changed frames, so a different handle
/// is a new frame; an equal one is skipped.
pub(crate) fn publish_script_paint(
    status: &mut SlotStatus,
    paint: Option<std::sync::Arc<script::paint::ScriptPaint>>,
) {
    match (&status.script_paint, &paint) {
        (Some(cur), Some(next)) if std::sync::Arc::ptr_eq(cur, next) => {}
        (None, None) => {}
        _ => status.script_paint = paint,
    }
}

/// Whether `paint` advertises `select_name` on the persistent chrome store
/// `key` (`strip:{id}`, `rail:{id}`, or `tabs:{id}`).
pub(crate) fn script_paint_select_advertised(
    paint: &script::paint::ScriptPaint,
    key: &str,
    select_name: &str,
) -> bool {
    if select_name.is_empty() {
        return false;
    }
    if let Some(id) = key.strip_prefix("strip:") {
        return paint
            .strip
            .as_ref()
            .is_some_and(|band| band.id == id && band.names.iter().any(|n| n == select_name));
    }
    if let Some(id) = key.strip_prefix("rail:") {
        return paint
            .rail
            .as_ref()
            .is_some_and(|band| band.id == id && band.names.iter().any(|n| n == select_name));
    }
    if let Some(id) = key.strip_prefix("tabs:") {
        return paint
            .tabs
            .iter()
            .any(|band| band.id == id && band.names.iter().any(|n| n == select_name));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn survival_status_copies_reports_and_clears_them_at_session_boundary() {
        let report = script::survival::SurvivalReport {
            decision: script::survival::Decision::Needed,
            why: None,
            need: Some(script::survival::math::SurvivalNeed::Food {
                count: 3,
                ref_heal: 12,
            }),
            tried: 0,
            owner: script::survival::Owner::Step,
        };
        let mut status = SlotStatus::default();
        publish_script_survival(&mut status, report, false);
        assert_eq!(status.survival, report);

        publish_script_survival(&mut status, report, true);
        assert_eq!(status.survival, script::survival::SurvivalReport::default());
    }
    #[test]
    fn survival_action_replaces_deferred_need() {
        use script::survival::{
            cure::CureWhy,
            math::{NeedWhy, SurvivalNeed},
            Decision, Owner, SurvivalReport,
        };

        let needed = SurvivalReport {
            decision: Decision::Needed,
            why: Some(CureWhy::Lethal),
            need: Some(SurvivalNeed::Cure {
                or_food: 0,
                ref_heal: 12,
                why: NeedWhy::Lethal,
            }),
            tried: 1,
            owner: Owner::Step,
        };
        let action = SurvivalReport {
            decision: Decision::Drinking,
            owner: Owner::Guard,
            ..SurvivalReport::default()
        };

        assert_eq!(
            merge_survival_action(needed, Some(action)),
            SurvivalReport {
                decision: Decision::Drinking,
                owner: Owner::Guard,
                ..needed
            }
        );
    }
}
