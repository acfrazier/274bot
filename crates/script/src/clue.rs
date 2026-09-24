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
//! This is the search, casket-open, unguarded-dig, guarded-dig encounter,
//! coordinate-trio acquire, trail-end collect, held-puzzle-box, talk-step and
//! key-keeper hunt slice and nothing else: no deposit, retry, return-grind or
//! puzzle-box extra. A held
//! row that is a selected search
//! membership — a selected `trail_loc=^true` **and** a decodable selected
//! `trail_coord` on the same row — walks to its decoded tile and then
//! dispatches the Search/Open picker over the posted loc page; both verbs are
//! enqueued by the wrapper as `InteractReq::Walk` / `InteractReq::Loc`. The
//! picker is the frozen one, minus its `walkLeg`: nearest then action rank,
//! always at the row's own posted tile and id.
//!
//! The sibling of that pin is the unguarded dig: a decodable selected
//! `trail_coord` on a row with no `trail_loc` and no `trail_guardian`, whose
//! `access` is not `"constrained"`. It reads no `trail_sextant` at all, so the
//! membership is the twenty medium sextant rows beside the coord-bearing map,
//! vague and riddle rows: forty rows on both pins, and the `trail_casket`
//! param is never part of it. It walks the same way — the same decoder, the
//! same radius, the same posted `here` — and then
//! dispatches the generic held step with the selected item display `Spade` and
//! the frozen `Dig`. The Sextant/Watch/Chart trio the guarded sibling's pin
//! stands for is never required, never waited for and never acquired. A pack
//! that does not post the `Spade` on an arrived call is the named
//! `supplies-needed` wait-class — the token lives, nothing is fetched, and it
//! is never an `abandon` and never a `no-spade` token. Dig repeats while that
//! same clue stays held, a produced casket Opens instead, and a `none-held`
//! right after a Dig still aborts: Collecting is the casket Open's alone.
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
//! disappearance without one waits. An owned index gone outside the grace —
//! without ever being seen at zero health — is `guardian-lost`: the encounter
//! is lost, the token dies with it and nothing redigs.
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
//! finishes on its own three-step exit — the exact `'clue solved'` status,
//! then the live-token `grind-ready` continue, then the `done` the token dies
//! on — armed once the reward window has passed with nothing left to take, or
//! once the pack-full WARNING has been logged. The latch is the collect's, so
//! a latched session never loots again, and a frozen call waits it out.
//!
//! Every other held step — the desc-only riddles with no decodable coord that
//! no selected key-keeper row names, and the empty-params 2722 — is identified
//! and then idled: no action and no walk. The packed 3554
//! `access: "constrained"` clue is the one identified row this machine refuses
//! instead: `aborted` / `constrained`, no verb, no token — and a begin that
//! identifies it is refused the same way.
//!
//! A posted effective `hitpoints` at or below zero is `dead` on any live
//! call: the token dies with the player and nothing posts `'clue solved'`. A
//! page that posted no stat is not a zero.
//!
//! The one desc-only exception is the held puzzle box. An identified row whose
//! own selected `{alias}_puzzlebox` item is held — the nine hard riddles, and
//! only while *that* row's box is the one on the page — is the frozen
//! `PuzzleBox` run instead of an idle: the box is opened by its own selected
//! display name the way the casket is, the posted board is planned with the
//! frozen grouped BFS in `api::clue_puzzle` (`read_puzzle_board` over the
//! sparse page and the selected piece map, `solve_puzzle` for the leg), and
//! exactly one `puzzle-move` is dispatched per call — the posted row's own
//! `id`, `slot` and `component` with this call's board generation. Every call
//! re-reads the board and replans, because a sent click is not an observed
//! move: the frozen engine drops a stale-slot click, and the frozen `want`
//! board is what the next call compares against. A board the plan has solved
//! is closed and then idled — the frozen `finally` close — and the exit is the
//! same either way: solved, stalled, unreadable, past `MAX_MOVES` or a board
//! that never opened all latch the step, so a solved box is never opened or
//! closed twice while the same step stays held. The latch is what re-talks:
//! the next live call falls through to the `Steady` arms below, where the nine
//! desc-only puzzle riddles the selected `talk_key.talk` family publishes are
//! the landed talk step, and every other latched row idles.
//!
//! In front of both dig arms sits the coordinate-trio acquire. An identified
//! row carrying the selected `trail_sextant=yes` needs the Sextant, the Watch
//! and the Chart, so it only Digs once this call's posted pack holds all three:
//! `hasAllTrio` is that posted read and the intercept's own completion, and
//! until it holds the arm is the frozen `nextCoordTool` order over the
//! published `trio_givers` rows. The sextant is the observatory professor's
//! teach stop and then Murphy's own, the watch is Brother Kojo's and the chart
//! is the professor's again. A stop walks to its giver's published
//! `{x, z, plane}` tile and Talks-to a posted npc of that giver's own packed id
//! — or, failing that, its posted display name — inside the frozen
//! `ARRIVE_RADIUS`, which is the talk arm's unique-spawn rule and never a
//! frozen coordinate table. Behind that giver's open chat the professor's one
//! posted option whose text folds to a selected closed-handler literal is
//! answered with its own posted 1-based slot, a posted list with no such row
//! waits, and Murphy and Kojo are talk-then-continue only. A stop is done when
//! its own chat has been posted open and then closed; a giver that is not
//! posted, an item that does not land and a locked door all wait with the token
//! live. Nothing is fetched, banked, shopped or dropped, the packed 3554 clue
//! is refused before this machine ever reaches `Steady`, and the fifty rows
//! that never carry the param are never asked for a trio at all.
//!
//! The next seam is the gate-toll shop, and it is the walk's and not an
//! identify's: any live walk this token dispatched that has not arrived, while
//! the posted page names a `Carry` short for the selected Shantay pass the pack
//! does not hold, is a walk the navigator could not route. The short is named by
//! the host's own strict-find diagnosis — `find_missing_item_reqs`, posted on
//! the walk outcome's own family — so a `NoPath`'s dest geometry is never read
//! as a shopping list and a page that names no short never shops. The trip walks
//! to the selected Shantay spawn (the unique jm2 tile, never a frozen stand), the
//! posted `Trade` click on the posted keeper of the selected type, the buy of one
//! chunk of the short's own posted stock row, and then the interface's close.
//! The latch is the frozen `gateItemsTried`: once per item id per token, set
//! before the first verb, so the Shantay walk never re-enters the intercept, and
//! the trip's exit walks the original dest back — the second walk — after which
//! the row's own arm resumes. A step whose own posted fact never appears inside
//! that step's window ends the trip with the named `no-shop`: the token lives,
//! nothing is fetched and no third trip starts. The Al Kharid toll's coins, the
//! extra-item Rope and every other named short are not this item and never shop,
//! and a pass the posted pack already holds is not shopped for at all.
//!
//! The last `Steady` arm is the talk step: a held row the selected
//! `talk_key.talk` family publishes is the NPC it names. Forty-two of them
//! publish the unique jm2 spawn, so the walk goes to the published
//! `{x, z, plane}` tile and the Talk-to only ever dispatches at a posted npc of
//! that identity standing on it — a wanderer outside the fogged radius is not
//! chased, nothing is Cleared, and a page with no match waits at the tile. The
//! five steps whose jm2 spawn is not unique take the nearest posted npc of
//! their own identity instead: the selected `npc.id` against the posted `id`,
//! then the selected display name against the posted `name`, never the alias
//! and never first-in-file. Both arms require the row's own posted talk action
//! and keep the posted name and posted scene index on the verb, and a page that
//! posts no match, no `here` or no talk action waits with the token live.
//!
//! An open chat closes the arm for the tick: the posted `chat_modal_id` beside
//! `chat_continue` is the landed `dialog_ready`, so no walk and no Talk-to goes
//! out while it holds, and the posted `count_dialog_open` blocks the same way.
//! The challenge seam is a `none-held` sibling of Collecting: `identify_step`
//! reads no challenge id, so a page that holds only a selected
//! `challenge_answers` scroll joins its parent talk step through the
//! `_challenge` strip — the parent is the step, never the scroll id — and the
//! posted count dialog is answered with that selected answer. Empty pages, zero
//! counts and unselected ids still abort `none-held`.
//!
//! The last `Steady` arm is the key-keeper hunt: a held row the selected
//! `talk_key.keys` family publishes is the one key its keeper drops for this
//! clue. That family is a sibling of the talk steps and never a second
//! identify — the held row stays the riddle `identify_step` returned, and a
//! key keeper is never Talked-to and a talk step is never hunted. Two rows
//! publish a unique jm2 spawn beside a packed-type keeper; the other five
//! publish no unique spawn — two of them beside a packed type the family
//! covered, and three a `category` or a bare `name` — and stay idle over any
//! scene: a published tile and a packed type are the only identities this arm
//! can walk to and match against the posted npc page, and neither is invented.
//!
//! The hunt is one verb per call over this call's own marshalled pages. The key
//! already on the posted pack page ends it: the original riddle idles with
//! `wait`, no Attack, no gate and no completion kind — a key banked but not
//! held is not observed at all. Otherwise the walk goes to the published
//! `{x, z, plane}` tile with `plane` as the verb's `level`, repeating until the
//! posted `here` holds, and only ever to that tile: a keeper wandering off it
//! is not chased and a page with no posted `here` waits. Arrived, the one
//! `Attack` is dispatched for a posted npc of the keeper's packed type standing
//! on that tile — the selected id against the posted `id`, then the display
//! name against the posted `name`, never the alias — and only a row that lists
//! the posted `Attack`. No prayer is raised for a keeper: the overlay stays the
//! wizard encounter's. The kill is the owned index posted at zero health beside
//! a posted maximum with this token's fight still on it, or that index leaving
//! the page inside the freeze-aware grace; an index this token never Attacked
//! is never a kill, and a disappearance outside the grace is a `wait` rather
//! than the encounter's `guardian-lost`. Only the kill lets the pickup run: the
//! posted ground row of the key's own id whose actions carry `Take` and whose
//! tile is on the spawn's own level inside the frozen radius is Taken with the
//! landed `kind: obj`, one verb per call, and a full pack waits — this arm
//! Drops no food. The hunt is over when the posted page holds the key, and that
//! is not trail completion: the original riddle goes on idling, nothing re-arms
//! the gate, and `'clue solved'`, `grind-ready` and `done` stay the collect's.
//!
//! Identify is casket-first, so a casket held beside its own clue is the
//! Open and never 3554 play. Yield keeps the token live, so it is not trail
//! completion: the completion kinds are the finished collect's alone and this
//! machine never returns a hunt `status: "done"`.
//!
//! The one gear arm is the Entrana strip and its restore, and it is the
//! identified row's own selected decode: a row whose `trail_coord` decodes
//! inside the cap's box on level 0 — the proof row `3579`,
//! `trail_clue_hard_riddle027`, `0_44_52_2_23` → `(2818, 3351, 0)` — is stripped
//! before the walk that row would otherwise make. A casket never arms it. The
//! stripped list is the frozen `strippedGear`: the posted worn rows whose
//! display name the frozen matcher folds are unequipped with the landed
//! `unequip` verb — the worn row's own `Remove`, because `wear` resolves
//! inventory rows alone — and listed, the two hard-trail dagger ids `1231` /
//! `1215` are unequipped but left off it, and every posted pack row whose name
//! the matcher folds is deposited before the row's own arms run: the names the
//! unequip put in the pack, the dagger ids that are never listed, and a
//! restricted item the player carried without wearing it. The list outlives a
//! step, a dead token and the connection-boundary reset; only the restore or a
//! fresh task instance (Stop/Start) empties it, and `ownsEquipment` is that
//! list and nothing else.
//!
//! The restore sits in front of the whole three-step finish latch: while the
//! list is non-empty the collect's exit wears what the pack holds, claims the
//! rest at the bank — the walk to the nearest stand, the posted booth's own
//! open, the frozen make-room deposit while the pack has fewer free slots than
//! the names it is missing, one `Withdraw-1` per missing name, the interface's
//! close — and only an empty list lets the exact `'clue solved'`, the
//! `grind-ready` continue and the `done` go out. A name that will not go back
//! on stays listed, is logged as the named `restore-incomplete` and blocks that
//! latch rather than posting a completion kind. Freeze, yield and the posted
//! hitpoints still win over the restore, exactly as they do over the collect.
//!
//! The frozen `abandonedClueId` is wired with no production trigger: an
//! `abandon` this machine emits latches the identified row, a `begin` on that
//! same row is refused `abandoned` while it stays held, a different held row or
//! the adapter's own `retry` clears the latch, and `retry` is not `on_reset`:
//! it never aborts the live token and never touches the stripped list.
//!
//! One token per isolate. A second begin, reset and stop abort the live token
//! and emit no verb for it. `on_reset` is the connection boundary: it drops the
//! step and its token alone, so a strip whose gear is already banked still owes
//! its reclaim after a relog. `on_stop` is the fresh task instance — operator
//! Stop, a new Start — and clears the stripped list and the abandon latch with
//! the step. Pause and hold freeze this machine's own clock,
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
use api::clue_puzzle::{self, Board, PuzzleRow};
use api::game_data::{
    SelectedGameData, TalkKeyKeeper, TalkKeyKeyRow, TalkKeyTalkRow, TrailMembershipRow,
    TrioGiverRow,
};
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

/// The exact completion status a finished collect posts: eleven characters,
/// lowercase, one space. This is the machine's own seat — the adapters forward
/// `callback.setStatus` and never remap another kind onto it.
const CLUE_SOLVED: &str = "clue solved";

/// The live-token continue kind between that status and `done`: the trail is
/// solved and no gear restore is pending — the Entrana list is empty, or this
/// kind never rides at all — so the sibling grind may take the next tick. No
/// verb rides it and the token stays live.
const GRIND_READY: &str = "grind-ready";

/// The finished collect's own end: the token dies with it, so the next call
/// with that token is `stale`. Never a hunt `status: "done"` — it is this
/// machine's own `kind`.
const DONE: &str = "done";

/// The posted effective hitpoints at or below zero kill the token. A page that
/// posted no stat at all is not a zero. Amends the mid-fight wait the guarded
/// encounter used to keep.
const DEAD: &str = "dead";

/// An arrived dig with no posted `Spade`: a named wait-class and not a
/// terminal. The token stays live, nothing is fetched, and no `no-spade` token
/// is ever published.
const SUPPLIES_NEEDED: &str = "supplies-needed";

/// The wizard this token owned left the posted page outside the freeze-aware
/// grace without a health-0 kill: the encounter is gone and the token dies
/// with it. A disappearance without an Attack is not this kind.
const GUARDIAN_LOST: &str = "guardian-lost";

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

/// The frozen `PuzzleBox.OPEN_WAIT_MS`: how long the held box's own Open has to
/// put a readable board up. Armed once, when this attempt's first Open goes
/// out, and freeze-honored like the collect's own window, so a frozen session
/// never spends it.
const OPEN_WAIT_MS: u64 = 5_000;

/// The frozen `PuzzleBox.MOVE_SETTLE_MS`: how long one dispatched
/// `puzzle-move` has to land. Sent is not observed, so the click is re-read
/// against the board the move was expected to produce and the bound is what
/// ends the wait when it never does.
const MOVE_SETTLE_MS: u64 = 2_000;

/// The frozen `PuzzleBox.CLOSE_WAIT_MS`: how long the one close waits for the
/// board to go. The close itself is dispatched once — a solved or given-up
/// step never closes again — and this window is what the wait before the latch
/// is measured by.
const CLOSE_WAIT_MS: u64 = 3_000;

/// The frozen `PuzzleBox.STALL_LIMIT`: consecutive refused clicks, settle
/// bounds that ran out unlanded and boards with no plan at all, before the
/// attempt gives up and closes.
const STALL_LIMIT: u32 = 8;

/// The frozen `PuzzleBox.MAX_MOVES`: the landed moves one attempt may spend.
const MAX_MOVES: u32 = 600;

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

/// The one `talk_key.keys` keeper matcher kind that carries a packed npc id.
/// A `category` or a bare `name` matcher matches many npcs and names no single
/// type, so its row stays idle rather than hunting the nearest anything.
const KEEPER_TYPE: &str = "type";

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

/// The selected `trail_sextant` param the coordinate trio's own membership
/// reads: the 50 playable rows that need the trio carry `yes`, and the packed
/// 3554 clue is the one constrained row of the 51. A param on the identified
/// row alone — never a second identify, never a talk-key join, and never the
/// row's guardian or casket — so the unguarded map, vague and riddle rows that
/// do not carry it never enter this arm and never grow a trio requirement.
const TRAIL_SEXTANT: &str = "trail_sextant";

/// The selected param value that reads as "this row needs the coordinate
/// tools". No other value is one, and a row that posted none is not.
const SEXTANT_YES: &str = "yes";

/// The three selected item **aliases** the trio's held read joins, in the
/// frozen `nextCoordTool` order. The ids joined are the selected item table's
/// own (`2574` / `2575` / `2576` on both pins) and are never copied numbers: a
/// missing item row is not a held tool.
const TRIO_ITEMS: [&str; 3] = ["trail_sextant", "trail_watch", "trail_chart"];

/// The three selected giver aliases of the published `trio_givers` family, in
/// the frozen `nextCoordTool` order per tool: the sextant is the observatory
/// professor's teach stop and then Murphy's own, the watch is Brother Kojo's
/// alone, and the chart is the professor's again. Each stop resolves its own
/// row from that family by alias and walks only the published `{x, z, plane}`
/// tile — no `PROFESSOR` / `MURPHY` / `KOJO` / `KOJO_EXIT` table is copied,
/// and the `observatory_professor2` lookalike is never this family.
const OBSERVATORY_PROFESSOR: &str = "observatory_professor";
const MURPHY: &str = "murphy";
const BROTHER_KOJO: &str = "brother_kojo";

/// The two closed-handler professor options this arm answers, ASCII-folded
/// against a posted `chat_options` text and matched on nothing else. Never a
/// frozen prefer fragment (`Treasure Trails`, `lost`, `navigation`), never the
/// trawler's or the Clock Tower quest's choices and never the last posted
/// option: a list with no such row waits with the token live.
const PROFESSOR_OPTIONS: [&str; 2] = [
    "Talk about Treasure Trails.",
    "I've lost my navigation chart.",
];

/// The clue-local chat verbs this arm emits: the landed host ops the `Reach`
/// and `dialog.rs` machines already enqueue. `continue` is the posted
/// `chat_continue` step and carries nothing, and `answer` carries the posted
/// option's own 1-based slot. Never the landed dialog sequencer nested into
/// this machine, never a `talk_key` row and never a `kind: "ops"` envelope.
const CONTINUE: &str = "continue";
const ANSWER: &str = "answer";

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
    /// unguarded-dig row walks then Digs with the held Spade, a guarded
    /// row walks, Digs its spawn and then fights the wizard that Dig spawned
    /// until the kill lets it walk back and Dig again, a talk step walks to
    /// its npc and Talks-to it, and a key-keeper row walks to its keeper,
    /// Attacks it and Takes the key it drops; every other row idles with no
    /// action and no walk, and no second callback is emitted for this step.
    Steady,
    /// Trail-end collect: `Steady` on the step whose casket Open went out, and
    /// this call's identify is `none-held`. The one phase that survives
    /// `none-held` — the token lives and the loot verbs are dispatched from
    /// here, one per call. Once the collect is over it runs the three-step
    /// completion below and the token dies.
    Collecting,
}

/// The finished collect's own exit, latched on the live token: the exact
/// `'clue solved'` status, then the `grind-ready` continue kind, then `done`.
/// The first state is the Entrana restore's: while `strippedGear` is non-empty
/// the reclaim owns the calls and none of the three kinds goes out. While the
/// latch is armed nothing loots again — `next` answers the latch before it
/// identifies — and the frozen clock still wins, so a frozen call waits with
/// the latch where it is. Step-scoped like `open`: a re-arm, a different held
/// row and an abort all drop it — the stripped list it may still owe is the
/// session's and is not dropped with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Completion {
    /// The collect is over and the Entrana reclaim is still owed: the restore
    /// runs before the status, the continue and the end, and none of the three
    /// goes out while a listed name is unclaimed.
    Restoring,
    /// The reclaim is done (or was never owed) and the status has not been
    /// posted yet.
    Solved,
    /// The status went out: `grind-ready` is next, and `done` after it.
    GrindReady,
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

/// The live key-keeper hunt on the identified clue row: the keeper this token's
/// `Attack` went out for, and the kill the pickup waits behind. The sibling of
/// `Guardian` — the same owned index and the same freeze-aware grace — with
/// that encounter's own halves left out: no spawn Dig, no prayer, no redig
/// after the kill, and a keeper that leaves the page outside the grace is never
/// `guardian-lost`. Session state on the live token, like `open`, never a
/// second scheduler and never a cached npc page.
struct Keeper {
    /// The posted index this token enqueued `Attack` for. `None` until that
    /// Attack goes out, and dropped again on the kill: an index this token
    /// never Attacked is never a kill.
    owned: Option<Owned>,
    /// The kill was observed: the pickup replaces the hunt, and no second
    /// keeper is Attacked while the one this token killed lies where it fell.
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

/// The live puzzle-box attempt on the identified clue row: the box this token
/// opened, the move it sent and the frozen counters. Session state on the live
/// token, like `open` and `guardian` — never a second scheduler, never a
/// `Phase::Solving`, and never a cached board.
struct Puzzle {
    /// The selected `{alias}_puzzlebox` item this attempt belongs to. The step
    /// already fixes which row is in hand, so this only tells the arm's own
    /// state apart from a later box.
    id: i32,
    /// A readable board has been observed: the Open landed, and this attempt
    /// never opens again.
    opened: bool,
    /// The one `close-modal` went out. Only the frozen close window is left.
    closing: bool,
    /// The solved-or-attempted latch: after the attempt ends — a solved board,
    /// a stall, an unreadable board, `MAX_MOVES`, or a board that never opened
    /// — nothing is opened or closed again while this step stays held, and the
    /// `Steady` dispatch falls through to the row's own arms instead of this
    /// one. `clear_step`, a different step or the frozen reset is what drops it.
    latch: bool,
    /// The outstanding sent click: the board that click was expected to
    /// produce, re-read next call and never the leftover plan.
    want: Option<Board>,
    /// Landed moves and consecutive refusals, the frozen loop's own counters.
    moved: u32,
    stall: u32,
}

impl Puzzle {
    /// A fresh attempt on one box: no board opened yet, nothing sent, and the
    /// counters at zero.
    const fn new(id: i32) -> Self {
        Self {
            id,
            opened: false,
            closing: false,
            latch: false,
            want: None,
            moved: 0,
            stall: 0,
        }
    }
}

/// The three coordinate tools of the frozen `nextCoordTool` order: the first
/// one this call's posted pack does not hold is what the intercept acquires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Sextant,
    Watch,
    Chart,
}

/// The live trio-acquire intercept on the identified clue row: which tool the
/// posted pack was short of, which giver of that tool's own chain this token is
/// working, and whether that stop's chat has been posted open. Session state on
/// the live token, like `open` and `guardian` — never a second scheduler, never
/// a `talk_key` join and never a cached page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Acquire {
    /// The tool `stop` indexes into: the first of the three the posted pack did
    /// not hold when this stop was entered. A call whose own first missing tool
    /// is a later one rebases the state, because the earlier item landed.
    tool: Tool,
    /// Which giver of that tool's chain this token works. Index `0` is the
    /// tool's first stop, and every index past that tool's own list is its last
    /// one, so a tool whose item never lands keeps working the giver it has
    /// instead of inventing a fourth stop or abandoning the chain.
    stop: usize,
    /// This stop's chat has been posted open: the next call that posts it
    /// closed is the stop's own completion, and the chain advances from it.
    open: bool,
}

/// One `next` call's posted puzzle board: the identified component, the
/// observed slot count and the sparse rows the wrapper marshalled out of
/// `snapshot.puzzle_board`, plus the board session generation that rides the
/// click. The board and its generation are one observation, so they are read
/// together.
struct PostedBoard {
    component_id: i32,
    size: i32,
    rows: Vec<PuzzleRow>,
    /// The posted `puzzle_board_generation`, or `None` when the page did not
    /// post one — then no click is dispatched on an invented session.
    generation: Option<u64>,
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
    /// and the next Collecting call runs the completion latch rather than a
    /// second warning.
    full_no_food: bool,
    /// The finished collect's own three-step exit. `None` until the collect is
    /// over, then the step that has not gone out yet — the completion latch.
    completion: Option<Completion>,
    /// The live guarded encounter: absent until this token's first Dig on a
    /// guarded row went out, then owned by this token until a different held
    /// row, an abort or the frozen reset clears it. Like `open`, it is the
    /// session's own state and never a world copy.
    guardian: Option<Guardian>,
    /// The live puzzle-box attempt: absent until this token's first held box
    /// Open went out, then owned by this token until a different held row, an
    /// abort or the frozen reset clears it. Like `open`, it is session state
    /// and never a cached board.
    puzzle: Option<Puzzle>,
    /// The live key-keeper hunt: absent until this token's first keeper
    /// `Attack` went out, then owned by this token until the kill, a different
    /// held row, an abort or the frozen reset clears it. Like `open`, it is the
    /// session's own state and never a cached npc page.
    keeper: Option<Keeper>,
    /// The live trio-acquire intercept: absent until a `Steady` row that needs
    /// the coordinate tools found the posted pack short of one, then owned by
    /// this token until the pack holds the trio, a different held row, an abort
    /// or the frozen reset clears it. Like `open` and `guardian`, it is the
    /// session's own state and never a cached page.
    acquire: Option<Acquire>,
    /// The dest of the last walk this machine dispatched. A live walk is this
    /// token's most recent verb still not arrived, and it is what tells the
    /// gate-toll intercept apart from a page that merely still names an old
    /// short: no walk out, no trip. Cleared with the token, never by a step.
    walk_dest: Option<Tile>,
    /// The live gate-toll shop trip: absent until an intercepted walk named an
    /// unlatched short the posted pack did not hold, then owned by this token
    /// until the trip's exit walk went out, an abort or the frozen reset. The
    /// latch below outlives it, so the same short never shops twice.
    shop: Option<Shop>,
    /// The gate-toll shorts this token has already shopped, in the order the
    /// trips were entered: the frozen `gateItemsTried`, once per item per token
    /// and never once per step. Cleared only by an abort / reset / a new begin.
    shopped: Vec<i32>,
    /// The live Entrana strip on this step: absent until an identified box row
    /// needed one, then owned by this token until the strip settles, a
    /// different held row, an abort or the frozen reset. Like `open`, it is
    /// session state and never a second scheduler.
    strip: Option<Strip>,
    /// The live Entrana restore on the collect's exit: absent until the collect
    /// ends with names still listed, then owned by this token until the list
    /// empties, a different held row, an abort or the frozen reset. The list it
    /// reads outlives it, so a restore that gives up is retried rather than
    /// forgotten.
    restore: Option<Restore>,
    /// The frozen `strippedGear`: the restricted display names the strip put in
    /// the pack and has not put back, in the order they were taken off. Session
    /// state that outlives the step, the trail and the token — it is what the
    /// adapter's `ownsEquipment` reports — and neither the reclaim nor `retry`
    /// clears it: only an emptied list, or the fresh instance [`on_stop`]
    /// builds, does.
    stripped: Vec<String>,
    /// The frozen `abandonedClueId`: the identified row this machine left in
    /// the pack. Set when `abandon` is emitted, read by `begin`, cleared by a
    /// different held id or by `retry` — never by an abort, and never by the
    /// connection-boundary reset. Only the fresh instance [`on_stop`] builds
    /// clears it without a held row of its own.
    abandoned: Option<i32>,
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
            completion: None,
            guardian: None,
            puzzle: None,
            keeper: None,
            acquire: None,
            walk_dest: None,
            shop: None,
            shopped: Vec::new(),
            strip: None,
            restore: None,
            stripped: Vec::new(),
            abandoned: None,
        }
    }

    /// Abort keeps the pause/hold freeze (`InstantTaskClock` contract: abort
    /// clears the deadline only) and emits nothing.
    ///
    /// The frozen `strippedGear` and `abandonedClueId` are not the token's and
    /// are not cleared here: a dead session's list is still what the adapter's
    /// `ownsEquipment` reports, and the latch is cleared by a different held id
    /// or by `retry`. Only the fresh task instance [`on_stop`] clears the list
    /// and the latch: this abort and the connection boundary [`on_reset`] both
    /// keep them, and neither is a `retry`.
    fn abort(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Idle;
        self.generation = 0;
        self.step_id = 0;
        // The walk memory, the live trip and the gate-toll latch are the
        // token's own: a reset, a stop and a second begin all clear them, and
        // nothing else does — not even a different held step, because the
        // frozen `gateItemsTried` is per item per trail and not per step.
        self.walk_dest = None;
        self.shop = None;
        self.shopped.clear();
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

    /// The owned wizard's and the owned keeper's last-seen are the two stamps
    /// this machine reads a duration from, so they are what a thaw shifts: each
    /// grace then measures only the time the session was actually unfrozen and
    /// observing.
    fn shift_instants(&mut self, gap: Duration) {
        if let Some(owned) = self.guardian.as_mut().and_then(|g| g.owned.as_mut()) {
            owned.seen_at += gap;
        }
        if let Some(owned) = self.keeper.as_mut().and_then(|k| k.owned.as_mut()) {
            owned.seen_at += gap;
        }
    }

    /// The step-scoped state a re-arm, a leave and an abort all drop: the
    /// dispatched Open with its `hard` capture, the collect deadline, the
    /// discarded ground ids, the settled-Take watch, the completion latch, the
    /// guarded encounter's owned wizard and post-kill flag, the live
    /// puzzle-box attempt with its latch, the key hunt's owned keeper with its
    /// own post-kill flag, the trio-acquire intercept with its own stop, and
    /// the Entrana strip's and restore's own live attempts.
    ///
    /// The stripped list and the abandon latch are not here: they are the
    /// session's and outlive the step, the token and the connection-boundary
    /// reset — only [`on_stop`], the fresh instance, clears them.
    fn clear_step(&mut self) {
        self.open = None;
        self.clock.deadline = None;
        self.discarded.clear();
        self.pending_take = None;
        self.full_no_food = false;
        self.completion = None;
        self.guardian = None;
        self.puzzle = None;
        self.keeper = None;
        self.acquire = None;
        self.strip = None;
        self.restore = None;
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

    /// One terminal kind the token dies on — `dead`, `guardian-lost` and the
    /// `done` a finished collect ends on. The latch is the abort: the next call
    /// with that token is `stale` and the next begin is a fresh session.
    fn ended(&mut self, kind: &str) -> Value {
        self.abort();
        self.emit(kind)
    }

    /// The posted effective hitpoints at or below zero: the player died, so
    /// the token dies with them. Freeze and yield still win, and a page that
    /// posted no stat is not a zero.
    fn dead(&mut self) -> Value {
        self.ended(DEAD)
    }

    /// The finished collect's own exit, one step per call: the exact
    /// `'clue solved'` status, then the live-token `grind-ready` continue, then
    /// `done` — the abort the token dies on. The first call also arms the
    /// latch, so the collect never loots again; nothing about the solved trail
    /// is looked at beyond it.
    ///
    /// The Entrana restore sits in front of all three steps and not only the
    /// continue: while `strippedGear` is non-empty the reclaim owns the call and
    /// none of the three kinds goes out, so a latched session puts its gear back
    /// before it reports the trail solved and a name that will not go back on
    /// blocks that report rather than racing it. Freeze, yield and the posted
    /// hitpoints were read before this arm and still win over it.
    fn finish(&mut self, selected: Option<&SelectedGameData>, input: &Value) -> Value {
        // The latch arms on the first call whatever the gear list says: from
        // here on the collect never loots, never re-arms and never re-reads the
        // step it ended on. A session whose reclaim is still owed arms
        // `Restoring` rather than the status.
        if self.completion.is_none() {
            self.completion = Some(Completion::Restoring);
        }
        // The reclaim in front of the status, and in front of the continue and
        // the `done` after it.
        if let Some(step) = self.restore(selected, input) {
            return step;
        }
        match self.completion {
            Some(Completion::Restoring) => {
                self.completion = Some(Completion::Solved);
                json!({
                    "kind": "callback.setStatus",
                    "token": self.token,
                    "message": CLUE_SOLVED,
                })
            }
            Some(Completion::Solved) => {
                self.completion = Some(Completion::GrindReady);
                self.emit(GRIND_READY)
            }
            // `GrindReady` — and any state the latch somehow is not — ends the
            // session.
            _ => self.ended(DONE),
        }
    }

    /// The Entrana restore the collect's exit owes while `strippedGear` is
    /// non-empty: the frozen `restoreStrippedGear`, one verb per call over this
    /// call's posted pages.
    ///
    /// In order: every listed name this call's posted worn page already shows
    /// is done with; every listed name that page does not show but the posted
    /// pack holds goes back on (`wear`, the landed equip-from-pack verb); and
    /// what is left is claimed at the bank — the walk to the nearest stand, the
    /// posted booth's own `open-booth`, the frozen make-room deposit while the
    /// pack is too full to take the claim, one `Withdraw-1` per missing name,
    /// and the interface's close — after which the wear pass runs again. The
    /// read is by display name, the way the frozen `Equipment.contains` and
    /// `Bank.withdraw` read it, and by nothing else.
    ///
    /// `None` is the restore's own completion: the list is empty and the
    /// caller's next step may go out. One attempt is bounded by this machine's
    /// own `ENTRANA_WAIT_MS` window; past it the names that are still not back
    /// on are the named `restore-incomplete` log and the next call starts a
    /// fresh attempt, so a name that will not go back on stays listed, is never
    /// a machine kind, and never lets the finish latch report the trail solved.
    fn restore(&mut self, selected: Option<&SelectedGameData>, input: &Value) -> Option<Value> {
        if self.stripped.is_empty() {
            return None;
        }
        // Every listed name is worn again: the frozen list empties and the
        // caller's own completion step may go out. Read before the attempt, so
        // a page that came back on its own needs no verb at all.
        if self.stripped.iter().all(|name| worn_name(input, name)) {
            self.stripped.clear();
            self.restore = None;
            self.clock.deadline = None;
            return None;
        }
        let mut state = match self.restore.take() {
            Some(state) => state,
            None => {
                // A fresh attempt: the previous one's window is not this one's.
                self.clock.arm(ENTRANA_WAIT_MS);
                Restore::default()
            }
        };
        // The wear pass: a listed name the posted worn page does not show but
        // the posted pack holds goes back on now, one per call — and only after
        // this attempt's own bank interface is closed, so nothing is equipped
        // behind an open bank. A wear verb the page never settles is the whole
        // attempt's window running out: the frozen `could not re-equip … — will
        // retry` over the names that are still not back on.
        if let Some(name) = self
            .stripped
            .iter()
            .find(|name| !worn_name(input, name) && pack_holds_name(input, name))
            .cloned()
        {
            if state.bank.opened && bank_open(input) {
                self.restore = Some(state);
                return Some(close_verb(self.token));
            }
            if self.clock.bound_reached() {
                return Some(self.incomplete(input));
            }
            self.restore = Some(state);
            return Some(wear_verb(&name, self.token));
        }
        // The claims: the names this call's posted pack page does not hold
        // either. One in flight at a time — a claim this call no longer reads as
        // missing has landed, and one that is still missing was not observed to
        // land, so this attempt gives up on that name. The next attempt starts
        // with an empty tried list and claims it again.
        let missing = |name: &String| !worn_name(input, name) && !pack_holds_name(input, name);
        if let Some(sent) = state.sent.clone() {
            if !missing(&sent) {
                state.sent = None;
            } else {
                state.tried.push(sent);
                state.sent = None;
            }
        }
        let claim = self
            .stripped
            .iter()
            .find(|name| missing(name) && !state.tried.contains(name))
            .cloned();
        match claim {
            Some(name) => {
                // The bank this claim needs is the shared approach: the walk to
                // the nearest stand, the posted booth's own open, and then the
                // claim itself.
                if let Some(step) = self.bank_approach(&mut state.bank, input) {
                    self.restore = Some(state);
                    return Some(step);
                }
                // The frozen make-room deposit, between the open bank and the
                // claim: a pack that cannot take the withdrawn name banks what
                // the frozen predicate takes first, so a pack that filled up
                // over the trail does not make the reclaim wait forever.
                if let Some(step) = self.make_room(selected, &mut state, input) {
                    self.restore = Some(state);
                    return Some(step);
                }
                state.sent = Some(name.clone());
                self.restore = Some(state);
                Some(withdraw_verb(&name, self.token))
            }
            None if bank_open(input) => {
                // Nothing left to claim and the interface is still up: the close
                // is this call's verb, and the names that did not come back are
                // read on the call after it.
                self.restore = Some(state);
                Some(close_verb(self.token))
            }
            // Nothing left to claim with the interface down: the attempt is
            // over and the names that are still missing are the named log.
            None => Some(self.incomplete(input)),
        }
    }

    /// The frozen `restoreStrippedGear`'s own make-room deposit, between the
    /// open bank and the claim: while this call's posted pack has fewer free
    /// slots than the listed names its pages do not hold, one posted pack row
    /// the frozen predicate takes goes to the bank, so the gear about to be
    /// claimed has somewhere to land. Without it a pack that filled up over the
    /// trail never takes a claim: the recoverable path that reaches the exit
    /// with a full pack is a real one — the casket reward fills it — and the
    /// reclaim would retry against the same full pack forever, holding the
    /// finish latch and starving the sibling grind.
    ///
    /// The predicate is the frozen `!want.includes(name) && CLUE_DB[id] ===
    /// undefined && CASKET_IDS[id] === undefined`: a listed name is never
    /// banked here, and a posted row whose id the selected trail facts name — a
    /// clue scroll, a casket, a challenge scroll — is left alone. Everything
    /// else in the pack, food included, is the frozen deposit's to take.
    ///
    /// The frozen `depositAllMatching` banks every match in one action; this
    /// machine banks one row per call and never the same row twice inside one
    /// attempt, so a bank that refuses the deposit ends the attempt — the
    /// claim it was making room for goes out as usual — rather than repeating
    /// the same verb forever. A page that posted no `inv_size` has not said how
    /// full the pack is, and no room is made on an invented one.
    fn make_room(
        &self,
        selected: Option<&SelectedGameData>,
        state: &mut Restore,
        input: &Value,
    ) -> Option<Value> {
        // The row this attempt's deposit went out for: a page that still holds
        // it did not observe it land, so this attempt does not send it again.
        if let Some(sent) = state.room_sent.take() {
            if pack_holds_name(input, &sent) {
                state.room_tried.push(sent);
            }
        }
        let free = i64::from(input.get("inv_size").and_then(i32_of)?) - occupied(input);
        let missing = self
            .stripped
            .iter()
            .filter(|name| !worn_name(input, name) && !pack_holds_name(input, name))
            .count();
        if free >= missing as i64 {
            return None;
        }
        let name = make_room_row(selected, input, &self.stripped, &state.room_tried)?;
        state.room_sent = Some(name.clone());
        Some(deposit_verb(&name, self.token))
    }

    /// The `restore-incomplete` give-up: the frozen `could not re-equip … — will
    /// retry` over the names this call's posted worn page still does not show,
    /// with the attempt dropped so the next call starts a fresh one. The list
    /// keeps them, so the finish latch stays blocked and nothing downstream ever
    /// reads the reclaim as done. A log through `callback.log` and never a
    /// machine kind.
    fn incomplete(&mut self, input: &Value) -> Value {
        let names: Vec<&str> = self
            .stripped
            .iter()
            .filter(|name| !worn_name(input, name))
            .map(String::as_str)
            .collect();
        self.restore = None;
        self.clock.deadline = None;
        json!({
            "kind": "callback.log",
            "token": self.token,
            "message": restore_incomplete(&names.join(", ")),
        })
    }

    /// The Entrana strip in front of the identified box row's own arms: the
    /// frozen `heldClueNeedsEntranaStrip` with `bankFirst`'s own two halves, one
    /// verb per call.
    ///
    /// Membership is this row's own selected `trail_coord`, decoded the landed
    /// way and inside the cap box — `entrana_coord` — and nothing else: a casket
    /// never arms it and no copied `CLUE_DB` is consulted. In order: every
    /// posted worn row whose name the frozen matcher folds is unequipped with
    /// the landed `unequip` verb — the worn row's own `Remove`, because the
    /// host's `wear` resolves inventory rows alone — and listed unless it is one
    /// of the two hard-trail dagger ids; then every posted pack row the matcher
    /// folds goes to the bank, one per call — the names the unequip put there,
    /// the dagger ids that were never listed, and a restricted item that was
    /// carried rather than worn — and the interface closes before the row's own
    /// arms run.
    ///
    /// The deposit is the frozen `depositAllMatching(name => !isKeep(name))` cut
    /// to the matcher's own rows: every regex-matching name in the pack, listed
    /// or not, and never the ordinary loot deposit.
    ///
    /// `None` is the fall-through: not a box row, or a strip this step already
    /// settled. The listed names outlive the step — they are the restore's own
    /// list — and a strip that cannot finish re-arms and logs rather than
    /// walking with the gear in hand: the frozen `bankFirst` returns false on
    /// that same failure and the trail does not run.
    fn strip(&mut self, row: &TrailMembershipRow, input: &Value) -> Option<Value> {
        if !entrana_coord(row) {
            return None;
        }
        let mut state = match self.strip.take() {
            Some(state) => state,
            None => {
                self.clock.arm(ENTRANA_WAIT_MS);
                Strip::default()
            }
        };
        if state.settled {
            self.strip = Some(state);
            return None;
        }
        // The unequip pass. The verb repeats while the row stays on the posted
        // page — the host fails closed on an item that is already gone — and a
        // whole window of that is the frozen `still holding …` line with a
        // fresh attempt behind it.
        if let Some(worn) = pick_worn(input) {
            if self.clock.bound_reached() {
                self.clock.arm(ENTRANA_WAIT_MS);
                return Some(json!({
                    "kind": "callback.log",
                    "token": self.token,
                    "message": STILL_HOLDING,
                }));
            }
            self.list(&worn);
            self.strip = Some(state);
            return Some(unequip_verb(worn.name, self.token));
        }
        // The deposit pass: a posted pack row the frozen matcher folds goes to
        // the bank, one per call and whatever put it there — the unequip above,
        // a dagger id that is never listed, or a restricted item the player was
        // carrying rather than wearing.
        if let Some(name) = pick_pack_restricted(input) {
            if let Some(step) = self.bank_approach(&mut state.bank, input) {
                self.strip = Some(state);
                return Some(step);
            }
            if self.clock.bound_reached() {
                self.clock.arm(ENTRANA_WAIT_MS);
                return Some(json!({
                    "kind": "callback.log",
                    "token": self.token,
                    "message": STILL_HOLDING,
                }));
            }
            self.strip = Some(state);
            return Some(deposit_verb(&name, self.token));
        }
        // The exit: the interface closes once while it is up, and then the row's
        // own arms run for the rest of the step.
        if bank_open(input) {
            self.strip = Some(state);
            return Some(close_verb(self.token));
        }
        state.settled = true;
        self.strip = Some(state);
        None
    }

    /// One step of the shared Entrana bank approach: the walk to the nearest
    /// stand, then the posted booth's own open, then the interface — which is
    /// the `None` that lets the caller's own deposit or claim go out.
    ///
    /// The approaching half is bounded by this machine's own `ENTRANA_WAIT_MS`
    /// window, armed when the walk goes out: past it the approach logs the named
    /// `restore-walk-failed` — the caps' own name for this bank trip — and
    /// re-arms, so a stand the page never posts is a named failure rather than a
    /// silent hang. Freeze-honored like every other window here.
    fn bank_approach(&mut self, bank: &mut BankApproach, input: &Value) -> Option<Value> {
        if bank_open(input) {
            bank.opened = true;
            return None;
        }
        if !bank.walked {
            bank.walked = true;
            self.clock.arm(ENTRANA_WAIT_MS);
            return Some(walk_nearest_bank_verb(self.token));
        }
        if let Some(booth) = nearest_booth(input) {
            if booth_arrival(&booth, input) == Arrival::Arrived {
                return Some(open_booth_verb(&booth, self.token));
            }
        }
        if self.clock.bound_reached() {
            *bank = BankApproach::default();
            self.clock.arm(ENTRANA_WAIT_MS);
            return Some(json!({
                "kind": "callback.log",
                "token": self.token,
                "message": restore_walk_failed(),
            }));
        }
        Some(self.emit("wait"))
    }

    /// List one stripped name the way the frozen `strippedGear` does: never
    /// twice under a different case, and never one of the two hard-trail dagger
    /// ids, which are unequipped like any other match and left off the list.
    fn list(&mut self, worn: &Worn<'_>) {
        if worn.id.is_some_and(|id| DDS_IDS.contains(&id)) {
            return;
        }
        if self
            .stripped
            .iter()
            .any(|name| name.eq_ignore_ascii_case(worn.name))
        {
            return;
        }
        self.stripped.push(worn.name.to_string());
    }

    /// The adapter's `ownsEquipment` read: the frozen formula's own half that
    /// exists this slice — `strippedGear` is non-empty. True means
    /// do-not-grind-equip, it is read off the machine and never off a page, and
    /// it survives the token the list was made on: a dead session still answers
    /// true while the names are unclaimed. `bankedThisSolve` is not this slice's
    /// and is never set.
    fn owns_equipment(&self) -> Value {
        json!({
            "kind": "ownsEquipment",
            "token": self.token,
            "owns": !self.stripped.is_empty(),
        })
    }

    /// The frozen `retry`'s own clear, and only it: the abandon latch. The live
    /// token is not aborted, `strippedGear` is not cleared, and no other latch
    /// exists yet. Distinct from `on_reset`, the connection boundary that kills
    /// the token and keeps this list, and from `on_stop`, the fresh instance
    /// that clears the list too.
    fn retry(&mut self) -> Value {
        self.abandoned = None;
        json!({ "kind": "retry", "token": self.token })
    }

    /// The abandon latch's own read over one identified row: `true` while the
    /// same id is still the one this machine left in the pack, and the clear a
    /// different id — or no identified row at all — makes on the way, exactly
    /// the frozen `validate`'s own clearing. `None` is a page that holds no
    /// identified row, which is a different state and not the same id.
    fn abandon_seen(&mut self, id: Option<i32>) -> bool {
        if id.is_some() && id == self.abandoned {
            return true;
        }
        self.abandoned = None;
        false
    }

    /// The frozen `abandon`: the identified row is left in the pack and the
    /// latch is what a later begin and `retry` read. The terminal aborts the
    /// token it ends, the way `done` does, and the frozen reclaim that follows
    /// an abandon is not this card's — the names stay listed for the next
    /// session's own restore.
    ///
    /// Not a production trigger: the emit is an explicit machine-state write
    /// (the sibling of `retry`'s clear), the frozen production that reaches it
    /// on its own — `MAX_STEPS`, the `blockReason` retries, `STEP_ATTEMPTS` —
    /// is a later card's, and no adapter calls this seat.
    fn abandoned(&mut self) -> Value {
        self.abandoned = Some(self.step_id);
        self.ended(ABANDON)
    }

    fn begin(&mut self, selected: Option<&SelectedGameData>, input: &Value) -> Value {
        // A second begin aborts the live token first and emits nothing for
        // it: begin has no continue kind, and a refused begin is still a
        // refusal, not a resumed session.
        self.abort();
        let row = match identify(selected, input) {
            Ok(row) => row,
            // A page that holds only a selected challenge scroll is the same
            // `none-held` identify has always made; the scroll's own parent is
            // the step the token keeps, exactly as it is for a live session.
            Err(NONE_HELD) => match challenge_step(selected, input) {
                Some(parent) => parent,
                None => {
                    self.abandon_seen(None);
                    return self.aborted(NONE_HELD);
                }
            },
            Err(reason) => {
                self.abandon_seen(None);
                return self.aborted(reason);
            }
        };
        if self.abandon_seen(Some(row.id)) {
            // The row this machine left in the pack is still the one held: the
            // frozen `validate` is false while the same id remains, so no token
            // is handed out for it and the sibling grind is not stolen. A
            // different row — and a page that holds none — cleared the latch
            // above.
            return self.aborted(ABANDONED);
        }
        if row.access.as_deref() == Some(CONSTRAINED) {
            // The one identified row this machine will not play: the packed
            // 3554 clue's own selected bound. The refusal is the same one a
            // live session makes, and no token is handed out for it.
            return self.aborted(CONSTRAINED);
        }
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
        // The posted effective hitpoints kill the token before anything else
        // this call could do, whatever phase the session was in. A page that
        // posted no stat at all is not a zero, and the frozen clock and the
        // interrupt above still win over it.
        if posted_i32(input, "hitpoints").is_some_and(|hp| hp <= 0) {
            return self.dead();
        }
        if self.completion.is_some() {
            // The collect is over and latched: its own exit runs before any
            // identify, so a latched session never loots and never re-arms.
            // The Entrana restore, when names are still listed, is the arm that
            // exit takes first — and the only one that reads this call's pages.
            return self.finish(selected, input);
        }
        // One identify per call, and the same landed `identify_step` the
        // machine always made: membership is never re-derived here. The first
        // seam is `none-held`, which a Collecting token — or the Steady step
        // whose casket Open already went out — survives. The second is the
        // challenge scroll: `identify_step` reads no challenge id, so a page
        // that holds only a selected one joins its parent talk step, and the
        // parent — never the scroll id — is the step this token keeps.
        let row = match identify(selected, input) {
            Ok(row) => Some(row),
            Err(NONE_HELD) if self.collects() => None,
            Err(NONE_HELD) => match challenge_step(selected, input) {
                Some(parent) => Some(parent),
                None => return self.aborted(NONE_HELD),
            },
            Err(reason) => return self.aborted(reason),
        };
        // The abandon latch's own clear, the frozen `validate`'s: a different
        // held row, and a page that holds none, both clear it. The same id can
        // never be read on a live token — the `abandon` terminal that sets the
        // latch kills the token it was set on — so this seat only clears.
        self.abandon_seen(row.as_ref().map(|row| row.id));
        let Some(row) = row else {
            self.enter_collect();
            return self.collect(selected, input);
        };
        if row.access.as_deref() == Some(CONSTRAINED) {
            // The packed 3554 clue's own bound, held by a live session: the
            // same refusal a begin makes — no verb, no walk and no token left.
            return self.aborted(CONSTRAINED);
        }
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
            Phase::Steady => {
                // The gate-toll walk intercept sits in front of every `Steady`
                // arm: a live walk that is not arriving while the page names an
                // unlatched short the posted pack does not hold is the shop's,
                // whichever arm walked, and a call that is neither of those
                // falls through untouched.
                if let Some(step) = self.toll(selected, input) {
                    return step;
                }
                match casket_name(selected, row) {
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
                    // Not a held casket: this row's own held puzzle box, the
                    // landed search dispatch, the guarded encounter, the sibling
                    // unguarded-dig dispatch, the talk step, the key-keeper hunt,
                    // or the idle every other row keeps.
                    None => match puzzle_box(selected, row) {
                        // The row's own box on this call's page, or the box this
                        // token already opened — while this step's solved-or-
                        // attempted latch is unset. A held box the identified row
                        // does not name is not this row's, a desc-only row without
                        // one keeps the idle, and a latched step is done with the
                        // board: it takes the `Steady` arms below, which is where
                        // the nine puzzle riddles' talk step lives.
                        Some((id, name)) if self.puzzle_arm(id, input) => {
                            self.puzzle(id, name, input, selected)
                        }
                        _ => self.steady(row, input, selected),
                    },
                }
            }
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
            // The WARNING was the last thing this collect had to say: the exit
            // is the machine's own completion, never a second warning and
            // never the landed `none-held` abort. The Entrana restore, when
            // names are listed, is that exit's own first arm.
            return self.finish(selected, input);
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
            return self.empty(selected, input);
        };
        let Some(drop) = pick_ground(input, here, &self.discarded) else {
            return self.empty(selected, input);
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
    /// window — and after it the collect is over: the exit is the machine's own
    /// three-step completion, never the landed `none-held` abort, and the
    /// Entrana restore is that exit's own first arm.
    fn empty(&mut self, selected: Option<&SelectedGameData>, input: &Value) -> Value {
        if self.clock.bound_reached() {
            return self.finish(selected, input);
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
    fn puzzle_arm(&self, id: i32, input: &Value) -> bool {
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
    fn puzzle(
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
    fn puzzle_closed(&mut self, box_id: i32, name: &str) -> Value {
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
    fn puzzle_stall(&mut self) -> Value {
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
    fn puzzle_close(&mut self) -> Value {
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
    fn puzzle_closing(&mut self) -> Value {
        if self.clock.bound_reached() {
            if let Some(state) = self.puzzle.as_mut() {
                state.latch = true;
            }
        }
        self.emit("wait")
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
    fn search(&mut self, row: &TrailMembershipRow, input: &Value) -> Value {
        let Some(tile) = search_tile(row) else {
            // Not a search membership: the coord-bearing rows without the loc
            // pin belong to the sibling dig classify, and the desc-only rows
            // with no decodable coord, the constrained 3554 clue and the rest
            // are identified and then idle.
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
    /// the coordinate-trio acquire intercept, the guarded encounter, the sibling
    /// unguarded-dig dispatch, the talk step, the key-keeper hunt, or the idle
    /// every other row keeps.
    ///
    /// A latched puzzle step arrives here as well: its own box is done with, so
    /// this is where the nine desc-only puzzle riddles the selected
    /// `talk_key.talk` family publishes re-talk, and where every other latched
    /// row idles.
    ///
    /// The search pin decides first: `search_tile` is the `trail_loc=^true`
    /// membership and the landed dispatch re-reads its own tile from it, so a
    /// search row can never reach either dig arm and Dig is never a second
    /// search classify. The trio intercept follows, on the row's own selected
    /// `trail_sextant=yes` and this call's posted pack alone: it is an arrival
    /// gate in front of both dig arms — a row that needs the coordinate tools
    /// never Digs before the posted pack holds them — and it is a no-op
    /// fall-through the moment it does, so the guarded encounter and the
    /// unguarded-dig dispatch keep their precedence and their rules untouched.
    /// The guarded pin decides next, and the encounter it picked up — session
    /// state on this same token — is what the following calls read. The talk
    /// step follows, so the casket Open, the row's own held puzzle box and all
    /// three walk arms keep their precedence and no talk row is ever a second
    /// classify of them. The key-keeper hunt is last, entered on its own
    /// selected family alone, and every other held type idles exactly as
    /// before: a talk step is never hunted for a key, and a key keeper is never
    /// Talked-to.
    fn steady(
        &mut self,
        row: &TrailMembershipRow,
        input: &Value,
        selected: Option<&SelectedGameData>,
    ) -> Value {
        // The Entrana strip sits in front of every `Steady` arm this row owns
        // and with them in front of the walk they would make: an identified row
        // whose own selected `trail_coord` decodes inside the cap box is stripped
        // — and its restricted names banked — before the search, dig or talk arm
        // it belongs to ever runs. Every other row falls straight through, and a
        // strip this step already settled never re-enters.
        if let Some(step) = self.strip(row, input) {
            return step;
        }
        if search_tile(row).is_some() {
            return self.search(row, input);
        }
        if needs_trio(row) {
            if let Some((givers, tool)) = trio_plan(selected, input) {
                return self.acquire(&givers, tool, input);
            }
        }
        if let Some(tile) = guarded_tile(row) {
            return self.guarded(row, tile, input, selected);
        }
        if let Some(tile) = dig_tile(row) {
            return self.dig(tile, input);
        }
        if let Some(talk) = talk_step(selected, row.id) {
            return self.talk(talk, input, selected);
        }
        self.keys(row, input, selected)
    }

    /// `Steady` on an identified row that needs the coordinate tools while this
    /// call's posted pack is short of one or more of them: the acquire chain,
    /// one verb per call.
    ///
    /// The chain is the frozen `nextCoordTool` order over the selected
    /// `trio_givers` rows and nothing else. The sextant is the observatory
    /// professor's teach stop and then Murphy's own, the watch is Brother Kojo's
    /// alone, and the chart is the professor's again; a stop walks to its
    /// giver's published `{x, z, plane}` tile with `plane` as the verb's `level`
    /// and never to a frozen coordinate. The Talk-to is the talk arm's
    /// unique-spawn rule: the posted npc whose own packed id is the giver's, or
    /// failing that whose posted display name is, standing on that tile inside
    /// the frozen `ARRIVE_RADIUS` and carrying a posted talk action. A wanderer
    /// is never chased, nothing is Cleared, a lookalike alias is not this giver,
    /// and a page with no posted `here`, no posted npc page or no posted match
    /// waits at the tile with the token live.
    ///
    /// Behind that giver's open chat the arm sends what the stop's own giver
    /// publishes, and only that: at the professor one posted option whose text
    /// ASCII-folds to a selected closed-handler literal, answered with its own
    /// posted 1-based slot, and at Murphy and Kojo a posted `chat_continue`.
    /// A posted option list with no such row waits — never the last option,
    /// never a frozen fragment and never another quest's choice — and a chat
    /// with neither an option nor a continue is waited out rather than
    /// Talked-to again behind it.
    ///
    /// A stop is complete when its own chat has been posted open and then
    /// closed: the chat is the observation, so no deadline, hop or world state
    /// is invented for it. `hasAllTrio` is the intercept's own completion, read
    /// off this call's posted pack — only then do the landed guarded and
    /// unguarded dig arms run. A giver that is not posted, an item that does not
    /// land, a full pack and a locked door all wait with the token live:
    /// nothing is fetched, banked, shopped, Dropped or abandoned here, and no
    /// completion kind is ever emitted from this arm.
    fn acquire(&mut self, givers: &TrioGivers<'_>, tool: Tool, input: &Value) -> Value {
        // This tool's own state: a call whose first missing tool is a later one
        // has watched the earlier tool's item land, so the stop index rebases to
        // that tool's first giver rather than carrying the old position over.
        let ready = dialog_ready(input);
        let mut state = match self.acquire {
            Some(state) if state.tool == tool => state,
            _ => Acquire {
                tool,
                stop: 0,
                open: false,
            },
        };
        if state.open && !ready {
            // The stop's own chat closed after it was posted open: this stop is
            // done, so the chain moves to that tool's next giver — its own last
            // one once the list is exhausted.
            state.stop = (state.stop + 1).min(last_stop(tool));
        }
        state.open = ready;
        self.acquire = Some(state);
        let stop = stop_of(givers, tool, state.stop);
        let Some(spawn) = stop.row.spawn.as_ref() else {
            // The entry filter publishes a spawn for every giver this arm walks;
            // a selected family that stops doing so waits rather than walking to
            // an invented tile.
            return self.emit("wait");
        };
        let tile = Tile {
            x: spawn.x,
            z: spawn.z,
            level: spawn.plane,
        };
        match arrival(tile, input) {
            // No posted `here`: no arrival claim to make and no walk to measure.
            Arrival::Unknown => self.emit("wait"),
            Arrival::Walking => self.walk(tile),
            // Arrived behind an open chat: the answer, the continue, or nothing
            // this arm may send.
            Arrival::Arrived if ready => self.chat(&stop, input),
            Arrival::Arrived => match input.get("npcs").and_then(Value::as_array) {
                Some(page) => match pick_at_spawn(&NpcIdentity::Giver(stop.row), page, tile) {
                    Some(pick) => self.npc_verb(&pick),
                    // Arrived with no posted row of this giver's identity on the
                    // tile: stay there and wait, never chase a wanderer, never
                    // Clear and never take a second target.
                    None => self.emit("wait"),
                },
                // No posted npc page this call: no giver can be observed, so
                // there is nothing to Talk-to.
                None => self.emit("wait"),
            },
        }
    }

    /// The open giver chat this arm may drive: the professor's one selected
    /// option, then a posted `chat_continue`, then a wait for everything else.
    ///
    /// The rule is the stop's own giver and not the text alone — the professor
    /// is the only giver this chain answers options in, so a posted option list
    /// at Murphy or Kojo waits. A list the professor's two literals do not match
    /// waits as well: the last option, a frozen fragment and another quest's
    /// choice are all nothing this arm may send, and an open chat with neither
    /// an option nor a continue is waited out rather than Talked-to again.
    fn chat(&self, stop: &Stop<'_>, input: &Value) -> Value {
        if stop.professor {
            if let Some(option) = professor_option(input) {
                return json!({
                    "kind": ANSWER,
                    "token": self.token,
                    "option": option,
                });
            }
        }
        if options_posted(input) {
            return self.emit("wait");
        }
        if continue_posted(input) {
            return self.emit(CONTINUE);
        }
        self.emit("wait")
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
    /// (the arrived no-Spade tick as the named `supplies-needed` class) and the
    /// fight never starts early.
    fn spawn(&mut self, tile: Tile, input: &Value) -> Value {
        match arrival(tile, input) {
            Arrival::Unknown => self.emit("wait"),
            Arrival::Walking => self.walk(tile),
            Arrival::Arrived if !spade_posted(input) => self.emit(SUPPLIES_NEEDED),
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
                // is this token's kill; outside it the wizard is gone without
                // ever being seen at zero health, so the encounter is lost and
                // the token dies with it — never a redig.
                let within = self
                    .guardian
                    .as_ref()
                    .and_then(|g| g.owned.as_ref())
                    .is_some_and(|owned| {
                        now.saturating_duration_since(owned.seen_at)
                            < Duration::from_millis(KILL_GRACE_MS)
                    });
                if !within {
                    return self.ended(GUARDIAN_LOST);
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
            pick_npc(
                names,
                page,
                self_slot,
                input.get("here").and_then(posted_tile),
            )
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
    /// without the `Spade` on the posted pack page is the named
    /// `supplies-needed` wait-class: the frozen `no Spade held` refusal is not
    /// this machine's token, nothing is acquired, and the token stays live for
    /// the pack that posts it. Dig repeats while this same clue id stays held,
    /// the way the casket's Open does, and a `none-held` after it still aborts.
    fn dig(&mut self, tile: Tile, input: &Value) -> Value {
        match arrival(tile, input) {
            Arrival::Unknown => self.emit("wait"),
            Arrival::Walking => self.walk(tile),
            Arrival::Arrived if spade_posted(input) => self.dig_verb(),
            Arrival::Arrived => self.emit(SUPPLIES_NEEDED),
        }
    }

    /// The landed arrival walk to a tile: the same verb the search arm and both
    /// dig arms dispatch, so `walk` has one shape on this machine.
    ///
    /// Every walk the machine dispatches is remembered here and nowhere else,
    /// including the gate-toll trip's own walk to the selected spawn: that
    /// memory is what makes a walk live, and the trip is latched before its
    /// first one goes out, so a shop-phase walk is never read as a new
    /// intercept.
    fn walk(&mut self, tile: Tile) -> Value {
        self.walk_dest = Some(tile);
        json!({
            "kind": "walk",
            "token": self.token,
            "x": tile.x,
            "z": tile.z,
            "level": tile.level,
        })
    }

    /// The gate-toll walk intercept: one live walk this token dispatched that
    /// has not arrived, whose page names a `Carry` short for the selected
    /// Shantay pass that the posted pack does not hold. `None` is the
    /// fall-through — the row's own `Steady` arms run exactly as they did
    /// before this intercept existed.
    ///
    /// It is the walk's intercept and not a second identify: the step is the one
    /// the landed identify already returned, and this reads no membership of its
    /// own. It sits in front of every arm a walk can come from — the search
    /// dispatch, both dig arms, the trio acquire chain, the talk step and the
    /// key hunt — so a failed walk is the same failure whichever arm armed it,
    /// and it is never an arm *after* the talk step.
    ///
    /// The short is the navigator's, never this machine's: a posted `Carry` row
    /// of the selected pass' own id is the whole nomination, and a `NoPath`'s
    /// dest geometry is never read as a shopping list. The Al Kharid toll's
    /// coins, the extra-item Rope and every other named short are not this item
    /// and never shop.
    ///
    /// Once per item id per token: the id is latched before the trip's first
    /// verb goes out, so the Shantay walk can never re-enter this intercept and
    /// a second failure of the same walk never starts a third trip.
    fn toll(&mut self, selected: Option<&SelectedGameData>, input: &Value) -> Option<Value> {
        if let Some(shop) = self.shop {
            return self.shop_trip(shop, input);
        }
        // The selected pass, by alias: no selected item is no shop at all, and
        // the id every posted row is joined to is that item's own.
        let item = selected?.item_by_alias(SHANTAY_PASS)?;
        if self.shopped.contains(&item.id) || holds(input, item.id) {
            // Already shopped for this token, or the pass is on this call's
            // posted pack page: there is nothing to buy.
            return None;
        }
        let dest = self.walk_dest?;
        if arrival(dest, input) != Arrival::Walking || !carry_names(input, item.id) {
            // No live walk — an arrived (or unposted) `here` is not the failure
            // this page is naming — and no posted short for the pass.
            return None;
        }
        self.shopped.push(item.id);
        self.shop = Some(Shop {
            id: item.id,
            dest,
            step: ShopStep::Stand,
            failed: false,
            closed: false,
        });
        Some(self.walk(shantay_spawn()))
    }

    /// One call of the live shop trip, one verb per call: the walk to the
    /// selected Shantay spawn, the posted keeper's `Trade`, the posted stock
    /// row's buy, and then the exit.
    ///
    /// Nothing is invented on any step. The keeper is the posted npc of the
    /// selected type — or, failing that, of the selected display name — standing
    /// on the spawn inside the frozen `ARRIVE_RADIUS` and listing a posted
    /// `Trade`; the buy rides the posted stock row's own name, id, slot and
    /// component; and the settle is the posted pack page holding the short's own
    /// id. A step whose own fact the page never posts waits inside the machine's
    /// own `SHOP_WAIT_MS` window and then gives up — with the token live and the
    /// latch kept, so the named `no-shop` is never a second trip.
    fn shop_trip(&mut self, mut shop: Shop, input: &Value) -> Option<Value> {
        let spawn = shantay_spawn();
        match shop.step {
            // The trip is over: nothing is observed again, and the exit owes at
            // most one call per step of its own.
            ShopStep::Exit => self.shop_exit(shop, input),
            ShopStep::Stand => match arrival(spawn, input) {
                // No posted `here`: no arrival claim to make and no walk to
                // measure, so the trip waits where it is.
                Arrival::Unknown => self.shop_wait(shop, input),
                Arrival::Walking => {
                    self.shop = Some(shop);
                    Some(self.walk(spawn))
                }
                Arrival::Arrived => {
                    // The first arrived call opens this step's own observation
                    // window; the walk itself is never bounded by it.
                    if self.clock.deadline.is_none() {
                        self.clock.arm(SHOP_WAIT_MS);
                    }
                    // An open chat is not a tick to click the keeper: the same
                    // rule the talk arm reads, and the count dialog with it.
                    let pick = if dialog_ready(input) || count_open(input) {
                        None
                    } else {
                        input
                            .get("npcs")
                            .and_then(Value::as_array)
                            .and_then(|page| pick_at_spawn(&NpcIdentity::Shantay, page, spawn))
                    };
                    match pick {
                        Some(pick) => {
                            shop.step = ShopStep::Trade;
                            self.shop = Some(shop);
                            // The interface's own window starts with the click.
                            self.clock.arm(SHOP_WAIT_MS);
                            Some(self.npc_verb(&pick))
                        }
                        // Arrived with no posted keeper of this identity on the
                        // spawn: stay there and wait it out rather than chasing
                        // a wanderer, and give up when the window ends.
                        None => self.shop_wait(shop, input),
                    }
                }
            },
            ShopStep::Trade => {
                if !shop_open(input) {
                    // No posted open interface: unobserved is not a closed shop,
                    // so this waits rather than clicking blind.
                    return self.shop_wait(shop, input);
                }
                match buy_row(input, shop.id) {
                    Some(row) => {
                        shop.step = ShopStep::Buy;
                        self.shop = Some(shop);
                        // The settle's own window starts with the click.
                        self.clock.arm(SHOP_WAIT_MS);
                        Some(buy_verb(&row, self.token))
                    }
                    // The interface is up and this short's own stock row is not
                    // on it: nothing here may be clicked, so the trip gives up
                    // instead of pressing a row it did not read.
                    None => {
                        shop.failed = true;
                        shop.step = ShopStep::Exit;
                        self.shop_exit(shop, input)
                    }
                }
            }
            ShopStep::Buy => {
                if holds(input, shop.id) {
                    // The short landed on the posted pack page: the trip is
                    // over and its interface is what is left.
                    return self.shop_exit(shop, input);
                }
                self.shop_wait(shop, input)
            }
        }
    }

    /// This step's own wait: inside the machine's `SHOP_WAIT_MS` window the call
    /// is a `wait` with the token live, and past it the short did not land — the
    /// trip gives up, which is the exit with `failed` set.
    fn shop_wait(&mut self, mut shop: Shop, input: &Value) -> Option<Value> {
        if !self.clock.bound_reached() {
            self.shop = Some(shop);
            return Some(self.emit("wait"));
        }
        // The step's own posted fact never came: the trip gives up — that is the
        // exit with `failed` set — and this step never waits again on a window
        // it already spent.
        shop.failed = true;
        shop.step = ShopStep::Exit;
        self.shop_exit(shop, input)
    }

    /// The trip's exit, one step per call and every one of them a posted fact or
    /// the trip's own outcome: the posted interface is closed once when it is
    /// up, a short that did not land exits with the named `no-shop`, and then
    /// the walk back to the original dest goes out.
    ///
    /// `None` is the fall-through the row's own arm takes once that walk is out.
    /// The latch still has the short, so the arm's next failed walk is never a
    /// third trip, and the walk back is the same tile that arm was walking to
    /// before the trip — the second walk, made here so it happens on a page that
    /// posts no `here` at all.
    fn shop_exit(&mut self, mut shop: Shop, input: &Value) -> Option<Value> {
        // No step is waiting any more: the exit is driven by this call's posted
        // interface, the trip's own outcome and the dest, never by a clock.
        self.clock.deadline = None;
        if !shop.closed && shop_open(input) {
            shop.closed = true;
            self.shop = Some(shop);
            return Some(self.emit("close-modal"));
        }
        if shop.failed {
            // The kind goes out once: the walk back is the next call's.
            shop.failed = false;
            self.shop = Some(shop);
            return Some(self.emit(NO_SHOP));
        }
        self.shop = None;
        Some(self.walk(shop.dest))
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

    /// `Steady` on an identified talk membership: the selected
    /// `talk_key.talk` row this held step owns, walked to and then Talk-to'd,
    /// one verb per call.
    ///
    /// The membership is the caller's, resolved from the selected `talk_key.talk`
    /// family alone: a key keeper the family does not publish never reaches this
    /// arm, so its `wait` is never a key hunt's fallthrough and no keeper is
    /// ever Talked-to.
    ///
    /// The arm opens on the posted chat facts, because an open chat is not a
    /// tick to Talk-to again: a posted `count_dialog_open` is answered — with
    /// this step's own selected challenge answer and nothing else — and a
    /// landed `dialog_ready` waits the tick out. An unobserved slot is neither.
    ///
    /// A step with a published spawn walks to that `{x, z, plane}` tile and
    /// Talks-to only a posted npc of this step's identity standing on it; the
    /// steps without one take the nearest posted npc of that identity. Either
    /// way a page with no posted match, no posted `here`, no talk action or no
    /// postable identity is a `wait` with the token live: nothing is invented,
    /// nothing is Cleared, and no second target is chased.
    fn talk(
        &mut self,
        talk: &TalkKeyTalkRow,
        input: &Value,
        selected: Option<&SelectedGameData>,
    ) -> Value {
        if count_open(input) {
            // The count dialog this step's own challenge scroll opens: the
            // selected answer, never a computed or remembered one. A step the
            // pin answered nothing for waits the tick out.
            return match challenge_answer(selected, talk) {
                Some(value) => json!({
                    "kind": "answer-count",
                    "token": self.token,
                    "value": value,
                }),
                None => self.emit("wait"),
            };
        }
        if dialog_ready(input) {
            // Owned open chat: no walk and no Talk-to goes out behind it, the
            // same way the landed dialog sequencer refuses to press a second
            // option. The token lives and the next call re-reads the page.
            return self.emit("wait");
        }
        let Some(page) = input.get("npcs").and_then(Value::as_array) else {
            // No posted npc page this call: no spawn can be observed, so there
            // is nothing to Talk-to.
            return self.emit("wait");
        };
        match &talk.spawn {
            Some(spawn) => {
                let tile = Tile {
                    x: spawn.x,
                    z: spawn.z,
                    level: spawn.plane,
                };
                match arrival(tile, input) {
                    Arrival::Unknown => self.emit("wait"),
                    Arrival::Walking => self.walk(tile),
                    Arrival::Arrived => {
                        let npc = NpcIdentity::Talk {
                            id: talk.npc.id,
                            name: &talk.npc.name,
                        };
                        match pick_at_spawn(&npc, page, tile) {
                            Some(pick) => self.npc_verb(&pick),
                            // Arrived with no posted row of this identity on the
                            // tile: stay there and wait, never chase a wanderer and
                            // never take a second target.
                            None => self.emit("wait"),
                        }
                    }
                }
            }
            None => {
                let Some(here) = input.get("here").and_then(posted_tile) else {
                    // No posted tile: there is no arrival claim to make and no
                    // distance to measure a posted row by, so this tick waits
                    // rather than walking blind.
                    return self.emit("wait");
                };
                let npc = NpcIdentity::Talk {
                    id: talk.npc.id,
                    name: &talk.npc.name,
                };
                match pick_talk(&npc, page, here) {
                    Some(pick) if pick.distance <= i64::from(ARRIVE_RADIUS) => self.npc_verb(&pick),
                    // Posted but out of reach: walk to the row's own posted
                    // tile and re-pick from the arrival. A row that posted no
                    // tile is unmeasured, so this tick waits.
                    Some(pick) => match pick.tile {
                        Some(tile) => self.walk(tile),
                        None => self.emit("wait"),
                    },
                    None => self.emit("wait"),
                }
            }
        }
    }

    /// The landed posted-npc verb: the posted display name the host resolves,
    /// the posted action the row listed (the talk action, or the toll keeper's
    /// `Trade`) and the posted scene index the host matches. No row id, no alias
    /// and no tile ride along. One shape for the talk arms and the Shantay
    /// click, so the host reads the same identity it was posted with.
    fn npc_verb(&self, pick: &TalkPick<'_>) -> Value {
        json!({
            "kind": "npc",
            "token": self.token,
            "name": pick.name,
            "action": pick.action,
            "index": pick.index,
        })
    }

    /// `Steady` on an identified key-keeper row: the one key the keeper the
    /// selected family names drops for this clue, walked to, Attacked and
    /// Taken, one verb per call.
    ///
    /// The membership is the selected `talk_key.keys` row whose own id is this
    /// step's, read on this arm alone: a talk step belongs to the arm above and
    /// a keeper is never Talked-to. The held clue stays the riddle the landed
    /// identify returned, so this is a sibling family and never a second
    /// identify — and the key in hand is not trail completion.
    ///
    /// Inside the arm the hunt is: the key already on the posted pack page ends
    /// it with the idle `wait` the original riddle keeps; the keeper this
    /// token's `Attack` went out for is observed first, and only its kill lets
    /// the pickup run; the walk goes to the published `{x, z, plane}` tile and
    /// repeats until this call's posted `here` holds; the one `Attack` is only
    /// ever a posted npc of the keeper's packed type standing on that tile
    /// carrying the posted `Attack`; and the Take is only ever a posted ground
    /// row of the key's own id at that same tile. A page with no posted match,
    /// no posted `here`, a missing slot count or a full pack waits with the
    /// token live: nothing is invented, nothing is Dropped, no prayer is raised
    /// and no completion kind is ever emitted here.
    fn keys(
        &mut self,
        row: &TrailMembershipRow,
        input: &Value,
        selected: Option<&SelectedGameData>,
    ) -> Value {
        let Some(key) = key_step(selected, row.id) else {
            // Not a selected key-keeper membership: the desc-only riddles no
            // keeper names, the empty-params 2722 and every other identified
            // row keep the idle they had.
            return self.emit("wait");
        };
        if holds(input, key.key_id) {
            // The key this keeper drops is already on the posted pack page: the
            // hunt is over and the original riddle idles. No Attack, no gate
            // and no completion kind — and a key banked but not held is not
            // observed at all, because the bank is not a posted page.
            return self.emit("wait");
        }
        let Some(spawn) = key.spawn.as_ref() else {
            // No published spawn: there is no tile this arm may walk to, and no
            // coordinate is invented for a keeper the family covered instead.
            return self.emit("wait");
        };
        let Some((keeper_id, keeper_name)) = keeper_type(&key.keeper) else {
            // A `category` or a bare `name` keeper names no packed npc type, so
            // its row hunts nothing and idles the way it always did.
            return self.emit("wait");
        };
        let tile = Tile {
            x: spawn.x,
            z: spawn.z,
            level: spawn.plane,
        };
        let now = self.clock.now();
        if let Some(index) = self
            .keeper
            .as_ref()
            .and_then(|keeper| keeper.owned.as_ref())
            .map(|owned| owned.index)
        {
            // The keeper this token Attacked, read before anything walks: only
            // a settled read of that index ends the hunt, and the Attack is
            // never issued twice for one owned index.
            let Some(page) = input.get("npcs").and_then(Value::as_array) else {
                // No posted npc page this call: the owned keeper cannot be
                // observed at all, so this call waits.
                return self.emit("wait");
            };
            match page
                .iter()
                .find(|posted| posted_i32(posted, "index") == Some(index))
            {
                Some(posted) => {
                    // Posted: this call's observation is the last-seen the
                    // grace reads.
                    if let Some(owned) = self.keeper.as_mut().and_then(|k| k.owned.as_mut()) {
                        owned.seen_at = now;
                    }
                    if !died_owned(posted, posted_i32(input, "self_slot"), self_target(input)) {
                        // Posted and alive: the hunt waits for its kill.
                        return self.emit("wait");
                    }
                }
                None => {
                    // The owned index left the page. Inside the frozen grace
                    // that is this token's kill; outside it the keeper is gone
                    // without ever being seen at zero health, which is this
                    // hunt's `wait` and never the wizard encounter's
                    // `guardian-lost`.
                    let within = self
                        .keeper
                        .as_ref()
                        .and_then(|keeper| keeper.owned.as_ref())
                        .is_some_and(|owned| {
                            now.saturating_duration_since(owned.seen_at)
                                < Duration::from_millis(KILL_GRACE_MS)
                        });
                    if !within {
                        return self.emit("wait");
                    }
                }
            }
            if let Some(keeper) = self.keeper.as_mut() {
                keeper.owned = None;
                keeper.post_kill = true;
            }
        }
        let after_kill = self.keeper.as_ref().is_some_and(|keeper| keeper.post_kill);
        match arrival(tile, input) {
            // No posted `here`: there is no arrival claim to make and no walk
            // to measure, so this tick waits rather than walking blind.
            Arrival::Unknown => self.emit("wait"),
            Arrival::Walking => self.walk(tile),
            Arrival::Arrived if after_kill => {
                // The kill is observed: the key lies on its own spawn tile, and
                // the hunt Takes it.
                match pick_key(input, key.key_id, tile) {
                    Some(drop) => {
                        let Some(size) = input.get("inv_size").and_then(i32_of) else {
                            // No posted slot count: the pack's fullness is not
                            // invented for it.
                            return self.emit("wait");
                        };
                        if occupied(input) >= i64::from(size) {
                            // A full pack waits — this arm Drops no food to make
                            // room, unlike the casket's own Take.
                            return self.emit("wait");
                        }
                        json!({
                            "kind": "obj",
                            "token": self.token,
                            "x": drop.tile.x,
                            "z": drop.tile.z,
                            "level": drop.tile.level,
                            "name": drop.name,
                            "action": TAKE,
                        })
                    }
                    // Arrived with nothing of the key posted on the tile: wait
                    // and re-read the page next call.
                    None => self.emit("wait"),
                }
            }
            Arrival::Arrived => {
                match input
                    .get("npcs")
                    .and_then(Value::as_array)
                    .and_then(|page| pick_keeper(keeper_id, keeper_name, page, tile))
                {
                    Some((index, name)) => {
                        self.keeper = Some(Keeper {
                            owned: Some(Owned {
                                index,
                                seen_at: now,
                            }),
                            post_kill: false,
                        });
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
                    // Arrived with no posted row of this keeper's type: stay on
                    // the tile and wait, never chasing a wanderer.
                    None => self.emit("wait"),
                }
            }
        }
    }
}

/// What this call's posted `here` says about the decoded tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
/// the bounded packed 3554 clue — is not a search step: the coord-bearing
/// ones are the sibling dig classify's, and a row the dig classify leaves out
/// idles. A pin with no coord, or an off-contract one, is the same idle:
/// nothing is invented.
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
/// `trail_coord` **and** no selected `trail_loc` **and** no selected
/// `trail_guardian` **and** an `access` that is not `"constrained"`.
///
/// The sibling of `search_tile`, not a fold into it: the loc pin is the search
/// membership and this is the selected-param classify that holds without
/// copying the frozen `type`. It reads no `trail_sextant` — that param stays
/// the guarded sibling's own pin — so the membership is the twenty medium
/// sextant rows and every coord-bearing row beside them: the map rows like
/// `2713`, the vague `3510` and the hard riddle-with-coord rows. Forty rows on
/// both pins, and the `trail_casket` param is never part of the classify. The
/// Sextant/Watch/Chart trio is never required, never waited for and never
/// acquired. A guarded row stays out: its first Dig is a spawn, and that row
/// belongs to the guarded encounter rather than this arm. The packed
/// constrained 3554 clue stays out with the desc-only rows that carry no coord
/// and the paramless 2722: identified, then idle rather than an invented
/// coordinate.
fn dig_tile(row: &TrailMembershipRow) -> Option<Tile> {
    if row.access.as_deref() == Some(CONSTRAINED) {
        return None;
    }
    let mut located = false;
    let mut guarded = false;
    let mut coord = None;
    for param in &row.params {
        match param.key.as_str() {
            "trail_loc" => located = true,
            "trail_guardian" => guarded = true,
            "trail_coord" if coord.is_none() => coord = Some(param.value.as_str()),
            _ => {}
        }
    }
    if located || guarded {
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

/// The posted ground row this key Take dispatches at: the posted row whose own
/// id is the key the key-keeper row names, whose actions carry the frozen
/// `Take`, and whose own posted tile is on the published spawn's level inside
/// `ARRIVE_RADIUS` of that tile. Posted order decides, so the first such row
/// wins, and the verb keeps that row's own tile and the name posted beside it.
///
/// The collect's own ground scan is not this read and is never reused: the
/// frozen `DROP_RADIUS` of twelve and the shark id are the jailer's and the
/// casket's, this row is identified by the key's own id rather than by the tile
/// the player stands on, and nothing about the pack's food is read. A row that
/// is not the marshalled shape, that posted no name and every row on another
/// tile or off the radius are skipped rather than guessed at.
fn pick_key(input: &Value, key_id: i32, spawn: Tile) -> Option<Ground<'_>> {
    let rows = input.get("ground")?.as_array()?;
    for row in rows {
        let (Some(id), Some(tile)) = (row.get("id").and_then(i32_of), posted_tile(row)) else {
            continue;
        };
        if id != key_id || posted_i32(row, "level") != Some(spawn.level) {
            continue;
        }
        if chebyshev(tile, spawn) > i64::from(ARRIVE_RADIUS) {
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

/// The identified row's own puzzle box: the selected `{alias}_puzzlebox` item
/// the frozen `solveHeld` is handed by id, together with the display name the
/// Open resolves.
///
/// The join is the row's own alias, so a held box this row does not name is
/// never its box, and a desc-only riddle without such an item has no puzzle
/// step at all. The name is that item's selected display name (`Puzzle box`)
/// — what the host resolves by first name match, never the alias and never an
/// item id.
fn puzzle_box<'a>(
    selected: Option<&'a SelectedGameData>,
    row: &TrailMembershipRow,
) -> Option<(i32, &'a str)> {
    let item = selected?.item_by_alias(&format!("{}_puzzlebox", row.alias))?;
    Some((item.id, item.name.as_deref()?))
}

/// Whether this call's posted pack page holds `id`: the same `(id, count)`
/// page the identify reads, and only a positive count holds. The box is held
/// by its own id, never by a display name and never by a scan of the page.
fn holds(input: &Value, id: i32) -> bool {
    posted_page(input)
        .iter()
        .any(|(row_id, count)| *row_id == id && *count > 0)
}

/// This call's posted board, as the wrapper marshalled
/// `snapshot.puzzle_board` and the generation beside it. `None` when the page
/// posted no board object at all: the frozen `boardNow()` cannot read one
/// either, and no board is invented.
fn posted_board(input: &Value) -> Option<PostedBoard> {
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
fn live_board(selected: Option<&SelectedGameData>, page: &PostedBoard) -> Option<Board> {
    if page.component_id < 0 || page.size != clue_puzzle::PUZZLE_SIZE as i32 {
        return None;
    }
    clue_puzzle::read_puzzle_board(&page.rows, selected)
}

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

/// The frozen `talk_op` prefix: the landed `reach.rs` / `dialog.rs` rule reads
/// the **first** posted action whose first four characters are `talk`, ignoring
/// ASCII case, and emits that posted string — `Talk-to` when the page lists it,
/// `Talk` otherwise. The dash is not part of the rule.
const TALK_PREFIX: &str = "talk";

/// The frozen `_challenge` suffix the selected `challenge_answers` aliases
/// carry beside their parent clue's alias: `path_join` strips it onto the
/// parent talk step, and the scroll id itself is never a step.
const CHALLENGE_SUFFIX: &str = "_challenge";

/// The frozen `SPADE_NAME = 'Spade'`: the Dig verb's item identity. The host
/// resolves the first inventory row with this display name, so this is the
/// selected-verified display and never the membership alias, never an item id
/// and never a scan of the item table.
const SPADE_NAME: &str = "Spade";

/// The frozen `'dig'` arm's action: the held step's other half beside the
/// Spade display name.
const DIG: &str = "Dig";

/// The selected item alias the gate-toll short joins: the Shantay pass the
/// desert toll's `item_req` names. The id this arm compares with a posted
/// `Carry` row is that selected item's own (`1854` on both pins, never a
/// copied number), so another named short — the Al Kharid toll's coins among
/// them — is never this trip's and never shops.
const SHANTAY_PASS: &str = "shantay_pass";

/// The toll keeper's own identity, selected constants and not a one-row family:
/// the packed npc type the posted page's `id` is joined to, the display name a
/// page that posted no id falls back to, the one action the click rides, and the
/// unique jm2 spawn the arm walks to. The frozen `GATE_ITEM_SHOPS` stand
/// `(3304, 3122, 0)` is off by one in z and is never ported.
const SHANTAY_NPC_ID: i32 = 836;
const SHANTAY_NAME: &str = "Shantay";
const TRADE: &str = "Trade";
const SHANTAY_X: i32 = 3304;
const SHANTAY_Z: i32 = 3123;
const SHANTAY_LEVEL: i32 = 0;

/// The one shop op the buy click carries. The frozen `Shop.buy(name, 1)`, cut
/// to the single chunk this arm ever sends: a stack of one pass.
const BUY: &str = "buy";

/// The named wait-class a trip that did not land the short ends with. The token
/// **lives** — it is never `supplies-needed`, never `done` and never the exact
/// `'clue solved'` — and it is neither the arrived dig's missing Spade nor an
/// `abandon`: the latch has the short, the arm resumes, and the same one never
/// shops twice.
const NO_SHOP: &str = "no-shop";

/// The machine's own observation window for one shop step: how long the posted
/// Trade, the posted interface, the posted stock row or the posted pack settle
/// has to appear before the trip gives up. The same `2_000` this machine's own
/// collect window and one sent puzzle move settle by, and armed per step, never
/// once per trip — the walk to the stand may take as long as it takes. Not the
/// frozen shop's `OPEN_WAIT_MS` / `OPEN_ATTEMPTS` / `SETTLE_MS` policy: there is
/// one attempt here, and a frozen session never spends this bound.
const SHOP_WAIT_MS: u64 = 2_000;

/// The one terminal kind this machine publishes and never triggers on its own:
/// the frozen `abandon` — the clue is left in the pack. The latch below is what
/// makes it observable; the frozen production that reaches it (`MAX_STEPS`, the
/// `blockReason` retries, `STEP_ATTEMPTS`) is a later card's and is not ported.
const ABANDON: &str = "abandon";

/// The begin refusal for the row this machine left in the pack: the frozen
/// `validate` is false while the same held id remains, so no token is handed
/// out for it. A different held row — or the adapter's `retry` — clears it.
const ABANDONED: &str = "abandoned";

/// The cap's Entrana box: membership is the identified row's own selected
/// `trail_coord`, decoded the landed way, inside this square on level 0 — the
/// frozen `isEntranaClueCoord` and never a copied `CLUE_DB`. The one selected
/// member is `trail_clue_hard_riddle027` (`3579`), whose `0_44_52_2_23`
/// decodes to `(2818, 3351, 0)`.
const ENTRANA_X_MIN: i32 = 2_802;
const ENTRANA_X_MAX: i32 = 2_878;
const ENTRANA_Z_MIN: i32 = 3_329;
const ENTRANA_Z_MAX: i32 = 3_393;
const ENTRANA_LEVEL: i32 = 0;

/// The frozen `DDS_IDS` the strip unequips like any other restricted name but
/// never lists: the two hard-trail daggers come back with the hard kit, and
/// nothing here stocks a weapon.
const DDS_IDS: [i32; 2] = [1231, 1215];

/// The frozen `ENTRANA_RESTRICTED_GEAR_RE` alternatives, ported whole and in
/// the frozen order — the same class of matcher the landed `keep` predicate is,
/// over **posted display names** and never a selected item family (schema 4
/// carries none, and none is invented). The three shapes the plain list cannot
/// spell are their own reads below: the `two.handed` wildcard, the
/// `gauntlets?` optional `s` (both spellings listed, which is the same language
/// under `\b`), and the `body(?!\s+rune\b)` lookahead.
const ENTRANA_GEAR_WORDS: [&str; 57] = [
    "sword",
    "dagger",
    "scimitar",
    "longsword",
    "2h",
    "mace",
    "warhammer",
    "battleaxe",
    "axe",
    "pickaxe",
    "spear",
    "hasta",
    "halberd",
    "maul",
    "claws",
    "whip",
    "bow",
    "shortbow",
    "longbow",
    "crossbow",
    "javelin",
    "dart",
    "thrownaxe",
    "knife",
    "staff",
    "wand",
    "battlestaff",
    "cannon",
    "helmet",
    "full helm",
    "med helm",
    "coif",
    "platebody",
    "chainbody",
    "platelegs",
    "plateskirt",
    "skirt of",
    "kiteshield",
    "square shield",
    "sq shield",
    "dragon square",
    "god cape",
    "fire cape",
    "obsidian cape",
    "defender",
    "chaps",
    "vambraces",
    "gauntlet",
    "gauntlets",
    "gloves",
    "shield",
    "cape",
    "cloak",
    "snelm",
    "cowl",
    "hat",
    "hood",
];

/// The frozen `Withdraw-1` label the restore's claim rides; the bank row's own
/// posted action slots are matched by the host and never by this machine.
const WITHDRAW_ONE: &str = "Withdraw-1";

/// The caps' own name for a bank approach that did not come up — the frozen
/// `walk to the bank failed — gear stays banked, will retry` line, logged with
/// this token and never a machine kind.
const RESTORE_WALK_FAILED: &str = "restore-walk-failed";

/// The caps' own name for a name that would not go back on — the frozen
/// `could not re-equip … — will retry` line, logged with this token and never a
/// machine kind. `supplies-needed` stays the arrived dig's own wait-class.
const RESTORE_INCOMPLETE: &str = "restore-incomplete";

/// The frozen `still holding Entrana-banned gear after bank prep — will retry`
/// line: the strip's own give-up, logged while the listed names stay listed and
/// the row's own arms wait for the next attempt rather than walking with the
/// gear in hand.
const STILL_HOLDING: &str = "still holding Entrana-banned gear after bank prep — will retry";

/// The machine's own window for one Entrana bank approach or one strip: how
/// long the walk to the nearest stand, the posted booth's own open and one
/// deposit or claim have to settle before the attempt logs its named failure
/// and re-arms. Armed per attempt, never once per trail, and freeze-honored
/// like every other window here, so a frozen session never spends it.
const ENTRANA_WAIT_MS: u64 = 30_000;

/// The frozen `ENTRANA_RESTRICTED_GEAR_RE` match over one posted display name:
/// the alternatives under their own `\b…\b`, ASCII-folded, exactly as the
/// frozen regex spells them. The vectors the frozen `entranaGear.test.ts`
/// binds are the whole of it: `Dragonhide body`, `Coif` and `Dragon dagger(p)`
/// are refused, and `Amulet of glory`, `Rune arrow`, `Body rune`, `Shark`,
/// `Clue scroll`, `Spade`, `Sextant` and `Coins` are let through.
fn entrana_restricted_gear(name: &str) -> bool {
    let folded = name.to_ascii_lowercase();
    ENTRANA_GEAR_WORDS
        .iter()
        .any(|word| word_hit(&folded, word))
        || two_handed_hit(&folded)
        || body_hit(&folded)
}

/// One frozen alternative under its own `\b…\b`: the word occurs with a
/// non-word byte — or the name's own start or end — on both sides.
fn word_hit(name: &str, word: &str) -> bool {
    let bytes = name.as_bytes();
    let mut from = 0;
    while let Some(at) = name[from..].find(word) {
        let start = from + at;
        let end = start + word.len();
        if boundary_before(bytes, start) && boundary_after(bytes, end) {
            return true;
        }
        from = start + 1;
    }
    false
}

/// The frozen `two.handed` alternative: `two`, exactly one byte, `handed`, all
/// of it under the same word boundaries. `Two-handed` is the spelled form and
/// `twohanded` is not this alternative — the `.` is one byte and not zero.
fn two_handed_hit(name: &str) -> bool {
    let bytes = name.as_bytes();
    let mut from = 0;
    while let Some(at) = name[from..].find("two") {
        let start = from + at;
        let end = start + 4 + "handed".len();
        if end <= bytes.len()
            && &bytes[start + 4..end] == b"handed"
            && boundary_before(bytes, start)
            && boundary_after(bytes, end)
        {
            return true;
        }
        from = start + 1;
    }
    false
}

/// The frozen `body(?!\s+rune\b)` alternative: the word `body` under its own
/// boundaries and not followed by whitespace and then the word `rune`. `Body
/// rune` is the composed rune and is let through; `Body runes` and `Rune body`
/// are not.
fn body_hit(name: &str) -> bool {
    let bytes = name.as_bytes();
    let mut from = 0;
    while let Some(at) = name[from..].find("body") {
        let start = from + at;
        let end = start + "body".len();
        if boundary_before(bytes, start)
            && boundary_after(bytes, end)
            && !rune_word_after(name, end)
        {
            return true;
        }
        from = start + 1;
    }
    false
}

/// The `(?!\s+rune\b)` lookahead over the rest of the name: whitespace, then
/// the word `rune`. A rest with no whitespace at all, or with anything but
/// `rune` after it, is not this lookahead.
fn rune_word_after(name: &str, at: usize) -> bool {
    let rest = &name[at..];
    let trimmed = rest.trim_start();
    if trimmed.len() == rest.len() {
        return false;
    }
    let Some(tail) = trimmed.strip_prefix("rune") else {
        return false;
    };
    !tail.starts_with(|ch: char| ch.is_ascii_alphanumeric() || ch == '_')
}

/// The frozen `\b`'s own `\w`: ASCII alphanumerics and the underscore. A byte
/// that is none of those is a boundary, a non-ASCII byte included — the frozen
/// regex is a JavaScript one without the `u` flag.
fn word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn boundary_before(bytes: &[u8], at: usize) -> bool {
    at == 0 || !word_byte(bytes[at - 1])
}

fn boundary_after(bytes: &[u8], at: usize) -> bool {
    at >= bytes.len() || !word_byte(bytes[at])
}

/// Whether the identified row arms the strip: the row's own selected
/// `trail_coord`, decoded by the landed decoder, inside the cap's box on level
/// 0 — and never a casket. The param is the whole membership: no copied
/// `CLUE_DB` coordinate, no frozen coordinate table and no second classify.
fn entrana_coord(row: &TrailMembershipRow) -> bool {
    if row.role == CASKET_ROLE {
        return false;
    }
    let coord = row
        .params
        .iter()
        .find(|param| param.key == "trail_coord")
        .map(|param| param.value.as_str());
    let Some(tile) = coord.and_then(decode_trail_coord) else {
        return false;
    };
    tile.level == ENTRANA_LEVEL
        && (ENTRANA_X_MIN..=ENTRANA_X_MAX).contains(&tile.x)
        && (ENTRANA_Z_MIN..=ENTRANA_Z_MAX).contains(&tile.z)
}

/// One posted worn row as this machine reads it: the display name the frozen
/// matcher folds and the packed id the DDS exclusion joins. A row the page
/// posted without a name is not a worn row here — the matcher is names, and no
/// id is ever folded into one.
struct Worn<'a> {
    name: &'a str,
    id: Option<i32>,
}

/// The first posted worn row this call's page carries whose name the frozen
/// matcher folds, in posted order. The page is the wrapper's marshalling of
/// `host().snapshot.equipment`, the raw rows with their own `slot`, and never
/// `Equipment.items()` — which drops the slot — and never a scan of the item
/// table.
fn pick_worn(input: &Value) -> Option<Worn<'_>> {
    input
        .get("equipment")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|row| {
            let name = row.get("name").and_then(Value::as_str)?;
            entrana_restricted_gear(name).then_some(Worn {
                name,
                id: posted_i32(row, "id"),
            })
        })
}

/// The first posted pack row whose display name the frozen matcher folds: the
/// strip's deposit candidate, listed or not. The row is the frozen
/// `depositAllMatching`'s own read of the pack — a positive count and the
/// display name — and nothing about which pass put it there is consulted, so
/// an unequipped helm, a dagger id that is never listed and a restricted item
/// the player was carrying are all the same candidate.
fn pick_pack_restricted(input: &Value) -> Option<String> {
    input
        .get("inv")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|row| {
            let name = row.get("name").and_then(Value::as_str)?;
            let count = posted_i32(row, "count")?;
            (count > 0 && entrana_restricted_gear(name)).then(|| name.to_string())
        })
}

/// The frozen make-room deposit's own row pick over this call's posted pack
/// page: the first row with a posted non-zero count, a posted name and a posted
/// id the selected trail facts do not name, that is not one of the restore's
/// own listed names and not a row this attempt has already tried.
///
/// The identity the frozen predicate protects is `CLUE_DB[id]` and
/// `CASKET_IDS[id]`: the selected `trails` facts are this revision's own answer
/// to both — the membership rows are the clue scrolls, the caskets and the
/// probes, and the `challenge_answers` rows are the challenge scrolls — so a
/// clue the trail still owns is never banked here. A row the page posted no id
/// for is not a row this deposit can identify, and it is skipped rather than
/// banked blind. No selected data at all is the same refusal: this machine
/// makes no room it cannot check.
fn make_room_row(
    selected: Option<&SelectedGameData>,
    input: &Value,
    listed: &[String],
    tried: &[String],
) -> Option<String> {
    let facts = selected?.trails()?;
    input
        .get("inv")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|row| {
            let name = row.get("name").and_then(Value::as_str)?;
            let id = posted_i32(row, "id")?;
            if posted_i32(row, "count")? <= 0 {
                return None;
            }
            if listed
                .iter()
                .any(|listed| listed.eq_ignore_ascii_case(name))
            {
                return None;
            }
            if tried.iter().any(|tried| tried.eq_ignore_ascii_case(name)) {
                return None;
            }
            if facts.rows.iter().any(|row| row.id == id) {
                return None;
            }
            if facts.challenge_answers.iter().any(|row| row.id == id) {
                return None;
            }
            Some(name.to_string())
        })
}

/// Whether this call's posted pack page holds a row with this display name and
/// a positive count: the observation the strip's deposit and the restore's
/// claim both read. A page that posted no such row holds nothing.
fn pack_holds_name(input: &Value, name: &str) -> bool {
    input
        .get("inv")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            rows.iter().any(|row| {
                row.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|posted| posted.eq_ignore_ascii_case(name))
                    && posted_i32(row, "count").is_some_and(|count| count > 0)
            })
        })
}

/// Whether this call's posted worn page holds a row with this display name: the
/// restore's own completion read, one listed name at a time.
fn worn_name(input: &Value, name: &str) -> bool {
    input
        .get("equipment")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            rows.iter().any(|row| {
                row.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|posted| posted.eq_ignore_ascii_case(name))
            })
        })
}

/// The posted nearest Use-quickly booth: the identity the strip's and the
/// restore's own `open-booth` click rides, exactly as the wrapper marshalled
/// it. A page that posted no booth has nothing to open, and no tile is invented
/// for one.
struct Booth<'a> {
    x: i32,
    z: i32,
    level: i32,
    id: i32,
    name: Option<&'a str>,
    action: Option<&'a str>,
}

fn nearest_booth(input: &Value) -> Option<Booth<'_>> {
    let row = input.get("nearest_booth")?;
    Some(Booth {
        x: posted_i32(row, "x")?,
        z: posted_i32(row, "z")?,
        level: posted_i32(row, "level")?,
        id: posted_i32(row, "id")?,
        name: row.get("name").and_then(Value::as_str),
        action: row.get("op").and_then(Value::as_str),
    })
}

/// Whether this call's posted `here` stands on the posted booth's own level
/// inside the frozen `ARRIVE_RADIUS`: the same arrival rule every walk arm here
/// reads, applied to the booth page rather than to a decoded tile.
fn booth_arrival(booth: &Booth<'_>, input: &Value) -> Arrival {
    let Some(here) = input.get("here").and_then(posted_tile) else {
        return Arrival::Unknown;
    };
    let tile = Tile {
        x: booth.x,
        z: booth.z,
        level: booth.level,
    };
    if here.level != booth.level || chebyshev(here, tile) > i64::from(ARRIVE_RADIUS) {
        Arrival::Walking
    } else {
        Arrival::Arrived
    }
}

/// This call's posted bank interface. Only a posted `true` is an open one: an
/// omitted slot is unobserved rather than closed, so no deposit or claim is
/// ever sent on an invented open bank.
fn bank_open(input: &Value) -> bool {
    input.get("bank_open").and_then(Value::as_bool) == Some(true)
}

/// The landed `unequip` step: the host's worn-row `Remove` by resolved display
/// name, which is the only verb that can take a worn restricted row off. The
/// landed `wear` resolves inventory rows alone, so it can put a name back on
/// and can never take one off. No row id and no slot: the host resolves the
/// name it is handed.
fn unequip_verb(name: &str, token: u64) -> Value {
    json!({ "kind": "unequip", "token": token, "name": name })
}

/// The landed `wear` step: the landed equip-from-pack verb the shim's own
/// `Equipment.equip` queues, and the one the restore's wear-back rides. No row
/// id and no slot: the host resolves the name it is handed.
fn wear_verb(name: &str, token: u64) -> Value {
    json!({ "kind": "wear", "token": token, "name": name })
}

/// The landed `deposit` step, one restricted name at a time: the frozen
/// `Bank.depositAllMatching` cut to the regex-matching names this strip put in
/// the pack — and to any other regex-matching row the pack holds — and never
/// the ordinary loot deposit.
fn deposit_verb(name: &str, token: u64) -> Value {
    json!({ "kind": "deposit", "token": token, "name": name })
}

/// The landed `withdraw` step for one missing listed name: the frozen
/// `Bank.withdraw(name, 'Withdraw-1')`, one chunk of one.
fn withdraw_verb(name: &str, token: u64) -> Value {
    json!({ "kind": "withdraw", "token": token, "name": name, "action": WITHDRAW_ONE })
}

/// The landed `walk-nearest-bank` step: Rust picks the stand from the packed
/// world, so this machine sends no tile of its own and never a frozen one.
fn walk_nearest_bank_verb(token: u64) -> Value {
    json!({ "kind": "walk-nearest-bank", "token": token })
}

/// The landed `close` step: the open bank interface's own close, once the
/// strip's deposit or the restore's claim is done with it.
fn close_verb(token: u64) -> Value {
    json!({ "kind": "close", "token": token })
}

/// The landed `open-booth` step over the posted booth: its own tile, id and —
/// when the page posted them — name and action, the way the landed bank
/// helpers enqueue it. Nothing about it is invented and no stand is picked
/// here.
fn open_booth_verb(booth: &Booth<'_>, token: u64) -> Value {
    let mut step = json!({
        "kind": "open-booth",
        "token": token,
        "x": booth.x,
        "z": booth.z,
        "level": booth.level,
        "id": booth.id,
    });
    if let Some(name) = booth.name {
        step["name"] = json!(name);
    }
    if let Some(action) = booth.action {
        step["action"] = json!(action);
    }
    step
}

/// The frozen `could not re-equip … — will retry` line, named the way the caps
/// do (`restore-incomplete`): a log through `callback.log`, never a machine
/// kind, and the names it carries stay listed.
fn restore_incomplete(names: &str) -> String {
    format!("{RESTORE_INCOMPLETE}: could not re-equip {names} — will retry")
}

/// The frozen `walk to the bank failed — gear stays banked, will retry` line,
/// named the way the caps do (`restore-walk-failed`): a log through
/// `callback.log`, never a machine kind.
fn restore_walk_failed() -> String {
    format!("{RESTORE_WALK_FAILED}: the bank did not come up — will retry")
}

/// One live Entrana bank approach, shared by the strip's deposit and the
/// restore's claim: whether this attempt's `walk-nearest-bank` has gone out.
/// Session state on the live token, like `shop` — never a second scheduler and
/// never a nested bank machine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct BankApproach {
    /// This attempt's walk has been sent: the calls after it read the posted
    /// booth page instead of walking again.
    walked: bool,
    /// The interface has been observed open on this attempt: the strip's exit
    /// and the restore's wear pass both owe it a close before their own verb,
    /// so nothing is worn back on behind an open bank.
    opened: bool,
}

/// The strip's own live state on one step: the bank approach's own half and
/// whether the strip is settled for this step. Dropped with the step like
/// `shop`; the listed names it writes are not.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Strip {
    bank: BankApproach,
    /// The unequip and deposit passes are done for this step: the row's own
    /// arms run and this arm never re-enters for it.
    settled: bool,
}

/// The restore's own live state on the collect's exit: the bank approach and
/// the names whose claim has gone out on this attempt. Dropped when the
/// attempt ends, never when the list is still full — the list is the latch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Restore {
    bank: BankApproach,
    /// The name whose own `withdraw` went out on the previous call and has not
    /// been observed to land: one claim in flight at a time.
    sent: Option<String>,
    /// The names this attempt has already claimed: a name whose claim never
    /// landed is not claimed twice inside one attempt, and the next attempt
    /// starts with an empty list of them.
    tried: Vec<String>,
    /// The pack row whose own make-room `deposit` went out on the previous call
    /// and has not been observed to land.
    room_sent: Option<String>,
    /// The rows this attempt has already tried to bank to make room: a deposit
    /// the page never showed landing is not sent twice inside one attempt, so a
    /// bank that refuses them ends the attempt instead of repeating itself.
    room_tried: Vec<String>,
}

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

/// Whether the identified row needs the coordinate trio: a selected
/// `trail_sextant=yes` param on the row itself.
///
/// The param is the whole of the membership and it is never widened: the
/// unguarded map, vague and riddle rows that do not carry it never enter this
/// arm, and no requirement is ever grown for them. The packed constrained 3554
/// clue carries it too and is refused above, before `Steady` is ever reached.
fn needs_trio(row: &TrailMembershipRow) -> bool {
    row.params
        .iter()
        .any(|param| param.key == TRAIL_SEXTANT && param.value == SEXTANT_YES)
}

/// The three selected givers of the published `trio_givers` family, each
/// resolved by its own alias and only when that row publishes the unique jm2
/// spawn this arm walks to.
///
/// A family that is absent, and one that does not publish all three identities
/// with a spawn, is not a giver set: the intercept does not fire and the row
/// keeps the landed guarded or unguarded-dig arm it has, rather than walking to
/// an invented tile or playing half a chain.
struct TrioGivers<'a> {
    professor: &'a TrioGiverRow,
    murphy: &'a TrioGiverRow,
    kojo: &'a TrioGiverRow,
}

fn trio_givers(selected: &SelectedGameData) -> Option<TrioGivers<'_>> {
    let rows = &selected.trio_givers()?.rows;
    let spawned = |alias: &str| {
        rows.iter()
            .find(|row| row.alias == alias && row.spawn.is_some())
    };
    Some(TrioGivers {
        professor: spawned(OBSERVATORY_PROFESSOR)?,
        murphy: spawned(MURPHY)?,
        kojo: spawned(BROTHER_KOJO)?,
    })
}

/// This row's own trio-acquire entry: the published givers and the first tool
/// this call's posted pack is short of, or `None` when this call is not the
/// intercept's — no selected pin at all, no three unique-spawn givers, or a
/// pack that already holds the whole trio.
///
/// `None` is the fall-through: the landed guarded and unguarded-dig arms run
/// exactly as they did before this arm existed.
fn trio_plan<'a>(
    selected: Option<&'a SelectedGameData>,
    input: &Value,
) -> Option<(TrioGivers<'a>, Tool)> {
    let selected = selected?;
    Some((trio_givers(selected)?, first_missing_tool(selected, input)?))
}

/// The first tool of the frozen order this call's posted pack does not hold: a
/// selected alias joins that item's own selected id — `2574` / `2575` / `2576`
/// on both pins, never a copied number — a missing item row is not held, and
/// only a posted positive count holds. `None` is the posted `hasAllTrio`, the
/// intercept's own completion.
///
/// The page is the already-marshalled `inv` sequence the collect arm reads out
/// of the same posted `snapshot.inv`, read at call time. A page that did not
/// post it holds nothing, so the tool reads as missing and the arm walks rather
/// than Digging without it. The display names are corroboration and never a
/// second identity: the join is the id.
fn first_missing_tool(selected: &SelectedGameData, input: &Value) -> Option<Tool> {
    for (tool, alias) in [Tool::Sextant, Tool::Watch, Tool::Chart]
        .into_iter()
        .zip(TRIO_ITEMS)
    {
        let held = selected
            .item_by_alias(alias)
            .is_some_and(|item| pack_holds(input, item.id));
        if !held {
            return Some(tool);
        }
    }
    None
}

/// Whether this call's posted pack page holds `id`: a posted row of the
/// marshalled `inv` page whose own id is this one and whose count is positive.
/// A row that posted no count, a zero count and a page that posted no such id
/// all hold nothing, and the held order is never read — the trio is a set of
/// three identities, not a position in the pack.
fn pack_holds(input: &Value, id: i32) -> bool {
    input
        .get("inv")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            rows.iter().any(|row| {
                row.get("id").and_then(i32_of) == Some(id)
                    && row
                        .get("count")
                        .and_then(i32_of)
                        .is_some_and(|count| count > 0)
            })
        })
}

/// The selected toll keeper's own spawn: the unique jm2 tile the Shantay arm
/// walks to. Selected constants and never the frozen `GATE_ITEM_SHOPS` stand —
/// `(3304, 3122, 0)` is off by one in z, and no frozen tile is ported here.
fn shantay_spawn() -> Tile {
    Tile {
        x: SHANTAY_X,
        z: SHANTAY_Z,
        level: SHANTAY_LEVEL,
    }
}

/// Whether this call's page names the short: a posted `walk_missing_carry` row
/// whose own id is the selected item's and whose count is positive. Only that
/// vector nominates a shop — the walk outcome's fail bit and its dest geometry
/// are never read as a shopping list, and a page that posted no vector names
/// nothing.
///
/// The row's posted display name is corroboration and never a second identity:
/// the join is the id, exactly the way the pack's own trio read joins one.
fn carry_names(input: &Value, id: i32) -> bool {
    let Some(rows) = input.get("walk_missing_carry").and_then(Value::as_array) else {
        return false;
    };
    rows.iter().any(|row| {
        posted_i32(row, "id") == Some(id)
            && posted_i32(row, "count").is_some_and(|count| count >= 1)
    })
}

/// This call's posted `shop_open`, and only a posted `true`: an omitted slot is
/// unobserved — neither a closed interface nor an open one — and a posted
/// `false` is a closed one.
fn shop_open(input: &Value) -> bool {
    input.get("shop_open").and_then(Value::as_bool) == Some(true)
}

/// One posted stock row the buy click can ride: the short's own id, the posted
/// display name the host resolves, and the posted slot and component its
/// presence check matches. No row id and no tile are invented for one.
struct BuyRow<'a> {
    name: &'a str,
    id: i32,
    slot: i32,
    component: i32,
}

/// This call's posted stock row for the short, on the open interface.
///
/// The join is the row's own id, and the three fields the host re-resolves it by
/// — name, slot, component — must all be posted: a row the page posted without
/// them is not clickable here and is skipped rather than guessed at, and a page
/// with no such row buys nothing. The count is not a second gate — the click is
/// the frozen `Shop.buy(name, 1)`, one chunk of one.
fn buy_row<'a>(input: &'a Value, id: i32) -> Option<BuyRow<'a>> {
    let rows = input.get("shop_stock").and_then(Value::as_array)?;
    rows.iter().find_map(|row| {
        if posted_i32(row, "id") != Some(id) {
            return None;
        }
        let name = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())?;
        Some(BuyRow {
            name,
            id,
            slot: posted_i32(row, "slot")?,
            component: posted_i32(row, "component")?,
        })
    })
}

/// The landed shop click the buy rides: the posted stock row's own identity and
/// one chunk of one, as `InteractReq::ShopButton` reads it. Its own envelope kind
/// — the adapters enqueue it as the landed `shop-button` op, never as a
/// `kind: "ops"` shopping list, never as a loc and never as a `V2_OPS` verb.
fn buy_verb(row: &BuyRow<'_>, token: u64) -> Value {
    json!({
        "kind": "shop-button",
        "token": token,
        "shop": BUY,
        "name": row.name,
        "id": row.id,
        "slot": row.slot,
        "component": row.component,
        "chunk": 1,
    })
}

/// The live gate-toll shop trip on the identified clue row: the short the
/// intercepted walk was missing, the dest that walk was going to, and how far
/// the trip has got. Session state on the live token, like `open` and
/// `guardian` — never a second scheduler, never a `Phase::Shopping`, never a
/// nested `shop.rs` token and never a cached page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shop {
    /// The selected item id this trip is for: the Shantay pass' own id, joined
    /// from the alias. The posted stock row and the posted pack settle are both
    /// read by this id, never by a copied number.
    id: i32,
    /// The intercepted walk's own dest. Remembered for two reads: the trip never
    /// walks anywhere but the selected spawn while it runs, and its exit walks
    /// this dest back — the second walk — so the row's own arm resumes where the
    /// failure left off even on a page that posts no `here`.
    dest: Tile,
    step: ShopStep,
    /// The short did not land: the trip's exit owes the named `no-shop`. Set
    /// when a step's window ends without its own observation, and cleared once
    /// that kind has gone out.
    failed: bool,
    /// The one interface close went out. The close is dispatched once per trip
    /// and never again, however long the posted interface takes to go.
    closed: bool,
}

/// How far one shop trip has got. The steps are the frozen `ensureGateItems`
/// order — walk to the stand, Trade, buy one, close — and every one of them is
/// observed on the posted page before the next goes out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShopStep {
    /// Walking to the selected Shantay spawn.
    Stand,
    /// At the spawn: picked the keeper and sent the `Trade`.
    Trade,
    /// The posted interface is up: sent the buy click for one chunk of the
    /// short's own posted stock row.
    Buy,
    /// The trip is over — the short landed, or a step's window ended without
    /// its own observation. Its exit is the interface close, the named
    /// `no-shop` for a short that did not land, and the walk back to the
    /// original dest, one per call, and never another wait on a spent window.
    Exit,
}

/// One stop of a tool's chain: the published giver row this stop walks to, and
/// whether that giver is the professor — the one giver whose chat this arm
/// answers options in.
struct Stop<'a> {
    row: &'a TrioGiverRow,
    professor: bool,
}

/// The stop a tool's own index works: the sextant is the professor's teach stop
/// and then Murphy's own, the watch is Kojo's alone and the chart is the
/// professor's again. Any index past a tool's own list is its last stop, so an
/// item that never lands keeps that giver working rather than inventing a
/// fourth one.
fn stop_of<'a>(givers: &TrioGivers<'a>, tool: Tool, stop: usize) -> Stop<'a> {
    match tool {
        Tool::Sextant if stop == 0 => Stop {
            row: givers.professor,
            professor: true,
        },
        Tool::Sextant => Stop {
            row: givers.murphy,
            professor: false,
        },
        Tool::Watch => Stop {
            row: givers.kojo,
            professor: false,
        },
        Tool::Chart => Stop {
            row: givers.professor,
            professor: true,
        },
    }
}

/// That tool's own last stop index: the sextant's chain is two givers long, and
/// the watch's and the chart's are one each.
fn last_stop(tool: Tool) -> usize {
    match tool {
        Tool::Sextant => 1,
        Tool::Watch | Tool::Chart => 0,
    }
}

/// This call's posted professor option: the first row — in posted order — whose
/// text ASCII-folds to one of the two selected closed-handler literals,
/// carrying that row's own posted 1-based slot.
///
/// A row whose text matches but whose slot is not a posted integer is skipped
/// rather than answered with an invented one, and a page that posted no such
/// row answers nothing at all.
fn professor_option(input: &Value) -> Option<i32> {
    input
        .get("chat_options")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|row| {
            let text = row.get("text").and_then(Value::as_str)?;
            if !PROFESSOR_OPTIONS
                .iter()
                .any(|known| text.eq_ignore_ascii_case(known))
            {
                return None;
            }
            row.get("option").and_then(i32_of)
        })
}

/// Whether this call's page posted an option list with at least one row: what a
/// chat this arm may not answer waits behind. An unmatched list is never a
/// reason to send the last option, and an omitted list is not an empty one.
fn options_posted(input: &Value) -> bool {
    input
        .get("chat_options")
        .and_then(Value::as_array)
        .is_some_and(|rows| !rows.is_empty())
}

/// The selected talk step an identified membership row owns: the
/// `talk_key.talk` row whose own id is the row's, or `None` when the row is not
/// a talk membership. The key keepers, the puzzle-box extras and every other
/// identified row are not this arm's, and nothing is ever played as if they
/// were.
fn talk_step(selected: Option<&SelectedGameData>, id: i32) -> Option<&TalkKeyTalkRow> {
    selected?.talk_key()?.talk.iter().find(|talk| talk.id == id)
}

/// The selected key-keeper step an identified membership row owns: the
/// `talk_key.keys` row whose own id is the row's, or `None` when the row is not
/// a key-hunt membership.
///
/// A sibling of `talk_step` and never a fold into it: the two families publish
/// different ids, so a talk step is never hunted and a keeper is never
/// Talked-to. The held clue stays the riddle the landed identify returned — the
/// key is not membership — and the row's own `key_id`, `keeper` and `spawn` are
/// what the arm reads.
fn key_step(selected: Option<&SelectedGameData>, id: i32) -> Option<&TalkKeyKeyRow> {
    selected?.talk_key()?.keys.iter().find(|key| key.id == id)
}

/// The packed npc type one key-keeper row names: that keeper's own id and the
/// display name the posted npc page carries beside it, and nothing else.
///
/// Only a `type` matcher is one npc. A `category` or a bare `name` keeper
/// matches many and carries no packed id, so those rows have no type here and
/// stay idle rather than hunting the nearest anything. The matcher's own script
/// alias is never part of this identity — the posted page carries no alias at
/// all — and it is never substituted for the display name the verb carries.
fn keeper_type(keeper: &TalkKeyKeeper) -> Option<(i32, &str)> {
    let id = keeper.id?;
    let name = keeper.name.as_deref().filter(|name| !name.is_empty())?;
    (keeper.kind == KEEPER_TYPE).then_some((id, name))
}

/// The challenge seam: `identify_step` reads no challenge id, so the page the
/// server leaves once a parent clue swaps for its own challenge scroll is
/// `none-held`. A selected `challenge_answers` id with a positive posted count
/// joins its **parent** talk membership row through the `_challenge` strip, and
/// the parent — never the scroll id — is the step the token keeps.
///
/// The first posted selected id wins, so two scrolls joined by one page resolve
/// in posted order. Nothing else is admitted: an empty page, a zero count, an
/// id no selected challenge answers, a strip that names no selected row, and a
/// parent the talk family does not publish are all `none-held`.
fn challenge_step<'a>(
    selected: Option<&'a SelectedGameData>,
    input: &Value,
) -> Option<&'a TrailMembershipRow> {
    let facts = selected?.trails()?;
    for (id, count) in posted_page(input) {
        if count <= 0 {
            continue;
        }
        let Some(challenge) = facts
            .challenge_answers
            .iter()
            .find(|challenge| challenge.id == id)
        else {
            continue;
        };
        let Some(alias) = challenge.alias.strip_suffix(CHALLENGE_SUFFIX) else {
            continue;
        };
        let Some(parent) = facts.rows.iter().find(|row| row.alias == alias) else {
            continue;
        };
        if talk_step(selected, parent.id).is_none() {
            continue;
        }
        return Some(parent);
    }
    None
}

/// The selected answer this talk step's own challenge scroll carries: the
/// `challenge_answers` row whose `_challenge`-stripped alias is the step's own
/// alias, parsed as the non-negative `i32` the count dialog wants.
///
/// `None` when the step has no challenge, and `None` when the selected string
/// is not one — an unparsed, negative or absent answer never clicks, so no
/// count is ever invented.
fn challenge_answer(selected: Option<&SelectedGameData>, talk: &TalkKeyTalkRow) -> Option<i32> {
    for challenge in &selected?.trails()?.challenge_answers {
        if challenge.alias.strip_suffix(CHALLENGE_SUFFIX) != Some(talk.alias.as_str()) {
            continue;
        }
        let Ok(value) = challenge.answer.parse::<i32>() else {
            continue;
        };
        if value >= 0 {
            return Some(value);
        }
    }
    None
}

/// The landed `talk_op` over one posted row: the first posted action whose
/// first four bytes are `talk`, ignoring ASCII case, emitted as that posted
/// string. A row that posted no talk action is not a row this arm dispatches
/// at, and the action is never canonicalized — the host receives the page's own
/// `Talk-to` or `Talk`.
fn talk_action(row: &Value) -> Option<&str> {
    let actions = row.get("actions")?.as_array()?;
    actions.iter().find_map(|action| {
        action.as_str().and_then(|text| {
            text.get(..TALK_PREFIX.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(TALK_PREFIX))
                .then_some(text)
        })
    })
}

/// The identity one posted npc page row is joined to. The identity belongs to
/// the caller's own family — a landed talk step's npc, or one published
/// `trio_givers` row — and the *rule* around it (the posted scene index, the
/// posted display name the verb carries, the posted talk action) is this
/// machine's single read.
enum NpcIdentity<'a> {
    /// A landed talk step's own npc: the selected packed type id against the
    /// posted `id`, or the selected display name against the posted `name`, the
    /// way that arm has always joined them.
    Talk { id: i32, name: &'a str },
    /// A published `trio_givers` row, whose packed id is the identity: the
    /// posted display name is only the fallback a page that posted no id at all
    /// leaves. The `observatory_professor2` lookalike posts this giver's own
    /// display name under another packed id and is never this family.
    Giver(&'a TrioGiverRow),
    /// The toll keeper this token shops at: the selected packed type and
    /// display name of the Shantay spawn. Its verb is the one this arm
    /// dispatches — a posted `Trade`, never a talk op — so the action it rides
    /// is read by [`Self::action`] rather than by the landed `talk_op`.
    Shantay,
}

impl NpcIdentity<'_> {
    /// Whether this posted row's own identity is this one, matched exactly the
    /// way the variant names it.
    fn names(&self, row: &Value, posted: &str) -> bool {
        match self {
            Self::Talk { id, name } => {
                posted_i32(row, "id") == Some(*id) || posted.eq_ignore_ascii_case(name)
            }
            Self::Giver(giver) => match posted_i32(row, "id") {
                Some(id) => id == giver.id,
                None => posted.eq_ignore_ascii_case(&giver.name),
            },
            Self::Shantay => match posted_i32(row, "id") {
                Some(id) => id == SHANTAY_NPC_ID,
                None => posted.eq_ignore_ascii_case(SHANTAY_NAME),
            },
        }
    }

    /// The posted action this identity's own verb rides: the talk arms read the
    /// page's own talk action (the landed `talk_op`), and the toll keeper must
    /// list the frozen `Trade` — a posted row without it is not a keeper this
    /// arm clicks. Neither arm ever invents an action the page did not post.
    fn action<'a>(&self, row: &'a Value) -> Option<&'a str> {
        match self {
            Self::Shantay => posted_action(row, TRADE).then_some(TRADE),
            Self::Talk { .. } | Self::Giver(_) => talk_action(row),
        }
    }
}

/// One posted npc row that names the npc this caller is looking for, as the
/// pickers read it: the posted scene index the host matches, the posted display
/// name and the posted action this identity's verb rides — the landed talk
/// action for a talk step or a giver, the frozen `Trade` for the toll keeper.
///
/// The identity and its action are `NpcIdentity`'s, and the script alias is
/// never compared to a posted string: the page carries no alias at all. A row
/// that posted no index, no name, or no action this identity dispatches at is
/// not a row this arm can use.
fn named_row<'a>(row: &'a Value, identity: &NpcIdentity<'_>) -> Option<(i32, &'a str, &'a str)> {
    let index = posted_i32(row, "index")?;
    let posted = row
        .get("name")
        .and_then(Value::as_str)
        .filter(|posted| !posted.is_empty())?;
    if !identity.names(row, posted) {
        return None;
    }
    Some((index, posted, identity.action(row)?))
}

/// One posted npc row the talk arm has picked: the posted scene index the host
/// matches, the posted name and posted talk action the verb carries, the posted
/// tile a walk would go to, and the measure the pick was ranked by.
struct TalkPick<'a> {
    index: i32,
    name: &'a str,
    action: &'a str,
    tile: Option<Tile>,
    distance: i64,
}

/// The identity-only picker: the nearest posted row that names this step's npc
/// and lists a talk action, measured the landed way — the posted `distance`
/// when the row carried one, else the Chebyshev distance from this call's
/// posted `here` to the row's own posted tile, on that same level. The scan
/// only replaces its best on a strict improvement, so ties keep posted order.
///
/// The five steps whose jm2 spawn is not unique take this read, and it is the
/// posted page's own answer: no alias, no first-in-file row and no frozen
/// coordinate is ever picked, and a page with no match picks nothing.
fn pick_talk<'a>(
    identity: &NpcIdentity<'_>,
    page: &'a [Value],
    here: Tile,
) -> Option<TalkPick<'a>> {
    let mut best: Option<TalkPick<'a>> = None;
    for row in page {
        let Some((index, posted, action)) = named_row(row, identity) else {
            continue;
        };
        let Some(distance) = npc_distance(row, Some(here)) else {
            continue;
        };
        let better = match &best {
            None => true,
            Some(best) => distance < best.distance,
        };
        if better {
            best = Some(TalkPick {
                index,
                name: posted,
                action,
                tile: posted_tile(row),
                distance,
            });
        }
    }
    best
}

/// The unique-spawn picker: the nearest posted row that names this npc, lists a
/// talk action, stands on the published spawn's own level, and is inside the
/// frozen `ARRIVE_RADIUS` of that spawn — by the row's own posted tile, or by
/// the posted `distance` the page carries to this player.
///
/// The radius is part of the membership and not only of the verb: a wanderer
/// outside it is not this step's npc, so the arm keeps the published tile and
/// waits instead of chasing a second target. Strict improvement only, so ties
/// keep posted order.
///
/// The talk arm's own unique-spawn steps and the trio acquire chain's givers are
/// both this read, each over its own family's `(id, name)`; neither ever
/// substitutes an alias for the posted name it dispatches.
fn pick_at_spawn<'a>(
    identity: &NpcIdentity<'_>,
    page: &'a [Value],
    spawn: Tile,
) -> Option<TalkPick<'a>> {
    let mut best: Option<TalkPick<'a>> = None;
    for row in page {
        let Some((index, posted, action)) = named_row(row, identity) else {
            continue;
        };
        if posted_i32(row, "level") != Some(spawn.level) {
            continue;
        }
        let tile = posted_tile(row);
        let distance = posted_i32(row, "distance").map(i64::from);
        let near = distance.is_some_and(|distance| distance <= i64::from(ARRIVE_RADIUS))
            || tile.is_some_and(|tile| chebyshev(tile, spawn) <= i64::from(ARRIVE_RADIUS));
        if !near {
            continue;
        }
        let Some(distance) = distance.or_else(|| tile.map(|tile| chebyshev(tile, spawn))) else {
            continue;
        };
        let better = match &best {
            None => true,
            Some(best) => distance < best.distance,
        };
        if better {
            best = Some(TalkPick {
                index,
                name: posted,
                action,
                tile,
                distance,
            });
        }
    }
    best
}

/// The keeper pick over this call's posted npc page: the posted row whose own
/// packed id — or, failing that, whose posted display name — is the keeper this
/// key row names, that lists the posted `Attack`, and that stands on the
/// published spawn's own level inside the frozen `ARRIVE_RADIUS` of that tile.
///
/// The sibling of `pick_at_spawn`, and never a reuse of it: the identity here is
/// the keeper's packed type and the action is `Attack`, where the talk arm's own
/// pick reads a `talk_op` over its family's npc. The radius is part of the
/// membership and not only of the verb, so a keeper further off is not this
/// tile's and the hunt waits at the published tile instead of chasing it.
/// Nearest wins and ties keep posted order — the scan only replaces its best on
/// a strict improvement. A row with no posted index, no posted name, no
/// `Attack`, another level, no distance and no marshalled tile matches nothing.
fn pick_keeper<'a>(id: i32, name: &str, page: &'a [Value], spawn: Tile) -> Option<(i32, &'a str)> {
    let mut best: Option<(i32, &'a str, i64)> = None;
    for row in page {
        let Some(index) = posted_i32(row, "index") else {
            continue;
        };
        let Some(posted) = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        let named = posted_i32(row, "id") == Some(id) || posted.eq_ignore_ascii_case(name);
        if !named || !posted_action(row, ATTACK) {
            continue;
        }
        if posted_i32(row, "level") != Some(spawn.level) {
            continue;
        }
        let tile = posted_tile(row);
        let distance = posted_i32(row, "distance").map(i64::from);
        let near = distance.is_some_and(|distance| distance <= i64::from(ARRIVE_RADIUS))
            || tile.is_some_and(|tile| chebyshev(tile, spawn) <= i64::from(ARRIVE_RADIUS));
        if !near {
            continue;
        }
        let Some(distance) = distance.or_else(|| tile.map(|tile| chebyshev(tile, spawn))) else {
            continue;
        };
        let better = match &best {
            None => true,
            Some((_, _, best)) => distance < *best,
        };
        if better {
            best = Some((index, posted, distance));
        }
    }
    best.map(|(index, posted, _)| (index, posted))
}

/// The landed `dialog_ready` over this call's posted chat slots: a posted
/// `chat_modal_id` that is not the closed `-1`, or a posted `chat_continue`.
///
/// Only posted facts count. A page that posted neither slot has not said the
/// chat is open, so an unobserved slot is neither an open chat nor a close, and
/// the arm is free to Talk-to.
fn dialog_ready(input: &Value) -> bool {
    posted_i32(input, "chat_modal_id").is_some_and(|id| id != -1)
        || input.get("chat_continue").and_then(Value::as_bool) == Some(true)
}

/// The posted `count_dialog_open` slot. Only a posted `true` blocks a Talk-to
/// and only a posted `true` is a dialog to answer: an omitted slot is
/// unobserved, not a close, and a posted `false` is a closed dialog.
fn count_open(input: &Value) -> bool {
    input.get("count_dialog_open").and_then(Value::as_bool) == Some(true)
}

/// The posted `chat_continue` slot: only a posted `true` is the frozen continue
/// step this arm may send. An omitted slot is unobserved and a posted `false` is
/// not a continue.
fn continue_posted(input: &Value) -> bool {
    input.get("chat_continue").and_then(Value::as_bool) == Some(true)
}

/// Progress line for the identified step: landed alias, role and id only.
/// Never an invented coord, npc or answer, and never `'clue solved'`.
fn progress(row: &TrailMembershipRow) -> String {
    format!("clue step held: {} {} [{}]", row.role, row.alias, row.id)
}

/// Status line for the identified step: the landed alias only, the progress
/// string the `Reporting` call posts. The exact `'clue solved'` status is the
/// finished collect's own, posted by `finish`, and never a progress line.
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

/// The connection boundary — a reconnect, a relog, `reset_session_work` — and
/// the token abort only: the live step and its token are dropped and nothing
/// else is. The strip list and the abandon latch are the session's own and
/// outlive it — the frozen `strippedGear` is what the reclaim owes after the
/// gear is already banked, and losing it here would leave the armour in the
/// bank with `ownsEquipment()` false, while the frozen `validate` keeps its
/// leave-in-pack latch across a relog too. Operator Stop is
/// [`on_stop`], not this.
pub fn on_reset() {
    RUNTIME.with(|rt| rt.borrow_mut().abort());
}

/// Operator Stop, and the fresh task instance a later Start builds: the whole
/// session starts over, so the strip list and the abandon latch go with the
/// step. A Load isolate reaches this by dying with its own thread — its
/// `RUNTIME` is thread-local — and the compiled slot reaches it through the
/// pump's owed Stop.
pub fn on_stop() {
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.stripped.clear();
        rt.abandoned = None;
        rt.abort();
    });
}

/// The machine's only entry point: the selected pin comes from the native
/// registration's captured `game_data`, and the payload is the wrapper's
/// marshalled call — never a host wire.
pub fn dispatch(selected: Option<&SelectedGameData>, input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => RUNTIME.with(|rt| rt.borrow_mut().begin(selected, input)),
        "next" => RUNTIME.with(|rt| rt.borrow_mut().next(selected, input)),
        // The machine-state seats. None is a step over a page and none takes a
        // token: `ownsEquipment` is a read of the stripped list the shim
        // publishes as `SolveClue.ownsEquipment`, `retry` is the frozen latch
        // clear the shim's and the v2 seat both call, and `abandon` is the
        // latch's own write — the machine's terminal an emitting card would
        // reach, never a verb an adapter sends. None of them is a `request()`
        // op.
        "ownsEquipment" => RUNTIME.with(|rt| rt.borrow().owns_equipment()),
        "retry" => RUNTIME.with(|rt| rt.borrow_mut().retry()),
        "abandon" => RUNTIME.with(|rt| rt.borrow_mut().abandoned()),
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
    /// `trail_clue_easy_map001`: the coord-only easy map the widened dig
    /// membership reads, `trail_coord=0_49_52_41_32` → (3177, 3360, 0) with no
    /// `trail_sextant` at all.
    const MAP: i32 = 2713;
    /// `trail_clue_hard_map001`: a selected clue row with no params at all.
    const MAP_EMPTY: i32 = 2722;
    /// `trail_clue_medium_riddle001`: the desc-only frozen `keyFrom` riddle the
    /// selected `talk_key.keys` family publishes as Black Heather's own step —
    /// key `2832`, packed type `202` at the published `(3039, 3700, plane 0)`.
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
    /// `trail_clue_hard_riddle014`: a desc-only hard riddle whose own selected
    /// `trail_clue_hard_riddle014_puzzlebox` item is the box its puzzle step
    /// joins to.
    const PUZZLE_RIDDLE: i32 = 2794;
    /// That box item, display `Puzzle box`.
    const PUZZLE_BOX: i32 = 2795;
    /// `trail_clue_hard_riddle022`: a desc-only hard riddle with **no**
    /// `_puzzlebox` sibling at all, which stays idle.
    const PUZZLE_RIDDLE_NO_BOX: i32 = 3572;
    /// `2831`'s own key: the ground row its keeper drops on the published
    /// spawn, and the pack row that ends the hunt.
    const KEEPER_KEY: i32 = 2832;
    /// The packed type that riddle's keeper names, and the posted display name
    /// the page carries beside it. The matcher's script alias (`black_heather`)
    /// is never compared to a posted string.
    const KEEPER_ID: i32 = 202;
    const KEEPER_NAME: &str = "Black Heather";
    /// `trail_clue_medium_riddle008`: the second unique-spawn type keeper —
    /// Penda, packed id 1087, at the published `(2910, 3539, plane 0)` — and
    /// the key `3608` it drops.
    const PENDA: i32 = 3607;
    const PENDA_KEY: i32 = 3608;
    const PENDA_ID: i32 = 1087;
    const PENDA_NAME: &str = "Penda";
    /// The five `talk_key.keys` rows that publish no unique spawn:
    /// `riddle002` and `riddle003` name a packed type the family covered for a
    /// non-unique jm2 NPC spawn, `riddle004` and `riddle007` a category and
    /// `riddle005` a bare name. None of them hunts anything — idle over any
    /// scene — because a packed type is the only identity the posted npc page
    /// can be matched by.
    const MATCHER_KEEPERS: [i32; 5] = [2833, 2835, 2837, 2839, 3605];

    fn selected() -> Arc<SelectedGameData> {
        api::game_data::for_revision(ClientRevision::R274).expect("selected data")
    }

    /// A held id that is none of the selected families' rows: not a membership
    /// row, not one of the six challenge scrolls — holding one of those is the
    /// challenge seam's own join onto its parent, not an unrelated page — and
    /// not a talk step. The C3 exemplar has to be a page no selected family
    /// names, or it would test a path that no longer exists.
    fn unrelated(data: &SelectedGameData) -> i32 {
        let trails = data.trails().expect("trails");
        let talk = data.talk_key().expect("talk_key");
        (1..)
            .find(|id| {
                !trails.rows.iter().any(|row| row.id == *id)
                    && !trails.challenge_answers.iter().any(|row| row.id == *id)
                    && !talk.talk.iter().any(|row| row.id == *id)
            })
            .expect("a held id no selected family names")
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

    /// A full pack page: the posted `inv_size` rows, none of them a listed name
    /// and none a selected trail item, so the frozen make-room deposit takes
    /// them. The first row is the one a fresh attempt's first deposit names.
    fn full_pack() -> Value {
        Value::Array(
            (0..28)
                .map(|slot| {
                    let name = if slot == 0 {
                        "Big bones".to_string()
                    } else {
                        format!("Loot {slot}")
                    };
                    inv(526 + slot, &name, 1)
                })
                .collect(),
        )
    }

    /// That page with its first row gone: the make-room deposit landed.
    fn full_minus_one() -> Value {
        Value::Array(full_pack().as_array().expect("rows")[1..].to_vec())
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

    /// The three selected coordinate-tool items the trio acquire chain joins by
    /// alias: the ids the posted pack rows carry, and the display names the
    /// pages post beside them. The join the machine makes is the id; the names
    /// are only corroboration.
    const SEXTANT_ITEM: i32 = 2574;
    const WATCH_ITEM: i32 = 2575;
    const CHART_ITEM: i32 = 2576;
    const SEXTANT_NAME: &str = "Sextant";
    const WATCH_NAME: &str = "Watch";
    const CHART_NAME: &str = "Chart";

    /// The three held trio rows as the posted pack posts them: what a sextant
    /// row's own acquire chain reads as `hasAllTrio` before it lets either dig
    /// arm run.
    fn trio_inv() -> Value {
        json!([
            inv(SEXTANT_ITEM, SEXTANT_NAME, 1),
            inv(WATCH_ITEM, WATCH_NAME, 1),
            inv(CHART_ITEM, CHART_NAME, 1),
        ])
    }

    /// The posted pack a sextant row's non-acquire scene carries: the held trio
    /// plus the rows a test names. A test that posts no trio at all is a test
    /// of the intercept itself.
    fn trio_pack(rows: &[Value]) -> Value {
        let mut pack = trio_inv();
        pack.as_array_mut().expect("rows").extend_from_slice(rows);
        pack
    }

    /// The tile the selected observatory professor stands on, as the published
    /// `trio_givers` row posts it — never a copied frozen coordinate.
    fn professor_tile(data: &SelectedGameData) -> Tile {
        giver_tile(data, OBSERVATORY_PROFESSOR)
    }

    /// The published spawn of one selected giver, as this machine reads it.
    fn giver_tile(data: &SelectedGameData, alias: &str) -> Tile {
        let row = data
            .trio_givers()
            .expect("trio_givers")
            .rows
            .iter()
            .find(|row| row.alias == alias)
            .unwrap_or_else(|| panic!("giver {alias}"));
        let spawn = row.spawn.as_ref().expect("published spawn");
        Tile {
            x: spawn.x,
            z: spawn.z,
            level: spawn.plane,
        }
    }

    /// The decoded tile of the guarded exemplar `2723`: `0_47_60_50_44`.
    fn guarded_tile_of(data: &SelectedGameData) -> Tile {
        guarded_tile(row(data, GUARDED)).expect("guarded tile")
    }

    /// The marshalled scene the guarded encounter walks and Digs over: the
    /// posted `here` on (or off) the decoded tile and the pack that carries
    /// the Spade beside the held trio the acquire chain already cleared.
    fn dig_scene(here_tile: Value, spade: bool) -> Value {
        json!({ "here": here_tile, "inv": dig_inv(spade) })
    }

    /// The pack a dig or fight scene posts: the held trio, and — when the test
    /// says so — the Spade the Dig resolves. The Spade is posted as the
    /// marshalled item row the host resolves by display name.
    fn dig_inv(spade: bool) -> Value {
        if spade {
            trio_pack(&[inv(SPADE_ITEM, SPADE_NAME, 1)])
        } else {
            trio_pack(&[])
        }
    }

    /// One selected giver's own packed id and display name, as the published
    /// `trio_givers` row carries them — never a copied number and never a
    /// frozen table.
    fn giver_identity<'a>(data: &'a SelectedGameData, alias: &str) -> (i32, &'a str) {
        let row = data
            .trio_givers()
            .expect("trio_givers")
            .rows
            .iter()
            .find(|row| row.alias == alias)
            .unwrap_or_else(|| panic!("giver {alias}"));
        (row.id, row.name.as_str())
    }

    /// One posted chat option row: the text the arm folds, and the 1-based
    /// posted slot it answers with.
    fn option(text: &str, slot: i32) -> Value {
        json!({ "text": text, "option": slot })
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
            "inv": dig_inv(true),
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
        let opened = begin(&data, json!([[MAP_EMPTY, 1]]));
        let token = token_of(&opened);

        let bumped = dispatch(
            Some(&data),
            &payload(
                "next",
                Some(token),
                json!([[MAP_EMPTY, 1]]),
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
            "grind-ready",
            "done",
            "dead",
            "abandon",
            "supplies-needed",
            "guardian-lost",
        ] {
            assert_ne!(bumped["kind"], kind, "{bumped}");
        }
        // The token died with the bump: the same call on the old generation
        // is stale, not a resumed session.
        let after = call(&data, token, json!([[MAP_EMPTY, 1]]), json!({}));
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
                json!([[MAP_EMPTY, 1]]),
                json!({ "enabled": false, "resume": false, "hold": true }),
            ),
        );
        assert_eq!(opened["kind"], "token", "{opened}");
        let token = token_of(&opened);
        assert!(
            opened.get("kind").is_some() && opened.get("reason").is_none(),
            "begin never returns a continue kind and never a reason: {opened}"
        );

        let first = call(&data, token, json!([[MAP_EMPTY, 1]]), json!({}));
        assert_eq!(first["kind"], "callback.enabled", "{first}");
        // False means do not execute: an idle continue with the token live.
        let denied = call(
            &data,
            token,
            json!([[MAP_EMPTY, 1]]),
            json!({ "resume": false }),
        );
        assert_eq!(denied["kind"], "wait", "{denied}");
        assert_eq!(token_of(&denied), token, "{denied}");
        // The next tick asks again rather than replaying the captured answer.
        let again = call(&data, token, json!([[MAP_EMPTY, 1]]), json!({}));
        assert_eq!(again["kind"], "callback.enabled", "{again}");
        // And a later true is honored, because nothing was captured at begin.
        let enabled = call(
            &data,
            token,
            json!([[MAP_EMPTY, 1]]),
            json!({ "resume": true }),
        );
        assert_eq!(enabled["kind"], "callback.log", "{enabled}");
        let message = enabled["message"].as_str().unwrap_or("");
        assert!(message.contains("trail_clue_hard_map001"), "{enabled}");
        assert!(message.contains("2722"), "{enabled}");

        // The paramless 2722 row keeps the token and then idles.
        let steady = call(&data, token, json!([[MAP_EMPTY, 1]]), json!({}));
        assert_eq!(steady["kind"], "callback.setStatus", "{steady}");
        assert!(
            steady["message"]
                .as_str()
                .unwrap_or("")
                .contains("trail_clue_hard_map001"),
            "{steady}"
        );
        for _ in 0..2 {
            let idle = call(&data, token, json!([[MAP_EMPTY, 1]]), json!({}));
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
        // Another row replaces the casket: the enabled read is not reused.
        let re_armed = call(
            &data,
            token,
            json!([[MAP_EMPTY, 1]]),
            json!({ "resume": true }),
        );
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

    /// The solved mark is eleven characters, lowercase, one space — the exact
    /// string — and it is the machine's own `callback.setStatus`, posted once a
    /// collect is over and by no other path on this machine.
    #[test]
    fn the_exact_clue_solved_string_is_the_finished_collects_alone() {
        on_reset();
        let data = selected();
        // The paramless 2722 row is identified, reported and then idled, and
        // never a verb of any kind — and never the solved mark.
        let token = token_of(&begin(&data, json!([[MAP_EMPTY, 1]])));
        let steps = vec![
            call(&data, token, json!([[MAP_EMPTY, 1]]), json!({})),
            call(
                &data,
                token,
                json!([[MAP_EMPTY, 1]]),
                json!({ "resume": true }),
            ),
            call(&data, token, json!([[MAP_EMPTY, 1]]), json!({})),
            call(&data, token, json!([[MAP_EMPTY, 1]]), json!({})),
            call(
                &data,
                token,
                json!([[MAP_EMPTY, 1]]),
                json!({ "hold": true }),
            ),
        ];
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
        for step in &steps {
            let text = step.to_string();
            assert!(!text.contains("clue solved"), "{step}");
            assert!(!text.contains("ownsEquipment"), "{step}");
            assert!(
                step["status"].is_null(),
                "the machine emits kinds, not the public status: {step}"
            );
        }

        // The finished collect is that one seat: the status is the exact
        // string, it rides the live token, and the two completion steps that
        // follow it are the continue and the end of the session.
        let token = opened(&data, EASY_CASKET);
        let scene = pages(json!([]), json!([]), json!(28), json!(-1));
        let waiting = call(&data, token, json!([]), scene.clone());
        assert_eq!(waiting["kind"], "wait", "{waiting}");
        assert!(!waiting.to_string().contains("clue solved"), "{waiting}");
        force_bound();
        let solved = call(&data, token, json!([]), scene.clone());
        assert_eq!(solved["kind"], "callback.setStatus", "{solved}");
        assert_eq!(solved["message"], "clue solved", "{solved}");
        assert_eq!(token_of(&solved), token, "{solved}");
        let ready = call(&data, token, json!([]), scene.clone());
        assert_eq!(ready["kind"], "grind-ready", "{ready}");
        assert_eq!(
            token_of(&ready),
            token,
            "the grind handback is live: {ready}"
        );
        assert!(!ready.to_string().contains("clue solved"), "{ready}");
        let done = call(&data, token, json!([]), scene.clone());
        assert_eq!(done["kind"], "done", "{done}");
        let after = call(&data, token, json!([]), scene);
        assert_eq!(after["kind"], "aborted", "{after}");
        assert_eq!(after["reason"], "stale", "{after}");
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
        // A casket is a held Open and the packed 3554 clue is the constrained
        // refusal, so neither is in this set: 2722 is a paramless clue row and
        // 2831 is the key-hunt riddle, which walks to its published spawn from
        // this very scene and has its own proof below. The coord-only map is no
        // longer one of them either: the widened dig arm walks and Digs from
        // its own tile. The five matcher-keepers the key family publishes no
        // packed type for are idled here too.
        for id in std::iter::once(MAP_EMPTY).chain(MATCHER_KEEPERS) {
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

    /// The packed 3554 `access: "constrained"` clue is not idled: the machine
    /// refuses it — `aborted` / `constrained`, no verb and no live token — and
    /// a `begin` that identifies it is refused the same way.
    #[test]
    fn a_held_constrained_row_is_refused_and_never_played() {
        on_reset();
        let data = selected();
        let page = json!([[CLUE, 1]]);
        let refused = begin(&data, page.clone());
        assert_eq!(refused["kind"], "aborted", "{refused}");
        assert_eq!(refused["reason"], "constrained", "{refused}");
        // No live token: the refusal's own number is not a session.
        let after = call(&data, token_of(&refused), page.clone(), json!({}));
        assert_eq!(after["reason"], "stale", "{after}");

        // The same row held by a live session, over a scene the dig arms would
        // walk and Dig from: identified, then refused with no verb at all. The
        // refusal is not the gate: the constrained check runs before any
        // re-arm, so the previous step's identity never gets a callback.
        let scene = json!({
            "here": here(3209, 3218, 1),
            "locs": [loc(11, 3209, 3218, 1, &["Search"])],
            "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)],
        });
        let token = token_of(&begin(&data, json!([[CASKET, 1]])));
        let denied = call(&data, token, page.clone(), scene.clone());
        assert_eq!(denied["kind"], "aborted", "{denied}");
        assert_eq!(denied["reason"], "constrained", "{denied}");
        for absent in ["name", "action", "x", "z", "level", "message", "id"] {
            assert!(denied.get(absent).is_none(), "{absent} {denied}");
        }
        let after = call(&data, token, page, scene);
        assert_eq!(after["kind"], "aborted", "{after}");
        assert_eq!(after["reason"], "stale", "{after}");
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
        // and the paramless row behind it is not opened.
        let other = call(&data, token, json!([[MAP_EMPTY, 1]]), json!({}));
        assert_eq!(other["kind"], "callback.enabled", "{other}");
        assert_eq!(token_of(&other), token, "{other}");
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
        // The clue left on its own is the constrained refusal, never 3554
        // play: casket-first is the precedence that kept it out of the Open.
        let clue_only = call(&data, token, json!([[CLUE, 1]]), json!({}));
        assert_eq!(clue_only["kind"], "aborted", "{clue_only}");
        assert_eq!(clue_only["reason"], "constrained", "{clue_only}");
        let after = call(&data, token, json!([[CLUE, 1]]), json!({}));
        assert_eq!(after["reason"], "stale", "{after}");
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
    /// page after it finishes with the machine's own three-step completion —
    /// the exact `'clue solved'` status, then `grind-ready`, then the `done`
    /// the token dies on. This is the sextant casket the packed 3554 clue
    /// belongs to, so the collect is never 3554 play.
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

        // Past it the collect is over: the machine's own completion, one step
        // per call — the exact status, the live-token continue, then the end.
        force_bound();
        let solved = call(
            &data,
            token,
            json!([]),
            pages(json!([]), json!([]), json!(28), json!(-1)),
        );
        assert_eq!(solved["kind"], "callback.setStatus", "{solved}");
        assert_eq!(solved["message"], "clue solved", "{solved}");
        assert_eq!(solved["token"], json!(token), "{solved}");
        assert!(bound_armed(), "the status is not the end of the session");

        // Latched: the same page runs the completion, not the loot again. The
        // grind handback is a live token and a verbless continue.
        let ready = call(
            &data,
            token,
            json!([]),
            pages(json!([]), json!([]), json!(28), json!(-1)),
        );
        assert_eq!(ready["kind"], "grind-ready", "{ready}");
        assert_eq!(ready["token"], json!(token), "{ready}");
        for absent in ["action", "name", "x", "z", "level", "message", "id"] {
            assert!(ready.get(absent).is_none(), "{absent} {ready}");
        }
        let done = call(
            &data,
            token,
            json!([]),
            pages(json!([]), json!([]), json!(28), json!(-1)),
        );
        assert_eq!(done["kind"], "done", "{done}");
        assert!(!bound_armed(), "the end clears the deadline");
        let after = call(
            &data,
            token,
            json!([]),
            pages(json!([]), json!([]), json!(28), json!(-1)),
        );
        assert_eq!(after["kind"], "aborted", "{after}");
        assert_eq!(after["reason"], "stale", "{after}");

        let text = json!([closed, other, took, logged, waiting, solved, ready]).to_string();
        for forbidden in [
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
        let page = json!([[RIDDLE, 1]]);
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
        assert!(message.contains("2831"), "{logged}");
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
    /// finishes with the machine's own completion: a log line, not an error
    /// token — and never the landed `none-held` abort, which is for the paths
    /// that are not a finished collect.
    #[test]
    fn a_full_pack_with_no_food_warns_and_then_finishes_done() {
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
        // rather than Taking, and the token dies with the `done`.
        let solved = call(&data, token, json!([]), scene.clone());
        assert_eq!(solved["kind"], "callback.setStatus", "{solved}");
        assert_eq!(solved["message"], "clue solved", "{solved}");
        let ready = call(&data, token, json!([]), scene.clone());
        assert_eq!(ready["kind"], "grind-ready", "{ready}");
        assert_eq!(ready["token"], json!(token), "{ready}");
        let done = call(&data, token, json!([]), scene.clone());
        assert_eq!(done["kind"], "done", "{done}");
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

    /// The whole collect arm's outcome set: the landed verbs, the wait inside
    /// the reward window, and then the machine's own completion — the exact
    /// `'clue solved'` status, the `grind-ready` handback and the `done` the
    /// token dies on. Nothing else in the envelope is reachable from this arm.
    #[test]
    fn the_collect_arm_outcome_set_is_the_verbs_and_the_completion() {
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

        // Past the reward window the arm finishes, and the finish is the three
        // completion steps above and nothing else.
        force_bound();
        let empty = pages(json!([]), json!([]), json!(28), json!(-1));
        let solved = call(&data, token, json!([]), empty.clone());
        assert_eq!(solved["kind"], "callback.setStatus", "{solved}");
        assert_eq!(solved["message"], "clue solved", "{solved}");
        assert_eq!(solved["token"], json!(token), "{solved}");
        let ready = call(&data, token, json!([]), empty.clone());
        assert_eq!(ready["kind"], "grind-ready", "{ready}");
        assert_eq!(ready["token"], json!(token), "{ready}");
        let done = call(&data, token, json!([]), empty.clone());
        assert_eq!(done["kind"], "done", "{done}");
        let after = call(&data, token, json!([]), empty);
        assert_eq!(after["kind"], "aborted", "{after}");
        assert_eq!(after["reason"], "stale", "{after}");
    }

    /// The dig membership is the selected-param classify, not the frozen
    /// `type`: a decodable `trail_coord` on a row with no `trail_loc`, no
    /// `trail_guardian` and an `access` that is not `"constrained"`. The
    /// `trail_sextant` param is not read here, so the forty rows are the twenty
    /// medium sextant rows plus the coord-bearing easy maps, the vague and the
    /// riddle-with-coord rows, on both pins — with no swallow of a search row
    /// and no leak of a guarded, packed, coord-less or casket row.
    #[test]
    fn the_unguarded_dig_membership_is_the_selected_param_set() {
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let data = api::game_data::for_revision(revision).expect("selected data");
            let facts = data.trails().expect("trails");
            // The pinned decodes: the medium sextant 2801's selected token is
            // (3160, 3251, 0), and the easy map 2713's is (3177, 3360, 0).
            assert_eq!(
                dig_tile(row(&data, UNGUARDED)),
                Some(Tile {
                    x: 3160,
                    z: 3251,
                    level: 0
                }),
                "{revision:?}"
            );
            assert_eq!(
                dig_tile(row(&data, MAP)),
                Some(Tile {
                    x: 3177,
                    z: 3360,
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
                    2713, 2716, 2719, 3510, 3516, 3518, 2827, 2801, 2803, 2805, 2807, 2809, 2811,
                    2813, 2815, 2817, 2819, 2821, 2823, 2825, 3582, 3584, 3586, 3588, 3590, 3592,
                    3594, 3596, 3599, 3602, 2774, 2776, 2780, 2783, 2786, 2788, 2790, 3520, 3522,
                    3580,
                ],
                "{revision:?}"
            );
            assert_eq!(members.len(), 40, "{revision:?}");
            // Half the forty pin the sextant and half do not: the param is not
            // the membership, and neither is `trail_casket`.
            let sextant = members
                .iter()
                .filter(|id| {
                    row(&data, **id)
                        .params
                        .iter()
                        .any(|param| param.key == "trail_sextant" && param.value == "yes")
                })
                .count();
            assert_eq!(sextant, 20, "{revision:?}");
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
            // No leak: the guarded rows are the same sextant shape plus a
            // guardian, the constrained 3554 clue carries a decodable coord
            // and its own casket, the desc-only 2831 has no coord at all, and
            // the paramless 2722 and every casket stay out.
            for id in [GUARDED, CLUE, RIDDLE, MAP_EMPTY, CASKET, SEXTANT_CASKET] {
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
        // Synthetic rows: the coord alone is the membership the sextant pin
        // used to gate, either sextant value rides along, a loc param of any
        // value is not this membership, and an off-contract token is never
        // rounded into an invented coordinate.
        let sextant = || param("trail_sextant", "yes");
        let coord = || param("trail_coord", "0_49_50_24_51");
        let hit = Some(Tile {
            x: 3160,
            z: 3251,
            level: 0,
        });
        assert_eq!(dig_tile(&member(vec![coord()], None)), hit);
        assert_eq!(dig_tile(&member(vec![sextant(), coord()], None)), hit);
        assert_eq!(
            dig_tile(&member(vec![param("trail_sextant", "no"), coord()], None)),
            hit
        );
        assert_eq!(
            dig_tile(&member(
                vec![coord(), param("trail_casket", "trail_clue_test_casket")],
                None
            )),
            hit
        );
        assert_eq!(dig_tile(&member(vec![sextant()], None)), None);
        for blocked in [
            vec![param("trail_loc", "^true"), coord()],
            vec![param("trail_loc", "^false"), sextant(), coord()],
            vec![param("trail_guardian", "trail_hard"), coord()],
            vec![param("trail_guardian", "trail_hard"), sextant(), coord()],
            vec![sextant(), param("trail_coord", "0_49_50_24")],
        ] {
            assert_eq!(dig_tile(&member(blocked, None)), None);
        }
        // `access` is read as the one constrained bound it is: any other value
        // — and a row that was posted with none — is outside the refusal.
        assert_eq!(dig_tile(&member(vec![coord()], Some("constrained"))), None);
        assert_eq!(dig_tile(&member(vec![coord()], Some("open"))), hit);
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
            let walk = call(&data, token, page.clone(), dig_scene(far, false));
            assert_eq!(walk["kind"], "walk", "{walk}");
            assert_eq!(walk["x"], 3160, "{walk}");
            assert_eq!(walk["z"], 3251, "{walk}");
            assert_eq!(walk["level"], 0, "{walk}");
            assert_eq!(token_of(&walk), token, "{walk}");
        }
        // No posted `here` at all: no arrival claim and no blind walk, even
        // with the Spade posted beside the held trio.
        let no_tile = call(&data, token, page.clone(), json!({ "inv": dig_inv(true) }));
        assert_eq!(no_tile["kind"], "wait", "{no_tile}");
        // Arrived: the held Spade is the Dig, and it repeats while this same
        // clue id stays held.
        for here_tile in [here(3160, 3251, 0), here(3161, 3250, 0)] {
            let dig = call(&data, token, page.clone(), dig_scene(here_tile, true));
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

    /// Arrived without the Spade on this call's posted pack page is the named
    /// `supplies-needed` wait-class: never `abandon`, never a public
    /// `no-spade`, and never a fetch. The token stays live for the page that
    /// posts it.
    #[test]
    fn an_unguarded_dig_row_without_the_spade_is_supplies_needed_and_live() {
        on_reset();
        let data = selected();
        let page = json!([[UNGUARDED, 1]]);
        let token = steady(&data, UNGUARDED);
        let arrived = here(3160, 3251, 0);
        for pack in [
            // No page at all, an empty page, another item, a zero count, a
            // nameless row and a name that is not the display. Every one of
            // them holds the trio the acquire chain already cleared: the
            // Spade is the only thing missing.
            trio_pack(&[]),
            trio_pack(&[inv(385, "Shark", 5)]),
            trio_pack(&[inv(SPADE_ITEM, SPADE_NAME, 0)]),
            trio_pack(&[inv(SPADE_ITEM, "", 1)]),
            trio_pack(&[inv(SPADE_ITEM, "Spade cert", 1)]),
            trio_pack(&[json!({ "id": SPADE_ITEM, "count": 1 })]),
        ] {
            let idle = call(
                &data,
                token,
                page.clone(),
                json!({ "here": arrived.clone(), "inv": pack }),
            );
            assert_eq!(idle["kind"], "supplies-needed", "{idle}");
            assert_eq!(token_of(&idle), token, "the wait-class is live: {idle}");
            for absent in ["action", "name", "x", "z", "level", "message", "id"] {
                assert!(idle.get(absent).is_none(), "{absent} {idle}");
            }
            let text = idle.to_string();
            for forbidden in ["no-spade", "abandon", "done", "clue solved", "dead"] {
                assert!(!text.contains(forbidden), "{idle}");
            }
        }
        // A malformed `here` is a plain wait, and the whole `inv` slot may be
        // omitted: neither is a verb.
        let malformed = call(
            &data,
            token,
            page.clone(),
            json!({ "here": json!({ "x": 3160 }), "inv": dig_inv(true) }),
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
                "inv": trio_pack(&[inv(385, "Shark", 5), inv(953, "sPaDe", 1)]),
            }),
        );
        assert_eq!(dig["kind"], "held", "{dig}");
        assert_eq!(dig["name"], "Spade", "{dig}");
        assert_eq!(dig["action"], "Dig", "{dig}");
    }

    /// The rows the dig classify leaves out stay identified then idle even over
    /// a scene the dig arm would walk and Dig from — `here` on the row's own
    /// selected tile with the Spade posted: the paramless 2722, the five
    /// matcher-keepers the key family publishes no packed type for, and the
    /// desc-only riddles no key row names. 2831 is no longer one of them: the
    /// key-keeper hunt walks to its own published spawn from that same tile.
    /// The packed constrained 3554 clue is refused instead of idled, and the
    /// guarded row is no longer one of them either: its own encounter walks and
    /// Digs from this same scene.
    #[test]
    fn rows_outside_the_dig_classify_stay_idle_over_a_walkable_dig_scene() {
        on_reset();
        let data = selected();
        for id in std::iter::once(MAP_EMPTY).chain(MATCHER_KEEPERS) {
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

        // The constrained clue is not idled over that scene: it is refused,
        // with no walk, no Dig and no token left.
        let scene = json!({
            "here": here(3160, 3251, 0),
            "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)],
        });
        let page = json!([[CLUE, 1]]);
        let token = token_of(&begin(&data, json!([[MAP_EMPTY, 1]])));
        let refused = call(&data, token, page.clone(), scene.clone());
        assert_eq!(refused["kind"], "aborted", "{refused}");
        assert_eq!(refused["reason"], "constrained", "{refused}");
        let after = call(&data, token, page, scene);
        assert_eq!(after["reason"], "stale", "{after}");
    }

    /// `Steady` on a coord-only map row: the widened unguarded dig is the same
    /// walk-then-Dig the medium sextant rows keep, so `trail_clue_easy_map001`
    /// walks to its own decoded (3177, 3360, 0) and Digs the Spade with no
    /// `trail_sextant` on the row at all.
    #[test]
    fn a_coord_only_map_row_walks_to_its_decoded_tile_and_digs_the_spade() {
        on_reset();
        let data = selected();
        let page = json!([[MAP, 1]]);
        let token = steady(&data, MAP);
        // Not arrived: the walk is the map's own decoded tile, not the
        // sextant sibling's.
        let walk = call(
            &data,
            token,
            page.clone(),
            json!({ "here": here(3100, 3300, 0) }),
        );
        assert_eq!(walk["kind"], "walk", "{walk}");
        assert_eq!(walk["x"], 3177, "{walk}");
        assert_eq!(walk["z"], 3360, "{walk}");
        assert_eq!(walk["level"], 0, "{walk}");
        assert_eq!(token_of(&walk), token, "{walk}");
        // Arrived without the Spade: the named wait-class, never a verb.
        let bare = call(
            &data,
            token,
            page.clone(),
            json!({ "here": here(3177, 3360, 0) }),
        );
        assert_eq!(bare["kind"], "supplies-needed", "{bare}");
        assert_eq!(token_of(&bare), token, "{bare}");
        // Arrived with it: the same held Spade Dig the sibling dispatches.
        let dig = call(
            &data,
            token,
            page.clone(),
            json!({ "here": here(3177, 3360, 0), "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)] }),
        );
        assert_eq!(dig["kind"], "held", "{dig}");
        assert_eq!(dig["name"], "Spade", "{dig}");
        assert_eq!(dig["action"], "Dig", "{dig}");
        assert_eq!(token_of(&dig), token, "{dig}");
        // The clue left: still the landed `none-held` abort, and a Dig never
        // arms the collect seam.
        let gone = call(&data, token, json!([]), json!({}));
        assert_eq!(gone["kind"], "aborted", "{gone}");
        assert_eq!(gone["reason"], "none-held", "{gone}");
        assert!(!bound_armed(), "a dig row never arms the reward window");
    }

    /// Freeze and yield beat the dig arm the way they beat the landed verbs: no
    /// walk and no held ride along, and the token lives for the thaw.
    #[test]
    fn freeze_and_yield_beat_the_dig() {
        on_reset();
        let data = selected();
        let page = json!([[UNGUARDED, 1]]);
        let token = steady(&data, UNGUARDED);
        let scene = dig_scene(here(3160, 3251, 0), true);
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
        let scene = dig_scene(here(3160, 3251, 0), true);
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

    /// The dig row emits walk, the held Dig, wait, `supplies-needed` or yield
    /// only — never a completion kind, and never a `status` field of its own.
    #[test]
    fn the_dig_row_emits_only_walk_held_wait_supplies_needed_and_yield() {
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
                json!({ "here": here(3160, 3251, 0), "inv": dig_inv(false) }),
            ),
            call(
                &data,
                token,
                page.clone(),
                json!({ "here": here(3160, 3251, 0), "inv": dig_inv(true) }),
            ),
            call(
                &data,
                token,
                page,
                json!({
                    "here": here(3160, 3251, 0),
                    "inv": dig_inv(true),
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
                    "walk" | "held" | "wait" | "yield" | "supplies-needed"
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
                json!("walk"),
                json!("supplies-needed"),
                json!("held"),
                json!("yield")
            ],
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
        // No posted `here` at all waits; arrived without the Spade is the
        // named wait-class. Neither starts an encounter.
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
        assert_eq!(no_spade["kind"], "supplies-needed", "{no_spade}");
        assert_eq!(token_of(&no_spade), token, "{no_spade}");
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

    /// A fight that has not settled waits: the owned wizard posted and alive,
    /// and a death beside another player that is not this token's fight. The
    /// owned index gone only after the frozen grace was spent is the other
    /// thing entirely: the encounter is lost, so the kind is `guardian-lost`
    /// and the token dies with it — never a redig, never `'clue solved'`.
    #[test]
    fn an_unsettled_fight_waits_and_a_lost_wizard_ends_the_token() {
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
        // Dead beside another player: not this token's kill, and not a loss.
        let stolen = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([targeting(npc(7, WIZARD, 3, 0, 10), 9)]), json!({})),
        );
        assert_eq!(stolen["kind"], "wait", "{stolen}");
        // Owned and still posted: the same wait.
        let alive = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([npc(7, WIZARD, 3, 10, 10)]), json!({})),
        );
        assert_eq!(alive["kind"], "wait", "{alive}");
        for step in [&stolen, &alive] {
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

        // Gone, and only after the grace was spent: the wizard is lost.
        age_owned_seen(KILL_GRACE_MS + 1);
        let lost = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([]), json!({})),
        );
        assert_eq!(lost["kind"], "guardian-lost", "{lost}");
        for absent in [
            "action",
            "index",
            "component_id",
            "name",
            "message",
            "x",
            "z",
            "level",
        ] {
            assert!(lost.get(absent).is_none(), "{absent} {lost}");
        }
        assert!(!lost.to_string().contains("clue solved"), "{lost}");
        // The token died with the encounter: no redig, and no session left.
        let after = call(&data, token, page, fight_scene(json!([]), json!({})));
        assert_eq!(after["kind"], "aborted", "{after}");
        assert_eq!(after["reason"], "stale", "{after}");
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
            fight_scene(
                json!([field(npc(7, WIZARD, 3, 10, 10), "level", json!(1))]),
                json!({}),
            ),
            fight_scene(json!([tiled(7, WIZARD, 3058, 3884, 1)]), json!({})),
            // No posted level at all is not the `here` level either, and a
            // row with no posted tile and no posted distance has no measure.
            fight_scene(
                json!([unfield(npc(7, WIZARD, 3, 10, 10), "level")]),
                json!({}),
            ),
            fight_scene(
                json!([unfield(
                    unfield(unfield(npc(7, WIZARD, 3, 10, 10), "distance"), "x"),
                    "z"
                )]),
                json!({}),
            ),
            // No posted `here`: no level to compare and no base to measure
            // from, whatever the row posted.
            fight_scene(
                json!([tiled(7, WIZARD, 3058, 3884, 0)]),
                json!({ "here": null }),
            ),
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
        assert_eq!(
            killed["kind"], "held",
            "a freeze never spends the grace: {killed}"
        );
        assert_eq!(killed["name"], SPADE_NAME, "{killed}");
        assert_eq!(killed["action"], DIG, "{killed}");
        // Frozen again: nothing is read and nothing is emitted, and the thaw
        // after it leaves the post-kill Dig where it was.
        on_pause();
        let frozen = call(
            &data,
            token,
            page.clone(),
            fight_scene(json!([]), json!({})),
        );
        assert_eq!(frozen["kind"], "wait", "{frozen}");
        assert!(frozen.get("action").is_none(), "{frozen}");
        on_resume();
        let redig = call(&data, token, page, fight_scene(json!([]), json!({})));
        assert_eq!(redig["kind"], "held", "{redig}");
        assert_eq!(redig["action"], DIG, "{redig}");
    }

    /// The posted effective hitpoints at or below zero kill the token: the kind
    /// is `dead` on any live call, the token dies with the player and nothing
    /// posts `'clue solved'`. A page that posted no stat is not a zero, so the
    /// fight still Attacks — and after the kill the redig is not gated by a
    /// player death that has already been posted.
    #[test]
    fn a_posted_hitpoints_at_zero_is_dead_and_kills_the_token() {
        on_reset();
        let data = selected();
        let page = json!([[GUARDED, 1]]);
        let wizard = json!([npc(7, WIZARD, 3, 10, 10)]);

        // Missing stat first: the Attack still goes out under a fight that is
        // posted and alive.
        let token = spawned(&data);
        let mut bare = fight_scene(wizard.clone(), json!({}));
        bare.as_object_mut().expect("object").remove("hitpoints");
        let attack = call(&data, token, page.clone(), bare);
        assert_eq!(attack["kind"], "npc", "{attack}");
        assert_eq!(attack["index"], 7, "{attack}");

        // Every posted zero-or-below is the terminal, whatever the page also
        // carries, and the token dies on it.
        for hp in [0, -1, -20] {
            on_reset();
            let token = spawned(&data);
            let downed = call(
                &data,
                token,
                page.clone(),
                fight_scene(wizard.clone(), json!({ "hitpoints": hp })),
            );
            assert_eq!(downed["kind"], "dead", "{hp} {downed}");
            for absent in ["action", "index", "component_id", "name", "message"] {
                assert!(downed.get(absent).is_none(), "{hp} {absent} {downed}");
            }
            assert!(!downed.to_string().contains("clue solved"), "{hp} {downed}");
            let after = call(
                &data,
                token,
                page.clone(),
                fight_scene(wizard.clone(), json!({})),
            );
            assert_eq!(after["kind"], "aborted", "{hp} {after}");
            assert_eq!(after["reason"], "stale", "{hp} {after}");
        }

        // The kill, then a zero posted on the very next call: the death is the
        // terminal on any live call, the post-kill redig included.
        let token = spawned(&data);
        let mut bare = fight_scene(wizard.clone(), json!({}));
        bare.as_object_mut().expect("object").remove("hitpoints");
        let attack = call(&data, token, page.clone(), bare);
        assert_eq!(attack["kind"], "npc", "{attack}");
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
        assert_eq!(after["kind"], "dead", "{after}");
        assert!(!after.to_string().contains("clue solved"), "{after}");
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

    /// `trail_clue_medium_sextant001`'s own acquire chain, walked end to end
    /// over the posted pages: the professor's published tile, his Talk-to, the
    /// one posted option the chain answers, the sextant's second giver, the
    /// watch's own giver — and then, once the posted pack holds the trio, the
    /// landed unguarded Dig.
    ///
    /// Every tile here is the published `trio_givers` row's own spawn, read
    /// from the selected family: no frozen `PROFESSOR` / `MURPHY` / `KOJO`
    /// table and no first-in-file coordinate is ever the destination.
    #[test]
    fn a_sextant_row_acquires_the_trio_in_the_frozen_order() {
        on_reset();
        let data = selected();
        let page = json!([[UNGUARDED, 1]]);
        let token = steady(&data, UNGUARDED);
        let professor = professor_tile(&data);
        let murphy = giver_tile(&data, MURPHY);
        let kojo = giver_tile(&data, BROTHER_KOJO);

        // Not arrived and no trio held: the walk is the professor's own
        // published tile, not the row's decoded one.
        let walk = call(
            &data,
            token,
            page.clone(),
            json!({ "here": here(3160, 3251, 0) }),
        );
        assert_eq!(walk["kind"], "walk", "{walk}");
        assert_eq!(walk["x"], professor.x, "{walk}");
        assert_eq!(walk["z"], professor.z, "{walk}");
        assert_eq!(walk["level"], professor.level, "{walk}");

        // Arrived with the professor posted: the talk arm's unique-spawn rule,
        // over that giver's own packed id and posted display name.
        let (id, name) = giver_identity(&data, OBSERVATORY_PROFESSOR);
        let posted = json!([talk_npc(
            9,
            id,
            name,
            Tile {
                x: professor.x,
                z: professor.z,
                level: professor.level,
            },
            1,
            &["Talk-to"],
        )]);
        let arrived = json!({
            "here": here(professor.x, professor.z, professor.level),
            "npcs": posted,
        });
        let talked = call(&data, token, page.clone(), arrived.clone());
        assert_eq!(talked["kind"], "npc", "{talked}");
        assert_eq!(talked["name"], name, "{talked}");
        assert_eq!(talked["action"], "Talk-to", "{talked}");
        assert_eq!(talked["index"], 9, "{talked}");

        // The chat opens on the closed handler's own option, posted third: the
        // answer carries that posted 1-based slot and nothing else.
        let mut chat = arrived.clone();
        chat["chat_modal_id"] = json!(968);
        chat["chat_options"] = json!([
            option("Can you tell me about Treasure Trails?", 2),
            option("Talk about Treasure Trails.", 3),
        ]);
        let answered = call(&data, token, page.clone(), chat.clone());
        assert_eq!(answered["kind"], "answer", "{answered}");
        assert_eq!(answered["option"], 3, "{answered}");
        assert_eq!(token_of(&answered), token, "{answered}");

        // The same option folds on ASCII case: the literal is matched, never a
        // fragment of it.
        chat["chat_options"] = json!([option("i've lost my navigation chart.", 1)]);
        let folded = call(&data, token, page.clone(), chat);
        assert_eq!(folded["kind"], "answer", "{folded}");
        assert_eq!(folded["option"], 1, "{folded}");

        // That chat closed: the sextant's second stop is Murphy's own tile.
        let closed = call(
            &data,
            token,
            page.clone(),
            json!({
                "here": here(professor.x, professor.z, professor.level),
                "npcs": posted,
            }),
        );
        assert_eq!(closed["kind"], "walk", "{closed}");
        assert_eq!(closed["x"], murphy.x, "{closed}");
        assert_eq!(closed["z"], murphy.z, "{closed}");

        // Murphy's chat is linear: a posted `chat_continue` is the step.
        let (mid, mname) = giver_identity(&data, MURPHY);
        let continued = call(
            &data,
            token,
            page.clone(),
            json!({
                "here": here(murphy.x, murphy.z, murphy.level),
                "npcs": [talk_npc(
                    11,
                    mid,
                    mname,
                    Tile {
                        x: murphy.x,
                        z: murphy.z,
                        level: murphy.level,
                    },
                    1,
                    &["Talk-to"],
                )],
                "chat_continue": true,
            }),
        );
        assert_eq!(continued["kind"], "continue", "{continued}");
        assert!(continued.get("option").is_none(), "{continued}");

        // The sextant landed: the chain rebases to the watch, which is Kojo's
        // alone, and Kojo's own options are none of this arm's business —
        // never the last one, never another giver's literal.
        let (kid, kname) = giver_identity(&data, BROTHER_KOJO);
        let kojo_posted = json!([talk_npc(
            12,
            kid,
            kname,
            Tile {
                x: kojo.x,
                z: kojo.z,
                level: kojo.level,
            },
            1,
            &["Talk-to"],
        )]);
        let watched = call(
            &data,
            token,
            page.clone(),
            json!({
                "here": here(murphy.x, murphy.z, murphy.level),
                "inv": [inv(SEXTANT_ITEM, SEXTANT_NAME, 1)],
            }),
        );
        assert_eq!(watched["kind"], "walk", "{watched}");
        assert_eq!(watched["x"], kojo.x, "{watched}");
        let foreign = call(
            &data,
            token,
            page.clone(),
            json!({
                "here": here(kojo.x, kojo.z, kojo.level),
                "npcs": kojo_posted,
                "inv": [inv(SEXTANT_ITEM, SEXTANT_NAME, 1)],
                "chat_modal_id": 968,
                "chat_options": [
                    option("What is this place?", 1),
                    option("Talk about Treasure Trails.", 2),
                ],
            }),
        );
        assert_eq!(foreign["kind"], "wait", "{foreign}");
        assert_eq!(token_of(&foreign), token, "{foreign}");

        // The watch landed: the chart is the professor's again, and the pack
        // that finally holds the whole trio falls through to the landed dig
        // arm — walk to the row's own decoded tile, then the held Dig.
        let charted = call(
            &data,
            token,
            page.clone(),
            json!({
                "here": here(kojo.x, kojo.z, kojo.level),
                "inv": [
                    inv(SEXTANT_ITEM, SEXTANT_NAME, 1),
                    inv(WATCH_ITEM, WATCH_NAME, 1),
                ],
            }),
        );
        assert_eq!(charted["kind"], "walk", "{charted}");
        assert_eq!(charted["x"], professor.x, "{charted}");
        let walked = call(
            &data,
            token,
            page.clone(),
            dig_scene(here(3100, 3300, 0), true),
        );
        assert_eq!(walked["kind"], "walk", "{walked}");
        assert_eq!(walked["x"], 3160, "{walked}");
        assert_eq!(walked["z"], 3251, "{walked}");
        let dig = call(&data, token, page, dig_scene(here(3160, 3251, 0), true));
        assert_eq!(dig["kind"], "held", "{dig}");
        assert_eq!(dig["name"], SPADE_NAME, "{dig}");
        assert_eq!(dig["action"], "Dig", "{dig}");
    }

    /// The intercept is in front of **both** dig arms: the guarded exemplar
    /// walks to the professor, never to its decoded tile, until the posted pack
    /// holds the trio — and only then does its own encounter spawn.
    #[test]
    fn the_trio_intercept_is_in_front_of_the_guarded_dig_too() {
        on_reset();
        let data = selected();
        let page = json!([[GUARDED, 1]]);
        let token = steady(&data, GUARDED);
        let professor = professor_tile(&data);
        let walk = call(
            &data,
            token,
            page.clone(),
            json!({ "here": here(3100, 3300, 0), "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)] }),
        );
        assert_eq!(walk["kind"], "walk", "{walk}");
        assert_eq!(walk["x"], professor.x, "{walk}");
        assert_eq!(walk["z"], professor.z, "{walk}");
        // Held trio and Spade: the encounter's own walk-then-Dig, unchanged.
        let tile = guarded_tile_of(&data);
        let walked = call(
            &data,
            token,
            page.clone(),
            dig_scene(here(3100, 3300, 0), true),
        );
        assert_eq!(walked["kind"], "walk", "{walked}");
        assert_eq!(walked["x"], tile.x, "{walked}");
        let dig = call(
            &data,
            token,
            page,
            dig_scene(here(tile.x, tile.z, tile.level), true),
        );
        assert_eq!(dig["kind"], "held", "{dig}");
        assert_eq!(dig["action"], "Dig", "{dig}");
    }

    /// A posted option list the selected literals do not match waits: never the
    /// last option, never a frozen fragment (`Treasure Trails`, `lost`), never
    /// the trawler's or the Clock Tower quest's choices, and never a Talk-to
    /// behind the open chat.
    #[test]
    fn the_trio_intercept_waits_on_options_it_may_not_answer() {
        on_reset();
        let data = selected();
        let page = json!([[UNGUARDED, 1]]);
        let token = steady(&data, UNGUARDED);
        let professor = professor_tile(&data);
        let (id, name) = giver_identity(&data, OBSERVATORY_PROFESSOR);
        let npcs = json!([talk_npc(
            9,
            id,
            name,
            Tile {
                x: professor.x,
                z: professor.z,
                level: professor.level,
            },
            1,
            &["Talk-to"],
        )]);
        let arrived = json!({
            "here": here(professor.x, professor.z, professor.level),
            "npcs": npcs,
        });
        // An open chat is posted open: even the giver's own row is not
        // Talked-to again behind it.
        let bare = call(
            &data,
            token,
            page.clone(),
            json!({
                "here": here(professor.x, professor.z, professor.level),
                "npcs": npcs,
                "chat_modal_id": 968,
            }),
        );
        assert_eq!(bare["kind"], "wait", "{bare}");
        for options in [
            // Frozen fragments, another giver's literal, the last option a
            // sequencer would fall back to, and a matched text with no slot.
            json!([
                option("Treasure Trails", 1),
                option("I am lost!!!", 2),
                option("Yes please.", 3),
            ]),
            json!([option("I've lost my navigation chart", 1)]),
            json!([json!({ "text": "Talk about Treasure Trails." })]),
            json!([]),
        ] {
            let mut scene = arrived.clone();
            scene["chat_modal_id"] = json!(968);
            scene["chat_options"] = options.clone();
            let idle = call(&data, token, page.clone(), scene);
            assert_eq!(idle["kind"], "wait", "{options} {idle}");
            assert_eq!(token_of(&idle), token, "{options} {idle}");
            assert!(
                idle.get("option").is_none() && idle.get("index").is_none(),
                "{options} {idle}"
            );
        }
    }

    /// The picker is the talk arm's unique-spawn rule: the giver's own packed
    /// id first, then its posted display name, a posted talk action, and the
    /// frozen radius around the published tile. A wanderer is not chased, a
    /// same-name lookalike with another packed id is not this giver, and a page
    /// with no match waits at the tile.
    #[test]
    fn the_trio_pick_is_the_unique_spawn_rule_and_never_a_lookalike() {
        on_reset();
        let data = selected();
        let page = json!([[UNGUARDED, 1]]);
        let token = steady(&data, UNGUARDED);
        let professor = professor_tile(&data);
        let here_tile = here(professor.x, professor.z, professor.level);
        let (id, name) = giver_identity(&data, OBSERVATORY_PROFESSOR);
        let tile = Tile {
            x: professor.x,
            z: professor.z,
            level: professor.level,
        };
        for npcs in [
            // No posted npc page at all, an empty page, and another npc.
            json!([]),
            json!([talk_npc(1, 541, "Zeke", tile, 1, &["Talk-to"])]),
            // The `observatory_professor2` lookalike: the same display name and
            // another packed id, which is not this family's row.
            json!([talk_npc(2, id + 1, name, tile, 1, &["Talk-to"])]),
            // This giver's own id, but no posted talk action.
            json!([talk_npc(3, id, name, tile, 1, &["Attack"])]),
            // Posted, off the published tile and out of the radius.
            json!([talk_npc(
                4,
                id,
                name,
                Tile {
                    x: tile.x + 8,
                    z: tile.z,
                    level: tile.level,
                },
                8,
                &["Talk-to"],
            )]),
            // Posted on another level.
            json!([talk_npc(
                5,
                id,
                name,
                Tile {
                    x: tile.x,
                    z: tile.z,
                    level: tile.level + 1,
                },
                1,
                &["Talk-to"],
            )]),
        ] {
            let idle = call(
                &data,
                token,
                page.clone(),
                json!({ "here": here_tile, "npcs": npcs }),
            );
            assert_eq!(idle["kind"], "wait", "{npcs} {idle}");
            assert_eq!(token_of(&idle), token, "{npcs} {idle}");
        }
        // The posted name alone is enough when the packed id is not posted: the
        // verb then carries that posted name and index.
        let named = call(
            &data,
            token,
            page.clone(),
            json!({
                "here": here_tile,
                "npcs": [unfield(talk_npc(6, id, name, tile, 1, &["Talk-to"]), "id")],
            }),
        );
        assert_eq!(named["kind"], "npc", "{named}");
        assert_eq!(named["name"], name, "{named}");
        assert_eq!(named["index"], 6, "{named}");
    }

    /// Freeze, hold, the posted interrupt and a posted death all beat the
    /// acquire chain exactly as they beat the landed arms, and the chain itself
    /// never emits a completion kind.
    #[test]
    fn freeze_yield_and_death_beat_the_trio_acquire() {
        on_reset();
        let data = selected();
        let page = json!([[UNGUARDED, 1]]);
        let token = steady(&data, UNGUARDED);
        let professor = professor_tile(&data);
        let here_tile = here(professor.x, professor.z, professor.level);
        let (id, name) = giver_identity(&data, OBSERVATORY_PROFESSOR);
        let scene = json!({
            "here": here_tile,
            "npcs": [talk_npc(
                9,
                id,
                name,
                Tile {
                    x: professor.x,
                    z: professor.z,
                    level: professor.level,
                },
                1,
                &["Talk-to"],
            )],
        });
        on_pause();
        let paused = call(&data, token, page.clone(), scene.clone());
        assert_eq!(paused["kind"], "wait", "{paused}");
        on_resume();
        on_hold(true);
        let held_clock = call(&data, token, page.clone(), scene.clone());
        assert_eq!(held_clock["kind"], "wait", "{held_clock}");
        on_hold(false);
        let mut yielded_scene = scene.clone();
        yielded_scene["hold"] = json!(true);
        let yielded = call(&data, token, page.clone(), yielded_scene);
        assert_eq!(yielded["kind"], "yield", "{yielded}");
        let mut dead_scene = scene.clone();
        dead_scene["hitpoints"] = json!(0);
        let dead = call(&data, token, page.clone(), dead_scene);
        assert_eq!(dead["kind"], "dead", "{dead}");
        for step in [&paused, &held_clock, &yielded, &dead] {
            let text = step.to_string();
            for forbidden in ["clue solved", "grind-ready", "cause", "message", "answer"] {
                assert!(!text.contains(forbidden), "{forbidden} {step}");
            }
        }
        assert_eq!(
            call(&data, token, page, scene)["kind"],
            "aborted",
            "the dead token is gone"
        );
    }

    /// The membership and the plan over the selected pins: the param is the
    /// whole of the row's need, the trio is the posted pack's own read, and the
    /// order is the frozen `nextCoordTool` one — sextant, then watch, then
    /// chart. A pack that already holds the whole trio is not this arm's.
    #[test]
    fn the_trio_plan_is_the_param_and_the_posted_pack() {
        on_reset();
        let data = selected();
        // The param on the row itself, and nothing else: another value, the
        // sibling params and a paramless row are all not this arm's.
        assert!(needs_trio(row(&data, UNGUARDED)));
        assert!(needs_trio(row(&data, GUARDED)));
        assert!(needs_trio(&member(vec![param(TRAIL_SEXTANT, "yes")], None)));
        assert!(!needs_trio(&member(vec![param(TRAIL_SEXTANT, "no")], None)));
        assert!(!needs_trio(&member(
            vec![param("trail_casket", "trail_clue_medium_sextant001_casket")],
            None
        )));
        assert!(!needs_trio(row(&data, MAP)));
        assert!(!needs_trio(row(&data, RIDDLE)));
        assert!(!needs_trio(&member(vec![], None)));
        // No selected pin and no published givers: no plan at all.
        assert!(trio_plan(None, &json!({})).is_none());
        // The plan is the first tool the posted pack is short of.
        let empty = json!({});
        let (givers, tool) = trio_plan(Some(&data), &empty).expect("plan");
        assert_eq!(tool, Tool::Sextant);
        assert_eq!(stop_of(&givers, tool, 0).row.alias, OBSERVATORY_PROFESSOR);
        assert_eq!(stop_of(&givers, tool, 1).row.alias, MURPHY);
        assert_eq!(last_stop(tool), 1);
        let sextant = json!({ "inv": [inv(SEXTANT_ITEM, SEXTANT_NAME, 1)] });
        let (givers, tool) = trio_plan(Some(&data), &sextant).expect("plan");
        assert_eq!(tool, Tool::Watch);
        assert_eq!(stop_of(&givers, tool, 0).row.alias, BROTHER_KOJO);
        assert_eq!(last_stop(tool), 0);
        let watch = json!({
            "inv": [
                inv(SEXTANT_ITEM, SEXTANT_NAME, 1),
                inv(WATCH_ITEM, WATCH_NAME, 1),
            ],
        });
        let (givers, tool) = trio_plan(Some(&data), &watch).expect("plan");
        assert_eq!(tool, Tool::Chart);
        assert_eq!(stop_of(&givers, tool, 0).row.alias, OBSERVATORY_PROFESSOR);
        // A zero count, a missing count and an unknown id are not held: the
        // join is the selected id beside a positive posted count, and the
        // posted display name is never the identity.
        for pack in [
            json!({ "inv": [inv(SEXTANT_ITEM, SEXTANT_NAME, 0)] }),
            json!({ "inv": [json!({ "id": SEXTANT_ITEM, "name": SEXTANT_NAME })] }),
            json!({ "inv": [inv(999_999, SEXTANT_NAME, 1)] }),
        ] {
            let (_, tool) = trio_plan(Some(&data), &pack).expect("plan");
            assert_eq!(tool, Tool::Sextant, "{pack}");
        }
        // The whole trio held: the intercept's own completion, and the landed
        // dig arms run.
        assert!(trio_plan(Some(&data), &json!({ "inv": trio_inv() })).is_none());
    }

    /// The rows this arm is not: a search row, a talk step and a key-keeper
    /// riddle over the same empty pack walk, Talk-to and idle exactly as they
    /// did — the intercept never becomes a second classify of them.
    #[test]
    fn search_talk_and_key_rows_never_enter_the_trio_acquire() {
        on_reset();
        let data = selected();
        // The search membership: its own decoded walk, not the professor's.
        let token = steady(&data, SEARCH);
        let walk = call(
            &data,
            token,
            json!([[SEARCH, 1]]),
            json!({ "here": here(3100, 3300, 0) }),
        );
        assert_eq!(walk["kind"], "walk", "{walk}");
        assert_eq!(walk["x"], 3209, "{walk}");
        assert_eq!(walk["z"], 3218, "{walk}");
        // The talk step: its own published spawn, not the professor's tile.
        let spawn = data
            .talk_key()
            .expect("talk_key")
            .talk
            .iter()
            .find(|talk| talk.id == TALK)
            .and_then(|talk| talk.spawn.as_ref())
            .expect("the talk step publishes a spawn");
        let token = steady(&data, TALK);
        let talked = call(
            &data,
            token,
            json!([[TALK, 1]]),
            json!({ "here": here(3100, 3300, 0), "npcs": [] }),
        );
        assert_eq!(talked["kind"], "walk", "{talked}");
        assert_eq!(talked["x"], spawn.x, "{talked}");
        assert_eq!(talked["z"], spawn.z, "{talked}");
        // The key-keeper riddle: the sibling hunt's own published spawn, never
        // the professor's tile and never a trio requirement.
        let spawn = data
            .talk_key()
            .expect("talk_key")
            .keys
            .iter()
            .find(|key| key.id == RIDDLE)
            .and_then(|key| key.spawn.as_ref())
            .expect("the keeper publishes a spawn");
        let token = steady(&data, RIDDLE);
        let hunted = call(
            &data,
            token,
            json!([[RIDDLE, 1]]),
            json!({ "here": here(3100, 3300, 0), "npcs": [] }),
        );
        assert_eq!(hunted["kind"], "walk", "{hunted}");
        assert_eq!(hunted["x"], spawn.x, "{hunted}");
        assert_eq!(hunted["z"], spawn.z, "{hunted}");
    }

    /// The b run's first piece: these tests post boards from that run's own
    /// numbering, so a cell's posted piece id is `PIECE_B + target`.
    const PIECE_B: i32 = 2749;

    /// The component the test boards post.
    const BOARD_COMPONENT: i32 = 6600;

    /// One wrapper-marshalled posted board page plus the session generation
    /// the click rides: a sparse row per filled cell, in slot order, so the
    /// machine's own read is what fills the 25.
    fn board_page(board: &Board, generation: u64) -> Value {
        let mut rows = Vec::new();
        for (slot, cell) in board.iter().enumerate() {
            if let Some(target) = *cell {
                rows.push(json!({ "slot": slot as i32, "id": PIECE_B + i32::from(target) }));
            }
        }
        json!({
            "puzzle_board": {
                "component_id": BOARD_COMPONENT,
                "size": clue_puzzle::PUZZLE_SIZE as i32,
                "items": rows,
            },
            "puzzle_board_generation": generation,
        })
    }

    /// The closed board SNAP posts beside a box that was never opened: a
    /// present object with no component and no rows.
    fn closed_board_page() -> Value {
        json!({
            "puzzle_board": { "component_id": -1, "size": 0, "items": [] },
            "puzzle_board_generation": 0,
        })
    }

    /// The solved board: every piece on its own slot and the gap on 24.
    fn solved_board() -> Board {
        let mut board: Board = [None; clue_puzzle::PUZZLE_SIZE];
        for (slot, cell) in board.iter_mut().enumerate() {
            *cell = (slot != clue_puzzle::PUZZLE_BLANK_SLOT).then_some(slot as u8);
        }
        board
    }

    /// One slide from solved: the piece belonging on 23 stands on the blank
    /// slot, so the frozen plan is the single click on 24.
    fn one_move_board() -> Board {
        let mut board = solved_board();
        board[clue_puzzle::PUZZLE_BLANK_SLOT] = Some(23);
        board[23] = None;
        board
    }

    /// The pack page of a held puzzle step: the desc-only riddle and its own
    /// selected box.
    fn held_box() -> Value {
        json!([[PUZZLE_RIDDLE, 1], [PUZZLE_BOX, 1]])
    }

    #[test]
    fn the_puzzle_join_is_the_rows_own_selected_box() {
        let data = selected();
        assert_eq!(
            puzzle_box(Some(&data), row(&data, PUZZLE_RIDDLE)),
            Some((PUZZLE_BOX, "Puzzle box"))
        );
        // No box of its own: the desc-only riddle with no such item, and the
        // search row whose alias joins to nothing.
        assert_eq!(
            puzzle_box(Some(&data), row(&data, PUZZLE_RIDDLE_NO_BOX)),
            None
        );
        assert_eq!(puzzle_box(Some(&data), row(&data, SEARCH)), None);
        assert_eq!(puzzle_box(None, row(&data, PUZZLE_RIDDLE)), None);
        // Held means the posted page carries a positive count for that id and
        // nothing else.
        assert!(holds(&json!({ "held": held_box() }), PUZZLE_BOX));
        assert!(!holds(&json!({ "held": [[PUZZLE_BOX, 0]] }), PUZZLE_BOX));
        assert!(!holds(&json!({ "held": [[PUZZLE_RIDDLE, 1]] }), PUZZLE_BOX));
        assert!(!holds(&json!({ "held": [] }), PUZZLE_BOX));
        assert!(!holds(&json!({}), PUZZLE_BOX));
    }

    #[test]
    fn a_held_puzzle_box_opens_by_its_selected_name_and_repeats_while_closed() {
        on_reset();
        let data = selected();
        let token = steady(&data, PUZZLE_RIDDLE);
        let opened = call(&data, token, held_box(), closed_board_page());
        assert_eq!(opened["kind"], "held", "{opened}");
        assert_eq!(opened["token"], token, "{opened}");
        assert_eq!(opened["name"], "Puzzle box", "{opened}");
        assert_eq!(opened["action"], "Open", "{opened}");
        // The name is the identity: no row id, no tile and no slot rides along.
        for absent in ["id", "x", "z", "level", "slot"] {
            assert!(opened.get(absent).is_none(), "{absent} {opened}");
        }
        // The first Open armed the frozen open window, and the repeats are
        // page-driven rather than a second arm.
        assert!(bound_armed(), "{opened}");
        let repeated = call(&data, token, held_box(), closed_board_page());
        assert_eq!(repeated["kind"], "held", "{repeated}");
        // The window runs out: the attempt ends with no closer verb, because
        // there is no board to close.
        force_bound();
        let expired = call(&data, token, held_box(), closed_board_page());
        assert_eq!(expired["kind"], "wait", "{expired}");
        let latched = call(&data, token, held_box(), closed_board_page());
        assert_eq!(latched["kind"], "wait", "{latched}");
    }

    #[test]
    fn a_readable_board_dispatches_one_puzzle_move_from_the_posted_row() {
        on_reset();
        let data = selected();
        let token = steady(&data, PUZZLE_RIDDLE);
        let opened = call(&data, token, held_box(), closed_board_page());
        assert_eq!(opened["kind"], "held", "{opened}");
        let moved = call(&data, token, held_box(), board_page(&one_move_board(), 7));
        assert_eq!(moved["kind"], "puzzle-move", "{moved}");
        assert_eq!(moved["token"], token, "{moved}");
        // The posted row's own identity and this call's board session: the
        // piece standing on the plan's slot, that slot, the posted component
        // and the posted generation.
        assert_eq!(moved["id"], PIECE_B + 23, "{moved}");
        assert_eq!(moved["slot"], 24, "{moved}");
        assert_eq!(moved["component"], BOARD_COMPONENT, "{moved}");
        assert_eq!(moved["generation"], 7, "{moved}");
        assert!(moved.get("name").is_none(), "{moved}");
        assert!(moved.get("action").is_none(), "{moved}");
    }

    #[test]
    fn a_landed_move_closes_the_solved_board_and_never_closes_it_twice() {
        on_reset();
        let data = selected();
        let token = steady(&data, PUZZLE_RIDDLE);
        let opened = call(&data, token, held_box(), closed_board_page());
        assert_eq!(opened["kind"], "held", "{opened}");
        let moved = call(&data, token, held_box(), board_page(&one_move_board(), 7));
        assert_eq!(moved["kind"], "puzzle-move", "{moved}");
        // The live board is the one that click was expected to produce, and it
        // is the solved one: the frozen `isPuzzleSolved` read closes it.
        let closed = call(&data, token, held_box(), board_page(&solved_board(), 7));
        assert_eq!(closed["kind"], "close-modal", "{closed}");
        assert_eq!(closed["token"], token, "{closed}");
        // The close landed: the board left the page and the step idles. A
        // solved board posted again is not a second close either.
        let gone = call(&data, token, held_box(), closed_board_page());
        assert_eq!(gone["kind"], "wait", "{gone}");
        let reposted = call(&data, token, held_box(), board_page(&solved_board(), 7));
        assert_eq!(reposted["kind"], "wait", "{reposted}");
        // A different held row re-arms the step, and `clear_step` drops the
        // latch with it: back on the riddle the box opens again.
        let other = call(&data, token, json!([[SEARCH, 1]]), json!({}));
        assert_eq!(other["kind"], "callback.enabled", "{other}");
        let gate = call(&data, token, held_box(), json!({}));
        assert_eq!(gate["kind"], "callback.enabled", "{gate}");
        let logged = call(&data, token, held_box(), json!({ "resume": true }));
        assert_eq!(logged["kind"], "callback.log", "{logged}");
        let posted = call(&data, token, held_box(), json!({}));
        assert_eq!(posted["kind"], "callback.setStatus", "{posted}");
        let reopened = call(&data, token, held_box(), closed_board_page());
        assert_eq!(reopened["kind"], "held", "{reopened}");
    }

    #[test]
    fn a_solved_board_the_token_never_opened_is_still_closed() {
        on_reset();
        let data = selected();
        let token = steady(&data, PUZZLE_RIDDLE);
        // The box is held and the board is already solved: the frozen `puzzle
        // already solved` path closes it rather than opening anything.
        let closed = call(&data, token, held_box(), board_page(&solved_board(), 3));
        assert_eq!(closed["kind"], "close-modal", "{closed}");
        let gone = call(&data, token, held_box(), closed_board_page());
        assert_eq!(gone["kind"], "wait", "{gone}");
    }

    #[test]
    fn a_board_that_goes_unreadable_mid_solve_closes_and_latches() {
        on_reset();
        let data = selected();
        let token = steady(&data, PUZZLE_RIDDLE);
        let opened = call(&data, token, held_box(), closed_board_page());
        assert_eq!(opened["kind"], "held", "{opened}");
        let moved = call(&data, token, held_box(), board_page(&one_move_board(), 7));
        assert_eq!(moved["kind"], "puzzle-move", "{moved}");
        // The board went away with the click outstanding, and the modal the
        // Open landed on may still be up: the attempt exits through the same
        // one close as a stall or `MAX_MOVES`.
        let closed = call(&data, token, held_box(), closed_board_page());
        assert_eq!(closed["kind"], "close-modal", "{closed}");
        assert_eq!(closed["token"], token, "{closed}");
        // The close is out: neither the closed page it leaves nor a board
        // posted again draws a second close, and the window is what is left of
        // the exit.
        for page in [closed_board_page(), board_page(&one_move_board(), 7)] {
            let waited = call(&data, token, held_box(), page);
            assert_eq!(waited["kind"], "wait", "{waited}");
        }
        force_bound();
        let latched = call(&data, token, held_box(), closed_board_page());
        assert_eq!(latched["kind"], "wait", "{latched}");
        // The solved-or-attempted latch holds: a readable board posted again is
        // not a second attempt and not a second close.
        let reposted = call(&data, token, held_box(), board_page(&solved_board(), 7));
        assert_eq!(reposted["kind"], "wait", "{reposted}");
    }

    #[test]
    fn a_still_open_board_the_map_cannot_place_takes_the_same_close() {
        on_reset();
        let data = selected();
        let token = steady(&data, PUZZLE_RIDDLE);
        let opened = call(&data, token, held_box(), closed_board_page());
        assert_eq!(opened["kind"], "held", "{opened}");
        let moved = call(&data, token, held_box(), board_page(&one_move_board(), 7));
        assert_eq!(moved["kind"], "puzzle-move", "{moved}");
        // The page still posts a live size-25 board, but one piece the selected
        // map cannot place: the board is up and unreadable, so the same close
        // goes out rather than a bare latch with the modal open.
        let mut unplaceable = board_page(&one_move_board(), 7);
        unplaceable["puzzle_board"]["items"][0]["id"] = json!(PIECE_B + 99);
        let closed = call(&data, token, held_box(), unplaceable);
        assert_eq!(closed["kind"], "close-modal", "{closed}");
        let waited = call(&data, token, held_box(), closed_board_page());
        assert_eq!(waited["kind"], "wait", "{waited}");
        force_bound();
        let latched = call(&data, token, held_box(), closed_board_page());
        assert_eq!(latched["kind"], "wait", "{latched}");
    }

    #[test]
    fn a_settle_bound_that_runs_out_unlanded_replans_from_the_live_board() {
        on_reset();
        let data = selected();
        let token = steady(&data, PUZZLE_RIDDLE);
        let opened = call(&data, token, held_box(), closed_board_page());
        assert_eq!(opened["kind"], "held", "{opened}");
        let moved = call(&data, token, held_box(), board_page(&one_move_board(), 7));
        assert_eq!(moved["kind"], "puzzle-move", "{moved}");
        // The click is out and the board has not moved: sent is not observed,
        // so this call waits the frozen window out.
        let unsettled = call(&data, token, held_box(), board_page(&one_move_board(), 7));
        assert_eq!(unsettled["kind"], "wait", "{unsettled}");
        force_bound();
        let stalled = call(&data, token, held_box(), board_page(&one_move_board(), 7));
        assert_eq!(stalled["kind"], "wait", "{stalled}");
        // The refusal is counted and the next call plans again from the board
        // it reads — the same one-move board, so the same click.
        let replanned = call(&data, token, held_box(), board_page(&one_move_board(), 7));
        assert_eq!(replanned["kind"], "puzzle-move", "{replanned}");
        assert_eq!(replanned["slot"], 24, "{replanned}");
        assert_eq!(replanned["id"], PIECE_B + 23, "{replanned}");
    }

    #[test]
    fn a_board_no_plan_exists_for_stalls_to_the_close() {
        on_reset();
        let data = selected();
        let token = steady(&data, PUZZLE_RIDDLE);
        let opened = call(&data, token, held_box(), closed_board_page());
        assert_eq!(opened["kind"], "held", "{opened}");
        // A mixed picture set: two pieces for one target and none for another
        // is not a valid board, so the frozen solver has no plan for it.
        let mut mixed = one_move_board();
        mixed[6] = Some(5);
        let page = board_page(&mixed, 7);
        for struck in 1..STALL_LIMIT {
            let stalled = call(&data, token, held_box(), page.clone());
            assert_eq!(stalled["kind"], "wait", "{struck} {stalled}");
        }
        let closed = call(&data, token, held_box(), page);
        assert_eq!(closed["kind"], "close-modal", "{closed}");
        let latched = call(&data, token, held_box(), closed_board_page());
        assert_eq!(latched["kind"], "wait", "{latched}");
    }

    #[test]
    fn a_multi_move_board_clicks_one_plan_slot_per_call_until_it_closes() {
        on_reset();
        let data = selected();
        let token = steady(&data, PUZZLE_RIDDLE);
        let opened = call(&data, token, held_box(), closed_board_page());
        assert_eq!(opened["kind"], "held", "{opened}");
        // Two slides from solved: the piece belonging on 18 stands on 23, the
        // one belonging on 23 on the blank slot, and the gap is on 18.
        let mut board = solved_board();
        board[18] = None;
        board[23] = Some(18);
        board[clue_puzzle::PUZZLE_BLANK_SLOT] = Some(23);
        let mut clicked = Vec::new();
        loop {
            let step = call(&data, token, held_box(), board_page(&board, 7));
            if step["kind"] == "close-modal" {
                break;
            }
            assert_eq!(step["kind"], "puzzle-move", "{step}");
            // The plan's own first slot, applied to the live board: the next
            // call is handed the page that click produced. One verb per call,
            // and the click is the posted row standing on that slot.
            let slot = usize::try_from(step["slot"].as_u64().expect("slot")).expect("slot");
            assert_eq!(
                step["id"],
                PIECE_B + i32::from(board[slot].expect("piece")),
                "{step}"
            );
            assert!(clue_puzzle::apply_puzzle_move(&mut board, slot), "{step}");
            clicked.push(step["slot"].as_i64().expect("slot"));
            assert!(clicked.len() < STALL_LIMIT as usize, "{step}");
        }
        assert_eq!(clicked, vec![23, 24], "the frozen plan, one click per call");
        assert!(clue_puzzle::is_puzzle_solved(&board));
        // The close landed: the step idles from here.
        let gone = call(&data, token, held_box(), closed_board_page());
        assert_eq!(gone["kind"], "wait", "{gone}");
    }

    #[test]
    fn freeze_and_yield_beat_the_puzzle_arm() {
        on_reset();
        let data = selected();
        let token = steady(&data, PUZZLE_RIDDLE);
        // Frozen: no Open, no click and no close, and the open window is not
        // spent by the frozen call.
        on_pause();
        let paused = call(&data, token, held_box(), closed_board_page());
        assert_eq!(paused["kind"], "wait", "{paused}");
        assert!(!bound_armed(), "a frozen call spends nothing: {paused}");
        on_resume();
        let opened = call(&data, token, held_box(), closed_board_page());
        assert_eq!(opened["kind"], "held", "{opened}");
        // The posted `hold || ours` interrupt, unfrozen: yield, and no verb
        // rides along with it. The click still goes out on the next call.
        on_hold(true);
        let held_clock = call(&data, token, held_box(), board_page(&one_move_board(), 7));
        assert_eq!(held_clock["kind"], "wait", "{held_clock}");
        on_hold(false);
        let mut yield_page = board_page(&one_move_board(), 7);
        yield_page["hold"] = json!(true);
        let yielded = call(&data, token, held_box(), yield_page);
        assert_eq!(yielded["kind"], "yield", "{yielded}");
        let moved = call(&data, token, held_box(), board_page(&one_move_board(), 7));
        assert_eq!(moved["kind"], "puzzle-move", "{moved}");
    }

    #[test]
    fn rows_without_their_own_held_box_keep_their_own_arm() {
        on_reset();
        let data = selected();
        // A desc-only row with no box of its own, with another row's box in
        // the pack: identified, then idle.
        let page = json!([[PUZZLE_RIDDLE_NO_BOX, 1], [PUZZLE_BOX, 1]]);
        let token = steady(&data, PUZZLE_RIDDLE_NO_BOX);
        let idle = call(&data, token, page.clone(), board_page(&solved_board(), 7));
        assert_eq!(idle["kind"], "wait", "{idle}");
        // The riddle's own box is not on this page: not a puzzle step either,
        // so the solved board it posts is not closed by this arm.
        let token = steady(&data, PUZZLE_RIDDLE);
        let unheld = call(
            &data,
            token,
            json!([[PUZZLE_RIDDLE, 1]]),
            board_page(&solved_board(), 7),
        );
        assert_eq!(unheld["kind"], "wait", "{unheld}");
        // A search row keeps the search arm with a box sitting in the pack:
        // the join is the row's own alias, so no box steals it.
        let token = steady(&data, SEARCH);
        let walked = call(
            &data,
            token,
            json!([[SEARCH, 1], [PUZZLE_BOX, 1]]),
            json!({ "here": here(3100, 3300, 1) }),
        );
        assert_eq!(walked["kind"], "walk", "{walked}");
    }

    /// `trail_clue_hard_riddle019`: the second puzzle riddle whose own selected
    /// `trail_clue_hard_riddle019_puzzlebox` item is held, and one of the five
    /// identity-only talk steps — `Examiner`, packed id 618, with no published
    /// spawn — so its latched fall-through takes the nearest posted npc of that
    /// identity.
    const PUZZLE_RIDDLE_IDENTITY: i32 = 3566;
    /// That box item, display `Puzzle box`.
    const PUZZLE_BOX_IDENTITY: i32 = 3567;
    /// The exemplar riddle014's own talk identity: `Oziach`, packed id 747, at
    /// the published `(3069, 3517, plane 0)`.
    const OZIACH_ID: i32 = 747;
    const OZIACH_NAME: &str = "Oziach";
    /// riddle019's identity: `Examiner`, packed id 618, no unique spawn.
    const EXAMINER_ID: i32 = 618;
    const EXAMINER_NAME: &str = "Examiner";

    /// The published spawn tile one selected talk step's walk carries, read
    /// from the family rather than copied.
    fn talk_tile(data: &SelectedGameData, id: i32) -> Tile {
        let spawn = talk_of(data, id)
            .spawn
            .as_ref()
            .expect("a unique jm2 spawn");
        Tile {
            x: spawn.x,
            z: spawn.z,
            level: spawn.plane,
        }
    }

    /// The latched puzzle riddle re-talks: once the solved-or-attempted latch is
    /// set, the `Steady` dispatch skips the box's own arm and the row's own
    /// talk step runs the landed walk-then-Talk-to. The latch-arming call is
    /// still the puzzle arm's own `wait`; the fall-through is the next live call.
    #[test]
    fn a_latched_puzzle_riddle_re_talks_over_the_landed_talk_step() {
        on_reset();
        let data = selected();
        let token = steady(&data, PUZZLE_RIDDLE);
        let opened = call(&data, token, held_box(), closed_board_page());
        assert_eq!(opened["kind"], "held", "{opened}");
        // The board never opened: the frozen open window runs out and latches
        // the step with no closer verb.
        force_bound();
        let latched = call(&data, token, held_box(), closed_board_page());
        assert_eq!(latched["kind"], "wait", "{latched}");
        let step = talk_of(&data, PUZZLE_RIDDLE);
        assert_eq!(step.id, PUZZLE_RIDDLE, "{}", step.alias);
        assert_eq!(
            (step.npc.id, step.npc.name.as_str()),
            (OZIACH_ID, OZIACH_NAME)
        );
        let tile = talk_tile(&data, PUZZLE_RIDDLE);
        // No posted npc page: no invented target and no walk, with the token
        // still live.
        let blind = call(
            &data,
            token,
            held_box(),
            json!({ "here": here(tile.x, tile.z, tile.level) }),
        );
        assert_eq!(blind["kind"], "wait", "{blind}");
        assert_eq!(token_of(&blind), token, "{blind}");
        // Posted far from the published tile: the walk is that tile, with the
        // published `plane` as the verb's `level`.
        let walked = call(
            &data,
            token,
            held_box(),
            talk_scene(here(tile.x - 30, tile.z, tile.level), json!([]), json!({})),
        );
        assert_eq!(walked["kind"], "walk", "{walked}");
        assert_eq!(
            (
                walked["x"].as_i64(),
                walked["z"].as_i64(),
                walked["level"].as_i64()
            ),
            (
                Some(i64::from(tile.x)),
                Some(i64::from(tile.z)),
                Some(i64::from(tile.level))
            ),
            "{walked}"
        );
        // Arrived with the step's own npc posted on the tile: the landed
        // Talk-to, carrying the posted name and the posted scene index.
        let npcs = json!([talk_npc(3, OZIACH_ID, OZIACH_NAME, tile, 1, &["Talk-to"])]);
        let talked = call(
            &data,
            token,
            held_box(),
            talk_scene(here(tile.x, tile.z, tile.level), npcs.clone(), json!({})),
        );
        assert_eq!(talked["kind"], "npc", "{talked}");
        assert_eq!(talked["name"], OZIACH_NAME, "{talked}");
        assert_eq!(talked["action"], "Talk-to", "{talked}");
        assert_eq!(talked["index"], 3, "{talked}");
        // The still-held solved board is never a second close and the box is
        // never re-Opened: the latched step takes the talk arm, not the board.
        let again = call(
            &data,
            token,
            held_box(),
            talk_scene(
                here(tile.x, tile.z, tile.level),
                npcs,
                board_page(&solved_board(), 7),
            ),
        );
        assert_eq!(again["kind"], "npc", "{again}");
    }

    /// The fall-through is either-way: a stalled attempt that solved nothing
    /// re-talks exactly like a solved board, because the trigger is the latch
    /// and never the solved read.
    #[test]
    fn a_stalled_puzzle_riddle_re_talks_the_same_way() {
        on_reset();
        let data = selected();
        let token = steady(&data, PUZZLE_RIDDLE);
        let opened = call(&data, token, held_box(), closed_board_page());
        assert_eq!(opened["kind"], "held", "{opened}");
        // A mixed picture set: the frozen solver has no plan for it, so the
        // consecutive refusals run out and the exit close goes out.
        let mut mixed = one_move_board();
        mixed[6] = Some(5);
        let page = board_page(&mixed, 7);
        for struck in 1..STALL_LIMIT {
            let stalled = call(&data, token, held_box(), page.clone());
            assert_eq!(stalled["kind"], "wait", "{struck} {stalled}");
        }
        let closed = call(&data, token, held_box(), page);
        assert_eq!(closed["kind"], "close-modal", "{closed}");
        force_bound();
        let latched = call(&data, token, held_box(), closed_board_page());
        assert_eq!(latched["kind"], "wait", "{latched}");
        let tile = talk_tile(&data, PUZZLE_RIDDLE);
        let npcs = json!([talk_npc(3, OZIACH_ID, OZIACH_NAME, tile, 1, &["Talk-to"])]);
        let talked = call(
            &data,
            token,
            held_box(),
            talk_scene(here(tile.x, tile.z, tile.level), npcs, json!({})),
        );
        assert_eq!(talked["kind"], "npc", "{talked}");
        assert_eq!(talked["index"], 3, "{talked}");
    }

    /// The frozen close window is not the exclusion: `latch` is unset while the
    /// one close is out, so a posted talk scene draws nothing until the window
    /// ends. The call that arms the latch still waits inside the puzzle arm,
    /// and the next live call talks although `closing` is never cleared.
    #[test]
    fn a_latched_puzzle_riddle_never_talks_inside_the_close_window() {
        on_reset();
        let data = selected();
        let token = steady(&data, PUZZLE_RIDDLE);
        let opened = call(&data, token, held_box(), closed_board_page());
        assert_eq!(opened["kind"], "held", "{opened}");
        let moved = call(&data, token, held_box(), board_page(&one_move_board(), 7));
        assert_eq!(moved["kind"], "puzzle-move", "{moved}");
        let closed = call(&data, token, held_box(), board_page(&solved_board(), 7));
        assert_eq!(closed["kind"], "close-modal", "{closed}");
        let tile = talk_tile(&data, PUZZLE_RIDDLE);
        let npcs = json!([talk_npc(3, OZIACH_ID, OZIACH_NAME, tile, 1, &["Talk-to"])]);
        let scene = talk_scene(here(tile.x, tile.z, tile.level), npcs, json!({}));
        // The close is out and its window is running: no walk and no Talk-to,
        // whatever the page posts.
        for waited in 0..2 {
            let waiting = call(&data, token, held_box(), scene.clone());
            assert_eq!(waiting["kind"], "wait", "{waited} {waiting}");
        }
        // The window ends: this call arms the latch and waits, and the next
        // live call is the landed talk step.
        force_bound();
        let latched = call(&data, token, held_box(), scene.clone());
        assert_eq!(latched["kind"], "wait", "{latched}");
        let talked = call(&data, token, held_box(), scene);
        assert_eq!(talked["kind"], "npc", "{talked}");
        assert_eq!(talked["index"], 3, "{talked}");
    }

    /// The identity-only latched riddle re-talks the same landed arm: no
    /// published spawn, so the posted npc of the step's own identity is what is
    /// walked to and Talked-to — and a page that posts none of them is a `wait`.
    /// No Examiner, tile or target is invented for it.
    #[test]
    fn a_latched_identity_only_puzzle_riddle_waits_without_its_own_npc() {
        on_reset();
        let data = selected();
        let token = steady(&data, PUZZLE_RIDDLE_IDENTITY);
        let page = json!([[PUZZLE_RIDDLE_IDENTITY, 1], [PUZZLE_BOX_IDENTITY, 1]]);
        let opened = call(&data, token, page.clone(), closed_board_page());
        assert_eq!(opened["kind"], "held", "{opened}");
        force_bound();
        let latched = call(&data, token, page.clone(), closed_board_page());
        assert_eq!(latched["kind"], "wait", "{latched}");
        let step = talk_of(&data, PUZZLE_RIDDLE_IDENTITY);
        assert_eq!(step.id, PUZZLE_RIDDLE_IDENTITY, "{}", step.alias);
        assert!(step.spawn.is_none(), "{}", step.alias);
        assert_eq!(
            (step.npc.id, step.npc.name.as_str()),
            (EXAMINER_ID, EXAMINER_NAME)
        );
        let here_tile = Tile {
            x: 3207,
            z: 3233,
            level: 0,
        };
        // No posted npc page at all, and an empty one: a wait with the token
        // live, never an invented Examiner.
        for scene in [
            json!({}),
            talk_scene(
                here(here_tile.x, here_tile.z, here_tile.level),
                json!([]),
                json!({}),
            ),
        ] {
            let waited = call(&data, token, page.clone(), scene);
            assert_eq!(waited["kind"], "wait", "{waited}");
            assert_eq!(token_of(&waited), token, "{waited}");
        }
        // Another identity posted on the tile is not this step's npc either.
        let other = json!([talk_npc(1, 0, "Hans", here_tile, 0, &["Talk-to"])]);
        let unmatched = call(
            &data,
            token,
            page.clone(),
            talk_scene(
                here(here_tile.x, here_tile.z, here_tile.level),
                other,
                json!({}),
            ),
        );
        assert_eq!(unmatched["kind"], "wait", "{unmatched}");
        // The identity-only rule: a posted Examiner out of reach is walked to
        // at its own posted tile, which is the only tile this arm has.
        let posted_tile = Tile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let far = json!([talk_npc(
            4,
            EXAMINER_ID,
            EXAMINER_NAME,
            posted_tile,
            5,
            &["Talk-to"]
        )]);
        let walked = call(
            &data,
            token,
            page.clone(),
            talk_scene(
                here(here_tile.x, here_tile.z, here_tile.level),
                far,
                json!({}),
            ),
        );
        assert_eq!(walked["kind"], "walk", "{walked}");
        assert_eq!(
            (walked["x"].as_i64(), walked["z"].as_i64()),
            (
                Some(i64::from(posted_tile.x)),
                Some(i64::from(posted_tile.z))
            ),
            "{walked}"
        );
        // Arrived there with its own identity posted: the landed Talk-to.
        let near = json!([talk_npc(
            4,
            EXAMINER_ID,
            EXAMINER_NAME,
            posted_tile,
            0,
            &["Talk-to"]
        )]);
        let talked = call(
            &data,
            token,
            page,
            talk_scene(
                here(posted_tile.x, posted_tile.z, posted_tile.level),
                near,
                json!({}),
            ),
        );
        assert_eq!(talked["kind"], "npc", "{talked}");
        assert_eq!(talked["name"], EXAMINER_NAME, "{talked}");
        assert_eq!(talked["index"], 4, "{talked}");
    }

    /// `trail_clue_easy_simple005`: a talk membership whose jm2 spawn is unique
    /// — `hans` at `(3207, 3233, plane 0)`.
    const TALK: i32 = 2681;
    /// `trail_clue_hard_riddle025`: the talk step that publishes a plane-1
    /// spawn — `Heckel Funch` at `(2493, 3488, plane 1)`. The published plane
    /// is the walk verb's `level`, and it is the only place a level comes from.
    const TALK_PLANE: i32 = 3575;
    /// `trail_clue_easy_simple008`: an identity-only talk step. Its jm2 spawn
    /// is not unique, so the family publishes no tile and the arm takes the
    /// nearest posted npc of its own identity — `Tanner`, packed id 804.
    const TALK_IDENTITY: i32 = 2684;
    /// `trail_clue_medium_anagram001`: the talk step that is also a challenge
    /// parent — `Hazelmere`, whose `2842` scroll answers `"6859"`.
    const TALK_CHALLENGE: i32 = 2841;
    /// That scroll.
    const CHALLENGE: i32 = 2842;
    /// `trail_clue_medium_anagram003`: the identity-only challenge parent —
    /// Zoo keeper, whose `2846` scroll answers `"40"`.
    const TALK_CHALLENGE_IDENTITY: i32 = 2845;
    /// That scroll.
    const CHALLENGE_IDENTITY: i32 = 2846;

    /// One wrapper-marshalled posted npc row as the talk arm reads it: the
    /// posted index the Talk-to carries, the packed id and posted display name
    /// the identity join compares, the posted tile and distance the arrival is
    /// measured by, and the posted action list the talk action is read from.
    fn talk_npc(
        index: i32,
        id: i32,
        name: &str,
        tile: Tile,
        distance: i32,
        actions: &[&str],
    ) -> Value {
        json!({
            "index": index,
            "id": id,
            "name": name,
            "x": tile.x,
            "z": tile.z,
            "level": tile.level,
            "distance": distance,
            "health": 0,
            "max_health": 0,
            "in_combat": false,
            "actions": actions,
            "target_kind": 0,
            "target_index": -1,
        })
    }

    /// One talk call's pages: the posted `here` tile and the posted npc page,
    /// plus whatever else the call under test needs — the posted chat slots,
    /// the posted count dialog, the posted hitpoints.
    fn talk_scene(here_tile: Value, npcs: Value, extra: Value) -> Value {
        let mut scene = json!({ "here": here_tile, "npcs": npcs });
        for (key, value) in extra.as_object().expect("extra") {
            scene[key] = value.clone();
        }
        scene
    }

    /// The selected talk step these tests read.
    fn talk_of(data: &SelectedGameData, id: i32) -> &TalkKeyTalkRow {
        talk_step(Some(data), id).unwrap_or_else(|| panic!("talk step {id}"))
    }

    /// The held page of one talk step.
    fn talk_page(id: i32) -> Value {
        json!([[id, 1]])
    }

    /// The talk membership is the selected `talk_key.talk` family and nothing
    /// else, and no talk step is a second classify of the landed arms: the
    /// seven key keepers, the desc-only rows the family does not publish and
    /// every search, guarded, dig and casket membership stay out.
    #[test]
    fn the_talk_membership_is_the_selected_talk_family_and_nothing_else() {
        on_reset();
        let data = selected();
        let talk = data.talk_key().expect("talk_key");
        assert_eq!(talk.talk.len(), 47, "the landed family");
        assert_eq!(
            talk.talk.iter().filter(|row| row.spawn.is_some()).count(),
            42,
            "the unique-spawn slice"
        );
        assert_eq!(
            talk.talk.iter().filter(|row| row.spawn.is_none()).count(),
            5,
            "the identity-only slice"
        );
        for step in &talk.talk {
            let row = row(&data, step.id);
            assert_eq!(
                talk_step(Some(&data), step.id).map(|talk| talk.id),
                Some(step.id),
                "{}",
                step.alias
            );
            // The talk arm is the last `Steady` arm, so a talk step is never
            // claimed by the search, guarded, unguarded-dig or casket classify.
            assert_eq!(search_tile(row), None, "{}", step.alias);
            assert_eq!(guarded_tile(row), None, "{}", step.alias);
            assert_eq!(dig_tile(row), None, "{}", step.alias);
            assert_eq!(casket_name(Some(&data), row), None, "{}", step.alias);
            // Every talk membership is a clue row the landed identify returns.
            assert_eq!(row.role, "clue", "{}", step.alias);
        }
        // The key keepers are not talk membership, and neither is any row the
        // family does not publish.
        for key in &talk.keys {
            assert_eq!(
                talk_step(Some(&data), key.id).map(|talk| talk.id),
                None,
                "{}",
                key.alias
            );
        }
        for id in [MAP_EMPTY, RIDDLE, SEARCH, UNGUARDED, GUARDED, CLUE, CASKET] {
            assert_eq!(talk_step(Some(&data), id).map(|talk| talk.id), None, "{id}");
        }
        // No selected pin is no talk step at all.
        assert_eq!(talk_step(None, TALK).map(|talk| talk.id), None);
    }

    /// The unique-spawn arm: the walk is the published `{x, z, plane}` and the
    /// Talk-to is only ever a posted npc of this step's identity standing on
    /// that tile. A wanderer, another identity, a wrong level and a row with no
    /// talk action all wait at the tile.
    #[test]
    fn a_unique_spawn_talk_step_walks_to_the_published_tile_and_then_talks() {
        on_reset();
        let data = selected();
        let step = talk_of(&data, TALK);
        assert_eq!(step.npc.id, 0, "{}", step.alias);
        assert_eq!(step.npc.name, "Hans", "{}", step.alias);
        let spawn = step.spawn.as_ref().expect("a unique jm2 spawn");
        assert_eq!((spawn.x, spawn.z, spawn.plane), (3207, 3233, 0));

        let page = talk_page(TALK);
        let token = steady(&data, TALK);

        // No posted `here`: there is no arrival claim to make, so this tick
        // waits rather than walking blind.
        let blind = call(&data, token, page.clone(), json!({ "npcs": [] }));
        assert_eq!(blind["kind"], "wait", "{blind}");
        assert_eq!(token_of(&blind), token, "{blind}");

        // Posted far from the tile: the walk is the published tile, and the
        // `plane` is the verb's own `level`.
        let walked = call(
            &data,
            token,
            page.clone(),
            talk_scene(here(3200, 3233, 0), json!([]), json!({})),
        );
        assert_eq!(walked["kind"], "walk", "{walked}");
        assert_eq!(walked["x"], spawn.x, "{walked}");
        assert_eq!(walked["z"], spawn.z, "{walked}");
        assert_eq!(walked["level"], spawn.plane, "{walked}");

        // Arrived, with the step's own npc posted on the tile: the posted name
        // and posted action ride the verb with the posted scene index.
        let on_tile = json!([talk_npc(
            11,
            0,
            "Hans",
            Tile {
                x: 3207,
                z: 3233,
                level: 0
            },
            1,
            &["Talk-to"]
        )]);
        let talked = call(
            &data,
            token,
            page.clone(),
            talk_scene(here(3207, 3233, 0), on_tile.clone(), json!({})),
        );
        assert_eq!(talked["kind"], "npc", "{talked}");
        assert_eq!(talked["name"], "Hans", "{talked}");
        assert_eq!(talked["action"], "Talk-to", "{talked}");
        assert_eq!(talked["index"], 11, "{talked}");
        assert_eq!(token_of(&talked), token, "{talked}");

        // Identity is the packed id first: a posted row that carries it keeps
        // the page's own display name on the verb.
        let by_id = json!([talk_npc(
            12,
            0,
            "Someone Else",
            Tile {
                x: 3208,
                z: 3233,
                level: 0,
            },
            1,
            &["Talk-to"]
        )]);
        let renamed = call(
            &data,
            token,
            page.clone(),
            talk_scene(here(3207, 3233, 0), by_id, json!({})),
        );
        assert_eq!(renamed["kind"], "npc", "{renamed}");
        assert_eq!(renamed["name"], "Someone Else", "{renamed}");
        assert_eq!(renamed["index"], 12, "{renamed}");

        // The frozen `talk_op` rule: the first posted action whose first four
        // characters are `talk`, emitted as the page posted it.
        let plain = json!([talk_npc(
            13,
            0,
            "Hans",
            Tile {
                x: 3207,
                z: 3233,
                level: 0,
            },
            1,
            &["Examine", "Talk"]
        )]);
        let action = call(
            &data,
            token,
            page.clone(),
            talk_scene(here(3207, 3233, 0), plain, json!({})),
        );
        assert_eq!(action["kind"], "npc", "{action}");
        assert_eq!(action["action"], "Talk", "{action}");

        // A wanderer four tiles off the published tile is not this step's npc:
        // the arm keeps the tile and waits — no walk, no npc and no Clear.
        let wandered = json!([talk_npc(
            14,
            0,
            "Hans",
            Tile {
                x: 3211,
                z: 3233,
                level: 0
            },
            4,
            &["Talk-to"]
        )]);
        let waited = call(
            &data,
            token,
            page.clone(),
            talk_scene(here(3207, 3233, 0), wandered, json!({})),
        );
        assert_eq!(waited["kind"], "wait", "{waited}");
        assert!(waited.get("x").is_none(), "{waited}");

        // Another identity on the tile is not this step's either, whatever it
        // is called and however close it stands.
        let other = json!([talk_npc(
            15,
            541,
            "Zeke",
            Tile {
                x: 3207,
                z: 3233,
                level: 0
            },
            1,
            &["Talk-to"]
        )]);
        let stranger = call(
            &data,
            token,
            page.clone(),
            talk_scene(here(3207, 3233, 0), other, json!({})),
        );
        assert_eq!(stranger["kind"], "wait", "{stranger}");

        // No talk action on the posted row: not a row this arm dispatches at.
        let silent = json!([talk_npc(
            16,
            0,
            "Hans",
            Tile {
                x: 3207,
                z: 3233,
                level: 0
            },
            1,
            &["Examine"]
        )]);
        let no_action = call(
            &data,
            token,
            page.clone(),
            talk_scene(here(3207, 3233, 0), silent, json!({})),
        );
        assert_eq!(no_action["kind"], "wait", "{no_action}");

        // Same level is part of the membership: the same npc posted one level
        // up is not the one this Dig-free spawn owns.
        let above = json!([talk_npc(
            17,
            0,
            "Hans",
            Tile {
                x: 3207,
                z: 3233,
                level: 1
            },
            1,
            &["Talk-to"]
        )]);
        let leveled = call(
            &data,
            token,
            page.clone(),
            talk_scene(here(3207, 3233, 0), above, json!({})),
        );
        assert_eq!(leveled["kind"], "wait", "{leveled}");

        // Posted on another level entirely: arrival is same-level, so the arm
        // walks again rather than Talking-to anything.
        let off_level = call(
            &data,
            token,
            page.clone(),
            talk_scene(here(3207, 3233, 1), on_tile, json!({})),
        );
        assert_eq!(off_level["kind"], "walk", "{off_level}");
        assert_eq!(off_level["level"], 0, "{off_level}");

        // The published plane is the walk's level on the plane-1 sibling too.
        let plane = talk_of(&data, TALK_PLANE);
        let spawn = plane.spawn.as_ref().expect("a unique jm2 spawn");
        assert_eq!(spawn.plane, 1, "{}", plane.alias);
        let token = steady(&data, TALK_PLANE);
        let upstairs = call(
            &data,
            token,
            talk_page(TALK_PLANE),
            talk_scene(here(2485, 3488, 1), json!([]), json!({})),
        );
        assert_eq!(upstairs["kind"], "walk", "{upstairs}");
        assert_eq!(upstairs["level"], 1, "{upstairs}");
        assert_eq!(upstairs["x"], spawn.x, "{upstairs}");
        assert_eq!(upstairs["z"], spawn.z, "{upstairs}");
    }

    /// The identity-only arm: the nearest posted npc of this step's own
    /// identity, walked to and then Talk-to'd, with no alias, no first-in-file
    /// row and no frozen tile anywhere in the pick.
    #[test]
    fn an_identity_only_talk_step_picks_the_nearest_posted_match() {
        on_reset();
        let data = selected();
        let step = talk_of(&data, TALK_IDENTITY);
        assert!(step.spawn.is_none(), "{}", step.alias);
        assert_eq!(step.npc.id, 804, "{}", step.alias);
        assert_eq!(step.npc.name, "Tanner", "{}", step.alias);

        let page = talk_page(TALK_IDENTITY);
        let token = steady(&data, TALK_IDENTITY);

        // No posted match at all: a wait, and the token stays live.
        let none = call(
            &data,
            token,
            page.clone(),
            talk_scene(here(3200, 3200, 0), json!([]), json!({})),
        );
        assert_eq!(none["kind"], "wait", "{none}");
        assert_eq!(token_of(&none), token, "{none}");

        // No posted `here`: no distance to measure a posted row by, so the arm
        // waits instead of inventing a base.
        let unmeasured = call(
            &data,
            token,
            page.clone(),
            json!({ "npcs": [talk_npc(21, 804, "Tanner", Tile { x: 3200, z: 3204, level: 0 }, 1, &["Talk-to"])] }),
        );
        assert_eq!(unmeasured["kind"], "wait", "{unmeasured}");

        // Two rows of this identity, farthest posted first: the nearest wins
        // and the walk goes to that row's own tile, never the first in file.
        let near = talk_npc(
            22,
            804,
            "Tanner",
            Tile {
                x: 3200,
                z: 3205,
                level: 0,
            },
            5,
            &["Talk-to"],
        );
        let far = talk_npc(
            23,
            804,
            "Tanner",
            Tile {
                x: 3300,
                z: 3300,
                level: 0,
            },
            30,
            &["Talk-to"],
        );
        let pick = call(
            &data,
            token,
            page.clone(),
            talk_scene(here(3200, 3200, 0), json!([far, near]), json!({})),
        );
        assert_eq!(pick["kind"], "walk", "{pick}");
        assert_eq!(pick["x"], 3200, "{pick}");
        assert_eq!(pick["z"], 3205, "{pick}");
        assert_eq!(pick["level"], 0, "{pick}");

        // Arrived: the nearest posted row is inside the frozen radius, so the
        // verb is the Talk-to with that row's own posted identity.
        let arrived = call(
            &data,
            token,
            page.clone(),
            talk_scene(
                here(3200, 3205, 0),
                json!([
                    talk_npc(
                        24,
                        999,
                        "Tanner",
                        Tile {
                            x: 3200,
                            z: 3206,
                            level: 0
                        },
                        3,
                        &["Talk-to"]
                    ),
                    talk_npc(
                        25,
                        804,
                        "Tanner",
                        Tile {
                            x: 3200,
                            z: 3205,
                            level: 0
                        },
                        1,
                        &["Talk-to"]
                    ),
                ]),
                json!({}),
            ),
        );
        assert_eq!(arrived["kind"], "npc", "{arrived}");
        assert_eq!(arrived["name"], "Tanner", "{arrived}");
        assert_eq!(arrived["action"], "Talk-to", "{arrived}");
        assert_eq!(arrived["index"], 25, "{arrived}");

        // Ties keep posted order: the scan only replaces on a strict
        // improvement.
        let tie = call(
            &data,
            token,
            page.clone(),
            talk_scene(
                here(3200, 3205, 0),
                json!([
                    talk_npc(
                        26,
                        804,
                        "Tanner",
                        Tile {
                            x: 3201,
                            z: 3205,
                            level: 0
                        },
                        1,
                        &["Talk-to"]
                    ),
                    talk_npc(
                        27,
                        804,
                        "Tanner",
                        Tile {
                            x: 3200,
                            z: 3206,
                            level: 0
                        },
                        1,
                        &["Talk-to"]
                    ),
                ]),
                json!({}),
            ),
        );
        assert_eq!(tie["index"], 26, "{tie}");

        // The posted `distance` is what the pick ranks by when the page
        // carries one: a row that reads near is the one that is Talk-to'd.
        let posted_near = call(
            &data,
            token,
            page.clone(),
            talk_scene(
                here(3200, 3205, 0),
                json!([
                    talk_npc(
                        28,
                        804,
                        "Tanner",
                        Tile {
                            x: 3290,
                            z: 3290,
                            level: 0
                        },
                        1,
                        &["Talk-to"]
                    ),
                    talk_npc(
                        29,
                        804,
                        "Tanner",
                        Tile {
                            x: 3200,
                            z: 3205,
                            level: 0
                        },
                        4,
                        &["Talk-to"]
                    ),
                ]),
                json!({}),
            ),
        );
        assert_eq!(posted_near["index"], 28, "{posted_near}");

        // Another step's identity is not this one's, and neither is a row that
        // carries only this step's script alias — the join is the packed id
        // and the display name, never the alias and never a nearest anything.
        let wrong = call(
            &data,
            token,
            page.clone(),
            talk_scene(
                here(3200, 3205, 0),
                json!([
                    talk_npc(
                        30,
                        541,
                        "Zeke",
                        Tile {
                            x: 3200,
                            z: 3205,
                            level: 0
                        },
                        1,
                        &["Talk-to"]
                    ),
                    talk_npc(
                        31,
                        999,
                        "tanner",
                        Tile {
                            x: 3200,
                            z: 3205,
                            level: 0
                        },
                        1,
                        &["Examine"]
                    ),
                ]),
                json!({}),
            ),
        );
        assert_eq!(wrong["kind"], "wait", "{wrong}");

        // A matching row with no talk action is not a match, and a matching
        // row the page posted no tile or distance for is unmeasured.
        let silent = call(
            &data,
            token,
            page.clone(),
            talk_scene(
                here(3200, 3205, 0),
                json!([talk_npc(
                    32,
                    804,
                    "Tanner",
                    Tile {
                        x: 3200,
                        z: 3205,
                        level: 0
                    },
                    1,
                    &["Examine"]
                )]),
                json!({}),
            ),
        );
        assert_eq!(silent["kind"], "wait", "{silent}");
        let mut unmeasured = talk_npc(
            33,
            804,
            "Tanner",
            Tile {
                x: 3200,
                z: 3205,
                level: 0,
            },
            2,
            &["Talk-to"],
        );
        unmeasured.as_object_mut().expect("row").remove("x");
        unmeasured.as_object_mut().expect("row").remove("z");
        unmeasured.as_object_mut().expect("row").remove("distance");
        let blind = call(
            &data,
            token,
            page,
            talk_scene(here(3200, 3205, 0), json!([unmeasured]), json!({})),
        );
        assert_eq!(blind["kind"], "wait", "{blind}");
    }

    /// The challenge seam: a page that holds only a selected challenge scroll
    /// joins that scroll's parent talk step, and the posted count dialog is
    /// answered with the selected answer. Empty, zero and unrelated pages still
    /// abort `none-held`.
    #[test]
    fn a_held_challenge_scroll_joins_the_parent_talk_step_and_answers_the_count() {
        on_reset();
        let data = selected();
        let parent = talk_of(&data, TALK_CHALLENGE);
        assert_eq!(parent.npc.name, "Hazelmere", "{}", parent.alias);

        // Begin on the scroll alone: the identify is `none-held`, and the seam
        // hands out the parent's token rather than a refusal.
        let opened = begin(&data, talk_page(CHALLENGE));
        assert_eq!(opened["kind"], "token", "{opened}");
        let token = token_of(&opened);
        let gate = call(&data, token, talk_page(CHALLENGE), json!({}));
        assert_eq!(gate["kind"], "callback.enabled", "{gate}");
        let logged = call(
            &data,
            token,
            talk_page(CHALLENGE),
            json!({ "resume": true }),
        );
        assert_eq!(logged["kind"], "callback.log", "{logged}");
        let message = logged["message"].as_str().unwrap_or("");
        assert!(message.contains("trail_clue_medium_anagram001"), "{logged}");
        assert!(message.contains(&TALK_CHALLENGE.to_string()), "{logged}");
        assert!(
            !message.contains(&CHALLENGE.to_string()),
            "the step is the parent, never the scroll: {logged}"
        );
        let posted = call(&data, token, talk_page(CHALLENGE), json!({}));
        assert_eq!(posted["kind"], "callback.setStatus", "{posted}");
        assert_eq!(
            RUNTIME.with(|rt| rt.borrow().step_id),
            TALK_CHALLENGE,
            "the scroll id is never the step"
        );

        // The parent's own talk path: arrived on the published plane-1 tile
        // with the parent's npc posted.
        let spawn = parent.spawn.as_ref().expect("a unique jm2 spawn");
        let hazelmere = json!([talk_npc(
            41,
            parent.npc.id,
            "Hazelmere",
            Tile {
                x: 2678,
                z: 3086,
                level: 1,
            },
            1,
            &["Talk-to"]
        )]);
        let talked = call(
            &data,
            token,
            talk_page(CHALLENGE),
            talk_scene(
                here(spawn.x, spawn.z, spawn.plane),
                hazelmere.clone(),
                json!({}),
            ),
        );
        assert_eq!(talked["kind"], "npc", "{talked}");
        assert_eq!(talked["index"], 41, "{talked}");

        // The posted count dialog is answered with the selected string, and
        // only with it: no Talk-to rides along.
        let answered = call(
            &data,
            token,
            talk_page(CHALLENGE),
            talk_scene(
                here(spawn.x, spawn.z, spawn.plane),
                hazelmere.clone(),
                json!({ "count_dialog_open": true }),
            ),
        );
        assert_eq!(answered["kind"], "answer-count", "{answered}");
        assert_eq!(answered["value"], 6859, "{answered}");
        assert_eq!(token_of(&answered), token, "{answered}");

        // A posted `false` is a closed dialog: the arm goes back to the talk
        // path rather than answering anything.
        let closed = call(
            &data,
            token,
            talk_page(CHALLENGE),
            talk_scene(
                here(spawn.x, spawn.z, spawn.plane),
                hazelmere,
                json!({ "count_dialog_open": false }),
            ),
        );
        assert_eq!(closed["kind"], "npc", "{closed}");

        // A talk step with no challenge of its own has no answer to give: the
        // open count dialog is a wait, never an invented number.
        let other = steady(&data, TALK);
        let no_answer = call(
            &data,
            other,
            talk_page(TALK),
            talk_scene(
                here(3207, 3233, 0),
                json!([]),
                json!({ "count_dialog_open": true }),
            ),
        );
        assert_eq!(no_answer["kind"], "wait", "{no_answer}");
        assert!(no_answer.get("value").is_none(), "{no_answer}");

        // The identity-only challenge parent joins the same way and answers
        // its own selected string.
        let zoo = begin(&data, talk_page(CHALLENGE_IDENTITY));
        assert_eq!(zoo["kind"], "token", "{zoo}");
        let zoo = token_of(&zoo);
        assert_eq!(
            call(&data, zoo, talk_page(CHALLENGE_IDENTITY), json!({}))["kind"],
            "callback.enabled"
        );
        assert_eq!(
            call(
                &data,
                zoo,
                talk_page(CHALLENGE_IDENTITY),
                json!({ "resume": true })
            )["kind"],
            "callback.log"
        );
        assert_eq!(
            call(&data, zoo, talk_page(CHALLENGE_IDENTITY), json!({}))["kind"],
            "callback.setStatus"
        );
        assert_eq!(
            RUNTIME.with(|rt| rt.borrow().step_id),
            TALK_CHALLENGE_IDENTITY
        );
        let zoo_answer = call(
            &data,
            zoo,
            talk_page(CHALLENGE_IDENTITY),
            json!({ "count_dialog_open": true }),
        );
        assert_eq!(zoo_answer["kind"], "answer-count", "{zoo_answer}");
        assert_eq!(zoo_answer["value"], 40, "{zoo_answer}");

        // The first posted selected scroll wins when a page holds two.
        let two = begin(&data, json!([[CHALLENGE, 1], [CHALLENGE_IDENTITY, 1]]));
        assert_eq!(two["kind"], "token", "{two}");
        assert_eq!(
            call(
                &data,
                token_of(&two),
                json!([[CHALLENGE, 1], [CHALLENGE_IDENTITY, 1]]),
                json!({})
            )["kind"],
            "callback.enabled"
        );
        assert_eq!(RUNTIME.with(|rt| rt.borrow().step_id), TALK_CHALLENGE);

        // The seam does not weaken C3: a zero count, an unrelated id and an
        // empty page are all still `none-held` refusals.
        let unrelated = unrelated(&data);
        for held in [
            json!([]),
            json!([[CHALLENGE, 0]]),
            json!([[unrelated, 1]]),
            json!([[CHALLENGE, -1]]),
        ] {
            let refused = begin(&data, held.clone());
            assert_eq!(refused["kind"], "aborted", "{held} {refused}");
            assert_eq!(refused["reason"], "none-held", "{held} {refused}");
        }
        // And the `one-held` sibling: an unrelated id beside the scroll is the
        // seam's page, because the membership rows still win first.
        let mixed = begin(&data, json!([[CASKET, 1], [CHALLENGE, 1]]));
        assert_eq!(mixed["kind"], "token", "{mixed}");
    }

    /// The parent clue swapping for its scroll keeps the live token: the
    /// identify goes `none-held`, the seam returns the same parent step, and
    /// `Collecting` still wins the page when the token is collecting.
    #[test]
    fn the_challenge_seam_keeps_the_live_token_on_the_parent_step() {
        on_reset();
        let data = selected();
        let token = steady(&data, TALK_CHALLENGE);
        // The server takes the parent clue and leaves the scroll: the token
        // survives and the step is still the parent's.
        let swapped = call(&data, token, talk_page(CHALLENGE), json!({ "npcs": [] }));
        assert_eq!(token_of(&swapped), token, "{swapped}");
        assert_ne!(swapped["kind"], "aborted", "{swapped}");
        assert_eq!(RUNTIME.with(|rt| rt.borrow().step_id), TALK_CHALLENGE);
        // The answer is still the parent's own.
        let answered = call(
            &data,
            token,
            talk_page(CHALLENGE),
            json!({ "count_dialog_open": true }),
        );
        assert_eq!(answered["kind"], "answer-count", "{answered}");
        assert_eq!(answered["value"], 6859, "{answered}");

        // Collecting first: a live collect is not a challenge join.
        let collecting = opened(&data, CASKET);
        let still = call(&data, collecting, talk_page(CHALLENGE), json!({}));
        assert_ne!(still["kind"], "aborted", "{still}");
        assert_eq!(token_of(&still), collecting, "{still}");
    }

    /// An open chat closes the talk arm for the tick: no Talk-to and no walk
    /// while the landed `dialog_ready` holds, and no answer behind an
    /// unobserved count dialog.
    #[test]
    fn no_talk_to_while_the_chat_or_the_count_dialog_is_posted_open() {
        on_reset();
        let data = selected();
        let page = talk_page(TALK);
        let token = steady(&data, TALK);
        let on_tile = json!([talk_npc(
            51,
            0,
            "Hans",
            Tile {
                x: 3207,
                z: 3233,
                level: 0
            },
            1,
            &["Talk-to"]
        )]);

        // The posted chat modal: an open chat is not a tick to Talk-to again,
        // and it does not walk either.
        for extra in [
            json!({ "chat_modal_id": 968 }),
            json!({ "chat_continue": true }),
            json!({ "chat_modal_id": 968, "chat_continue": true }),
        ] {
            let open = call(
                &data,
                token,
                page.clone(),
                talk_scene(here(3207, 3233, 0), on_tile.clone(), extra.clone()),
            );
            assert_eq!(open["kind"], "wait", "{extra} {open}");
            let far = call(
                &data,
                token,
                page.clone(),
                talk_scene(here(3100, 3233, 0), on_tile.clone(), extra.clone()),
            );
            assert_eq!(far["kind"], "wait", "{extra} {far}");
        }

        // The posted closed chat is not an open one, and an omitted slot is
        // unobserved rather than open: both leave the Talk-to free.
        for extra in [
            json!({ "chat_modal_id": -1, "chat_continue": false }),
            json!({ "chat_continue": false }),
            json!({}),
        ] {
            let free = call(
                &data,
                token,
                page.clone(),
                talk_scene(here(3207, 3233, 0), on_tile.clone(), extra.clone()),
            );
            assert_eq!(free["kind"], "npc", "{extra} {free}");
        }

        // A posted open count dialog blocks the Talk-to the same way — and for
        // a step with no challenge of its own there is nothing to answer.
        let counted = call(
            &data,
            token,
            page.clone(),
            talk_scene(
                here(3207, 3233, 0),
                on_tile.clone(),
                json!({ "count_dialog_open": true }),
            ),
        );
        assert_eq!(counted["kind"], "wait", "{counted}");

        // The landed precedence is untouched by the talk arm: the posted
        // interrupt yields, the frozen clock waits, and a posted hitpoints at
        // or below zero is `dead` — never `done` and never `'clue solved'`.
        let yielded = call(
            &data,
            token,
            page.clone(),
            talk_scene(
                here(3207, 3233, 0),
                on_tile.clone(),
                json!({ "hold": true }),
            ),
        );
        assert_eq!(yielded["kind"], "yield", "{yielded}");
        let dead = call(
            &data,
            token,
            page.clone(),
            talk_scene(
                here(3207, 3233, 0),
                on_tile.clone(),
                json!({ "hitpoints": 0 }),
            ),
        );
        assert_eq!(dead["kind"], "dead", "{dead}");
        assert!(dead["token"].is_number(), "{dead}");
        assert!(!dead.to_string().contains("clue solved"), "{dead}");
        let after = call(
            &data,
            token,
            page,
            talk_scene(here(3207, 3233, 0), on_tile, json!({})),
        );
        assert_eq!(after["reason"], "stale", "{after}");
    }

    /// The talk arm's own outcome set: the walk, the Talk-to, the answer and
    /// the waits, and never a verb or a completion that belongs to another arm.
    #[test]
    fn the_talk_arm_emits_only_walk_npc_answer_count_wait_and_yield() {
        on_reset();
        let data = selected();
        let scenes = [
            (TALK, json!({ "npcs": [] }), "no page"),
            (
                TALK,
                talk_scene(here(3100, 3233, 0), json!([]), json!({})),
                "far",
            ),
            (
                TALK,
                talk_scene(
                    here(3207, 3233, 0),
                    json!([talk_npc(
                        61,
                        0,
                        "Hans",
                        Tile {
                            x: 3207,
                            z: 3233,
                            level: 0
                        },
                        1,
                        &["Talk-to"]
                    )]),
                    json!({}),
                ),
                "arrived",
            ),
            (
                TALK_IDENTITY,
                talk_scene(here(3200, 3200, 0), json!([]), json!({})),
                "no match",
            ),
            (
                TALK_CHALLENGE,
                talk_scene(
                    here(2678, 3086, 1),
                    json!([]),
                    json!({ "count_dialog_open": true }),
                ),
                "count",
            ),
        ];
        let mut kinds = Vec::new();
        for (id, scene, name) in scenes {
            let token = steady(&data, id);
            let step = call(&data, token, talk_page(id), scene.clone());
            assert_eq!(token_of(&step), token, "{name} {step}");
            kinds.push(step["kind"].as_str().unwrap_or("").to_string());
            let text = step.to_string();
            for forbidden in ["clue solved", "done", "abandon", "supplies-needed"] {
                assert!(!text.contains(forbidden), "{name} {step}");
            }
        }
        assert_eq!(
            kinds,
            vec!["wait", "walk", "npc", "wait", "answer-count"],
            "{kinds:?}"
        );
        // The same step driven through the collect's `'clue solved'` is not
        // this arm's: a talk step never reaches a completion kind.
        for kind in &kinds {
            assert!(
                ![
                    "done",
                    "grind-ready",
                    "held",
                    "loc",
                    "obj",
                    "if-button",
                    "close-modal",
                    "puzzle-move",
                    "dead",
                    "guardian-lost",
                    "aborted"
                ]
                .contains(&kind.as_str()),
                "{kind}"
            );
        }
        // `status` is still the machine's continue shape on every one of them,
        // and `dead` never posts the solved string.
        let token = steady(&data, TALK);
        let dead = call(
            &data,
            token,
            talk_page(TALK),
            talk_scene(here(3207, 3233, 0), json!([]), json!({ "hitpoints": 0 })),
        );
        assert_eq!(dead["kind"], "dead", "{dead}");
        assert!(!dead.to_string().contains("clue solved"), "{dead}");
    }

    /// The key-hunt step of one of the two unique-spawn type keepers.
    fn key_of(data: &SelectedGameData, id: i32) -> &TalkKeyKeyRow {
        key_step(Some(data), id).unwrap_or_else(|| panic!("key keeper {id}"))
    }

    /// The published spawn tile a key row's walk carries.
    fn spawn_of(key: &TalkKeyKeyRow) -> Tile {
        let spawn = key.spawn.as_ref().expect("a unique jm2 spawn");
        Tile {
            x: spawn.x,
            z: spawn.z,
            level: spawn.plane,
        }
    }

    /// One key-hunt call's pages: the posted `here` tile, the posted npc page,
    /// the posted ground page, the posted pack rows with their slot count, and
    /// the posted local-player slot the kill's own `targetsMe` read compares
    /// with. Everything the hunt does not need this call is left empty, so each
    /// test names only the page it is about.
    fn key_scene(here_tile: Value, extra: Value) -> Value {
        let mut scene = json!({
            "here": here_tile,
            "npcs": [],
            "ground": [],
            "inv": [],
            "inv_size": 28,
            "self_slot": 0,
        });
        for (key, value) in extra.as_object().expect("extra") {
            scene[key] = value.clone();
        }
        scene
    }

    /// One wrapper-marshalled posted npc row as the keeper hunt reads it: the
    /// posted index the Attack carries, the packed id and posted display name
    /// the identity join compares, the posted tile and distance the published
    /// spawn's radius is measured by, the posted health pair the kill is read
    /// through, and the posted action list the `Attack` is read from.
    fn keeper_npc(
        index: i32,
        id: i32,
        name: &str,
        tile: Tile,
        distance: i32,
        actions: &[&str],
    ) -> Value {
        json!({
            "index": index,
            "id": id,
            "name": name,
            "x": tile.x,
            "z": tile.z,
            "level": tile.level,
            "distance": distance,
            "health": 10,
            "max_health": 10,
            "in_combat": false,
            "actions": actions,
            "target_kind": 0,
            "target_index": -1,
        })
    }

    /// The owned keeper's last-seen aged past the frozen grace: the only way to
    /// reach the gone-outside-grace read without a six-second test.
    fn age_keeper_seen(ms: u64) {
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            if let Some(owned) = rt.keeper.as_mut().and_then(|keeper| keeper.owned.as_mut()) {
                owned.seen_at -= Duration::from_millis(ms);
            }
        });
    }

    /// A freeze that outlasted the remaining keeper grace, without a
    /// six-second test: the clock's own `frozen_at` and the owned keeper's
    /// last-seen are both placed at the freeze's start, so the reclaim the thaw
    /// makes is exactly the frozen interval — the shape a real long freeze
    /// hands the session.
    fn froze_across_the_keeper_grace() {
        on_pause();
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            let frozen_at = Instant::now() - Duration::from_millis(KILL_GRACE_MS + 1);
            rt.clock.frozen_at = Some(frozen_at);
            if let Some(owned) = rt.keeper.as_mut().and_then(|keeper| keeper.owned.as_mut()) {
                owned.seen_at = frozen_at;
            }
        });
        on_resume();
    }

    /// The key-hunt membership is the selected `talk_key.keys` family and
    /// nothing else: the two unique-spawn type keepers are the hunt's steps,
    /// the five matcher-keepers publish no unique spawn and are not, and
    /// neither family's step is the other's — a key keeper is never a talk step
    /// and a talk step is never hunted.
    #[test]
    fn the_key_membership_is_the_selected_keys_family_and_nothing_else() {
        on_reset();
        let data = selected();
        let talk = data.talk_key().expect("talk_key");
        assert_eq!(talk.keys.len(), 7, "the landed family");
        assert_eq!(
            talk.keys.iter().filter(|key| key.spawn.is_some()).count(),
            2,
            "the unique-spawn slice"
        );
        for key in &talk.keys {
            let row = row(&data, key.id);
            assert_eq!(row.role, "clue", "{}", key.alias);
            // Every key row is a membership row the landed identify returns,
            // and none is a second classify of the arms ahead of the hunt.
            assert_eq!(
                key_step(Some(&data), key.id).map(|key| key.id),
                Some(key.id),
                "{}",
                key.alias
            );
            assert_eq!(search_tile(row), None, "{}", key.alias);
            assert_eq!(guarded_tile(row), None, "{}", key.alias);
            assert_eq!(dig_tile(row), None, "{}", key.alias);
            assert_eq!(casket_name(Some(&data), row), None, "{}", key.alias);
            assert_eq!(
                talk_step(Some(&data), key.id).map(|talk| talk.id),
                None,
                "{}",
                key.alias
            );
        }
        // The two slice rows: one packed-type keeper each, on a published
        // spawn, with the pinned key the arm Takes.
        for (id, key_id, keeper, name, spawn) in [
            (
                RIDDLE,
                KEEPER_KEY,
                KEEPER_ID,
                KEEPER_NAME,
                Tile {
                    x: 3039,
                    z: 3700,
                    level: 0,
                },
            ),
            (
                PENDA,
                PENDA_KEY,
                PENDA_ID,
                PENDA_NAME,
                Tile {
                    x: 2910,
                    z: 3539,
                    level: 0,
                },
            ),
        ] {
            let step = key_of(&data, id);
            assert_eq!(step.key_id, key_id, "{}", step.alias);
            assert_eq!(
                keeper_type(&step.keeper),
                Some((keeper, name)),
                "{}",
                step.alias
            );
            assert_eq!(spawn_of(step), spawn, "{}", step.alias);
        }
        // The five matcher-keepers publish no unique spawn, so not one of them
        // is a hunt: two name a packed type the family covered for a
        // non-unique jm2 NPC spawn, and three a category or a bare name that
        // is not one npc at all. No coordinate is invented for the first two
        // and no type list for the other three.
        for id in MATCHER_KEEPERS {
            assert!(key_of(&data, id).spawn.is_none(), "{id}");
        }
        for id in [2833, 2835] {
            assert!(keeper_type(&key_of(&data, id).keeper).is_some(), "{id}");
        }
        for id in [2837, 2839, 3605] {
            assert_eq!(keeper_type(&key_of(&data, id).keeper), None, "{id}");
        }
        // No selected pin is no key step, and neither is a talk step or any
        // other landed membership.
        assert_eq!(key_step(None, RIDDLE).map(|key| key.id), None);
        for id in [
            TALK,
            TALK_IDENTITY,
            MAP_EMPTY,
            SEARCH,
            UNGUARDED,
            GUARDED,
            CLUE,
            CASKET,
        ] {
            assert_eq!(key_step(Some(&data), id).map(|key| key.id), None, "{id}");
        }
    }

    /// The two unique-spawn type keepers driven through the whole hunt: the
    /// walk to the published spawn with its `plane` as the verb's `level`, the
    /// one Attack on the posted keeper, the kill, the Take of the key it drops,
    /// and the idle the original riddle keeps once that key is on the posted
    /// pack page.
    #[test]
    fn a_key_keeper_step_walks_attacks_and_takes_the_key_it_drops() {
        on_reset();
        let data = selected();
        for (id, key_id, keeper, name, spawn) in [
            (
                RIDDLE,
                KEEPER_KEY,
                KEEPER_ID,
                KEEPER_NAME,
                Tile {
                    x: 3039,
                    z: 3700,
                    level: 0,
                },
            ),
            (
                PENDA,
                PENDA_KEY,
                PENDA_ID,
                PENDA_NAME,
                Tile {
                    x: 2910,
                    z: 3539,
                    level: 0,
                },
            ),
        ] {
            let page = json!([[id, 1]]);
            let token = steady(&data, id);
            let arrived = here(spawn.x, spawn.z, spawn.level);
            let posted = keeper_npc(21, keeper, name, spawn, 1, &[ATTACK]);

            // Not arrived: the walk is the published spawn itself, repeating
            // until the posted `here` holds.
            let walked = call(
                &data,
                token,
                page.clone(),
                key_scene(here(spawn.x - 30, spawn.z, spawn.level), json!({})),
            );
            assert_eq!(walked["kind"], "walk", "{id} {walked}");
            assert_eq!(walked["x"], spawn.x, "{id} {walked}");
            assert_eq!(walked["z"], spawn.z, "{id} {walked}");
            assert_eq!(walked["level"], spawn.level, "{id} {walked}");

            // Arrived with the keeper posted on its own tile: the one Attack,
            // carrying the posted name, the frozen action and the posted scene
            // index and nothing else.
            let attacked = call(
                &data,
                token,
                page.clone(),
                key_scene(arrived.clone(), json!({ "npcs": [posted.clone()] })),
            );
            assert_eq!(attacked["kind"], "npc", "{id} {attacked}");
            assert_eq!(attacked["name"], name, "{id} {attacked}");
            assert_eq!(attacked["action"], ATTACK, "{id} {attacked}");
            assert_eq!(attacked["index"], 21, "{id} {attacked}");
            for absent in ["component_id", "id", "x", "z", "level", "message"] {
                assert!(attacked.get(absent).is_none(), "{id} {absent} {attacked}");
            }

            // Posted and alive: the hunt waits, and the Attack is never issued
            // twice for one owned index.
            let alive = call(
                &data,
                token,
                page.clone(),
                key_scene(arrived.clone(), json!({ "npcs": [posted.clone()] })),
            );
            assert_eq!(alive["kind"], "wait", "{id} {alive}");
            assert_eq!(token_of(&alive), token, "{id} {alive}");

            // The kill: the owned index posted at zero health beside a posted
            // maximum with this token's own fight on it. The key is already on
            // the tile, so the kill lets the Take out on this same call.
            let dying = targeting(field(posted.clone(), "health", json!(0)), 0);
            let dropped = ground(key_id, "Key", spawn.x, spawn.z, spawn.level, &[TAKE]);
            let taken = call(
                &data,
                token,
                page.clone(),
                key_scene(
                    arrived.clone(),
                    json!({ "npcs": [dying], "ground": [dropped.clone()] }),
                ),
            );
            assert_eq!(taken["kind"], "obj", "{id} {taken}");
            assert_eq!(taken["x"], spawn.x, "{id} {taken}");
            assert_eq!(taken["z"], spawn.z, "{id} {taken}");
            assert_eq!(taken["level"], spawn.level, "{id} {taken}");
            assert_eq!(taken["name"], "Key", "{id} {taken}");
            assert_eq!(taken["action"], TAKE, "{id} {taken}");

            // The key on the posted page ends the hunt: the original riddle
            // idles, the gate is not re-armed and no completion kind is
            // emitted — whatever the page still posts beside the key. The key
            // rides the same `(id, count)` page the identify reads, exactly as
            // the wrapper posts the pack page it is built from.
            let keyed = json!([[id, 1], [key_id, 1]]);
            for extra in [
                json!({ "npcs": [posted.clone()], "ground": [dropped.clone()] }),
                json!({}),
            ] {
                let mut scene = key_scene(arrived.clone(), extra);
                scene["inv"] = json!([inv(key_id, "Key", 1)]);
                let idle = call(&data, token, keyed.clone(), scene);
                assert_eq!(idle["kind"], "wait", "{id} {idle}");
                assert_eq!(token_of(&idle), token, "{id} {idle}");
                let text = idle.to_string();
                for forbidden in [
                    "clue solved",
                    "grind-ready",
                    "\"done\"",
                    "guardian-lost",
                    "abandon",
                    "supplies-needed",
                ] {
                    assert!(!text.contains(forbidden), "{id} {forbidden} {idle}");
                }
            }
        }
    }

    /// The owned keeper that leaves the posted page inside the frozen grace is
    /// this token's kill even without a posted zero health — and the kill walks
    /// back to the published spawn before it Takes anything. A page that posted
    /// no npc page at all cannot observe the keeper, so the hunt waits.
    #[test]
    fn the_owned_keeper_gone_inside_the_grace_is_the_kill_and_walks_back() {
        on_reset();
        let data = selected();
        let page = json!([[RIDDLE, 1]]);
        let spawn = spawn_of(key_of(&data, RIDDLE));
        let token = steady(&data, RIDDLE);
        let arrived = here(spawn.x, spawn.z, spawn.level);
        let posted = keeper_npc(21, KEEPER_ID, KEEPER_NAME, spawn, 1, &[ATTACK]);
        let attacked = call(
            &data,
            token,
            page.clone(),
            key_scene(arrived.clone(), json!({ "npcs": [posted] })),
        );
        assert_eq!(attacked["kind"], "npc", "{attacked}");

        // No posted npc page this call: the owned keeper cannot be read, so
        // nothing walks and nothing is Taken behind it.
        let blind = call(
            &data,
            token,
            page.clone(),
            json!({ "here": here(spawn.x - 30, spawn.z, spawn.level) }),
        );
        assert_eq!(blind["kind"], "wait", "{blind}");

        // The owned index left the page inside the grace, from thirty tiles
        // off: the kill, and the walk back to the published spawn.
        let gone = call(
            &data,
            token,
            page.clone(),
            key_scene(
                here(spawn.x - 30, spawn.z, spawn.level),
                json!({ "npcs": [] }),
            ),
        );
        assert_eq!(gone["kind"], "walk", "{gone}");
        assert_eq!(gone["x"], spawn.x, "{gone}");
        assert_eq!(gone["z"], spawn.z, "{gone}");
        assert_eq!(gone["level"], spawn.level, "{gone}");

        // Arrived: the key the kill dropped one step off the spawn is Taken
        // with the landed `obj`, at the row's own posted tile.
        let dropped = ground(
            KEEPER_KEY,
            "Key",
            spawn.x + 1,
            spawn.z,
            spawn.level,
            &[TAKE],
        );
        let taken = call(
            &data,
            token,
            page.clone(),
            key_scene(arrived, json!({ "ground": [dropped] })),
        );
        assert_eq!(taken["kind"], "obj", "{taken}");
        assert_eq!(taken["x"], spawn.x + 1, "{taken}");
        assert_eq!(taken["z"], spawn.z, "{taken}");
        assert_eq!(taken["level"], spawn.level, "{taken}");
        assert_eq!(taken["action"], TAKE, "{taken}");
    }

    /// The owned keeper gone outside the frozen grace without ever being seen
    /// at zero health is not a kill: this hunt has no `guardian-lost`, no
    /// invented respawn timer and no second kind. The token waits, and a key
    /// already on the floor is not Taken before the kill it belongs to.
    #[test]
    fn a_keeper_gone_outside_the_grace_is_a_wait_and_never_a_lost_encounter() {
        on_reset();
        let data = selected();
        let page = json!([[RIDDLE, 1]]);
        let spawn = spawn_of(key_of(&data, RIDDLE));
        let token = steady(&data, RIDDLE);
        let arrived = here(spawn.x, spawn.z, spawn.level);
        let posted = keeper_npc(21, KEEPER_ID, KEEPER_NAME, spawn, 1, &[ATTACK]);
        let attacked = call(
            &data,
            token,
            page.clone(),
            key_scene(arrived.clone(), json!({ "npcs": [posted] })),
        );
        assert_eq!(attacked["kind"], "npc", "{attacked}");

        age_keeper_seen(KILL_GRACE_MS + 1);
        let dropped = ground(KEEPER_KEY, "Key", spawn.x, spawn.z, spawn.level, &[TAKE]);
        let idle = call(
            &data,
            token,
            page.clone(),
            key_scene(arrived.clone(), json!({ "ground": [dropped] })),
        );
        assert_eq!(idle["kind"], "wait", "{idle}");
        assert_eq!(token_of(&idle), token, "{idle}");
        let text = idle.to_string();
        for forbidden in ["guardian-lost", "keeper-lost", "obj", "\"done\"", "abandon"] {
            assert!(!text.contains(forbidden), "{forbidden} {idle}");
        }
    }

    /// A freeze that outlasted the remaining keeper grace still ends in the
    /// kill: the thaw reclaims the frozen interval into the owned last-seen, so
    /// the disappearance that follows it is read as this token's kill and the
    /// key it left behind is Taken.
    #[test]
    fn a_freeze_across_the_keeper_grace_still_ends_in_the_kill() {
        on_reset();
        let data = selected();
        let page = json!([[RIDDLE, 1]]);
        let spawn = spawn_of(key_of(&data, RIDDLE));
        let token = steady(&data, RIDDLE);
        let arrived = here(spawn.x, spawn.z, spawn.level);
        let posted = keeper_npc(21, KEEPER_ID, KEEPER_NAME, spawn, 1, &[ATTACK]);
        let attacked = call(
            &data,
            token,
            page.clone(),
            key_scene(arrived.clone(), json!({ "npcs": [posted] })),
        );
        assert_eq!(attacked["kind"], "npc", "{attacked}");

        froze_across_the_keeper_grace();
        let kill = call(
            &data,
            token,
            page.clone(),
            key_scene(arrived.clone(), json!({ "npcs": [] })),
        );
        assert_eq!(kill["kind"], "wait", "{kill}");
        assert!(!kill.to_string().contains("guardian-lost"), "{kill}");

        let dropped = ground(KEEPER_KEY, "Key", spawn.x, spawn.z, spawn.level, &[TAKE]);
        let taken = call(
            &data,
            token,
            page.clone(),
            key_scene(arrived, json!({ "ground": [dropped] })),
        );
        assert_eq!(taken["kind"], "obj", "{taken}");
        assert_eq!(taken["action"], TAKE, "{taken}");
    }

    /// The key Take is the published spawn's own read: the posted ground row
    /// must carry the key's own id, the posted `Take`, a posted name, the
    /// spawn's own level and a tile inside `ARRIVE_RADIUS` of that spawn — and
    /// the pack must have room for it. A full pack waits, because this arm
    /// Drops no food, and a page that posted no slot count waits too.
    #[test]
    fn the_key_take_reads_the_posted_key_row_at_the_spawn() {
        on_reset();
        let data = selected();
        let page = json!([[RIDDLE, 1]]);
        let spawn = spawn_of(key_of(&data, RIDDLE));
        let token = steady(&data, RIDDLE);
        let arrived = here(spawn.x, spawn.z, spawn.level);
        let posted = keeper_npc(21, KEEPER_ID, KEEPER_NAME, spawn, 1, &[ATTACK]);
        // The hunt's own Attack first: the pickup is never armed without the
        // kill this token's Attack went out for.
        let attacked = call(
            &data,
            token,
            page.clone(),
            key_scene(arrived.clone(), json!({ "npcs": [posted.clone()] })),
        );
        assert_eq!(attacked["kind"], "npc", "{attacked}");
        let dying = targeting(field(posted, "health", json!(0)), 0);
        // The kill, with nothing on the floor: the pickup is armed and waits.
        let kill = call(
            &data,
            token,
            page.clone(),
            key_scene(arrived.clone(), json!({ "npcs": [dying] })),
        );
        assert_eq!(kill["kind"], "wait", "{kill}");

        for (name, row) in [
            (
                "no Take",
                ground(
                    KEEPER_KEY,
                    "Key",
                    spawn.x,
                    spawn.z,
                    spawn.level,
                    &["Examine"],
                ),
            ),
            (
                "another id",
                ground(
                    KEEPER_KEY + 1,
                    "Key",
                    spawn.x,
                    spawn.z,
                    spawn.level,
                    &[TAKE],
                ),
            ),
            (
                "another level",
                ground(
                    KEEPER_KEY,
                    "Key",
                    spawn.x,
                    spawn.z,
                    spawn.level + 1,
                    &[TAKE],
                ),
            ),
            (
                "off the radius",
                ground(
                    KEEPER_KEY,
                    "Key",
                    spawn.x + 2,
                    spawn.z,
                    spawn.level,
                    &[TAKE],
                ),
            ),
            (
                "no name",
                ground(KEEPER_KEY, "", spawn.x, spawn.z, spawn.level, &[TAKE]),
            ),
        ] {
            let idle = call(
                &data,
                token,
                page.clone(),
                key_scene(arrived.clone(), json!({ "ground": [row] })),
            );
            assert_eq!(idle["kind"], "wait", "{name} {idle}");
            assert_eq!(token_of(&idle), token, "{name} {idle}");
        }

        // A full pack: the Take waits rather than Dropping a food row for it.
        let dropped = ground(KEEPER_KEY, "Key", spawn.x, spawn.z, spawn.level, &[TAKE]);
        let full = call(
            &data,
            token,
            page.clone(),
            key_scene(
                arrived.clone(),
                json!({
                    "ground": [dropped.clone()],
                    "inv": [inv(SPADE_ITEM, SPADE_NAME, 1)],
                    "inv_size": 1,
                }),
            ),
        );
        assert_eq!(full["kind"], "wait", "{full}");

        // No posted slot count: the pack's fullness is not invented for it.
        let mut unknown = key_scene(arrived.clone(), json!({ "ground": [dropped.clone()] }));
        unknown.as_object_mut().expect("object").remove("inv_size");
        let waited = call(&data, token, page.clone(), unknown);
        assert_eq!(waited["kind"], "wait", "{waited}");

        // Room in the pack and the key on the spawn: the landed Take.
        let taken = call(
            &data,
            token,
            page.clone(),
            key_scene(arrived, json!({ "ground": [dropped] })),
        );
        assert_eq!(taken["kind"], "obj", "{taken}");
        assert_eq!(taken["name"], "Key", "{taken}");
        assert_eq!(taken["action"], TAKE, "{taken}");
    }

    /// The key this keeper drops is the whole end of the hunt: on the posted
    /// pack page the original riddle idles — no walk, no Attack, no gate
    /// re-arm, no new kind, and never `'clue solved'`, `grind-ready` or `done`.
    #[test]
    fn a_held_key_ends_the_hunt_with_the_original_riddles_idle() {
        on_reset();
        let data = selected();
        for (id, key_id, keeper, name, spawn) in [
            (
                RIDDLE,
                KEEPER_KEY,
                KEEPER_ID,
                KEEPER_NAME,
                Tile {
                    x: 3039,
                    z: 3700,
                    level: 0,
                },
            ),
            (
                PENDA,
                PENDA_KEY,
                PENDA_ID,
                PENDA_NAME,
                Tile {
                    x: 2910,
                    z: 3539,
                    level: 0,
                },
            ),
        ] {
            let page = json!([[id, 1]]);
            let token = steady(&data, id);
            let posted = keeper_npc(21, keeper, name, spawn, 1, &[ATTACK]);
            // The keeper posted on its own spawn and the key already on the
            // posted page: no Attack. And a page with no `here` at all: no
            // walk.
            let keyed = json!([[id, 1], [key_id, 1]]);
            for extra in [
                key_scene(
                    here(spawn.x, spawn.z, spawn.level),
                    json!({ "npcs": [posted] }),
                ),
                json!({}),
            ] {
                let idle = call(&data, token, keyed.clone(), extra);
                assert_eq!(idle["kind"], "wait", "{id} {idle}");
                assert_eq!(token_of(&idle), token, "{id} {idle}");
                assert_eq!(idle["token"], json!(token), "{id} {idle}");
                for absent in ["x", "z", "level", "name", "action", "index"] {
                    assert!(idle.get(absent).is_none(), "{id} {absent} {idle}");
                }
            }
        }
    }

    /// The hunt's own outcome set: the walk, the Attack, the key Take and the
    /// idle `wait` — plus the yield the posted cooperative interrupt still wins
    /// with. No Protect from Magic click, no Spade Dig, no collect verb and no
    /// completion kind is ever this arm's.
    #[test]
    fn the_key_hunt_emits_only_walk_npc_obj_wait_and_yield() {
        on_reset();
        let data = selected();
        let page = json!([[RIDDLE, 1]]);
        let spawn = spawn_of(key_of(&data, RIDDLE));
        let arrived = here(spawn.x, spawn.z, spawn.level);
        let posted = keeper_npc(21, KEEPER_ID, KEEPER_NAME, spawn, 1, &[ATTACK]);
        let dying = targeting(field(posted.clone(), "health", json!(0)), 0);
        let dropped = ground(KEEPER_KEY, "Key", spawn.x, spawn.z, spawn.level, &[TAKE]);
        let scenes = [
            // Not arrived: the walk.
            key_scene(here(spawn.x - 30, spawn.z, spawn.level), json!({})),
            // Arrived with the keeper posted: the one Attack.
            key_scene(arrived.clone(), json!({ "npcs": [posted.clone()] })),
            // Posted and alive: the wait.
            key_scene(arrived.clone(), json!({ "npcs": [posted.clone()] })),
            // The kill and the key it dropped: the Take.
            key_scene(
                arrived.clone(),
                json!({ "npcs": [dying], "ground": [dropped.clone()] }),
            ),
            // The key in hand: the original riddle's idle.
            key_scene(
                arrived.clone(),
                json!({ "inv": [inv(KEEPER_KEY, "Key", 1)] }),
            ),
        ];
        let token = steady(&data, RIDDLE);
        let mut kinds = Vec::new();
        for scene in scenes {
            let step = call(&data, token, page.clone(), scene);
            assert_eq!(token_of(&step), token, "{step}");
            kinds.push(step["kind"].as_str().unwrap_or("").to_string());
        }
        assert_eq!(
            kinds,
            vec!["walk", "npc", "wait", "obj", "wait"],
            "{kinds:?}"
        );
        assert_eq!(
            kinds
                .iter()
                .filter(|kind| !["walk", "npc", "obj", "wait"].contains(&kind.as_str()))
                .count(),
            0,
            "{kinds:?}"
        );

        // The posted `hold || ours` interrupt still wins over the hunt.
        let mut interrupt = key_scene(arrived.clone(), json!({ "npcs": [posted] }));
        interrupt["hold"] = json!(true);
        let yielded = call(&data, token, page.clone(), interrupt);
        assert_eq!(yielded["kind"], "yield", "{yielded}");
        assert_eq!(token_of(&yielded), token, "{yielded}");
        assert!(!yielded.to_string().contains("clue solved"), "{yielded}");
    }

    /// The frozen clock, the posted hitpoints and the token's own end keep
    /// their precedence over the hunt: a frozen call emits no walk and no
    /// Attack, and a posted effective hitpoints at zero is the `dead` terminal
    /// — never `done`, and never a completion kind.
    #[test]
    fn freeze_yield_and_death_beat_the_key_hunt() {
        on_reset();
        let data = selected();
        let page = json!([[RIDDLE, 1]]);
        let spawn = spawn_of(key_of(&data, RIDDLE));
        let token = steady(&data, RIDDLE);
        let danger = key_scene(here(spawn.x - 30, spawn.z, spawn.level), json!({}));
        on_pause();
        let paused = call(&data, token, page.clone(), danger.clone());
        assert_eq!(paused["kind"], "wait", "{paused}");
        on_resume();
        on_hold(true);
        let held_clock = call(&data, token, page.clone(), danger.clone());
        assert_eq!(held_clock["kind"], "wait", "{held_clock}");
        on_hold(false);
        for step in [&paused, &held_clock] {
            for absent in ["x", "z", "level", "action", "name", "index"] {
                assert!(step.get(absent).is_none(), "{absent} {step}");
            }
            assert_eq!(token_of(step), token, "{step}");
        }
        // Thawed and unheld, the walk is still there.
        let walked = call(&data, token, page.clone(), danger.clone());
        assert_eq!(walked["kind"], "walk", "{walked}");

        // The posted effective hitpoints read zero: the terminal wins over the
        // walk, the Attack and the Take alike.
        let dead = call(
            &data,
            token,
            page.clone(),
            key_scene(
                here(spawn.x, spawn.z, spawn.level),
                json!({
                    "npcs": [keeper_npc(21, KEEPER_ID, KEEPER_NAME, spawn, 1, &[ATTACK])],
                    "inv": [inv(KEEPER_KEY, "Key", 1)],
                    "hitpoints": 0,
                }),
            ),
        );
        assert_eq!(dead["kind"], "dead", "{dead}");
        assert!(!dead.to_string().contains("clue solved"), "{dead}");
        assert!(!dead.to_string().contains("\"done\""), "{dead}");
        let after = call(&data, token, page, danger);
        assert_eq!(after["kind"], "aborted", "{after}");
        assert_eq!(after["reason"], "stale", "{after}");
    }

    /// A different held step drops the live hunt with the step: coming back to
    /// the key riddle walks and Attacks the posted keeper again rather than
    /// resuming the index the previous session owned.
    #[test]
    fn a_different_held_step_drops_the_key_hunt() {
        on_reset();
        let data = selected();
        let page = json!([[RIDDLE, 1]]);
        let spawn = spawn_of(key_of(&data, RIDDLE));
        let arrived = here(spawn.x, spawn.z, spawn.level);
        let posted = keeper_npc(21, KEEPER_ID, KEEPER_NAME, spawn, 1, &[ATTACK]);
        let token = steady(&data, RIDDLE);
        let attacked = call(
            &data,
            token,
            page.clone(),
            key_scene(arrived.clone(), json!({ "npcs": [posted.clone()] })),
        );
        assert_eq!(attacked["kind"], "npc", "{attacked}");

        // A different membership row is held: the landed gate re-arms for it.
        let other = json!([[MAP_EMPTY, 1]]);
        let re_armed = call(
            &data,
            token,
            other.clone(),
            key_scene(arrived.clone(), json!({ "npcs": [posted.clone()] })),
        );
        assert_eq!(re_armed["kind"], "callback.enabled", "{re_armed}");
        let logged = call(&data, token, other.clone(), json!({ "resume": true }));
        assert_eq!(logged["kind"], "callback.log", "{logged}");
        let _ = call(&data, token, other.clone(), json!({}));

        // Back to the key riddle: the gate re-arms again, and the next steady
        // call is the hunt's own start — the posted keeper is Attacked again
        // rather than read as the old session's kill.
        let back = call(
            &data,
            token,
            page.clone(),
            key_scene(arrived.clone(), json!({ "npcs": [posted.clone()] })),
        );
        assert_eq!(back["kind"], "callback.enabled", "{back}");
        let logged = call(&data, token, page.clone(), json!({ "resume": true }));
        assert_eq!(logged["kind"], "callback.log", "{logged}");
        let _ = call(&data, token, page.clone(), json!({}));
        let reborn = call(
            &data,
            token,
            page.clone(),
            key_scene(arrived, json!({ "npcs": [posted] })),
        );
        assert_eq!(reborn["kind"], "npc", "{reborn}");
        assert_eq!(reborn["action"], ATTACK, "{reborn}");
        assert_eq!(reborn["index"], 21, "{reborn}");
    }

    // ── the Entrana strip, its restore and the abandon latch ──

    /// `trail_clue_hard_riddle027`: the sole selected row whose own decode lands
    /// in the cap box — `trail_coord=0_44_52_2_23` → `(2818, 3351, 0)`.
    const ENTRANA: i32 = 3579;
    const ENTRANA_X: i32 = 2818;
    const ENTRANA_Z: i32 = 3351;
    /// The two hard-trail dagger ids the strip unequips but never lists.
    const DDS_POISONED: i32 = 1231;
    /// A refused name the frozen vectors bind, and its item id.
    const HELM: i32 = 1163;

    /// One wrapper-marshalled worn row: the raw `host().snapshot.equipment`
    /// shape, with the name the matcher folds and the id the DDS exclusion
    /// joins.
    fn worn(id: i32, name: &str, slot: i32) -> Value {
        json!({ "id": id, "name": name, "count": 1, "slot": slot })
    }

    /// The machine's own stripped list.
    fn stripped() -> Vec<String> {
        RUNTIME.with(|rt| rt.borrow().stripped.clone())
    }

    /// The adapter's own read, straight through the dispatch seat.
    fn owns() -> bool {
        dispatch(None, &json!({ "op": "ownsEquipment" }))["owns"] == true
    }

    /// The posted booth the bank trip opens, as the wrapper marshals
    /// `nearest_booth`: the tile the player stands on beside it.
    fn booth_page() -> Value {
        json!({
            "x": 2810,
            "z": 3350,
            "level": 0,
            "id": 2213,
            "name": "Bank booth",
            "op": "Use-quickly",
        })
    }

    /// The completion kinds no step of a strip or a reclaim may carry: the
    /// three-step latch is the finished collect's own and nothing else's.
    fn assert_no_completion(step: &Value) {
        let text = step.to_string();
        for forbidden in ["clue solved", "grind-ready", "\"done\""] {
            assert!(!text.contains(forbidden), "{forbidden} {step}");
        }
    }

    /// The frozen matcher's own vectors, ported whole: the twelve names the
    /// frozen `entranaGear.test.ts` binds refused and its ten let through, plus
    /// the same frozen regex' edges — the `\b` on a longer word, the
    /// `two.handed` wildcard, the `gauntlets?` optional `s` and the
    /// `body(?!\s+rune\b)` lookahead beside `Body runes`. Every expectation is
    /// the frozen regex' own answer.
    #[test]
    fn the_frozen_entrana_matcher_folds_the_names_it_was_bound_to() {
        for name in [
            "Dragonhide body",
            "Dragonhide chaps",
            "Dragon vambraces",
            "Coif",
            "Dragonfire shield",
            "Legends cape",
            "Leather gloves",
            "Studded body",
            "Wizard hat",
            "Dragon dagger(p)",
            "Magic shortbow",
            "Maple longbow",
            "Rune full helm",
            "Two-handed sword",
            "Two handed",
            "Rune platebody",
            "Body runes",
            "Dragon battleaxe",
            "Rune kiteshield",
            "Cape of legends",
            "2h sword",
            "Battlestaff",
            "Staff of fire",
            "Rune plateskirt",
            "Skirt of silk",
            "Dragon sq shield",
            "Dragon square shield",
            "Snakeskin chaps",
            "snelm",
            "Cowl",
            "Hood",
            "Dragon gloves",
            "Rune claws",
            "Granite maul",
            "Rune cannon",
            "Med helm",
            "Full helm",
            "Rune chainbody",
            "Rune platelegs",
            "Rune defender",
            "Obsidian cape",
            "Fire cape",
            "God cape",
            "Rune cloak",
            "Air battlestaff",
            "Magic longbow",
            "Rune crossbow",
            "Rune javelin",
            "Rune dart",
            "Rune thrownaxe",
            "Rune knife",
            "Rune warhammer",
            "Rune spear",
            "Rune hasta",
            "Rune halberd",
            "Rune mace",
            "Rune scimitar",
            "Rune longsword",
            "Rune axe",
            "Rune pickaxe",
            "Dragon whip",
            "Bronze dagger",
            "Rune gloves",
            "Leather vambraces",
            "Anti-dragon shield",
            "Cape",
            "somebody body",
        ] {
            assert!(entrana_restricted_gear(name), "{name}");
        }
        for name in [
            "Amulet of glory",
            "Leather boots",
            "Rune arrow",
            "Body rune",
            "Shark",
            "Clue scroll",
            "Spade",
            "Sextant",
            "Superantipoison(4)",
            "Coins",
            "twohanded",
            "Somebody",
            "Shielded",
            "Swordfish",
            "Rune skirt",
            "Cannonball",
            "Brown apron",
            "Amulet of fury",
            "Zamorak monk top",
            "Priest gown",
            "Desert shirt",
            "Boots of lightness",
            "Rune boots",
            "Climbing boots",
            "Dragonstone",
            "Rune arrowtips",
            "Arrow shaft",
            "Coifed",
            "Hatchet",
            "Sharktooth",
            "Hooded",
            "Bodyguard",
        ] {
            assert!(!entrana_restricted_gear(name), "{name}");
        }
    }

    /// The strip's membership is the row's own selected decode and never a
    /// copied coordinate, a casket row is not a member, and a row whose decode
    /// is outside the box or off the level is not one either.
    #[test]
    fn the_entrana_membership_is_the_rows_own_decoded_coord() {
        let data = selected();
        assert_eq!(row(&data, ENTRANA).alias, "trail_clue_hard_riddle027");
        assert!(entrana_coord(row(&data, ENTRANA)));
        // The box's own ends are in it and one step outside is not.
        let inside = member(vec![param("trail_coord", "0_43_52_50_33")], None);
        assert!(entrana_coord(&inside), "the box's own west edge is in it");
        for value in [
            "0_43_52_49_33", // x 2817, one west of the box
            "0_44_52_63_33", // x 2879, one east
            "1_43_52_50_33", // the right square on the wrong level
        ] {
            assert!(
                !entrana_coord(&member(vec![param("trail_coord", value)], None)),
                "{value}"
            );
        }
        // A desc-only row carries no coord at all, and a casket never arms it
        // even when the id it is posted under is the box row's own.
        assert!(!entrana_coord(&member(vec![], None)));
        let mut casket = member(vec![param("trail_coord", "0_44_52_2_23")], None);
        casket.role = "casket".into();
        assert!(!entrana_coord(&casket), "a casket never arms the strip");
    }

    /// The strip's unequip pass: the posted worn rows the frozen matcher folds
    /// go out as the landed `unequip` verb — the worn row's own `Remove`, which
    /// the host's `wear` cannot do — one per call, the two hard-trail dagger
    /// ids are unequipped but never listed, and the rows the matcher lets
    /// through are never a verb at all. The deposit pass that follows takes
    /// every regex-matching pack row, listed or not.
    #[test]
    fn the_strip_unequips_the_folded_names_and_never_lists_the_dds() {
        on_reset();
        let data = selected();
        let page = json!([[ENTRANA, 1]]);
        let token = steady(&data, ENTRANA);
        // The dagger is first in posted order, then the helm, then the two the
        // monks let through.
        let posted = json!([
            worn(DDS_POISONED, "Dragon dagger(p)", 3),
            worn(HELM, "Rune full helm", 0),
            worn(1704, "Amulet of glory", 2),
            worn(3791, "Leather boots", 10),
        ]);
        let dagger = call(&data, token, page.clone(), json!({ "equipment": posted }));
        assert_eq!(dagger["kind"], "unequip", "{dagger}");
        assert_eq!(dagger["name"], "Dragon dagger(p)", "{dagger}");
        assert_eq!(dagger["token"], token, "{dagger}");
        assert!(
            stripped().is_empty(),
            "a hard-trail dagger id is never listed: {:?}",
            stripped()
        );
        assert_no_completion(&dagger);
        // It left the page: the helm is next, and the two non-matches are never
        // dispatched at.
        let left = json!([
            worn(HELM, "Rune full helm", 0),
            worn(1704, "Amulet of glory", 2),
            worn(3791, "Leather boots", 10),
        ]);
        let helm = call(&data, token, page.clone(), json!({ "equipment": left }));
        assert_eq!(helm["kind"], "unequip", "{helm}");
        assert_eq!(helm["name"], "Rune full helm", "{helm}");
        assert_eq!(stripped(), vec!["Rune full helm".to_string()]);
        // Both landed in the pack. The deposit pass is the matcher over the
        // posted pack page and never the listed names: the dagger id that was
        // never listed is first, because it is first in posted order — and it
        // is still not listed after it is deposited.
        let worn_free = json!([
            worn(1704, "Amulet of glory", 2),
            worn(3791, "Leather boots", 10),
        ]);
        let both = json!([
            inv(DDS_POISONED, "Dragon dagger(p)", 1),
            inv(HELM, "Rune full helm", 1),
        ]);
        let bank_ready = json!({
            "equipment": worn_free,
            "inv": both,
            "here": here(2810, 3350, 0),
            "nearest_booth": booth_page(),
        });
        let walked = call(&data, token, page.clone(), bank_ready.clone());
        assert_eq!(walked["kind"], "walk-nearest-bank", "{walked}");
        let opened = call(&data, token, page.clone(), bank_ready.clone());
        assert_eq!(opened["kind"], "open-booth", "{opened}");
        let banked = call(
            &data,
            token,
            page.clone(),
            json!({ "equipment": worn_free, "inv": both, "bank_open": true }),
        );
        assert_eq!(banked["kind"], "deposit", "{banked}");
        assert_eq!(
            banked["name"], "Dragon dagger(p)",
            "the pack's own regex match is the candidate, listed or not: {banked}"
        );
        assert_eq!(
            stripped(),
            vec!["Rune full helm".to_string()],
            "and the dagger stays off the list"
        );
        // The helm's own deposit follows in posted order.
        let helm_pack = json!([inv(HELM, "Rune full helm", 1)]);
        let banked = call(
            &data,
            token,
            page.clone(),
            json!({ "equipment": worn_free, "inv": helm_pack, "bank_open": true }),
        );
        assert_eq!(banked["kind"], "deposit", "{banked}");
        assert_eq!(banked["name"], "Rune full helm", "{banked}");
        // A restricted name the player carried and never wore is the same
        // candidate: the predicate is the matcher over the pack page, so a
        // spare weapon that was never on the worn page is banked too — and it
        // is not listed either, because the list is the worn rows'.
        let carried = json!([inv(1181, "Rune platebody", 1)]);
        let banked = call(
            &data,
            token,
            page.clone(),
            json!({ "equipment": worn_free, "inv": carried, "bank_open": true }),
        );
        assert_eq!(banked["kind"], "deposit", "{banked}");
        assert_eq!(banked["name"], "Rune platebody", "{banked}");
        assert_eq!(
            stripped(),
            vec!["Rune full helm".to_string()],
            "a carried name is deposited but never listed"
        );
        assert!(owns());
        // Nothing restricted left in the pack: the interface closes and the
        // row's own search arm waits on the decoded tile with no locs posted.
        let settled = call(
            &data,
            token,
            page.clone(),
            json!({
                "equipment": worn_free,
                "here": here(ENTRANA_X, ENTRANA_Z, 0),
                "bank_open": true,
            }),
        );
        assert_eq!(settled["kind"], "close", "{settled}");
        let settled = call(
            &data,
            token,
            page.clone(),
            json!({
                "equipment": worn_free,
                "here": here(ENTRANA_X, ENTRANA_Z, 0),
            }),
        );
        assert_eq!(settled["kind"], "wait", "{settled}");
        assert!(
            !settled.to_string().contains("Amulet"),
            "a name the frozen matcher lets through is never dispatched at: {settled}"
        );
        assert!(owns(), "the helm is listed, so the adapter reads true");
        on_reset();
    }

    /// The strip's deposit pass: the listed names the posted pack holds are
    /// deposited at the posted booth — the walk, the booth's own open, one
    /// deposit per name, the close — and only then does the row's own arm walk.
    #[test]
    fn the_strip_deposits_the_listed_names_at_the_posted_booth() {
        on_reset();
        let data = selected();
        let page = json!([[ENTRANA, 1]]);
        let token = steady(&data, ENTRANA);
        let helm = call(
            &data,
            token,
            page.clone(),
            json!({ "equipment": json!([worn(HELM, "Rune full helm", 0)]) }),
        );
        assert_eq!(helm["kind"], "unequip", "{helm}");
        assert_eq!(stripped(), vec!["Rune full helm".to_string()]);
        // The unequip landed: the worn page is empty and the pack holds it.
        let pack = json!([inv(HELM, "Rune full helm", 1)]);
        let banked = json!({
            "equipment": json!([]),
            "inv": pack,
            "here": here(2810, 3350, 0),
            "nearest_booth": booth_page(),
        });
        let walked = call(&data, token, page.clone(), banked.clone());
        assert_eq!(walked["kind"], "walk-nearest-bank", "{walked}");
        assert_eq!(walked["token"], token, "{walked}");
        assert_no_completion(&walked);
        let opened = call(&data, token, page.clone(), banked.clone());
        assert_eq!(opened["kind"], "open-booth", "{opened}");
        assert_eq!(opened["x"], 2810, "{opened}");
        assert_eq!(opened["z"], 3350, "{opened}");
        assert_eq!(opened["id"], 2213, "{opened}");
        assert_eq!(opened["name"], "Bank booth", "{opened}");
        assert_eq!(opened["action"], "Use-quickly", "{opened}");
        // The interface is up: the deposit goes out by the name the strip put
        // in the pack.
        let deposited = call(
            &data,
            token,
            page.clone(),
            json!({ "inv": pack, "bank_open": true }),
        );
        assert_eq!(deposited["kind"], "deposit", "{deposited}");
        assert_eq!(deposited["name"], "Rune full helm", "{deposited}");
        // It landed: the interface closes, and the call after that settles the
        // strip and runs the row's own search arm — the name stays listed.
        let closed = call(
            &data,
            token,
            page.clone(),
            json!({ "inv": json!([]), "bank_open": true }),
        );
        assert_eq!(closed["kind"], "close", "{closed}");
        let walked_on = call(
            &data,
            token,
            page.clone(),
            json!({
                "inv": json!([]),
                "bank_open": false,
                "here": here(ENTRANA_X, ENTRANA_Z, 0),
            }),
        );
        assert_eq!(walked_on["kind"], "wait", "{walked_on}");
        assert_eq!(stripped(), vec!["Rune full helm".to_string()]);
        assert!(owns());
        on_reset();
    }

    /// The restore in front of the three-step latch: no `'clue solved'`, no
    /// `grind-ready` and no `done` while a listed name is unclaimed, the pack's
    /// own name worn back on first, and the three kinds in order once the list
    /// empties.
    #[test]
    fn the_restore_runs_before_the_whole_three_step_latch() {
        on_reset();
        let data = selected();
        // One strip first, so this session owns a listed name.
        let page = json!([[ENTRANA, 1]]);
        let token = steady(&data, ENTRANA);
        let helm = call(
            &data,
            token,
            page,
            json!({ "equipment": json!([worn(HELM, "Rune full helm", 0)]) }),
        );
        assert_eq!(helm["kind"], "unequip", "{helm}");
        assert_eq!(stripped(), vec!["Rune full helm".to_string()]);
        assert!(owns());
        // The casket collect runs to its own bound with the name still listed.
        let casket = casket_of(&data, CLUE);
        let token = opened(&data, casket);
        let pack = json!([inv(HELM, "Rune full helm", 1)]);
        let scene = pages(json!([]), pack.clone(), json!(28), json!(-1));
        let waiting = call(&data, token, json!([]), scene.clone());
        assert_eq!(waiting["kind"], "wait", "{waiting}");
        assert_no_completion(&waiting);
        force_bound();
        // The collect is over and the reclaim owns the token: the pack's own
        // name goes back on, and no completion kind rides it.
        let wear_back = call(&data, token, json!([]), scene.clone());
        assert_eq!(wear_back["kind"], "wear", "{wear_back}");
        assert_eq!(wear_back["name"], "Rune full helm", "{wear_back}");
        assert_eq!(wear_back["token"], token, "{wear_back}");
        assert_no_completion(&wear_back);
        assert!(owns(), "the name is listed until it is worn again");
        // The worn page shows it again: the list empties and the latch runs its
        // three steps on this same live token.
        let back_on = json!({
            "here": loot_tile(),
            "ground": json!([]),
            "inv": json!([]),
            "inv_size": 28,
            "main_modal_id": -1,
            "equipment": json!([worn(HELM, "Rune full helm", 0)]),
        });
        let solved = call(&data, token, json!([]), back_on.clone());
        assert_eq!(solved["kind"], "callback.setStatus", "{solved}");
        assert_eq!(solved["message"], "clue solved", "{solved}");
        assert!(stripped().is_empty(), "the restore empties the list");
        assert!(!owns(), "and the adapter's own read follows it");
        let ready = call(&data, token, json!([]), back_on.clone());
        assert_eq!(ready["kind"], "grind-ready", "{ready}");
        assert_eq!(ready["token"], token, "{ready}");
        let done = call(&data, token, json!([]), back_on);
        assert_eq!(done["kind"], "done", "{done}");
        on_reset();
    }

    /// The restore's bank leg: the names the pack does not hold are walked to
    /// the posted stand for, opened, claimed one `Withdraw-1` at a time, and the
    /// interface closed before anything is worn back on.
    #[test]
    fn the_restore_claims_a_missing_name_at_the_bank() {
        on_reset();
        let data = selected();
        let page = json!([[ENTRANA, 1]]);
        let token = steady(&data, ENTRANA);
        let helm = call(
            &data,
            token,
            page,
            json!({ "equipment": json!([worn(HELM, "Rune full helm", 0)]) }),
        );
        assert_eq!(helm["kind"], "unequip", "{helm}");
        assert_eq!(stripped(), vec!["Rune full helm".to_string()]);
        // The collect ends with nothing in the pack: the walk to the stand.
        let casket = casket_of(&data, CLUE);
        let token = opened(&data, casket);
        let scene = pages(json!([]), json!([]), json!(28), json!(-1));
        let _ = call(&data, token, json!([]), scene.clone());
        force_bound();
        let banked = json!({
            "here": loot_tile(),
            "ground": json!([]),
            "inv": json!([]),
            "inv_size": 28,
            "main_modal_id": -1,
            "equipment": json!([]),
            "nearest_booth": booth_page(),
        });
        let walked = call(&data, token, json!([]), banked.clone());
        assert_eq!(walked["kind"], "walk-nearest-bank", "{walked}");
        assert_no_completion(&walked);
        // Arrived beside the posted booth: its own open, and then the claim.
        let arrived = json!({
            "here": here(2810, 3350, 0),
            "ground": json!([]),
            "inv": json!([]),
            "inv_size": 28,
            "main_modal_id": -1,
            "equipment": json!([]),
            "nearest_booth": booth_page(),
        });
        let opened_booth = call(&data, token, json!([]), arrived.clone());
        assert_eq!(opened_booth["kind"], "open-booth", "{opened_booth}");
        let open = json!({
            "here": here(2810, 3350, 0),
            "ground": json!([]),
            "inv": json!([]),
            "inv_size": 28,
            "main_modal_id": -1,
            "equipment": json!([]),
            "nearest_booth": booth_page(),
            "bank_open": true,
        });
        let claimed = call(&data, token, json!([]), open.clone());
        assert_eq!(claimed["kind"], "withdraw", "{claimed}");
        assert_eq!(claimed["name"], "Rune full helm", "{claimed}");
        assert_eq!(claimed["action"], "Withdraw-1", "{claimed}");
        assert_no_completion(&claimed);
        // The claim landed in the pack: the open interface closes first, and
        // only then does the wear pass put the name back on.
        let claimed_open = json!({
            "here": here(2810, 3350, 0),
            "ground": json!([]),
            "inv": json!([inv(HELM, "Rune full helm", 1)]),
            "inv_size": 28,
            "main_modal_id": -1,
            "equipment": json!([]),
            "nearest_booth": booth_page(),
            "bank_open": true,
        });
        let closed = call(&data, token, json!([]), claimed_open);
        assert_eq!(closed["kind"], "close", "{closed}");
        let claimed_closed = json!({
            "here": here(2810, 3350, 0),
            "ground": json!([]),
            "inv": json!([inv(HELM, "Rune full helm", 1)]),
            "inv_size": 28,
            "main_modal_id": -1,
            "equipment": json!([]),
            "nearest_booth": booth_page(),
            "bank_open": false,
        });
        let wear_back = call(&data, token, json!([]), claimed_closed);
        assert_eq!(wear_back["kind"], "wear", "{wear_back}");
        assert_eq!(wear_back["name"], "Rune full helm", "{wear_back}");
        assert_no_completion(&wear_back);
        // Worn again: the list empties and the exact status goes out.
        let back_on = json!({
            "here": here(2810, 3350, 0),
            "ground": json!([]),
            "inv": json!([]),
            "inv_size": 28,
            "main_modal_id": -1,
            "equipment": json!([worn(HELM, "Rune full helm", 0)]),
        });
        let solved = call(&data, token, json!([]), back_on);
        assert_eq!(solved["kind"], "callback.setStatus", "{solved}");
        assert_eq!(solved["message"], "clue solved", "{solved}");
        assert!(stripped().is_empty());
        on_reset();
    }

    /// A listed name the page never puts back on is the named
    /// `restore-incomplete` log — never a machine kind and never a completion
    /// kind — and the list keeps it for the next attempt.
    #[test]
    fn a_name_that_will_not_go_back_on_stays_listed_and_logs() {
        on_reset();
        let data = selected();
        let page = json!([[ENTRANA, 1]]);
        let token = steady(&data, ENTRANA);
        let helm = call(
            &data,
            token,
            page,
            json!({ "equipment": json!([worn(HELM, "Rune full helm", 0)]) }),
        );
        assert_eq!(helm["kind"], "unequip", "{helm}");
        let casket = casket_of(&data, CLUE);
        let token = opened(&data, casket);
        let scene = pages(json!([]), json!([]), json!(28), json!(-1));
        let _ = call(&data, token, json!([]), scene.clone());
        force_bound();
        // The restore attempt: nothing in the pack, no booth posted at all, so
        // the walk goes out and the window is what ends the attempt.
        let bare = json!({
            "here": loot_tile(),
            "ground": json!([]),
            "inv": json!([]),
            "inv_size": 28,
            "main_modal_id": -1,
            "equipment": json!([]),
        });
        let walked = call(&data, token, json!([]), bare.clone());
        assert_eq!(walked["kind"], "walk-nearest-bank", "{walked}");
        force_bound();
        let failed = call(&data, token, json!([]), bare.clone());
        assert_eq!(failed["kind"], "callback.log", "{failed}");
        let message = failed["message"].as_str().unwrap_or("");
        assert!(
            message.contains("restore-walk-failed"),
            "the named bank failure: {failed}"
        );
        assert_no_completion(&failed);
        assert_eq!(stripped(), vec!["Rune full helm".to_string()]);
        // The next call starts a fresh attempt, and the list is still what the
        // adapter reads: nothing posts the status while it is non-empty.
        let again = call(&data, token, json!([]), bare.clone());
        assert_eq!(again["kind"], "walk-nearest-bank", "{again}");
        assert!(owns());
        // A claim the posted bank never lands: the attempt gives up on the
        // name, the interface closes, and what is still missing is the named
        // `restore-incomplete` — the list keeps it and the latch stays blocked.
        let open = json!({
            "here": loot_tile(),
            "ground": json!([]),
            "inv": json!([]),
            "inv_size": 28,
            "main_modal_id": -1,
            "equipment": json!([]),
            "bank_open": true,
        });
        let claimed = call(&data, token, json!([]), open.clone());
        assert_eq!(claimed["kind"], "withdraw", "{claimed}");
        assert_eq!(claimed["name"], "Rune full helm", "{claimed}");
        assert_eq!(claimed["action"], "Withdraw-1", "{claimed}");
        let closed = call(&data, token, json!([]), open.clone());
        assert_eq!(closed["kind"], "close", "{closed}");
        let down = json!({
            "here": loot_tile(),
            "ground": json!([]),
            "inv": json!([]),
            "inv_size": 28,
            "main_modal_id": -1,
            "equipment": json!([]),
            "bank_open": false,
        });
        let incomplete = call(&data, token, json!([]), down.clone());
        assert_eq!(incomplete["kind"], "callback.log", "{incomplete}");
        let message = incomplete["message"].as_str().unwrap_or("");
        assert!(
            message.contains("restore-incomplete") && message.contains("Rune full helm"),
            "the named re-equip failure carries the names: {incomplete}"
        );
        assert_eq!(incomplete["token"], token, "{incomplete}");
        assert_no_completion(&incomplete);
        assert_eq!(stripped(), vec!["Rune full helm".to_string()]);
        // And the reclaim retries rather than finishing: a fresh attempt walks
        // again and no completion kind ever rides the still-listed name.
        let retry_walk = call(&data, token, json!([]), down);
        assert_eq!(retry_walk["kind"], "walk-nearest-bank", "{retry_walk}");
        assert_no_completion(&retry_walk);
        on_reset();
    }

    /// `ownsEquipment` is the machine's own list and `retry` is the machine's
    /// own latch clear: retry never aborts the live token and never touches the
    /// list.
    #[test]
    fn owns_equipment_reads_the_list_and_retry_clears_only_the_latch() {
        on_reset();
        let data = selected();
        let page = json!([[ENTRANA, 1]]);
        assert!(!owns(), "an empty list owns nothing");
        let token = steady(&data, ENTRANA);
        let helm = call(
            &data,
            token,
            page.clone(),
            json!({ "equipment": json!([worn(HELM, "Rune full helm", 0)]) }),
        );
        assert_eq!(helm["kind"], "unequip", "{helm}");
        assert!(owns());
        let retried = dispatch(Some(&data), &json!({ "op": "retry" }));
        assert_eq!(retried["kind"], "retry", "{retried}");
        assert_eq!(retried["token"], token, "{retried}");
        assert!(owns(), "the list is not retry's: {:?}", stripped());
        // The live token is still the machine's own: its next step answers.
        let live = call(
            &data,
            token,
            page.clone(),
            json!({
                "equipment": json!([]),
                "inv": json!([inv(HELM, "Rune full helm", 1)]),
            }),
        );
        assert_eq!(live["kind"], "walk-nearest-bank", "{live}");
        on_reset();
    }

    /// A connection boundary keeps the session's own strip list: `on_reset`
    /// drops the live step and its token and nothing else, so the reclaim the
    /// strip already owes survives a relog and `ownsEquipment` still reads
    /// true. The fresh instance `on_stop` is what clears it.
    #[test]
    fn a_connection_boundary_keeps_the_stripped_list_and_a_stop_clears_it() {
        on_reset();
        let data = selected();
        let page = json!([[ENTRANA, 1]]);
        let token = steady(&data, ENTRANA);
        let helm = call(
            &data,
            token,
            page.clone(),
            json!({ "equipment": json!([worn(HELM, "Rune full helm", 0)]) }),
        );
        assert_eq!(helm["kind"], "unequip", "{helm}");
        assert_eq!(stripped(), vec!["Rune full helm".to_string()]);
        assert!(owns());
        // The connection boundary: the live step and its token are gone.
        on_reset();
        assert_eq!(
            dispatch(Some(&data), &json!({ "op": "next", "token": token }))["kind"],
            "aborted",
            "the boundary kills the live token"
        );
        assert_eq!(
            stripped(),
            vec!["Rune full helm".to_string()],
            "the list the reclaim still owes outlives it"
        );
        assert!(owns(), "and the adapter still reads it");
        // A fresh session on the live machine still owes that reclaim: a begin
        // on the casket row reaches the collect, and its exit walks to the bank
        // for the name the earlier session banked, because the list outlived
        // the boundary.
        let casket = casket_of(&data, CLUE);
        let again = opened(&data, casket);
        let quiet = pages(json!([]), json!([]), json!(28), json!(-1));
        let _ = call(&data, again, json!([]), quiet.clone());
        force_bound();
        let walk = call(&data, again, json!([]), quiet);
        assert_eq!(walk["kind"], "walk-nearest-bank", "{walk}");
        // The fresh task instance clears the list with the step.
        on_stop();
        assert!(
            stripped().is_empty(),
            "Stop starts the session over: {:?}",
            stripped()
        );
        assert!(!owns(), "and the adapter reads an empty list");
        on_reset();
    }

    /// The restore's make-room deposit: a full pack at the trail's end banks
    /// what the frozen predicate takes before the claim goes out, so the
    /// withdrawn name has a slot to land in — and a bank that never lands the
    /// deposit does not spin the same verb.
    #[test]
    fn a_full_pack_is_made_room_for_before_the_reclaim_claim() {
        on_reset();
        let data = selected();
        let page = json!([[ENTRANA, 1]]);
        let token = steady(&data, ENTRANA);
        let helm = call(
            &data,
            token,
            page.clone(),
            json!({ "equipment": json!([worn(HELM, "Rune full helm", 0)]) }),
        );
        assert_eq!(helm["kind"], "unequip", "{helm}");
        // The strip banks the helm, so the reclaim has to fetch it back.
        let banked = json!({
            "equipment": json!([]),
            "inv": json!([inv(HELM, "Rune full helm", 1)]),
            "here": here(2810, 3350, 0),
            "nearest_booth": booth_page(),
            "bank_open": true,
        });
        let _ = call(&data, token, page.clone(), banked.clone());
        let deposited = call(&data, token, page.clone(), banked.clone());
        assert_eq!(deposited["kind"], "deposit", "{deposited}");
        let _ = call(
            &data,
            token,
            page.clone(),
            json!({ "inv": json!([]), "bank_open": true, "nearest_booth": booth_page() }),
        );
        // The collect runs to its bound with the pack full of loot: 28 posted
        // rows and no room for the helm.
        let casket = casket_of(&data, CLUE);
        let token = opened(&data, casket);
        let full = full_pack();
        let scene = json!({
            "here": loot_tile(),
            "ground": json!([]),
            "inv": full,
            "inv_size": 28,
            "main_modal_id": -1,
            "equipment": json!([]),
        });
        let _ = call(&data, token, json!([]), scene.clone());
        force_bound();
        // The bank trip first, then the make-room deposit, and only then the
        // claim: the frozen `restoreStrippedGear` order.
        let open = json!({
            "here": here(2810, 3350, 0),
            "ground": json!([]),
            "inv": full,
            "inv_size": 28,
            "main_modal_id": -1,
            "equipment": json!([]),
            "nearest_booth": booth_page(),
            "bank_open": true,
        });
        let room = call(&data, token, json!([]), open.clone());
        assert_eq!(room["kind"], "deposit", "{room}");
        assert_eq!(
            room["name"], "Big bones",
            "a non-want row the trail facts do not name is banked to make room: {room}"
        );
        assert_no_completion(&room);
        // It landed: one slot free and the claim goes out.
        let freed = json!({
            "here": here(2810, 3350, 0),
            "ground": json!([]),
            "inv": full_minus_one(),
            "inv_size": 28,
            "main_modal_id": -1,
            "equipment": json!([]),
            "nearest_booth": booth_page(),
            "bank_open": true,
        });
        let claimed = call(&data, token, json!([]), freed.clone());
        assert_eq!(claimed["kind"], "withdraw", "{claimed}");
        assert_eq!(claimed["name"], "Rune full helm", "{claimed}");
        assert_eq!(claimed["action"], "Withdraw-1", "{claimed}");
        // The claim did not land inside one more call: `tried` ends the deposit
        // pass, the interface closes, and what is still missing is the named
        // log — the list keeps it, so the latch stays blocked. No second
        // deposit of the same row and no unbounded loop.
        let stuck = call(&data, token, json!([]), freed.clone());
        assert_eq!(stuck["kind"], "close", "{stuck}");
        let down = json!({
            "here": here(2810, 3350, 0),
            "ground": json!([]),
            "inv": full_minus_one(),
            "inv_size": 28,
            "main_modal_id": -1,
            "equipment": json!([]),
            "nearest_booth": booth_page(),
            "bank_open": false,
        });
        let incomplete = call(&data, token, json!([]), down);
        assert_eq!(incomplete["kind"], "callback.log", "{incomplete}");
        let message = incomplete["message"].as_str().unwrap_or("");
        assert!(message.contains("restore-incomplete"), "{incomplete}");
        assert_eq!(stripped(), vec!["Rune full helm".to_string()]);
        on_reset();
    }
}
