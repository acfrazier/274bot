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

Match `--profile` / engine ports to the revision (274: `:43594`/`:80`;
289: `:44594`/`:1080`). The door test also accepts
`BOT_NAV_DOOR_REVERSE_LOGIN=1`. Its closer remains active while the driven
player must reach the exact destination. Pack format is version **11**
(magic `274V`; `decode` rejects v10 and older as `BadVersion`). **Rebake
existing override packs** after updating. A new executable does not rewrite
an existing override pack on its own.

The `nav_door` integration test uses the local-274 engine and the fixed
`$HOME/experiments/Server/engine/data/pack/client` cache path:

```sh
BOT_TARGET=local BOT_NAV_REVISION=274 ENGINE_DIR="$HOME/experiments/Server/engine" LIVE=1 cargo test --locked --release -p e2e --test nav_door -- --ignored --test-threads=1
```

Some existing integration-test helpers use the default
`HOME/experiments/Server/engine/data/pack/client` layout. Check the
relevant test's cache options when running it on another machine; setting
`ENGINE_DIR` is not a universal override for those older helpers. Frontend
harnesses use the configured engine/cache path from the resolved profile.
Windows uses `USERPROFILE` only when `HOME` is unavailable; an explicitly
blank `HOME` remains explicit.

Catalog `stat_xp_gain` proofs capture all later cumulative skill baselines
immediately before `StartScript`, excluding preparation XP. A gain remains
observable even if another skill or an item is watched first.
`fresh_stat_xp_gain` is deliberately different: it arms only when its own
step begins, so an earlier trip cannot qualify a later bank-return phase.
Scenarios without `StartScript` retain first-watch cumulative baselines.

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

LIVE=1 BOT_TARGET=local \
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
free challenge-area candidate is the partner. Unstarted slots wait outside the
lobby. A finished pair is parked back outside before the next pair is admitted,
so an already-fought slot is never a lobby candidate for a starting pair —
including on the frame the completed-duel watch advances and the parking tele
has not yet been sent (`ScenarioRunner` only `advance_step()`s that tick;
`play_slots` then runs `slot_frame`). Each slot qualifies only after its own
snapshots show a fight pen visit followed by a return to the lobby, the
transition the card counts as a finished duel. After every slot has entered a
pen, parked slots are released to the lobby so observation is a fighting fleet.
A duel only moves its two fighters (no stakes, HP restored, no spawned or
dropped world state), so a cell leaves nothing for the next one.

Seed/proof or script failures fail the run and return a nonzero exit, and so does
a fleet that has not qualified after 30 minutes (`blocked: ready=… seeded=…
proved=… wanted=… ever_ready=…`). A `duel_arena` fleet stages one pair at a time,
so that bound is 60 minutes. Every such failure first appends one
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
