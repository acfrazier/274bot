use super::*;


/// The identified row's own puzzle box: the selected `{alias}_puzzlebox` item
/// the frozen `solveHeld` is handed by id, together with the display name the
/// Open resolves.
///
/// The join is the row's own alias, so a held box this row does not name is
/// never its box, and a desc-only riddle without such an item has no puzzle
/// step at all. The name is that item's selected display name (`Puzzle box`)
/// — what the host resolves by first name match, never the alias and never an
/// item id.
pub(super) fn puzzle_box<'a>(
    selected: Option<&'a SelectedGameData>,
    row: &TrailMembershipRow,
) -> Option<(i32, &'a str)> {
    let item = selected?.item_by_alias(&format!("{}_puzzlebox", row.alias))?;
    Some((item.id, item.name.as_deref()?))
}

/// This call's posted board, as the wrapper marshalled
/// `snapshot.puzzle_board` and the generation beside it. `None` when the page
/// posted no board object at all: the frozen `boardNow()` cannot read one
/// either, and no board is invented.
pub(super) fn posted_board(input: &Value) -> Option<PostedBoard> {
    let page = input.get("puzzle_board")?;
    let component_id = page.get("component_id").and_then(i32_of)?;
    let size = page.get("size").and_then(i32_of)?;
    let rows = page.get("items")?.as_array()?;
    let mut pieces = Vec::with_capacity(rows.len());
    for row in rows {
        let (Some(slot), Some(id)) = (
            row.get("slot").and_then(i32_of),
            row.get("id").and_then(i32_of),
        ) else {
            // A row without a posted slot or id is not a piece this page
            // carries, exactly like a posted `(id, count)` row that is not a
            // pair: it is skipped, never guessed at.
            continue;
        };
        pieces.push(PuzzleRow { slot, id });
    }
    Some(PostedBoard {
        component_id,
        size,
        rows: pieces,
        generation: input.get("puzzle_board_generation").and_then(Value::as_u64),
    })
}

/// The frozen `boardNow()`: this call's board is readable only when the
/// observed slot count is the frozen 25 and its rows fill the board's 24
/// pieces around one gap. A closed board posts `component_id -1` with no rows,
/// and a page whose pieces the selected map cannot place reads as unreadable
/// rather than as a partial board — SNAP's own observation is never filled to
/// 25 here.
pub(super) fn live_board(selected: Option<&SelectedGameData>, page: &PostedBoard) -> Option<Board> {
    if page.component_id < 0 || page.size != clue_puzzle::PUZZLE_SIZE as i32 {
        return None;
    }
    clue_puzzle::read_puzzle_board(&page.rows, selected)
}
impl ClueRuntime {

    /// Whether this call runs the held puzzle box's own arm or falls through to
    /// the `Steady` arms below it: the row's own box is on this call's page — or
    /// this token already opened that board — and this step's solved-or-
    /// attempted latch is not set.
    ///
    /// The latch is the re-talk. The attempt is over — solved, stalled,
    /// unreadable, past `MAX_MOVES`, or a board that never opened — so nothing
    /// is opened or closed again while this step stays held and the row's own
    /// selected families decide what follows: for the nine desc-only puzzle
    /// riddles the selected `talk_key.talk` family publishes that is the landed
    /// talk step, and every other latched row idles. `closing` stays true after
    /// the frozen close window ends and is never the exclusion here: a call
    /// still inside that window enters the arm and waits it out.
    pub(super) fn puzzle_arm(&self, id: i32, input: &Value) -> bool {
        match self.puzzle.as_ref().filter(|puzzle| puzzle.id == id) {
            Some(puzzle) => !puzzle.latch && (holds(input, id) || puzzle.opened),
            None => holds(input, id),
        }
    }

    /// `Steady` on an identified row whose own selected `{alias}_puzzlebox` is
    /// held on this call's page — or whose board this token already opened:
    /// the frozen `PuzzleBox.solveHeld` run, one verb per call.
    ///
    /// The frozen sequence is followed step for step, with each call its own
    /// loop iteration: the held box is opened while the board stays closed, a
    /// readable board is read and planned, one `puzzle-move` goes out for the
    /// plan's own first slot, and the next call re-reads the board — a sent
    /// click is not an observed move, so the board the click was expected to
    /// produce is what the settle reads and the leftover plan is never walked.
    /// Every board the caller hands in is this call's marshalling of
    /// `host().snapshot.puzzle_board`; nothing about it is cached.
    pub(super) fn puzzle(
        &mut self,
        box_id: i32,
        name: &str,
        input: &Value,
        selected: Option<&SelectedGameData>,
    ) -> Value {
        if self.puzzle.as_ref().is_some_and(|puzzle| puzzle.latch) {
            // The arm's own latch guard: the `Steady` dispatch above already
            // falls through to `steady()` once the latch is set, so this is
            // never the re-talk — it only keeps this arm from opening or
            // closing a board it is done with while the step stays held.
            return self.emit("wait");
        }
        let Some(page) = posted_board(input) else {
            // No posted board page at all: the frozen `boardNow()` cannot read
            // one either, so the held box is opened again — or the attempt is
            // over — exactly as a closed board is.
            return self.puzzle_closed(box_id, name);
        };
        let Some(live) = live_board(selected, &page) else {
            return self.puzzle_closed(box_id, name);
        };
        {
            let state = self.puzzle.get_or_insert_with(|| Puzzle::new(box_id));
            state.opened = true;
        }
        if self.puzzle.as_ref().is_some_and(|puzzle| puzzle.closing) {
            // The one close is out and this call's board is still posted.
            return self.puzzle_closing();
        }
        if clue_puzzle::is_puzzle_solved(&live) {
            // The frozen `isPuzzleSolved` read over the reconstructed board:
            // never the posted row count and never the board generation, which
            // only says whether this page is the session in hand.
            return self.puzzle_close();
        }
        let outstanding = self.puzzle.as_ref().and_then(|puzzle| puzzle.want);
        if let Some(want) = outstanding {
            if live == want {
                // Landed: the live board is the one the click was expected to
                // produce, so the frozen loop's own counters advance and this
                // call plans again from that board.
                if let Some(state) = self.puzzle.as_mut() {
                    state.want = None;
                    state.moved += 1;
                    state.stall = 0;
                }
            } else if self.clock.bound_reached() {
                // The settle bound ran out with the board unmoved: the click
                // was refused or the engine dropped the stale slot.
                return self.puzzle_stall();
            } else {
                // Sent is not observed: wait, and read the board again next
                // call rather than replaying anything.
                return self.emit("wait");
            }
        }
        let (moved, stall) = self
            .puzzle
            .as_ref()
            .map(|puzzle| (puzzle.moved, puzzle.stall))
            .unwrap_or_default();
        if moved >= MAX_MOVES || stall >= STALL_LIMIT {
            // The frozen loop's own condition, read before the next plan.
            return self.puzzle_close();
        }
        let Some(plan) = clue_puzzle::solve_puzzle(&live) else {
            // Unsolvable as read: a mixed picture set, a plan that does not
            // converge. The frozen branch retries from a fresh read and counts
            // a refusal.
            return self.puzzle_stall();
        };
        let Some(&slot) = plan.first() else {
            return self.puzzle_stall();
        };
        // The posted row the click rides: its own id, the slot it sits in and
        // the posted component, with this call's board generation. A slot the
        // page did not post a piece on is the frozen `clickPiece` refusal.
        let Some(row) = page.rows.iter().find(|row| row.slot == slot as i32) else {
            return self.puzzle_stall();
        };
        let Some(generation) = page.generation else {
            // The page did not post the board's session: no click is sent on
            // an invented one, and the identity the host checks stays whole.
            return self.emit("wait");
        };
        let mut want = live;
        if !clue_puzzle::apply_puzzle_move(&mut want, slot) {
            return self.puzzle_stall();
        }
        if let Some(state) = self.puzzle.as_mut() {
            state.want = Some(want);
        }
        self.clock.arm(MOVE_SETTLE_MS);
        json!({
            "kind": "puzzle-move",
            "token": self.token,
            "id": row.id,
            "slot": row.slot,
            "component": page.component_id,
            "generation": generation,
        })
    }

    /// This call's page has no readable board, which is the frozen
    /// `boardNow() === null`.
    ///
    /// Either the Open has not landed yet — the held box is opened again,
    /// repeating while the board stays closed, until the frozen open window
    /// runs out — or the attempt is over: this token had already opened a live
    /// board, and the frozen `finally` closes the modal that Open landed on,
    /// through the same one close as a stall or `MAX_MOVES`. A close that is
    /// already out is never sent twice, so the closed `{-1, 0, []}` page a
    /// landed close leaves only waits the close window out.
    pub(super) fn puzzle_closed(&mut self, box_id: i32, name: &str) -> Value {
        let (opened, closing) = {
            let state = self.puzzle.get_or_insert_with(|| Puzzle::new(box_id));
            (state.opened, state.closing)
        };
        if closing {
            // The one close is already out and this call's page posts no
            // readable board — the shape the landed close itself leaves, or a
            // board that followed it away.
            return self.puzzle_closing();
        }
        if opened {
            // A live board went unreadable mid-solve with no close out yet: the
            // modal the Open landed on may still be up, so the attempt exits
            // through the close rather than latching with the modal open.
            return self.puzzle_close();
        }
        if self.clock.deadline.is_none() {
            // The first Open of this attempt arms the frozen window; the
            // repeats are page-driven and do not extend it.
            self.clock.arm(OPEN_WAIT_MS);
        }
        if self.clock.bound_reached() {
            // `puzzle box did not open`: the attempt ends without a close,
            // because no board was ever opened.
            if let Some(state) = self.puzzle.as_mut() {
                state.latch = true;
            }
            return self.emit("wait");
        }
        json!({
            "kind": "held",
            "token": self.token,
            "name": name,
            "action": OPEN,
        })
    }

    /// One refused click, one settle bound that ran out unlanded, one board the
    /// frozen solver has no plan for: the frozen loop's own `stalled++`, and
    /// the exit close once it reaches the frozen limit.
    pub(super) fn puzzle_stall(&mut self) -> Value {
        let limit = match self.puzzle.as_mut() {
            Some(state) => {
                state.want = None;
                state.stall += 1;
                state.stall >= STALL_LIMIT
            }
            None => false,
        };
        if limit {
            return self.puzzle_close();
        }
        self.emit("wait")
    }

    /// The close-on-exit of a live board: one `close-modal`, then the frozen
    /// close window. Solved, stalled, unreadable and past `MAX_MOVES` are all
    /// this same exit, and this arm never sends a second close for the step.
    pub(super) fn puzzle_close(&mut self) -> Value {
        if let Some(state) = self.puzzle.as_mut() {
            state.closing = true;
        }
        self.clock.arm(CLOSE_WAIT_MS);
        self.emit("close-modal")
    }

    /// The exit close is out and the frozen close window is running out: no
    /// second close is sent for the step, and the solved-or-attempted latch
    /// lands when the window ends. Whether this call's page still posts the
    /// board or none at all is not this window's question.
    pub(super) fn puzzle_closing(&mut self) -> Value {
        if self.clock.bound_reached() {
            if let Some(state) = self.puzzle.as_mut() {
                state.latch = true;
            }
        }
        self.emit("wait")
    }
}
