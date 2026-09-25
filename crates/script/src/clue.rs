//! Isolate-owned clue-session machine: `api.clue.begin` / `next`, the v1
//! SolveClue execute family, and Sherlock.
//!
//! One token per isolate over the landed held-step identify. Callers that
//! omit marshalled pages are filled from the isolate scene
//! ([`crate::observed`]); Sherlock and the unit tests still pass pages.
//! Verbs are [`verb_req`]. The v1 adapter's `execute` awaits the `clue`
//! family; `validate` is the enabled gate plus one begin. This machine owns
//! the token, the generation captured at begin, the frozen clock, the
//! identify call, the callback kinds and the idle end state.
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
//! An open chat closes the Talk-to for the tick: the posted `chat_modal_id`
//! beside `chat_continue` is the landed `dialog_ready`, so no walk and no
//! Talk-to goes out while it holds, and the posted `count_dialog_open` blocks
//! the same way. A posted `chat_continue` with no option list and no count
//! dialog is drained at the head of `Steady` — the frozen `drainChat` — so the
//! continue goes out before the casket Open or the Talk-to; a page that posted
//! only `chat_modal_id` still waits, and a posted option list is the
//! professor's to answer, not that drain.
//!
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
//! not advance the session. Nothing is cached here and there is no world
//! copy: the pages are the ones the wrapper hands in at call time, and the
//! player tile, the slot count and the `hold || ours` interrupt fall back to
//! the isolate scene when the wrapper omits them.

mod acquire;
mod combat;
mod entrana;
mod family;
mod puzzle;
mod scene;
mod shop;
mod talk;
mod verbs;

use acquire::*;
#[cfg(test)]
use combat::{keeper_type, key_step};
#[cfg(test)]
use entrana::{entrana_coord, entrana_restricted_gear};
pub(crate) use family::Clue;
#[allow(unused_imports)]
pub(crate) use family::ClueArgs;
use puzzle::*;
use scene::*;
use talk::*;
pub(crate) use verbs::verb_req;

use crate::food_policy::food_forms_for;
use crate::machine::{Begin, Call, Cx, Family, Reply, Step};
use crate::observed;
use crate::shim::InteractReq;
use crate::task_clock::InstantTaskClock;
use api::clue_logic::{identify_step, NONE_HELD};
use api::clue_pack::SHARK_ID;
use api::clue_puzzle::{self, Board, PuzzleRow};
use api::game_data::{
    SelectedGameData, TalkKeyKeeper, TalkKeyKeyRow, TalkKeyTalkRow, TrailMembershipRow,
    TrioGiverRow,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::borrow::Cow;
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
        // Captured at begin. A posted generation that does not match is
        // aborted; an omitted key (the Clue family payload) is not.
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
        if let Some(generation) = input.get("generation").and_then(Value::as_u64) {
            if generation != self.generation {
                // Reset, stop and a second begin abort silently; the first thing
                // the dead token hears about it is this error, never a kind.
                return self.aborted(ABORTED);
            }
        }
        if self.clock.frozen() {
            // Frozen: no callback, no verb and no burn. `resume` is not
            // consumed, so the gate is still unanswered after the thaw.
            return self.emit("wait");
        }
        if posted_hold(input) {
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
                // The frozen `drainChat`: a posted continue with no option
                // list and no count dialog is sent before the casket Open or
                // the step's own verb, so a giver chat still open when the
                // casket lands is continued rather than Opened through. A
                // page that posted only `chat_modal_id` is not a continue,
                // and a posted option list is the professor's to answer.
                if !count_open(input) && continue_posted(input) && !options_posted(input) {
                    return self.emit(CONTINUE);
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
        let Some(here) = posted_here(input) else {
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
        let Some(size) = posted_inv_size(input) else {
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
        let Some(here) = posted_here(input) else {
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

/// The frozen `paintClueProgress` rows this machine can state truthfully:
/// none when no token is live (the caller prints its own idle line), else the
/// machine's own phase and its last dispatched walk dest. Never an invented
/// clue name, leg, attempt, target or distance: this machine models none of
/// those, so it paints none of them.
fn paint_rows(rt: &ClueRuntime) -> Value {
    if matches!(rt.phase, Phase::Idle) {
        return Value::Array(Vec::new());
    }
    let mut rows = vec![json!({ "text": format!("clue: {}", phase_name(&rt.phase)) })];
    if let Some(dest) = rt.walk_dest {
        rows.push(json!({
            "text": format!("walk dest ({},{},{})", dest.x, dest.z, dest.level)
        }));
    }
    Value::Array(rows)
}

fn phase_name(phase: &Phase) -> &'static str {
    match phase {
        Phase::Idle => "idle",
        Phase::Gate => "gate",
        Phase::Reporting => "reporting",
        Phase::Steady => "steady",
        Phase::Collecting => "collecting",
    }
}

/// The ground row a Collecting call dispatched a Take for: the posted id that
/// tells the row apart, and the name its `took '…' from the casket` line
/// reports.
struct Take {
    id: i32,
    name: String,
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
        "begin" => {
            let input = hydrate(input);
            RUNTIME.with(|rt| rt.borrow_mut().begin(selected, &input))
        }
        "next" => {
            let input = hydrate(input);
            RUNTIME.with(|rt| rt.borrow_mut().next(selected, &input))
        }
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
        // The paint seat: the rows the frozen `paintClueProgress` shows for a
        // live clue, read by the shim as one crossing. `[]` sends the caller's
        // own idle line, exactly as the frozen no-clue branch does.
        "paint" => RUNTIME.with(|rt| paint_rows(&rt.borrow())),
        _ => json!({ "kind": "notImpl", "reason": "unknown clue op" }),
    }
}

#[cfg(test)]
#[path = "clue_tests.rs"]
mod tests;
