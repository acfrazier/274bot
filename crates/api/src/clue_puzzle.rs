//! Sliding-puzzle plan: the frozen grouped BFS over a board the caller
//! reconstructed from the posted page, and the frozen gap fill the
//! reconstruction is made of. Pure over the caller's own page rows and the
//! selected piece map: nothing here reads `host().snapshot`, an inventory, a
//! loc page or `identify_step`, so a missing page is not an error of either
//! read.
//!
//! Frozen source of truth: `puzzleLogic.ts` (`GROUPS`, `NEIGHBOURS`,
//! `puzzleNeighbours`, `isPuzzleSolved`, `isValidBoard`, `applyPuzzleMove`,
//! `readPuzzleBoard`, `search`, `solvePuzzle`) in
//! `experiments/rs2b0t/src/bot/api/ai/clues/`. A board cell holds the **target
//! slot** of the piece sitting in it, or `None` for the gap, exactly as the
//! frozen `PuzzleBoard` does. The frozen `PuzzleBox.ts` sequencing
//! (`boardNow`, `MOVE_SETTLE_MS`, `MAX_MOVES`, `STALL_LIMIT`, the close) is the
//! caller's own arm, not this module's.
//!
//! The frozen `PUZZLE_PIECE_SLOT` table is **not** copied: the three 24-piece
//! runs are read off the selected items' own aliases, so the map is a
//! derivation from the pin rather than a second table beside it. A board whose
//! pieces are a mixture of two runs has duplicate targets and is
//! unsolvable-as-read, the way frozen `isValidBoard` rejects it.
//!
//! The V8 wrapper is the only production caller.

use crate::game_data::SelectedGameData;

/// Frozen `PUZZLE_WIDTH * PUZZLE_WIDTH`: the slots in one board.
pub const PUZZLE_SIZE: usize = 25;

/// Frozen `PUZZLE_SIZE - 1`: the slot the gap has to end on.
pub const PUZZLE_BLANK_SLOT: usize = 24;

/// Frozen groups: cells are placed in these batches and each batch is frozen
/// once solved. Frozen `search` state is one group's piece slots plus the gap,
/// so the batch that carries a row's last two cells — or the final 3×3 — is
/// what lets the search find the rotation that frees them instead of a
/// hardcoded escape sequence.
const GROUPS: [&[u8]; 14] = [
    &[0],
    &[1],
    &[2],
    &[3, 4],
    &[5],
    &[10],
    &[15, 20],
    &[6],
    &[7],
    &[8, 9],
    &[11],
    &[16, 21],
    &[12, 13, 14],
    &[17, 18, 19, 22, 23],
];

/// The widest frozen group, and so the longest piece list one search encodes.
const MAX_GROUP: usize = 5;

/// Frozen fail-closed search bound: a group's own state space is at most
/// `P(22, 3) = 9240` states (the `[3, 4]` batch against 22 unfrozen cells), so
/// a search that would visit more than this is not a live board's group and
/// returns `None` instead of running away. The bound is per group and the
/// caller's call is what it runs on.
const GROUP_VISIT_CAP: usize = 16_384;

/// The selected piece alias prefix: the three 24-piece runs are
/// `trail_slidingpuzzleb01`–`b24`, `c01`–`c24` and `d01`–`d24`, one run per
/// picture set, each numbering the board's target slots `1`–`24`.
const PIECE_ALIAS_PREFIX: &str = "trail_slidingpuzzle";

/// One posted board row: the slot the piece sits in and the piece's own item
/// id — the frozen `{ slot, id }` page the interface stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PuzzleRow {
    pub slot: i32,
    pub id: i32,
}

/// A reconstructed board: slot → the target slot of the piece standing there,
/// or `None` for the gap. The frozen `PuzzleBoard` of the same length.
pub type Board = [Option<u8>; PUZZLE_SIZE];

/// The frozen `NEIGHBOURS`: a full board is never a distance away from
/// wrapping, so the left and right neighbours of a row's edge cells simply do
/// not exist. `NONE` pads a cell that has fewer than four.
const NONE: u8 = u8::MAX;

/// Every slot's 4-adjacent neighbours in the frozen order — left, right, up,
/// down — with no wrap across a row's edge.
const NEIGHBOURS: [[u8; 4]; PUZZLE_SIZE] = build_neighbours();

const fn build_neighbours() -> [[u8; 4]; PUZZLE_SIZE] {
    let mut out = [[NONE; 4]; PUZZLE_SIZE];
    let mut slot = 0;
    while slot < PUZZLE_SIZE {
        let col = slot % 5;
        let row = slot / 5;
        let mut next = 0;
        if col > 0 {
            out[slot][next] = (slot - 1) as u8;
            next += 1;
        }
        if col < 4 {
            out[slot][next] = (slot + 1) as u8;
            next += 1;
        }
        if row > 0 {
            out[slot][next] = (slot - 5) as u8;
            next += 1;
        }
        if row < 4 {
            out[slot][next] = (slot + 5) as u8;
        }
        slot += 1;
    }
    out
}

/// `slot`'s frozen neighbours, in the frozen order. A slot off the board has
/// none: the caller never indexes a neighbour it did not read.
fn neighbours(slot: usize) -> impl Iterator<Item = usize> {
    NEIGHBOURS
        .get(slot)
        .copied()
        .unwrap_or([NONE; 4])
        .into_iter()
        .filter(|neighbour| *neighbour != NONE)
        .map(usize::from)
}

/// The selected piece map: which board target slot one posted piece id belongs
/// to, or `None` when this pin holds no such piece.
///
/// The frozen `PUZZLE_PIECE_SLOT` is not pasted here. Every piece item of the
/// three runs is named `trail_slidingpuzzle<set><NN>`, so its target slot is
/// `NN - 1` — the same derivation for all three sets, and the only one: an id
/// the selected items do not hold, an item that is not a piece, and an alias
/// outside the three runs all have no slot rather than a guessed one.
pub fn slot_of_piece(selected: Option<&SelectedGameData>, id: i32) -> Option<u8> {
    let alias = selected?.item_by_id(id)?.alias.as_deref()?;
    let rest = alias.strip_prefix(PIECE_ALIAS_PREFIX)?;
    let target = match rest.as_bytes() {
        [b'b' | b'c' | b'd', tens, ones] if tens.is_ascii_digit() && ones.is_ascii_digit() => {
            (tens - b'0') * 10 + (ones - b'0')
        }
        _ => return None,
    };
    if !(1..=PUZZLE_BLANK_SLOT as u8).contains(&target) {
        return None;
    }
    Some(target - 1)
}

/// The frozen `readPuzzleBoard`: the posted sparse page plus the piece map
/// filled into a board, or `None` until the page carries all 24 pieces and
/// leaves exactly one slot empty.
///
/// A row whose piece the map cannot place, a row off the board, and a second
/// row for a slot this page already filled are all the same refusal — the read
/// fails closed rather than filling a board the interface did not post. The
/// gap is the slot no row named; the page itself is never padded to 25.
pub fn read_puzzle_board(rows: &[PuzzleRow], selected: Option<&SelectedGameData>) -> Option<Board> {
    let mut board: Board = [None; PUZZLE_SIZE];
    let mut filled = 0usize;
    for row in rows {
        let target = slot_of_piece(selected, row.id)?;
        let slot = usize::try_from(row.slot).ok()?;
        if slot >= PUZZLE_SIZE || board[slot].is_some() {
            return None;
        }
        board[slot] = Some(target);
        filled += 1;
    }
    (filled == PUZZLE_BLANK_SLOT).then_some(board)
}

/// The frozen `isPuzzleSolved`: every slot holds the piece that belongs in it
/// and the gap sits on `PUZZLE_BLANK_SLOT`.
///
/// This is the reconstructed board's own predicate — never the observed slot
/// count, never the posted row count and never the board session generation,
/// which says only whether the page is the session the caller is reading.
pub fn is_puzzle_solved(board: &Board) -> bool {
    board
        .iter()
        .enumerate()
        .all(|(slot, cell)| *cell == (slot != PUZZLE_BLANK_SLOT).then_some(slot as u8))
}

/// The frozen `isValidBoard`: 24 distinct targets and exactly one gap. A mixed
/// picture set has two pieces for one target and none for another, so it is
/// not a valid board and no plan is invented for it.
fn valid_board(board: &Board) -> bool {
    let mut seen = [false; PUZZLE_BLANK_SLOT];
    let mut gaps = 0usize;
    for cell in board {
        let Some(target) = *cell else {
            gaps += 1;
            continue;
        };
        let target = usize::from(target);
        if target >= PUZZLE_BLANK_SLOT || seen[target] {
            return false;
        }
        seen[target] = true;
    }
    gaps == 1 && seen.iter().all(|seen| *seen)
}

/// The frozen `applyPuzzleMove`: slide the piece standing on `slot` into the
/// gap beside it, and report whether that was a legal move. Only a 4-adjacent
/// non-wrapping neighbour that is the gap takes the piece, so a click is never
/// a move across the board's edge.
pub fn apply_puzzle_move(board: &mut Board, slot: usize) -> bool {
    if board.get(slot).copied().flatten().is_none() {
        return false;
    }
    let Some(gap) = neighbours(slot).find(|neighbour| board[*neighbour].is_none()) else {
        return false;
    };
    board[gap] = board[slot];
    board[slot] = None;
    true
}

/// The frozen `solvePuzzle`: the slots to click, in order, or `None` when the
/// board is not a valid one or a group of it cannot be placed at all. A solved
/// board is `Some(vec![])`.
///
/// Each frozen group is searched on its own — its pieces' slots and the gap,
/// with every already-placed cell skipped — and frozen once its leg lands, so
/// a later leg can never disturb a solved batch. The plan is the legs
/// concatenated; the caller clicks its first slot and replans from the live
/// board rather than walking the rest out.
pub fn solve_puzzle(board: &Board) -> Option<Vec<usize>> {
    if !valid_board(board) {
        return None;
    }
    let mut work = *board;
    let mut frozen = [false; PUZZLE_SIZE];
    let mut moves = Vec::new();
    for group in GROUPS {
        let mut pieces = [0u8; MAX_GROUP];
        for (index, target) in group.iter().enumerate() {
            // A valid board holds every target exactly once, so this is the
            // slot the piece belonging on `target` now stands in.
            pieces[index] = work.iter().position(|cell| *cell == Some(*target))? as u8;
        }
        let gap = work.iter().position(Option::is_none)? as u8;
        let leg = search(&pieces[..group.len()], gap, group, &frozen)?;
        for slot in &leg {
            apply_puzzle_move(&mut work, *slot);
        }
        for target in group {
            frozen[usize::from(*target)] = true;
        }
        moves.extend(leg);
    }
    is_puzzle_solved(&work).then_some(moves)
}

/// The frozen per-group BFS over `gap` followed by each of the group's piece
/// slots, base 25, so one number names one state.
///
/// A step slides the piece beside the gap into it, which is the click the
/// caller dispatches; a goal state has every group piece on its own target and
/// the gap outside the group. Frozen cells are skipped, so a batch the board
/// already placed is never disturbed. The breadth-first order is what keeps
/// the leg short — the first goal reached is a shortest one — and the caller
/// only ever clicks `leg[0]`.
fn search(
    start: &[u8],
    start_gap: u8,
    group: &[u8],
    frozen: &[bool; PUZZLE_SIZE],
) -> Option<Vec<usize>> {
    let encode = |pieces: &[u8], gap: u8| {
        let mut key = u64::from(gap);
        for piece in pieces {
            key = key * PUZZLE_SIZE as u64 + u64::from(*piece);
        }
        key
    };
    let decode = |mut key: u64| {
        let mut pieces = [0u8; MAX_GROUP];
        for index in (0..start.len()).rev() {
            pieces[index] = (key % PUZZLE_SIZE as u64) as u8;
            key /= PUZZLE_SIZE as u64;
        }
        (pieces, key as u8)
    };
    let done = |pieces: &[u8], gap: u8| {
        !group.contains(&gap)
            && pieces
                .iter()
                .zip(group)
                .all(|(piece, target)| piece == target)
    };

    if done(start, start_gap) {
        return Some(Vec::new());
    }

    let start_key = encode(start, start_gap);
    // state → the state it came from and the click that got there. The start's
    // own move is never read: the walk back stops when it reaches that key.
    let mut from: std::collections::HashMap<u64, (u64, u8)> =
        std::collections::HashMap::with_capacity(64);
    from.insert(start_key, (start_key, NONE));
    let mut frontier = vec![start_key];
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for key in frontier {
            let (pieces, gap) = decode(key);
            for step in neighbours(usize::from(gap)) {
                if frozen[step] {
                    continue;
                }
                let mut pieces = pieces;
                if let Some(index) = pieces.iter().position(|piece| usize::from(*piece) == step) {
                    pieces[index] = gap;
                }
                let reached = encode(&pieces[..start.len()], step as u8);
                if from.contains_key(&reached) {
                    continue;
                }
                if from.len() >= GROUP_VISIT_CAP {
                    // Fail closed: a group this large is not a live board's.
                    return None;
                }
                from.insert(reached, (key, step as u8));
                if done(&pieces[..start.len()], step as u8) {
                    let mut leg = Vec::new();
                    let mut cursor = reached;
                    while cursor != start_key {
                        let (previous, click) = from[&cursor];
                        leg.push(usize::from(click));
                        cursor = previous;
                    }
                    leg.reverse();
                    return Some(leg);
                }
                next.push(reached);
            }
        }
        frontier = next;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::ClientRevision;
    use std::sync::Arc;

    const PINS: [ClientRevision; 2] = [ClientRevision::R274, ClientRevision::R289];

    fn data(revision: ClientRevision) -> Arc<SelectedGameData> {
        crate::game_data::for_revision(revision).expect("selected data")
    }

    /// The solved board: the piece belonging in `slot` stands there, and the
    /// gap sits on the frozen blank slot.
    fn solved() -> Board {
        let mut board: Board = [None; PUZZLE_SIZE];
        for (slot, cell) in board.iter_mut().enumerate() {
            *cell = (slot != PUZZLE_BLANK_SLOT).then_some(slot as u8);
        }
        board
    }

    /// The selected id of the piece belonging on `target`, found through the
    /// same derivation the read uses — never a pasted piece id.
    fn piece_for(data: &SelectedGameData, target: usize) -> i32 {
        data.items()
            .iter()
            .find(|item| slot_of_piece(Some(data), item.id) == Some(target as u8))
            .unwrap_or_else(|| panic!("piece for slot {target}"))
            .id
    }

    /// One board as the posted sparse page: a row per filled slot, in slot
    /// order, carrying the piece id belonging on that cell's target.
    fn sparse(data: &SelectedGameData, board: &Board) -> Vec<PuzzleRow> {
        let mut rows = Vec::new();
        for (slot, cell) in board.iter().enumerate() {
            if let Some(target) = *cell {
                rows.push(PuzzleRow {
                    slot: slot as i32,
                    id: piece_for(data, usize::from(target)),
                });
            }
        }
        rows
    }

    /// The engine's own shuffle: `moves` legal slides from the solved board,
    /// each one stepping the gap to a non-wrapping neighbour. The frozen
    /// mechanics prove every board that shuffle can reach is solvable.
    fn shuffled(moves: u32) -> Board {
        let mut board = solved();
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut gap = PUZZLE_BLANK_SLOT;
        for _ in 0..moves {
            let slots: Vec<usize> = neighbours(gap).collect();
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let step = slots[(seed >> 33) as usize % slots.len()];
            assert!(apply_puzzle_move(&mut board, step));
            gap = step;
        }
        board
    }

    /// Whether every click of a leg is a legal slide when walked in order.
    fn replays(board: &Board, plan: &[usize]) -> bool {
        let mut work = *board;
        plan.iter().all(|slot| apply_puzzle_move(&mut work, *slot)) && is_puzzle_solved(&work)
    }

    #[test]
    fn a_solved_board_is_solved_and_plans_nothing() {
        let board = solved();
        assert!(is_puzzle_solved(&board));
        assert_eq!(solve_puzzle(&board), Some(Vec::new()));
    }

    #[test]
    fn one_slide_from_solved_plans_that_one_slide() {
        // The piece belonging on 23 stands on the blank slot and the gap is
        // beside it: one click puts it back.
        let mut board = solved();
        board[PUZZLE_BLANK_SLOT] = Some(23);
        board[23] = None;
        assert!(!is_puzzle_solved(&board));
        let plan = solve_puzzle(&board).expect("one-move board");
        assert_eq!(plan, vec![24]);
        assert!(replays(&board, &plan));
    }

    #[test]
    fn a_board_read_from_the_posted_page_needs_all_but_one_slot() {
        for revision in PINS {
            let data = data(revision);
            let board = solved();
            let rows = sparse(&data, &board);
            assert_eq!(rows.len(), PUZZLE_BLANK_SLOT);
            assert_eq!(read_puzzle_board(&rows, Some(&data)), Some(board));

            // One piece short: the page has no board yet.
            assert_eq!(read_puzzle_board(&rows[..23], Some(&data)), None);
            // No row at all: not a board, and never a padded one.
            assert_eq!(read_puzzle_board(&[], Some(&data)), None);
            // Two rows for one slot: the page contradicts itself.
            let mut doubled = rows.clone();
            doubled[1].slot = doubled[0].slot;
            assert_eq!(read_puzzle_board(&doubled, Some(&data)), None);
            // A row off the board.
            let mut off_board = rows.clone();
            off_board[0].slot = PUZZLE_SIZE as i32;
            assert_eq!(read_puzzle_board(&off_board, Some(&data)), None);
            let mut negative = rows.clone();
            negative[0].slot = -1;
            assert_eq!(read_puzzle_board(&negative, Some(&data)), None);
            // A piece the selected map cannot place: the board is not read at
            // all rather than filled with a guessed target.
            let mut unknown = rows.clone();
            unknown[0].id = -1;
            assert_eq!(read_puzzle_board(&unknown, Some(&data)), None);
            // No selected pin at all: every piece is unplaceable.
            assert_eq!(read_puzzle_board(&rows, None), None);
        }
    }

    #[test]
    fn the_piece_map_is_the_selected_three_runs() {
        for revision in PINS {
            let data = data(revision);
            let mut placed = 0;
            for item in data.items() {
                let alias = item.alias.as_deref().unwrap_or_default();
                let Some(rest) = alias.strip_prefix(PIECE_ALIAS_PREFIX) else {
                    // Every other selected item has no slot at all: the puzzle
                    // boxes among them.
                    assert_eq!(slot_of_piece(Some(&data), item.id), None, "{alias}");
                    continue;
                };
                let target: u8 = rest[1..].parse().expect("piece number");
                assert_eq!(
                    slot_of_piece(Some(&data), item.id),
                    Some(target - 1),
                    "{alias}"
                );
                placed += 1;
            }
            // Three sets of 24: every target slot is covered exactly three
            // times, one per picture set.
            assert_eq!(placed, 72);
            // The one puzzle box of this family, an id the pin does not hold,
            // and no pin at all: no piece, so no slot.
            let box_id = data
                .item_by_alias("trail_clue_hard_riddle014_puzzlebox")
                .expect("box item")
                .id;
            assert_eq!(slot_of_piece(Some(&data), box_id), None);
            assert_eq!(slot_of_piece(Some(&data), i32::MAX), None);
            assert_eq!(slot_of_piece(None, 2749), None);
        }
    }

    #[test]
    fn a_mixed_picture_set_is_unsolvable_as_read() {
        let data = data(ClientRevision::R274);
        let board = solved();
        let rows = sparse(&data, &board);
        // The piece belonging on 5 is posted a second time in place of the one
        // belonging on 6: two pieces for one target, none for another.
        let mut mixed = rows;
        let duplicate = mixed[5];
        let other = mixed[6];
        mixed[6] = PuzzleRow {
            slot: other.slot,
            id: duplicate.id,
        };
        let board = read_puzzle_board(&mixed, Some(&data)).expect("the read itself fills");
        assert!(!is_puzzle_solved(&board));
        assert_eq!(solve_puzzle(&board), None);
    }

    #[test]
    fn a_click_never_wraps_a_row_and_never_moves_the_gap() {
        // The gap on the end of the first row and a piece on the start of the
        // second: the two slots touch in the array and not on the board. The
        // board stays a placement of the same 24 pieces, one gap.
        let mut board = solved();
        board[3] = None;
        board[PUZZLE_BLANK_SLOT] = Some(3);
        assert!(!apply_puzzle_move(&mut board, 5), "5 is not beside 3");
        assert!(!apply_puzzle_move(&mut board, 0), "0 is not beside 3");
        assert_eq!(board[4], Some(4), "a refused click moves nothing");
        assert_eq!(board[5], Some(5));
        // The slot beside the gap takes it, and the piece's own slot empties.
        assert!(apply_puzzle_move(&mut board, 2));
        assert_eq!(board[3], Some(2));
        assert_eq!(board[2], None);
        // The gap is not a piece, and a slot off the board is not a slot.
        assert!(!apply_puzzle_move(&mut board, 2));
        assert!(!apply_puzzle_move(&mut board, PUZZLE_SIZE));
        // The last row's edge pair is the same shape: 19 and 20 touch only
        // downwards.
        let mut edge = solved();
        edge[18] = None;
        edge[PUZZLE_BLANK_SLOT] = Some(18);
        assert!(!apply_puzzle_move(&mut edge, 20), "20 is not beside 18");
        assert!(
            !apply_puzzle_move(&mut edge, PUZZLE_BLANK_SLOT),
            "24 is not beside 18"
        );
        assert!(apply_puzzle_move(&mut edge, 23), "23 is beside 18");
        assert_eq!(edge[18], Some(23));
        assert_eq!(edge[23], None);
    }

    #[test]
    fn the_grouped_plan_replays_to_solved_from_a_shuffled_board() {
        let data = data(ClientRevision::R274);
        for moves in [1u32, 2, 5, 17, 41, 101, 101] {
            let board = shuffled(moves);
            let plan = solve_puzzle(&board).expect("a shuffled board is solvable");
            if is_puzzle_solved(&board) {
                assert!(plan.is_empty(), "{moves} moves: {plan:?}");
                continue;
            }
            assert!(!plan.is_empty(), "{moves} moves");
            assert!(replays(&board, &plan), "{moves} moves: {plan:?}");
            // The plan is the caller's own page again once read back.
            let rows = sparse(&data, &board);
            let read = read_puzzle_board(&rows, Some(&data)).expect("read back");
            assert_eq!(read, board);
            let again = solve_puzzle(&read).expect("read back plan");
            assert!(replays(&read, &again));
        }
    }

    #[test]
    fn every_picture_set_is_read_and_solved() {
        // Each of the three runs numbers the same 24 targets, so a board built
        // from any one of them reads and solves — and a board that takes one
        // cell from another run is the mixed set no plan exists for.
        let data = data(ClientRevision::R274);
        let board = shuffled(23);
        for base in [2749, 3619, 3643] {
            let rows: Vec<PuzzleRow> = board
                .iter()
                .enumerate()
                .filter_map(|(slot, cell)| {
                    (*cell).map(|target| PuzzleRow {
                        slot: slot as i32,
                        id: base + i32::from(target),
                    })
                })
                .collect();
            assert_eq!(read_puzzle_board(&rows, Some(&data)), Some(board));
            let plan = solve_puzzle(&board).expect("one set solves");
            assert!(replays(&board, &plan), "{base}: {plan:?}");

            // One cell taken from another run with a different number: two
            // pieces for one target and none for another, so the read fills a
            // board with no plan at all.
            let mut mixed_rows = rows;
            mixed_rows[0].id = if base == 2749 { 3619 } else { 2749 } + 5;
            let mixed = read_puzzle_board(&mixed_rows, Some(&data)).expect("the read itself fills");
            assert_ne!(mixed, board);
            assert!(!is_puzzle_solved(&mixed));
            assert_eq!(solve_puzzle(&mixed), None, "{base} mixed");
        }
    }
}
