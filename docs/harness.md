# Reusable live harness

The harness runs real revision-274 clients against your local engine. It keeps
scenario preparation, proof predicates and failure exits in Rust so preservation
projects can reuse the same behavior through `host-play`, the TUI or the panel.
No engine assets or external catalog scripts are included in this repository.

## Ordinary scenarios

Start the local engine, provide its client cache, and bake a navigation pack:

```sh
export ENGINE_DIR=/absolute/path/to/Server/engine
export RS2B0T=/absolute/path/to/rs2b0t
export NAV_PACK=/absolute/output/274bot.navpack
export NAV_FLAGS=/absolute/output/274bot.navflags
cargo run --locked --release -p nav --bin nav-pack

LIVE=1 BOT_TARGET=local cargo test --locked --release -p e2e --test nav_door -- --ignored --test-threads=1
cargo run --locked --release -p tui --bin tui-play -- --live script_thiever
cargo run --locked --release -p panel --bin panel-play -- --live script_thiever
```

The door test also accepts `BOT_NAV_DOOR_REVERSE_LOGIN=1`. Its closer remains
active while the driven player must reach the exact destination. The pack is
still version 8: **rebake old v8 packs** to receive the adjacent-door edge fix.
A new executable does not rewrite an existing pack.

Some existing integration-test helpers use the default
`HOME/experiments/Server/engine/data/pack/client` layout. Check the relevant test's
cache options when running it on another machine; setting `ENGINE_DIR` is not a
universal override for those older helpers. Frontend harnesses use the configured
engine/cache path. Windows uses `USERPROFILE` only when `HOME` is unavailable;
an explicitly blank `HOME` remains explicit.

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
target/release/tui-play
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
