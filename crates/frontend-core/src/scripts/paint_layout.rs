//! One paint frame's layout: every rectangle a renderer draws, in
//! applet pixels, from a [`PaintView`] and one measured cell width.
//!
//! Widths are `chars().count() as f32 * cell_w`, as rs2b0t's monospace
//! `charW`, so the layout needs no text-measure callback and its tests
//! need no font. The renderer measures `cell_w` once per frame (the
//! width of "0" at the body size) and maps each rectangle with
//! `min + v * s`.
//!
//! A [`PaintLayout`] is reused every frame. Its vectors keep their
//! capacity, so a steady frame allocates nothing. They grow once to
//! the largest paint seen. The script crate caps the lines and the
//! buttons a frame can advertise.

use super::paint::PaintView;
use super::paint_style::{
    BUTTON_GAP, BUTTON_H, BUTTON_PAD, BUTTON_ROW_ADVANCE, BUTTON_TOP_GAP, FOOTER_H, LINE, PAD,
    PANEL_RECT, RAIL_INSET, RAIL_TEXT_DX, RAIL_TEXT_TOP_FRAC, RAIL_W, STRIP_TAB_INSET, TAB_GAP,
    TAB_H, TAB_INSET, TAB_PAD, TAB_TEXT_DX, TAB_TEXT_DY, TAB_TRAIL, TAB_X0, TEXT_DY, TITLE_H,
    TITLE_TEXT_DY, TOGGLE_W,
};

/// One applet-space rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl PaintRect {
    fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// The left edge.
    pub fn x(&self) -> f32 {
        self.x
    }

    /// The top edge.
    pub fn y(&self) -> f32 {
        self.y
    }

    /// The width.
    pub fn w(&self) -> f32 {
        self.w
    }

    /// The height.
    pub fn h(&self) -> f32 {
        self.h
    }

    /// The right edge: `x + w`.
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    /// The bottom edge: `y + h`.
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

/// One named chrome entry: a strip tab or a rail entry. The layout
/// bounds the entry to its strip or rail area and gives it the cell
/// count its name may use, so a renderer only draws
/// [`clipped_prefix`] of the name at the text origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChromeItem {
    name_index: usize,
    rect: PaintRect,
    cells: usize,
    text_origin: [f32; 2],
}

impl ChromeItem {
    /// The index into the band's `names()`.
    pub fn name_index(&self) -> usize {
        self.name_index
    }

    /// The entry's rectangle, and its click target.
    pub fn rect(&self) -> PaintRect {
        self.rect
    }

    /// The cells the name may use, the ellipsis included when the
    /// name is clipped.
    pub fn cells(&self) -> usize {
        self.cells
    }

    /// The name's text origin, in applet pixels.
    pub fn text_origin(&self) -> [f32; 2] {
        self.text_origin
    }
}

/// One tabs-band entry. The layout bounds the entry to the panel
/// and gives it the cell count its name may use, so a renderer only
/// draws [`clipped_prefix`] of the name at the text origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabItem {
    band_index: usize,
    name_index: usize,
    rect: PaintRect,
    cells: usize,
    text_origin: [f32; 2],
}

impl TabItem {
    /// The index into the paint's `tabs()`.
    pub fn band_index(&self) -> usize {
        self.band_index
    }

    /// The index into the band's `names()`.
    pub fn name_index(&self) -> usize {
        self.name_index
    }

    /// The entry's rectangle, and its click target.
    pub fn rect(&self) -> PaintRect {
        self.rect
    }

    /// The cells the name may use, the ellipsis included when the
    /// name is clipped.
    pub fn cells(&self) -> usize {
        self.cells
    }

    /// The name's text origin, in applet pixels.
    pub fn text_origin(&self) -> [f32; 2] {
        self.text_origin
    }
}

/// A text slot with the cell count its text may use: the strip status
/// and the strip brand. A renderer draws [`clipped_prefix`] of the text
/// at the slot's left edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintSlot {
    rect: PaintRect,
    cells: usize,
}

impl PaintSlot {
    /// The slot's rectangle.
    pub fn rect(&self) -> PaintRect {
        self.rect
    }

    /// The cells the text may use, the ellipsis included when the text
    /// is clipped.
    pub fn cells(&self) -> usize {
        self.cells
    }
}

/// One visible text row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowItem {
    index: usize,
    rect: PaintRect,
    cells: usize,
}

impl RowItem {
    /// The index into the paint's `lines()`.
    pub fn index(&self) -> usize {
        self.index
    }

    /// The row's rectangle. Its top is the row top. The text top is
    /// [`TEXT_DY`](super::paint_style::TEXT_DY) below it.
    pub fn rect(&self) -> PaintRect {
        self.rect
    }

    /// The cells the row text may use, the ellipsis included when the
    /// text is clipped.
    pub fn cells(&self) -> usize {
        self.cells
    }
}

/// One drawn button.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ButtonItem {
    index: usize,
    rect: PaintRect,
    label_cells: usize,
}

impl ButtonItem {
    /// The index into the paint's `buttons()`.
    pub fn index(&self) -> usize {
        self.index
    }

    /// The button's rectangle, and its click target.
    pub fn rect(&self) -> PaintRect {
        self.rect
    }

    /// The cells the label may use, the ellipsis included when the
    /// label is clipped.
    pub fn label_cells(&self) -> usize {
        self.label_cells
    }
}

/// The footer's place: the origin its (possibly clipped) text draws
/// from, so the text ends at the panel's right edge less [`PAD`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FooterLayout {
    origin: [f32; 2],
    cells: usize,
}

impl FooterLayout {
    /// The text origin, in applet pixels.
    pub fn origin(&self) -> [f32; 2] {
        self.origin
    }

    /// The cells the footer text may use, the ellipsis included when
    /// the text is clipped.
    pub fn cells(&self) -> usize {
        self.cells
    }
}

/// The prefix of `text` that fits in `cells` cells, and whether the
/// caller must draw an ellipsis after it. Text that fits returns
/// itself and `false`. Longer text returns its first `cells - 1`
/// chars and `true`, so the prefix plus the ellipsis use `cells`
/// cells. Zero cells fit nothing, not even the ellipsis.
pub fn clipped_prefix(text: &str, cells: usize) -> (&str, bool) {
    if cells == 0 {
        return ("", false);
    }
    let mut last_start = 0usize;
    for (count, (index, _ch)) in text.char_indices().enumerate() {
        if count == cells {
            return (&text[..last_start], true);
        }
        if count == cells - 1 {
            last_start = index;
        }
    }
    (text, false)
}

/// One button's plan while the flow runs: its natural width, whether
/// it is a host control, and whether the flow keeps it.
#[derive(Debug, Clone, Copy)]
struct ButtonPlan {
    index: usize,
    width: f32,
    is_host: bool,
    keep: bool,
}

/// A reused paint layout. See the module text.
#[derive(Debug, Default)]
pub struct PaintLayout {
    panel: PaintRect,
    title_bar: PaintRect,
    toggle: PaintRect,
    title_cells: usize,
    strip_tabs: Vec<ChromeItem>,
    strip_status: Option<PaintSlot>,
    strip_brand: Option<PaintSlot>,
    tabs: Vec<TabItem>,
    rail_rect: Option<PaintRect>,
    rail: Vec<ChromeItem>,
    rows: Vec<RowItem>,
    buttons: Vec<ButtonItem>,
    footer: Option<FooterLayout>,
    button_plans: Vec<ButtonPlan>,
}

impl Default for PaintRect {
    fn default() -> Self {
        Self::new(PANEL_RECT[0], PANEL_RECT[1], PANEL_RECT[2], PANEL_RECT[3])
    }
}

/// The cells that fit in `width` at `cell_w`, never negative. The
/// epsilon keeps a width built as `n * cell_w` (a button's label area)
/// from flooring to `n - 1` cells through float rounding: at cell
/// widths just above a whole pixel the quotient can land at
/// 3.9999998 for n = 4. The PAINT-SLICE-A headed check first drew a
/// "Stop" label as "St…" that way.
fn cells_in(width: f32, cell_w: f32) -> usize {
    let cells = (width / cell_w + 1e-4).floor();
    if cells > 0.0 {
        cells as usize
    } else {
        0
    }
}

/// The cell width of `text` at `cell_w`.
fn text_cells(text: &str, cell_w: f32) -> f32 {
    text.chars().count() as f32 * cell_w
}

/// Lay the kept buttons out from (`left`, 0), wrapping at `right`.
/// The y of each rect is its row's offset from the flow top. Returns
/// the row count.
fn flow_buttons(
    plans: &[ButtonPlan],
    out: &mut Vec<ButtonItem>,
    left: f32,
    right: f32,
    cell_w: f32,
) -> usize {
    out.clear();
    let body_w = (right - left).max(1.0);
    let mut x = left;
    let mut row = 0usize;
    for plan in plans {
        if !plan.keep {
            continue;
        }
        let w = plan.width.min(body_w);
        if x > left && x + w > right {
            row += 1;
            x = left;
        }
        out.push(ButtonItem {
            index: plan.index,
            rect: PaintRect::new(x, row as f32 * BUTTON_ROW_ADVANCE, w, BUTTON_H),
            label_cells: cells_in(w - BUTTON_PAD, cell_w),
        });
        x += w + BUTTON_GAP;
    }
    if out.is_empty() {
        0
    } else {
        row + 1
    }
}

impl PaintLayout {
    /// An empty layout. [`compute`](Self::compute) fills it.
    pub fn new() -> Self {
        Self::default()
    }

    /// The panel rectangle. Its height is the title bar's while the
    /// layout is collapsed.
    pub fn panel(&self) -> PaintRect {
        self.panel
    }

    /// The region under the title bar at the panel's full height.
    pub fn body(&self) -> PaintRect {
        PaintRect::new(
            PANEL_RECT[0],
            PANEL_RECT[1] + TITLE_H,
            PANEL_RECT[2],
            PANEL_RECT[3] - TITLE_H,
        )
    }

    /// The title bar rectangle.
    pub fn title_bar(&self) -> PaintRect {
        self.title_bar
    }

    /// The collapse toggle square, and its click target. A click
    /// anywhere else on the title bar does not collapse the paint.
    pub fn toggle(&self) -> PaintRect {
        self.toggle
    }

    /// The cells the title text may use before the toggle.
    pub fn title_cells(&self) -> usize {
        self.title_cells
    }

    /// The strip's tabs, left to right.
    pub fn strip_tabs(&self) -> &[ChromeItem] {
        &self.strip_tabs
    }

    /// The strip's status slot. It starts at the first free slot after
    /// the strip tabs. Its cells stop one cell short of the brand.
    pub fn strip_status(&self) -> Option<PaintSlot> {
        self.strip_status
    }

    /// The strip's brand slot.
    pub fn strip_brand(&self) -> Option<PaintSlot> {
        self.strip_brand
    }

    /// The tabs-band entries, band by band, left to right.
    pub fn tabs(&self) -> &[TabItem] {
        &self.tabs
    }

    /// The rail's whole area. Its height is the body less one line.
    pub fn rail_rect(&self) -> Option<PaintRect> {
        self.rail_rect
    }

    /// The rail's entries, top to bottom.
    pub fn rail(&self) -> &[ChromeItem] {
        &self.rail
    }

    /// The visible text rows, top to bottom. Rows that would pass
    /// under the button flow are absent.
    pub fn rows(&self) -> &[RowItem] {
        &self.rows
    }

    /// The drawn buttons, in advertised order.
    pub fn buttons(&self) -> &[ButtonItem] {
        &self.buttons
    }

    /// The footer's place.
    pub fn footer(&self) -> Option<FooterLayout> {
        self.footer
    }

    /// Fill the layout for `view`. `collapsed` keeps only the title
    /// bar. `cell_w` is the renderer's measured cell width in applet
    /// pixels. A non-positive or non-finite `cell_w` counts as 1.
    pub fn compute(&mut self, view: PaintView<'_>, collapsed: bool, cell_w: f32) {
        let cell = if cell_w.is_finite() && cell_w > 0.0 {
            cell_w
        } else {
            1.0
        };
        let [px, py, pw, ph] = PANEL_RECT;
        self.panel = PaintRect::new(px, py, pw, if collapsed { TITLE_H } else { ph });
        self.title_bar = PaintRect::new(px, py, pw, TITLE_H);
        self.toggle = PaintRect::new(px + pw - TOGGLE_W, py, TOGGLE_W, TITLE_H);
        self.title_cells = cells_in(self.toggle.x - (px + PAD), cell);
        self.strip_tabs.clear();
        self.strip_status = None;
        self.strip_brand = None;
        self.tabs.clear();
        self.rail_rect = None;
        self.rail.clear();
        self.rows.clear();
        self.buttons.clear();
        self.footer = None;
        self.button_plans.clear();

        if let Some(strip) = view.strip() {
            let brand = strip.brand().or_else(|| view.title()).unwrap_or("");
            let brand_chars = brand.chars().count();
            let brand_reserve = if brand_chars == 0 {
                TOGGLE_W
            } else {
                brand_chars as f32 * cell + PAD + TOGGLE_W
            };
            // The strip tabs stop at the toggle. They also leave the
            // brand its smallest readable place — one char plus the
            // ellipsis, or the whole brand when it is shorter — so an
            // over-long strip clips its last tab with an ellipsis and
            // drops the tabs after it instead of covering the brand.
            let brand_min = brand_chars.min(2) as f32 * cell;
            let strip_limit = self.toggle.x - brand_min;
            let tab_h = (TITLE_H - 2.0 * STRIP_TAB_INSET).max(1.0);
            let mut x = px + PAD;
            for (name_index, name) in strip.names().iter().enumerate() {
                if x >= strip_limit {
                    break;
                }
                let natural = (text_cells(name, cell) + TAB_PAD).max(1.0);
                let w = natural.min(strip_limit - x);
                let cells = cells_in(w - TAB_PAD, cell);
                if w < natural && cells == 0 {
                    break;
                }
                self.strip_tabs.push(ChromeItem {
                    name_index,
                    rect: PaintRect::new(x, py + STRIP_TAB_INSET, w, tab_h),
                    cells,
                    text_origin: [x + TAB_TEXT_DX, py + TITLE_TEXT_DY],
                });
                x += w;
            }
            // The brand slot ends one PAD short of the panel's right
            // edge (frozen `stripSegments`). Its width reserves the
            // brand text, one PAD, and the toggle square. When the
            // tabs reach their limit the brand starts there and clips
            // to the cells left before the toggle, which the limit
            // keeps at the minimum above, so it never disappears.
            let brand_x = x.max(px + pw - PAD - brand_reserve);
            if strip.status().is_some() {
                let slot = PaintRect::new(x, py, (brand_x - x).max(0.0), TITLE_H);
                self.strip_status = Some(PaintSlot {
                    rect: slot,
                    cells: cells_in(slot.w, cell).saturating_sub(1),
                });
            }
            if brand_chars > 0 {
                self.strip_brand = Some(PaintSlot {
                    rect: PaintRect::new(brand_x, py, (px + pw - PAD - brand_x).max(0.0), TITLE_H),
                    cells: cells_in(self.toggle.x - brand_x, cell),
                });
            }
        }
        if collapsed {
            return;
        }

        let mut content_top = py + TITLE_H;
        for (band_index, band) in view.tabs().enumerate() {
            if band.names().is_empty() {
                continue;
            }
            // A band's entries stop at the body's right edge. The
            // last entry that reaches it clips with an ellipsis; the
            // entries after it are not laid out.
            let band_limit = px + pw - PAD;
            let mut x = px + TAB_X0;
            for (name_index, name) in band.names().iter().enumerate() {
                if x >= band_limit {
                    break;
                }
                let natural = (text_cells(name, cell) + TAB_PAD).max(1.0);
                let w = natural.min(band_limit - x);
                let cells = cells_in(w - TAB_PAD, cell);
                if w < natural && cells == 0 {
                    break;
                }
                let rect = PaintRect::new(x, content_top + TAB_INSET, w, TAB_H);
                self.tabs.push(TabItem {
                    band_index,
                    name_index,
                    rect,
                    cells,
                    text_origin: [rect.x + TAB_TEXT_DX, rect.y + TAB_TEXT_DY],
                });
                x += w + TAB_GAP;
            }
            content_top += TAB_INSET + TAB_H + TAB_TRAIL;
        }

        let panel_bottom = py + ph;
        let footer_h = if view.footer().is_some() {
            FOOTER_H
        } else {
            0.0
        };
        let content_bottom = panel_bottom - footer_h;

        let rail = view.rail().filter(|rail| !rail.names().is_empty());
        if let Some(rail) = rail {
            let rail_h = (panel_bottom - content_top - LINE).max(0.0);
            let area = PaintRect::new(
                px + RAIL_INSET,
                content_top,
                RAIL_W - 2.0 * RAIL_INSET,
                rail_h,
            );
            self.rail_rect = Some(area);
            let slot_h = rail_h / rail.names().len() as f32;
            for name_index in 0..rail.names().len() {
                let rect = PaintRect::new(
                    area.x,
                    content_top + slot_h * name_index as f32,
                    area.w,
                    slot_h,
                );
                self.rail.push(ChromeItem {
                    name_index,
                    rect,
                    cells: cells_in(rect.w - RAIL_TEXT_DX, cell),
                    text_origin: [rect.x + RAIL_TEXT_DX, rect.y + rect.h * RAIL_TEXT_TOP_FRAC],
                });
            }
        }
        let body_left = px + PAD + if rail.is_some() { RAIL_W } else { 0.0 };
        let body_right = px + pw - PAD;
        let body_w = (body_right - body_left).max(1.0);

        for (index, button) in view.buttons().enumerate() {
            self.button_plans.push(ButtonPlan {
                index,
                width: (text_cells(button.label(), cell) + BUTTON_PAD).max(1.0),
                is_host: button.control().is_some(),
                keep: true,
            });
        }
        // Buttons get at most the rows that fit the content area. A
        // paint with more buttons than rows drops its last script
        // buttons first. The host controls are never dropped: a
        // paint that advertises them always keeps at least their
        // row, even when tab bands or text rows have already used
        // the whole content area. That row then ends at the content
        // bottom (below), over the place the bands and rows wanted.
        let has_host = self.button_plans.iter().any(|plan| plan.is_host);
        let avail_h = content_bottom - content_top - BUTTON_TOP_GAP;
        let fit_rows = if avail_h < BUTTON_H {
            0
        } else {
            1 + ((avail_h - BUTTON_H) / BUTTON_ROW_ADVANCE) as usize
        };
        let max_button_rows = if has_host { fit_rows.max(1) } else { fit_rows };
        let mut button_rows = flow_buttons(
            &self.button_plans,
            &mut self.buttons,
            body_left,
            body_right,
            cell,
        );
        while button_rows > max_button_rows {
            let drop_at = self
                .button_plans
                .iter()
                .rposition(|plan| plan.keep && !plan.is_host);
            match drop_at {
                Some(at) => self.button_plans[at].keep = false,
                None => break,
            }
            button_rows = flow_buttons(
                &self.button_plans,
                &mut self.buttons,
                body_left,
                body_right,
                cell,
            );
        }

        let rows_fit = cells_in(content_bottom - content_top, LINE);
        let mut visible = view.lines().len().min(rows_fit);
        if button_rows > 0 {
            let flow_h = (button_rows - 1) as f32 * BUTTON_ROW_ADVANCE + BUTTON_H;
            let natural_top = content_top + visible as f32 * LINE + BUTTON_TOP_GAP;
            // The flow sits under the text rows when it fits there.
            // When it does not fit, it moves up to end at the content
            // bottom, and the text rows that would pass under it are
            // not drawn. The reserved host row of a paint whose tab
            // bands pass the content bottom ends there too: it never
            // starts above the body, and the tab bands that would
            // pass under it are dropped, so no band entry keeps a
            // click target under the controls.
            let flow_top = natural_top.min(content_bottom - flow_h).max(py + TITLE_H);
            visible = visible.min(cells_in(flow_top - BUTTON_TOP_GAP - content_top, LINE));
            self.tabs.retain(|tab| tab.rect.bottom() <= flow_top + 0.01);
            for button in &mut self.buttons {
                button.rect.y += flow_top;
            }
        }
        let row_cells = cells_in(body_w, cell);
        for index in 0..visible {
            self.rows.push(RowItem {
                index,
                rect: PaintRect::new(body_left, content_top + index as f32 * LINE, body_w, LINE),
                cells: row_cells,
            });
        }

        if let Some(footer) = view.footer() {
            let cells = cells_in(pw - 2.0 * PAD, cell);
            let drawn = footer.chars().count().min(cells);
            self.footer = Some(FooterLayout {
                origin: [
                    px + pw - PAD - drawn as f32 * cell,
                    panel_bottom - FOOTER_H + TEXT_DY,
                ],
                cells,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{clipped_prefix, PaintLayout};
    use crate::scripts::ScriptRunState;
    use crate::test_support::paint::TestPaint;

    const CELL: f32 = 7.0;

    fn compute_layout(paint: &TestPaint) -> PaintLayout {
        let frame = paint.frame();
        let mut layout = PaintLayout::new();
        layout.compute(frame.view(), false, CELL);
        layout
    }

    /// Design test 4: the layout uses rs2b0t's metrics.
    #[test]
    fn paint_layout_uses_rs2b0t_metrics() {
        let paint = TestPaint::new(Some("JiveCrafting"), &["one", "two", "three"])
            .with_strip(
                "k",
                &["Statistics", "Options"],
                "Statistics",
                Some("walking to the bank and then some more text here"),
                Some("JiveCrafting"),
            )
            .with_rail("r", &["Overview", "Supplies"], "Overview");
        let layout = compute_layout(&paint);
        assert_eq!(layout.title_bar().h(), 20.0, "the title bar is 20 px");
        let rows = layout.rows();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[1].rect().y() - rows[0].rect().y(),
            16.0,
            "row pitch 16"
        );
        assert_eq!(
            rows[2].rect().y() - rows[1].rect().y(),
            16.0,
            "row pitch 16"
        );
        assert_eq!(
            rows[0].rect().x() - layout.panel().x(),
            8.0 + 72.0,
            "the body inset is 8 px right of the 72 px rail"
        );
        let no_rail = compute_layout(&TestPaint::new(Some("T"), &["one"]));
        assert_eq!(
            no_rail.rows()[0].rect().x() - no_rail.panel().x(),
            8.0,
            "the body inset is 8 px without a rail"
        );
        let rail = layout.rail_rect().expect("rail area");
        assert_eq!(
            rail.h(),
            layout.body().h() - 16.0,
            "the rail height is the body less one line"
        );
        let tabs = layout.strip_tabs();
        let status = layout.strip_status().expect("status slot");
        assert_eq!(
            status.rect().x(),
            tabs.last().expect("strip tabs").rect().right(),
            "the status starts at the first free slot after the tabs"
        );
        let brand = layout.strip_brand().expect("brand slot");
        assert!(
            status.rect().x() + status.cells() as f32 * CELL <= brand.rect().x() - CELL + 0.01,
            "the status stops one cell short of the brand: status {:?} brand {:?}",
            status.rect(),
            brand.rect()
        );
        assert_eq!(
            brand.rect().right(),
            layout.panel().right() - 8.0,
            "the brand slot ends one inset short of the panel edge"
        );
    }

    /// Design test 2 at layout level, with an exact cell width: Pause
    /// and Stop share one compact row.
    #[test]
    fn pause_and_stop_share_one_compact_row_in_the_layout() {
        let paint = TestPaint::new(Some("Helper"), &["status"]).with_host_controls();
        let frame = paint.frame_for_run("alice", ScriptRunState::Running);
        let mut layout = PaintLayout::new();
        layout.compute(frame.view(), false, CELL);
        let buttons = layout.buttons();
        assert_eq!(buttons.len(), 2);
        let pause = buttons[0].rect();
        let stop = buttons[1].rect();
        assert_eq!(pause.h(), 16.0);
        assert_eq!(stop.h(), 16.0);
        assert_eq!(pause.y(), stop.y(), "one row");
        assert_eq!(pause.x(), 16.0, "Pause left = panel 8 + inset 8");
        assert_eq!(pause.w(), 5.0 * CELL + 18.0, "Pause width = 5 cells + 18");
        assert_eq!(stop.x(), pause.right() + 6.0, "Stop follows Pause by 6 px");
        assert_eq!(stop.w(), 4.0 * CELL + 18.0, "Stop width = 4 cells + 18");
    }

    /// A button sized from its label keeps the whole label at a
    /// measured-style cell width. The PAINT-SLICE-A headed check first
    /// drew "Stop" as "St…": the shot's glyph starts are 14 window px
    /// apart at scale 2 (a 7.0 px cell), and at cell widths just above
    /// 7.0 — this test's 7.0016656 is one — `(4 * cell + 18 - 18) /
    /// cell` rounds to 3.9999998 and floored to three cells.
    #[test]
    fn button_labels_fit_their_buttons_at_the_measured_cell_width() {
        let paint = TestPaint::new(Some("Helper"), &["status"]).with_host_controls();
        let frame = paint.frame_for_run("alice", ScriptRunState::Running);
        let mut layout = PaintLayout::new();
        layout.compute(frame.view(), false, 7.0016656);
        let buttons = layout.buttons();
        assert_eq!(buttons.len(), 2);
        assert_eq!(buttons[0].label_cells(), 5, "Pause keeps five cells");
        assert_eq!(buttons[1].label_cells(), 4, "Stop keeps four cells");
        assert_eq!(
            clipped_prefix("Stop", buttons[1].label_cells()),
            ("Stop", false),
            "the Stop label draws whole"
        );
    }

    /// Design test 3 at layout level: a Jive-like paint that fills the
    /// dock keeps both host controls inside the body and above the
    /// footer row. The text rows that would pass under them are gone.
    #[test]
    fn host_controls_stay_visible_when_rows_fill_the_dock_in_the_layout() {
        let paint = TestPaint::new(
            Some("JiveCrafting"),
            &["r1", "r2", "r3", "r4", "r5", "r6", "r7", "r8", "r9"],
        )
        .with_strip(
            "k",
            &["Statistics", "Options"],
            "Statistics",
            Some("ok"),
            Some("JiveCrafting"),
        )
        .with_rail("r", &["Overview", "Supplies"], "Overview")
        .with_footer("by Jive")
        .with_host_controls();
        let frame = paint.frame_for_run("alice", ScriptRunState::Running);
        let mut layout = PaintLayout::new();
        layout.compute(frame.view(), false, CELL);
        let buttons = layout.buttons();
        assert_eq!(buttons.len(), 2, "Pause and Stop are both laid out");
        let footer_top = layout.panel().bottom() - 16.0;
        for button in buttons {
            let rect = button.rect();
            assert!(
                rect.x() >= layout.panel().x() && rect.right() <= layout.panel().right(),
                "the control stays inside the dock: {rect:?}"
            );
            assert!(
                rect.bottom() <= footer_top + 0.01,
                "the control stays above the footer row: {rect:?}"
            );
            assert!(
                rect.y() >= layout.title_bar().bottom(),
                "the control stays below the title bar: {rect:?}"
            );
        }
        let flow_top = buttons[0].rect().y();
        for row in layout.rows() {
            assert!(
                row.rect().bottom() <= flow_top + 0.01,
                "no text row passes under the controls: row {:?} flow top {flow_top}",
                row.rect()
            );
        }
        assert!(
            layout.rows().len() < 9,
            "the rows that would pass under the controls are not drawn"
        );
    }

    /// The isolate admits 16 tab bands. Five bands already use the
    /// whole content area at this cell width. Pause and Stop stay
    /// laid out for 5, 6 and 16 bands, with and without a footer:
    /// their row ends at the content bottom, and no tab entry or
    /// text row keeps a place under it.
    #[test]
    fn host_controls_stay_visible_with_five_or_more_tab_bands_in_the_layout() {
        for bands in [5usize, 6, 16] {
            for footer in [false, true] {
                let mut paint = TestPaint::new(Some("T"), &["row one", "row two", "row three"]);
                for band in 0..bands {
                    paint = paint.with_tab_band(&format!("b{band}"), &["A", "B"], "A");
                }
                if footer {
                    paint = paint.with_footer("by x");
                }
                let paint = paint.with_host_controls();
                let frame = paint.frame_for_run("alice", ScriptRunState::Running);
                let mut layout = PaintLayout::new();
                layout.compute(frame.view(), false, CELL);
                let buttons = layout.buttons();
                assert_eq!(
                    buttons.len(),
                    2,
                    "bands {bands} footer {footer}: Pause and Stop are both laid out"
                );
                let content_bottom = layout.panel().bottom() - if footer { 16.0 } else { 0.0 };
                let flow_top = buttons[0].rect().y();
                for button in buttons {
                    let rect = button.rect();
                    assert_eq!(rect.h(), 16.0, "bands {bands} footer {footer}");
                    assert_eq!(rect.y(), flow_top, "bands {bands} footer {footer}: one row");
                    assert!(
                        (rect.bottom() - content_bottom).abs() <= 0.01,
                        "bands {bands} footer {footer}: the row ends at the content bottom: {rect:?}"
                    );
                    assert!(
                        rect.y() >= layout.title_bar().bottom(),
                        "bands {bands} footer {footer}: the row stays in the body: {rect:?}"
                    );
                    for tab in layout.tabs() {
                        let tab = tab.rect();
                        assert!(
                            tab.bottom() <= rect.y() + 0.01
                                || tab.right() <= rect.x()
                                || rect.right() <= tab.x(),
                            "bands {bands} footer {footer}: no tab entry under a control: tab {tab:?} control {rect:?}"
                        );
                    }
                }
                for row in layout.rows() {
                    assert!(
                        row.rect().bottom() <= flow_top + 0.01,
                        "bands {bands} footer {footer}: no row under the controls: {:?}",
                        row.rect()
                    );
                }
                if bands == 16 {
                    assert!(
                        layout.tabs().len() < bands * 2,
                        "footer {footer}: the bands that pass under the controls are dropped"
                    );
                }
            }
        }
    }

    /// An over-long strip clips its last visible tab with an
    /// ellipsis, drops the tabs after it, and still shows the brand
    /// as a prefix plus an ellipsis. A strip that fits keeps every
    /// name whole.
    #[test]
    fn strip_tabs_and_the_brand_clip_with_an_ellipsis() {
        let names = [
            "StatisticsPageOne",
            "StatisticsPageTwo",
            "StatisticsPageSix",
            "StatisticsPageTen",
            "StatisticsPageMax",
        ];
        let brand = "JiveCraftingBySomeone";
        let paint = TestPaint::new(Some("T"), &["row"]).with_strip(
            "k",
            &names,
            names[0],
            Some("walking"),
            Some(brand),
        );
        let layout = compute_layout(&paint);
        let tabs = layout.strip_tabs();
        assert!(
            !tabs.is_empty() && tabs.len() < names.len(),
            "the strip drops the tabs past its limit: {tabs:?}"
        );
        for tab in tabs {
            assert!(
                tab.rect().right() <= layout.toggle().x() + 0.01,
                "no strip tab passes the toggle: {:?}",
                tab.rect()
            );
            let name = names[tab.name_index()];
            let (prefix, ellipsis) = clipped_prefix(name, tab.cells());
            assert!(
                name.starts_with(prefix),
                "a tab draws a prefix of its name: {prefix:?} of {name:?}"
            );
            if tab.rect().right() >= layout.toggle().x() - 2.0 * CELL - 0.01 {
                assert!(ellipsis, "the tab at the strip limit clips: {name:?}");
            }
        }
        let slot = layout.strip_brand().expect("brand slot");
        assert!(
            slot.cells() >= 2,
            "the brand keeps at least one char plus the ellipsis: {} cells",
            slot.cells()
        );
        assert_eq!(
            clipped_prefix(brand, slot.cells()),
            ("J", true),
            "the brand draws its first char plus the ellipsis at the minimum"
        );
        assert!(
            slot.rect().right() <= layout.panel().right() - 8.0 + 0.01,
            "the brand slot stays inside the panel: {:?}",
            slot.rect()
        );

        let fits = TestPaint::new(Some("T"), &["row"]).with_strip(
            "k",
            &["Statistics", "Options"],
            "Statistics",
            Some("ok"),
            Some("JiveCrafting"),
        );
        let layout = compute_layout(&fits);
        for (tab, name) in layout.strip_tabs().iter().zip(["Statistics", "Options"]) {
            assert_eq!(
                clipped_prefix(name, tab.cells()),
                (name, false),
                "a strip tab that fits draws whole"
            );
        }
        let slot = layout.strip_brand().expect("brand slot");
        assert_eq!(
            clipped_prefix("JiveCrafting", slot.cells()),
            ("JiveCrafting", false),
            "a brand that fits draws whole"
        );
    }

    /// A tabs band wider than the panel clips its last visible entry
    /// with an ellipsis at the body's right edge and drops the
    /// entries after it.
    #[test]
    fn tab_band_entries_clip_with_an_ellipsis_at_the_panel_edge() {
        let names = [
            "FirstLongTabName",
            "SecondLongTabName",
            "ThirdLongTab",
            "FourthLongTabName",
            "FifthLongTabName",
        ];
        let paint = TestPaint::new(Some("T"), &["row"]).with_tab_band("t", &names, names[0]);
        let layout = compute_layout(&paint);
        let tabs = layout.tabs();
        assert!(
            !tabs.is_empty() && tabs.len() < names.len(),
            "the band drops the entries past the panel: {tabs:?}"
        );
        for tab in tabs {
            assert!(
                tab.rect().right() <= layout.panel().right() - 8.0 + 0.01,
                "no entry passes the body's right edge: {:?}",
                tab.rect()
            );
        }
        let last = tabs.last().expect("a last visible entry");
        let (prefix, ellipsis) = clipped_prefix(names[last.name_index()], last.cells());
        assert!(
            ellipsis && names[last.name_index()].starts_with(prefix),
            "the last visible entry draws a prefix plus the ellipsis: {prefix:?}"
        );
    }

    /// The rail's text cells and text origin come from the layout,
    /// with the same epsilon rule as the button labels: at a 6.2 px
    /// cell the 62 px text area holds exactly ten cells, where a
    /// plain floor of the float quotient gives nine.
    #[test]
    fn rail_items_carry_their_text_cells_and_origin() {
        let paint = TestPaint::new(Some("T"), &["row"]).with_rail(
            "r",
            &["OverviewLongName", "Supplies"],
            "OverviewLongName",
        );
        let frame = paint.frame();
        let mut layout = PaintLayout::new();
        layout.compute(frame.view(), false, 6.2);
        let items = layout.rail();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].cells(), 10, "the rail text area holds ten cells");
        assert_eq!(
            clipped_prefix("OverviewLongName", items[0].cells()),
            ("OverviewL", true),
            "a long rail name clips with an ellipsis"
        );
        for item in items {
            let rect = item.rect();
            let origin = item.text_origin();
            assert_eq!(origin[0], rect.x() + 6.0, "the text starts 6 px in");
            assert_eq!(
                origin[1],
                rect.y() + rect.h() * 0.35,
                "the text top is 0.35 of the entry height"
            );
        }
    }

    /// The footer text sits in its 16 px row at the same 2 px offset
    /// as a body row's text, so its glyphs end at the panel bottom
    /// instead of running 2 px past it into the border.
    #[test]
    fn footer_text_stays_inside_the_panel_bottom_border() {
        let paint = TestPaint::new(Some("T"), &["row"]).with_footer("Jive scripts");
        let layout = compute_layout(&paint);
        let footer = layout.footer().expect("footer place");
        let panel_bottom = layout.panel().bottom();
        assert_eq!(
            footer.origin()[1],
            panel_bottom - 16.0 + crate::scripts::paint_style::TEXT_DY,
            "the footer text top is TEXT_DY below its row top"
        );
        assert_eq!(crate::scripts::paint_style::TEXT_DY, 2.0);
        // A body row's glyph line is the row height less TEXT_DY;
        // the footer's line therefore ends at the panel bottom.
        let row = layout.rows()[0].rect();
        assert_eq!(
            footer.origin()[1] + (row.h() - crate::scripts::paint_style::TEXT_DY),
            panel_bottom,
            "the footer glyph line ends at the panel bottom, inside the border"
        );
    }

    #[test]
    fn collapsed_layout_keeps_only_the_title_bar() {
        let paint = TestPaint::new(Some("T"), &["one"]).with_host_controls();
        let frame = paint.frame();
        let mut layout = PaintLayout::new();
        layout.compute(frame.view(), true, CELL);
        assert_eq!(layout.panel().h(), 20.0);
        assert!(layout.rows().is_empty());
        assert!(layout.buttons().is_empty());
        assert_eq!(layout.toggle().right(), layout.panel().right());
        assert_eq!(layout.toggle().w(), 20.0);
    }

    #[test]
    fn a_button_row_that_does_not_fit_wraps() {
        let mut paint = TestPaint::new(Some("T"), &[]);
        for i in 0..10 {
            paint = paint.with_button(&format!("b{i}"), "0123456789");
        }
        let layout = compute_layout(&paint);
        let buttons = layout.buttons();
        assert_eq!(buttons.len(), 10);
        let first_y = buttons[0].rect().y();
        assert!(
            buttons.iter().any(|b| b.rect().y() > first_y),
            "the flow wrapped to a second row"
        );
        for button in buttons {
            assert!(button.rect().right() <= layout.panel().right() - 8.0 + 0.01);
            assert_eq!(button.rect().h(), 16.0);
        }
        let wrapped = buttons
            .iter()
            .find(|b| b.rect().y() > first_y)
            .expect("a wrapped button");
        assert_eq!(wrapped.rect().y(), first_y + 20.0, "the row advance is 20");
        assert_eq!(
            wrapped.rect().x(),
            16.0,
            "a wrapped row starts at the inset"
        );
    }

    #[test]
    fn clipped_prefix_keeps_fitting_text_and_marks_longer_text() {
        assert_eq!(clipped_prefix("abc", 5), ("abc", false));
        assert_eq!(clipped_prefix("abc", 3), ("abc", false));
        assert_eq!(clipped_prefix("abcdef", 3), ("ab", true));
        assert_eq!(clipped_prefix("abcdef", 1), ("", true));
        assert_eq!(clipped_prefix("abcdef", 0), ("", false));
        assert_eq!(clipped_prefix("", 3), ("", false));
        // Cells count chars, not bytes.
        assert_eq!(clipped_prefix("aébc", 2), ("a", true));
    }
}
