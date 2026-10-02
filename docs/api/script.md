# Scripts (`crates/script`)

Script **kernel** for the bot host. Compiled cards are rust-first on 274bot
`api`. Catalog and external TypeScript/JavaScript bots load through a thin
compatibility shim; **behavior, configuration, and fail-closed helpers are
owned in Rust**. The shim does coercion and marshaling only. The only
JS ↔ Rust isolate wire is **FlatBuffers** — there is no extra JSON host wire
and no foreign JS policy runtime in-tree.

WalkTo is **host nav** (panel picker / TUI map), not a script card.
`native::Script::on_random` is a rising-edge knock (`RandomClaim::Host` default).
Catalog cards come from an external `$RS2B0T` / `--catalog` checkout
(upstream `rs2b2t/rs2b0t` layout: `src/bot/scripts`), not a copy in this
tree. `$RS2B0T` wins over the persisted root (`~/.274bot/rs2b0t-path`,
written on the first successful catalog parse). Scripts run on whatever
world the bound **server profile** logged into (see [README.md](../../README.md)).

Compatibility is **partial**. Unsupported helpers and options fail
explicitly. Do not treat runner PASS, a single live gold, or a historical
inventory count as “all catalog scripts / all options qualified.”

## Native contract boundary

Compiled cards and the slot use `script::native::Script` exclusively. Each
`CompiledCard` descriptor owns its static metadata, versioned `SettingDef`
schema, per-account bulk-copy exclusions, typed preparer and Rust factory.
Browse, scenario Starts and Start handles all use that registry; there is no
second parameter-default table or crate-root `Script` trait.

Start prepares settings and selected resources on a `FamilyPreparation`
worker, then installs only if the captured slot/control generation is current.
Assignment changes after Ready, never on enqueue. Stop, replacement and
profile removal invalidate late replies. Invalid settings or factory failure
retain the previous assignment and report a structured rejection.

`NativeTick` borrows the frame/evidence/pin/recovery views and the shared
status/paint/log output. Native instances allocate no isolate. Status and paint
are shared on change; focused detail retains rich status while fleet rows remain
scalar. A slot that has never attempted a native Start allocates no native
instance, preparation or retained cell; a rejected Start may retain its cell.

The typed action-machine facility is a separate cutover: begins/polls still
refuse with `ActionError::Unavailable`; this registration implementation does
not qualify quiet leases, native walk/journal operations or watchdog clocks.
Sherlock retains its existing clue queue/dispatch behavior through one
crate-private borrowed host frame (no raw driver). It remains `load`-gated for
the existing clue implementation, but never creates a V8 isolate. Clue
transport/recovery extraction replaces that bridge, not the registration API.

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
| House API | Rust `tick(&mut NativeTick) -> Result<ScriptFlow, ScriptFailure>` | `export function tick` **or** `defineBot`; explicit `export const apiVersion = 2` is JS API v2 ([js-api-v2.md](js-api-v2.md)) |

Idle = no isolate. Stop tears down V8. Pause / not `is_up` keeps the
instance; `want_run` distinguishes operator Pause from offline.

A Load slot can also host one API seat: `api.gather.run` prepares and ticks
the genuine Gatherer card inside the slot with the slot's own ledger, and
`api.snapshot.gather` reports its live session. `api.questPaths()` is a sync
read of the release Path index for the four released quests, needing no seat;
`api.questProgress({ quest })` runs one owned progress read in the same seat —
tab colour first, then the quiet host journal read only when the colour is
in-progress and the released Path has journal rules. While the seat is live the
host owns the slot's foreground: only the script's game rows are drained and
dropped at admission — never dispatched, never deferred. A `questProgress`
read while a gather session is live — and a gather `run` while a read is
live — is refused `busy`. Control rows
(`gather.stop`, run-policy) still pass while paused. BroadcastChannel
open/post/close, inspect-route requests and acknowledgements, and host-local
camera yaw writes survive foreground admission, but stay queued while the Load
slot is Starting or Paused, even offline, held, or without a snapshot. They
follow ordinary dispatch after Resume.
Held reconnect walks and the host's carried walk are discarded while the seat
owns the slot, not saved for replay after it ends. This deliberately differs
from design §3.6's held-walk deferral: an old route must not regain foreground
authority after a native session. The carried host walk is not counted as a
dropped script row. When the session ends ordinary game dispatch resumes.
The Load-slot seat's worked example is
`crates/script/examples/gather_quest_v2.ts` (authoritative) beside
`gather_quest_v2.js` (checked-in plain-JS form): one Power-mode gather session
to a drop quota, then one read-only `questProgress` check, with no script game
actions. See [js-api-v2.md](js-api-v2.md) "Worked example: GatherQuest v2".

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

### Quester combat outcomes

During a multi-kill combat or acquisition step, status exposes the latest
completed combat sub-operation, including its end, exact target and evidence
stamp, even while the enclosing step remains pending. A new receipt publishes
on change; unchanged polls do not allocate or republish it. These intermediate
status fields do not complete the step or replace the final outcome used by
Path predicates.

### Quester journal reads

Native Quester dialogue completion requires four observed game ticks with
chat closed, rather than an elapsed host-millisecond gap. Combat interruption
is keyed directly to the client's `in_combat` flag, not to a newly observed hit
or a combat baseline. The flag stays set for about 8 s after any hitsplat
(`client.rs:9211` sets it to `loop_cycle + 400`; `decode.rs:1288` reads it).
Thus a talk step that starts or ends within that window after unrelated combat
parks as **Blocked**, even if the dialogue itself was not hit. This is
fail-closed and requires an explicit operator retry. Poison hits count too:
poison closes interfaces before applying its damage hitsplat, which sets the
same flag. A closed chat while the flag is set produces an explicit
combat-interruption outcome, including during dialogue opening or page
acknowledgement. Quester does not count that interruption as successful work,
settle the step, or request an advancement journal read. It does not fight or
automatically retry the conversation. A journal transaction
that loses ownership or becomes transiently busy is retried only after both
main and chat modals have been observed closed for three game ticks. Unknown
modal observations or reopened modals restart this quiet interval.

A logical progress read allows at most three journal transactions, each with
at most one quest-row click; adopting an already-open matching page also
consumes a transaction but does not click. Exhaustion parks and reports
`journal read retry limit reached` alongside the transient failure reason.
Successful reads and explicit Retry reset this budget. A chat modal at read
start waits within the existing bounded read window; if it remains occupied,
the parked status names the chat root and text. Modal ownership, quiet leases
and Stop/Pause revocation still govern all captures and closes.

### Gatherer power gathering

Gatherer supports Woodcutting, Mining and Fishing with a usable carried or
equipped tool, at the Start area, a Custom location or an Auto-selected area.
Power mode drops selected logs, ores or fish in bounded batches, counts drops only after their slots are observed
empty, and retains confirmed partial-batch progress across interruptions.
Unsettled drops are retried even after their dispatch receipts age out.

Incidental uncut gems are power-dropped along with mining products. Tools,
fishing bait and other non-products are kept. If protected items fill the pack
and no selected product can be dropped, the card stops with `inventory-blocked`
rather than gathering against a full inventory. Clear space and use **Retry**.

Fishing requires the selected method's tool and bait before gathering; missing
bait stops with `supply-missing`. Moving fishing spots are re-acquired by NPC
identity. Depleted resources are not clicked: a live lower-tier resource can
be selected while the higher tier respawns. Wait deadlines use the observed
group's respawn bound, capped at eight minutes since gameplay progress, and do
not slide on unchanged observations or Pause/Resume. Auto widening shares that
gameplay cap across depleted groups rather than restarting it at each group.
A recreated card starts with a fresh wait baseline rather than immediately
treating the wait as expired.

Auto searches outward in sliced 32-tile rings, up to 128 tiles from the Start
anchor, and temporarily skips exhausted groups until their respawn bound.
Four unexpired skipped groups produce `widen-limit`; exhausted search produces
`resource-unavailable`. Retry resets the search only for these search failures;
other retryable failures keep the current area and temporarily skipped groups.
Gas, ents and whirlpools are identified by generated IDs and trigger reselection
or a walk away, not another gathering click on the hazard. Pause/Resume preserves
an unfinished escape walk. A temporary hold defers actions and resumes on release
without requiring Retry.

Level-up chat pages are continued individually. An observed change of chat
root completes only the previous page; the new page requires its own Continue.
An unchanged page still fails after eight ticks rather than waiting indefinitely.

Changes marked restart-required (including skill, resources and location)
remain pending until the slot restarts; they never switch the active run in
place. Death stops the card; automatic recovery, banking and supply trips are
not part of this stage. Closest mode and shop provisioning are not offered.

### Quiet quest-journal painting

Host-owned quest-journal reads (the native Quester and the Rust journal machine
used by Load cards) have a quiet-paint lease. It arms before the quest-row click
and ends after the server's closed modal/text pair is observed. One client
skip-paint flag hides the main modal while retaining the pre-read side panel
and tab chrome; packets, input actions, widgets and server state are unchanged.
The existing last-framebuffer freeze during `scene_state == 1` is unchanged.
Stop, cancellation, error, disconnect/work-generation reset, or a ten-second
wall-clock safety fuse restores ordinary painting even without an eligible
script tick. Pause and hold do not freeze that wall-clock fuse. The inactive
path does no capture, widget scan, backup copy, extra clock read or bridge lock.

The Windows `host-play` live journal fixture's `journal-paint-proof` feature
records every completed CPU/GPU paint with producer-attached root/flag metadata.
Its restoration and close acknowledgments require paints newer than the
corresponding request. The fixture resolves the close control from the live
component bounds and lets a real paint build its hover menu before clicking.
If an ordinary read close was already accepted at Stop, the unowned control
reopen crosses the server's deferred-close tick before sending its button.
These capture and input steps are fixture-only; native journal policy is
unchanged.

### File Load and catalog cards

- **Load** registers a picker card tagged **File** from an absolute/relative
  path (`~/.274bot/js-scripts.json` remembers `{name, path}`). Same path
  overwrites; different paths stay distinct even when stems match.
- Compiled registry cards (currently Sherlock in `load` builds) are also in
  Browse, grouped under **Treasure Trails** and tagged **Compiled**. Sherlock
  has a known-empty schema version 1, distinct from unavailable metadata or an
  unsupported saved schema. Compiled names remain reserved.
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

Native settings use the same per-account map, under `compiled_identity_key`,
with envelopes `{"schema_version":1,"values":{...}}`; JS bags are unchanged.
Missing entries use current defaults; legacy empty native entries mean schema
1. Malformed or unknown versions remain unavailable and are never overwritten
with defaults. Merge precedence remains defaults → account overrides → explicit
inject, plus profile-global settings unless explicitly injected.

Native edits decode to a card-owned type off-pump **before** staging a durable
write. A failed validation/save changes neither the effective run nor durable
settings. Delivery after persistence is fenced to the exact account
incarnation/run/session, and reports Applied, PendingBoundary, RestartRequired,
Unchanged, Stale or rejection separately from the save. Pending-boundary
acknowledgements must name the latest revision; restart-required drafts never
become effective just because they were saved.
Discarding a prepared card configuration contains card-owned destructor panics,
including late worker results and stale UI deliveries.

Only native parameter edits compose over an unvalidated draft; assignments,
Start-Ready commits and Loaded-card edits use the durable-staged profile.
A valid preparation rebases only its own card's settings onto that latest row;
unrelated writes do not supersede it. A newer settings edit for the same card
or profile removal does supersede it.
Profile-form credentials and partner fields persist independently of native
preparation. Their live bag prepares only after a successful save; Stop or a
replacement Start makes delivery Stale without cancelling that save.
Unrelated profile writes do not make a live partner delivery stale.

Native Apply to all uses the same coordinator as loaded cards. Its confirmation
and result identify excluded per-account fields for each target. Full copy
preserves target-specific pairing fields, including absent and explicitly empty
values; a single-field request for an excluded field is refused before a write.
Sherlock validates the profile-global clue partner but does not add a duel
family to its existing solver.

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
| **Start / Pause / Resume / Stop** | Focused profile only. Per-bot Stop also cancels a Start that is still waiting. |
| **Start all / Start on marked / Stop all** | Bulk script controls (panel MultiBox rail; TUI Script tab `T` / `E`). Separate from **Login all / Logout all**. Start all and Start on marked bots (one shared command for panel and TUI) skip already running/paused/stopping members and start the rest one after another over the next frames, not all in one frame. One running report covers every Start click while any of its bots still waits or is still setting up: it counts each bot once as `started`, `queued`, `skipped` or `failed`, lists failures before skips, and updates as waiting bots start or are refused. A second click in that time joins the same report instead of replacing it. Every skipped or failed bot also gets a line in its own log. Waiting rows show `queued k/n`. Marked Start with a heading card starts that card on every marked row. Stop on marked rows reports waiting bots as `cancelled`. Stop all stops running and paused across wall members and live slots **and cancels waiting Starts**. A member removed, re-assigned, or operator-logged-out while waiting does not start stale work. |
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

## Host input versus native observations

`ScriptCtx` is the host pump's input to `SlotScript`, not a compiled card's
authoring context. Cards receive `NativeTick` and its borrowed `SnapshotView`.
Inventory, bank and quest observations carry readiness and an `EvidenceStamp`;
unavailable is not an observed empty list. A closed main modal is an observed
root of `-1` only on an ingame frame. Snapshot views do not clone the world or
grant a send-side driver.

## Nav vs scripts

WalkTo stays the panel **WalkTo** button + traveller. Script walk requests use
the host's shared `nav::router::find` and the slot pump's
`nav::traveller::Traveller::follow`; `SlotStatus.walk_{x,z,level}` mirrors the
armed destination and clears on arrival. Find runs off-pump; follow steps on
the slot pump under the existing admission fence. Typed native walk operations
are not installed by the registration cutover. Authored Path `walk` steps do
not yet implement protected walking: a nonempty `cross` is rejected during
Path compilation with code `invalid-args` and detail
`walk: cross needs protected walk (combat slice)`.

Manual game movement or a TUI Manual step takes ownership before either follow
pump. A matching native walk returns a normal `WalkReceipt` with
`WalkEnd::UserInput` (“cancelled by user input”) and no blocked reason. The host
invalidates the cancelled operation's route, pending plans, bank work and carry;
late results and automatic walking requests queued before takeover cannot re-arm
it. Live walking families inherit one intent baseline through route-less scene,
door, recovery and follow-up phases, so those phases stop too. The interrupted
walking composer's next sustain/log callback or stored completion cannot run
ahead of the takeover check.

When the script remains running, Gatherer parks retryably with code
`manual-movement`; Quester parks its current step without advancing it or
charging an attempt or failure streak. Compat walking calls settle false without
internal retries, hunt callers do not re-walk, and ReturnToAnchor settles its void
call without starting an approach leg. Pause → manual movement → Resume and
reconnect hold → manual movement → relog both preserve the carried compat walk:
the same post-hold rebaseline refreshes its intent from the first Running
observation. Movement during either hold is not a takeover of the idle carry.
A click after Resume, even before the isolate processes that command, instead
cancels the carry with one `UserInput` receipt for the original request. A click
after a completed route does not replace its terminal receipt; retained request
metadata is only for deduplication. An independent active WalkArm still yields.
Other slots are unaffected. A fresh top-level decision is admitted only after it
observes the takeover snapshot. If an in-flight tick sees the atomic takeover
notice before that snapshot, a new walking operation keeps the tick's observed
baseline and settles false during admission or its next machine step, rather than
waiting on a request the host's dispatch fence rejected. Stop/Start clears the prior
run's cancellation outcome and delivery guard.

The owner-pause preference is `nav.pause_script_on_manual_walk_abort`,
displayed as “Pause script on manual movement,” and defaults to ON. It gates
only pausing the script that owns the cancelled operation; OFF still cancels
and publishes the same terminal while leaving the script running. In that mode
compat walking settles `false`, while v2 can inspect its correlated
`walk_outcome_cancel_reason: 'user-input'` and make its own decision. The
setting does not forbid a fresh script-issued walk after the takeover is
observed.

This differs from movement during work that was already operator-paused or
reconnect-carried: that idle carry is excluded from cancellation and retains
its existing Resume behavior. Conversely, after an active walk is cancelled,
explicit Resume may permit a native script to make a fresh decision, but the
old cancelled request, host carry, queued work, and watchdog recovery never
replay automatically.

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
  `avoidZones` accepts up to 16 rectangles or the known catalog ids
  `white-wolf-mountain` and `draynor-jail-guards`; unknown ids, invalid
  rectangles or too many entries are refused by the host. `crossZones`
  accepts up to eight named danger zones to exempt for this walk and its
  verify probe; unknown names, excessive counts or an unavailable zone
  catalog are refused as well. `bankItemCounts` is not an input (the host
  bank fetch reads the bank).
- **`Traversal.walkTo`** is one Rust walk: radius 2 and 300 s by default,
  the same teleport, `avoidZones` and `crossZones` rules as
  `walkResilient`, `maxExpansions` accepted as `maxBudget` is, and the
  card's `Sustain` hook once a tick while it walks. A random event ends it
  false and stops its route. Walking off Karamja without the 30-coin fare
  (the navigator names only the fare as missing), it earns the fare at
  Luthas's plantation and walks once more. Teleport id lists, ship or
  shortcut exclusion, `pathFollow` and `forceRepath` fail with `not impl`.
- The host owns a script's active route. Operator Pause ends the follow
  and keeps the route as carried state, as a reconnect does; the first
  dispatch after Resume sends it once more under its own request id,
  whichever script code asked for it (walks, reach, the hunt steppers, a
  bank open's booth walk, raw v2 requests, `walkNearestBank` while it is
  still choosing). Nothing is re-sent when the script run changed, the
  walk was cancelled (manual movement, an `AbortWalk`, a failed walk machine,
  Stop, or the script stopping itself), the player already stands within the walk's
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
