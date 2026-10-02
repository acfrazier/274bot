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
use dear_imgui_rs::{ListClipper, Ui};

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
fn text_width(ui: &Ui, text: &str) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    ui.current_font()
        .calc_text_size(ui.current_font_size(), f32::MAX, 0.0, text)[0]
}

#[derive(Debug, Clone, Default)]
struct FormattedRow {
    secondary: String,
    secondary_width: f32,
}

/// Formatted picker strings and font measurements for one immutable result set.
///
/// Keep one cache with each picker and call [`invalidate`](Self::invalidate)
/// whenever its query or source data changes.
#[derive(Debug, Clone, Default)]
pub struct PickerRows {
    rows: Vec<FormattedRow>,
    max_name_width: f32,
    max_secondary_width: f32,
    metric_key: Option<(f32, f32)>,
    content_width_key: Option<(f32, f32, f32)>,
    content_width: f32,
    dirty: bool,
}

impl PickerRows {
    /// Discard query/result-specific formatting while retaining vector capacity.
    pub fn invalidate(&mut self) {
        self.dirty = true;
        self.content_width_key = None;
    }

    /// Make sure this result set has formatted rows and metrics for the current
    /// font. Callers invalidate when the query or source data changes.
    pub(crate) fn ensure(&mut self, ui: &Ui, hits: &[impl NameHitRow]) {
        if self.dirty || self.rows.len() != hits.len() {
            self.rows.truncate(hits.len());
            self.rows
                .reserve(hits.len().saturating_sub(self.rows.len()));
            for (index, hit) in hits.iter().enumerate() {
                if index == self.rows.len() {
                    self.rows.push(FormattedRow::default());
                }
                self.rows[index].secondary = hit.hit_secondary();
            }
            self.dirty = false;
            self.content_width_key = None;
            self.measure(ui, hits);
            return;
        }

        let key = (crate::theme::ui_scale(ui), ui.current_font_size());
        if self.metric_key != Some(key) {
            self.content_width_key = None;
            self.measure(ui, hits);
        }
    }

    fn measure(&mut self, ui: &Ui, hits: &[impl NameHitRow]) {
        self.max_name_width = 0.0;
        self.max_secondary_width = 0.0;
        for (row, hit) in self.rows.iter_mut().zip(hits) {
            self.max_name_width = self.max_name_width.max(text_width(ui, hit.hit_name()));
            row.secondary_width = text_width(ui, &row.secondary);
            self.max_secondary_width = self.max_secondary_width.max(row.secondary_width);
        }
        self.metric_key = Some((crate::theme::ui_scale(ui), ui.current_font_size()));
    }

    /// Content-fit width is cached for this result set, font and parent width.
    pub(crate) fn content_width_for(&mut self, ui: &Ui, parent_width: f32) -> f32 {
        let key = (
            parent_width,
            crate::theme::ui_scale(ui),
            ui.current_font_size(),
        );
        if self.content_width_key != Some(key) {
            self.content_width = popup_width_for(
                ui,
                parent_width,
                self.max_name_width,
                self.max_secondary_width,
            );
            self.content_width_key = Some(key);
        }
        self.content_width
    }

    fn row(&self, index: usize) -> &FormattedRow {
        &self.rows[index]
    }
}

/// Screen-space work area of the current parent window, excluding its chrome.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PopupWorkArea {
    pub(crate) min: [f32; 2],
    pub(crate) max: [f32; 2],
}

impl PopupWorkArea {
    pub(crate) fn current(ui: &Ui) -> Self {
        ui.with_bound_context(|| {
            use dear_imgui_rs::sys;

            // SAFETY: `with_bound_context` installs this `Ui`'s context in
            // `GImGui`, and the `Ui` is in its active frame.
            let window = unsafe { sys::igGetCurrentWindowRead() };
            if window.is_null() {
                return Self {
                    min: [0.0; 2],
                    max: [0.0; 2],
                };
            }
            // SAFETY: `window` was just returned as this context's current
            // window; copy its POD rectangles immediately without retaining it.
            let (work, clip) = unsafe { ((*window).WorkRect, (*window).InnerClipRect) };
            Self {
                min: [work.Min.x.max(clip.Min.x), work.Min.y.max(clip.Min.y)],
                max: [work.Max.x.min(clip.Max.x), work.Max.y.min(clip.Max.y)],
            }
        })
    }

    fn extent(self, axis: usize) -> Option<f32> {
        let extent = self.max[axis] - self.min[axis];
        (self.min[axis].is_finite()
            && self.max[axis].is_finite()
            && extent.is_finite()
            && extent > 0.0)
            .then_some(extent)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PopupLayout {
    pub(crate) content_width: f32,
    pub(crate) list_height: f32,
    position: [f32; 2],
    desired_window_width: f32,
    constrained_height: f32,
}

/// Compute popup geometry for the parent's work area. The resulting geometry
/// is applied by `popup`, which immediately begins the popup.
pub(crate) fn prepare_popup(
    ui: &Ui,
    work_area: PopupWorkArea,
    anchor: [[f32; 2]; 2],
    hits: &[impl NameHitRow],
    rows: &mut PickerRows,
) -> PopupLayout {
    // SAFETY: the frame owns a live style here; copy its scalar geometry
    // immediately rather than retaining the reference.
    let (padding, border, spacing_y) = unsafe {
        let style = ui.style();
        (
            style.window_padding(),
            style.popup_border_size(),
            style.item_spacing()[1],
        )
    };
    let chrome = [2.0 * (padding[0] + border), 2.0 * (padding[1] + border)];
    let work_width = work_area.extent(0);
    let work_height = work_area.extent(1);
    let content_width_limit = work_width.map_or(0.0, |width| (width - chrome[0]).max(0.0));
    rows.ensure(ui, hits);
    let requested_width = rows.content_width_for(ui, content_width_limit);
    let content_width = if work_width.is_some() {
        requested_width.min(content_width_limit)
    } else {
        // Without a usable parent width, use the picker minimum, not its max.
        requested_width.min(crate::theme::scale_px(ui, PICKER_MIN_WIDTH))
    };

    let fixed_content_height =
        2.0 * ui.text_line_height() + 2.0 * ui.frame_height() + 5.0 * spacing_y;
    let requested_list_height = crate::theme::scale_px(ui, PICKER_LIST_HEIGHT);
    let list_height = work_height.map_or(requested_list_height, |height| {
        requested_list_height.min((height - chrome[1] - fixed_content_height).max(0.0))
    });

    let max_window_width = work_width.unwrap_or(content_width + chrome[0]).max(1.0);
    let max_window_height = work_height
        .unwrap_or(fixed_content_height + list_height + chrome[1])
        .max(1.0);
    let desired_window_width = (content_width + chrome[0]).min(max_window_width);
    let desired_window_height =
        (fixed_content_height + list_height + chrome[1]).min(max_window_height);

    let fallback_position = [
        if work_area.min[0].is_finite() {
            work_area.min[0]
        } else {
            0.0
        },
        if work_area.min[1].is_finite() {
            work_area.min[1]
        } else {
            0.0
        },
    ];
    let anchor_min_x = if anchor[0][0].is_finite() {
        anchor[0][0]
    } else {
        fallback_position[0]
    };
    let anchor_min_y = if anchor[0][1].is_finite() {
        anchor[0][1]
    } else {
        fallback_position[1]
    };
    let anchor_max_y = if anchor[1][1].is_finite() {
        anchor[1][1]
    } else {
        anchor_min_y
    };
    let position_x = work_width.map_or(anchor_min_x, |_| {
        let max_x = work_area.max[0] - desired_window_width;
        if max_x >= work_area.min[0] {
            anchor_min_x.clamp(work_area.min[0], max_x)
        } else {
            work_area.min[0]
        }
    });
    let position_x = if position_x.is_finite() {
        position_x
    } else {
        fallback_position[0]
    };
    let position_y = work_height.map_or(anchor_max_y, |_| {
        let below = anchor_max_y;
        let above = anchor_min_y - desired_window_height;
        if below + desired_window_height <= work_area.max[1] {
            below.max(work_area.min[1])
        } else if above >= work_area.min[1] {
            above
        } else {
            work_area.min[1]
        }
    });
    let position_y = if position_y.is_finite() {
        position_y
    } else {
        fallback_position[1]
    };
    let constrained_height = work_height.map_or(max_window_height, |height| {
        (work_area.max[1] - position_y).max(0.0).min(height)
    });

    PopupLayout {
        content_width,
        list_height,
        position: [position_x, position_y],
        desired_window_width,
        constrained_height,
    }
}

/// Apply prepared geometry and begin the popup as one operation so its
/// next-window data cannot leak to a later window.
pub(crate) fn popup<State>(
    ui: &Ui,
    id: &str,
    state: &mut State,
    prepare: impl FnOnce(&mut State) -> PopupLayout,
    draw: impl FnOnce(&mut State, PopupLayout),
) -> PopupLayout {
    let layout = prepare(state);
    use dear_imgui_rs::sys;
    // SAFETY: `with_bound_context` installs this `Ui`'s context in `GImGui`.
    // The position is finite and the computed constraints are finite and
    // non-negative. `ui.popup` immediately follows and calls BeginPopup on
    // this same `Ui`, consuming this context's next-window data on every path.
    ui.with_bound_context(|| unsafe {
        sys::igSetNextWindowPos(
            sys::ImVec2_c {
                x: layout.position[0],
                y: layout.position[1],
            },
            sys::ImGuiCond_Always,
            sys::ImVec2_c { x: 0.0, y: 0.0 },
        );
        sys::igSetNextWindowSizeConstraints(
            sys::ImVec2_c { x: 0.0, y: 0.0 },
            sys::ImVec2_c {
                x: layout.desired_window_width,
                y: layout.constrained_height,
            },
            None,
            std::ptr::null_mut(),
        );
    });
    ui.popup(id, || draw(state, layout));
    layout
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

/// Popup content width from measured rows, clamped to the available parent
/// work area after popup padding and border are removed.
///
/// Font measurements and the min/max/pad/gap constants use scaled physical px.
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

/// Core width calculation with explicit bounds, usable without an ImGui context.
pub fn popup_width_in(
    parent_avail: f32,
    max_name_w: f32,
    max_secondary_w: f32,
    min_w: f32,
    max_w: f32,
    pad: f32,
    gap: f32,
) -> f32 {
    let fallback = min_w.min(max_w);
    let parent = if parent_avail.is_finite() && parent_avail > 0.0 {
        parent_avail
    } else {
        fallback
    };
    (pad + max_name_w + gap + max_secondary_w)
        .clamp(min_w, max_w)
        .min(parent)
}
/// The selectable spans the full row (`content_w`, at least the child's
/// available width) so its click target and highlight cover the whole row.
/// Cached strings and measurements are rebuilt only for a changed result set.
pub fn draw_hit_rows(
    ui: &Ui,
    hits: &[impl NameHitRow],
    rows: &mut PickerRows,
    selected: Option<usize>,
) -> Option<usize> {
    rows.ensure(ui, hits);
    if hits.is_empty() {
        return None;
    }
    let avail = ui.content_region_avail()[0].max(1.0);
    let gap = crate::theme::scale_px(ui, NAME_GAP);
    let content_w = (rows.max_name_width + gap + rows.max_secondary_width).max(avail);
    let highlight = selected.unwrap_or(0).min(hits.len() - 1);

    let mut picked = None;
    for index in ListClipper::new(hits.len()).begin(ui).iter() {
        let hit = &hits[index];
        let cached = rows.row(index);
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
            ui.tooltip(|| {
                ui.text(hit.hit_name());
                ui.same_line();
                ui.text(&cached.secondary);
            });
        }
        ui.same_line();
        ui.set_cursor_pos_x((row_x + content_w - cached.secondary_width).max(row_x));
        ui.text_disabled(&cached.secondary);
    }
    picked
}

#[cfg(test)]
mod tests {
    use super::*;
    use dear_imgui_rs::{Condition, FramePrepareOptions, WindowFlags};
    use std::cell::Cell;
    use std::rc::Rc;

    struct CountingHit {
        name: String,
        alias: String,
        secondary_calls: Rc<Cell<usize>>,
    }

    impl NameHitRow for CountingHit {
        fn hit_name(&self) -> &str {
            &self.name
        }

        fn hit_secondary(&self) -> String {
            self.secondary_calls.set(self.secondary_calls.get() + 1);
            format!("#1  {}", self.alias)
        }

        fn hit_value(&self) -> &str {
            &self.name
        }
    }

    fn row(name: &str, alias: &str) -> DebugName {
        DebugName {
            id: 41,
            alias: alias.into(),
            name: name.into(),
        }
    }

    fn draw_popup_frame(
        ctx: &mut dear_imgui_rs::Context,
        display_size: [f32; 2],
        parent_size: [f32; 2],
        hits: &[DebugName],
        cached_rows: &mut PickerRows,
    ) -> (PopupWorkArea, [f32; 2], [f32; 2], PopupLayout) {
        ctx.prepare_frame(
            FramePrepareOptions::new(display_size, 1.0 / 60.0).renderer_has_textures(),
        );
        let mut actual = None;
        let mut work_area = None;
        let mut layout = None;
        {
            let ui = ctx.frame();
            ui.window("dp-parent")
                .flags(WindowFlags::NO_TITLE_BAR | WindowFlags::NO_SAVED_SETTINGS)
                .position([20.0, 20.0], Condition::Always)
                .size(parent_size, Condition::Always)
                .build(|| {
                    let area = PopupWorkArea::current(ui);
                    work_area = Some(area);
                    ui.set_cursor_screen_pos([area.max[0] - 48.0, area.min[1] + 40.0]);
                    ui.button("Pick");
                    let anchor = [ui.item_rect_min(), ui.item_rect_max()];
                    ui.open_popup("##debug-name-picker");
                    let next = popup(
                        ui,
                        "##debug-name-picker",
                        cached_rows,
                        |rows| prepare_popup(ui, area, anchor, hits, rows),
                        |rows, next| {
                            ui.text("Pick object");
                            ui.set_next_item_width(next.content_width);
                            let mut query = String::new();
                            ui.input_text("##headless-picker-query", &mut query).build();
                            ui.child_window("##headless-picker-rows")
                                .size([next.content_width, next.list_height])
                                .build(ui, || {
                                    draw_hit_rows(ui, hits, rows, None);
                                });
                            ui.button("Close##headless-picker-close");
                            actual = Some((ui.window_pos(), ui.window_size()));
                        },
                    );
                    layout = Some(next);
                });
        }
        ctx.render();
        let area = work_area.expect("parent window work area");
        let (pos, size) = actual.expect("popup opened");
        (area, pos, size, layout.expect("popup layout"))
    }
    fn draw_synthetic_popup_frame(
        ctx: &mut dear_imgui_rs::Context,
        id: &str,
        work_area: PopupWorkArea,
        anchor: [[f32; 2]; 2],
        rows: &mut PickerRows,
    ) -> (PopupLayout, ([f32; 2], [f32; 2])) {
        ctx.prepare_frame(
            FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
        );
        let hits = [row("Bow", "shortbow")];
        let mut actual = None;
        let mut layout = None;
        {
            let ui = ctx.frame();
            ui.window("dp-synthetic-popup-parent")
                .flags(WindowFlags::NO_TITLE_BAR | WindowFlags::NO_SAVED_SETTINGS)
                .position([20.0, 20.0], Condition::Always)
                .size([400.0, 300.0], Condition::Always)
                .build(|| {
                    ui.open_popup(id);
                    let next = popup(
                        ui,
                        id,
                        rows,
                        |rows| prepare_popup(ui, work_area, anchor, &hits, rows),
                        |_, layout| {
                            ui.text("Popup geometry");
                            ui.set_next_item_width(layout.content_width);
                            actual = Some((ui.window_pos(), ui.window_size()));
                            ui.close_current_popup();
                        },
                    );
                    layout = Some(next);
                });
        }
        ctx.render();
        (layout.expect("popup layout"), actual.expect("popup opened"))
    }

    fn assert_rect_inside(area: PopupWorkArea, pos: [f32; 2], size: [f32; 2]) {
        const EPSILON: f32 = 0.05;
        assert!(
            pos[0] >= area.min[0] - EPSILON
                && pos[1] >= area.min[1] - EPSILON
                && pos[0] + size[0] <= area.max[0] + EPSILON
                && pos[1] + size[1] <= area.max[1] + EPSILON,
            "popup rect {pos:?} + {size:?} escaped parent work area {:?}..{:?}",
            area.min,
            area.max
        );
    }

    #[test]
    fn popup_rect_stays_inside_narrow_parent_at_right_edge_and_fractional_dpi() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = dear_imgui_rs::Context::create();
        crate::app::apply_ui_scale(ctx.style_mut(), 1.5);
        let short = vec![row("Bow", "bow_alias")];
        let widened = vec![
            row("Bow", "bow_alias"),
            row(
                "A very long display name that widens the picker after opening",
                "long_alias",
            ),
        ];
        let mut cached_rows = PickerRows::default();

        let (wide_area, short_pos, short_size, _) = draw_popup_frame(
            &mut ctx,
            [820.0, 420.0],
            [700.0, 280.0],
            &short,
            &mut cached_rows,
        );
        assert_rect_inside(wide_area, short_pos, short_size);

        cached_rows.invalidate();
        let (widened_area, widened_pos, widened_size, _) = draw_popup_frame(
            &mut ctx,
            [820.0, 420.0],
            [700.0, 280.0],
            &widened,
            &mut cached_rows,
        );
        assert_eq!(wide_area, widened_area);
        assert_rect_inside(widened_area, widened_pos, widened_size);
        assert!(
            widened_size[0] > short_size[0],
            "popup did not widen with its content: {} <= {}",
            widened_size[0],
            short_size[0]
        );

        cached_rows.invalidate();
        let (narrow_area, narrow_pos, narrow_size, _) = draw_popup_frame(
            &mut ctx,
            [520.0, 420.0],
            [320.0, 280.0],
            &widened,
            &mut cached_rows,
        );
        assert_rect_inside(narrow_area, narrow_pos, narrow_size);
    }

    #[test]
    fn popup_content_fit_width_tracks_the_longest_name() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = dear_imgui_rs::Context::create();
        let short = vec![row("Bow", "stable_alias")];
        let long = vec![
            row("Bow", "stable_alias"),
            row(
                "A substantially longer display name that should widen the picker",
                "stable_alias",
            ),
        ];
        let mut cached_rows = PickerRows::default();
        let (area, _, short_popup_size, short_layout) = draw_popup_frame(
            &mut ctx,
            [1400.0, 800.0],
            [1000.0, 650.0],
            &short,
            &mut cached_rows,
        );
        cached_rows.invalidate();
        let (long_area, _, long_popup_size, long_layout) = draw_popup_frame(
            &mut ctx,
            [1400.0, 800.0],
            [1000.0, 650.0],
            &long,
            &mut cached_rows,
        );
        assert_eq!(area, long_area);
        assert!(
            long_layout.content_width > short_layout.content_width,
            "fit width did not grow: {} <= {}",
            long_layout.content_width,
            short_layout.content_width
        );
        assert!(
            long_popup_size[0] > short_popup_size[0],
            "actual popup did not grow with the longest name: {} <= {}",
            long_popup_size[0],
            short_popup_size[0]
        );
    }

    #[test]
    fn cached_rows_format_once_per_result_set() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = dear_imgui_rs::Context::create();
        let calls = Rc::new(Cell::new(0));
        let hits = vec![CountingHit {
            name: "Bow".into(),
            alias: "shortbow".into(),
            secondary_calls: calls.clone(),
        }];
        let mut cached_rows = PickerRows::default();

        for _ in 0..2 {
            ctx.prepare_frame(
                FramePrepareOptions::new([600.0, 400.0], 1.0 / 60.0).renderer_has_textures(),
            );
            {
                let ui = ctx.frame();
                ui.window("dp-cached-rows")
                    .position([20.0, 20.0], Condition::Always)
                    .size([400.0, 300.0], Condition::Always)
                    .build(|| {
                        draw_hit_rows(ui, &hits, &mut cached_rows, None);
                    });
            }
            ctx.render();
        }
        assert_eq!(calls.get(), 1, "unchanged rows were formatted again");

        let mut hits = hits;
        hits[0].alias = "changed".into();
        cached_rows.invalidate();
        ctx.prepare_frame(
            FramePrepareOptions::new([600.0, 400.0], 1.0 / 60.0).renderer_has_textures(),
        );
        {
            let ui = ctx.frame();
            ui.window("dp-cached-rows")
                .position([20.0, 20.0], Condition::Always)
                .size([400.0, 300.0], Condition::Always)
                .build(|| {
                    draw_hit_rows(ui, &hits, &mut cached_rows, None);
                });
        }
        ctx.render();
        assert_eq!(calls.get(), 2, "changed result data was not reformatted");
        assert_eq!(cached_rows.row(0).secondary, "#1  changed");
    }
    #[test]
    fn popup_without_positive_parent_width_uses_minimum_content_width() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
        );
        let hits = vec![row("Bow", "shortbow")];
        let mut cached_rows = PickerRows::default();
        {
            let ui = ctx.frame();
            ui.window("dp-no-positive-width")
                .size([400.0, 500.0], Condition::Always)
                .build(|| {
                    let area = PopupWorkArea {
                        min: [100.0, 100.0],
                        max: [100.0, 600.0],
                    };
                    let layout = popup(
                        ui,
                        "##no-positive-width",
                        &mut cached_rows,
                        |rows| {
                            prepare_popup(ui, area, [[100.0, 140.0], [100.0, 160.0]], &hits, rows)
                        },
                        |_, _| {},
                    );
                    assert_eq!(
                        layout.content_width,
                        crate::theme::scale_px(ui, PICKER_MIN_WIDTH)
                    );
                });
        }
        ctx.render();
    }

    #[test]
    fn closed_popup_consumes_next_window_data() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            FramePrepareOptions::new([600.0, 400.0], 1.0 / 60.0).renderer_has_textures(),
        );
        let area = PopupWorkArea {
            min: [250.0, 150.0],
            max: [580.0, 350.0],
        };
        let anchor = [[350.0, 250.0], [350.0, 270.0]];
        let hits = [row("Bow", "shortbow")];
        let mut rows = PickerRows::default();
        let content_ran = Cell::new(false);
        {
            let ui = ctx.frame();
            ui.window("dp-closed-popup-parent")
                .size([400.0, 300.0], Condition::Always)
                .build(|| {
                    let layout = popup(
                        ui,
                        "##closed-picker-popup",
                        &mut rows,
                        |rows| prepare_popup(ui, area, anchor, &hits, rows),
                        |_, _| content_ran.set(true),
                    );
                    let flags = ui.with_bound_context(|| {
                        // SAFETY: `with_bound_context` binds the live context
                        // whose next-window data is being inspected.
                        unsafe {
                            (*dear_imgui_rs::sys::igGetCurrentContext())
                                .NextWindowData
                                .HasFlags
                        }
                    });
                    assert_eq!(flags, 0, "closed popup left next-window data pending");
                    let mut following_position = None;
                    ui.window("dp-following-plain-window")
                        .size([200.0, 100.0], Condition::Always)
                        .build(|| {
                            following_position = Some(ui.window_pos());
                        });
                    assert_ne!(
                        following_position.expect("following plain window built"),
                        layout.position,
                        "closed popup position leaked onto the following window"
                    );
                });
        }
        ctx.render();
        assert!(!content_ran.get(), "the popup was unexpectedly open");
    }

    #[test]
    fn nan_anchor_and_degenerate_work_area_keep_popup_rect_finite() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = dear_imgui_rs::Context::create();
        let anchor = [[f32::NAN; 2]; 2];
        let mut rows = PickerRows::default();
        let zero_size_area = PopupWorkArea {
            min: [100.0, 100.0],
            max: [100.0, 100.0],
        };
        let (zero_layout, zero_size_rect) = draw_synthetic_popup_frame(
            &mut ctx,
            "##zero-size-picker-popup",
            zero_size_area,
            anchor,
            &mut rows,
        );
        assert!(
            zero_layout
                .position
                .iter()
                .chain(zero_size_rect.0.iter())
                .chain(zero_size_rect.1.iter())
                .all(|value| value.is_finite()),
            "zero-size work area produced non-finite popup geometry: \
             layout={zero_layout:?}, rect={zero_size_rect:?}"
        );

        let nonfinite_area = PopupWorkArea {
            min: [f32::NAN; 2],
            max: [f32::NAN; 2],
        };
        let (nonfinite_layout, nonfinite_rect) = draw_synthetic_popup_frame(
            &mut ctx,
            "##nonfinite-picker-popup",
            nonfinite_area,
            anchor,
            &mut rows,
        );
        assert!(
            nonfinite_layout
                .position
                .iter()
                .chain(nonfinite_rect.0.iter())
                .chain(nonfinite_rect.1.iter())
                .all(|value| value.is_finite()),
            "non-finite work area produced non-finite popup geometry: \
             layout={nonfinite_layout:?}, rect={nonfinite_rect:?}"
        );
    }
}
