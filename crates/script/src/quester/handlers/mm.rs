//! Monkey Madness hangar reinitialisation board (`x:mm:solve_reinitialisation`).
//!
//! The panel loc `bunker_controlpanal` opens interface `reinitialisation_puzzle`
//! (`quest_mm.rs2:638-641` -> `mm_puzzle.rs2:32-41`). Its `com_3` is a 5x5
//! inventory of 24 pieces and one gap, shuffled by 255 random gap moves
//! (`mm_puzzle.rs2:71-85`). The step starts with that board already open: the
//! guide opens it with an `interact` step on the panel, then this step reads
//! the board the client holds, plans the slides with the shared sliding-puzzle
//! solver (`api::clue_puzzle`) and clicks one piece per observed move. The
//! shuffle is never assumed: Glough's fee leaves it one move from solved
//! (`mm_puzzle.rs2:72-75`), and every plan starts from the board as read. The
//! step ends when the cutscene the last click starts has put the player on the
//! authored `arrival` tile (`mm_puzzle.rs2:190`, just before `mm_daero` is set).
use crate::interact::InteractReq;
use crate::native::{
    ActionContext, ActionError, ActionHandle, InteractionDispatch, NativeActions, NativeMachine,
};
use crate::quester::compile::{
    CompileContext, CompileError, StepContext, StepOutcome, StepPlan, StepRun,
};
use crate::quester::families::reach;
use api::clue_puzzle::{
    apply_puzzle_move, is_puzzle_solved, read_puzzle_board_by, solve_puzzle, Board, PuzzleRow,
    PUZZLE_BLANK_SLOT, PUZZLE_SIZE,
};
use api::WorldTile;
use serde::Deserialize;
use std::sync::Arc;
use std::task::Poll;

/// `mm_puzzle.rs2:144`: the line sent when the last piece lands, after
/// `if_close` (`:142`).
const SOLVED_MESSAGE: &str = "As you slide the final piece into place";
/// Ticks the board may take to show after the step starts. `mm_open_puzzle`
/// opens it in the same server tick as the panel's `Operate`
/// (`mm_puzzle.rs2:32-41`); the guide's `interact` step settles on that open.
const OPEN_TICKS: u64 = 5;
/// Ticks one click may take to show as a moved piece. `opheld5` runs in the
/// server tick after the packet and `inv_movetoslot` transmits in that tick
/// (`mm_puzzle.rs2:116-139`), so the move is seen one or two observed ticks
/// after the click; two more absorb a late frame.
const SETTLE_TICKS: u64 = 4;
/// Consecutive clicks that moved nothing (refused, dropped or never shown)
/// before the step fails and the runner retries it from a fresh read.
const STALL_LIMIT: u8 = 3;
/// Ticks a solved board may stay open. The completing click runs `if_close`
/// and sends the line in the same tick (`mm_puzzle.rs2:141-149`).
const SOLVED_TICKS: u64 = 3;
/// Moves one attempt may spend. The shuffle is 255 moves (`mm_puzzle.rs2:72`).
/// The planner's longest plan over 2000 such shuffles was 217 moves
/// (`plan_length_is_bounded_for_255_move_shuffles` re-checks 500 of them), so
/// this is more than twice the measured worst case.
const MAX_MOVES: u32 = 480;
/// Ticks the cutscene may take from the last click's line to the arrival
/// teleport. The timer fires 3 ticks after the line (`mm_puzzle.rs2:149`),
/// then `p_delay` 4, 3 and 7 pass (`:175`, `:181`, `:187`) before
/// `p_teleport` (`:190`) and `mm_daero` 6 (`:192`): 17 ticks; 34 is twice that.
const CUTSCENE_TICKS: u64 = 34;

/// Pieces are `<prefix>_01` .. `<prefix>_24`, piece `NN` belonging in slot
/// `NN - 1` (`mm_puzzle.rs2:87-114`, `reinit_puzzle_complete`).
const PIECES: usize = PUZZLE_BLANK_SLOT;

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    /// Object alias prefix of the 24 board pieces: `reinitialisation` names
    /// `reinitialisation_01` to `reinitialisation_24`.
    pieces: String,
    /// Where the cutscene leaves the player when it ends
    /// (`p_teleport(0_41_70_25_27)`, `mm_puzzle.rs2:190`); the step is done
    /// there, with the progress already set.
    arrival: ArrivalArg,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct ArrivalArg {
    tile: [i32; 3],
    /// Source citation for this authored tile.
    source: String,
}

pub(super) const STEPS: &[crate::quester::compile::StepHandler] =
    &[crate::quester::compile::step!(
        "x:mm:solve_reinitialisation",
        1,
        Explicit,
        Args,
        compile
    )];

pub(super) const FACTS: &[crate::quester::compile::PredicateHandler] = &[];

fn compile(args: Args, cx: &CompileContext<'_>) -> Result<Arc<dyn StepPlan>, CompileError> {
    let prefix = args.pieces.trim();
    if prefix.is_empty() {
        return Err(CompileError::code("invalid-args"));
    }
    if args.arrival.source.trim().is_empty() {
        return Err(CompileError::code("tile-source-required"));
    }
    let [x, z, level] = args.arrival.tile;
    if !(0..=16383).contains(&x) || !(0..=16383).contains(&z) || !(0..=3).contains(&level) {
        return Err(CompileError::code("invalid-tile"));
    }
    let mut pieces = Vec::with_capacity(PIECES);
    for number in 1..=PIECES {
        let item = cx
            .selected
            .item_by_alias(&format!("{prefix}_{number:02}"))
            .ok_or_else(|| CompileError::code("unresolved-item"))?;
        pieces.push((item.id, (number - 1) as u8));
    }
    Ok(Arc::new(Plan {
        pieces: pieces.into(),
        arrival: WorldTile { x, z, level },
    }))
}

struct Plan {
    pieces: Arc<[(i32, u8)]>,
    arrival: WorldTile,
}

impl StepPlan for Plan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(Run {
            action: Some(cx.tick.actions.begin::<Slide>(
                SlideArgs {
                    pieces: Arc::clone(&self.pieces),
                    arrival: self.arrival,
                },
                &mut cx.tick.cx,
            )?),
        }))
    }
}

struct Run {
    action: Option<ActionHandle<Slide>>,
}

impl StepRun for Run {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        let Some(handle) = &self.action else {
            return Poll::Ready(Err(ActionError::Cancelled));
        };
        cx.tick.actions.poll(handle, &mut cx.tick.cx).map(|result| {
            result.map(|()| StepOutcome {
                progress: None,
                evidence: cx.tick.cx.evidence(),
                receipt: None,
            })
        })
    }

    fn cancel(&mut self, actions: &mut NativeActions) {
        if let Some(handle) = self.action.take() {
            actions.cancel(handle);
        }
    }
}

/// One click in flight: the board it should produce and the tick it left.
struct Pending {
    request: u64,
    want: Board,
    sent: u64,
}

struct SlideArgs {
    pieces: Arc<[(i32, u8)]>,
    arrival: WorldTile,
}

struct Slide {
    pieces: Arc<[(i32, u8)]>,
    arrival: WorldTile,
    /// The tick the last piece's line was seen: the cutscene runs from it.
    solved_at: Option<u64>,
    chat_since: i32,
    started: u64,
    seen_open: bool,
    /// The tick the board was first unreadable or solved-but-open, for its bound.
    stuck_since: Option<u64>,
    pending: Option<Pending>,
    moved: u32,
    stalls: u8,
}

impl NativeMachine for Slide {
    type Args = SlideArgs;
    type Output = ();

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Ok(Self {
            pieces: args.pieces,
            arrival: args.arrival,
            solved_at: None,
            chat_since: reach::last_chat_seq(cx),
            started: cx.evidence().tick,
            seen_open: false,
            stuck_since: None,
            pending: None,
            moved: 0,
            stalls: 0,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<(), ActionError>> {
        let tick = cx.evidence().tick;
        if self.solved_at.is_none() && reach::saw_game_message(cx, self.chat_since, SOLVED_MESSAGE)
        {
            self.solved_at = Some(tick);
        }
        if let Some(at) = self.solved_at {
            return self.cutscene(cx, tick, at);
        }
        let Some(observed) = cx.snapshot().puzzle_board() else {
            return Poll::Pending;
        };
        let view = observed.value;
        if view.component_id < 0 || view.size != PUZZLE_SIZE as i32 {
            return self.closed(cx, tick);
        }
        self.seen_open = true;
        let rows: Vec<PuzzleRow> = view
            .items
            .iter()
            .map(|item| PuzzleRow {
                slot: item.slot,
                id: item.def.id,
            })
            .collect();
        let Some(live) = read_puzzle_board_by(&rows, |id| {
            self.pieces
                .iter()
                .find(|(piece, _)| *piece == id)
                .map(|(_, target)| *target)
        }) else {
            return self.stuck(tick, OPEN_TICKS, "reinitialisation board is not readable");
        };
        if let Some(pending) = &self.pending {
            match cx.interaction_dispatch(pending.request) {
                InteractionDispatch::Queued => return Poll::Pending,
                InteractionDispatch::Unsent => self.lost_click(),
                InteractionDispatch::Settled(receipt) if !receipt.accepted => self.lost_click(),
                InteractionDispatch::Settled(_) if pending.want == live => {
                    self.pending = None;
                    self.moved += 1;
                    self.stalls = 0;
                }
                InteractionDispatch::Settled(_) if tick < pending.sent + SETTLE_TICKS => {
                    return Poll::Pending;
                }
                InteractionDispatch::Settled(_) => self.lost_click(),
            }
        }
        if self.stalls >= STALL_LIMIT {
            return fail("reinitialisation clicks did not move a piece");
        }
        if self.moved >= MAX_MOVES {
            return fail("reinitialisation board was not solved within the move bound");
        }
        if is_puzzle_solved(&live) {
            return self.stuck(
                tick,
                SOLVED_TICKS,
                "reinitialisation board stayed open once solved",
            );
        }
        self.stuck_since = None;
        let Some(&slot) = solve_puzzle(&live).as_deref().and_then(<[usize]>::first) else {
            return fail("reinitialisation board has no plan");
        };
        let Some(row) = view.items.iter().find(|item| item.slot == slot as i32) else {
            return fail("reinitialisation plan names a slot without a piece");
        };
        let mut want = live;
        if !apply_puzzle_move(&mut want, slot) {
            return fail("reinitialisation plan made an illegal slide");
        }
        match cx.emit(InteractReq::PuzzleMove {
            id: row.def.id,
            slot: row.slot,
            component: view.component_id,
            generation: view.generation,
        }) {
            Ok(request) => {
                self.pending = Some(Pending {
                    request,
                    want,
                    sent: tick,
                });
                Poll::Pending
            }
            Err(ActionError::BudgetExhausted) => Poll::Pending,
            Err(error) => Poll::Ready(Err(error)),
        }
    }

    fn cancel(&mut self) {}
}

impl Slide {
    /// The click was refused, dropped or never produced its board: count it
    /// and plan again from the next read.
    fn lost_click(&mut self) {
        self.pending = None;
        self.stalls += 1;
    }

    /// No open 25-slot board. Before the first sight of it, wait for the
    /// open. After it, the board closing on a click whose result was the solved
    /// board is the completion (`if_close` in `mm_puzzle.rs2:141-142`); any
    /// other close ends the attempt.
    fn closed(&mut self, cx: &ActionContext<'_>, tick: u64) -> Poll<Result<(), ActionError>> {
        if !self.seen_open {
            if tick >= self.started + OPEN_TICKS {
                return fail("reinitialisation board never opened");
            }
            return Poll::Pending;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| is_puzzle_solved(&pending.want))
        {
            self.solved_at = Some(tick);
            return self.cutscene(cx, tick, tick);
        }
        fail("reinitialisation board closed before it was solved")
    }

    /// The last piece is in: the step is done when the cutscene has teleported
    /// the player to the authored arrival tile, in the tick `mm_daero` is set
    /// (`mm_puzzle.rs2:190-192`).
    fn cutscene(
        &self,
        cx: &ActionContext<'_>,
        tick: u64,
        at: u64,
    ) -> Poll<Result<(), ActionError>> {
        if cx
            .snapshot()
            .here()
            .is_some_and(|here| here.value == self.arrival)
        {
            return Poll::Ready(Ok(()));
        }
        if tick >= at + CUTSCENE_TICKS {
            return fail("reinitialisation cutscene did not reach its arrival tile");
        }
        Poll::Pending
    }

    /// A board that cannot be acted on yet: unreadable, or solved and still
    /// open. It is waited out for `bound` ticks, then fails.
    fn stuck(
        &mut self,
        tick: u64,
        bound: u64,
        what: &'static str,
    ) -> Poll<Result<(), ActionError>> {
        let since = *self.stuck_since.get_or_insert(tick);
        if tick >= since + bound {
            return fail(what);
        }
        Poll::Pending
    }
}

fn fail(reason: &'static str) -> Poll<Result<(), ActionError>> {
    Poll::Ready(Err(ActionError::Failed(Arc::from(reason))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::{ledger, HostEffect, InteractionReceipt};
    use crate::quester::families::tests::{local_player, with_tick};
    use api::obj_names::ItemDefView;
    use api::quest_progress::EvidenceStamp;
    use api::snapshot::{
        ChatLineView, GameSnapshot, ItemActionFamily, ItemContainer, ItemView, WidgetKind,
        WidgetRoot, WidgetView,
    };

    /// `interface.pack`: `reinitialisation_puzzle:com_3`.
    const BOARD: i32 = 11130;
    const ROOT: i32 = 11126;
    /// Test piece ids: piece `target` is object `PIECE_BASE + target`.
    const PIECE_BASE: i32 = 5000;
    /// `mm_puzzle.rs2:190`: `p_teleport(0_41_70_25_27)`.
    const ARRIVAL: WorldTile = WorldTile {
        x: 2649,
        z: 4507,
        level: 0,
    };
    const FINAL_LINE: &str =
        "As you slide the final piece into place you begin to hear a low rumbling";

    fn pieces() -> Arc<[(i32, u8)]> {
        (0..PIECES as u8)
            .map(|target| (PIECE_BASE + i32::from(target), target))
            .collect()
    }

    fn solved() -> Board {
        let mut board: Board = [None; PUZZLE_SIZE];
        for (slot, cell) in board.iter_mut().enumerate().take(PIECES) {
            *cell = Some(slot as u8);
        }
        board
    }

    /// The engine's shuffle: the gap takes `moves` random steps
    /// (`mm_puzzle.rs2:71-85`).
    fn shuffled(moves: u32, mut seed: u64) -> Board {
        let mut board = solved();
        let mut gap = PUZZLE_BLANK_SLOT;
        for _ in 0..moves {
            let mut next = Vec::new();
            if !gap.is_multiple_of(5) {
                next.push(gap - 1);
            }
            if gap % 5 != 4 {
                next.push(gap + 1);
            }
            if gap >= 5 {
                next.push(gap - 5);
            }
            if gap <= 19 {
                next.push(gap + 5);
            }
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let step = next[(seed >> 33) as usize % next.len()];
            assert!(apply_puzzle_move(&mut board, step));
            gap = step;
        }
        board
    }

    fn widget(items: Vec<ItemView>) -> WidgetView {
        WidgetView {
            kind: WidgetKind::Widget,
            component_id: BOARD,
            layer_id: ROOT,
            parent_id: ROOT,
            root_component_id: ROOT,
            root: WidgetRoot::Main,
            type_: 2,
            button_type: 0,
            client_code: 0,
            x: 0,
            y: 0,
            width: 5,
            height: 5,
            scroll_height: 0,
            inventory_ready: false,
            scroll_position: 0,
            hidden: false,
            text: None,
            alternate_text: None,
            button_text: None,
            target_verb: None,
            target_base: None,
            target_mask: 0,
            model_type: 0,
            model_id: -1,
            alternate_model_type: 0,
            alternate_model_id: -1,
            scripts: None,
            script_comparators: None,
            script_operands: None,
            varp_bindings: Vec::new(),
            colour: 0,
            actions: Vec::new(),
            items,
        }
    }

    fn item(slot: usize, target: u8) -> ItemView {
        ItemView {
            def: ItemDefView {
                id: PIECE_BASE + i32::from(target),
                name: Some("Sliding button".into()),
                stackable: false,
                members: true,
                base_value: 0,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: ItemContainer::Widget,
            action_family: ItemActionFamily::Held,
            slot: slot as i32,
            count: 1,
            actions: Vec::new(),
            component_id: BOARD,
        }
    }

    /// The client's view of the open board: the main modal, its rows and the
    /// identified board component.
    fn open_snapshot(board: &Board, chat: &[&str]) -> GameSnapshot {
        let items = board
            .iter()
            .enumerate()
            .filter_map(|(slot, cell)| cell.map(|target| item(slot, target)))
            .collect();
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_main_modal(ROOT, vec![widget(items)]);
        snapshot.seed_puzzle_board(BOARD, PUZZLE_SIZE as i32, 7);
        seed_chat(&mut snapshot, chat);
        snapshot
    }

    /// The board closed; `player` is where the client sees the player.
    fn closed_snapshot(chat: &[&str], player: Option<WorldTile>) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_main_modal(-1, Vec::new());
        snapshot.seed_puzzle_board(-1, 0, 8);
        if let Some(tile) = player {
            snapshot.seed_local_player(local_player(tile));
        }
        seed_chat(&mut snapshot, chat);
        snapshot
    }

    fn seed_chat(snapshot: &mut GameSnapshot, chat: &[&str]) {
        snapshot.seed_chat_lines(
            chat.iter()
                .enumerate()
                .map(|(index, text)| ChatLineView {
                    type_: 0,
                    username: None,
                    text: (*text).to_owned(),
                    sequence: index as i32 + 1,
                })
                .collect(),
        );
    }

    type Ledger = Option<Box<ledger::Ledger>>;

    fn answer(ledger: &mut Ledger, tick: u64, accepted: bool) -> InteractReq {
        let ledger = ledger.as_mut().expect("ledger");
        let action = ledger.outbox.remove(0);
        ledger.complete_interaction(
            &action.authority(),
            InteractionReceipt {
                request_id: action.authority().request_id().get(),
                evidence: EvidenceStamp {
                    run: action.authority().run(),
                    tick,
                    sequence: tick,
                },
                accepted,
                chat_since: 0,
            },
        );
        let HostEffect::Interaction(request) = action.effect else {
            panic!("the board step only emits interactions");
        };
        request
    }

    fn poll(
        snapshot: &GameSnapshot,
        ledger: &mut Ledger,
        tick: u64,
        handle: &crate::native::ActionHandle<Slide>,
    ) -> Poll<Result<(), ActionError>> {
        with_tick(snapshot, ledger, tick, |native| {
            native.actions.poll(handle, &mut native.cx)
        })
    }

    fn begin(snapshot: &GameSnapshot, ledger: &mut Ledger) -> crate::native::ActionHandle<Slide> {
        with_tick(snapshot, ledger, 1, |native| {
            native
                .actions
                .begin::<Slide>(
                    SlideArgs {
                        pieces: pieces(),
                        arrival: ARRIVAL,
                    },
                    &mut native.cx,
                )
                .expect("begin")
        })
    }

    /// How the simulated engine ends the run after the completing click.
    #[derive(Clone, Copy)]
    struct Ending {
        /// The line `mm_puzzle.rs2:144` sends is seen.
        line: bool,
        /// The cutscene teleports the player to the arrival tile.
        arrives: bool,
    }

    /// Ticks from the completing click to the arrival teleport in the
    /// simulation: more than the 17 the content takes would only slow the test.
    const CUTSCENE_SIM: u64 = 17;

    /// Drive the machine against a simulated engine until it finishes: each
    /// accepted click moves the piece into the gap the tick after (the
    /// `opheld5` rule, `mm_puzzle.rs2:116-139`); the completing click closes
    /// the board and sends the line (`:141-144`); the cutscene teleports the
    /// player `CUTSCENE_SIM` ticks later (`:190`). Returns the click count and
    /// the ticks from the completing click to the result.
    fn play(start: Board, ending: Ending) -> (Poll<Result<(), ActionError>>, u32, u64) {
        let mut board = start;
        let mut ledger: Ledger = None;
        let handle = begin(&open_snapshot(&board, &[]), &mut ledger);
        let mut clicks = 0;
        let mut snapshot = open_snapshot(&board, &[]);
        let mut solved_at: Option<u64> = None;
        for tick in 2..4_000u64 {
            if let Some(at) = solved_at {
                let player = (ending.arrives && tick >= at + CUTSCENE_SIM).then_some(ARRIVAL);
                snapshot = closed_snapshot(
                    if ending.line { &[FINAL_LINE] } else { &[] },
                    Some(player.unwrap_or(WorldTile {
                        x: 2394,
                        z: 9883,
                        level: 0,
                    })),
                );
            }
            let result = poll(&snapshot, &mut ledger, tick, &handle);
            if !result.is_pending() {
                let waited = solved_at.map_or(0, |at| tick - at);
                if solved_at.is_some() && ending.arrives {
                    assert!(waited >= CUTSCENE_SIM, "finished before the cutscene ended");
                }
                return (result, clicks, waited);
            }
            let queued = ledger.as_ref().is_some_and(|l| !l.outbox.is_empty());
            if queued {
                let InteractReq::PuzzleMove {
                    id,
                    slot,
                    component,
                    generation,
                } = answer(&mut ledger, tick, true)
                else {
                    panic!("the board step must click through puzzle-move");
                };
                assert_eq!((component, generation), (BOARD, 7));
                let slot = slot as usize;
                assert_eq!(
                    board[slot].map(|target| PIECE_BASE + i32::from(target)),
                    Some(id),
                    "the click names the piece standing on its slot"
                );
                assert!(apply_puzzle_move(&mut board, slot), "illegal slide");
                clicks += 1;
                if is_puzzle_solved(&board) {
                    solved_at = Some(tick);
                } else {
                    snapshot = open_snapshot(&board, &[]);
                }
            }
        }
        panic!("the board step never finished");
    }

    const FULL: Ending = Ending {
        line: true,
        arrives: true,
    };

    #[test]
    fn solves_a_255_move_shuffle_one_observed_move_at_a_time() {
        if !api::test_support::game_data_available() {
            return;
        }
        let start = shuffled(255, 0x2545_F491_4F6C_DD1D);
        assert!(!is_puzzle_solved(&start));
        let (result, clicks, _) = play(start, FULL);
        assert!(matches!(result, Poll::Ready(Ok(()))), "{result:?}");
        assert!(clicks > 0 && clicks <= MAX_MOVES);
    }

    /// Glough's fee leaves the board one move from solved
    /// (`mm_puzzle.rs2:72-75`): the plan is read from the board, so it is one
    /// click.
    #[test]
    fn a_board_one_move_from_solved_takes_one_click() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (result, clicks, _) = play(shuffled(1, 99), FULL);
        assert!(matches!(result, Poll::Ready(Ok(()))), "{result:?}");
        assert_eq!(clicks, 1);
    }

    /// The board closing on a solved click is enough to start the cutscene
    /// wait when the line was missed; the step still ends only on arrival.
    #[test]
    fn the_cutscene_wait_starts_from_the_closed_solved_board_and_ends_on_arrival() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (result, clicks, waited) = play(
            shuffled(1, 3),
            Ending {
                line: false,
                arrives: true,
            },
        );
        assert!(matches!(result, Poll::Ready(Ok(()))), "{result:?}");
        assert_eq!(clicks, 1);
        assert_eq!(waited, CUTSCENE_SIM);
    }

    /// A cutscene that never reaches the arrival tile fails at its bound
    /// instead of waiting forever.
    #[test]
    fn a_cutscene_that_never_arrives_fails_at_its_bound() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (result, _, waited) = play(
            shuffled(1, 4),
            Ending {
                line: true,
                arrives: false,
            },
        );
        assert!(
            matches!(result, Poll::Ready(Err(ActionError::Failed(_)))),
            "{result:?}"
        );
        // The machine sees the closed board one tick after the click.
        assert_eq!(waited, CUTSCENE_TICKS + 1);
    }

    #[test]
    fn plan_length_is_bounded_for_255_move_shuffles() {
        let mut longest = 0;
        for seed in 0..500u64 {
            let board = shuffled(255, seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let plan = solve_puzzle(&board).expect("a shuffled board is solvable");
            longest = longest.max(plan.len());
        }
        assert!(
            longest as u32 * 2 <= MAX_MOVES,
            "MAX_MOVES {MAX_MOVES} must be at least twice the longest plan {longest}"
        );
    }

    #[test]
    fn clicks_that_move_nothing_fail_the_step_after_the_stall_limit() {
        if !api::test_support::game_data_available() {
            return;
        }
        let board = shuffled(30, 5);
        let snapshot = open_snapshot(&board, &[]);
        let mut ledger: Ledger = None;
        let handle = begin(&snapshot, &mut ledger);
        let mut tick = 2;
        for _ in 0..STALL_LIMIT {
            assert!(poll(&snapshot, &mut ledger, tick, &handle).is_pending());
            answer(&mut ledger, tick, true);
            // The board never changes: wait out the settle bound.
            tick += SETTLE_TICKS;
            assert!(poll(&snapshot, &mut ledger, tick - 1, &handle).is_pending());
            if ledger.as_ref().is_some_and(|l| !l.outbox.is_empty()) {
                panic!("a click went out before its settle bound");
            }
            tick += 1;
        }
        assert!(matches!(
            poll(&snapshot, &mut ledger, tick, &handle),
            Poll::Ready(Err(ActionError::Failed(_)))
        ));
    }

    #[test]
    fn a_refused_click_counts_as_a_stall_and_is_replanned() {
        if !api::test_support::game_data_available() {
            return;
        }
        let board = shuffled(30, 6);
        let snapshot = open_snapshot(&board, &[]);
        let mut ledger: Ledger = None;
        let handle = begin(&snapshot, &mut ledger);
        assert!(poll(&snapshot, &mut ledger, 2, &handle).is_pending());
        answer(&mut ledger, 2, false);
        assert!(poll(&snapshot, &mut ledger, 3, &handle).is_pending());
        assert!(
            ledger.as_ref().is_some_and(|l| l.outbox.len() == 1),
            "the refused click is planned again from the next read"
        );
    }

    #[test]
    fn a_board_that_never_opens_or_closes_early_fails() {
        if !api::test_support::game_data_available() {
            return;
        }
        let snapshot = closed_snapshot(&[], None);
        let mut ledger: Ledger = None;
        let handle = begin(&snapshot, &mut ledger);
        assert!(poll(&snapshot, &mut ledger, 2, &handle).is_pending());
        assert!(matches!(
            poll(&snapshot, &mut ledger, 1 + OPEN_TICKS, &handle),
            Poll::Ready(Err(ActionError::Failed(_)))
        ));

        let board = shuffled(30, 7);
        let open = open_snapshot(&board, &[]);
        let mut ledger: Ledger = None;
        let handle = begin(&open, &mut ledger);
        assert!(poll(&open, &mut ledger, 2, &handle).is_pending());
        assert!(matches!(
            poll(&closed_snapshot(&[], None), &mut ledger, 3, &handle),
            Poll::Ready(Err(ActionError::Failed(_)))
        ));
    }

    #[test]
    fn a_board_the_piece_map_cannot_read_is_waited_out_then_fails() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut board = shuffled(30, 8);
        let gap = board.iter().position(Option::is_none).expect("gap");
        let other = (0..PUZZLE_SIZE).find(|slot| *slot != gap).expect("piece");
        board[gap] = board[other];
        let snapshot = open_snapshot(&board, &[]);
        let mut ledger: Ledger = None;
        let handle = begin(&snapshot, &mut ledger);
        assert!(poll(&snapshot, &mut ledger, 2, &handle).is_pending());
        assert!(matches!(
            poll(&snapshot, &mut ledger, 2 + OPEN_TICKS, &handle),
            Poll::Ready(Err(ActionError::Failed(_)))
        ));
        assert!(ledger.as_ref().is_none_or(|l| l.outbox.is_empty()));
    }
}
