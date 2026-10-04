# Scripts (`crates/script`)

Script **kernel** for the bot host. Compiled cards are rust-first on 274bot
`api`. Catalog and external TypeScript/JavaScript bots load through a thin
compatibility shim; **behavior, configuration, and fail-closed helpers are
owned in Rust**. The shim does coercion and marshaling only. The only
JS ↔ Rust isolate wire is **FlatBuffers** — there is no extra JSON host wire
and no foreign JS policy runtime in-tree.

WalkTo is **host nav** (panel picker / TUI map), not a script card.
`native::Script::on_random` is a rising-edge knock (`RandomClaim::Host` default).
While the host guardian holds a trapped random (Maze, Mime, Strange box) the
native run is frozen. The guardian gives one Maze visit 1,000 game ticks (twice
the content's 500-tick reward clock); after that, and for the Maze or Mime
square with random events off, the square is inert and any unheld running
script (native or Load) standing on it fails with `random-trapped` and takes
the terminal Blocked Stop described below instead of continuing there.
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

A native `ScriptFlow::Blocked` is terminal. So is a published
`NativePhase::Blocked` carrying a failure, even if the tick returns `Continue`.
Quester, Gatherer and Sherlock take the same slot-level Stop path as operator
Stop: cancel actions and walks, release quiet leases, and discharge an owned
WalkGuard's owed off-click. For each `Failed`, `Stopped`, or `Completed`
lifecycle receipt, host observation resets the navigation session exactly
once, including routes, route inspect, bank picks, carried walks, and duel
offers. A blocked instance is dropped and the slot is Idle, not Running. Its
terminal blocked status and reason remain available to panel and TUI through
frontend-core and to the change-only status log. They carry no run/control
authority. Watchdog, reconnect and fleet polling cannot restart that run; the
operator must explicitly Start again.
`Waiting` remains live until the card's own bound produces a terminal failure.

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

### Quester queue and provisioning

Quester uses the ordered `crates/script/paths/289/index.json` release roster.
`quests` selects quest IDs (an empty list selects all released Paths);
`order_override` prioritizes selected IDs and `skip` excludes IDs. `partner_account`
and `gang` are per-account settings, not bulk-copy settings; partner quest
execution remains separate from this release slice.

`crest_gauntlets` is also per-account: `chaos` (the default), `cooking`, or
`goldsmith`. Runtime account choices are available to quest handlers when a step
begins, without specializing the shared compiled Path. Omitted saved values use
`chaos`, and unknown or random values are refused.

In the panel, edit these settings in **Script prefs**; in the TUI, use **Params**.
For Cook's Assistant alone, set `quests` to `cook` while stopped, then Start.
The other released IDs are `sheep`, `runemysteries`, `romeojuliet`, and `imp`.

`max_deaths` defaults to **2**, using the Gatherer's `maxDeaths` limit policy:
two deaths can recover, and the third stops Quester as blocked with a
maximum-deaths reason. The count spans the queued Paths in one run.
The additive setting keeps schema version 3; saved records without it receive
the default. An explicit Start begins a new death allowance.

Quester uses the Gatherer's slot-retained recovery lifecycle: watchdog
recreation and reconnect preserve its death count, consumed chat watermark,
last quest stand, completion count and bank-retreat receipts. An old death
message does not count again; a new one received during recreation still does.
The shared slot watchdog walks a displaced Quester back to that stand without
recreating its instance. If it must recreate the card, Quester rereads server
quest evidence instead of treating its discarded local step cursor as progress.
Operator Stop releases retained run memory; Stop/Start begins a fresh budget.

The Quester's no-progress window counts unchanged step boundaries (warn at
three, block at eight), not time spent paused or held. Pause/Resume, guardian
holds, reconnect and death reset that window, including ineligible frames
without an explicit hold callback. This remains distinct from the shared slot's
wall-clock wedge clock and Gatherer's eligible-gameplay-tick idle bound.

Rust-native cards and the compatibility clue machine observe death through the
same death-chat latch. Quester, Sherlock and compatibility clues temporarily
suppress work at zero HP while the content's delayed death message or restored
HP is awaited. HP alone neither spends a death allowance nor terminates a clue;
lethal damage must not turn an active Quester dialogue into a living-player
combat interruption. A new death message cancels the current clue without
counting it as solved, even while held or after HP has already been restored.
Posted compatibility arguments cannot manufacture a death observation.
Sherlock retains its consumed death-chat watermark across watchdog recreation,
so a death received in that gap suppresses new clue work once rather than
being discarded as the recreated card's initial chat baseline. Operator
Stop/Start discards that retained watermark and begins a fresh baseline.

Running Load cards can exempt explicitly named random events through their
cached `ignoredRandoms()` list. The event is still published, but the guardian
does not act or hold for that event. Unlisted events and inactive cards retain
host guardian handling; this is not a general-purpose decline hook. Ignoring
Maze or Mime declines the guardian's solve/hold, not the slot's terminal
`random-trapped` protection if the script is left unheld on the trap square.

Eligibility publishes DONE, READY, or BLOCKED with the requirement's reason.
Item requirements gate a new quest, not an in-progress quest whose hand-ins
already consumed them. An unread bank is unknown, not an empty bank.

An unobserved quest-list family waits without compiling a Path or sending gameplay
effects for up to 30 seconds of eligible active time. It admits automatically when
the family arrives; otherwise it blocks with normal-login recovery instructions.
Missing individual quest rows and unknown colours remain fail-closed. Excluding
every selected quest is a blocked queue, not successful completion.
The existing panel script section and TUI status show the native phase and the
current wait or refusal reason, including how to resolve it before Stop/Start.

For Paths that do not own their inventory, provisioning checks the pack before
withdrawing or acquiring supplies, preserves tools when freshening the pack,
and returns to the selected bank after completion before advancing the queue.
An `acquirable` row with `acquire: null` is a withdrawal hint; its authored Path
steps perform acquisition. Only `mustHave` shortfalls block. Tools are preservation
and withdrawal hints, not additional `mustHave` requirements.
Coin and loadout carry floats are drawn once per pass rather than replenished
after every dose or meal; death resets those latches. Paths marked
`owns_inventory` retain their authored inventory steps. Automatic coin funding
is not provided.

Path loadout headers use selected item aliases, such as `rune_scimitar` and
`4doseprayerrestore`. Compilation resolves each worn and carried item once into
the display-name rows consumed by Loadouts; operator overrides remain ordinary
display-name Loadouts rows.
Certificate aliases in either header section are rejected before that conversion;
the shared display name must not erase a certificate's distinct item identity.

Acquisition recipes can call other recipes with `acquire` steps. The compiler
binds dependencies first and compiles each recipe once, independent of its
declaration order. A chain can contain at most 32 recipes. A cycle returns
`recipe-cycle` with the cycle's recipe names. A missing dependency remains
`unresolved-recipe`; excess nesting returns `recipe-nesting-limit`.
Native Quester and Gatherer bank selection chooses the eligible, routable bank
with the lowest walking-route cost in ticks; teleport grants and held runes do
not change that ranking. A bank must have usable packed or declared access.
For a packed booth, selection routes to every usable access tile of the chosen
stand in one multi-goal search and returns its cheapest reachable approach.
The catalog tile need not be one of those access tiles. An unreachable staff-side
tile cannot displace a reachable public approach.
Standing near a bank across a wall is not proof of reachability. A selection
timeout may offer an air-nearest candidate, but the subsequent walk must still
route successfully and fails closed otherwise. Compatibility `SelectBank` and
`WalkNearestBank` retain their frozen four-tile shortcut, 500,000-expansion
budget, five-second timeout and air fallback.

An authored Path bank tile is not an exclusive destination by default:
`{"tile": [3092, 3242, 0], "source": "author reference"}` still selects the
cheapest routable bank. Add `"required": true` only when that particular bank
is necessary. Required tiles must resolve to an eligible named bank with access
and a successful route; an unmatched or unreachable required bank refuses
rather than substituting another bank. `"bank": "nearest"` is unchanged.

The native status producer exposes active quest progress under `quest`, its
name under `display`, queue states, requirement and provisioning details, and
step/session counters. `required_vs_live` counts skill gates; `required_<skill>`
and `live_<skill>` carry integer base levels (a missing live field means
unobserved). `tested_stats_warning` reports absent qualification or levels below
the recorded profile. `journal_lines` is sent only once after a fresh read;
consumers retain the last received lines. These fields do not add a panel window
or TUI pane.

### Quester combat outcomes

During a multi-kill combat or acquisition step, status exposes the latest
completed combat sub-operation, including its end, exact target and evidence
stamp, even while the enclosing step remains pending. A new receipt publishes
on change; unchanged polls do not allocate or republish it. These intermediate
status fields do not complete the step or replace the final outcome used by
Path predicates.

Loot is optional: an unreachable drop is skipped and the combat step continues.
Manual movement, cancellation and missing-evidence outcomes still end the
operation. Non-loot Reach door-recovery failures still park Quester.

### Dialogue page acknowledgements

Native and compatibility dialogue acknowledge Continue and answers only when
the chat root, Continue visibility, or modal-page content changes. Modal content
includes every body text line and each option's component id and text; a new
chat-history ring line is not page progress. The host computes the shared native
fingerprint and sends one `u64` in the existing FlatBuffer snapshot, rather than
copying modal text into JavaScript. An unchanged page times out without repeating
the action.

Selected `DialogueUiIds` provide only source-proven main `scroll` and `book`
identities. Book forwarding resolves the script's forward handler separately
from its last-page visibility marker; generic component names in revision 274
are not guessed from their numeric ids. These facts are part of the pinned
main asset and its source provenance, not the opt-in `DebugCatalog`.
Message and object-box `mesbox` pages are chat surfaces and keep the ordinary
chat continuation path.

### Quester magic combat arguments

The combat family accepts `tactic.style: "mage"`. Root `spells: null` (or
omission) keeps native strongest-castable selection; an explicit non-empty
`spells: ["fire_bolt", "Wind Strike"]` list resolves against the selected
spell facts and preserves its order for manual casting. Empty, unknown or
over-255-entry orders fail compilation. `fallback_spells: false` is the default;
`true` permits the native strongest-castable fallback when the fixed order is
exhausted. Spell-specific refusal and rune evidence still belong to the native
combat core.

### Quester expected combat handoff

A `talk` step may declare `expect_combat: {"npc": "desertminingcaptain"}` when
the conversation deliberately starts a fight. The config name resolves to an
exact NPC type. A combat interruption succeeds only when the local player's
observed NPC target has that type and targets the local player in return.
The NPC's own damage timer need not be set before the player retaliates.
The step returns a `HandedToCombat` receipt; wrong, missing, player or unrelated
targets remain **Blocked**.

Author the handoff with `"advances": false` and an `in_combat` settle, followed
by a `combat` step on the declared opponent. This avoids opening a progress
journal during the fight. The combat step may declare `"advances": true` once
the fight and its reward have settled. An ordinary dialogue completion retains
the normal talk outcome; `expect_combat` does not fabricate a fight.

### Quester journal reads

Native Quester dialogue completion requires four observed game ticks with
chat closed, rather than an elapsed host-millisecond gap. Combat interruption
is keyed directly to the client's `in_combat` flag, not to a newly observed hit
or a combat baseline. The flag stays set for about 8 s after any hitsplat
(`client.rs:9211` sets it to `loop_cycle + 400`; `decode.rs:1288` reads it).
Thus a talk step that starts or ends within that window after unrelated combat
parks as **Blocked**, even if the dialogue itself was not hit. This is
fail-closed and requires an explicit operator Start. Poison hits count too:
poison closes interfaces before applying its damage hitsplat, which sets the
same flag. A closed chat while the flag is set produces an explicit
combat-interruption outcome, including during dialogue opening or page
acknowledgement. Without an explicit expected-combat handoff, Quester does not
count that interruption as successful work, settle the step, or request an
advancement journal read. It does not fight or
automatically retry the conversation. A journal transaction
that loses ownership or becomes transiently busy is retried only after both
main and chat modals have been observed closed for three game ticks. Unknown
modal observations or reopened modals restart this quiet interval.

A logical progress read allows at most three journal transactions, each with
at most one quest-row click; adopting an already-open matching page also
consumes a transaction but does not click. Exhaustion parks and reports
`journal read retry limit reached` alongside the transient failure reason.
Successful reads reset this budget; an explicit Start begins a fresh budget.
A chat modal at read
start waits within the existing bounded read window; if it remains occupied,
the parked status names the chat root and text. Modal ownership, quiet leases
and Stop/Pause revocation still govern all captures and closes.

### Gatherer gathering and supplies

Gatherer supports Woodcutting, Mining and Fishing with a usable carried or
equipped tool, at the Start area, a named Site, a Custom location or an Auto-selected area.
Power mode drops selected logs, ores or fish in bounded batches, counts drops only after their slots are observed
empty, and retains confirmed partial-batch progress across interruptions.
Unsettled drops are retried even after their dispatch receipts age out.

With random-event handling enabled, a lost axe or pickaxe head is picked up
before the two held pieces are reattached. Recovery is bounded to twelve
seconds, including time for the flying head to land. If it cannot settle,
Gatherer revalidates the headless tool as missing and makes a replacement-tool
bank trip instead of remaining held until the watchdog restarts it.
A blast-broken tool follows the same observed missing-tool bank path; the
Gatherer does not repair tools or implement a second lost-head handler.
Owned random events revoke gathering work while held. Release revalidates
inventory, equipment and the work area before dispatch; foreign events do not hold.

Incidental uncut gems are power-dropped along with mining products. Tools,
fishing bait and other non-products are kept. If protected items fill the pack
and no selected product can be dropped, the card stops with `inventory-blocked`
rather than gathering against a full inventory. Clear space and **Start** again.

Fishing requires the selected method's tool and bait before gathering; missing
bait stops with `supply-missing`. Moving fishing spots are re-acquired by NPC
identity. Depleted resources are not clicked: a live lower-tier resource can
be selected while the higher tier respawns. Wait deadlines use the observed
group's respawn bound, capped at eight minutes since gameplay progress, and do
not slide on unchanged observations or Pause/Resume. Auto widening shares that
gameplay cap across depleted groups rather than restarting it at each group.
A recreated card starts with a fresh wait baseline rather than immediately
treating the wait as expired.

When gathering becomes idle, the content-derived stall window permits one
retry before reselection. Each fresh product or XP gain resets that window;
an earlier gain cannot keep a later idle attempt alive. Confirmed yield and
XP totals remain intact across the retry.

Auto searches outward in sliced 32-tile rings, up to 128 tiles from the Start
anchor, and temporarily skips exhausted groups until their respawn bound.
Four unexpired skipped groups produce `widen-limit`; exhausted search produces
`resource-unavailable`. These terminal search failures stop the run;
an explicit Start begins a fresh search at the current position.

`location` defaults to `Start`, unchanged for existing schema-4 settings.
Choose `Site` to pick a content-derived named camp for the selected resources
or fishing method. The site list follows the active selection; an old or
incompatible saved site stays visible with its reason but cannot be picked
and refuses Start. Entering Site mode saves before a place is picked; an empty
site refuses Start until a named place is selected. In the TUI, Space cycles
eligible choices and Enter opens
a choice list; lists with more than 16 choices support search. The panel
combo has the same eligibility and search threshold. Search matches site
names and ids; Esc cancels without changing the selection.

Site chooses an accessible placement of a selected resource nearest the
camp's resource centroid. That retained anchor survives Pause/Resume; changing
the site requires a fresh Start. The work area remains `anchor ± radius`,
not the camp's entire box. Site does not widen on exhaustion. `Custom` keeps
free tile and radius entry, including underground locations that have no named
surface site. Banking with `Nearest` picks the nearest routable eligible bank;
Return uses a resource/observation-stand Area arrival at radius 1, even when
the bank is inside the configured gathering radius.
Unloaded loc resources retain loc-aware Reach arrival and their `loc_id`,
including the refresh to the live loc's operable stands when it loads. Only
unloaded fishing NPC observation stands use Area arrival; live NPC operations
own their own approach. Large fishing movement envelopes remain eligible for
Return and observation beyond the first eight cells. The eight-approach
budget still bounds unsuccessful surveys; exhausting it is not absence proof.
Gas, ents and whirlpools are identified by generated IDs and trigger reselection
or a walk away, not another gathering click on the hazard. Pause/Resume preserves
an unfinished escape walk. A temporary hold defers actions and resumes on release
without requiring a new Start.

Level-up chat pages are continued individually. An observed change of chat
root completes only the previous page; the new page requires its own Continue.
An unchanged page still fails after eight ticks rather than waiting indefinitely.

The compiled Gatherer card schema is version 4. `disposition` defaults to
`Bank`; `Power` drops selected gathering products, while `Bank` returns to the
selected bank and deposits them. Bank mode also makes a trip when no inventory
slot is free; Power mode can still take a supply-only trip. `bank` defaults to
`Nearest`. A named bank chosen by the user is required: always use that bank,
and refuse if it is unavailable or unroutable instead of substituting another.
`useMageBank` and `useZanarisBank` remain eligibility opt-ins, not ranking
preferences. The unsupported `Closest` mode is not offered.
Ordinary banks use packed stand access. Declared teller banks use the same
live NPC/dialogue opener as `Bank.openNpcAccess`, including multi-page chat;
an open modal without a loaded item table is not a successful native open.
A bank without packed stand or declared NPC access fails closed with its name.
Native counted withdrawals use the same session-fenced host amount-dialog
continuation as compatibility scripts; dispatch alone never confirms a transfer.
Named bank deposits press one content `Deposit All` operation on that exact
bank-side row (id, slot and component), never a same-named noted or unnoted
copy, rather than sending an operation for each occupied slot. Other item ids
and configured keep items are untouched; observed inventory changes, not
accepted dispatch alone, still settle the transfer.

Supply targets are upper bounds, not a requirement that the bank contain the
entire refill. Each withdrawal targets the lesser of the configured count and
the combined held and loaded-bank stock. A usable partial refill continues
gathering; `supply-missing` means a required tool, bait, configured food or a
complete reserve cast is unavailable. Coins never gate a trip. Available
withdrawals, including the tool, settle before another unavailable supply is
reported. Transfer failures use `bank-deposit-failed` or `bank-withdraw-failed`,
not a missing-supply or full-inventory label. Product deposits succeed only
after inventory confirms that no unprotected gathering products remain.

`baitTarget` defaults to 100 (range 1–10,000). Only a selected fishing method
that consumes bait uses it: zero held bait makes a trip due, and the next bank
trip tops it up to the target. `food` is an optional selected-cache item name;
`foodTarget` defaults to 0 (range 0–28); with a configured food name, a nonzero
target makes a trip due only when no food is held. `eatBelow` defaults to 0
(meaning half the observed maximum HP, rounded up; a nonzero setting is an
explicit 1–99 HP threshold). At a boundary the card eats configured food only
when it is held and current HP is at or below that threshold. Food is protected
from deposits and product disposal.

`coinTarget` defaults to 0 (range 0–2,000,000,000). Coins top up to that
target during another bank trip but a coin deficit never starts a trip.
`reserveTeleport` defaults to `Off`; its options are the selected cache's
available teleport spells. An enabled spell requires `reserveCasts` of at
least 1 (maximum 1,000). A reserve refill becomes due only when the held runes
cannot pay for one cast; once due, the whole selected rune-cost batch is
planned to the configured cast count. Stocking those runes does not enable
teleport walking: `allowTeleports` remains a separate opt-in. Unread bank
contents are pending, not empty; missing stock is reported only after a loaded
bank observation. `deathPolicy` defaults to `Recover`; `Stop` ends the run
as blocked on death. Review the recovery setting before starting again.
`maxDeaths` defaults to 2; the third death stops the run as blocked.

Recovery observes restored HP in the Lumbridge respawn square (no region change
is required), waits three ticks, re-observes kept supplies, uses the existing
bank trip if needed, verifies equipment and requires an arrived return walk.
Only a fresh product **and** XP gain increments `recoveries`. `recovery_step`
reports 1–5 for the pending sequence, 6 while proving yield and 0 when idle.
A second death before recovery and a completed disposal blocks with `died-again`.
A blocked recovery stops the run, including a refused return. Watchdog recreation
and reconnect retain both the step and chat watermark only while recovery is
live, so old deaths cannot
re-latch and gap deaths are still detected. No-stock failures remain
`supply-missing`; missing respawn evidence is `respawn-not-observed`.
Shop provisioning and self-defence are not offered.
Skill, resource and location changes remain pending until the slot restarts.
Bank and supply settings apply at a pending boundary, never midway through a
bank batch.

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

Hunt Fight and Hold use the same native protection picker and Prayer
level/points gates as Combat. Each mode handles its outstanding reply before
inserting a protect click; that click's acknowledgement resumes the saved mode
effect, not its previous request. An unobserved toggle is retried after three
game ticks, without re-clicking while pending. Dragonfire inputs use the worn
selected Dragonfire shield and a confirmed antifire sip (fresh consume chat
plus a selected-dose decrease), expiring after 600 game ticks. Combat tables
are cached for the isolate's selected content; a build failure logs an
actionable protection error and fails the awaited Hunt run rather than silently
disabling protection.

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
A valid preparation rebases its own card's settings onto that latest row;
an accepted marked copy that assigns an unassigned bot carries that assignment
in the same transaction. Settings-only edits do not replace a concurrent
assignment. Unrelated writes do not supersede preparation; a newer settings
edit for the same card or profile removal does supersede it.
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

While a settings-copy confirmation waits for **Apply** or **Cancel**, all new
script Starts share one host dispatch hold, including focused and per-profile
Starts, bulk Starts, restarts and scenario `StartScript`. Direct Starts report
the affected bot and the waiting-copy reason. Already queued Starts keep their
FIFO places rather than failing or disappearing, and scenario Starts stay
pending without failing their witness. Applying or cancelling releases the
hold; queued operator Starts then read the resolved settings. Marked Apply can
assign an unassigned marked bot, but that assignment and its accepted settings
commit together only after native preparation succeeds.

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
A bank-side backpack is posted only while the bank's side modal root is up
(`modals().side`, sent to compatibility scripts as `side_modal_id`): an open
main bank whose side root is still down has no side observation, even with an
empty list, while a raised root with no rows is a posted empty pack.

## Native combat prayer ownership

Combat waits for one complete observation of all 15 prayer overlay varps before
emitting any prayer toggle. Missing rows are unknown, not inactive prayers.
Prayers already on in that observation belong to the user: Combat preserves
them during the fight and at wind-down, and does not replace a user's offensive
tier with a stronger one.

Protection is the exception because the game permits only one protect at a
time. An accepted Combat switch relinquishes the displaced prayer. If Combat
raises that style again later, it owns the new activation and clears it when
the fight ends. It never restores a displaced protect. Planned, pending and
refused clicks do not change ownership.

Quester and Sherlock carry the accepted-raise cleanup obligation across a
cancelled fight or Pause and clear only those prayers before continuing.
Scoped cleanup waits for missing owned prayer rows rather than treating them
as off. Accepted dispatches are included even when cancellation occurs before
Combat's next poll.
Quester and Sherlock retire that obligation when death clears prayers, so a
later Pause or Resume does not turn off prayers the user activates after respawn.
Quester recipe substeps retain that obligation when a combat child completes,
and finish scoped cleanup before settling or starting the next recipe child.
Any Stop ends the fight too, whether the operator stops it or a native run
stops itself on a terminal block. Before revoking and dropping the native card,
the slot transfers its accepted Combat raises to the host. The ordinary host
pump pays only those owed off-clicks with WalkGuard's bounded,
observation-settled retirement rule; a new run waits for that cleanup instead
of adopting the raised prayers as its baseline. User prayers remain untouched.
Cancellation still revokes the fight's action authority; the retirement
off-clicks belong to the host, not that revoked owner. A completed fight does
not trigger another clear merely because user prayers remain on. Explicit
`Prayer.clear` retains its broad all-prayers meaning.

## Nav vs scripts

WalkTo stays the panel **WalkTo** button + traveller. Script walk requests use
the host's shared `nav::router::find` and the slot pump's
`nav::traveller::Traveller::follow`; `SlotStatus.walk_{x,z,level}` mirrors the
armed destination and clears on arrival. Find runs off-pump; follow steps on
the slot pump under the existing admission fence. Typed native walks correlate
host outcomes with their run, action and request owners. Inherited walk options
resolve against the current global grants and the instance's committed script
permissions, captured at Start and when a settings revision becomes effective.
Either may allow the walk, while a walk-local `false` forbids it even when a global or
per-script setting allows it. Native script settings default off and use
`allow_teleports`, `allow_wilderness` and `allow_danger_zones` (Gatherer
exposes camelCase ids). Path `walk` steps accept the same three optional
booleans, so omitted bits inherit and `true` opts in for only that step. Native
script walking never exposes bank fetch.

The panel Nav config and TUI settings share these global permissions in
`panel-ui.json`. Teleports and wilderness apply to manual and Rust-native
walks. Danger is off by default; the map danger control opts in for this walk
only while the global is off, and is replaced by a warning when it is on.
Native scripts cannot use BankBudget, even when global bank fetch is enabled.
Native bank selection ranks walking routes without teleports; the walk to the
chosen bank may still use granted teleports. Compatibility scripts keep their
existing option wiring, including always-enabled wilderness and bank fetch.
All shared preference writers, including the TUI log pane, serialize through
an in-process mutex and the `panel-ui.json.lock` advisory lock. Both frontends
refresh a shared durable walk-permission projection for danger controls and
inherited script rows, and refresh it again at manual admission. Missing or
malformed preferences fail closed. A peer frontend's grant change therefore
updates both the warning and the next walk's options.
Danger grants permit a fallback crossing, not a shorter dangerous route:
the router always tries the filtered safe pass first. Released Cook's
Assistant mill walks inherit teleport permission, as their frozen callers do.

Path `cross` names danger-zone exemptions for that walk; it is independent of
`guard: "protect"`, which controls hold-mode protection. A named crossing may
be used with or without that guard.

The guard holds the selected protection prayer without attacking, eating or
flicking. A Prayer-level shortfall, or zero points with no prayer potion, produces
one non-terminal `WalkEventKind::Unprotectable` warning for that style and cause.
The native owner consumes it with `NativeActions::take_walk_event`; the walk
keeps following and any other held protection remains managed. Quester shows
the cause under **Walk protection**, for example “Prayer 40 needed for Protect
from Missiles”, without parking or retrying the step.

The guard owns only a protect it raised during this walk. A protect already on
when the walk begins, or raised by another owner, is not turned off at the end.
Only one protect can be active: switching from a pre-existing user protect to
the guard's protect deactivates the old style in the game. The guard clears only
its own new style at the end; it never restores the user's old style.
A late on observation beyond the three-tick admission window is not adopted
as guard-owned, even while the walk is still active.

Arrival, Stop, Pause, cancellation, owner revocation and manual takeover retire
the guard but preserve its conditional off-click on the host. An in-flight
enable or switch has three ticks from successful send admission to be observed
on. If a switch expires while the previous guard-owned style is still observed
on, cleanup retires that old style instead; a user-owned style is never a
fallback, and an observed real switch is never reversed. Otherwise the host
logs and drops the unobserved-enable debt without a click.
For an already-observed owned protect, the bounded cleanup attempt window
starts at the first cleanup pump, not the guard's last evaluation before a
hold. It receives an off-click, and the debt remains until observed off. If
it stays on, the host retries once after three ticks and drops the debt after
three more ticks if the retry has not been observed off. Timed drops are
logged, including unavailable varps and refused off-clicks.
An already-off protect receives no toggle. Later walks and bank work resume
after at most the first three-tick window, even while cleanup is watched; a
new guard cannot raise protection until that old cleanup ends.

Cleanup reads the current snapshot, not another owner's not-yet-observed
same-frame prayer write. Such writes are not an atomic handoff: a simultaneous
combat prayer switch can still conflict with cleanup before its varps arrive.
Once the switch is observed, the old protect's off state settles the debt
without another click. Relog and death discard the obligation because the
server resets temporary prayer state. The manual WalkTo arm installs no guard.

Manual game movement or a TUI Manual step takes ownership before either follow
pump. A matching native walk returns a normal `WalkReceipt` with
`WalkEnd::UserInput` (“cancelled by user input”) and no blocked reason. The host
invalidates the cancelled operation's route, pending plans, bank work and carry;
late results and automatic walking requests queued before takeover cannot re-arm
it. Live walking families inherit one intent baseline through route-less scene,
door, recovery and follow-up phases, so those phases stop too. The interrupted
walking composer's next sustain/log callback or stored completion cannot run
ahead of the takeover check.

Gatherer and Quester publish `manual-movement` as blocked and stop their runs;
Quester does not advance its current step or charge an attempt or failure streak.
Compat walking calls settle false without
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

## Bank item ops (compat v1)

`Bank` transfers are Rust families on the shared transfer kernel
(`crates/script/src/bank/ops.rs`); the shim passes the caller's arguments and
awaits the frozen return. One transfer runs at a time: a second start while
`withdraw*`, `withdrawLoad`, `close`, a deposit loop or `withdrawTo` is in
flight settles false (a deposit loop does nothing).

- **`Bank.deposit(name, op = 'Deposit-1')`** presses that label on the first
  bank-side row with the name, by id, slot and component, and answers whether
  it pressed. A row without the label presses nothing; it is never an All.
- **`depositAllMatching` / `depositInventory` / `depositAllExcept`** press
  each matching row's All op by id and wait up to 4 s for that id to leave
  the pack. `depositAllExcept(names)` keeps every id whose display name is
  kept, noted or not. A nameless row is still an item the matcher decides.
  A posted empty side ends the loop at once; a side root still down is
  waited on for 1.2 s.
- **`Bank.withdrawLoad(name)`** presses the row's Withdraw-All and answers
  true once more pack slots are used, the pack is full or that row emptied,
  within 4 s; a row without All is withdrawn as `withdrawX` of the free
  slots. A full pack is true with no press.
- **`Bank.close(timeoutMs)`** is true at once on a shut bank. Otherwise one
  Close is true only when the bank is shut, its old side root is released
  and the bank session generation moved on, within `timeoutMs` (4 s when
  omitted). A reopened or logged-out session is false.

## Hard no

No dummy tick-end opcode. No `Arc<World>` on extras. No bot action API in
`vendor/fr-client-rust`. No extra JSON host wire. Never Fairy-Ring.
