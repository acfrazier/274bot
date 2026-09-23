//! Isolate-owned clue-session machine skeleton: `api.clue.begin` / `next`.
//!
//! One token per isolate over the landed held-step identify. The JS wrapper
//! owns the page reads — the posted `snapshot.inv` `(id, count)` page, the
//! posted `hold || ours` signal, the wrapper's `lifecycleGeneration` — and
//! hands them in; this machine owns the token, the generation captured at
//! begin, the frozen clock, the identify call, the callback kinds and the
//! idle end state.
//!
//! `api::clue_logic::identify_step` over the current posted page and the
//! selected `trails()` family is the only membership decision, in its own
//! order: `missing-selected-data`, then `family-unavailable:trails`, then
//! `none-held`, then the landed row. A family absence is not `none-held`. A
//! `none-held` begin is refused with no live token — the caller begins again
//! after the pickup — and a live session that loses its held membership
//! errors `none-held` and aborts.
//!
//! This is the search, casket-open, unguarded-dig and trail-end collect slice
//! and nothing else: no talk, guardian, puzzle, deposit, retry or
//! return-grind. A held row that is a selected search membership — a selected
//! `trail_loc=^true` **and** a decodable selected `trail_coord` on the same
//! row — walks to its decoded tile and then dispatches the Search/Open picker
//! over the posted loc page; both verbs are enqueued by the wrapper as
//! `InteractReq::Walk` / `InteractReq::Loc`. The picker is the frozen one,
//! minus its `walkLeg`: nearest then action rank, always at the row's own
//! posted tile and id.
//!
//! The sibling of that pin is the unguarded dig: a decodable selected
//! `trail_coord` on a row with no `trail_loc`, `trail_sextant=yes` and no
//! `trail_guardian`, whose `access` is not `"constrained"`. It walks the same
//! way — the same decoder, the same radius, the same posted `here` — and then
//! dispatches the generic held step with the selected item display `Spade` and
//! the frozen `Dig`. The Sextant/Watch/Chart trio is membership input only:
//! never required, never waited for and never acquired. A pack that does not
//! post the `Spade` is a `wait`, never an `abandon` and never a public
//! `no-spade` token. Dig repeats while that same clue stays held, a produced
//! casket Opens instead, and a `none-held` right after a Dig still aborts:
//! Collecting is the casket Open's alone.
//!
//! A held `role: "casket"` row is the held casket item: once its own report is
//! posted it dispatches the generic held step — the selected item display name
//! joined by the row's own id, and the frozen `Open` — enqueued by the wrapper
//! as `InteractReq::Held` the way the journal enqueues its modal clicks. It
//! repeats while that same casket id stays held, and a different held row
//! re-arms the gate. The name is the identity the host resolves by first name
//! match: never the row alias, never an item id, and never a scan of the item
//! table.
//!
//! That Open is also the trail-end seam. The alias' own `_hard_` is captured
//! while the row is still in hand, and the first call that identifies
//! `none-held` after the casket left — `Steady` on that same step — is the
//! **Collecting** phase: the token lives on and the loot is dispatched one
//! verb per call. A posted main modal is closed while its own page says it is
//! open, the casket's same-tile ground overflow is Taken by its posted name,
//! and a full pack Drops one frozen food row before that Take. Identify is
//! still made on every Collecting call, so a next scroll — or a leftover
//! casket — leaves the phase and re-arms the landed gate. Collecting is only
//! ever reached from `Steady` on a casket whose Open went out, and every other
//! `none-held` (begin, gate, report, search, idle) still aborts. Collect
//! finishes with that same `none-held` abort — never `status: "done"`, never
//! `'clue solved'`, never `abandon` — and it idles until the frozen reward
//! window has passed when its pages are still empty.
//!
//! Every other held step — the packed 3554 `access: "constrained"` clue, the
//! guarded rows whose first Dig would spawn a wizard, the coord-only map
//! rows, the desc-only key-gated riddles and the empty-params 2722 — is
//! identified and then idled: no action and no walk.
//! Identify is casket-first, so a casket held beside its own clue is the Open
//! and never 3554 play. Yield keeps the token live, so it is not trail
//! completion, and this machine never returns `status: "done"`, never
//! restores gear, and never emits the exact `'clue solved'` string.
//!
//! One token per isolate. A second begin, reset and stop abort the live token
//! and emit no verb for it. Pause and hold freeze this machine's own clock,
//! so a frozen call emits no callback, no walk, no loc and no held, and does
//! not advance the session. `on_snapshot` is fan-out only: begin and next
//! read the pages the wrapper hands in at call time — the parked page, this
//! call's `here` tile and its posted loc page — so nothing is cached here and
//! there is no world copy.

use crate::food_policy::food_forms_for;
use crate::isolate_fb::SnapshotReader;
use crate::task_clock::InstantTaskClock;
use api::clue_logic::{identify_step, NONE_HELD};
use api::clue_pack::SHARK_ID;
use api::game_data::{SelectedGameData, TrailMembershipRow};
use serde_json::{json, Value};
use std::cell::RefCell;

/// No selected pin. The same public token the landed V8 held-step wrapper
/// publishes: the machine refuses with it rather than calling a page empty.
const MISSING_SELECTED_DATA: &str = "missing-selected-data";

/// The token is not this machine's live one.
const STALE: &str = "stale";

/// The live session was aborted out from under the token by the generation
/// bump (reset / stop). The token is dead and nothing is emitted for it.
const ABORTED: &str = "aborted";

/// The frozen `REWARD_WAIT_MS`: the window the reward interface and the first
/// loot row have to appear after the Open. Armed once on Collecting entry and
/// never on the Open, which is page-driven and repeats while the casket is
/// held. Freeze keeps it paused, so the bound only ever runs in live time.
const COLLECT_WAIT_MS: u64 = 2_000;

/// The frozen `casketObj.includes('_hard_')`: the one hard-trail signal the
/// reward arm reads. The same case-sensitive marker `casket_reward_slots`
/// tests, and it is captured from the live alias while the row is still in
/// hand — identify goes `none-held` the moment the server takes the casket.
const HARD_MARKER: &str = "_hard_";

/// The frozen `collectReward` ground action: the posted op a ground row must
/// list before its Take is dispatched.
const TAKE: &str = "Take";

/// The frozen `collectReward` pack-full action: one Dropped food row frees the
/// slot the next Take needs.
const DROP: &str = "Drop";

/// The frozen `food.js` option names, carried as the **keys** handed to the
/// landed `food_policy::food_forms_for` and for nothing else. Not one of these
/// names is ever compared to a posted display name — the selected forms the
/// helper returns are what decide, exactly the shim's `isFoodItem` — so this is
/// the landed policy's own input list and never a second food table.
const FOOD_OPTION_KEYS: [&str; 25] = [
    "Shark",
    "Lobster",
    "Swordfish",
    "Tuna",
    "Salmon",
    "Trout",
    "Pike",
    "Bass",
    "Herring",
    "Sardine",
    "Anchovies",
    "Shrimps",
    "Cooked meat",
    "Cooked chicken",
    "Bread",
    "Stew",
    "Cake",
    "Chocolate cake",
    "Plain pizza",
    "Meat pizza",
    "Anchovy pizza",
    "Pineapple pizza",
    "Redberry pie",
    "Meat pie",
    "Apple pie",
];

thread_local! {
    static RUNTIME: RefCell<ClueRuntime> = const { RefCell::new(ClueRuntime::new()) };
}

/// The live session's phase. `Idle` is the only phase without a token, so a
/// refused begin leaves it in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No token.
    Idle,
    /// A held step was identified and the fresh `enabled` read is unanswered.
    Gate,
    /// Enabled: the progress status line is not posted yet.
    Reporting,
    /// Progress posted: the same step stays held. A held casket dispatches
    /// its Open from here, a search row walks then dispatches the picker, and
    /// an unguarded-dig row walks then Digs with the held Spade; every other
    /// row idles with no action and no walk, and no second callback is
    /// emitted for this step.
    Steady,
    /// Trail-end collect: `Steady` on the step whose casket Open went out, and
    /// this call's identify is `none-held`. The one phase that survives
    /// `none-held` — the token lives and the loot verbs are dispatched from
    /// here, one per call, until the same `none-held` abort ends the session.
    Collecting,
}

struct ClueRuntime {
    clock: InstantTaskClock,
    token: u64,
    phase: Phase,
    /// The wrapper's `lifecycleGeneration` captured at begin. Nothing else
    /// about the live session is captured: `enabled`, the page and the
    /// `hold || ours` signal are re-read at call time.
    generation: u64,
    /// The membership row this phase belongs to: a different held row
    /// re-arms the gate, so the enabled read is never reused across steps.
    step_id: i32,
    /// The casket Open dispatched for the step in hand: `None` until `Steady`
    /// emits it, `Some(hard)` after, with `hard` the live alias' own
    /// `contains("_hard_")`. It is both the collect's entry permit and its
    /// food policy, and it is dropped on every re-arm and abort.
    open: Option<bool>,
    /// The Collecting discarded ground ids: the frozen `SHARK_ID` plus every
    /// id this collect Dropped, so a Take never picks the food the Drop just
    /// put on the floor back up. Session state, never a world copy.
    discarded: Vec<i32>,
    /// The Take dispatched on the last Collecting call, waiting for its posted
    /// ground row to leave. The frozen `took '…' from the casket` line is that
    /// observation, one call later.
    pending_take: Option<Take>,
    /// The frozen `pack-full-no-food` WARNING is logged: the collect is over
    /// (not `done`) and the next Collecting call is the `none-held` abort.
    full_no_food: bool,
}

impl ClueRuntime {
    const fn new() -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token: 0,
            phase: Phase::Idle,
            generation: 0,
            step_id: 0,
            open: None,
            discarded: Vec::new(),
            pending_take: None,
            full_no_food: false,
        }
    }

    /// Abort keeps the pause/hold freeze (`InstantTaskClock` contract: abort
    /// clears the deadline only) and emits nothing.
    fn abort(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Idle;
        self.generation = 0;
        self.step_id = 0;
        self.clear_step();
    }

    /// The step-scoped state a re-arm, a leave and an abort all drop: the
    /// dispatched Open with its `hard` capture, the collect deadline, the
    /// discarded ground ids and the settled-Take watch.
    fn clear_step(&mut self) {
        self.open = None;
        self.clock.deadline = None;
        self.discarded.clear();
        self.pending_take = None;
        self.full_no_food = false;
    }

    /// Whether this token survives an identify `none-held`. Only the collect
    /// arm does: the phase itself, or the `Steady` step whose casket Open was
    /// already dispatched. The gate, the report, a search step, an idle token
    /// and every begin still error `none-held` and abort.
    fn collects(&self) -> bool {
        self.phase == Phase::Collecting || (self.phase == Phase::Steady && self.open.is_some())
    }

    /// The `hard` capture the Collecting Drop reads: the casket alias'
    /// `contains("_hard_")`, taken at Open.
    fn hard(&self) -> bool {
        self.open.is_some_and(|hard| hard)
    }

    /// Enter the trail-end collect: `arm` the one frozen reward window here
    /// and nowhere else, and start the discarded set at the frozen `SHARK_ID`.
    fn enter_collect(&mut self) {
        if self.phase == Phase::Collecting {
            return;
        }
        self.phase = Phase::Collecting;
        self.clock.arm(COLLECT_WAIT_MS);
        self.discarded.clear();
        self.discarded.push(SHARK_ID);
    }

    fn aborted(&mut self, reason: &str) -> Value {
        self.abort();
        json!({ "kind": "aborted", "token": self.token, "reason": reason })
    }

    fn emit(&self, kind: &str) -> Value {
        json!({ "kind": kind, "token": self.token })
    }

    fn begin(&mut self, selected: Option<&SelectedGameData>, input: &Value) -> Value {
        // A second begin aborts the live token first and emits nothing for
        // it: begin has no continue kind, and a refused begin is still a
        // refusal, not a resumed session.
        self.abort();
        let row = match identify(selected, input) {
            Ok(row) => row,
            Err(reason) => return self.aborted(reason),
        };
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Gate;
        // Strictly captured: `next` requires the same generation to be
        // posted again, so a wrapper that stops sending it fails closed.
        self.generation = input.get("generation").and_then(Value::as_u64).unwrap_or(0);
        self.step_id = row.id;
        json!({ "kind": "token", "token": self.token })
    }

    fn next(&mut self, selected: Option<&SelectedGameData>, input: &Value) -> Value {
        let Some(token) = input.get("token").and_then(Value::as_u64) else {
            return self.aborted(STALE);
        };
        if token != self.token || self.phase == Phase::Idle {
            return self.aborted(STALE);
        }
        if input.get("generation").and_then(Value::as_u64) != Some(self.generation) {
            // Reset, stop and a second begin abort silently; the first thing
            // the dead token hears about it is this error, never a kind.
            return self.aborted(ABORTED);
        }
        if self.clock.frozen() {
            // Frozen: no callback, no verb and no burn. `resume` is not
            // consumed, so the gate is still unanswered after the thaw.
            return self.emit("wait");
        }
        if input.get("hold").and_then(Value::as_bool).unwrap_or(false) {
            // The posted `hold || ours` cooperative interrupt. The token
            // lives and the step is not trail completion.
            return self.emit("yield");
        }
        // One identify per call, and the same landed `identify_step` the
        // machine always made: membership is never re-derived here. The only
        // seam is `none-held`, which a Collecting token — or the Steady step
        // whose casket Open already went out — survives.
        let row = match identify(selected, input) {
            Ok(row) => Some(row),
            Err(NONE_HELD) if self.collects() => None,
            Err(reason) => return self.aborted(reason),
        };
        let Some(row) = row else {
            self.enter_collect();
            return self.collect(selected, input);
        };
        if self.step_id != row.id {
            // A different step is held: the previous progress and its
            // `enabled` answer belong to the old row, so this call re-asks
            // and a `resume` it carried is not the new step's answer. The
            // previous step's Open and collect state go with it.
            self.clear_step();
            self.step_id = row.id;
            self.phase = Phase::Gate;
            return self.emit("callback.enabled");
        }
        match self.phase {
            Phase::Idle => self.aborted(STALE),
            Phase::Gate => match input.get("resume").and_then(Value::as_bool) {
                // The callback return. False means do not execute: the
                // session idles with its token live, and the following gate
                // re-reads instead of replaying this answer.
                Some(false) => self.emit("wait"),
                Some(true) => {
                    self.phase = Phase::Reporting;
                    json!({
                        "kind": "callback.log",
                        "token": self.token,
                        "message": progress(row),
                    })
                }
                None => self.emit("callback.enabled"),
            },
            Phase::Reporting => {
                self.phase = Phase::Steady;
                json!({
                    "kind": "callback.setStatus",
                    "token": self.token,
                    "message": status(row),
                })
            }
            Phase::Steady => match casket_name(selected, row) {
                // The held casket's own item, opened by name. Repeats while
                // this same casket id stays held: the host fails a verb whose
                // item is already gone, and the next call re-reads the page.
                // This is also where the collect's `hard` capture happens:
                // the alias is gone once the server takes the casket.
                Some(name) => {
                    self.open = Some(row.alias.contains(HARD_MARKER));
                    json!({
                        "kind": "held",
                        "token": self.token,
                        "name": name,
                        "action": OPEN,
                    })
                }
                // Not a held casket: the landed search dispatch, the sibling
                // unguarded-dig dispatch, or the idle every other row keeps.
                None => self.steady(row, input),
            },
            Phase::Collecting => {
                // Identity still holds a step: the next scroll, or a leftover
                // casket. The collect is skipped — its own deadline, discard
                // set and settled-Take watch with it — and the landed gate
                // re-arms for the row, whose Open, search or dig follows as
                // usual.
                self.clear_step();
                self.phase = Phase::Gate;
                self.step_id = row.id;
                self.emit("callback.enabled")
            }
        }
    }

    /// One Collecting tick: the trail-end loot over this call's posted pages,
    /// one verb per call.
    ///
    /// Precedence inside the arm: the logged `pack-full-no-food` finish, the
    /// settled Take line, the posted main modal, the same-tile posted ground,
    /// then the pack-full food Drop. Nothing here is cached — `ground`,
    /// `here`, `inv`, `inv_size` and `main_modal_id` are this call's own
    /// marshalling of `host().snapshot` — and nothing is invented: a page that
    /// did not post a slot is a `wait`, and only a posted fact dispatches a
    /// verb. The frozen `28` is not a fallback for a missing `inv_size`, and
    /// `6960` is not a second open-modal definition.
    fn collect(&mut self, selected: Option<&SelectedGameData>, input: &Value) -> Value {
        if self.full_no_food {
            // The WARNING was the last thing this collect had to say.
            return self.aborted(NONE_HELD);
        }
        if let Some(take) = self.settled_take(input) {
            return json!({
                "kind": "callback.log",
                "token": self.token,
                "message": took_line(&take),
            });
        }
        // The reward interface: close while this call's page posts an open
        // main modal. A posted `-1` is closed and an omitted slot was never
        // posted at all, so neither is a close; the id is never compared to
        // another constant and never read from the journal's machine.
        if input
            .get("main_modal_id")
            .and_then(i32_of)
            .is_some_and(|id| id != -1)
        {
            return self.emit("close-modal");
        }
        let Some(here) = input.get("here").and_then(posted_tile) else {
            // No posted tile: no ground row can be claimed as same-tile, so
            // this call has nothing to close and nothing to take.
            return self.empty();
        };
        let Some(drop) = pick_ground(input, here, &self.discarded) else {
            return self.empty();
        };
        // The pack's own numbers: occupied positive-count rows against the
        // posted slot count. A page that posted no `inv_size` has not said how
        // full the pack is, and this machine does not invent the client's 28
        // for it — it waits for the page instead.
        let Some(size) = input.get("inv_size").and_then(i32_of) else {
            return self.emit("wait");
        };
        if occupied(input) < i64::from(size) {
            return self.take(&drop);
        }
        let Some((id, name)) = drop_row(selected, input, self.hard()) else {
            // The frozen `pack-full-no-food` WARNING: nothing in the pack may
            // be Dropped for the row that is waiting. Log it, finish the
            // collect, and let the next call be the landed abort.
            self.full_no_food = true;
            return json!({
                "kind": "callback.log",
                "token": self.token,
                "message": pack_full_warning(drop.name, self.hard()),
            });
        };
        // One Drop per call, then the Take next call. The dropped id joins the
        // discarded set, so the scan does not pick the food the Drop just put
        // on the floor back up.
        self.discarded.push(id);
        json!({
            "kind": "held",
            "token": self.token,
            "name": name,
            "action": DROP,
        })
    }

    /// Nothing on this call's page to close and nothing to take. Before the
    /// bound the reward may still be posting — the frozen `REWARD_WAIT_MS`
    /// window — and after it the loot is over: the finish is the landed
    /// `none-held` abort, never `done`, never `abandon`.
    fn empty(&mut self) -> Value {
        if self.clock.bound_reached() {
            return self.aborted(NONE_HELD);
        }
        self.emit("wait")
    }

    /// The Take dispatched last call, once this call's posted ground no longer
    /// carries that row: the row left, so the frozen `took '…' from the
    /// casket` line is that observation and not an inventory read. A row that
    /// is still posted is not settled, and the scan below re-dispatches it.
    fn settled_take(&mut self, input: &Value) -> Option<String> {
        let take = self.pending_take.take()?;
        if ground_posted(input, take.id) {
            self.pending_take = Some(take);
            return None;
        }
        Some(take.name)
    }

    /// Dispatch the Take for one posted ground row and remember it: the next
    /// call's page says whether the row left. The verb keeps the row's own
    /// posted tile and the name posted beside it, so the host matches the
    /// identity the row was posted with.
    fn take(&mut self, row: &Ground<'_>) -> Value {
        self.pending_take = Some(Take {
            id: row.id,
            name: row.name.to_string(),
        });
        json!({
            "kind": "obj",
            "token": self.token,
            "x": row.tile.x,
            "z": row.tile.z,
            "level": row.tile.level,
            "name": row.name,
            "action": TAKE,
        })
    }

    /// `Steady` on an identified non-casket row: a search membership walks to
    /// its decoded tile and then dispatches the picker from this call's pages;
    /// every other row idles exactly as before.
    ///
    /// The two pages are the wrapper's call-time marshalling of
    /// `host().snapshot` — `here` and the posted loc page — and never a
    /// cached world copy. Emitting `walk` and `loc` repeats while the row
    /// stays held, because arrival is `here` and a stale loc id is the
    /// host's own refuse: the session waits the tick out instead of
    /// abandoning, and the pick is re-read next call.
    fn search(&self, row: &TrailMembershipRow, input: &Value) -> Value {
        let Some(tile) = search_tile(row) else {
            // Not a search membership: the desc-only key-gated riddles, the
            // coord-only map rows and the constrained 3554 clue, among
            // everything else, are identified and then idle.
            return self.emit("wait");
        };
        let Some(here) = input.get("here").and_then(posted_tile) else {
            // No posted tile: there is no arrival claim to make and no walk
            // to measure, so this tick waits rather than walking blind.
            return self.emit("wait");
        };
        if here.level != tile.level || chebyshev(here, tile) > i64::from(ARRIVE_RADIUS) {
            return json!({
                "kind": "walk",
                "token": self.token,
                "x": tile.x,
                "z": tile.z,
                "level": tile.level,
            });
        }
        match pick_loc(input, tile) {
            Some(pick) => json!({
                "kind": "loc",
                "token": self.token,
                "x": pick.tile.x,
                "z": pick.tile.z,
                "level": pick.tile.level,
                "action": pick.action,
                // The posted scene id, always present: the host matches that
                // identity and refuses a stale one rather than taking a
                // co-located other row.
                "id": pick.id,
            }),
            // Arrived with nothing to search this tick — a scene that is not
            // loaded, an empty page, or a loc id the host refused. Wait, keep
            // the token, and re-pick next call; never abandon and never a
            // token-killing `no-searchable-loc`.
            None => self.emit("wait"),
        }
    }

    /// `Steady` on an identified non-casket row: the landed search dispatch,
    /// the sibling unguarded-dig dispatch, or the idle every other row keeps.
    ///
    /// The search pin decides first: `search_tile` is the `trail_loc=^true`
    /// membership and the landed dispatch re-reads its own tile from it, so a
    /// search row can never reach the dig arm and Dig is never a second search
    /// classify. Every other held type still idles exactly as before.
    fn steady(&self, row: &TrailMembershipRow, input: &Value) -> Value {
        if search_tile(row).is_some() {
            return self.search(row, input);
        }
        match dig_tile(row) {
            Some(tile) => self.dig(tile, input),
            None => self.emit("wait"),
        }
    }

    /// `Steady` on an identified unguarded-dig membership: walk to the decoded
    /// `trail_coord` tile, then Dig with the held Spade.
    ///
    /// The arrival is the landed search arrival — this call's posted `here`,
    /// same level and Chebyshev `ARRIVE_RADIUS` — and the walk repeats until it
    /// holds, because arrival is a posted fact and not a walk receipt. A tick
    /// with no posted `here` has no arrival claim to make. A tick that arrived
    /// without the `Spade` on the posted pack page is the same `wait`: the
    /// frozen `no Spade held` refusal is not this machine's token, nothing is
    /// acquired, and the token stays live for the pack that posts it. Dig
    /// repeats while this same clue id stays held, the way the casket's Open
    /// does, and a `none-held` after it still aborts.
    fn dig(&self, tile: Tile, input: &Value) -> Value {
        let Some(here) = input.get("here").and_then(posted_tile) else {
            return self.emit("wait");
        };
        if here.level != tile.level || chebyshev(here, tile) > i64::from(ARRIVE_RADIUS) {
            return json!({
                "kind": "walk",
                "token": self.token,
                "x": tile.x,
                "z": tile.z,
                "level": tile.level,
            });
        }
        if !spade_posted(input) {
            return self.emit("wait");
        }
        json!({
            "kind": "held",
            "token": self.token,
            "name": SPADE_NAME,
            "action": DIG,
        })
    }
}

/// The landed identify over the wrapper's page and the selected family. The
/// family token and `none-held` are the helper's own; only the missing pin is
/// named here.
fn identify<'a>(
    selected: Option<&'a SelectedGameData>,
    input: &Value,
) -> Result<&'a TrailMembershipRow, &'static str> {
    let Some(data) = selected else {
        return Err(MISSING_SELECTED_DATA);
    };
    let held = posted_page(input);
    identify_step(&held, data.trails())
}

/// The posted `(id, count)` page in posted order, as the wrapper marshals it.
/// A missing, non-array, or malformed entry is skipped the same way a row
/// that is not an `i32` pair cannot be held — never a second page and never a
/// snapshot error. Only a positive count holds, and only a membership row
/// wins; that stays in the landed helper.
fn posted_page(input: &Value) -> Vec<(i32, i32)> {
    let Some(rows) = input.get("held").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut held = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(pair) = row.as_array() else {
            continue;
        };
        let (Some(id), Some(count)) = (pair.first().and_then(i32_of), pair.get(1).and_then(i32_of))
        else {
            continue;
        };
        held.push((id, count));
    }
    held
}

/// The wrapper writes page numbers as JSON integers.
fn i32_of(value: &Value) -> Option<i32> {
    i32::try_from(value.as_i64()?).ok()
}

/// One packed-coord square: `SQUARE` tiles per map square on both axes.
/// Same packed contract as the landed `nav::canlight::unpack_packed_coord`;
/// the script crate does not take a `nav` runtime dependency, so the
/// arithmetic lives here.
const SQUARE: i32 = 64;

/// Planes `0..=3`.
const LEVELS: i32 = 4;

/// The frozen picker's `ARRIVE_RADIUS`: the decoded tile itself, and one step
/// off it on either axis.
const ARRIVE_RADIUS: i32 = 1;

/// A world tile: the same three fields the posted `here` object and every
/// posted `SceneEntity` row carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tile {
    x: i32,
    z: i32,
    level: i32,
}

/// Chebyshev distance, widened: `max(|dx|, |dz|)` over the whole `i32` range
/// so a decoded tile and a posted tile can never overflow the subtraction.
/// Callers compare levels first, because a level mismatch is not a distance.
fn chebyshev(a: Tile, b: Tile) -> i64 {
    let dx = (i64::from(a.x) - i64::from(b.x)).abs();
    let dz = (i64::from(a.z) - i64::from(b.z)).abs();
    dx.max(dz)
}

/// A selected `trail_coord` token → the tile it packs.
///
/// The landed `nav::canlight::unpack_packed_coord` contract, copied: five
/// `_`-separated integers `level_mapX_mapZ_localX_localZ`, level in `0..=3`,
/// both locals in `0..64`, and `x = mapX * 64 + localX` (`z` likewise). A
/// missing or extra part, a non-integer, an out-of-range level or local, and
/// a map that overflows the packed widening are all **not** a tile: the row
/// idles rather than walking to an invented coordinate.
fn decode_trail_coord(token: &str) -> Option<Tile> {
    let mut parts = token.split('_');
    let mut next = || parts.next()?.parse::<i32>().ok();
    let level = next()?;
    let map_x = next()?;
    let map_z = next()?;
    let local_x = next()?;
    let local_z = next()?;
    if parts.next().is_some() {
        return None;
    }
    if !(0..LEVELS).contains(&level)
        || !(0..SQUARE).contains(&local_x)
        || !(0..SQUARE).contains(&local_z)
    {
        return None;
    }
    Some(Tile {
        x: map_x.checked_mul(SQUARE)?.checked_add(local_x)?,
        z: map_z.checked_mul(SQUARE)?.checked_add(local_z)?,
        level,
    })
}

/// The identified row's selected search membership: a selected
/// `trail_loc=^true` param **and** a decodable selected `trail_coord` on the
/// same row, in file order for the coord.
///
/// The pin is the membership; the coord alone is not. A coord-only row — the
/// easy maps, the frozen `keyFrom` riddles that carry only `trail_desc`, and
/// the bounded packed 3554 clue — is not a search step and idles. A pin with
/// no coord, or an off-contract one, is the same idle: nothing is invented.
fn search_tile(row: &TrailMembershipRow) -> Option<Tile> {
    let mut located = false;
    let mut coord = None;
    for param in &row.params {
        if param.key == "trail_loc" && param.value == "^true" {
            located = true;
        } else if param.key == "trail_coord" && coord.is_none() {
            coord = Some(param.value.as_str());
        }
    }
    if !located {
        return None;
    }
    decode_trail_coord(coord?)
}

/// The one selected `access` value that is not playable here: the packed 3554
/// clue's own bound. Every other access, and a row that was posted with none
/// at all, is outside this machine's refusals.
const CONSTRAINED: &str = "constrained";

/// The identified row's unguarded-dig membership: a decodable selected
/// `trail_coord` **and** no selected `trail_loc` **and** `trail_sextant=yes`
/// **and** no selected `trail_guardian` **and** an `access` that is not
/// `"constrained"`.
///
/// The sibling of `search_tile`, not a fold into it: the loc pin is the search
/// membership and this is the selected-param classify that holds without
/// copying the frozen `type`. `trail_sextant=yes` is membership only — the
/// Sextant/Watch/Chart trio is never required, never waited for and never
/// acquired — and it is what keeps the coord-only map rows and the
/// riddle-with-coord rows out. A guarded row stays out: its first Dig spawns a
/// wizard this machine cannot fight. The packed constrained 3554 clue stays
/// out with the desc-only, paramless and off-contract rows: identified, then
/// idle rather than an invented coordinate.
fn dig_tile(row: &TrailMembershipRow) -> Option<Tile> {
    if row.access.as_deref() == Some(CONSTRAINED) {
        return None;
    }
    let mut sextant = false;
    let mut located = false;
    let mut guarded = false;
    let mut coord = None;
    for param in &row.params {
        match param.key.as_str() {
            "trail_loc" => located = true,
            "trail_sextant" if param.value == "yes" => sextant = true,
            "trail_guardian" => guarded = true,
            "trail_coord" if coord.is_none() => coord = Some(param.value.as_str()),
            _ => {}
        }
    }
    if located || guarded || !sextant {
        return None;
    }
    decode_trail_coord(coord?)
}

/// A posted `{ x, z, level }` value — the wrapper's `here` tile or one posted
/// loc row. A missing, null, or malformed object is not a tile.
fn posted_tile(value: &Value) -> Option<Tile> {
    Some(Tile {
        x: i32_of(value.get("x")?)?,
        z: i32_of(value.get("z")?)?,
        level: i32_of(value.get("level")?)?,
    })
}

/// The picker's chosen row: the posted tile and id the verb is dispatched at,
/// and the canonical action.
struct Pick {
    tile: Tile,
    id: i32,
    action: &'static str,
}

/// The frozen `pickSearchLoc` rank: `Search` before `Open`, matched
/// case-insensitively on the posted action strings and emitted canonically.
const SEARCH_OPS: [&str; 2] = ["Search", "Open"];

/// Whether a posted page row lists `wanted` among its actions, ignoring ASCII
/// case. The loc and ground pages are the one posted action shape. A missing
/// or non-array `actions` matches nothing.
fn posted_action(row: &Value, wanted: &str) -> bool {
    let Some(actions) = row.get("actions").and_then(Value::as_array) else {
        return false;
    };
    actions.iter().any(|action| {
        action
            .as_str()
            .is_some_and(|text| text.eq_ignore_ascii_case(wanted))
    })
}

/// The frozen picker over this call's posted loc page: posted rows on the
/// decoded tile's level, within `ARRIVE_RADIUS` Chebyshev **of the decoded
/// tile** (not of `here`), whose actions carry `Search` then `Open`.
/// Nearest wins, then the action rank, then posted order — the scan only
/// replaces the best on a strict improvement. The verb keeps the row's own
/// tile and posted id, so the host matches the type identity it was posted
/// with; a row that is not the marshalled `{ id, x, z, level, actions }`
/// shape is skipped rather than guessed at.
fn pick_loc(input: &Value, tile: Tile) -> Option<Pick> {
    let rows = input.get("locs")?.as_array()?;
    let mut best: Option<(Pick, i64, usize)> = None;
    for row in rows {
        let (Some(id), Some(row_tile)) = (row.get("id").and_then(i32_of), posted_tile(row)) else {
            continue;
        };
        if row_tile.level != tile.level {
            continue;
        }
        let distance = chebyshev(row_tile, tile);
        if distance > i64::from(ARRIVE_RADIUS) {
            continue;
        }
        let Some(rank) = SEARCH_OPS.iter().position(|op| posted_action(row, op)) else {
            continue;
        };
        let closer = match &best {
            None => true,
            Some((_, best_distance, best_rank)) => {
                distance < *best_distance || (distance == *best_distance && rank < *best_rank)
            }
        };
        if closer {
            best = Some((
                Pick {
                    tile: row_tile,
                    id,
                    action: SEARCH_OPS[rank],
                },
                distance,
                rank,
            ));
        }
    }
    best.map(|(pick, _, _)| pick)
}

/// One posted ground row a Take can be dispatched at: the posted id that tells
/// the row apart, the name the page posted beside it, and its own posted tile.
struct Ground<'a> {
    id: i32,
    name: &'a str,
    tile: Tile,
}

/// The frozen `collectReward` ground scan: the first posted row on the posted
/// `here` tile whose actions carry `Take` and whose id is neither the frozen
/// `SHARK_ID` nor an id this collect already Dropped.
///
/// A same-tile row is at Chebyshev zero from `here`, so nearest-then-posted is
/// posted order. A row that is not the marshalled
/// `{ id, name, x, z, level, actions }` shape, a row with no posted name, and
/// every row on another tile are skipped rather than guessed at: the verb is
/// the row's own tile and the name the host resolves by first name match.
fn pick_ground<'a>(input: &'a Value, here: Tile, discarded: &[i32]) -> Option<Ground<'a>> {
    let rows = input.get("ground")?.as_array()?;
    for row in rows {
        let (Some(id), Some(tile)) = (row.get("id").and_then(i32_of), posted_tile(row)) else {
            continue;
        };
        if id == SHARK_ID || discarded.contains(&id) || tile != here {
            continue;
        }
        let Some(name) = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        if !posted_action(row, TAKE) {
            continue;
        }
        return Some(Ground { id, name, tile });
    }
    None
}

/// Whether this call's posted ground page still carries `id`: the settlement a
/// Take is read through, never an inventory count.
fn ground_posted(input: &Value, id: i32) -> bool {
    input
        .get("ground")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            rows.iter()
                .any(|row| row.get("id").and_then(i32_of) == Some(id))
        })
}

/// The identified row's role for the held casket item. Membership is the
/// landed identify's; this only reads the role it returned.
const CASKET_ROLE: &str = "casket";

/// The frozen casket arm's action. The held step's other half is the selected
/// item display name, so the host resolves the item by name.
const OPEN: &str = "Open";

/// The identified casket row's held-item identity: the selected item display
/// name joined by the row's own id, or `None` when this row is not a casket.
///
/// The name is what the host's `held` verb resolves by first name match on the
/// inventory page, so the machine emits no item id and no bound row identity:
/// the alias is not a display name, and the item table is never scanned. A
/// casket whose id joins no selected item, or joins one with no display name,
/// has no identity to dispatch and idles rather than inventing one.
fn casket_name<'a>(
    selected: Option<&'a SelectedGameData>,
    row: &TrailMembershipRow,
) -> Option<&'a str> {
    if row.role != CASKET_ROLE {
        return None;
    }
    let name = selected?.item_by_id(row.id)?.name.as_deref()?;
    (!name.is_empty()).then_some(name)
}

/// The frozen `SPADE_NAME = 'Spade'`: the Dig verb's item identity. The host
/// resolves the first inventory row with this display name, so this is the
/// selected-verified display and never the membership alias, never an item id
/// and never a scan of the item table.
const SPADE_NAME: &str = "Spade";

/// The frozen `'dig'` arm's action: the held step's other half beside the
/// Spade display name.
const DIG: &str = "Dig";

/// Whether this call's posted pack page carries the Dig verb's item: a row
/// with a positive count whose posted display name is the frozen `Spade`,
/// compared the way the landed collect compares a posted droppable name.
///
/// The page is the already-marshalled `{ id, name, count }` sequence the
/// collect arm reads from the same `snapshot.inv`, and it is read at call
/// time. A page that does not carry the name is a `wait` — never a refusal
/// token, never `abandon`, and never a reason to fetch from a bank or scan a
/// ground spawn.
fn spade_posted(input: &Value) -> bool {
    input
        .get("inv")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            rows.iter().any(|row| {
                row.get("count")
                    .and_then(i32_of)
                    .is_some_and(|count| count > 0)
                    && row
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| name.eq_ignore_ascii_case(SPADE_NAME))
            })
        })
}

/// Progress line for the identified step: landed alias, role and id only.
/// Never an invented coord, npc or answer, and never `'clue solved'`.
fn progress(row: &TrailMembershipRow) -> String {
    format!("clue step held: {} {} [{}]", row.role, row.alias, row.id)
}

/// Status line for the identified step. Trail completion — `done`, restore,
/// then the exact `'clue solved'` string — is a later slice, not this one.
fn status(row: &TrailMembershipRow) -> String {
    format!("clue: {}", row.alias)
}

/// The ground row a Collecting call dispatched a Take for: the posted id that
/// tells the row apart, and the name its `took '…' from the casket` line
/// reports.
struct Take {
    id: i32,
    name: String,
}

/// The posted pack page's occupied slots: the rows carrying a positive count.
/// A row with no posted count is not an occupied slot. Widened so the compare
/// against the posted slot count can never wrap.
fn occupied(input: &Value) -> i64 {
    input
        .get("inv")
        .and_then(Value::as_array)
        .map_or(0, |rows| {
            rows.iter()
                .filter(|row| {
                    row.get("count")
                        .and_then(i32_of)
                        .is_some_and(|count| count > 0)
                })
                .count() as i64
        })
}

/// The landed helper's own forms for every frozen option key: the selected
/// pin's display names for the foods the pack-full Drop may take, deduplicated
/// the way the shim's own set does.
///
/// Only these forms are ever compared to a posted display name — never the
/// option keys themselves, which are inputs to the helper and not a matcher.
fn droppable_forms(selected: Option<&SelectedGameData>) -> Vec<String> {
    let mut forms: Vec<String> = Vec::new();
    for key in FOOD_OPTION_KEYS {
        for form in food_forms_for(selected, key) {
            if !forms.iter().any(|seen| seen.eq_ignore_ascii_case(&form)) {
                forms.push(form);
            }
        }
    }
    forms
}

/// The frozen `collectReward` pack-full pick: `hard` takes the posted
/// `SHARK_ID` row, else the first posted row the landed `food_forms_for`
/// resolves for a frozen option key.
///
/// The row's own posted display name is what the host resolves by first name
/// match, so a row the page posted no name for, and a row with no positive
/// count, is not a droppable row here and nothing is invented for it.
fn drop_row<'a>(
    selected: Option<&SelectedGameData>,
    input: &'a Value,
    hard: bool,
) -> Option<(i32, &'a str)> {
    let rows = input.get("inv")?.as_array()?;
    let forms = (!hard).then(|| droppable_forms(selected));
    for row in rows {
        let Some(id) = row.get("id").and_then(i32_of) else {
            continue;
        };
        if hard && id != SHARK_ID {
            continue;
        }
        if !row
            .get("count")
            .and_then(i32_of)
            .is_some_and(|count| count > 0)
        {
            continue;
        }
        let Some(name) = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        if forms
            .as_ref()
            .is_some_and(|forms| !forms.iter().any(|form| form.eq_ignore_ascii_case(name)))
        {
            continue;
        }
        return Some((id, name));
    }
    None
}

/// The frozen `collectReward` line for a Take that settled next call.
fn took_line(name: &str) -> String {
    format!("took '{name}' from the casket")
}

/// The frozen `pack-full-no-food` WARNING: the ground row that is left, and
/// the food the `hard` capture names. A log line through `callback.log`, never
/// an error token and never `'clue solved'`.
fn pack_full_warning(name: &str, hard: bool) -> String {
    format!(
        "WARNING: '{name}' is left on the ground, the pack is full with no {} to drop",
        if hard { "Shark" } else { "food" }
    )
}

/// The posted snapshot is not this machine's page: begin and next read the
/// page the wrapper hands in at call time, so nothing is cached here.
/// Registered so the machine's hook set matches the isolate's fan-out.
pub fn on_snapshot(_snap: &SnapshotReader<'_>) {}

pub fn on_pause() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().clock.held;
        rt.borrow_mut().clock.set_freeze(true, held);
    });
}

pub fn on_resume() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().clock.held;
        rt.borrow_mut().clock.set_freeze(false, held);
    });
}

pub fn on_hold(held: bool) {
    RUNTIME.with(|rt| {
        let paused = rt.borrow().clock.paused;
        rt.borrow_mut().clock.set_freeze(paused, held);
    });
}

pub fn on_reset() {
    RUNTIME.with(|rt| rt.borrow_mut().abort());
}

/// The machine's only entry point: the selected pin comes from the native
/// registration's captured `game_data`, and the payload is the wrapper's
/// marshalled call — never a host wire.
pub fn dispatch(selected: Option<&SelectedGameData>, input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => RUNTIME.with(|rt| rt.borrow_mut().begin(selected, input)),
        "next" => RUNTIME.with(|rt| rt.borrow_mut().next(selected, input)),
        _ => json!({ "kind": "notImpl", "reason": "unknown clue op" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::game_data::TrailParam;
    use client::io::ClientRevision;
    use std::sync::Arc;

    const CASKET: i32 = 3531;
    const CLUE: i32 = 3554;
    /// `trail_clue_easy_map001_casket`: an easy casket, so its pack-full food
    /// is the frozen option forms and not the hard `SHARK_ID`.
    const EASY_CASKET: i32 = 2714;
    /// `trail_clue_hard_sextant028_casket`: the casket of the held 3554 clue.
    const SEXTANT_CASKET: i32 = 3555;
    /// `trail_clue_easy_simple001`: the selected search membership,
    /// `trail_loc=^true` with `trail_coord=1_50_50_9_18`.
    const SEARCH: i32 = 2677;
    /// `trail_clue_easy_map001`: a selected `trail_coord` with no `trail_loc`.
    const MAP: i32 = 2713;
    /// `trail_clue_hard_map001`: a selected clue row with no params at all.
    const MAP_EMPTY: i32 = 2722;
    /// `trail_clue_medium_riddle001`: a frozen `keyFrom` riddle, selected
    /// `trail_desc` only.
    const RIDDLE: i32 = 2831;
    /// `trail_clue_medium_sextant001`: the unguarded-dig membership,
    /// `trail_coord=0_49_50_24_51` → (3160, 3251, 0).
    const UNGUARDED: i32 = 2801;
    /// `trail_clue_hard_sextant001`: the guarded sibling of that membership,
    /// same `trail_sextant` and a `trail_guardian`.
    const GUARDED: i32 = 2723;
    /// The item id the frozen `SPADE_NAME` display belongs to. Posted on the
    /// test pack pages the way `snapshot.inv` posts it; the machine itself
    /// never reads an id for the Dig verb.
    const SPADE_ITEM: i32 = 952;

    fn selected() -> Arc<SelectedGameData> {
        api::game_data::for_revision(ClientRevision::R274).expect("selected data")
    }

    /// A held id that is not a membership row: a challenge-answer row, which
    /// stays unread.
    fn unrelated(data: &SelectedGameData) -> i32 {
        data.trails()
            .expect("trails")
            .challenge_answers
            .first()
            .expect("challenge row")
            .id
    }

    fn payload(op: &str, token: Option<u64>, held: Value, extra: Value) -> Value {
        let mut call = json!({ "op": op, "generation": 4, "held": held });
        if let Some(token) = token {
            call["token"] = json!(token);
        }
        for (key, value) in extra.as_object().expect("extra") {
            call[key] = value.clone();
        }
        call
    }

    fn begin(data: &SelectedGameData, held: Value) -> Value {
        dispatch(Some(data), &payload("begin", None, held, json!({})))
    }

    fn call(data: &SelectedGameData, token: u64, held: Value, extra: Value) -> Value {
        dispatch(Some(data), &payload("next", Some(token), held, extra))
    }

    fn token_of(step: &Value) -> u64 {
        step["token"].as_u64().expect("token")
    }

    /// The wrapper's posted `here` tile.
    fn here(x: i32, z: i32, level: i32) -> Value {
        json!({ "x": x, "z": z, "level": level })
    }

    /// One wrapper-marshalled posted loc row.
    fn loc(id: i32, x: i32, z: i32, level: i32, actions: &[&str]) -> Value {
        json!({ "id": id, "x": x, "z": z, "level": level, "actions": actions })
    }

    /// One wrapper-marshalled posted ground row: the same `SceneEntity` shape
    /// as a loc, plus the display name the Take rides along with.
    fn ground(id: i32, name: &str, x: i32, z: i32, level: i32, actions: &[&str]) -> Value {
        json!({ "id": id, "name": name, "x": x, "z": z, "level": level, "actions": actions })
    }

    /// One wrapper-marshalled posted pack row.
    fn inv(id: i32, name: &str, count: i32) -> Value {
        json!({ "id": id, "name": name, "count": count })
    }

    /// The tile the casket's overflow lands on, as the wrapper posts it.
    fn loot_tile() -> Value {
        here(3222, 3223, 1)
    }

    /// One Collecting call's pages: the posted `here`, the posted ground page,
    /// the posted pack rows and slot count, and the posted main modal.
    fn pages(ground_rows: Value, pack: Value, size: Value, main: Value) -> Value {
        json!({
            "here": loot_tile(),
            "ground": ground_rows,
            "inv": pack,
            "inv_size": size,
            "main_modal_id": main,
        })
    }

    /// A casket step driven to its Open: the landed report, then the `held`
    /// step that captures the trail-end `hard` and is the collect's entry
    /// permit.
    fn opened(data: &SelectedGameData, id: i32) -> u64 {
        let page = json!([[id, 1]]);
        let token = steady(data, id);
        let open = call(data, token, page, json!({}));
        assert_eq!(open["kind"], "held", "{open}");
        assert_eq!(open["action"], "Open", "{open}");
        token
    }

    /// The machine's own collect deadline, armed once on Collecting entry.
    fn bound_armed() -> bool {
        RUNTIME.with(|rt| rt.borrow().clock.deadline.is_some())
    }

    /// Arm the deadline as already due: the only way to reach the frozen bound
    /// without a two-second test.
    fn force_bound() {
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            rt.clock.deadline = Some(rt.clock.now());
        });
    }

    /// One row of the selected family.
    fn row(data: &SelectedGameData, id: i32) -> &TrailMembershipRow {
        data.trails()
            .expect("trails")
            .rows
            .iter()
            .find(|row| row.id == id)
            .unwrap_or_else(|| panic!("row {id}"))
    }

    /// The casket row the selected family maps to `clue` through the clue's own
    /// `trail_casket` param — the casket a dig produces.
    fn casket_of(data: &SelectedGameData, clue: i32) -> i32 {
        let alias = row(data, clue)
            .params
            .iter()
            .find(|param| param.key == "trail_casket")
            .expect("trail_casket")
            .value
            .clone();
        data.trails()
            .expect("trails")
            .rows
            .iter()
            .find(|row| row.alias == alias)
            .unwrap_or_else(|| panic!("casket row {alias}"))
            .id
    }

    /// Drive a held row to `Steady`: begin, the gate, the log line, then the
    /// status line. The search verbs start on the call after this one.
    fn steady(data: &SelectedGameData, id: i32) -> u64 {
        let page = json!([[id, 1]]);
        let token = token_of(&begin(data, page.clone()));
        let gate = call(data, token, page.clone(), json!({}));
        assert_eq!(gate["kind"], "callback.enabled", "{gate}");
        let logged = call(data, token, page.clone(), json!({ "resume": true }));
        assert_eq!(logged["kind"], "callback.log", "{logged}");
        let posted = call(data, token, page, json!({}));
        assert_eq!(posted["kind"], "callback.setStatus", "{posted}");
        token
    }

    #[test]
    fn a_frozen_clock_emits_wait_and_keeps_the_gate_unanswered() {
        on_reset();
        let data = selected();
        let opened = begin(&data, json!([[CASKET, 1]]));
        assert_eq!(opened["kind"], "token");
        let token = token_of(&opened);
        let gate = call(&data, token, json!([[CASKET, 1]]), json!({}));
        assert_eq!(gate["kind"], "callback.enabled", "{gate}");

        // Paused: no callback, no verb, and the answer is not consumed. The
        // freeze wins over the posted interrupt carried in the same call.
        on_pause();
        let paused = call(
            &data,
            token,
            json!([[CASKET, 1]]),
            json!({ "resume": true, "hold": true }),
        );
        assert_eq!(paused["kind"], "wait", "{paused}");
        assert!(paused.get("message").is_none(), "{paused}");
        on_resume();
        let after_pause = call(&data, token, json!([[CASKET, 1]]), json!({}));
        assert_eq!(
            after_pause["kind"], "callback.enabled",
            "a frozen call burns nothing: {after_pause}"
        );

        // A held clock is the same wait.
        on_hold(true);
        let held_clock = call(
            &data,
            token,
            json!([[CASKET, 1]]),
            json!({ "resume": true }),
        );
        assert_eq!(held_clock["kind"], "wait", "{held_clock}");
        on_hold(false);

        // The posted `hold || ours` signal without a frozen clock: yield, and
        // the token survives it.
        let yielded = call(&data, token, json!([[CASKET, 1]]), json!({ "hold": true }));
        assert_eq!(yielded["kind"], "yield", "{yielded}");
        assert_eq!(token_of(&yielded), token, "{yielded}");

        // The gate was still open, so the enabled answer lands now.
        let enabled = call(
            &data,
            token,
            json!([[CASKET, 1]]),
            json!({ "resume": true }),
        );
        assert_eq!(enabled["kind"], "callback.log", "{enabled}");
    }

    #[test]
    fn a_generation_bump_aborts_the_session_without_a_verb() {
        on_reset();
        let data = selected();
        let opened = begin(&data, json!([[CLUE, 1]]));
        let token = token_of(&opened);

        let bumped = dispatch(
            Some(&data),
            &payload(
                "next",
                Some(token),
                json!([[CLUE, 1]]),
                json!({ "generation": 5 }),
            ),
        );
        assert_eq!(bumped["kind"], "aborted", "{bumped}");
        assert_eq!(bumped["reason"], "aborted", "{bumped}");
        for kind in [
            "wait",
            "yield",
            "callback.enabled",
            "callback.log",
            "callback.setStatus",
        ] {
            assert_ne!(bumped["kind"], kind, "{bumped}");
        }
        // The token died with the bump: the same call on the old generation
        // is stale, not a resumed session.
        let after = call(&data, token, json!([[CLUE, 1]]), json!({}));
        assert_eq!(after["kind"], "aborted", "{after}");
        assert_eq!(after["reason"], "stale", "{after}");
    }

    #[test]
    fn enabled_is_re_read_at_every_gate_and_never_captured() {
        on_reset();
        let data = selected();
        // Extra begin keys are ignored, not captured: no `enabled`, `resume`,
        // hold flag or page identity is frozen into the session.
        let opened = dispatch(
            Some(&data),
            &payload(
                "begin",
                None,
                json!([[CLUE, 1]]),
                json!({ "enabled": false, "resume": false, "hold": true }),
            ),
        );
        assert_eq!(opened["kind"], "token", "{opened}");
        let token = token_of(&opened);
        assert!(
            opened.get("kind").is_some() && opened.get("reason").is_none(),
            "begin never returns a continue kind and never a reason: {opened}"
        );

        let first = call(&data, token, json!([[CLUE, 1]]), json!({}));
        assert_eq!(first["kind"], "callback.enabled", "{first}");
        // False means do not execute: an idle continue with the token live.
        let denied = call(&data, token, json!([[CLUE, 1]]), json!({ "resume": false }));
        assert_eq!(denied["kind"], "wait", "{denied}");
        assert_eq!(token_of(&denied), token, "{denied}");
        // The next tick asks again rather than replaying the captured answer.
        let again = call(&data, token, json!([[CLUE, 1]]), json!({}));
        assert_eq!(again["kind"], "callback.enabled", "{again}");
        // And a later true is honored, because nothing was captured at begin.
        let enabled = call(&data, token, json!([[CLUE, 1]]), json!({ "resume": true }));
        assert_eq!(enabled["kind"], "callback.log", "{enabled}");
        let message = enabled["message"].as_str().unwrap_or("");
        assert!(message.contains("trail_clue_hard_sextant028"), "{enabled}");
        assert!(message.contains("3554"), "{enabled}");

        // The constrained 3554 row keeps the token and then idles.
        let steady = call(&data, token, json!([[CLUE, 1]]), json!({}));
        assert_eq!(steady["kind"], "callback.setStatus", "{steady}");
        assert!(
            steady["message"]
                .as_str()
                .unwrap_or("")
                .contains("trail_clue_hard_sextant028"),
            "{steady}"
        );
        for _ in 0..2 {
            let idle = call(&data, token, json!([[CLUE, 1]]), json!({}));
            assert_eq!(idle["kind"], "wait", "{idle}");
            assert_eq!(token_of(&idle), token, "{idle}");
        }
    }

    #[test]
    fn a_new_held_step_re_arms_the_gate() {
        on_reset();
        let data = selected();
        let token = token_of(&begin(&data, json!([[CASKET, 1]])));
        assert_eq!(
            call(&data, token, json!([[CASKET, 1]]), json!({}))["kind"],
            "callback.enabled"
        );
        assert_eq!(
            call(
                &data,
                token,
                json!([[CASKET, 1]]),
                json!({ "resume": true })
            )["kind"],
            "callback.log"
        );
        // The clue replaces the casket: the enabled read is not reused.
        let re_armed = call(&data, token, json!([[CLUE, 1]]), json!({ "resume": true }));
        assert_eq!(re_armed["kind"], "callback.enabled", "{re_armed}");
        assert_eq!(token_of(&re_armed), token, "{re_armed}");
    }

    #[test]
    fn none_held_is_a_refusal_and_never_a_live_token_or_a_done() {
        on_reset();
        let data = selected();
        let challenge = unrelated(&data);
        for held in [json!([]), json!([[challenge, 1]]), json!([[CASKET, 0]])] {
            let refused = begin(&data, held.clone());
            assert_eq!(refused["kind"], "aborted", "{held} {refused}");
            assert_eq!(refused["reason"], "none-held", "{held} {refused}");
            // No live token: the refusal's own number is not a session.
            let after = call(&data, token_of(&refused), held.clone(), json!({}));
            assert_eq!(after["reason"], "stale", "{held} {after}");
        }

        // A live session that loses its held membership errors `none-held`
        // and aborts — it does not quietly become done.
        let token = token_of(&begin(&data, json!([[CASKET, 1]])));
        let lost = call(&data, token, json!([[challenge, 1]]), json!({}));
        assert_eq!(lost["kind"], "aborted", "{lost}");
        assert_eq!(lost["reason"], "none-held", "{lost}");
        let after = call(&data, token, json!([[CASKET, 1]]), json!({}));
        assert_eq!(after["reason"], "stale", "{after}");
    }

    #[test]
    fn a_missing_selected_pin_is_not_none_held() {
        on_reset();
        let refused = dispatch(
            None,
            &payload("begin", None, json!([[CASKET, 1]]), json!({})),
        );
        assert_eq!(refused["kind"], "aborted", "{refused}");
        assert_eq!(refused["reason"], "missing-selected-data", "{refused}");
        // The live token's view of a vanished pin is the same refusal.
        let data = selected();
        let token = token_of(&begin(&data, json!([[CASKET, 1]])));
        let gone = dispatch(
            None,
            &payload("next", Some(token), json!([[CASKET, 1]]), json!({})),
        );
        assert_eq!(gone["kind"], "aborted", "{gone}");
        assert_eq!(gone["reason"], "missing-selected-data", "{gone}");
    }

    #[test]
    fn the_exact_clue_solved_string_is_never_emitted() {
        on_reset();
        let data = selected();
        // The packed 3554 clue is not openable: it is identified, reported
        // and then idled, and never a verb of any kind.
        let token = token_of(&begin(&data, json!([[CLUE, 1]])));
        let steps = vec![
            call(&data, token, json!([[CLUE, 1]]), json!({})),
            call(&data, token, json!([[CLUE, 1]]), json!({ "resume": true })),
            call(&data, token, json!([[CLUE, 1]]), json!({})),
            call(&data, token, json!([[CLUE, 1]]), json!({})),
            call(&data, token, json!([[CLUE, 1]]), json!({ "hold": true })),
        ];
        for step in &steps {
            let text = step.to_string();
            assert!(!text.contains("clue solved"), "{step}");
            assert!(!text.contains("ownsEquipment"), "{step}");
            assert!(
                step["status"].is_null(),
                "the machine emits kinds, not the public status: {step}"
            );
            assert_ne!(step["kind"], "done", "{step}");
        }
        assert_eq!(
            steps
                .iter()
                .map(|step| step["kind"].clone())
                .collect::<Vec<_>>(),
            vec![
                json!("callback.enabled"),
                json!("callback.log"),
                json!("callback.setStatus"),
                json!("wait"),
                json!("yield"),
            ],
            "{steps:?}"
        );
    }

    #[test]
    fn the_packed_coord_contract_is_copied_and_off_contract_tokens_idle() {
        // The pinned vector: `trail_clue_easy_simple001`'s selected token.
        assert_eq!(
            decode_trail_coord("1_50_50_9_18"),
            Some(Tile {
                x: 3209,
                z: 3218,
                level: 1
            })
        );
        // The landed `nav::canlight` vector, plus both ends of each bound.
        assert_eq!(
            decode_trail_coord("0_50_53_50_24"),
            Some(Tile {
                x: 3250,
                z: 3416,
                level: 0
            })
        );
        assert_eq!(
            decode_trail_coord("3_0_0_0_0"),
            Some(Tile {
                x: 0,
                z: 0,
                level: 3
            })
        );
        assert_eq!(
            decode_trail_coord("0_0_0_63_63"),
            Some(Tile {
                x: 63,
                z: 63,
                level: 0
            })
        );
        for token in [
            "",
            "1_50_50_9",
            "1_50_50_9_18_1",
            "1_50_50_9_x",
            "one_50_50_9_18",
            "4_50_50_9_18",
            "-1_50_50_9_18",
            "1_50_50_64_18",
            "1_50_50_9_64",
            "1_50_50_-1_18",
            "1_50_50_9_-1",
            " 1_50_50_9_18",
            "1_50_50_9_18 ",
            // A map that would overflow the packed widening is not a tile.
            "2147483647_50_50_9_18",
            "1_2147483647_50_9_18",
        ] {
            assert_eq!(decode_trail_coord(token), None, "{token:?}");
        }
    }

    #[test]
    fn membership_is_the_loc_pin_plus_a_decodable_coord_on_the_same_row() {
        let data = selected();
        assert_eq!(
            search_tile(row(&data, SEARCH)),
            Some(Tile {
                x: 3209,
                z: 3218,
                level: 1
            })
        );
        // The coord-only maps, the desc-only frozen `keyFrom` riddles, the
        // constrained 3554 clue and a paramless casket are not search rows.
        for id in [MAP, RIDDLE, CLUE, CASKET] {
            assert_eq!(search_tile(row(&data, id)), None, "{id}");
        }
        // The selected pin is the membership, and every pinned row on this
        // pin carries a decodable coord.
        let facts = data.trails().expect("trails");
        let pinned: Vec<&TrailMembershipRow> = facts
            .rows
            .iter()
            .filter(|row| {
                row.params
                    .iter()
                    .any(|param| param.key == "trail_loc" && param.value == "^true")
            })
            .collect();
        assert_eq!(pinned.len(), 58, "selected trail_loc=^true rows");
        for row in &pinned {
            assert!(search_tile(row).is_some(), "{}", row.alias);
        }
        // Synthetic rows: the pin without a coord, an off-contract coord, and
        // a bare `true` that is not the `^true` pin all idle.
        let pin = || TrailParam {
            key: "trail_loc".into(),
            value: "^true".into(),
        };
        let coord = |value: &str| TrailParam {
            key: "trail_coord".into(),
            value: value.into(),
        };
        let member = |params: Vec<TrailParam>| TrailMembershipRow {
            alias: "trail_clue_test".into(),
            id: 1,
            role: "clue".into(),
            params,
            access: None,
        };
        assert_eq!(search_tile(&member(vec![pin()])), None);
        assert_eq!(search_tile(&member(vec![coord("1_50_50_9_18")])), None);
        assert_eq!(search_tile(&member(vec![pin(), coord("1_50_50_9")])), None);
        assert_eq!(
            search_tile(&member(vec![
                TrailParam {
                    key: "trail_loc".into(),
                    value: "true".into(),
                },
                coord("1_50_50_9_18"),
            ])),
            None
        );
        assert_eq!(
            search_tile(&member(vec![pin(), coord("1_50_50_9_18")])),
            Some(Tile {
                x: 3209,
                z: 3218,
                level: 1
            })
        );
    }

    #[test]
    fn a_search_row_walks_to_the_decoded_tile_and_then_picks_the_loc() {
        on_reset();
        let data = selected();
        let page = json!([[SEARCH, 1]]);
        let token = steady(&data, SEARCH);

        // Not arrived: the walk is the decoded tile, and it repeats.
        for _ in 0..2 {
            let walk = call(
                &data,
                token,
                page.clone(),
                json!({ "here": here(3200, 3218, 1) }),
            );
            assert_eq!(walk["kind"], "walk", "{walk}");
            assert_eq!(walk["x"], 3209, "{walk}");
            assert_eq!(walk["z"], 3218, "{walk}");
            assert_eq!(walk["level"], 1, "{walk}");
            assert_eq!(token_of(&walk), token, "{walk}");
        }
        // Another level is not arrival, even standing on the decoded tile,
        // and neither is two tiles away on one axis.
        for far in [here(3209, 3218, 0), here(3211, 3218, 1)] {
            let walk = call(&data, token, page.clone(), json!({ "here": far }));
            assert_eq!(walk["kind"], "walk", "{walk}");
        }

        // Arrived. The nearest row wins over the better rank: the adjacent
        // `Search` loses to the `Open` on the decoded tile.
        let nearest = call(
            &data,
            token,
            page.clone(),
            json!({
                "here": here(3209, 3218, 1),
                "locs": [
                    loc(11, 3210, 3218, 1, &["Search"]),
                    loc(12, 3209, 3218, 1, &["Open"]),
                ],
            }),
        );
        assert_eq!(nearest["kind"], "loc", "{nearest}");
        assert_eq!(nearest["action"], "Open", "{nearest}");
        assert_eq!(nearest["id"], 12, "{nearest}");
        assert_eq!(
            (
                nearest["x"].clone(),
                nearest["z"].clone(),
                nearest["level"].clone()
            ),
            (json!(3209), json!(3218), json!(1)),
            "{nearest}"
        );

        // Same distance: the rank decides, not the posted order.
        let ranked = call(
            &data,
            token,
            page.clone(),
            json!({
                "here": here(3209, 3218, 1),
                "locs": [
                    loc(13, 3209, 3218, 1, &["Open"]),
                    loc(14, 3209, 3218, 1, &["Search", "Pick"]),
                ],
            }),
        );
        assert_eq!(ranked["kind"], "loc", "{ranked}");
        assert_eq!(ranked["action"], "Search", "{ranked}");
        assert_eq!(ranked["id"], 14, "{ranked}");

        // Same distance and rank: the first posted row wins.
        let first = call(
            &data,
            token,
            page.clone(),
            json!({
                "here": here(3209, 3218, 1),
                "locs": [
                    loc(15, 3209, 3218, 1, &["Search"]),
                    loc(16, 3209, 3218, 1, &["search"]),
                ],
            }),
        );
        assert_eq!(first["id"], 15, "{first}");
        assert_eq!(first["action"], "Search", "{first}");

        // Off-contract rows are skipped: another level, distance two, no
        // searchable action, a non-array action list, and no posted id. The
        // survivor matches case-insensitively and keeps its own tile.
        let filtered = call(
            &data,
            token,
            page,
            json!({
                "here": here(3209, 3218, 1),
                "locs": [
                    loc(20, 3209, 3218, 2, &["Search"]),
                    loc(21, 3211, 3218, 1, &["Search"]),
                    loc(22, 3209, 3218, 1, &["Pick", "Use"]),
                    json!({ "id": 23, "x": 3209, "z": 3218, "level": 1, "actions": "Search" }),
                    json!({ "x": 3209, "z": 3218, "level": 1, "actions": ["Search"] }),
                    loc(25, 3210, 3217, 1, &["sEaRcH"]),
                ],
            }),
        );
        assert_eq!(filtered["kind"], "loc", "{filtered}");
        assert_eq!(filtered["action"], "Search", "{filtered}");
        assert_eq!(filtered["id"], 25, "{filtered}");
        assert_eq!(
            (
                filtered["x"].clone(),
                filtered["z"].clone(),
                filtered["level"].clone()
            ),
            (json!(3210), json!(3217), json!(1)),
            "the verb keeps the posted row's own tile: {filtered}"
        );
    }

    #[test]
    fn an_arrived_search_row_waits_without_a_loc_and_never_abandons() {
        on_reset();
        let data = selected();
        let page = json!([[SEARCH, 1]]);
        let token = steady(&data, SEARCH);
        for locs in [json!([]), json!([loc(11, 3209, 3218, 1, &["Pick"])])] {
            let idle = call(
                &data,
                token,
                page.clone(),
                json!({ "here": here(3209, 3218, 1), "locs": locs }),
            );
            assert_eq!(idle["kind"], "wait", "{idle}");
            assert_eq!(token_of(&idle), token, "{idle}");
            let text = idle.to_string();
            for forbidden in ["no-searchable-loc", "abandon", "done", "clue solved"] {
                assert!(!text.contains(forbidden), "{idle}");
            }
        }
        // No posted `here` at all is the same wait: no arrival claim and no
        // blind walk.
        let no_tile = call(&data, token, page.clone(), json!({ "locs": [] }));
        assert_eq!(no_tile["kind"], "wait", "{no_tile}");
        let malformed = call(
            &data,
            token,
            page.clone(),
            json!({ "here": json!({ "x": 1 }) }),
        );
        assert_eq!(malformed["kind"], "wait", "{malformed}");
        // And the session is still live for the loc that appears later.
        let picked = call(
            &data,
            token,
            page,
            json!({
                "here": here(3208, 3218, 1),
                "locs": [loc(42, 3209, 3218, 1, &["Search"])],
            }),
        );
        assert_eq!(picked["kind"], "loc", "{picked}");
        assert_eq!(picked["id"], 42, "{picked}");
    }

    #[test]
    fn freeze_and_yield_beat_the_search_walk() {
        on_reset();
        let data = selected();
        let page = json!([[SEARCH, 1]]);
        let token = steady(&data, SEARCH);
        let scene = json!({
            "here": here(3100, 3300, 1),
            "locs": [loc(11, 3209, 3218, 1, &["Search"])],
        });
        // Frozen: wait, and neither verb rides along with it.
        on_pause();
        let paused = call(&data, token, page.clone(), scene.clone());
        assert_eq!(paused["kind"], "wait", "{paused}");
        on_resume();
        on_hold(true);
        let held_clock = call(&data, token, page.clone(), scene.clone());
        assert_eq!(held_clock["kind"], "wait", "{held_clock}");
        on_hold(false);
        // The posted `hold || ours` interrupt, unfrozen: yield, still no verb.
        let yielded = call(
            &data,
            token,
            page.clone(),
            json!({
                "here": here(3100, 3300, 1),
                "locs": [loc(11, 3209, 3218, 1, &["Search"])],
                "hold": true,
            }),
        );
        assert_eq!(yielded["kind"], "yield", "{yielded}");
        for step in [&paused, &held_clock, &yielded] {
            assert!(step.get("x").is_none(), "{step}");
            assert!(step.get("action").is_none(), "{step}");
            assert!(step.get("id").is_none(), "{step}");
        }
        // The walk is still there once the interrupt clears.
        let walking = call(&data, token, page, scene);
        assert_eq!(walking["kind"], "walk", "{walking}");
    }

    #[test]
    fn non_search_rows_stay_idle_over_a_fully_posted_scene() {
        on_reset();
        let data = selected();
        let scene = json!({
            "here": here(3209, 3218, 1),
            "locs": [loc(11, 3209, 3218, 1, &["Search"])],
        });
        // A casket is a held Open, so it is not in this set: 3554 is the
        // packed constrained clue, 2722 and 2713 are clue rows with no search
        // pin, and 2831 is a desc-only riddle.
        for id in [CLUE, MAP_EMPTY, MAP, RIDDLE] {
            let page = json!([[id, 1]]);
            let token = steady(&data, id);
            for _ in 0..2 {
                let idle = call(&data, token, page.clone(), scene.clone());
                assert_eq!(idle["kind"], "wait", "{id} {idle}");
                assert_eq!(token_of(&idle), token, "{id} {idle}");
                assert!(idle.get("x").is_none(), "{id} {idle}");
                assert!(idle.get("action").is_none(), "{id} {idle}");
            }
        }
    }

    #[test]
    fn a_casket_row_opens_the_held_item_only_after_its_own_report() {
        on_reset();
        let data = selected();
        let page = json!([[CASKET, 1]]);
        let token = token_of(&begin(&data, page.clone()));
        // The landed report comes first: no Open rides along with the gate,
        // the log line or the status line.
        let steps = vec![
            call(&data, token, page.clone(), json!({})),
            call(&data, token, page.clone(), json!({ "resume": true })),
        ];
        assert_eq!(steps[0]["kind"], "callback.enabled", "{steps:?}");
        assert_eq!(steps[1]["kind"], "callback.log", "{steps:?}");
        let posted = call(&data, token, page.clone(), json!({}));
        assert_eq!(posted["kind"], "callback.setStatus", "{posted}");

        // Open replaces `Steady` from here, and repeats while the same casket
        // id stays held — the host refuses a casket it no longer holds.
        for _ in 0..2 {
            let open = call(&data, token, page.clone(), json!({}));
            assert_eq!(open["kind"], "held", "{open}");
            assert_eq!(open["name"], "Casket", "{open}");
            assert_eq!(open["action"], "Open", "{open}");
            assert_eq!(token_of(&open), token, "{open}");
            // No bound row identity and no scene verb: the host resolves the
            // first inventory row with this name.
            assert!(open.get("id").is_none(), "{open}");
            assert!(open.get("x").is_none(), "{open}");
            assert!(open.get("z").is_none(), "{open}");
            assert!(open.get("level").is_none(), "{open}");
            assert_ne!(open["kind"], "done", "{open}");
            assert!(!open.to_string().contains("clue solved"), "{open}");
        }

        // A different held row re-arms the gate: the Open was the casket's,
        // and the packed clue behind it is not opened.
        let clue = call(&data, token, json!([[CLUE, 1]]), json!({}));
        assert_eq!(clue["kind"], "callback.enabled", "{clue}");
        assert_eq!(token_of(&clue), token, "{clue}");
    }

    #[test]
    fn the_sextant_casket_is_the_open_and_not_the_packed_clue() {
        on_reset();
        let data = selected();
        // Identify is casket-first: with the constrained 3554 clue and its
        // casket both held, the casket row is the step. The landed report is
        // the casket's own, and the Open is never 3554 play.
        let both = json!([[CLUE, 1], [SEXTANT_CASKET, 1]]);
        let token = token_of(&begin(&data, both.clone()));
        assert_eq!(
            call(&data, token, both.clone(), json!({}))["kind"],
            "callback.enabled"
        );
        let logged = call(&data, token, both.clone(), json!({ "resume": true }));
        assert_eq!(logged["kind"], "callback.log", "{logged}");
        let message = logged["message"].as_str().unwrap_or("");
        assert!(
            message.contains("trail_clue_hard_sextant028_casket"),
            "{logged}"
        );
        assert!(message.contains("3555"), "{logged}");
        assert!(!message.contains("3554"), "{logged}");
        assert_eq!(
            call(&data, token, both.clone(), json!({}))["kind"],
            "callback.setStatus"
        );
        let open = call(&data, token, both.clone(), json!({}));
        assert_eq!(open["kind"], "held", "{open}");
        assert_eq!(open["name"], "Casket", "{open}");
        assert_eq!(open["action"], "Open", "{open}");
        // The clue left on its own re-arms the gate rather than opening it.
        let clue_only = call(&data, token, json!([[CLUE, 1]]), json!({}));
        assert_eq!(clue_only["kind"], "callback.enabled", "{clue_only}");
    }

    #[test]
    fn freeze_and_yield_beat_the_casket_open() {
        on_reset();
        let data = selected();
        let page = json!([[CASKET, 1]]);
        let token = steady(&data, CASKET);
        // Frozen: wait, and no Open rides along with it.
        on_pause();
        let paused = call(&data, token, page.clone(), json!({ "hold": true }));
        assert_eq!(paused["kind"], "wait", "{paused}");
        on_resume();
        on_hold(true);
        let held_clock = call(&data, token, page.clone(), json!({}));
        assert_eq!(held_clock["kind"], "wait", "{held_clock}");
        on_hold(false);
        // The posted `hold || ours` interrupt, unfrozen: yield, still no Open
        // and the token lives.
        let yielded = call(&data, token, page.clone(), json!({ "hold": true }));
        assert_eq!(yielded["kind"], "yield", "{yielded}");
        assert_eq!(token_of(&yielded), token, "{yielded}");
        for step in [&paused, &held_clock, &yielded] {
            assert!(step.get("name").is_none(), "{step}");
            assert!(step.get("action").is_none(), "{step}");
        }
        // Thawed and unheld, the Open is still there.
        let open = call(&data, token, page, json!({}));
        assert_eq!(open["kind"], "held", "{open}");
        assert_eq!(open["name"], "Casket", "{open}");
        assert_eq!(open["action"], "Open", "{open}");
    }

    #[test]
    fn every_selected_casket_row_resolves_the_casket_display_name() {
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let data = api::game_data::for_revision(revision).expect("selected data");
            let facts = data.trails().expect("trails");
            let caskets: Vec<&TrailMembershipRow> = facts
                .rows
                .iter()
                .filter(|row| row.role == CASKET_ROLE)
                .collect();
            assert_eq!(caskets.len(), 71, "{revision:?}");
            for casket in &caskets {
                assert!(casket.params.is_empty(), "{revision:?} {}", casket.alias);
                assert_eq!(
                    casket_name(Some(&data), casket),
                    Some("Casket"),
                    "{revision:?} {}",
                    casket.alias
                );
            }
            // The clue rows carry a display name too, and are still not
            // caskets: the role decides, not the item table.
            assert_eq!(casket_name(Some(&data), row(&data, CLUE)), None);
            assert_eq!(casket_name(Some(&data), row(&data, SEARCH)), None);
            // No selected pin, and a casket id that joins no selected item,
            // are both no identity to dispatch — never an invented name.
            let orphan = TrailMembershipRow {
                alias: "trail_clue_test_casket".into(),
                id: i32::MAX,
                role: CASKET_ROLE.into(),
                params: Vec::new(),
                access: None,
            };
            assert_eq!(casket_name(None, &orphan), None, "{revision:?}");
            assert_eq!(casket_name(Some(&data), &orphan), None, "{revision:?}");
        }
    }

    #[test]
    fn the_search_row_emits_only_walk_and_loc_and_never_a_completion() {
        on_reset();
        let data = selected();
        let page = json!([[SEARCH, 1]]);
        let token = steady(&data, SEARCH);
        let steps = vec![
            call(
                &data,
                token,
                page.clone(),
                json!({ "here": here(3200, 3218, 1) }),
            ),
            call(
                &data,
                token,
                page.clone(),
                json!({
                    "here": here(3209, 3218, 1),
                    "locs": [loc(11, 3209, 3218, 1, &["Search"])],
                }),
            ),
            call(
                &data,
                token,
                page.clone(),
                json!({ "here": here(3209, 3218, 1) }),
            ),
            call(&data, token, page, json!({ "hold": true })),
        ];
        for step in &steps {
            let text = step.to_string();
            for forbidden in ["clue solved", "abandon", "supplies-needed", "dead", "done"] {
                assert!(!text.contains(forbidden), "{step}");
            }
            assert!(
                step["status"].is_null(),
                "the machine emits kinds, not the public status: {step}"
            );
            assert!(
                matches!(
                    step["kind"].as_str().unwrap_or(""),
                    "walk" | "loc" | "wait" | "yield"
                ),
                "the search row emits walk, loc, wait or yield only: {step}"
            );
        }
        assert_eq!(
            steps
                .iter()
                .map(|step| step["kind"].clone())
                .collect::<Vec<_>>(),
            vec![json!("walk"), json!("loc"), json!("wait"), json!("yield"),],
            "{steps:?}"
        );
    }

    #[test]
    fn an_unknown_op_is_not_impl() {
        on_reset();
        let data = selected();
        let step = dispatch(Some(&data), &json!({ "op": "solve" }));
        assert_eq!(step["kind"], "notImpl", "{step}");
    }

    /// `Steady` on a casket whose Open went out survives the identify
    /// `none-held` that used to abort: the posted reward interface closes, the
    /// casket's own overflow is Taken, the settled Take logs, and the empty
    /// page after it finishes with the landed `none-held` abort — never
    /// `done`, never `'clue solved'`. This is the sextant casket the packed
    /// 3554 clue belongs to, so the collect is never 3554 play.
    #[test]
    fn collecting_closes_the_posted_reward_then_takes_the_casket_overflow() {
        on_reset();
        let data = selected();
        let token = opened(&data, SEXTANT_CASKET);
        // The Open is not the bound: the window is armed on Collecting entry
        // and nowhere else.
        assert!(!bound_armed(), "the Open arms nothing");

        // The casket left the pack: identify is `none-held` from the step
        // whose Open went out. The posted reward interface is closed first, by
        // the id the page posted — never a 6960 constant.
        let scene = pages(
            json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
            json!([inv(385, "Shark", 5)]),
            json!(28),
            json!(6960),
        );
        let closed = call(&data, token, json!([]), scene.clone());
        assert_eq!(closed["kind"], "close-modal", "{closed}");
        assert_eq!(closed["token"], json!(token), "{closed}");
        assert!(closed.get("reason").is_none(), "{closed}");
        assert!(bound_armed(), "entry arms the frozen reward window");

        // Any other posted open id closes the same way; the verb is the page's
        // own fact and carries nothing else.
        let other = call(
            &data,
            token,
            json!([]),
            pages(json!([]), json!([]), json!(28), json!(42)),
        );
        assert_eq!(other["kind"], "close-modal", "{other}");
        for absent in ["x", "z", "level", "name", "action", "id", "message"] {
            assert!(other.get(absent).is_none(), "{absent} {other}");
        }

        // Closed: the overflow on the posted tile is Taken by posted name.
        let took = call(
            &data,
            token,
            json!([]),
            pages(
                json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
                json!([inv(385, "Shark", 5)]),
                json!(28),
                json!(-1),
            ),
        );
        assert_eq!(took["kind"], "obj", "{took}");
        assert_eq!(took["action"], "Take", "{took}");
        assert_eq!(took["name"], "Rune platebody", "{took}");
        assert_eq!(
            (took["x"].clone(), took["z"].clone(), took["level"].clone()),
            (json!(3222), json!(3223), json!(1)),
            "{took}"
        );
        assert_eq!(took["token"], json!(token), "{took}");

        // The row left the posted ground: the frozen line, one call later.
        let logged = call(
            &data,
            token,
            json!([]),
            pages(
                json!([]),
                json!([inv(900, "Rune platebody", 1)]),
                json!(28),
                json!(-1),
            ),
        );
        assert_eq!(logged["kind"], "callback.log", "{logged}");
        assert_eq!(
            logged["message"], "took 'Rune platebody' from the casket",
            "{logged}"
        );

        // Still within the window an empty page is a wait, not a finish.
        let waiting = call(
            &data,
            token,
            json!([]),
            pages(json!([]), json!([]), json!(28), json!(-1)),
        );
        assert_eq!(waiting["kind"], "wait", "{waiting}");
        assert_eq!(waiting["token"], json!(token), "{waiting}");

        // Past it the loot is over: the landed abort, and the deadline with it.
        force_bound();
        let finished = call(
            &data,
            token,
            json!([]),
            pages(json!([]), json!([]), json!(28), json!(-1)),
        );
        assert_eq!(finished["kind"], "aborted", "{finished}");
        assert_eq!(finished["reason"], "none-held", "{finished}");
        assert!(!bound_armed(), "the finish clears the deadline");
        let after = call(
            &data,
            token,
            json!([]),
            pages(json!([]), json!([]), json!(28), json!(-1)),
        );
        assert_eq!(after["reason"], "stale", "{after}");

        let text = json!([closed, other, took, logged, waiting, finished]).to_string();
        for forbidden in [
            "clue solved",
            "done",
            "abandon",
            "ownsEquipment",
            "supplies-needed",
            "trail complete",
        ] {
            assert!(!text.contains(forbidden), "{forbidden} {text}");
        }
    }

    /// Identify is still made on every Collecting call: the next scroll — or a
    /// leftover casket — leaves the phase and re-arms the landed gate, so no
    /// close and no Take is ever dispatched under a step that is still held.
    #[test]
    fn collecting_leaves_for_the_next_scroll_and_re_arms_the_gate() {
        on_reset();
        let data = selected();
        let token = opened(&data, SEXTANT_CASKET);
        let scene = pages(
            json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
            json!([]),
            json!(28),
            json!(6960),
        );

        // The next scroll came back: the collect is skipped.
        let page = json!([[CLUE, 1]]);
        let re_armed = call(&data, token, page.clone(), scene.clone());
        assert_eq!(re_armed["kind"], "callback.enabled", "{re_armed}");
        assert_eq!(re_armed["token"], json!(token), "{re_armed}");
        assert!(!bound_armed(), "leaving the collect clears its window");

        // And the landed report for the new row follows, with the progress
        // line of the row identify returned — never of the casket.
        let mut gate = scene.clone();
        gate["resume"] = json!(true);
        let logged = call(&data, token, page, gate);
        assert_eq!(logged["kind"], "callback.log", "{logged}");
        let message = logged["message"].as_str().unwrap_or("");
        assert!(message.contains("3554"), "{logged}");
        assert!(!message.contains("3555"), "{logged}");
    }

    /// A leftover casket after the Open is the landed Open again, not a
    /// second collect: identify wins and the gate re-arms for it.
    #[test]
    fn a_leftover_casket_opens_instead_of_collecting() {
        on_reset();
        let data = selected();
        let token = opened(&data, SEXTANT_CASKET);
        let scene = pages(
            json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
            json!([]),
            json!(28),
            json!(6960),
        );
        let page = json!([[CASKET, 1]]);
        assert_eq!(
            call(&data, token, page.clone(), scene.clone())["kind"],
            "callback.enabled"
        );
        assert_eq!(
            call(
                &data,
                token,
                page.clone(),
                json!({ "resume": true, "here": loot_tile(), "main_modal_id": 6960 })
            )["kind"],
            "callback.log"
        );
        assert_eq!(
            call(&data, token, page.clone(), scene.clone())["kind"],
            "callback.setStatus"
        );
        let open = call(&data, token, page, scene);
        assert_eq!(open["kind"], "held", "{open}");
        assert_eq!(open["name"], "Casket", "{open}");
        assert_eq!(open["action"], "Open", "{open}");
    }

    /// The ground scan: same tile as the posted `here`, a posted `Take`, the
    /// frozen `SHARK_ID` skipped, and the row's own posted name riding along.
    #[test]
    fn the_take_needs_the_posted_tile_a_posted_take_and_a_posted_name() {
        on_reset();
        let data = selected();
        let token = opened(&data, SEXTANT_CASKET);
        let pack = json!([inv(385, "Shark", 5)]);

        // The shark is skipped, so a shark-only page has nothing to take: a
        // wait, and the token lives.
        let shark_only = call(
            &data,
            token,
            json!([]),
            pages(
                json!([ground(SHARK_ID, "Shark", 3222, 3223, 1, &["Take"])]),
                pack.clone(),
                json!(28),
                json!(-1),
            ),
        );
        assert_eq!(shark_only["kind"], "wait", "{shark_only}");

        // Off-tile, another level, no posted `Take`, no posted name, and a
        // malformed row are all skipped rather than guessed at.
        let filtered = call(
            &data,
            token,
            json!([]),
            pages(
                json!([
                    ground(900, "Rune platebody", 3223, 3223, 1, &["Take"]),
                    ground(901, "Rune platebody", 3222, 3223, 0, &["Take"]),
                    ground(902, "Rune platebody", 3222, 3223, 1, &["Use", "Examine"]),
                    json!({ "id": 903, "name": "Rune platebody", "x": 3222, "z": 3223, "level": 1, "actions": "Take" }),
                    json!({ "id": 904, "x": 3222, "z": 3223, "level": 1, "actions": ["Take"] }),
                ]),
                pack.clone(),
                json!(28),
                json!(-1),
            ),
        );
        assert_eq!(filtered["kind"], "wait", "{filtered}");

        // The survivor matches case-insensitively and keeps its own posted
        // name and tile.
        let picked = call(
            &data,
            token,
            json!([]),
            pages(
                json!([ground(905, "Dragon bones", 3222, 3223, 1, &["tAkE"])]),
                pack,
                json!(28),
                json!(-1),
            ),
        );
        assert_eq!(picked["kind"], "obj", "{picked}");
        assert_eq!(picked["name"], "Dragon bones", "{picked}");
        assert_eq!(picked["action"], "Take", "{picked}");
    }

    /// A Take that has not settled is re-dispatched: the row is still posted,
    /// so the next call is the verb again and never the `took` line.
    #[test]
    fn an_unsettled_take_is_dispatched_again() {
        on_reset();
        let data = selected();
        let token = opened(&data, SEXTANT_CASKET);
        let scene = pages(
            json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
            json!([]),
            json!(28),
            json!(-1),
        );
        for _ in 0..2 {
            let took = call(&data, token, json!([]), scene.clone());
            assert_eq!(took["kind"], "obj", "{took}");
            assert_eq!(took["action"], "Take", "{took}");
        }
    }

    /// A full pack Drops one frozen food row and Takes next call. The easy
    /// casket reaches the frozen option forms through the landed helper: the
    /// row is a *form* of `Cake`, whose display name is not one of the option
    /// keys.
    #[test]
    fn a_full_pack_drops_one_food_row_then_takes() {
        on_reset();
        let data = selected();
        assert!(
            droppable_forms(Some(&data))
                .iter()
                .any(|form| form == "slice of cake"),
            "the landed helper resolves the cake family"
        );
        let token = opened(&data, EASY_CASKET);
        let loot = json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]);
        let pack = json!([inv(1895, "Slice of cake", 1), inv(1200, "Rune scimitar", 1)]);

        let dropped = call(
            &data,
            token,
            json!([]),
            pages(loot.clone(), pack.clone(), json!(2), json!(-1)),
        );
        assert_eq!(dropped["kind"], "held", "{dropped}");
        assert_eq!(dropped["name"], "Slice of cake", "{dropped}");
        assert_eq!(dropped["action"], "Drop", "{dropped}");
        assert_eq!(dropped["token"], json!(token), "{dropped}");
        // No tile and no id ride along: the host resolves the name.
        for absent in ["x", "z", "level", "id"] {
            assert!(dropped.get(absent).is_none(), "{absent} {dropped}");
        }

        // The Drop freed the slot: the Take is next call, and the row the Drop
        // put on the floor is not the row that is taken.
        let took = call(
            &data,
            token,
            json!([]),
            pages(
                json!([
                    ground(900, "Rune platebody", 3222, 3223, 1, &["Take"]),
                    ground(1895, "Slice of cake", 3222, 3223, 1, &["Take"]),
                ]),
                json!([inv(1200, "Rune scimitar", 1)]),
                json!(2),
                json!(-1),
            ),
        );
        assert_eq!(took["kind"], "obj", "{took}");
        assert_eq!(took["name"], "Rune platebody", "{took}");
    }

    /// The hard casket's own food is the frozen `SHARK_ID` — the same id the
    /// Take skip already carries — and never a name match.
    #[test]
    fn a_full_pack_drops_the_shark_on_a_hard_casket() {
        on_reset();
        let data = selected();
        let token = opened(&data, SEXTANT_CASKET);
        let loot = json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]);

        // A cake form is droppable on an easy casket and not on this one.
        let caked = call(
            &data,
            token,
            json!([]),
            pages(
                loot.clone(),
                json!([inv(1895, "Slice of cake", 1), inv(1200, "Rune scimitar", 1)]),
                json!(2),
                json!(-1),
            ),
        );
        assert_eq!(caked["kind"], "callback.log", "{caked}");
        assert!(
            caked["message"]
                .as_str()
                .unwrap_or("")
                .ends_with("no Shark to drop"),
            "{caked}"
        );

        // The shark itself is the hard drop: a fresh session, because the
        // WARNING above already ended this collect.
        on_reset();
        let token = opened(&data, SEXTANT_CASKET);
        let dropped = call(
            &data,
            token,
            json!([]),
            pages(
                json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
                json!([inv(1200, "Rune scimitar", 1), inv(SHARK_ID, "Shark", 3)]),
                json!(2),
                json!(-1),
            ),
        );
        assert_eq!(dropped["kind"], "held", "{dropped}");
        assert_eq!(dropped["name"], "Shark", "{dropped}");
        assert_eq!(dropped["action"], "Drop", "{dropped}");
    }

    /// A full pack with nothing droppable logs the frozen WARNING and then
    /// finishes with the landed `none-held` abort: a log line, not an error
    /// token, and not a `done`.
    #[test]
    fn a_full_pack_with_no_food_warns_and_then_finishes_none_held() {
        on_reset();
        let data = selected();
        let token = opened(&data, EASY_CASKET);
        let scene = pages(
            json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
            json!([
                inv(1200, "Rune scimitar", 1),
                inv(1201, "Rune platebody", 1)
            ]),
            json!(2),
            json!(-1),
        );
        let warned = call(&data, token, json!([]), scene.clone());
        assert_eq!(warned["kind"], "callback.log", "{warned}");
        assert_eq!(
            warned["message"],
            "WARNING: 'Rune platebody' is left on the ground, the pack is full with no food to drop",
            "{warned}"
        );
        assert_eq!(warned["token"], json!(token), "{warned}");
        assert!(!warned.to_string().contains("clue solved"), "{warned}");

        // The WARNING ended the collect: the same page finishes next call
        // rather than Taking, and the token dies with the landed abort.
        let finished = call(&data, token, json!([]), scene.clone());
        assert_eq!(finished["kind"], "aborted", "{finished}");
        assert_eq!(finished["reason"], "none-held", "{finished}");
        let after = call(&data, token, json!([]), scene);
        assert_eq!(after["reason"], "stale", "{after}");
    }

    /// The pack's fullness is the posted `inv_size`: a page that did not post
    /// it is a wait, and the client's 28 is never invented for it.
    #[test]
    fn a_missing_inv_size_waits_and_never_invents_the_default_pack() {
        on_reset();
        let data = selected();
        let token = opened(&data, SEXTANT_CASKET);
        let loot = json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]);
        // Twenty-eight occupied rows and still no posted slot count.
        let full_looking: Vec<Value> = (0..28)
            .map(|slot| inv(1200 + slot, "Rune scimitar", 1))
            .collect();
        for size in [json!(null), json!("28"), json!(28.5)] {
            let waiting = call(
                &data,
                token,
                json!([]),
                pages(loot.clone(), json!(full_looking), size, json!(-1)),
            );
            assert_eq!(waiting["kind"], "wait", "{waiting}");
            assert_eq!(waiting["token"], json!(token), "{waiting}");
        }
        // The posted count is honoured: one occupied row in a one-slot pack
        // is full, so the missing-food WARNING is what comes next.
        let warned = call(
            &data,
            token,
            json!([]),
            pages(
                loot,
                json!([inv(1200, "Rune scimitar", 1)]),
                json!(1),
                json!(-1),
            ),
        );
        assert_eq!(warned["kind"], "callback.log", "{warned}");
    }

    /// The main modal is closed only while this call's page posts it open: an
    /// omitted slot is not a close, and the posted `-1` is the closed state.
    #[test]
    fn a_posted_main_is_closed_and_an_omitted_slot_is_not() {
        on_reset();
        let data = selected();
        let token = opened(&data, SEXTANT_CASKET);
        let loot = json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]);

        // Omitted: the page never posted a modal, so there is nothing to close
        // and the Take is the step.
        let omitted = call(
            &data,
            token,
            json!([]),
            json!({ "here": loot_tile(), "ground": loot.clone(), "inv": [], "inv_size": 28 }),
        );
        assert_eq!(omitted["kind"], "obj", "{omitted}");
        // A malformed id is not a close either.
        let malformed = call(
            &data,
            token,
            json!([]),
            pages(loot.clone(), json!([]), json!(28), json!("6960")),
        );
        assert_eq!(malformed["kind"], "obj", "{malformed}");
        // Posted open: the close is the step, whatever the id is.
        for id in [json!(6960), json!(0), json!(-2)] {
            let closed = call(
                &data,
                token,
                json!([]),
                pages(loot.clone(), json!([]), json!(28), id.clone()),
            );
            assert_eq!(closed["kind"], "close-modal", "{id} {closed}");
        }
    }

    /// Freeze and yield beat the collect arm exactly as they beat the landed
    /// verbs: no close, no Take, no Drop, and the token lives.
    #[test]
    fn freeze_and_yield_beat_the_collect() {
        on_reset();
        let data = selected();
        let token = opened(&data, SEXTANT_CASKET);
        let scene = pages(
            json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
            json!([]),
            json!(28),
            json!(6960),
        );

        // Frozen before the entry call: the first Collecting tick has not run,
        // so the freeze is a plain wait and nothing else.
        on_pause();
        let paused = call(&data, token, json!([]), scene.clone());
        assert_eq!(paused["kind"], "wait", "{paused}");
        assert!(!bound_armed(), "a frozen call enters nothing");
        on_resume();
        on_hold(true);
        let held_clock = call(&data, token, json!([]), scene.clone());
        assert_eq!(held_clock["kind"], "wait", "{held_clock}");
        on_hold(false);
        // The posted `hold || ours` interrupt, unfrozen: yield, still no verb.
        let mut interrupted = scene.clone();
        interrupted["hold"] = json!(true);
        let yielded = call(&data, token, json!([]), interrupted);
        assert_eq!(yielded["kind"], "yield", "{yielded}");
        assert_eq!(yielded["token"], json!(token), "{yielded}");
        for step in [&paused, &held_clock, &yielded] {
            for absent in ["action", "name", "x", "z", "level", "message", "id"] {
                assert!(step.get(absent).is_none(), "{absent} {step}");
            }
        }
        // Thawed and unheld, the collect picks the page up where it left it.
        let closed = call(&data, token, json!([]), scene);
        assert_eq!(closed["kind"], "close-modal", "{closed}");
        assert!(bound_armed(), "the entry arms the window once");
    }

    /// A collect that enters and then loses the selected pin, or the family,
    /// is still the landed refusal — the seam is `none-held` alone.
    #[test]
    fn the_collect_seam_is_none_held_alone() {
        on_reset();
        let data = selected();
        let token = opened(&data, SEXTANT_CASKET);
        let scene = pages(
            json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
            json!([]),
            json!(28),
            json!(-1),
        );
        // The first Collecting call, so the phase and its window are live.
        assert_eq!(
            call(&data, token, json!([]), scene)["kind"],
            "obj",
            "the token is Collecting"
        );
        let gone = dispatch(None, &payload("next", Some(token), json!([]), json!({})));
        assert_eq!(gone["kind"], "aborted", "{gone}");
        assert_eq!(gone["reason"], "missing-selected-data", "{gone}");
        assert!(!bound_armed(), "the refusal clears the window");
        let after = call(&data, token, json!([]), json!({}));
        assert_eq!(after["reason"], "stale", "{after}");
    }

    /// The whole collect arm's outcome set: verbs, waits, logs and the landed
    /// abort, over a fully posted scene. Never `done`, never `'clue solved'`,
    /// never an abandon token, and never `ownsEquipment`.
    #[test]
    fn the_collect_arm_emits_no_completion() {
        on_reset();
        let data = selected();
        let token = opened(&data, EASY_CASKET);
        let steps = vec![
            call(
                &data,
                token,
                json!([]),
                pages(json!([]), json!([]), json!(28), json!(6960)),
            ),
            call(
                &data,
                token,
                json!([]),
                pages(
                    json!([ground(900, "Rune platebody", 3222, 3223, 1, &["Take"])]),
                    json!([]),
                    json!(28),
                    json!(-1),
                ),
            ),
            call(
                &data,
                token,
                json!([]),
                pages(json!([]), json!([]), json!(28), json!(-1)),
            ),
            call(
                &data,
                token,
                json!([]),
                pages(json!([]), json!([]), json!(28), json!(-1)),
            ),
        ];
        for step in &steps {
            let text = step.to_string();
            for forbidden in [
                "clue solved",
                "done",
                "abandon",
                "supplies-needed",
                "dead",
                "guardian-lost",
                "grind-ready",
                "ownsEquipment",
                "trail complete",
                "the trail is complete",
            ] {
                assert!(!text.contains(forbidden), "{forbidden} {step}");
            }
            assert!(step["status"].is_null(), "{step}");
            assert!(
                matches!(
                    step["kind"].as_str().unwrap_or(""),
                    "close-modal" | "obj" | "held" | "callback.log" | "callback.enabled" | "wait"
                ),
                "{step}"
            );
        }
        assert_eq!(
            steps
                .iter()
                .map(|step| step["kind"].clone())
                .collect::<Vec<_>>(),
            vec![
                json!("close-modal"),
                json!("obj"),
                json!("callback.log"),
                json!("wait"),
            ],
            "{steps:?}"
        );
    }

    /// The dig membership is the selected-param classify, not the frozen
    /// `type`: a decodable `trail_coord` on a row with no `trail_loc`,
    /// `trail_sextant=yes`, no `trail_guardian` and an `access` that is not
    /// `"constrained"`. Twenty rows on both pins, with no swallow of a search
    /// row and no leak of a guarded, packed, coord-only or desc-only one.
    #[test]
    fn the_unguarded_dig_membership_is_the_selected_param_set() {
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let data = api::game_data::for_revision(revision).expect("selected data");
            let facts = data.trails().expect("trails");
            // The pinned decode: 2801's selected token is (3160, 3251, 0).
            assert_eq!(
                dig_tile(row(&data, UNGUARDED)),
                Some(Tile {
                    x: 3160,
                    z: 3251,
                    level: 0
                }),
                "{revision:?}"
            );
            let members: Vec<i32> = facts
                .rows
                .iter()
                .filter(|row| dig_tile(row).is_some())
                .map(|row| row.id)
                .collect();
            assert_eq!(
                members,
                vec![
                    2801, 2803, 2805, 2807, 2809, 2811, 2813, 2815, 2817, 2819, 2821, 2823, 2825,
                    3582, 3584, 3586, 3588, 3590, 3592, 3594,
                ],
                "{revision:?}"
            );
            // No swallow: the 58 search rows are the other classify.
            let searchable = facts
                .rows
                .iter()
                .filter(|row| search_tile(row).is_some())
                .collect::<Vec<_>>();
            assert_eq!(searchable.len(), 58, "{revision:?}");
            for row in &searchable {
                assert_eq!(dig_tile(row), None, "{revision:?} {}", row.alias);
            }
            assert_eq!(search_tile(row(&data, UNGUARDED)), None, "{revision:?}");
            // No leak: the guarded rows and the constrained clue are the same
            // sextant shape, the map and the riddle carry a coord without the
            // pin, and the paramless 2722 and every casket stay out.
            for id in [
                GUARDED,
                CLUE,
                MAP,
                RIDDLE,
                MAP_EMPTY,
                CASKET,
                SEXTANT_CASKET,
            ] {
                assert_eq!(dig_tile(row(&data, id)), None, "{revision:?} {id}");
            }
            // The guarded 30 are the sextant rows that carry a guardian: the
            // sextant param alone is never the membership.
            let guarded = facts
                .rows
                .iter()
                .filter(|row| row.params.iter().any(|param| param.key == "trail_guardian"))
                .count();
            assert_eq!(guarded, 30, "{revision:?}");
        }
        // Synthetic rows: each half of the pin on its own idles, a loc param of
        // any value is not this membership, and an off-contract token is never
        // rounded into an invented coordinate.
        let param = |key: &str, value: &str| TrailParam {
            key: key.into(),
            value: value.into(),
        };
        let member = |params: Vec<TrailParam>, access: Option<&str>| TrailMembershipRow {
            alias: "trail_clue_test".into(),
            id: 1,
            role: "clue".into(),
            params,
            access: access.map(str::to_string),
        };
        let sextant = || param("trail_sextant", "yes");
        let coord = || param("trail_coord", "0_49_50_24_51");
        let hit = Some(Tile {
            x: 3160,
            z: 3251,
            level: 0,
        });
        assert_eq!(dig_tile(&member(vec![coord()], None)), None);
        assert_eq!(dig_tile(&member(vec![sextant()], None)), None);
        assert_eq!(dig_tile(&member(vec![sextant(), coord()], None)), hit);
        for blocked in [
            vec![param("trail_loc", "^true"), sextant(), coord()],
            vec![param("trail_loc", "^false"), sextant(), coord()],
            vec![param("trail_guardian", "trail_hard"), sextant(), coord()],
            vec![param("trail_sextant", "no"), coord()],
            vec![sextant(), param("trail_coord", "0_49_50_24")],
        ] {
            assert_eq!(dig_tile(&member(blocked, None)), None);
        }
        // `access` is read as the one constrained bound it is: any other value
        // — and a row that was posted with none — is outside the refusal.
        assert_eq!(
            dig_tile(&member(vec![sextant(), coord()], Some("constrained"))),
            None
        );
        assert_eq!(
            dig_tile(&member(vec![sextant(), coord()], Some("open"))),
            hit
        );
    }

    /// The Dig identity is the selected-verified display the host resolves by
    /// first name match — not an item id, not the membership alias.
    #[test]
    fn the_dig_identity_is_the_selected_spade_display() {
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let data = api::game_data::for_revision(revision).expect("selected data");
            let spade = data.item_by_id(SPADE_ITEM).expect("spade item");
            assert_eq!(spade.name.as_deref(), Some(SPADE_NAME), "{revision:?}");
        }
    }

    /// `Steady` on an unguarded-dig row: walk to the decoded tile until the
    /// posted `here` arrives, then Dig with the held Spade — repeating while
    /// that same clue stays held, and never collecting.
    #[test]
    fn an_unguarded_dig_row_walks_to_the_decoded_tile_and_then_digs_the_spade() {
        on_reset();
        let data = selected();
        let page = json!([[UNGUARDED, 1]]);
        let token = steady(&data, UNGUARDED);
        // Not arrived: the walk is the decoded tile, and it repeats. Another
        // level and two tiles along one axis are both not arrival.
        for far in [
            here(3100, 3300, 0),
            here(3160, 3251, 1),
            here(3162, 3251, 0),
            here(3160, 3253, 0),
        ] {
            let walk = call(&data, token, page.clone(), json!({ "here": far }));
            assert_eq!(walk["kind"], "walk", "{walk}");
            assert_eq!(walk["x"], 3160, "{walk}");
            assert_eq!(walk["z"], 3251, "{walk}");
            assert_eq!(walk["level"], 0, "{walk}");
            assert_eq!(token_of(&walk), token, "{walk}");
        }
        // No posted `here` at all: no arrival claim and no blind walk, even
        // with the Spade posted.
        let no_tile = call(
            &data,
            token,
            page.clone(),
            json!({ "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)] }),
        );
        assert_eq!(no_tile["kind"], "wait", "{no_tile}");
        // Arrived: the held Spade is the Dig, and it repeats while this same
        // clue id stays held.
        for here_tile in [here(3160, 3251, 0), here(3161, 3250, 0)] {
            let dig = call(
                &data,
                token,
                page.clone(),
                json!({ "here": here_tile, "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)] }),
            );
            assert_eq!(dig["kind"], "held", "{dig}");
            assert_eq!(dig["name"], "Spade", "{dig}");
            assert_eq!(dig["action"], "Dig", "{dig}");
            assert_eq!(token_of(&dig), token, "{dig}");
            // The verb is the item identity alone: no bound row id and no tile.
            for absent in ["id", "x", "z", "level", "message"] {
                assert!(dig.get(absent).is_none(), "{absent} {dig}");
            }
        }
        // The clue left: the landed `none-held` abort, and the collect seam is
        // never armed by a Dig — Collecting stays the casket Open's alone.
        let gone = call(&data, token, json!([]), json!({}));
        assert_eq!(gone["kind"], "aborted", "{gone}");
        assert_eq!(gone["reason"], "none-held", "{gone}");
        assert!(!bound_armed(), "a dig row never arms the reward window");
        let after = call(&data, token, json!([]), json!({}));
        assert_eq!(after["reason"], "stale", "{after}");
    }

    /// Arrived without the Spade on this call's posted pack page is a `wait`:
    /// never `abandon`, never `supplies-needed`, never a public `no-spade`, and
    /// never a fetch. The token stays live for the page that posts it.
    #[test]
    fn an_unguarded_dig_row_without_the_spade_waits_and_keeps_its_token() {
        on_reset();
        let data = selected();
        let page = json!([[UNGUARDED, 1]]);
        let token = steady(&data, UNGUARDED);
        let arrived = here(3160, 3251, 0);
        for pack in [
            // No page at all, an empty page, another item, a zero count, a
            // nameless row and a name that is not the display.
            json!([]),
            json!([inv(385, "Shark", 5)]),
            json!([inv(SPADE_ITEM, SPADE_NAME, 0)]),
            json!([inv(SPADE_ITEM, "", 1)]),
            json!([inv(SPADE_ITEM, "Spade cert", 1)]),
            json!([json!({ "id": SPADE_ITEM, "count": 1 })]),
        ] {
            let idle = call(
                &data,
                token,
                page.clone(),
                json!({ "here": arrived.clone(), "inv": pack }),
            );
            assert_eq!(idle["kind"], "wait", "{idle}");
            assert_eq!(token_of(&idle), token, "{idle}");
            let text = idle.to_string();
            for forbidden in [
                "no-spade",
                "abandon",
                "supplies-needed",
                "done",
                "clue solved",
            ] {
                assert!(!text.contains(forbidden), "{idle}");
            }
        }
        // A malformed `here` is the same wait, and the whole `inv` slot may be
        // omitted: neither is a verb.
        let malformed = call(
            &data,
            token,
            page.clone(),
            json!({ "here": json!({ "x": 3160 }), "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)] }),
        );
        assert_eq!(malformed["kind"], "wait", "{malformed}");
        // The page that carries it: the Dig, and the token was live all along.
        // The identity is the posted display name, matched the way the landed
        // collect matches one, so the cert id and the case both still Dig.
        let dig = call(
            &data,
            token,
            page,
            json!({
                "here": here(3160, 3251, 0),
                "inv": [inv(385, "Shark", 5), inv(953, "sPaDe", 1)],
            }),
        );
        assert_eq!(dig["kind"], "held", "{dig}");
        assert_eq!(dig["name"], "Spade", "{dig}");
        assert_eq!(dig["action"], "Dig", "{dig}");
    }

    /// The rows the dig classify leaves out stay identified then idle even over
    /// a scene the dig arm would walk and Dig from — `here` on the row's own
    /// selected tile with the Spade posted: the guarded row whose first Dig
    /// would spawn a wizard, the packed constrained 3554 clue, the coord-only
    /// map and the paramless 2722.
    #[test]
    fn guarded_and_coord_only_rows_stay_idle_over_a_walkable_dig_scene() {
        on_reset();
        let data = selected();
        for id in [GUARDED, CLUE, MAP, MAP_EMPTY] {
            let here_tile = row(&data, id)
                .params
                .iter()
                .find(|param| param.key == "trail_coord")
                .and_then(|param| decode_trail_coord(&param.value))
                .unwrap_or(Tile {
                    x: 3160,
                    z: 3251,
                    level: 0,
                });
            let scene = json!({
                "here": here(here_tile.x, here_tile.z, here_tile.level),
                "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)],
            });
            let page = json!([[id, 1]]);
            let token = steady(&data, id);
            for _ in 0..2 {
                let idle = call(&data, token, page.clone(), scene.clone());
                assert_eq!(idle["kind"], "wait", "{id} {idle}");
                assert_eq!(token_of(&idle), token, "{id} {idle}");
                assert!(idle.get("x").is_none(), "{id} {idle}");
                assert!(idle.get("action").is_none(), "{id} {idle}");
            }
        }
    }

    /// Freeze and yield beat the dig arm the way they beat the landed verbs: no
    /// walk and no held ride along, and the token lives for the thaw.
    #[test]
    fn freeze_and_yield_beat_the_dig() {
        on_reset();
        let data = selected();
        let page = json!([[UNGUARDED, 1]]);
        let token = steady(&data, UNGUARDED);
        let scene = json!({ "here": here(3160, 3251, 0), "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)] });
        on_pause();
        let paused = call(&data, token, page.clone(), scene.clone());
        assert_eq!(paused["kind"], "wait", "{paused}");
        on_resume();
        on_hold(true);
        let held_clock = call(&data, token, page.clone(), scene.clone());
        assert_eq!(held_clock["kind"], "wait", "{held_clock}");
        on_hold(false);
        let mut yield_scene = scene.clone();
        yield_scene["hold"] = json!(true);
        let yielded = call(&data, token, page.clone(), yield_scene);
        assert_eq!(yielded["kind"], "yield", "{yielded}");
        assert_eq!(token_of(&yielded), token, "{yielded}");
        for step in [&paused, &held_clock, &yielded] {
            assert!(step.get("name").is_none(), "{step}");
            assert!(step.get("action").is_none(), "{step}");
            assert!(step.get("x").is_none(), "{step}");
            assert!(step.get("z").is_none(), "{step}");
        }
        // Thawed and unheld, the Dig is still there.
        let dig = call(&data, token, page, scene);
        assert_eq!(dig["kind"], "held", "{dig}");
        assert_eq!(dig["action"], "Dig", "{dig}");
    }

    /// Identify is casket-first, so the casket a dig produced is the step: the
    /// dig row re-arms the gate for it, the Open follows, and the clue's return
    /// re-arms again — never a second Dig under the casket.
    #[test]
    fn a_produced_casket_opens_and_the_dig_row_re_arms() {
        on_reset();
        let data = selected();
        let clue_page = json!([[UNGUARDED, 1]]);
        let token = steady(&data, UNGUARDED);
        let scene = json!({ "here": here(3160, 3251, 0), "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)] });
        let dig = call(&data, token, clue_page.clone(), scene.clone());
        assert_eq!(dig["kind"], "held", "{dig}");
        assert_eq!(dig["action"], "Dig", "{dig}");

        // The casket the dig produced is held beside its own clue.
        let casket = casket_of(&data, UNGUARDED);
        let casket_alias = row(&data, casket).alias.clone();
        let both = json!([[UNGUARDED, 1], [casket, 1]]);
        let re_armed = call(&data, token, both.clone(), scene.clone());
        assert_eq!(re_armed["kind"], "callback.enabled", "{re_armed}");
        assert_eq!(token_of(&re_armed), token, "{re_armed}");
        let logged = call(&data, token, both.clone(), json!({ "resume": true }));
        assert_eq!(logged["kind"], "callback.log", "{logged}");
        let message = logged["message"].as_str().unwrap_or("");
        assert!(message.contains(&casket_alias), "{logged}");
        assert!(!message.contains(" [2801]"), "{logged}");
        assert_eq!(
            call(&data, token, both.clone(), scene.clone())["kind"],
            "callback.setStatus"
        );
        let open = call(&data, token, both.clone(), scene.clone());
        assert_eq!(open["kind"], "held", "{open}");
        assert_eq!(open["name"], "Casket", "{open}");
        assert_eq!(open["action"], "Open", "{open}");

        // And the clue alone again: the dig row re-arms rather than Digging
        // under whatever the casket left behind.
        let back = call(&data, token, clue_page, scene);
        assert_eq!(back["kind"], "callback.enabled", "{back}");
    }

    /// The dig row emits walk, held Dig, wait or yield only — never a
    /// completion, and never a `status` field of its own.
    #[test]
    fn the_dig_row_emits_only_walk_held_and_wait_and_never_a_completion() {
        on_reset();
        let data = selected();
        let page = json!([[UNGUARDED, 1]]);
        let token = steady(&data, UNGUARDED);
        let steps = vec![
            call(
                &data,
                token,
                page.clone(),
                json!({ "here": here(3100, 3300, 0) }),
            ),
            call(
                &data,
                token,
                page.clone(),
                json!({ "here": here(3160, 3251, 0) }),
            ),
            call(
                &data,
                token,
                page.clone(),
                json!({
                    "here": here(3160, 3251, 0),
                    "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)],
                }),
            ),
            call(
                &data,
                token,
                page,
                json!({
                    "here": here(3160, 3251, 0),
                    "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)],
                    "hold": true,
                }),
            ),
        ];
        for step in &steps {
            let text = step.to_string();
            for forbidden in [
                "clue solved",
                "done",
                "abandon",
                "supplies-needed",
                "dead",
                "guardian-lost",
                "grind-ready",
                "no-spade",
                "ownsEquipment",
            ] {
                assert!(!text.contains(forbidden), "{forbidden} {step}");
            }
            assert!(step["status"].is_null(), "{step}");
            assert!(
                matches!(
                    step["kind"].as_str().unwrap_or(""),
                    "walk" | "held" | "wait" | "yield"
                ),
                "{step}"
            );
        }
        assert_eq!(
            steps
                .iter()
                .map(|step| step["kind"].clone())
                .collect::<Vec<_>>(),
            vec![json!("walk"), json!("wait"), json!("held"), json!("yield")],
            "{steps:?}"
        );
    }
}
