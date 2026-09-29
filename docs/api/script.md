# Scripts (`crates/script`)

Script **kernel** for the bot host. Compiled cards are rust-first on 274bot
`api`. Catalog and external TypeScript/JavaScript bots load through a thin
compatibility shim; **behavior, configuration, and fail-closed helpers are
owned in Rust**. The shim does coercion and marshaling only. The only
JS ↔ Rust isolate wire is **FlatBuffers** — there is no extra JSON host wire
and no foreign JS policy runtime in-tree.

WalkTo is **host nav** (panel picker / TUI map), not a script card.
`ctx::Script::on_random` is a rising-edge knock (`RandomClaim::Host` default).
Catalog cards come from an external `$RS2B0T` / `--catalog` checkout
(upstream `rs2b2t/rs2b0t` layout: `src/bot/scripts`), not a copy in this
tree. `$RS2B0T` wins over the persisted root (`~/.274bot/rs2b0t-path`,
written on the first successful catalog parse). Scripts run on whatever
world the bound **server profile** logged into (see [README.md](../../README.md)).

Compatibility is **partial**. Unsupported helpers and options fail
explicitly. Do not treat runner PASS, a single live gold, or a historical
inventory count as “all catalog scripts / all options qualified.”

## Native contract boundary

New compiled-card consumers use `script::native::Script`. The currently
installed registry and slot use `script::ctx::Script` until their lifecycle
cutover; there is no ambiguous crate-root `Script` re-export.

`native::ActionContext` exposes borrowed frame/evidence/pin/recovery views,
and `quest_journal::{JournalRequest, JournalMachine}` and `clue::ClueRecovery`
are available without `load`. These declarations do not install a native
action facility: begins/polls refuse with `ActionError::Unavailable`, and no
public constructor can mint a frame, handle or quiet lease. Isolate lifecycle
hooks remain crate-private and behind `load`.

Selected fact strings can be shared with a decode-local `api::selected::FactStrings`;
drop the interner after preparation and let the family-held Arcs own their
lifetimes. `FactKey::new` has no global intern pool. `RunKey.slot` comes from
Play's worker lifetime owner, not a second allocator. Evidence freshness
compares `(tick, sequence)` lexicographically within that exact run/session.
Authorize quest gates through `QuestCatalog::test_gate`, not raw numeric
signal ranges. Fact-record optional fields require explicit `null` when no
value applies; omission is not proof of no requirement.

## Two runners

| | Compiled (Browse our cards) | Load / catalog (operator file or `$RS2B0T`) |
| --- | --- | --- |
| When V8 exists | Never | Start of a **JS/TS** picker card only |
| Wake | `host::should_emit_tick` (PLAYER_INFO) | same, posted to the isolate thread |
| House API | Rust `tick(&mut ScriptCtx)` | `export function tick` **or** `defineBot`; explicit `export const apiVersion = 2` is JS API v2 ([js-api-v2.md](js-api-v2.md)) |

Idle = no isolate. Stop tears down V8. Pause / not `is_up` keeps the
instance; `want_run` distinguishes operator Pause from offline.

An offline slot logs in while a login is wanted (auto-login or a Log in) or a
script is running or paused on it (rs2b0t's `autoLogin || scriptActive()`,
re-evaluated on every title-loop pass), unless the operator logged the slot
out. A dropped connection relogged that way pauses a Load script whole: every
await, step machine and task runtime stays, their clocks and the `Execution`
wait clock stop, and the script resumes on the relogged session's first
tick. A repeated-exit guard parks automatic login but holds the work the same
way; an explicit **Log in** clears the guard and resumes it. The slot then
re-sends the script walk it was following, and the last eight walk, walk-near
and abort-walk requests the script queued that never reached the dropped
connection. Other unsent requests are dropped; their owners retry on their own
timeouts. An operator logout, Stop, or slot removal ends the session instead:
the in-flight machine rows and task runtimes end (`aborted`, `reset`). Compiled
scripts end their live step at either boundary.

### File Load and catalog cards

- **Load** registers a picker card tagged **File** from an absolute/relative
  path (`~/.274bot/js-scripts.json` remembers `{name, path}`). Same path
  overwrites; different paths stay distinct even when stems match.
- Compiled registry cards (currently Sherlock in `load` builds) are also in
  Browse, grouped under **Treasure Trails** and tagged **Compiled**. They use
  no parameter schema. Compiled names remain reserved.
- Catalog cards are **Catalog** source: static parse of
  `src/bot/scripts/index.ts` (no V8 at registration). Browse fills both
  panel and TUI pickers.
- Isolate scripts cannot use browser timer APIs (`setTimeout`, `setInterval`,
  `clearTimeout`, or `clearInterval`); calls throw a catchable error. Use
  `Execution.delayTicks` for game-time waits.
- Isolate is its own OS thread; a non-yielding execution has a **600 ms
  runaway horizon** measured from that execution's own start; **64 MB V8 heap
  cap** (`heap_limits(0, 64 MiB)`). A confirmed watchdog, Pause-deadline, or
  session-reset cut is logged and recreates the script runtime (still paused
  after an operator Pause). Three cuts within five minutes stop the script
  with an error instead of restarting forever. The isolate starts small and
  grows with the live set — not 64 MB reserved per card. Over the heap cap,
  the isolate is terminated. Extra RSS (OS thread, deno/rustyscript, code
  space) sits on top of JS heap and is **unmeasured** at the 50-slot wall —
  `rss_ladder` is Null/draw-off clients, not Started JS.

### Persistence

`~/.274bot/js-scripts.json` (the Load list) and `~/.274bot/rs2b0t-path` (the
catalog root) decide which files the host later reads and runs, so restore
treats them as untrusted input:

- Both are written atomically with mode `0o600` and read with
  `vault::read_private_file`: a file that is not a regular file, is over its
  bound, belongs to another user (root is accepted) or is writable by its group
  or by others is **refused**; a file others can only read (written before the
  `0o600` writer) is tightened to `0o600` and read
  ([vault.md](vault.md#state-files-beside-the-vault)). On Unix a filesystem
  that reports every file `0o777` is therefore refused too. Windows has no such
  mode to check and no ACL is inspected: the directory's inherited ACL is the
  only protection there.
- A refused `js-scripts.json` restores nothing, is reported on stderr (and in
  the panel log), and nothing is saved over it until the operator fixes or
  removes it: **Load** then reports that it could not save instead of replacing
  the file. A refused `rs2b0t-path` counts as no catalog root and is reported
  once; choosing the catalog again writes a fresh file.
- Paths are stored absolute and `..`-free (a relative Load path is resolved
  against the working directory when it is loaded). Restore skips, and lists as
  a load failure (“not restored: …”), an entry whose path is relative, has
  `..`, or names something that is not a regular file or is over 8 MiB; a
  missing file is dropped quietly as before. Each source is opened once
  (without blocking) and its shape and size are checked on that open file, so a
  swap after the check cannot substitute a FIFO or an endless file. Script
  sources themselves are not permission-checked: they live wherever the
  operator keeps them.

### Content-addressed transpile cache

Origin bytes (TS or JS) are hashed with **SHA-256**. Hits under
`~/.274bot/js-cache/objects/<hex>.js` skip transpile; misses transpile `.ts`
(plain `.js` stored verbatim) and update `manifest.json`. Reload that sees
unchanged origin bytes reports nothing changed and does not re-transpile.
Sibling/import dependencies participate in the prepared-card fingerprint
used by manual reload.

### Per-profile assignment and settings

A successful **Start** persists `ProfileSettings.script_assignment` (source,
path/name identity, optional `unavailable` reason) and merges
`script_settings` for that card identity into the vault profile. Focus
restore prefers a pending Browse selection, then the saved assignment.
Parameters **Edit** writes the focused profile’s bag (typed editors honour
`showIf` / `group`; File Load parses `export const SETTINGS` with no V8)
and persists it under that card identity. Values a script reads only at
**Start** take effect on the next Start. Live edits push into a matching
**running or paused** isolate without restart (next relevant tick path) —
there is no separate settings API beyond the panel/TUI editors and the
vault bag.

Missing catalog/file sources stay visible with `unavailable` rather than
silently dropping the assignment.

## Operator controls

Browse / Start / Pause / Stop / Load are wired in both operator panels
(`panel-play` script chrome and `tui-play` script pane) over the same
`host_play::Play` dispatch. Both surfaces expose Reload and the TUI Script
tab also exposes Start all / Stop all; the native panel additionally exposes
Refresh catalog and the MultiBox bulk script controls described below.

| Control | Behavior |
| --- | --- |
| **Browse…** | Pick a compiled or loaded/catalog card for the focused profile (pending until Start). |
| **Load** | Open a file picker; register/transpile a File card. Disabled while a script is active on the focus. |
| **Reload** | Hash current card origin (+ siblings). Unchanged → “Nothing changed; nothing to reload”. Changed with no running/paused owners → apply. Changed with owners → **Confirm** / **Cancel reload**: running bots that still match the warned generation **restart**; named **paused** bots are **Stopped** (not left half-reloaded). |
| **Start / Pause / Resume / Stop** | Focused profile only. |
| **Start all / Stop all** | Bulk script controls (panel MultiBox rail; TUI Script tab `T` / `E`). Separate from **Login all / Logout all**. Start all skips already running/paused/stopping members; Stop all stops running and paused across wall members and live slots. |
| **Refresh catalog** | Re-scan `$RS2B0T` / catalog root. Unchanged scan → “Nothing changed.” Changed with owners → confirm; same restart/stop policy as manual reload. |

Script paint (`ScriptPaint`) draws over the Game chatbox in the panel and
replaces the chat pane in the TUI (`p` toggles back to the game ring).

**Live gold example:** `panel-play --live script_bone_burier` (and its TUI
twin) can start the real `$RS2B0T` BoneBurier card on a unique minted
account when the catalog and engine are present — a supported-path proof,
not catalog-wide qualification. `scenario::ScenarioSettings.start_script`
names the card; the host fills the catalog and dispatches
`script_start_load` at live boot.

External raw-TypeScript smoke for the suite runner is a separate
`external_watch` / `--external-ts` path ([../e2e-suite.md](../e2e-suite.md));
ordinary operator Load is the File/catalog flow above.

## ScriptCtx read surface

```rust
pub struct ScriptCtx<'a> {
    pub driver: &'a mut dyn Driver,
    pub tick: u64,
    pub here: Option<(i32, i32, i32)>,           // local player world tile
    pub walk: Option<&'a mut dyn FnMut(i32, i32, i32) -> bool>, // arm find + follow
    pub inv: Option<&'a [(i32, i32)]>,            // (real obj id, count)
    pub obj_names: Option<&'a ObjNames>,          // id -> name table
}
impl ScriptCtx<'_> { pub fn has_item(&self, name: &str) -> bool; }
```

`has_item` resolves real obj ids case-insensitively. Inventory ids are real
(`stored - 1`), matching `ItemDefView.id` and `ObjNames`.

## Nav vs scripts

WalkTo stays the panel **WalkTo** button + traveller. `ctx.walk(x, z, level)`
arms `nav::router::find` on the shared whole-world `NavWorld` and the slot
pump drives `nav::traveller::Traveller::follow`; `SlotStatus.walk_{x,z,level}`
mirrors the armed dest and clears on arrival. The nav `find` runs off-pump
(a short-lived worker); `follow` steps on the slot pump, one send per tick.

## Catalog walking and recovery (compat v1)

The rs2b0t walk and reach helpers are Rust step machines; the shim passes
arguments and awaits one completion.

- **`Traversal.walkResilient`** runs the frozen ladder: baked walk, scene
  step (`sceneRadius`, default `radius + 1`), door-or-step unstick, backoff,
  and after three no-progress passes a verify probe (one route preview from
  here, 30 s). A probe with no route, or the same end as the last probe,
  ends the walk as unreachable; a fresh one resets the passes. Teleports
  follow `useTeleportCatalog` / `policy.useTeleports` (an explicit false
  wins; unset is off) and `policy.distanceBeforeTeleport` (the route span
  must reach it). A settled blocked route end returns true. `maxBudget` is
  accepted: it bounds frozen's PathFinder, and the host router searches
  every walk and the probe to its own 4,000,000-node bound, never less.
  `avoidZones` rectangles ride the walk and the probe, and every host search
  of the walk keeps out of them; a catalog zone id (not a host table), an
  inverted rectangle or more than 16 fail with `not impl`.
  `bankItemCounts` is not an input (the host bank fetch reads the bank).
- **`Traversal.walkTo`** is one Rust walk: radius 2 and 300 s by default,
  the same teleport rules and `avoidZones` as `walkResilient`,
  `maxExpansions` accepted as `maxBudget` is, and the card's `Sustain` hook
  once a tick while it walks. A random event ends it false and stops its
  route. Walking off Karamja without the 30-coin fare (the navigator names
  only the fare as missing), it earns the fare at Luthas's plantation and
  walks once more. Teleport id lists, ship or shortcut exclusion,
  `pathFollow` and `forceRepath` fail with `not impl`.
- The host owns a script's active route. Operator Pause ends the follow
  and keeps the route as carried state, as a reconnect does; the first
  dispatch after Resume sends it once more under its own request id,
  whichever script code asked for it (walks, reach, the hunt steppers, a
  bank open's booth walk, raw v2 requests, `walkNearestBank` while it is
  still choosing). Nothing is re-sent when the script run changed, the
  walk was cancelled (an `AbortWalk`, a failed walk machine, Stop, or the
  script stopping itself), the player already stands within the walk's
  arrival radius, or a newer walk request replaces it; a bank trip planned
  for the walk is planned again from the current pack. A late copy of the
  request the host already follows is not sent twice. A request queued
  but not yet dispatched when Pause lands goes out once, as queued. A
  watchdog recovery walk interrupted by Pause is re-armed on Resume.
  Scene clicks (`WalkTo`, `DirectNavigator`) are not host routes and are
  not carried.
- **`createReturnToAnchorTask`:** `validate` is beyond the bot's leash plus
  slack. `execute` does nothing inside the arrive disk, walks a resilient leg
  first when farther than `longRangeTiles`, opens `obstacles` on the way
  with `walkOpening`, else walks once; `timeoutMs` defaults to 90 s.
- **`DirectNavigator.walk`** clamps the click to 48 tiles, clicks on the
  player's plane and returns false with no player tile or a target outside
  the loaded scene. **`walkTo`** re-checks every two ticks and re-clicks
  after 2400 ms or when the player did not move.
- **`Reach.npcDialog`** walks with the resilient ladder (close-in radius 3,
  stand radius 1, 4 attempts) and answers `unreachable` when that ladder
  proves the tile unreachable. The talk runs up to eight rounds: it opens or
  closes a door in front of an NPC the scene cannot reach, clears the door
  after "I can't reach that" (no door is `unreachable`), and talks again one
  tick after an unanswered round. A door approach is itself a resilient walk
  (radius 1, 3 attempts, 30 s). A random-event interrupt or a newer reach
  stops the walk the reach armed.
- **`RecoveryHints`** outlive a watchdog restart: the restarted script's
  `takeAnchor()` returns the anchor the stalled run latched; an unused hint
  clears once the new `onStart` succeeds.

## Hard no

No dummy tick-end opcode. No `Arc<World>` on extras. No bot action API in
`vendor/fr-client-rust`. No extra JSON host wire. Never Fairy-Ring.
