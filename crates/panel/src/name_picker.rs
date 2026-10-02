//! Shared name-hit picker rows for the Loadouts search and the Debug name picker.
//!
//! Both popups list selected-content name hits (Loadouts [`ItemSearchHit`] rows
//! and Debug [`DebugName`] rows). One row implementation keeps them consistent:
//! the full display name is the primary text, `#id alias` is dimmed secondary
//! text pinned to the right, and the name column is sized from the longest
//! visible name — so a tight popup clips or scrolls the alias first and never
//! truncates the name before it. The picked row stays obvious: exactly one row
//! carries the `selected` highlight (the row matching the current field value,
//! else the first row).

use api::debug_commands::DebugName;
use api::game_data::ItemSearchHit;
use dear_imgui_rs::Ui;

/// Narrowest the picker popup may shrink to, in logical px. The old Debug
/// popup bottomed out at 180, which is what truncated every name column.
pub const PICKER_MIN_WIDTH: f32 = 360.0;
/// Widest the popup may grow to from content alone, in logical px. Wider than
/// this the rows scroll instead of pushing the popup out of the panel.
pub const PICKER_MAX_WIDTH: f32 = 560.0;
/// Fixed list height for the hit rows, in logical px.
pub const PICKER_LIST_HEIGHT: f32 = 200.0;
/// Gap between the name column and the right-aligned secondary text.
const NAME_GAP: f32 = 12.0;
/// Popup chrome allowance (frame padding plus the list scrollbar) subtracted
/// before comparing content against the popup width.
const ROW_PAD: f32 = 28.0;

/// One selected-content name hit, regardless of which search produced it.
pub trait NameHitRow {
    /// Full display name: primary text, never truncated before the alias.
    fn hit_name(&self) -> &str;
    /// Secondary identity text (`#id alias`): dimmed, pinned right.
    fn hit_secondary(&self) -> String;
    /// Current field value this row would write back when picked.
    fn hit_value(&self) -> &str;
}

impl NameHitRow for DebugName {
    fn hit_name(&self) -> &str {
        &self.name
    }

    fn hit_secondary(&self) -> String {
        if self.alias.is_empty() {
            format!("#{}", self.id)
        } else {
            format!("#{}  {}", self.id, self.alias)
        }
    }

    fn hit_value(&self) -> &str {
        &self.alias
    }
}

impl NameHitRow for ItemSearchHit {
    fn hit_name(&self) -> &str {
        &self.name
    }

    fn hit_secondary(&self) -> String {
        if self.alias.is_empty() {
            format!("#{}", self.id)
        } else {
            format!("#{}  {}", self.id, self.alias)
        }
    }

    fn hit_value(&self) -> &str {
        &self.name
    }
}

/// Width of `text` in physical px under the current font, for column math.
pub fn text_width(ui: &Ui, text: &str) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    ui.current_font()
        .calc_text_size(ui.current_font_size(), f32::MAX, 0.0, text)[0]
}
/// Name column width for the given rows: the longest visible name, so every
/// name fits before any alias text has to give way.
pub fn name_column_width(ui: &Ui, hits: &[impl NameHitRow]) -> f32 {
    hits.iter()
        .map(|hit| text_width(ui, hit.hit_name()))
        .fold(0.0, f32::max)
}

/// Index of the row to highlight: the row matching the field's current value,
/// else the first row so the highlight always marks something.
pub fn selected_row(hits: &[impl NameHitRow], current_value: &str) -> Option<usize> {
    if hits.is_empty() {
        return None;
    }
    Some(
        hits.iter()
            .position(|hit| hit.hit_value() == current_value)
            .unwrap_or(0),
    )
}

/// Popup width from measured content, clamped to stay inside the parent.
///
/// `parent_avail` is the containing panel width in the same (scaled) px as the
/// measurements; the popup never exceeds it. Content wider than
/// [`PICKER_MAX_WIDTH`] (or the parent) keeps the full name column and lets
/// the row overflow into a horizontal scroll instead of truncating the name.
pub fn popup_width(parent_avail: f32, max_name_w: f32, max_secondary_w: f32) -> f32 {
    popup_width_in(
        parent_avail,
        max_name_w,
        max_secondary_w,
        PICKER_MIN_WIDTH,
        PICKER_MAX_WIDTH,
        ROW_PAD,
        NAME_GAP,
    )
}
/// [`popup_width`] scaled for the current DPI: the font measurements are
/// physical px while the min/max/pad/gap constants are logical.
pub fn popup_width_for(ui: &Ui, parent_avail: f32, max_name_w: f32, max_secondary_w: f32) -> f32 {
    use crate::theme::scale_px;
    popup_width_in(
        parent_avail,
        max_name_w,
        max_secondary_w,
        scale_px(ui, PICKER_MIN_WIDTH),
        scale_px(ui, PICKER_MAX_WIDTH),
        scale_px(ui, ROW_PAD),
        scale_px(ui, NAME_GAP),
    )
}

/// Core of [`popup_width`] with explicit bounds, so tests can drive the math
/// without an ImGui context.
pub fn popup_width_in(
    parent_avail: f32,
    max_name_w: f32,
    max_secondary_w: f32,
    min_w: f32,
    max_w: f32,
    pad: f32,
    gap: f32,
) -> f32 {
    let parent = if parent_avail.is_finite() && parent_avail > 0.0 {
        parent_avail
    } else {
        max_w
    };
    (pad + max_name_w + gap + max_secondary_w)
        .clamp(min_w, max_w)
        .min(parent)
}
///
/// The selectable spans the full row (`content_w`, at least the child's
/// available width) so the click target and the `selected` highlight cover the
/// name and the secondary text alike; the dimmed secondary text is pinned to
/// the row's right edge. Returns the picked row index, if any.
pub fn draw_hit_rows(ui: &Ui, hits: &[impl NameHitRow], selected: Option<usize>) -> Option<usize> {
    if hits.is_empty() {
        return None;
    }
    let name_col_w = name_column_width(ui, hits);
    let max_secondary_w = hits
        .iter()
        .map(|hit| text_width(ui, &hit.hit_secondary()))
        .fold(0.0, f32::max);
    let avail = ui.content_region_avail()[0].max(1.0);
    let gap = crate::theme::scale_px(ui, NAME_GAP);
    let content_w = (name_col_w + gap + max_secondary_w).max(avail);
    let highlight = selected.unwrap_or(0).min(hits.len() - 1);

    let mut picked = None;
    for (index, hit) in hits.iter().enumerate() {
        let _id = ui.push_id(index);
        let row_x = ui.cursor_pos_x();
        if ui
            .selectable_config(hit.hit_name())
            .selected(index == highlight)
            .size([content_w, 0.0])
            .build()
        {
            picked = Some(index);
        }
        if ui.is_item_hovered() {
            ui.tooltip_text(format!("{}  {}", hit.hit_name(), hit.hit_secondary()));
        }
        ui.same_line();
        let secondary = hit.hit_secondary();
        let secondary_w = text_width(ui, &secondary);
        ui.set_cursor_pos_x((row_x + content_w - secondary_w).max(row_x));
        ui.text_disabled(secondary);
    }
    #[cfg(test)]
    record_layout(RecordedLayout {
        avail,
        content_w,
        name_col_w,
        highlight,
        rows: hits.len(),
    });
    picked
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RecordedLayout {
    pub(crate) avail: f32,
    pub(crate) content_w: f32,
    pub(crate) name_col_w: f32,
    pub(crate) highlight: usize,
    pub(crate) rows: usize,
}

#[cfg(test)]
static LAST_LAYOUT: std::sync::Mutex<Option<RecordedLayout>> = std::sync::Mutex::new(None);

#[cfg(test)]
fn record_layout(layout: RecordedLayout) {
    *crate::test_support::lock_unpoisoned(&LAST_LAYOUT) = Some(layout);
}

#[cfg(test)]
pub(crate) fn last_layout() -> RecordedLayout {
    crate::test_support::lock_unpoisoned(&LAST_LAYOUT).expect("draw_hit_rows records layout")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, alias: &str) -> DebugName {
        DebugName {
            id: 41,
            alias: alias.into(),
            name: name.into(),
        }
    }

    #[test]
    fn popup_width_fits_short_content_between_min_and_max() {
        // "Steel arrowtips" + "#41  steel_arrowtips" easily fits: the popup
        // hugs the content instead of collapsing to the old 180 px minimum.
        let width = popup_width(800.0, 120.0, 110.0);
        assert!(width >= ROW_PAD + 120.0 + NAME_GAP + 110.0);
        assert!((PICKER_MIN_WIDTH..=PICKER_MAX_WIDTH).contains(&width));
    }

    #[test]
    fn popup_width_never_exceeds_the_parent_panel() {
        // A narrow panel clamps the popup even below the content minimum, so
        // the popup stays inside the panel bounds.
        assert_eq!(popup_width(300.0, 120.0, 110.0), 300.0);
        assert_eq!(popup_width(300.0, 900.0, 400.0), 300.0);
    }

    /// Headless geometry proof for the operator screenshot: rows drawn from
    /// real hits keep a name column at least as wide as the longest visible
    /// name, stay inside the parent window, and highlight exactly one row.
    #[test]
    fn hit_rows_keep_full_names_inside_a_narrow_parent() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let hits = vec![
            row("Steel arrowtips", "steel_arrowtips"),
            row("Shortbow (unstrung)", "shortbow_unstrung_s"),
            row("Strength potion(4)", "strength_potion_4"),
        ];
        let mut expected = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        {
            let ui = ctx.frame();
            ui.window("dp-parent")
                .size([400.0, 500.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    let parent_w = ui.content_region_avail()[0];
                    // Independent measures: the test recomputes what the row
                    // drawer must honor, straight from the font.
                    let max_name = hits
                        .iter()
                        .map(|hit| text_width(ui, hit.hit_name()))
                        .fold(0.0, f32::max);
                    let gap = crate::theme::scale_px(ui, super::NAME_GAP);
                    let max_secondary = hits
                        .iter()
                        .map(|hit| text_width(ui, &hit.hit_secondary()))
                        .fold(0.0, f32::max);
                    expected = (parent_w, max_name, max_secondary, gap);
                    let width = popup_width(parent_w, max_name, max_secondary);
                    ui.child_window("##dp-hits")
                        .size([width, super::PICKER_LIST_HEIGHT])
                        .build(ui, || {
                            draw_hit_rows(ui, &hits, selected_row(&hits, ""));
                        });
                });
        }
        ctx.render();

        let (parent_w, max_name, max_secondary, gap) = expected;
        assert!(parent_w <= 400.0, "parent window honors its 400 px size");
        let layout = last_layout();
        assert_eq!(layout.rows, 3);
        // The picker list never leaves the 400 px parent window.
        assert!(
            layout.avail <= 400.0,
            "picker list width {} exceeds parent 400",
            layout.avail
        );
        // The drawn name column fits the longest visible name, measured
        // independently above: names are never truncated before the alias.
        assert_eq!(layout.name_col_w, max_name);
        assert!(max_name > 0.0);
        // The row spans name + gap + secondary (scrolling past the child
        // only when even the clamped popup cannot hold it all).
        assert_eq!(
            layout.content_w,
            (max_name + gap + max_secondary).max(layout.avail)
        );
        // Exactly the first row carries the highlight when nothing matches.
        assert_eq!(layout.highlight, 0);
    }

    /// `name_column_width` must return the max, not the mean or first.
    #[test]
    fn name_column_width_takes_the_maximum() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        ui.window("dp-measure")
            .size([800.0, 600.0], dear_imgui_rs::Condition::Always)
            .build(|| {
                let hits = vec![
                    row("Bow", "shortbow"),
                    row("Shortbow (unstrung)", "shortbow_u"),
                ];
                let column = name_column_width(ui, &hits);
                let short = text_width(ui, "Bow");
                let long = text_width(ui, "Shortbow (unstrung)");
                assert!(long > short);
                assert_eq!(column, long);
            });
        ctx.render();
    }
}
