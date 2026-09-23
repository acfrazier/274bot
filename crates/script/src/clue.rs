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
//! This is the search, casket-open, unguarded-dig, guarded-dig encounter and
//! trail-end collect slice and nothing else: no talk, puzzle, deposit, retry
//! or return-grind. A held row that is a selected search membership — a
//! selected `trail_loc=^true` **and** a decodable selected `trail_coord` on
//! the same row — walks to its decoded tile and then dispatches the
//! Search/Open picker over the posted loc page; both verbs are enqueued by the
//! wrapper as `InteractReq::Walk` / `InteractReq::Loc`. The picker is the
//! frozen one, minus its `walkLeg`: nearest then action rank, always at the
//! row's own posted tile and id.
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
//! The guarded sibling of that pin is the encounter. A decodable selected
//! `trail_coord` on a row with no `trail_loc`, `trail_sextant=yes`, a
//! `trail_guardian` and an `access` that is not `"constrained"` is the guarded
//! membership — the thirty hard sextant rows, `2723` among them. Its first Dig
//! spawns the family wizard the cap documents (`trail_hard` → Zamorak Wizard,
//! `trail_hard2` → Saradomin Wizard), so the walk-then-Dig is only the spawn:
//! after it the machine observes the posted npc page first and only then
//! raises Protect from Magic with the selected `if-button` while the
//! marshalled overlay varp says the overlay is not already up — a call with no
//! matching spawn posted clicks nothing and Attacks nothing — and enqueues one
//! `Attack` for a posted npc from that family on the posted `here` level
//! inside the frozen radius, measured by the posted `distance` when the row
//! carried one and by the row's own posted tile otherwise — preferring the row
//! that targets the player, else the nearest match, and never the nearest
//! anything. It then waits for the kill: that owned index posted at zero
//! health while the page still shows this token's fight on it, or the owned
//! index leaving the page inside the frozen grace. Only a kill walks back to
//! the decoded tile and Digs again, and that Dig repeats while the same clue
//! stays held. An index this token never Attacked is never a kill: a
//! disappearance without one waits, an index gone outside the grace waits, and
//! this machine has no `guardian-lost`.
//! A freeze reclaims the owned last-seen on the thaw the way the landed hunt
//! fight shifts its own stamps, so a frozen session never spends the grace.
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
//! coord-only map rows, the desc-only key-gated riddles and the empty-params
//! 2722 — is identified and then idled: no action and no walk.
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
use std::time::{Duration, Instant};

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

/// The frozen `Guardian.ts` spawn radius: a posted npc further than this many
/// tiles from the posted `here` is not a wizard this Dig's spawn can be
/// observed through, and the encounter waits rather than reaching for it. The
/// measure is the posted `distance` when the row carried one and the row's own
/// posted tile otherwise, always on the posted `here` level.
const GUARDIAN_RADIUS: i32 = 12;

/// The frozen `KILL_GRACE_MS` the landed hunt fight reads its engaged index
/// through: an owned index may leave the posted page inside this window and
/// still be this token's kill. Read against the owned wizard's last-seen,
/// which the thaw reclaim shifts by the frozen gap, so a paused or held
/// session never spends it and a freeze longer than the remaining grace still
/// ends in the kill.
const KILL_GRACE_MS: u64 = 6_000;

/// The frozen npc action the wizard is Attacked with, matched on the posted
/// action strings the way the landed loc picker matches its own.
const ATTACK: &str = "Attack";

/// The one selected `cap.prayer` row this fight raises, looked up in the
/// selected table the way the landed `api::prayer::lookup` looks one up. Not a
/// second prayer table: the component id the click carries is that row's own.
const PROTECT_FROM_MAGIC: &str = "Protect from Magic";

/// A posted `SceneEntity.target_kind` of `2` is a player, so a row whose
/// `target_kind` is this and whose `target_index` is this token's `self_slot`
/// is a row whose own posted target is the local player.
const PLAYER_KIND: i32 = 2;

/// A posted `self_target_kind` of `1` is an npc, so the local player's own
/// posted target is an npc whose index can be compared with an owned one.
const NPC_KIND: i32 = 1;

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
    /// its Open from here, a search row walks then dispatches the picker, an
    /// unguarded-dig row walks then Digs with the held Spade, and a guarded
    /// row walks, Digs its spawn and then fights the wizard that Dig spawned
    /// until the kill lets it walk back and Dig again; every other row idles
    /// with no action and no walk, and no second callback is emitted for this
    /// step.
    Steady,
    /// Trail-end collect: `Steady` on the step whose casket Open went out, and
    /// this call's identify is `none-held`. The one phase that survives
    /// `none-held` — the token lives and the loot verbs are dispatched from
    /// here, one per call, until the same `none-held` abort ends the session.
    Collecting,
}

/// The live guarded encounter on the identified clue row: the wizard this
/// token's first Dig spawned, and the kill the fight waits to observe. Session
/// state on the live token, like `open` — never a second scheduler, never a
/// `Phase::Fighting`, and never a cached npc page.
struct Guardian {
    /// The posted index this token enqueued `Attack` for. `None` until that
    /// Attack goes out, and dropped again on the kill: an index this token
    /// never Attacked is never a kill.
    owned: Option<Owned>,
    /// The kill was observed: the walk back to the decoded tile and its Dig
    /// replace the fight, and that Dig repeats while the same clue stays held.
    post_kill: bool,
}

/// One owned wizard: the posted scene index this token enqueued `Attack` for,
/// and the last call that index was posted on. The posted name rides the verb
/// and is never kept — nothing is Attacked twice and no name is ever copied
/// onto a row.
struct Owned {
    index: i32,
    seen_at: Instant,
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
    /// The live guarded encounter: absent until this token's first Dig on a
    /// guarded row went out, then owned by this token until a different held
    /// row, an abort or the frozen reset clears it. Like `open`, it is the
    /// session's own state and never a world copy.
    guardian: Option<Guardian>,
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
            guardian: None,
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

    /// The hunt reclaim shape, over this session's own stamps: a freeze that
    /// thaws moves them forward by the frozen gap, so a duration that spans
    /// the freeze is not spent by it. `InstantTaskClock` reclaims its own
    /// `deadline` alone, and the guarded grace reads the owned wizard's own
    /// last-seen — a raw `Instant` — so without this a freeze longer than the
    /// remaining grace would burn the window and the fight would wait forever.
    fn apply_freeze(&mut self, paused: bool, held: bool) {
        let was = self.clock.frozen();
        let frozen_at = self.clock.frozen_at;
        self.clock.set_freeze(paused, held);
        if was && !self.clock.frozen() {
            if let Some(at) = frozen_at {
                let gap = Instant::now().saturating_duration_since(at);
                self.shift_instants(gap);
            }
        }
    }

    /// The owned wizard's last-seen is the one stamp this machine reads a
    /// duration from, so it is the one a thaw shifts: the grace then measures
    /// only the time the session was actually unfrozen and observing.
    fn shift_instants(&mut self, gap: Duration) {
        if let Some(owned) = self.guardian.as_mut().and_then(|g| g.owned.as_mut()) {
            owned.seen_at += gap;
        }
    }

    /// The step-scoped state a re-arm, a leave and an abort all drop: the
    /// dispatched Open with its `hard` capture, the collect deadline, the
    /// discarded ground ids, the settled-Take watch and the guarded
    /// encounter's owned wizard and post-kill flag.
    fn clear_step(&mut self) {
        self.open = None;
        self.clock.deadline = None;
        self.discarded.clear();
        self.pending_take = None;
        self.full_no_food = false;
        self.guardian = None;
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
                // Not a held casket: the landed search dispatch, the guarded
                // encounter, the sibling unguarded-dig dispatch, or the idle
                // every other row keeps.
                None => self.steady(row, input, selected),
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
            return self.walk(tile);
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
    /// the guarded encounter, the sibling unguarded-dig dispatch, or the idle
    /// every other row keeps.
    ///
    /// The search pin decides first: `search_tile` is the `trail_loc=^true`
    /// membership and the landed dispatch re-reads its own tile from it, so a
    /// search row can never reach either dig arm and Dig is never a second
    /// search classify. The guarded pin decides next, and the encounter it
    /// picked up — session state on this same token — is what the following
    /// calls read. Every other held type still idles exactly as before.
    fn steady(
        &mut self,
        row: &TrailMembershipRow,
        input: &Value,
        selected: Option<&SelectedGameData>,
    ) -> Value {
        if search_tile(row).is_some() {
            return self.search(row, input);
        }
        if let Some(tile) = guarded_tile(row) {
            return self.guarded(row, tile, input, selected);
        }
        match dig_tile(row) {
            Some(tile) => self.dig(tile, input),
            None => self.emit("wait"),
        }
    }

    /// `Steady` on an identified guarded row: the first Dig, the fight, or the
    /// post-kill redig.
    ///
    /// The encounter is this row's own session state: absent until this token's
    /// first Dig went out, present until a different held row, an abort or the
    /// reset clears it. Nothing else about it is cached — every call re-reads
    /// this call's marshalled pages, and the decoded tile is re-read from the
    /// row's own selected `trail_coord` rather than kept.
    fn guarded(
        &mut self,
        row: &TrailMembershipRow,
        tile: Tile,
        input: &Value,
        selected: Option<&SelectedGameData>,
    ) -> Value {
        if self.guardian.is_none() {
            // No encounter yet: the landed walk-then-Dig, and the Dig that
            // goes out is the spawn.
            return self.spawn(tile, input);
        }
        if self.guardian.as_ref().is_some_and(|g| g.post_kill) {
            // The kill was observed: walk back to the decoded tile and Dig
            // again, repeating while this same clue stays held.
            return self.dig(tile, input);
        }
        self.fight(guardian_names(row), tile, input, selected)
    }

    /// The guarded row's first Dig: the landed walk-then-Dig of the sibling
    /// unguarded arm, and the verb that spawns the wizard. The encounter is
    /// created from that Dig and from nothing else — a call that is still
    /// walking, has no posted `here`, or has no posted Spade waits without one
    /// and the fight never starts early.
    fn spawn(&mut self, tile: Tile, input: &Value) -> Value {
        match arrival(tile, input) {
            Arrival::Unknown => self.emit("wait"),
            Arrival::Walking => self.walk(tile),
            Arrival::Arrived if !spade_posted(input) => self.emit("wait"),
            Arrival::Arrived => {
                self.guardian = Some(Guardian {
                    owned: None,
                    post_kill: false,
                });
                self.dig_verb()
            }
        }
    }

    /// The fight after the spawn: the frozen mid-fight hitpoints wait, this
    /// call's posted-npc observation, and only then the Protect from Magic
    /// overlay and the Attack.
    ///
    /// The npc page is this call's own marshalling of `host().snapshot.npcs`,
    /// so nothing is cached and no scene is scanned. The spawn is observed
    /// **before** anything is raised: the row this token already owns, or —
    /// before that Attack — a posted index of the row family's cap-documented
    /// names inside the frozen radius. A call with neither waits with no click
    /// and no Attack, so the click is only ever enqueued behind a matching
    /// spawn and the nearest anything is never Attacked. The encounter then
    /// owns the Attacked index: the kill is the owned row posted at zero
    /// health while the page still shows this token's fight on it, or the
    /// owned index leaving the page inside the frozen grace. An index this
    /// token never Attacked is never a kill, a disappearance outside the grace
    /// is a `wait`, and this machine has no `guardian-lost`.
    fn fight(
        &mut self,
        names: &'static [&'static str],
        tile: Tile,
        input: &Value,
        selected: Option<&SelectedGameData>,
    ) -> Value {
        if posted_i32(input, "hitpoints").is_some_and(|hp| hp <= 0) {
            // The posted effective hitpoints at zero: the frozen mid-fight
            // wait, with no verb at all and never a public `dead`. A page that
            // posted no stat at all is not a zero.
            return self.emit("wait");
        }
        let Some(page) = input.get("npcs").and_then(Value::as_array) else {
            // No posted npc page this call: no spawn is posted, so there is
            // nothing to observe and nothing to raise the overlay for. The
            // spawn wait and the kill wait are the same `wait`.
            return self.emit("wait");
        };
        let self_slot = posted_i32(input, "self_slot");
        let now = self.clock.now();
        let owned = self
            .guardian
            .as_ref()
            .and_then(|g| g.owned.as_ref())
            .map(|owned| owned.index);
        if let Some(index) = owned {
            // The wizard this token Attacked, read before the overlay: a kill
            // never clicks, and a page that no longer posts the spawn is not a
            // reason to raise the prayer.
            let Some(row) = page
                .iter()
                .find(|row| posted_i32(row, "index") == Some(index))
            else {
                // The owned index left the page. Inside the frozen grace that
                // is this token's kill; outside it the fight waits — never a
                // `guardian-lost`.
                let within = self
                    .guardian
                    .as_ref()
                    .and_then(|g| g.owned.as_ref())
                    .is_some_and(|owned| {
                        now.saturating_duration_since(owned.seen_at)
                            < Duration::from_millis(KILL_GRACE_MS)
                    });
                if !within {
                    return self.emit("wait");
                }
                self.kill();
                return self.dig(tile, input);
            };
            // Posted: this call's observation is the last-seen the grace reads.
            if let Some(owned) = self.guardian.as_mut().and_then(|g| g.owned.as_mut()) {
                owned.seen_at = now;
            }
            if died_owned(row, self_slot, self_target(input)) {
                self.kill();
                return self.dig(tile, input);
            }
            // Posted and alive: the fight is on, and the Attack is enqueued
            // once per owned index — this call only reads the overlay below
            // and never re-Attacks.
        }
        // The spawn this call raises the overlay for: the owned wizard just
        // observed, or — before that Attack — the posted family wizard the
        // Attack is about to own. A call with neither is the spawn wait.
        let spawn = if owned.is_some() {
            None
        } else {
            pick_npc(names, page, self_slot, input.get("here").and_then(posted_tile))
        };
        if owned.is_none() && spawn.is_none() {
            return self.emit("wait");
        }
        let Some(prayer) = selected.and_then(|data| api::prayer::lookup(data, PROTECT_FROM_MAGIC))
        else {
            // The one selected row this fight raises is not in the pin:
            // nothing is invented in its place and the fight waits.
            return self.emit("wait");
        };
        if posted_i32(input, "varp95") != Some(1) {
            // Not a proven on: the landed generic `if-button` raises the
            // row's own selected component. An overlay already up skips the
            // click, an unobserved one never Attacks, and nothing here nests
            // the prayer isolate or waits a toggle out.
            return json!({
                "kind": "if-button",
                "token": self.token,
                "component_id": prayer.button_com,
            });
        }
        match spawn {
            // The overlay reads up and the posted spawn is now this token's:
            // the one Attack for this index, carrying the posted name and the
            // posted scene index and nothing else.
            Some((index, name)) => {
                if let Some(guardian) = self.guardian.as_mut() {
                    guardian.owned = Some(Owned {
                        index,
                        seen_at: now,
                    });
                }
                json!({
                    "kind": "npc",
                    "token": self.token,
                    "name": name,
                    "action": ATTACK,
                    // The posted scene index, always present: the host
                    // matches that identity and refuses a stale one.
                    "index": index,
                })
            }
            // Owned and still posted: the fight waits for its kill.
            None => self.emit("wait"),
        }
    }

    /// The kill was observed: the owned index is dropped and the walk back
    /// with its Dig replaces the fight. Only an owned, settled read reaches
    /// here.
    fn kill(&mut self) {
        if let Some(guardian) = self.guardian.as_mut() {
            guardian.owned = None;
            guardian.post_kill = true;
        }
    }

    /// `Steady` on an identified guarded or unguarded-dig membership: walk to
    /// the decoded `trail_coord` tile, then Dig with the held Spade.
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
        match arrival(tile, input) {
            Arrival::Unknown => self.emit("wait"),
            Arrival::Walking => self.walk(tile),
            Arrival::Arrived if spade_posted(input) => self.dig_verb(),
            Arrival::Arrived => self.emit("wait"),
        }
    }

    /// The landed arrival walk to a tile: the same verb the search arm and both
    /// dig arms dispatch, so `walk` has one shape on this machine.
    fn walk(&self, tile: Tile) -> Value {
        json!({
            "kind": "walk",
            "token": self.token,
            "x": tile.x,
            "z": tile.z,
            "level": tile.level,
        })
    }

    /// The landed held Dig: the selected Spade display the host resolves by
    /// first name match, and the frozen action, with no row id and no tile.
    fn dig_verb(&self) -> Value {
        json!({
            "kind": "held",
            "token": self.token,
            "name": SPADE_NAME,
            "action": DIG,
        })
    }
}

/// What this call's posted `here` says about the decoded tile.
enum Arrival {
    /// No posted `here`: no arrival claim to make and no walk to measure.
    Unknown,
    /// Posted, and not this tile's level or not within `ARRIVE_RADIUS` of it.
    Walking,
    /// Posted on the tile's level and within `ARRIVE_RADIUS` of it.
    Arrived,
}

/// The landed arrival read over this call's posted `here`: the same level and
/// Chebyshev `ARRIVE_RADIUS` the search walk uses, so the guarded and unguarded
/// Dig arrive exactly the way their sibling search row does.
fn arrival(tile: Tile, input: &Value) -> Arrival {
    let Some(here) = input.get("here").and_then(posted_tile) else {
        return Arrival::Unknown;
    };
    if here.level == tile.level && chebyshev(here, tile) <= i64::from(ARRIVE_RADIUS) {
        Arrival::Arrived
    } else {
        Arrival::Walking
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

/// One posted integer field of a marshalled page row — an npc row's health,
/// distance or target, and the payload's own `self_slot`, `hitpoints` and
/// overlay varp. A missing, null or non-integer field is `None`: the machine
/// never rounds a posted value into the number it wants.
fn posted_i32(row: &Value, key: &str) -> Option<i32> {
    row.get(key).and_then(i32_of)
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

/// The identified row's guarded-dig membership: a decodable selected
/// `trail_coord` **and** no selected `trail_loc` **and** `trail_sextant=yes`
/// **and** a selected `trail_guardian` **and** an `access` that is not
/// `"constrained"`.
///
/// The sibling of `dig_tile`, not a fold into it: the guardian param is what
/// makes the first Dig a spawn, so the unguarded arm must never reach this
/// encounter and this arm must never Dig a row without one. The param value is
/// the family alias the thirty hard sextant rows carry; the wizard name it
/// stands for lives in `guardian_names` alone and is only ever a posted-name
/// filter, never a row field.
fn guarded_tile(row: &TrailMembershipRow) -> Option<Tile> {
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
    if located || !sextant || !guarded {
        return None;
    }
    decode_trail_coord(coord?)
}

/// The cap-documented wizard names the row's own `trail_guardian` family alias
/// stands for: `trail_hard` is the Zamorak Wizard and `trail_hard2` the
/// Saradomin Wizard.
///
/// The alias is a family and not an npc debugname, so this list is only ever
/// compared against a posted npc page **after** the first Dig. It is never
/// written onto the row, never used to invent a scene entity, and an unpinned
/// family has no list at all — the encounter then waits rather than Attacking
/// the nearest anything.
fn guardian_names(row: &TrailMembershipRow) -> &'static [&'static str] {
    let alias = row
        .params
        .iter()
        .find(|param| param.key == "trail_guardian")
        .map(|param| param.value.as_str());
    match alias {
        Some("trail_hard") => &["Zamorak Wizard"],
        Some("trail_hard2") => &["Saradomin Wizard"],
        _ => &[],
    }
}

/// The local player's own posted target pair: the posted `self_target_kind`
/// and `self_target_index`. Both are needed for the read, and a page that
/// posted neither is not a target at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelfTarget {
    kind: i32,
    index: i32,
}

/// This call's posted local-player target, or `None` when the page did not
/// post the pair.
fn self_target(input: &Value) -> Option<SelfTarget> {
    Some(SelfTarget {
        kind: posted_i32(input, "self_target_kind")?,
        index: posted_i32(input, "self_target_index")?,
    })
}

/// The frozen `targetsMe`: this posted row's own target is the local player.
/// A page that posted no `self_slot` is never a match — the machine does not
/// invent the zero slot.
fn targets_me(row: &Value, self_slot: Option<i32>) -> bool {
    posted_i32(row, "target_kind") == Some(PLAYER_KIND)
        && self_slot.is_some_and(|slot| posted_i32(row, "target_index") == Some(slot))
}

/// The mirror read: the local player's own posted target is this posted row.
/// A page that posted no pair is never a match.
fn we_target(row: &Value, target: Option<SelfTarget>) -> bool {
    target.is_some_and(|target| {
        target.kind == NPC_KIND && posted_i32(row, "index") == Some(target.index)
    })
}

/// The frozen `sawDeath` read: the owned row posted zero health beside a posted
/// maximum, and this call's page still shows this token's fight on it — the
/// row's own posted target is the player, or the player's own posted target is
/// that row. A row that merely died beside another player is not this token's
/// kill.
fn died_owned(row: &Value, self_slot: Option<i32>, target: Option<SelfTarget>) -> bool {
    posted_i32(row, "health") == Some(0)
        && posted_i32(row, "max_health").is_some_and(|max| max > 0)
        && (targets_me(row, self_slot) || we_target(row, target))
}

/// The frozen spawn filter over this call's posted npc page: a row whose
/// posted display name is one of the row family's cap-documented wizards,
/// whose posted actions carry `Attack`, and whose distance from this call's
/// posted `here` is inside the frozen radius on that same level. The row that
/// targets the player wins, else the nearest, then posted order — the scan
/// only replaces its best on a strict improvement, exactly like the landed
/// loc picker.
///
/// A row without an index, without a posted name, without the action, or
/// without the marshalled tile that `npc_distance` needs matches nothing, and
/// a page with no match picks nothing: the encounter waits rather than
/// Attacking the nearest anything. The name compared is the posted one and the
/// name the verb carries is that same posted string — no frozen debugname is
/// ever substituted for it.
fn pick_npc<'a>(
    names: &[&str],
    page: &'a [Value],
    self_slot: Option<i32>,
    here: Option<Tile>,
) -> Option<(i32, &'a str)> {
    let mut best: Option<(i32, &'a str, i64, bool)> = None;
    for row in page {
        let Some(index) = posted_i32(row, "index") else {
            continue;
        };
        let Some(name) = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        if !names.iter().any(|wanted| name.eq_ignore_ascii_case(wanted)) {
            continue;
        }
        if !posted_action(row, ATTACK) {
            continue;
        }
        let Some(distance) = npc_distance(row, here) else {
            continue;
        };
        if distance > i64::from(GUARDIAN_RADIUS) {
            continue;
        }
        let mine = targets_me(row, self_slot);
        let better = match &best {
            None => true,
            Some((_, _, best_distance, best_mine)) => {
                (mine && !*best_mine) || (mine == *best_mine && distance < *best_distance)
            }
        };
        if better {
            best = Some((index, name, distance, mine));
        }
    }
    best.map(|(index, name, _, _)| (index, name))
}

/// The distance the spawn filter reads for one posted npc row: the posted
/// `distance` when the row carried one, else the Chebyshev distance from this
/// call's posted `here` to the row's own posted `x`/`z` tile — the same
/// level-first `max(|dx|, |dz|)` measure the landed loc picker reads its own
/// rows with.
///
/// The same level is part of the membership whichever half the distance comes
/// from: a row on another level than the posted `here` is not the wizard this
/// Dig spawned. A call that posted no `here`, and a row that did not post the
/// marshalled `x`/`z`/`level` tile, have no distance here at all — `None`, so
/// the row matches nothing rather than being measured against an invented
/// base.
fn npc_distance(row: &Value, here: Option<Tile>) -> Option<i64> {
    let here = here?;
    if posted_i32(row, "level") != Some(here.level) {
        return None;
    }
    match posted_i32(row, "distance") {
        Some(distance) => Some(i64::from(distance)),
        None => Some(chebyshev(posted_tile(row)?, here)),
    }
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
        rt.borrow_mut().apply_freeze(true, held);
    });
}

pub fn on_resume() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().clock.held;
        rt.borrow_mut().apply_freeze(false, held);
    });
}

pub fn on_hold(held: bool) {
    RUNTIME.with(|rt| {
        let paused = rt.borrow().clock.paused;
        rt.borrow_mut().apply_freeze(paused, held);
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

    /// One synthetic membership row over the params a test names, so a
    /// classify can be read without the selected family.
    fn member(params: Vec<TrailParam>, access: Option<&str>) -> TrailMembershipRow {
        TrailMembershipRow {
            alias: "trail_clue_test".into(),
            id: 1,
            role: "clue".into(),
            params,
            access: access.map(str::to_string),
        }
    }

    /// One synthetic selected param on such a row.
    fn param(key: &str, value: &str) -> TrailParam {
        TrailParam {
            key: key.into(),
            value: value.into(),
        }
    }

    /// The decoded tile of the guarded exemplar `2723`: `0_47_60_50_44`.
    fn guarded_tile_of(data: &SelectedGameData) -> Tile {
        guarded_tile(row(data, GUARDED)).expect("guarded tile")
    }

    /// The marshalled scene the guarded encounter walks and Digs over: the
    /// posted `here` on (or off) the decoded tile and the pack that carries
    /// the Spade.
    fn dig_scene(here_tile: Value, spade: bool) -> Value {
        if spade {
            json!({ "here": here_tile, "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)] })
        } else {
            json!({ "here": here_tile })
        }
    }

    /// One wrapper-marshalled posted npc row on the decoded `here` tile: the
    /// posted index the Attack carries, the posted id, tile and `in_combat`
    /// flag the marshalled page always carries, the posted name the family
    /// filter matches, the posted health pair and target pair the kill is read
    /// through, and the posted distance the frozen radius prefers.
    fn npc(index: i32, name: &str, distance: i32, health: i32, max_health: i32) -> Value {
        json!({
            "index": index,
            "id": 100 + index,
            "name": name,
            "x": 3058,
            "z": 3884,
            "level": 0,
            "distance": distance,
            "health": health,
            "max_health": max_health,
            "in_combat": false,
            "actions": [ATTACK],
            "target_kind": 0,
            "target_index": -1,
        })
    }

    /// The same posted row without a posted distance: the Chebyshev read the
    /// spawn filter makes from the row's own posted tile and this call's
    /// `here`.
    fn tiled(index: i32, name: &str, x: i32, z: i32, level: i32) -> Value {
        let mut row = npc(index, name, 3, 10, 10);
        row.as_object_mut().expect("object").remove("distance");
        row["x"] = json!(x);
        row["z"] = json!(z);
        row["level"] = json!(level);
        row
    }

    /// One posted field of a marshalled npc row replaced.
    fn field(mut row: Value, key: &str, value: Value) -> Value {
        row[key] = value;
        row
    }

    /// One posted field of a marshalled npc row dropped: what a page that
    /// never posted it hands the machine.
    fn unfield(mut row: Value, key: &str) -> Value {
        row.as_object_mut().expect("object").remove(key);
        row
    }

    /// The same posted row with the npc's own target on the player.
    fn targeting(mut row: Value, slot: i32) -> Value {
        row["target_kind"] = json!(PLAYER_KIND);
        row["target_index"] = json!(slot);
        row
    }

    /// One fight call's pages: the posted npc page, the posted local-player
    /// slot, the Protect from Magic overlay the gate reads, and the posted
    /// effective hitpoints. The overlay is up and the player is healthy unless
    /// a test says otherwise, so each test names only the gate it is about.
    fn fight_scene(npcs: Value, extra: Value) -> Value {
        let mut scene = json!({
            "here": here(3058, 3884, 0),
            "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)],
            "npcs": npcs,
            "self_slot": 0,
            "varp95": 1,
            "hitpoints": 40,
        });
        for (key, value) in extra.as_object().expect("extra") {
            scene[key] = value.clone();
        }
        scene
    }

    /// Drive the guarded exemplar from `Steady` to its spawn: the walk, then
    /// the first Dig. The encounter is live after this and the fight pages
    /// begin.
    fn spawned(data: &SelectedGameData) -> u64 {
        let page = json!([[GUARDED, 1]]);
        let token = steady(data, GUARDED);
        let tile = guarded_tile_of(data);
        let walk = call(
            data,
            token,
            page.clone(),
            dig_scene(here(3100, 3300, 0), true),
        );
        assert_eq!(walk["kind"], "walk", "{walk}");
        let dig = call(
            data,
            token,
            page,
            dig_scene(here(tile.x, tile.z, tile.level), true),
        );
        assert_eq!(dig["kind"], "held", "{dig}");
        assert_eq!(dig["action"], "Dig", "{dig}");
        token
    }

    /// The owned wizard's last-seen aged past the frozen grace: the only way to
    /// reach the gone-outside-grace read without a six-second test.
    fn age_owned_seen(ms: u64) {
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            if let Some(owned) = rt.guardian.as_mut().and_then(|g| g.owned.as_mut()) {
                owned.seen_at -= Duration::from_millis(ms);
            }
        });
    }

    /// A freeze that outlasted the remaining kill grace, without a six-second
    /// test: the clock's own `frozen_at` and the owned wizard's last-seen are
    /// both placed at the freeze's start, so the reclaim the thaw makes is
    /// exactly the frozen interval — the shape a real long freeze hands the
    /// session, where the pause begins with the stamp it found and the thaw
    /// lands a whole interval later.
    fn froze_across_grace() {
        on_pause();
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            let frozen_at = Instant::now() - Duration::from_millis(KILL_GRACE_MS + 1);
            rt.clock.frozen_at = Some(frozen_at);
            if let Some(owned) = rt.guardian.as_mut().and_then(|g| g.owned.as_mut()) {
                owned.seen_at = frozen_at;
            }
        });
        on_resume();
    }

    /// The wizard the frozen cap documents for `2723`'s family alias.
    const WIZARD: &str = "Zamorak Wizard";

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
    /// selected tile with the Spade posted: the packed constrained 3554 clue,
    /// the coord-only map and the paramless 2722. The guarded row is no longer
    /// one of them: its own encounter walks and Digs from this same scene.
    #[test]
    fn coord_only_rows_stay_idle_over_a_walkable_dig_scene() {
        on_reset();
        let data = selected();
        for id in [CLUE, MAP, MAP_EMPTY] {
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

    /// The guarded membership is the selected param set — a decodable coord,
    /// no loc pin, `trail_sextant=yes`, a `trail_guardian` and an access that
    /// is not constrained — thirty rows on both pins, the exemplar decode, and
    /// no swallow of its unguarded sibling. The family alias is a posted-name
    /// filter and never a row field.
    #[test]
    fn the_guarded_dig_membership_is_the_selected_param_set() {
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let data = api::game_data::for_revision(revision).expect("selected data");
            let facts = data.trails().expect("trails");
            // The pinned decode: 2723's selected token is (3058, 3884, 0).
            assert_eq!(
                guarded_tile(row(&data, GUARDED)),
                Some(Tile {
                    x: 3058,
                    z: 3884,
                    level: 0
                }),
                "{revision:?}"
            );
            let members: Vec<i32> = facts
                .rows
                .iter()
                .filter(|row| guarded_tile(row).is_some())
                .map(|row| row.id)
                .collect();
            assert_eq!(members.len(), 30, "{revision:?} {members:?}");
            for id in &members {
                let row = row(&data, *id);
                assert!(!guardian_names(row).is_empty(), "{revision:?} {id}");
                // No swallow: the search and the unguarded classify are other
                // memberships, and neither is this one.
                assert!(dig_tile(row).is_none(), "{revision:?} {id}");
                assert!(search_tile(row).is_none(), "{revision:?} {id}");
            }
            // Every guarded row carries one of the two family aliases the cap
            // documents, and each maps to its own posted-name filter.
            let mut families = facts
                .rows
                .iter()
                .filter(|row| guarded_tile(row).is_some())
                .filter_map(|row| {
                    row.params
                        .iter()
                        .find(|param| param.key == "trail_guardian")
                        .map(|param| param.value.clone())
                })
                .collect::<Vec<_>>();
            families.sort();
            families.dedup();
            assert_eq!(families, vec!["trail_hard", "trail_hard2"], "{revision:?}");
            assert_eq!(
                guardian_names(row(&data, GUARDED)).to_vec(),
                vec!["Zamorak Wizard"],
                "{revision:?}"
            );
            let hard2 = facts
                .rows
                .iter()
                .find(|row| {
                    row.params
                        .iter()
                        .any(|param| param.key == "trail_guardian" && param.value == "trail_hard2")
                })
                .expect("trail_hard2 row");
            assert_eq!(
                guardian_names(hard2).to_vec(),
                vec!["Saradomin Wizard"],
                "{revision:?}"
            );
            // No leak: the unguarded sibling, the search rows, the constrained
            // clue, the coord-only map, the riddle, the paramless row and the
            // caskets are not this membership and carry no filter.
            for id in [
                UNGUARDED,
                SEARCH,
                CLUE,
                MAP,
                RIDDLE,
                MAP_EMPTY,
                CASKET,
                SEXTANT_CASKET,
            ] {
                assert_eq!(guarded_tile(row(&data, id)), None, "{revision:?} {id}");
                assert!(
                    guardian_names(row(&data, id)).is_empty(),
                    "{revision:?} {id}"
                );
            }
        }
        // Synthetic rows: each half of the pin on its own idles, a loc pin of
        // any value is never this membership, and an off-contract token is
        // never rounded into an invented coordinate.
        let sextant = || param("trail_sextant", "yes");
        let coord = || param("trail_coord", "0_47_60_50_44");
        let guardian = || param("trail_guardian", "trail_hard");
        let hit = Some(Tile {
            x: 3058,
            z: 3884,
            level: 0,
        });
        assert_eq!(guarded_tile(&member(vec![coord(), guardian()], None)), None);
        assert_eq!(
            guarded_tile(&member(vec![sextant(), guardian()], None)),
            None
        );
        assert_eq!(guarded_tile(&member(vec![sextant(), coord()], None)), None);
        assert_eq!(
            guarded_tile(&member(vec![sextant(), coord(), guardian()], None)),
            hit
        );
        for blocked in [
            vec![param("trail_loc", "^true"), sextant(), coord(), guardian()],
            vec![param("trail_loc", "^false"), sextant(), coord(), guardian()],
            vec![param("trail_sextant", "no"), coord(), guardian()],
            vec![sextant(), param("trail_coord", "0_47_60_50"), guardian()],
        ] {
            assert_eq!(guarded_tile(&member(blocked, None)), None);
        }
        assert_eq!(
            guarded_tile(&member(
                vec![sextant(), coord(), guardian()],
                Some("constrained")
            )),
            None
        );
        assert_eq!(
            guarded_tile(&member(vec![sextant(), coord(), guardian()], Some("open"))),
            hit
        );
        // The alias is a filter and not a name: an unpinned family has none,
        // and reading one never writes the wizard onto the row.
        assert_eq!(
            guardian_names(&member(vec![guardian()], None)).to_vec(),
            vec!["Zamorak Wizard"]
        );
        assert!(
            guardian_names(&member(vec![param("trail_guardian", "trail_hard9")], None)).is_empty()
        );
        assert!(guardian_names(&member(vec![param("trail_guardian", "")], None)).is_empty());
        let row = member(vec![sextant(), coord(), guardian()], None);
        let before = row.clone();
        assert_eq!(guardian_names(&row).to_vec(), vec!["Zamorak Wizard"]);
        assert_eq!(row.alias, before.alias);
        assert_eq!(row.params.len(), before.params.len());
    }

    /// The selected `cap.prayer` row the fight raises: `Protect from Magic`,
    /// the component the generic `if-button` carries and the overlay varp the
    /// marshalled `varp95` is read from, on both pins — and never a copy of a
    /// prayer table in this file.
    #[test]
    fn the_selected_protect_from_magic_row_is_the_click_and_the_overlay() {
        assert_eq!(PROTECT_FROM_MAGIC, "Protect from Magic");
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let data = api::game_data::for_revision(revision).expect("selected data");
            let row = api::prayer::lookup(&data, PROTECT_FROM_MAGIC).expect("prayer row");
            assert_eq!(row.button_com, 5621, "{revision:?}");
            assert_eq!(row.varp, 95, "{revision:?}");
        }
    }

    /// `Steady` on a guarded row: walk to the decoded tile, then Dig with the
    /// held Spade — the landed sibling arrival and the verb that spawns the
    /// wizard. The fight is what follows, never a second Dig off the same
    /// scene.
    #[test]
    fn a_guarded_row_walks_then_digs_the_spade_and_that_dig_is_the_spawn() {
        on_reset();
        let data = selected();
        let page = json!([[GUARDED, 1]]);
        let token = steady(&data, GUARDED);
        let tile = guarded_tile_of(&data);
        // Not arrived: the walk is the decoded tile, and it repeats.
        for far in [
            here(3100, 3300, 0),
            here(3058, 3884, 1),
            here(3060, 3884, 0),
        ] {
            let walk = call(&data, token, page.clone(), dig_scene(far, true));
            assert_eq!(walk["kind"], "walk", "{walk}");
            assert_eq!(walk["x"], 3058, "{walk}");
            assert_eq!(walk["z"], 3884, "{walk}");
            assert_eq!(walk["level"], 0, "{walk}");
            assert_eq!(token_of(&walk), token, "{walk}");
        }
        // No posted `here` at all, and arrived without the Spade: both wait,
        // and neither starts an encounter.
        let no_tile = call(
            &data,
            token,
            page.clone(),
            json!({ "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)] }),
        );
        assert_eq!(no_tile["kind"], "wait", "{no_tile}");
        let no_spade = call(
            &data,
            token,
            page.clone(),
            dig_scene(here(tile.x, tile.z, tile.level), false),
        );
        assert_eq!(no_spade["kind"], "wait", "{no_spade}");
        // Arrived with the Spade: the held Dig, which is the spawn.
        let dig = call(
            &data,
            token,
            page.clone(),
            dig_scene(here(tile.x, tile.z, tile.level), true),
        );
        assert_eq!(dig["kind"], "held", "{dig}");
        assert_eq!(dig["name"], SPADE_NAME, "{dig}");
        assert_eq!(dig["action"], DIG, "{dig}");
        // The encounter is live: the same row and the same arrived scene are
        // the fight now, so a page with no wizard waits instead of Digging
        // again.
        let idle = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([]), json!({})),
        );
        assert_eq!(idle["kind"], "wait", "{idle}");
        assert!(idle.get("action").is_none(), "{idle}");
        // A frozen call in the spawn stage burned nothing: the encounter is
        // this token's own and the spawn Dig was the last verb.
        assert_eq!(token_of(&idle), token, "{idle}");
    }

    /// The spawn wait: a posted npc page with no wizard of the row family is a
    /// wait whatever else it carries. The nearest anything is never Attacked.
    #[test]
    fn the_spawn_wait_never_attacks_the_nearest_anything() {
        on_reset();
        let data = selected();
        let page = json!([[GUARDED, 1]]);
        let token = spawned(&data);
        let mut unposted = fight_scene(json!([]), json!({}));
        unposted.as_object_mut().expect("object").remove("npcs");
        for scene in [
            // No npc page at all, and an empty one.
            unposted,
            fight_scene(json!([]), json!({})),
            // Another name with the Attack action, closest of all.
            fight_scene(json!([npc(7, "Guard", 1, 10, 10)]), json!({})),
            // The right name without the Attack action.
            fight_scene(
                json!([{
                    "index": 7, "id": 107, "name": WIZARD, "distance": 3,
                    "health": 10, "max_health": 10, "actions": ["Talk-to"],
                    "target_kind": 0, "target_index": -1,
                }]),
                json!({}),
            ),
            // The right name one tile outside the frozen radius.
            fight_scene(
                json!([npc(7, WIZARD, GUARDIAN_RADIUS + 1, 10, 10)]),
                json!({}),
            ),
            // The right name with no posted distance, no posted index, and no
            // posted name at all.
            fight_scene(
                json!([{
                    "index": 7, "name": WIZARD, "health": 10, "max_health": 10,
                    "actions": [ATTACK],
                }]),
                json!({}),
            ),
            fight_scene(
                json!([{
                    "name": WIZARD, "distance": 3, "health": 10, "max_health": 10,
                    "actions": [ATTACK],
                }]),
                json!({}),
            ),
            fight_scene(json!([npc(7, "", 3, 10, 10)]), json!({})),
            // A name that only shares the prefix.
            fight_scene(
                json!([npc(7, "Zamorak Wizard (hard)", 3, 10, 10)]),
                json!({}),
            ),
        ] {
            let idle = call(&data, token, page.clone(), scene.clone());
            assert_eq!(idle["kind"], "wait", "{scene} {idle}");
            assert!(idle.get("action").is_none(), "{scene} {idle}");
            assert!(idle.get("index").is_none(), "{scene} {idle}");
        }
        // Inside the frozen radius the same row is the Attack.
        let attack = call(
            &data,
            token,
            page,
            fight_scene(json!([npc(7, WIZARD, GUARDIAN_RADIUS, 10, 10)]), json!({})),
        );
        assert_eq!(attack["kind"], "npc", "{attack}");
        assert_eq!(attack["name"], WIZARD, "{attack}");
        assert_eq!(attack["action"], ATTACK, "{attack}");
        assert_eq!(attack["index"], 7, "{attack}");
    }

    /// The Attack carries the posted name — the filter folds case and never
    /// substitutes the frozen spelling — and no row id, tile or health rides
    /// along: the posted index is the identity the host matches.
    #[test]
    fn the_attack_carries_the_posted_name_of_the_posted_row() {
        on_reset();
        let data = selected();
        let page = json!([[GUARDED, 1]]);
        let token = spawned(&data);
        let attack = call(
            &data,
            token,
            page,
            fight_scene(json!([npc(4, "zamorak wizard", 3, 9, 9)]), json!({})),
        );
        assert_eq!(attack["kind"], "npc", "{attack}");
        assert_eq!(attack["name"], "zamorak wizard", "{attack}");
        assert_eq!(attack["action"], ATTACK, "{attack}");
        assert_eq!(attack["index"], 4, "{attack}");
        for absent in ["id", "x", "z", "level", "health", "max_health", "message"] {
            assert!(attack.get(absent).is_none(), "{absent} {attack}");
        }
    }

    /// Among matching wizards the row whose own posted target is the player
    /// wins; with none of them on the player the nearest posted distance wins,
    /// then posted order. A page that posted no local-player slot never reads
    /// a `targetsMe`.
    #[test]
    fn the_attack_prefers_the_posted_target_of_the_player() {
        on_reset();
        let data = selected();
        let page = json!([[GUARDED, 1]]);
        let two = json!([
            targeting(npc(3, WIZARD, 9, 30, 30), 0),
            npc(5, WIZARD, 2, 30, 30),
        ]);
        let token = spawned(&data);
        let preferred = call(
            &data,
            token,
            page.clone(),
            fight_scene(two.clone(), json!({})),
        );
        assert_eq!(preferred["kind"], "npc", "{preferred}");
        assert_eq!(preferred["index"], 3, "{preferred}");
        // Neither on the player: the nearer posted distance.
        let token = spawned(&data);
        let nearest = call(
            &data,
            token,
            page.clone(),
            fight_scene(
                json!([npc(6, WIZARD, 7, 30, 30), npc(8, WIZARD, 4, 30, 30)]),
                json!({}),
            ),
        );
        assert_eq!(nearest["kind"], "npc", "{nearest}");
        assert_eq!(nearest["index"], 8, "{nearest}");
        // Neither on the player and both at the same distance: posted order.
        let token = spawned(&data);
        let first = call(
            &data,
            token,
            page.clone(),
            fight_scene(
                json!([npc(2, WIZARD, 4, 30, 30), npc(9, WIZARD, 4, 30, 30)]),
                json!({}),
            ),
        );
        assert_eq!(first["kind"], "npc", "{first}");
        assert_eq!(first["index"], 2, "{first}");
        // No posted slot: the zero slot is not invented for the preference.
        let token = spawned(&data);
        let no_slot = call(
            &data,
            token,
            page,
            fight_scene(two, json!({ "self_slot": null })),
        );
        assert_eq!(no_slot["kind"], "npc", "{no_slot}");
        assert_eq!(no_slot["index"], 5, "{no_slot}");
    }

    /// The Protect from Magic overlay gates the Attack: an overlay posted off
    /// enqueues the generic `if-button` with the selected component id and
    /// Attacks nothing, an overlay posted on skips the click and Attacks, and
    /// an overlay that was not posted is not a proven on — the click still
    /// goes out, no Attack does, and no toggle is waited out.
    #[test]
    fn the_protect_from_magic_overlay_gates_the_attack() {
        on_reset();
        let data = selected();
        let page = json!([[GUARDED, 1]]);
        let wizard = json!([npc(7, WIZARD, 3, 10, 10)]);
        let token = spawned(&data);
        let off = call(
            &data,
            token,
            page.clone(),
            fight_scene(wizard.clone(), json!({ "varp95": 0 })),
        );
        assert_eq!(off["kind"], "if-button", "{off}");
        assert_eq!(off["component_id"], 5621, "{off}");
        for absent in ["name", "action", "index", "message"] {
            assert!(off.get(absent).is_none(), "{absent} {off}");
        }
        // Still off: the same click, still no Attack.
        let again = call(
            &data,
            token,
            page.clone(),
            fight_scene(wizard.clone(), json!({ "varp95": 0 })),
        );
        assert_eq!(again["kind"], "if-button", "{again}");
        assert_eq!(again["component_id"], 5621, "{again}");
        // Unobserved: not a proven on, so the click goes out and never an
        // Attack — and no timeout token is invented for it.
        let mut unobserved = fight_scene(wizard.clone(), json!({}));
        unobserved.as_object_mut().expect("object").remove("varp95");
        let unknown = call(&data, token, page.clone(), unobserved);
        assert_eq!(unknown["kind"], "if-button", "{unknown}");
        assert_eq!(unknown["component_id"], 5621, "{unknown}");
        // A posted value that is not the on value is not the on value.
        let other = call(
            &data,
            token,
            page.clone(),
            fight_scene(wizard.clone(), json!({ "varp95": 2 })),
        );
        assert_eq!(other["kind"], "if-button", "{other}");
        // Posted on: no click, and the Attack goes out.
        let on = call(
            &data,
            token,
            page.clone(),
            fight_scene(wizard.clone(), json!({ "varp95": 1 })),
        );
        assert_eq!(on["kind"], "npc", "{on}");
        assert_eq!(on["index"], 7, "{on}");
        assert!(on.get("component_id").is_none(), "{on}");
        // The gate is read on every fight call rather than latched: an overlay
        // that reads off again mid-fight is clicked again, and only a posted-on
        // overlay leaves the fight to its kill wait.
        let dropped = call(
            &data,
            token,
            page.clone(),
            fight_scene(wizard.clone(), json!({ "varp95": 0 })),
        );
        assert_eq!(dropped["kind"], "if-button", "{dropped}");
        assert_eq!(dropped["component_id"], 5621, "{dropped}");
        let settled = call(&data, token, page, fight_scene(wizard, json!({})));
        assert_eq!(settled["kind"], "wait", "{settled}");
    }

    /// The kill is the owned wizard: zero health beside a posted maximum on the
    /// page that still shows this token's fight on it, or the owned index
    /// leaving the page inside the frozen grace. Only that kill walks back to
    /// the decoded tile and Digs again.
    #[test]
    fn the_owned_kill_walks_back_and_digs_again() {
        on_reset();
        let data = selected();
        let page = json!([[GUARDED, 1]]);
        let tile = guarded_tile_of(&data);
        let token = spawned(&data);
        let attack = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
        );
        assert_eq!(attack["kind"], "npc", "{attack}");
        // Posted and alive: the fight waits, and the Attack is not re-issued.
        let alive = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([npc(7, WIZARD, 3, 9, 10)]), json!({})),
        );
        assert_eq!(alive["kind"], "wait", "{alive}");
        assert!(alive.get("action").is_none(), "{alive}");
        // Killed by health, off the tile: the walk back to the decoded pin.
        let walked_back = call(
            &data,
            token,
            page.clone(),
            fight_scene(
                json!([targeting(npc(7, WIZARD, 3, 0, 10), 0)]),
                json!({ "here": here(3100, 3300, 0) }),
            ),
        );
        assert_eq!(walked_back["kind"], "walk", "{walked_back}");
        assert_eq!(walked_back["x"], tile.x, "{walked_back}");
        // Arrived: the post-kill Dig, repeating while the clue stays held.
        for _ in 0..2 {
            let redig = call(
                &data,
                token,
                page.clone(),
                fight_scene(json!([]), json!({})),
            );
            assert_eq!(redig["kind"], "held", "{redig}");
            assert_eq!(redig["name"], SPADE_NAME, "{redig}");
            assert_eq!(redig["action"], DIG, "{redig}");
        }
        // The same grace path: the owned index leaving the page.
        let token = spawned(&data);
        let attack = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
        );
        assert_eq!(attack["kind"], "npc", "{attack}");
        let gone = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([]), json!({})),
        );
        assert_eq!(gone["kind"], "held", "{gone}");
        assert_eq!(gone["action"], DIG, "{gone}");
    }

    /// A fight that has not settled waits: the owned wizard posted and alive, a
    /// death that is not this token's fight, and an owned index gone only after
    /// the frozen grace was spent. None of them is a redig and none is a
    /// `guardian-lost`.
    #[test]
    fn an_unsettled_fight_waits_without_reaching_for_guardian_lost() {
        on_reset();
        let data = selected();
        let page = json!([[GUARDED, 1]]);
        let token = spawned(&data);
        let attack = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
        );
        assert_eq!(attack["kind"], "npc", "{attack}");
        // Dead beside another player: not this token's kill.
        let stolen = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([targeting(npc(7, WIZARD, 3, 0, 10), 9)]), json!({})),
        );
        assert_eq!(stolen["kind"], "wait", "{stolen}");
        // Gone, but only after the grace was spent: still a wait.
        age_owned_seen(KILL_GRACE_MS + 1);
        let late = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([]), json!({})),
        );
        assert_eq!(late["kind"], "wait", "{late}");
        for step in [&stolen, &late] {
            let text = step.to_string();
            for forbidden in [
                "guardian-lost",
                "clue solved",
                "dead",
                "done",
                "abandon",
                "Dig",
            ] {
                assert!(!text.contains(forbidden), "{forbidden} {step}");
            }
            assert!(step.get("action").is_none(), "{step}");
            assert_eq!(token_of(step), token, "{step}");
        }
    }

    /// A disappearance this token never Attacked for is a wait, not a redig:
    /// while a wizard of the row family is posted the overlay gate is the
    /// click, and a page that posts none is the spawn wait — the overlay is
    /// never raised for a spawn that was not posted, and the wizard coming and
    /// going under it never Digs and never walks.
    #[test]
    fn a_wizard_that_leaves_without_an_attack_is_a_wait_not_a_redig() {
        on_reset();
        let data = selected();
        let page = json!([[GUARDED, 1]]);
        let token = spawned(&data);
        let click = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({ "varp95": 0 })),
        );
        assert_eq!(click["kind"], "if-button", "{click}");
        // The wizard left before any Attack: nothing of the family is posted,
        // so the click is not raised either and the fight waits.
        let empty = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([]), json!({ "varp95": 0 })),
        );
        assert_eq!(empty["kind"], "wait", "{empty}");
        assert!(empty.get("component_id").is_none(), "{empty}");
        // Another wizard of the family posted and still unowned, but dead on
        // the page: the spawn is observed, so the click goes out and no
        // Attack or redig follows from a death this token never fought.
        let posted = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([npc(9, WIZARD, 3, 0, 10)]), json!({ "varp95": 0 })),
        );
        assert_eq!(posted["kind"], "if-button", "{posted}");
        for step in [&empty, &posted] {
            let text = step.to_string();
            for forbidden in ["guardian-lost", "Dig", "walk", "done", "abandon"] {
                assert!(!text.contains(forbidden), "{forbidden} {step}");
            }
            assert_eq!(token_of(step), token, "{step}");
        }
    }

    /// The spawn observation runs before the overlay: the Protect from Magic
    /// click is only ever raised behind a posted wizard of the row family, and
    /// the Attack only ever follows that click's on read. A page with no such
    /// wizard waits with no click at all — whatever the overlay reads, and
    /// whatever else it posted — and the owned wizard's own leave is read
    /// before the gate too, so a kill never waits on the prayer.
    #[test]
    fn the_overlay_is_only_raised_behind_a_posted_spawn() {
        on_reset();
        let data = selected();
        let page = json!([[GUARDED, 1]]);
        let wizard = json!([npc(7, WIZARD, 3, 10, 10)]);
        let token = spawned(&data);
        let mut unobserved = fight_scene(json!([npc(7, "Guard", 1, 10, 10)]), json!({}));
        unobserved.as_object_mut().expect("object").remove("varp95");
        for scene in [
            // No spawn posted with the overlay off, on and unobserved: the
            // spawn wait, never a click.
            fight_scene(json!([]), json!({ "varp95": 0 })),
            fight_scene(json!([]), json!({ "varp95": 1 })),
            unobserved,
            // A wizard of the family the frozen radius refuses.
            fight_scene(
                json!([npc(7, WIZARD, GUARDIAN_RADIUS + 1, 10, 10)]),
                json!({ "varp95": 0 }),
            ),
        ] {
            let idle = call(&data, token, page.clone(), scene.clone());
            assert_eq!(idle["kind"], "wait", "{scene} {idle}");
            assert!(idle.get("component_id").is_none(), "{scene} {idle}");
            assert!(idle.get("action").is_none(), "{scene} {idle}");
        }
        // The spawn posted and the overlay off: the click, and still no
        // Attack.
        let click = call(
            &data,
            token,
            page.clone(),
            fight_scene(wizard.clone(), json!({ "varp95": 0 })),
        );
        assert_eq!(click["kind"], "if-button", "{click}");
        assert_eq!(click["component_id"], 5621, "{click}");
        assert!(click.get("index").is_none(), "{click}");
        // The posted-on overlay leaves the Attack, for that posted index.
        let attack = call(
            &data,
            token,
            page.clone(),
            fight_scene(wizard.clone(), json!({ "varp95": 1 })),
        );
        assert_eq!(attack["kind"], "npc", "{attack}");
        assert_eq!(attack["index"], 7, "{attack}");
        // Owned and still posted with the overlay off again: the gate is read
        // on every fight call rather than latched, so the click goes out again
        // and the Attack is never re-issued.
        let owned = call(
            &data,
            token,
            page.clone(),
            fight_scene(wizard.clone(), json!({ "varp95": 0 })),
        );
        assert_eq!(owned["kind"], "if-button", "{owned}");
        assert!(owned.get("index").is_none(), "{owned}");
        // The owned wizard gone with nothing of the family posted: the grace
        // kill, read before the overlay and never a click.
        let gone = call(
            &data,
            token,
            page,
            fight_scene(json!([]), json!({ "varp95": 0 })),
        );
        assert_eq!(gone["kind"], "held", "{gone}");
        assert_eq!(gone["name"], SPADE_NAME, "{gone}");
        assert_eq!(gone["action"], DIG, "{gone}");
    }

    /// The spawn filter reads the posted tile: the posted row must be on this
    /// call's posted `here` level, and a row that posted no distance is
    /// measured from its own posted tile with the landed Chebyshev read — the
    /// nearer of two such rows wins before posted order, and a tile outside
    /// the frozen radius is refused exactly like a posted distance outside it.
    #[test]
    fn the_spawn_pick_reads_the_posted_level_and_the_posted_tile() {
        on_reset();
        let data = selected();
        let page = json!([[GUARDED, 1]]);
        let token = spawned(&data);
        for scene in [
            // Another level, with a posted distance and with only its own
            // tile to measure.
            fight_scene(json!([field(npc(7, WIZARD, 3, 10, 10), "level", json!(1))]), json!({})),
            fight_scene(json!([tiled(7, WIZARD, 3058, 3884, 1)]), json!({})),
            // No posted level at all is not the `here` level either, and a
            // row with no posted tile and no posted distance has no measure.
            fight_scene(json!([unfield(npc(7, WIZARD, 3, 10, 10), "level")]), json!({})),
            fight_scene(
                json!([unfield(unfield(unfield(npc(7, WIZARD, 3, 10, 10), "distance"), "x"), "z")]),
                json!({}),
            ),
            // No posted `here`: no level to compare and no base to measure
            // from, whatever the row posted.
            fight_scene(json!([tiled(7, WIZARD, 3058, 3884, 0)]), json!({ "here": null })),
            fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({ "here": null })),
            // The row's own tile one step outside the frozen radius.
            fight_scene(
                json!([tiled(7, WIZARD, 3058 + GUARDIAN_RADIUS + 1, 3884, 0)]),
                json!({}),
            ),
        ] {
            let idle = call(&data, token, page.clone(), scene.clone());
            assert_eq!(idle["kind"], "wait", "{scene} {idle}");
            assert!(idle.get("index").is_none(), "{scene} {idle}");
            assert!(idle.get("component_id").is_none(), "{scene} {idle}");
        }
        // Two rows measured by their own tiles: the nearer one wins before
        // posted order, the way the posted-distance read already does.
        let near = call(
            &data,
            token,
            page.clone(),
            fight_scene(
                json!([
                    tiled(4, WIZARD, 3058, 3884 - 9, 0),
                    tiled(6, WIZARD, 3058 + 2, 3884, 0),
                ]),
                json!({}),
            ),
        );
        assert_eq!(near["kind"], "npc", "{near}");
        assert_eq!(near["index"], 6, "{near}");
        // Exactly on the frozen radius is inside it, and the tile-measured row
        // is the Attack with no posted distance at all.
        let token = spawned(&data);
        let edge = call(
            &data,
            token,
            page,
            fight_scene(
                json!([tiled(7, WIZARD, 3058, 3884 + GUARDIAN_RADIUS, 0)]),
                json!({}),
            ),
        );
        assert_eq!(edge["kind"], "npc", "{edge}");
        assert_eq!(edge["index"], 7, "{edge}");
        assert_eq!(edge["name"], WIZARD, "{edge}");
    }

    /// A freeze that outlasted the remaining kill grace: the owned last-seen
    /// lives at the freeze's own start and the wizard is gone by the time the
    /// session thaws. The thaw reclaims the frozen gap into that stamp the way
    /// the landed hunt fight shifts its own, so the disappearance is still
    /// this token's kill rather than a grace the pause spent.
    #[test]
    fn a_freeze_across_the_kill_grace_still_ends_in_the_kill() {
        on_reset();
        let data = selected();
        let page = json!([[GUARDED, 1]]);
        let token = spawned(&data);
        let attack = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
        );
        assert_eq!(attack["kind"], "npc", "{attack}");
        assert_eq!(attack["index"], 7, "{attack}");
        // The freeze: without the reclaim the next call's own `now` is the
        // whole frozen interval ahead of the owned stamp, so the grace would
        // already read as spent and the fight would wait forever.
        froze_across_grace();
        let killed = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([]), json!({})),
        );
        assert_eq!(killed["kind"], "held", "a freeze never spends the grace: {killed}");
        assert_eq!(killed["name"], SPADE_NAME, "{killed}");
        assert_eq!(killed["action"], DIG, "{killed}");
        // Frozen again: nothing is read and nothing is emitted, and the thaw
        // after it leaves the post-kill Dig where it was.
        on_pause();
        let frozen = call(&data, token, page.clone(), fight_scene(json!([]), json!({})));
        assert_eq!(frozen["kind"], "wait", "{frozen}");
        assert!(frozen.get("action").is_none(), "{frozen}");
        on_resume();
        let redig = call(&data, token, page, fight_scene(json!([]), json!({})));
        assert_eq!(redig["kind"], "held", "{redig}");
        assert_eq!(redig["action"], DIG, "{redig}");
    }

    /// The mid-fight hitpoints wait: a posted effective hitpoints at or below
    /// zero emits nothing at all and never a public `dead`; a page that posted
    /// no stat is not a zero. After the kill the fight is over, so the redig is
    /// not gated by it.
    #[test]
    fn the_mid_fight_hitpoints_wait_never_emits_dead() {
        on_reset();
        let data = selected();
        let page = json!([[GUARDED, 1]]);
        let wizard = json!([npc(7, WIZARD, 3, 10, 10)]);
        let token = spawned(&data);
        for hp in [0, -1, -20] {
            let downed = call(
                &data,
                token,
                page.clone(),
                fight_scene(wizard.clone(), json!({ "hitpoints": hp })),
            );
            assert_eq!(downed["kind"], "wait", "{hp} {downed}");
            for absent in ["action", "index", "component_id", "name"] {
                assert!(downed.get(absent).is_none(), "{hp} {absent} {downed}");
            }
            assert!(!downed.to_string().contains("dead"), "{hp} {downed}");
        }
        // No posted stat at all is not a zero: the Attack still goes out.
        let mut bare = fight_scene(wizard.clone(), json!({}));
        bare.as_object_mut().expect("object").remove("hitpoints");
        let attack = call(&data, token, page.clone(), bare);
        assert_eq!(attack["kind"], "npc", "{attack}");
        assert_eq!(attack["index"], 7, "{attack}");
        // The kill, then the post-kill Dig with the same zero posted: the wait
        // belonged to the fight and not to the encounter.
        let kill = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([]), json!({})),
        );
        assert_eq!(kill["kind"], "held", "{kill}");
        assert_eq!(kill["action"], DIG, "{kill}");
        let after = call(
            &data,
            token,
            page,
            fight_scene(json!([]), json!({ "hitpoints": 0 })),
        );
        assert_eq!(after["kind"], "held", "{after}");
        assert_eq!(after["action"], DIG, "{after}");
    }

    /// Freeze and yield beat the guarded encounter the way they beat the landed
    /// verbs: no walk, no held, no npc and no if-button ride along, and the
    /// token lives for the thaw.
    #[test]
    fn freeze_and_yield_beat_the_guarded_encounter() {
        on_reset();
        let data = selected();
        let page = json!([[GUARDED, 1]]);
        let tile = guarded_tile_of(&data);
        // The ladder over one scene: paused, held, then the posted interrupt.
        let ladder = |token: u64, scene: &Value| {
            let mut out = Vec::new();
            on_pause();
            out.push(call(&data, token, page.clone(), scene.clone()));
            on_resume();
            on_hold(true);
            out.push(call(&data, token, page.clone(), scene.clone()));
            on_hold(false);
            let mut yielded_scene = scene.clone();
            yielded_scene["hold"] = json!(true);
            out.push(call(&data, token, page.clone(), yielded_scene));
            out
        };
        // The spawn stage: the walk never goes out under a frozen clock.
        let token = steady(&data, GUARDED);
        let spawn_scene = dig_scene(here(3100, 3300, 0), true);
        let spawn_steps = ladder(token, &spawn_scene);
        assert_eq!(
            spawn_steps
                .iter()
                .map(|step| step["kind"].clone())
                .collect::<Vec<_>>(),
            vec![json!("wait"), json!("wait"), json!("yield")],
            "{spawn_steps:?}"
        );
        for step in &spawn_steps {
            assert!(step.get("x").is_none(), "{step}");
            assert!(step.get("action").is_none(), "{step}");
            assert_eq!(token_of(step), token, "{step}");
        }
        // Nothing burned: the thawed call is still the walk.
        let walk = call(&data, token, page.clone(), spawn_scene);
        assert_eq!(walk["kind"], "walk", "{walk}");
        let dig = call(
            &data,
            token,
            page.clone(),
            dig_scene(here(tile.x, tile.z, tile.level), true),
        );
        assert_eq!(dig["kind"], "held", "{dig}");
        // The fight stage: the same ladder, and the overlay-off click is not
        // emitted under it either.
        let fight = fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({ "varp95": 0 }));
        let fight_steps = ladder(token, &fight);
        assert_eq!(
            fight_steps
                .iter()
                .map(|step| step["kind"].clone())
                .collect::<Vec<_>>(),
            vec![json!("wait"), json!("wait"), json!("yield")],
            "{fight_steps:?}"
        );
        for step in &fight_steps {
            assert!(step.get("component_id").is_none(), "{step}");
            assert!(step.get("action").is_none(), "{step}");
        }
        // Thawed and unheld, the click is still there.
        let click = call(&data, token, page, fight);
        assert_eq!(click["kind"], "if-button", "{click}");
        assert_eq!(click["component_id"], 5621, "{click}");
    }

    /// The encounter is session state on the live step: a different held row
    /// re-arms the gate and drops it, so coming back to the guarded row walks
    /// and Digs its spawn again rather than resuming the fight the old token
    /// already owned.
    #[test]
    fn a_different_held_step_drops_the_guarded_encounter() {
        on_reset();
        let data = selected();
        let guard_page = json!([[GUARDED, 1]]);
        let token = spawned(&data);
        let attack = call(
            &data,
            token,
            guard_page.clone(),
            fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
        );
        assert_eq!(attack["kind"], "npc", "{attack}");
        // A different membership row is held: the landed gate re-arms for it.
        let other = json!([[RIDDLE, 1]]);
        let re_armed = call(
            &data,
            token,
            other.clone(),
            fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
        );
        assert_eq!(re_armed["kind"], "callback.enabled", "{re_armed}");
        let logged = call(&data, token, other.clone(), json!({ "resume": true }));
        assert_eq!(logged["kind"], "callback.log", "{logged}");
        let _ = call(&data, token, other.clone(), json!({}));
        // Back to the guarded row: the gate re-arms again, and the next steady
        // call is the spawn again — a walk and a Dig, never the old Attack.
        let back = call(
            &data,
            token,
            guard_page.clone(),
            fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
        );
        assert_eq!(back["kind"], "callback.enabled", "{back}");
        let logged = call(&data, token, guard_page.clone(), json!({ "resume": true }));
        assert_eq!(logged["kind"], "callback.log", "{logged}");
        let _ = call(&data, token, guard_page.clone(), json!({}));
        let reborn = call(
            &data,
            token,
            guard_page.clone(),
            fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
        );
        assert_eq!(reborn["kind"], "held", "{reborn}");
        assert_eq!(reborn["action"], DIG, "{reborn}");
    }

    /// The post-kill Dig is the landed sibling Dig: it repeats while the clue
    /// stays held, the casket it produces is the landed casket-first Open that
    /// re-arms the gate, and a `none-held` right after a guarded Dig is still
    /// the landed abort — the collect is the casket Open's alone.
    #[test]
    fn the_guarded_redig_repeats_and_its_casket_opens() {
        on_reset();
        let data = selected();
        let clue_page = json!([[GUARDED, 1]]);
        let token = spawned(&data);
        let attack = call(
            &data,
            token,
            clue_page.clone(),
            fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
        );
        assert_eq!(attack["kind"], "npc", "{attack}");
        let redig = call(
            &data,
            token,
            clue_page.clone(),
            fight_scene(json!([]), json!({})),
        );
        assert_eq!(redig["kind"], "held", "{redig}");
        assert_eq!(redig["action"], DIG, "{redig}");
        // The casket the Dig produced is held beside its own clue: identify is
        // casket-first, so the gate re-arms for it and its own Open follows.
        let casket = casket_of(&data, GUARDED);
        let casket_alias = row(&data, casket).alias.clone();
        let both = json!([[GUARDED, 1], [casket, 1]]);
        let scene = fight_scene(json!([]), json!({}));
        let re_armed = call(&data, token, both.clone(), scene.clone());
        assert_eq!(re_armed["kind"], "callback.enabled", "{re_armed}");
        let logged = call(&data, token, both.clone(), json!({ "resume": true }));
        assert_eq!(logged["kind"], "callback.log", "{logged}");
        assert!(
            logged["message"]
                .as_str()
                .unwrap_or("")
                .contains(&casket_alias),
            "{logged}"
        );
        assert_eq!(
            call(&data, token, both.clone(), scene.clone())["kind"],
            "callback.setStatus"
        );
        let open = call(&data, token, both.clone(), scene.clone());
        assert_eq!(open["kind"], "held", "{open}");
        assert_eq!(open["action"], OPEN, "{open}");
        // A `none-held` right after a guarded Dig is the landed abort, and the
        // reward window was never armed by it.
        let token = spawned(&data);
        let gone = call(&data, token, json!([]), json!({}));
        assert_eq!(gone["kind"], "aborted", "{gone}");
        assert_eq!(gone["reason"], NONE_HELD, "{gone}");
        assert!(!bound_armed(), "a guarded Dig never arms the reward window");
    }

    /// The guarded encounter emits walk, held Dig, npc Attack, if-button, wait
    /// or yield only — never a completion, never a hunt kind of its own, and
    /// never one of the public refusal tokens.
    #[test]
    fn the_guarded_encounter_emits_only_walk_held_npc_if_button_and_wait() {
        on_reset();
        let data = selected();
        let page = json!([[GUARDED, 1]]);
        let tile = guarded_tile_of(&data);
        let token = steady(&data, GUARDED);
        let mut yielded = dig_scene(here(tile.x, tile.z, tile.level), true);
        yielded["hold"] = json!(true);
        let steps = vec![
            // The spawn walk, the spawn Dig, the overlay click, the Attack,
            // the kill wait, the post-kill redig and the interrupt.
            call(
                &data,
                token,
                page.clone(),
                dig_scene(here(3100, 3300, 0), true),
            ),
            call(
                &data,
                token,
                page.clone(),
                dig_scene(here(tile.x, tile.z, tile.level), true),
            ),
            call(
                &data,
                token,
                page.clone(),
                fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({ "varp95": 0 })),
            ),
            call(
                &data,
                token,
                page.clone(),
                fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
            ),
            call(
                &data,
                token,
                page.clone(),
                fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
            ),
            call(
                &data,
                token,
                page.clone(),
                fight_scene(json!([targeting(npc(7, WIZARD, 3, 0, 10), 0)]), json!({})),
            ),
            call(&data, token, page, yielded),
        ];
        for step in &steps {
            assert!(step["status"].is_null(), "{step}");
            assert!(
                matches!(
                    step["kind"].as_str().unwrap_or(""),
                    "walk" | "held" | "npc" | "if-button" | "wait" | "yield"
                ),
                "{step}"
            );
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
                "sustain",
                "arm-special",
                "safespot",
            ] {
                assert!(!text.contains(forbidden), "{forbidden} {step}");
            }
        }
        assert_eq!(
            steps
                .iter()
                .map(|step| step["kind"].clone())
                .collect::<Vec<_>>(),
            vec![
                json!("walk"),
                json!("held"),
                json!("if-button"),
                json!("npc"),
                json!("wait"),
                json!("held"),
                json!("yield"),
            ],
            "{steps:?}"
        );
    }
}
