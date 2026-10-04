# Reusable live harness

The harness runs real revision clients against your local engine for the
bound **server profile**. It keeps scenario preparation, proof predicates
and failure exits in Rust so preservation projects can reuse the same
behavior through `host-play`, the TUI or the panel. No engine assets or
external catalog scripts are included in this repository.

For ordered multi-case native runs with ledgers, receipts and process-tree
ownership, prefer the tracked suite entrypoint
[`e2e-suite`](e2e-suite.md). A green child or zero exit is an observation
record — not automatic script qualification. Captures that require human
readback stay `pending_visual_review`; raw failures stay preserved.

## Path-backed Quester qualification

New quest fixtures use `scenario::quester::quester_stage(QuesterStage { ... })`.
The older numeric `scenario::quester_stage` remains available to the shipped
S2, combat and WalkGuard cells; it does not apply the new qualification profile.

Add a `QuestFixtureProfile { quest, profile }` row to `FIXTURE_PROFILES` in
`crates/scenario/src/quester.rs`. `Base40` and `Base60` set Attack, Strength,
Defence, Hitpoints, Magic and Ranged to the named floor, and Prayer to 43.
The builder adds the Path's declared skill requirements, at their exact levels
for noncombat skills and without reducing a combat floor. The six harder
quests named by the operator already have Base60 rows. A profile is a fixture
plan, not an eligibility requirement or evidence of a successful live run.

`QuesterStage` takes a scenario name, the selected quest-tab display,
the decoded `PathDocument`, its selected `QuestIdentityRow` and
`SelectedGameData`, a stage key, an optional loadout, extra carried items,
and the starting tile. Each stage needs a `progress.rules` varp hint;
missing or contradictory hints fail rather than becoming stage zero.

Paired quests use `quester_role_stage(request, gang, stage_varp, seed_vars)`.
It selects an explicitly authored Phoenix or Black Arm role, seeds that role's
real content variable, and keeps prerequisite variables in pre-Start setup.
`run_pair(PairCell { roles, mode })` prepares both accounts before either Start,
adds reciprocal account settings, and starts each account's own Quester. Role 0
is Phoenix; role 1 is Black Arm. Restart cells explicitly Stop and restart both
accounts; an ordinary paired step never starts or stops the other account.
Miniquests use `miniquest_stage(MiniquestStage { ... })`, with explicit content
variable seeds and a proof predicate instead of an invented quest-tab identity.
Their terminal proof also requires the Path's owned progress reader to publish
completion; a closed card or successful click alone is not completion.

`FixtureLoadout::Path(name)` seeds that Path's actual kit.
`FixtureLoadout::Standard(StandardKit::{Melee, Magic, Ranged})` supplies the
operator's starting gear. Gear never raises the profile to make it wearable:
an item refusal fails the fixture and is a real qualification limit.
Product Path loadout headers require selected config aliases. Fixture lookup
also accepts case-insensitive display names; this does not relax product header
validation. The builder emits only resolved config aliases in `give` commands
and merges carried kit/extras by item ID.

The returned `QuesterFixture` has independently owned `scenario`,
`start_settings`, `seed_commands` and a cloneable `seed`. Move `scenario`
into `ScenarioRunner`, and retain `seed` and `start_settings` in the live cell.
Use the returned settings for the compiled Start; they select only this quest.
The scenario clears carried and worn items, sets the profile and quest stage,
seeds and equips the chosen kit, then relogs and teleports to the authored
starting tile. Teleporting after relog prevents the mainland login hop from
overwriting the fixture origin. Its final pre-Start observation gate requires
the exact posted base/effective stats, every carried/worn seed and the exact
starting tile. It reads actual skill slots, including Agility, not the energy
proof. At Start, record `seed.observe_start(snapshot)` with the live receipt;
that receipt includes the observed tile. Only actual observations may be used
to write the Path's `tested_stats`.

All builder cheats occur before Start. After Start it only observes completion.
Sequence, Stop/restart and death assertions belong to the live cell. Additional
bank stock or auxiliary quest flags may be inserted into the pre-Start setup,
never into the gameplay proof. Shared world entities must remain real content.

## Ordinary scenarios

Start the local engine for the profile under test and provide its client
cache. Application builds already bake and stage nav for the selected
**build-time** revision (`BOT_NAV_REVISION`, default **289**) next to the
binary — no manual pack step for ordinary WalkTo. Use `nav-pack` only for
deliberate custom-input bakes:

```sh
export ENGINE_DIR=/absolute/path/to/engine
export RS2B0T=/absolute/path/to/rs2b0t
# optional custom bake (not required for default bundled nav):
# export NAV_PACK=/absolute/output/274bot.navpack
# export NAV_FLAGS=/absolute/output/274bot.navflags
# cargo run --locked --release -p nav --bin nav-pack

cargo run --locked --release -p tui --bin tui-play -- --profile local-289 --live script_thiever
cargo run --locked --release -p panel --bin panel-play -- --profile local-289 --live script_thiever
```

Live runs mint fresh accounts named `live<token>_<i>` (at most 12
characters). Set `BOT_LIVE_NAME_PREFIX` to 1–4 lowercase letters to replace
`live`, for example `BOT_LIVE_NAME_PREFIX=tm`, so concurrent runs can be told
apart in engine logs and player saves. An invalid value stops the run.

Match `--profile` / engine ports to the revision (274: `:43594`/`:80`;
289: `:44594`/`:1080`). The door test also accepts
`BOT_NAV_DOOR_REVERSE_LOGIN=1`. Its closer remains active while the driven
player must reach the exact destination. Pack format is version **11**
(magic `274V`; `decode` rejects v10 and older as `BadVersion`). **Rebake
existing override packs** after updating. A new executable does not rewrite
an existing override pack on its own.

The `nav_door` integration test uses hard-coded Direct local-274 options and
the `ENGINE_DIR/data/pack/client` cache path. `BOT_SERVER_PROFILE` does not
select that test's endpoint:

```sh
BOT_NAV_REVISION=274 ENGINE_DIR=/absolute/path/to/274-engine LIVE=1 cargo test --locked --release -p e2e --test nav_door -- --ignored --test-threads=1
```

Direct integration-test helpers that use engine cache data require
`ENGINE_DIR`; frontend harnesses use the configured engine/cache path from
the resolved profile.
Windows uses `USERPROFILE` only when `HOME` is unavailable; an explicitly
blank `HOME` remains explicit.

Catalog `stat_xp_gain` proofs capture all later cumulative skill baselines
immediately before `StartScript`, excluding preparation XP. A gain remains
observable even if another skill or an item is watched first.
`fresh_stat_xp_gain` is deliberately different: it arms only when its own
step begins, so an earlier trip cannot qualify a later bank-return phase.
Scenarios without `StartScript` retain first-watch cumulative baselines.

Compiled-card scenarios wait for Start to reach Running and fail immediately
with the recorded rejection or preparation error. Stashed cards survive setup
settlement: a scenario Stop revokes the script walk and bank selection, verifies
the slot is Idle, and resets the card for a new Start. Quester's Cook fresh,
resume, restart and login fixtures use this same compiled Start outcome gate.

The headless Quester journal fixtures run with
`LIVE=1 cargo test -p host-play --lib live_quester_journal -- --ignored --nocapture --test-threads=1`.
Set an inline disposable `HOME`, `BOT_ENGINE_DIR` to the local R289 engine,
and `BOT_NAV_PACK` to a matching navigation pack; remove the disposable
directory after the command finishes. `BOT_GAME_PORT` and `BOT_HTTP_PORT`
override these fixtures' default local endpoints. The fixtures cover a
synthetic Rune Mysteries stage advance, parked ReadJournal retry, Stop/Start
and Pause/Resume with a retained journal page, and a DebugPanel `getcoord`
command sent while the journal is open. The overlap proof requires the host
chat-reply candidate and the journal's own fresh evidence and close sequence;
temporal debug replies are never command acknowledgements or journal progress.

The journal and fresh Cook fixtures mint their accounts like every other live
run, so simultaneous processes do not share an account and
`BOT_LIVE_NAME_PREFIX` tags an owner's fixtures. Keep the strict journal click
accounting: Stop/Pause recovery adopts the retained page without a new click.

### Gatherer target-switch checks

`gatherer_live::gatherer_mine_tier_power` supports a single selected ore through
`GATHERER_MINE_RESOURCES` and an explicit `GATHERER_MINE_TILE=x,z,level`.
At Varrock East, `(3286,3365,0)` with `tin` exercises one-step switches;
the same origin with `copper` exercises targets operable from one stand.
The fixture requires real yield, two full disposal cycles, and renewed gathering.
It seeds one uncut sapphire before Start and requires observed disposal of that
gem. Naturally mined gems are reported as observations (`not_exercised` if none);
an incidental random drop is never required to pass. The fixed deadline remains.

The Gatherer runner reselects on the same observation that completes a depleted
or missing target, or a resource-approach walk. It still validates supplies and
equipment before the next action. An already usable loc stand permits one gather
click; targets requiring movement retain native navigation and its approach and
danger-zone rules. Bank, recovery, disposal, refusal and failed-walk settlement
remain fenced to a later tick. If no replacement target is available, terminal
unavailability is confirmed on the next observation so a delayed yield packet is
accounted before the runner blocks.

## Gatherer fishing bank returns

The ignored `gatherer_live::gatherer_fish_harpoon_bank` cell tests a fixed
Start location at Catherby with an inventory harpoon and one seeded `casket`
(item ID 405). The ignored
`gatherer_live::gatherer_fish_harpoon_bank_auto` cell exercises the operator's
Auto return settings: Fishing level 76, canonical method ID
`fishing.rarefish.op3`, location `Auto`, radius 40, Bank disposition, and
Catherby bank. Its fixed test target is tile `2840,3436,0`. Initial level/tool
fixtures precede the progression baseline; no fish, casket, proof XP gains, or
randomized fishing-location state are seeded.

Both cells require at least two positive fish deposits followed by a fresh
fishing yield after each bank return. Only the fixed-location cell requires the
seeded casket in the Start baseline and verifies its deposit; the Auto case has
no seeded-casket gate. Each trip receipt derives expected item and product
counts from the actual pre-deposit inventory, excluding protected
tools/supplies; it verifies deposited IDs leave the pack and their bank counts
increase while the bank is loaded. A naturally dropped casket is recorded only
as an observation, never required. The existing preparation and wedge deadlines
still bound both runs.

After banking, Gatherer returns to the selected method's content-derived
observation stand using area arrival with a one-tile radius. The work-area radius
remains the gathering bound; a bank inside that bound does not complete Return.
Area arrival settles only at a currently loaded, standable tile near the
observation stand, then Gatherer observes resources again.

The command below runs both ignored cells (the filter intentionally is not
`--exact`):

```sh
LIVE=1 BOT_CPU=1 BOT_NAV_BUILD=skip BOT_LIVE_NAME_PREFIX=gf \
cargo test -p host-play --test gatherer_live gatherer_fish_harpoon_bank \
  -- --ignored --nocapture --test-threads=1
```

Set a disposable `HOME` with copied `BOT_CACHE_DIR`/`CLIENT_UNPACK_DIR` inputs,
absolute `GATHERER_ENGINE_DIR`, `GATHERER_NAV_PACK`, and `GATHERER_CATALOG_ROOT`,
and matching explicit `WORLD_ENGINE_DIR`/`WORLD_NAV_PACK`. The defaults use the
builder engine at `45594`/`2080`; `GATHERER_GAME_PORT`/`GATHERER_HTTP_PORT` can
select the local engine at `44594`/`1080`. `GATHERER_FISH_HARPOON_TILE` optionally
overrides the default `2840,3436,0`. `LIVE_EVIDENCE_DIR` saves a real client
capture and a receipt containing the tick-stamped bank status/event sequence.

Fishing tools are carried, not wielded. In the Auto case, radius 40 bounds the
gathering area only; it is not a bank-return shortcut. After banking, the return
must reach a resource observation stand using navigation `Area` arrival radius
1 before another catch can demonstrate return progress. NPC view is smaller
than the loaded map, so returning inside the gathering area alone does not
prove that moving fishing spots are visible. For NPC-backed fishing, the
selector derives observation stands from the content movement envelope and
accumulates empty coverage for at most 100 gameplay ticks. It caps reapproaches
at eight per method/placement; exhaustion follows existing Avoided/Exhausted
selection instead of falsely declaring the spot absent. A live actor clears
empty evidence but does not renew the approach budget; positive gather progress
does. Rock and tree placement observation still uses the loaded map rectangle.

## Walk Guard W1 live and receipt replay

The ignored W1 fixtures exercise the `death-plateau-throwers` crossing against
the local R289 engine. The gate retains the ranged-projectile queue rule
`floor((32 + 5d) / 30)` without relaxing it; at distance 1 the expected delay
is one tick. The first thrower is staged on a standable tile three to five
tiles ahead of the route start: the fixture teleports there, spawns the NPC,
then teleports back to the route start before scripted movement begins. This
avoids attributing a distance-zero launch at the start to the moving crossing.

The live and replay gate requires exactly two accepted guard clicks, both
component 5622. The enable click must show Missiles off before the click and
occur from the first launch through arrival; the off-click must show Missiles
on before the click and occur from arrival through the observed off tick.
Every per-tick crossing observation from `protect_tick` through
`arrival_tick`, inclusive, must show Missiles on. The receipt must also have
an observation for every tick from the first launch through the off tick and
three following ticks, with all prayers remaining off during cleanup. Any
extra click, non-Missiles click, missing crossing observation, Missiles lapse,
or prayer return fails the same gate used by offline replay.

Arrival uses the host's network tile (`Snapshot.tile`, derived from `route[0]`),
not the rendered actor's interpolated pixel tile. `last_tile` and
`final_snapshot.tile` use the rendered local-player tile consistently;
`final_snapshot.network_tile` retains the separate host coordinate.
If crossing completion and prayer cleanup are observed on the same game tick,
the pre-cleanup off-click snapshot supplies the genuine Missiles-on crossing
observation only when its network tile confirms arrival at the destination;
the post-click frame is a separate cleanup observation at that tick. Receipts
label those phases explicitly; they do not invent a game tick or waive the
inclusive arrival requirement.

Use the isolated worktree's nav pack and retained cache snapshot. The host
connects through builder ports `45594/2080`, forwarded to the local engine's
`44594/1080` endpoints. Set `LIVE_EVIDENCE_DIR` to the R2 evidence directory;
both the test's generated HOME and launcher HOME are under it:

```sh
WT=/path/to/walk-guard-lifecycle
ENGINE_DIR=/path/to/matching/local-289/engine
EVIDENCE=$LIVE_EVIDENCE_DIR
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
mkdir -p "$EVIDENCE/launcher-home"
cd "$WT"
export CARGO_HOME RUSTUP_HOME
HOME="$EVIDENCE/launcher-home" LIVE=1 BOT_CPU=1 BOT_NAV_BUILD=skip LIVE_EVIDENCE_DIR="$EVIDENCE" \
BOT_LIVE_NAME_PREFIX=wg \
BOT_ENGINE_DIR="$ENGINE_DIR" \
WORLD_ENGINE_DIR="$ENGINE_DIR" \
BOT_NAV_PACK="$WT/target/debug/nav/289/274bot.navpack" \
WORLD_NAV_PACK="$WT/target/debug/nav/289/274bot.navpack" \
BOT_COMBAT_CACHE_SNAPSHOT=/path/to/retained-cache-snapshot \
cargo test -p host-play --lib walk_guard_live_tests::live_walk_guard_w1_protected_crossing -- --exact --ignored --nocapture --test-threads=1
```

The separate `live_walk_guard_w1_stop_mid_crossing` test uses the real
`ScriptStartHandle::stop` API after Missiles is observed on during the crossing
and before arrival. It requires every prayer to turn off within four ticks of
Stop and remain off for three more observed ticks:

```sh
WT=/path/to/walk-guard-lifecycle
ENGINE_DIR=/path/to/matching/local-289/engine
EVIDENCE=$LIVE_EVIDENCE_DIR
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
mkdir -p "$EVIDENCE/launcher-home"
cd "$WT"
export CARGO_HOME RUSTUP_HOME
HOME="$EVIDENCE/launcher-home" LIVE=1 BOT_CPU=1 BOT_NAV_BUILD=skip LIVE_EVIDENCE_DIR="$EVIDENCE" \
BOT_LIVE_NAME_PREFIX=wg \
BOT_ENGINE_DIR="$ENGINE_DIR" \
WORLD_ENGINE_DIR="$ENGINE_DIR" \
BOT_NAV_PACK="$WT/target/debug/nav/289/274bot.navpack" \
WORLD_NAV_PACK="$WT/target/debug/nav/289/274bot.navpack" \
BOT_COMBAT_CACHE_SNAPSHOT=/path/to/retained-cache-snapshot \
cargo test -p host-play --lib walk_guard_live_tests::live_walk_guard_w1_stop_mid_crossing -- --exact --ignored --nocapture --test-threads=1
```

Both modes save a real final scene PNG with a matching JSON/Core record in an
R2 per-run capture folder under `$LIVE_EVIDENCE_DIR`; inspect that image
alongside the receipt before accepting the live proof. The final receipt's
`tile` is normalized to `local_player.tile`. The explicit navigation pack
must have its matching `.navpack.json`, `.navreach`, `.navflags`,
`.navcanlight`, and `.navpois` files beside it.

The ignored offline replay applies the same gate to the retained bad receipt
`W1-wgkiu3hcw6_0-receipt.json` and passing receipts `W1-wg9l6smxhw_0-receipt.json`
and `W1-wgefe0xaj8_0-receipt.json`, all read from `$LIVE_EVIDENCE_DIR`. The
replay skips with a message when the directory is unset or a receipt is
absent. Legacy receipts without per-tick
rows use their guard-click pre-click prayer varps, recorded protect/off
observations, and holds between clicks; the replay output marks that their
crossing frames and three post-off ticks were not directly recorded. It does
not fabricate per-tick rows. The replay table is written to
`$LIVE_EVIDENCE_DIR/W1-lifecycle-replay.json`:

```sh
LIVE_EVIDENCE_DIR=/path/to/w1-evidence \
cargo test -p host-play --lib walk_guard_live_tests::replay_walk_guard_w1_retained_receipts -- --exact --ignored --nocapture
```

## JS API v2 GatherQuest sample checks

The `host_js` integration test renders `crates/script/host-js/index.d.ts` from
the Rust API tables. Regenerate it with:

```sh
cargo test -p script --test host_js regen_host_js -- --ignored
```

The pinned TypeScript 5.8.3 consumer probes check the generated declarations,
including the `gather_quest_v2.ts` sample. Its probe checks with and without
`--strict`, emits JavaScript, and requires the checked-in
`gather_quest_v2.js` to match that emit:

```sh
cargo test -p script --test host_js -- --ignored tsc
```

## JS API v2 GatherQuest live cells

The ignored `script_api_live` tests load the checked-in `gather_quest_v2.js`
through the real Load path and exercise it against a local R289 server. The
engine and game/cache endpoints must be reachable on `127.0.0.1:45594` and
`127.0.0.1:2080`. Provide a matching `WORLD_ENGINE_DIR` and `WORLD_NAV_PACK`,
and set `BOT_CACHE_DIR` to a copied client cache.

Run from the repository root with `LIVE=1`. The `HOME` used by the test must be
a disposable directory under `BOT_EVIDENCE_DIR`; the test writes receipts
there. `BOT_LIVE_NAME_PREFIX` may be set to distinguish the minted test
account from other live runs.

```sh
LIVE=1 cargo test -p host-play --features test-support --test script_api_live -- --ignored --test-threads=1
```

`script_api_gather_and_quest` records the running/gathering phases, confirmed
yields and emptied drop slots, the Driver packet budget, the stopped session
envelope, and Cook's colour-only progress row. Its receipt is
`script-api-gather-and-quest-receipt.json` in `BOT_EVIDENCE_DIR`.

`script_api_progress_journal` uses the released Romeo & Juliet journal rules.
It seeds stage 30, reads progress, advances the fixture to stage 40 and reads
again. It requires known stage/rule keys, `inProgress`, `complete: 'false'`,
hidden journal paint while the quiet-paint lease is held, and public evidence
matching the real closed-journal observation after the acquired observation.
Its receipt, `script-api-progress-journal-receipt.json`, contains both reads,
their host evidence stamps and the journal open/close packet trace.

A scenario that waits for a card's clean stop (`wait_script_stop`) must
also watch the card's own work. Its post-Start watch and terminal proof must
be an outcome its pre-Start seed cannot already satisfy, or the run passes
on its Start snapshot and the 45-second stop grace becomes the only check.
The catalog test `clean_stop_scenarios_are_not_satisfied_by_their_own_seed`
builds each such scenario's seeded state (mainland landing, then its fixture
prerequisites) and fails any proof that already holds there. Cards that
only observe prove themselves with the receipt row they paint
(`script_receipt(prefix)`); cards that walk use `arrived_ring` around the
tile they start on.

The `ardy_cakes_fight` fixture waits, within its ordinary bounded scenario
step, for a nearby unengaged Guard with native scene line of sight before
starting the catalog script. It prepares Attack, Strength and Hitpoints 70
with the same adamant scimitar so back-to-back Guard fights do not consume
the entire stall watch. A ready Guard can still wander away: the Strength
watch tolerates one catch-less stall session, observes its bank visit,
closure and return, then requires renewed Guard readiness at the stall.
A second unqualified bank visit fails rather than admitting a third try.
The scenario has a 360-second wall deadline and a 900-dirty-snapshot combat
watch; the Thieving XP and exact Cake watches retain their original bounds.
All three still require real post-Start evidence; no catch is forced.
Script kill messages alone are not XP evidence:
in a crowded single-combat camp, a selected NPC can disappear after
another player kills it while the observing slot receives no XP.

### Core-gated qualification

A scenario PASS alone proves the scenario's own predicates. The shared core
witnesses in `host-play` (`catalog_core`, `paired_core`) prove the full
post-Start cycle. Both front ends run them through one gate,
`host_play::live_gate`:

```sh
cargo run --locked --release -p tui --bin tui-play -- --profile local-289 --live script_thiever --catalog-core
cargo run --locked --release -p tui --bin tui-play -- --profile local-289 --live script_flax_runner --pair-core
# environment form for harnesses that pass only environment:
BOT_LIVE_CORE=catalog cargo run --locked --release -p tui --bin tui-play -- --profile local-289 --live script_thiever
```

These are the panel's `catalog_watch` / `pair_watch` modes. The witness is
armed before either slot publishes, and its Start baseline is frozen
immediately before the actual isolate Start. While the witness is Pending,
a scenario PASS is held and the 45-second clean-stop grace has not started.
A failed witness, or one still unqualified at the scenario deadline
(`BUDGET_S` when set), fails the run. The compact witness receipt is printed
as `CATALOG_CORE: script_<name> {…}` / `PAIRED_CORE: script_<name> {…}`
next to the terminal line. The paired proofs (`nature_crafter_air`,
`mule_crafter_air`, `flax_runner`, `duel_arena`) refuse to run without the
pair gate.

## Fleet preparation and basic samples

Both frontends expose the same opt-in `host_play::memory::{Config, Run, Sample}`
harness. Build with `memory-profile` for allocation counters, or
`memory-profile-no-alloc` to use the system allocator directly. The latter leaves
allocator fields null while retaining real CPU, RSS, V8 and tracked GPU samples.
Unset `BOT_MEMORY_N` for normal interactive use.

```sh
cargo build --locked --release -p tui -p panel --features memory-profile-no-alloc

LIVE=1 BOT_SERVER_PROFILE=local-274 \
BOT_MEMORY_N=1 BOT_MEMORY_WORKLOAD=active BOT_MEMORY_SUSTAIN=1 \
BOT_MEMORY_WARMUP_S=30 BOT_MEMORY_OBSERVE_S=240 \
BOT_MEMORY_OUTPUT=/absolute/new-run/samples.jsonl \
target/release/tui-play --profile local-289
```

Use a real terminal for `tui-play`. Substitute `panel-play` for a native window.
Create a fresh output directory first. The harness creates isolated accounts and
an ephemeral encrypted vault; sustained Thiever preparation supplies its own food
loadout and bank stock. It does not read the operator's saved loadouts. Seed
runners share the Play navigation `Arc`; if Play has no pack, they do not silently
load another copy.

| Setting | Meaning |
|---|---|
| `BOT_MEMORY_N` | Exactly 1, 10, 16, 32, 50 or 128; accepting a count is not a capacity guarantee. |
| `BOT_MEMORY_WORKLOAD` | `idle`, `seeded-idle`, `active` or `lifecycle`; default `idle`. |
| `BOT_MEMORY_SCENARIO` | Catalog scenario for `active`/`lifecycle`; default `thiever`. Representative runner cells also use `moss_giant_bank_start` and `duel_arena`. |
| `BOT_MEMORY_WARMUP_S` / `BOT_MEMORY_OBSERVE_S` | Positive seconds, defaults 120 / 600. |
| `BOT_MEMORY_TEARDOWN_S` | Positive teardown seconds after scripts Stop; default 60. |
| `BOT_MEMORY_SUSTAIN=1` | Active/lifecycle Thiever only: four food initially, stock in the bank, target 22 food and restocking at three. Other scenarios reject this setting. |
| `BOT_MEMORY_DIAGNOSTICS=1` | Optional bounded frames, logs, requests and per-script progress sidecar. |
| `BOT_MEMORY_RENDER_POLICY` | Panel `rotating-all`, `fixed-one`, `focused-one`, `focused-plus-background`, `stress50` or `stress50-full`. `stress50` draws one of 50 slots; `stress50-full` draws all 50 at full rate. |
| `BOT_MEMORY_SINGLE_RENDERER=1` | Legacy fixed slot-zero rendering. |
| `BOT_CPU=1` | CPU fallback for the panel's client renderer. |

`idle` only logs clients in. `seeded-idle` performs the ordinary Thiever seed
without starting the catalog script. `active` starts the selected real catalog
script; `lifecycle` additionally alternates Stop and restart every 60
observation seconds. After observation, scripts Stop and the harness remains
open for `BOT_MEMORY_TEARDOWN_S`.

Ingame readiness requires every slot to remain `ingame && scene_state == 2`
continuously for two seconds. A transient ready pulse before the mainland-hop
reload does not latch. Samples report that settled `all_bots_ingame_scene2_s`
milestone separately from `qualification_complete_s`. Qualification additionally
requires seed/proof completion and running scripts for active workloads.
Multi-slot `moss_giant_bank_start` replaces only its contended post-return
fresh-XP checkpoint with fail-closed local-player named-NPC engagement; the
scenario's final Strength-XP-since-Start proof remains mandatory.

`duel_arena` is the pair-script cell: every slot runs the frozen Duel Arena
Combat Trainer, so N=10 is five duels and N=50 is twenty-five, and an odd N is
refused at start. The fixture seeds a bronze scimitar (the existing
`duel_arena` pair seed) and stages mint-order pairs so that at Start the only
free challenge-area candidate is the partner. Unstarted slots wait at the Al
Kharid bank hold, outside the lobby. A finished pair is parked back at the hold
before the next pair is admitted, so an already-fought slot is not a lobby
candidate on the frame the completed-duel watch advances and the parking tele
has not yet been sent (`ScenarioRunner` only `advance_step()`s that tick;
`play_slots` then runs `slot_frame`).

Admission has one owner with the login FIFO's shape: one mutex, slot owner
tokens, a FIFO of Start requests, a grant, and a slot-worker retirement guard.
Every slot thread's per-frame position report, every admission and Start
decision, every breach and every fence transition happen under that one lock,
so no decision reads a half-updated fleet. The benchmark poll Starts a slot only
on a granted permit, and a grant needs every slot outside the pair to have
reported a clear position (at least 20 steps from `DUEL_ZONE`) with an epoch
newer than the request. A clear reading from before the request, left by a
stalled or delayed slot thread, cannot admit a Start; the request waits for that
thread's next report. A slot worker that exits or panics while the fence is up
fails the run instead of stalling the grant.

The frozen card walks a parked slot back toward the arena. The fixture
re-teleports it once it is 8 tiles from the hold, but that tele is neither
acknowledged nor sent while the scene settles, so it is not the guarantee. The
zone-clearance fence is: every in-scene client frame of a waiting or parked
slot reports its tile, not gated by scene settling, and the first such tile
closer than 20 steps to the challenge area records `duel fleet clearance
breach: slot … was seen at …` under the admission lock. After it no permit is
granted and the next benchmark poll fails the run with it. The fence stays up
after every pair has entered a pen: it comes down only once the last pair has
parked back at the hold and every slot has reported a clear position since, so a
late report from near the arena is a breach rather than a release. Parked slots
are then released to the lobby so observation is a fighting fleet.

A step moves at most one tile on each axis and a running player takes two steps
per game tick, so a slot last seen outside the clearance is at least 10 game
ticks (6 s) from becoming a candidate, and its thread reports it on the way. The
hold is 59 steps from the challenge area, which leaves room for the correction's
own excursion (22 tiles from the hold at most in the live N=50 runs). The fence
bounds distance to the arena, not to the hold: parked slots have landed at the
Lumbridge spawn, 106 steps out, and are simply re-teleported. What remains is
that a grant proves every other slot was clear at a moment after the request,
not at the instant the started card evaluates `challengeTargets()`: a
fought slot moved straight into `DUEL_ZONE` after a grant and before its next
report would be a candidate first and a breach (the run fails) second. The
fixture has no such mover: its corrective teles target the hold, observed
respawns land 106+ steps out, and the card walks at least 10 ticks through the
fence. The qualification records carry a `duel_gate` object with the
clearance, the closest approach and the farthest hold distance seen for a
fenced slot, the fence state, the permits granted, any pending Start and the
slots it still awaits a clear report from, and the fleet failure if any. Each
slot qualifies only after its own snapshots show a fight pen visit followed by a
return to the lobby, the transition the card counts as a finished duel. A duel
only moves its two fighters (no stakes, HP restored, no spawned or dropped world
state), so a cell leaves nothing for the next one.

Seed/proof or script failures fail the run and return a nonzero exit, and so does
a fleet that has not qualified after 30 minutes (`blocked: ready=… seeded=…
proved=… wanted=… ever_ready=…`). A `duel_arena` fleet stages one pair at a time,
so that bound is 60 minutes; on the reference M4 Max host N=50 qualifies in
about 40 minutes and N=10 in about 8. The perf runner's default per-cell timeout
is this bound plus the warmup, observe and teardown windows and 300 s of launch
and exit time, so the product's own bound, not the runner's kill, ends an
unqualified cell. Every such failure first appends one
`"phase":"failed"` record to `samples.qualification.jsonl` with the error and,
for every slot, whether it ever reached `ingame && scene_state == 2` (and when),
its last observed session state (startup phase and how long it has been in it,
login-queue place, error, worker terminal, latch, welcome hold), its scenario
step by name, and its script state and error. `qualified_now` says whether the
slot satisfies the run's own qualification definition at that instant (for an
active workload: in game, proof passed and script running), so a ready slot
whose script has stopped is not reported qualified; `reached_ingame_scene2` is
history. The observation-boundary records carry the same per-slot fields.
Inspect them before treating a run as a
benchmark: a process that exits normally is not by itself evidence that every
bot did useful work for the whole interval.

## Reading results

`samples.jsonl` separates current resident bytes from process-lifetime peak RSS.
It also records cumulative process CPU, optional allocator counts, V8 sample
coverage/age, snapshot capacity and tracked GPU buffer/texture bytes. Host
mainloop/observe/raster fields are cumulative **wall-time** counters, including
waits, preemption and contention; they are not CPU time. Retired slot counters
remain in the per-username process totals across relogs, while
`host_profile_slots` counts only active slots.

Sample JSON serialization is buffered to avoid a file write for every JSON
token, including on shared VM volumes. Each complete row is explicitly flushed
before the sample poll returns, so the frontend's immediate process exit does
not leave completed rows in a userspace buffer. This does not change the sample
fields or exclude sample-writing work from the whole-panel frame timer.

Panel samples include a cumulative one-millisecond frame histogram whose final
bucket means “at least 250 ms”, cumulative `ui_frame_count` and
`ui_frame_total_ns`, and a swap-reset `ui_frame_max_ns` for that sample
interval. A receipt must label overflow percentiles as censored and report
overflow count, observation-window mean and the maximum of the interval maxima
rather than presenting 250 ms as an exact slow-frame duration. Requested
rendering metadata does not prove the observed backend or cadence.

Every sample row also carries run metadata, each field a JSON `null` when it does
not apply (a headless `tui-play` has no adapter or panel frames):

- `adapter`: the wgpu adapter the panel selected (`name`, `backend`,
  `device_type`, `driver`, `driver_info`, `vendor`, `device`, `pci_bus_id`), with
  `adapter_selections` counting how many times the panel selected one (more than
  one means the GPU stack was rebuilt). It is recorded once, in the run metadata,
  without `BOT_DEBUG` and without a log line.
- `game_data`: the profile's generated-facts trust decision with its reason:
  `attached`, or withheld because the selection is `unsupported-server`, the
  cache does not match the generated asset (`cache-identity`), or a pinned
  generator source failed verification (`source-rejected`, with how many of the
  inputs failed and the first failures by root, pinned path and kind). Facts are
  withheld exactly as before; only the cause is recorded, and the same cause is
  logged once at warn level when a profile is bound for play.
- Startup timeline, in seconds from the process entry the frontend `main`
  recorded: `startup_window_created_s`, `startup_gpu_ready_s`,
  `startup_first_render_s`, `startup_first_frame_presented_s` (the first
  presented panel frame), `startup_first_client_frame_s`, and
  `run_started_process_s` (when this harness's own timer began), all placed
  against launch by `process_epoch_unix_ms`. `render_gap_*` times the space
  between panel render callbacks, which the frame histogram does not: the longest
  gap since the process began (`render_gap_max_ns`, ended at
  `render_gap_max_at_s`), the longest in this sample interval
  (`render_gap_interval_max_ns`, swap-reset), and how many gaps reached five
  seconds (`render_gaps_ge_5s`), the point at which an OS “Not Responding” state
  can begin. A large gap shows the UI thread was elsewhere; it does not by itself
  prove the OS marked the window unresponsive.
- `ready_ever`: how many slots have ever been `ingame && scene_state == 2`,
  against `ready`, how many are now. Active workloads also carry `seeded`
  (scenario runners that passed, or sit at the Start step waiting for the script
  to load) and `proved` (runners that passed their final proof), so a fleet that
  stalls shows when `proved` stopped rising; both are `null` for idle workloads.

`samples.qualification.jsonl` contains observation start/end progress and, for a
failed fleet, the per-slot failure record above. With optional diagnostics,
`samples.diagnostics.jsonl` adds bounded per-slot evidence.
Detailed navigation history is null because the campaign capture system is not
part of this harness. Direct-owner census, scheduling/latency journals, managed
process controllers and campaign replay/provisioning tools are not dependencies.

Compare matched workloads, platforms and intervals. Allocation avoidance is not
necessarily an RSS reduction, and current RSS must not be confused with peak RSS
or allocator counts. Negative or non-monotonic RSS slopes can be sampler noise,
especially with macOS `resident_size`; they are measurements, not savings
claims. The selected fixes have focused allocation/ownership evidence; they do
not establish an additive total saving, universal responsiveness improvement,
low-end budget, or 128-client capacity guarantee.

## Known validation limits

The revision-specific stun recovery infers an onset from spot animation 245 and
waits eleven distinct player-update ticks, at most once per follow run. It is not
a universal stunned flag. Seeing an ordinary thieving stun does not prove the
navigation recovery path ran.

The client still has a documented Windows/Linux GPU shade-boundary test failure
(expected approximately 223, observed 255) and a Windows unreachable-CRC timing
assumption that can exceed five seconds. The modal restoration addresses a
separate black-modal defect. Some older panel unit fixtures create real network
workers and need a reachable local fixture for prompt teardown; this remains a
test isolation limitation.

Runner completion is not script qualification. Prefer capability language over
copying rapidly changing pass/fail inventory counts into product docs.
