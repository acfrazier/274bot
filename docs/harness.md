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
player must reach the exact destination. Pack format is version **10**
(magic `274V`; `decode` rejects v9 and older as `BadVersion`). **Rebake
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
| `BOT_MEMORY_N` | Exactly 1, 16, 32 or 128; accepting a count is not a capacity guarantee. |
| `BOT_MEMORY_WORKLOAD` | `idle`, `seeded-idle`, `active` or `lifecycle`; default `idle`. |
| `BOT_MEMORY_WARMUP_S` / `BOT_MEMORY_OBSERVE_S` | Positive seconds, defaults 120 / 600. |
| `BOT_MEMORY_SUSTAIN=1` | Active Thiever fixture with four food initially, stock in the bank, target 22 food and restocking at three. |
| `BOT_MEMORY_DIAGNOSTICS=1` | Optional bounded frames, logs, requests and per-script progress sidecar. |
| `BOT_MEMORY_RENDER_POLICY` | Panel `rotating-all`, `focused-one` or `focused-plus-background`. |
| `BOT_MEMORY_SINGLE_RENDERER=1` | Legacy fixed slot-zero rendering. |
| `BOT_CPU=1` | CPU fallback for the panel's client renderer. |

`idle` only logs clients in. `seeded-idle` performs the ordinary Thiever seed
without starting the catalog script. `active` starts the real catalog script;
`lifecycle` additionally alternates Stop and restart every 60 observation seconds.
After observation, scripts Stop and the harness keeps a 60-second teardown window.

Readiness requires every slot to be `ingame && scene_state == 2`. Seed/proof or
script failures fail the run and return a nonzero exit. The observation-boundary
qualification file records each slot's state, error and script progress. Inspect
those records before treating a run as a benchmark: a process that exits normally
is not by itself evidence that every bot did useful work for the whole interval.

## Reading results

`samples.jsonl` separates current resident bytes from process-lifetime peak RSS.
It also records cumulative process CPU, optional allocator counts, V8 sample
coverage/age, snapshot capacity and tracked GPU buffer/texture bytes. Timing
fields are basic count/total/maximum aggregates, not latency percentiles.
Requested rendering metadata does not prove the observed backend or cadence.

`samples.qualification.jsonl` contains observation start/end progress. With
optional diagnostics, `samples.diagnostics.jsonl` adds bounded per-slot evidence.
Detailed navigation history is null because the campaign capture system is not
part of this harness. Direct-owner census, scheduling/latency journals, managed
process controllers and campaign replay/provisioning tools are not dependencies.

Compare matched workloads and intervals. Allocation avoidance is not necessarily
an RSS reduction, and current RSS must not be confused with peak RSS or allocator
counts. The selected fixes have focused allocation/ownership evidence; they do
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
