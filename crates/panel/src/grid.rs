//! MultiBox grid-mode cell layout: the Game pane divided into equal slots,
//! one 765:503 applet fitted per slot.

/// Applet aspect (`client::APPLET_W` / `APPLET_H`), the shape every cell
/// keeps no matter how the pane is divided.
pub const CELL_ASPECT: f32 = 765.0 / 503.0;

/// Row-major cell rects `[x, y, w, h]` in Game-pane content space, one per
/// member. The pane is split into `cols` equal slots wide and
/// `ceil(n / cols)` rows tall; each cell is the largest 765:503 box that
/// fits its slot, centred in it. `cols` is chosen to maximise the cell
/// area. Empty when `n` is zero or the pane has no room.
pub fn grid_cells(n: usize, avail: [f32; 2]) -> Vec<[f32; 4]> {
    if n == 0 || avail[0] <= 0.0 || avail[1] <= 0.0 {
        return Vec::new();
    }
    let (aw, ah) = (avail[0], avail[1]);
    let mut best = (1usize, 0.0f32, 0.0f32); // (cols, cell_w, cell_h)
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let scale = ((aw / cols as f32) / 765.0).min((ah / rows as f32) / 503.0);
        let (w, h) = (765.0 * scale, 503.0 * scale);
        if w * h > best.1 * best.2 {
            best = (cols, w, h);
        }
    }
    let (cols, w, h) = best;
    let rows = n.div_ceil(cols);
    let (slot_w, slot_h) = (aw / cols as f32, ah / rows as f32);
    (0..n)
        .map(|i| {
            let (col, row) = (i % cols, i / cols);
            [
                col as f32 * slot_w + ((slot_w - w) * 0.5).max(0.0),
                row as f32 * slot_h + ((slot_h - h) * 0.5).max(0.0),
                w,
                h,
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{grid_cells, CELL_ASPECT};

    #[test]
    fn grid_cells_two_side_by_side_keep_aspect() {
        let cells = grid_cells(2, [800.0, 400.0]);
        assert_eq!(cells.len(), 2);
        let [_x0, _y0, w0, h0] = cells[0];
        assert!((w0 / h0 - 765.0 / 503.0).abs() < 0.02);
    }

    #[test]
    fn single_cell_fills_the_whole_pane() {
        let cells = grid_cells(1, [765.0, 503.0]);
        assert_eq!(cells, vec![[0.0, 0.0, 765.0, 503.0]]);
    }

    #[test]
    fn cells_fit_inside_the_pane_and_keep_aspect() {
        let cells = grid_cells(4, [800.0, 600.0]);
        assert_eq!(cells.len(), 4);
        for [x, y, w, h] in &cells {
            assert!(*x >= 0.0 && *y >= 0.0);
            assert!(x + w <= 800.0 + 0.01);
            assert!(y + h <= 600.0 + 0.01);
            assert!((w / h - CELL_ASPECT).abs() < 0.001);
        }
    }

    #[test]
    fn four_cells_in_a_wide_pane_are_two_by_two_row_major() {
        let cells = grid_cells(4, [800.0, 503.0]);
        // 2 cols × 2 rows: first row left then right, second row below.
        assert!(cells[1][0] > cells[0][0], "second cell right of first");
        assert_eq!(cells[1][1], cells[0][1], "same row");
        assert!(cells[2][1] > cells[0][1], "third cell below first row");
        assert_eq!(cells[2][0], cells[0][0], "left column again");
    }

    #[test]
    fn three_cells_flow_to_a_second_row() {
        let cells = grid_cells(3, [800.0, 400.0]);
        assert_eq!(cells.len(), 3);
        // 2 cols × 2 rows: the third cell is the left slot of row two.
        assert!(cells[2][1] > cells[0][1]);
        assert_eq!(cells[2][0], cells[0][0]);
    }

    #[test]
    fn no_members_or_no_room_yields_no_cells() {
        assert!(grid_cells(0, [800.0, 400.0]).is_empty());
        assert!(grid_cells(3, [0.0, 400.0]).is_empty());
    }

    #[test]
    fn tall_pane_stacks_cells_vertically() {
        let cells = grid_cells(2, [400.0, 800.0]);
        // 1 col × 2 rows: the second cell sits below the first.
        assert!(cells[1][1] > cells[0][1]);
        assert_eq!(cells[1][0], cells[0][0]);
    }

    #[test]
    fn same_sized_slots_share_each_cell_size() {
        let cells = grid_cells(6, [900.0, 600.0]);
        assert_eq!(cells.len(), 6);
        for c in &cells[1..] {
            assert_eq!(c[2], cells[0][2]);
            assert_eq!(c[3], cells[0][3]);
        }
    }
}
