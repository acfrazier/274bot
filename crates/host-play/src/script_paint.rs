use crate::SlotStatus;

use super::{script_slot, ScriptWall};
/// `name`'s slot script's latest paint frame, `None` for a slot with no
/// script or a script that has not painted. Copied onto the status row
/// each observe so the TUI can show paint-as-chat without a probe
/// round-trip (the isolate forwards the frame after each tick).
pub(crate) fn script_paint_of(
    scripts: &ScriptWall,
    name: &str,
) -> Option<std::sync::Arc<script::shim::ScriptPaint>> {
    script_slot(scripts, name).and_then(|s| s.lock().unwrap().paint())
}

/// Publish `paint` onto `status.script_paint`, sharing the frame the isolate
/// recorded. The isolate forwards only changed frames, so a different handle
/// is a new frame; an equal one is skipped.
pub(crate) fn publish_script_paint(
    status: &mut SlotStatus,
    paint: Option<std::sync::Arc<script::shim::ScriptPaint>>,
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
    paint: &script::shim::ScriptPaint,
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
