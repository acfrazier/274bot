# Reusable harness extraction

Prepared September 9, 2026, for the approved selective merge. No live service,
remote, submodule, main-branch, or campaign-controller operation was performed
by this implementer.

## Code-only commits

- `85a9a1d`: existing reusable `Run`/configuration/preparation/output foundation,
  explicit fixtures, frontend wiring, shared navigation, terminal cleanup, and
  basic accounting.
- `f2c260d5`: accurately mark excluded detailed navigation history unavailable;
  correct the shared-world accessor documentation.

Branch: `codex/selective-harness`, based on `54cfcf8`.
The commits deliberately exclude the client gitlink and host dependency lockfile.
Root supplies selected client `d5f26e3` (including the small `b555f84` basic
accounting dependency), current-resident/resource changes, and the product
routing/banking group `c5e0f03`.

## Retained behavior and source map

- `e6ec9e6` / `e1ebd59` foundation: the existing `host_play::memory::{Config,
  Sample, Run}` entry points, optional frontend features, ephemeral account/vault
  preparation, output creation, local/LIVE guard, `ingame && scene_state == 2`
  readiness, script error/proof checks, warmup/observation/Stop lifecycle, and
  failure exit behavior.
- Required `a52b12c` pieces: sustained Thiever setup, explicit fixture loadouts,
  real per-isolate snapshot/V8 counters and script progress, basic GPU counters,
  bounded optional frame/log/request diagnostics. Both sustained and ordinary
  harness starts now explicitly supply fixture loadouts; the ordinary fixture
  supplies an empty list instead of reading operator loadouts.
- `c4943a7`: system-allocator mode (allocator fields null when uncounted), actual
  process CPU, and qualification output at observation boundaries.
- `d3abfb4`: seeded idle without script/catalog startup.
- `7d026bc` + `606c93b`: seeds share the Play `Arc`; panel/TUI prepare the vault
  first, construct Play, bind navigation, then spawn slots. `FromPlay(None)`
  remains missing without another pack decode.
- `9f687af`: N16 and existing explicit panel rendering policies.
- `122ad32` + `0e909b9`: release completed runner snapshots only after callbacks
  and same-tick terminal evidence finish.
- Final `4b1d0bb` / `273ecd4` / `f9675b6` intent: a test-only per-session spawn
  switch keeps both catalog-preparation unit fixtures on the actual production
  preparation path without creating network workers. No duplicate preparation
  implementation or production spawn switch was introduced.
- `c8b9334` memory.rs hunks: Windows process CPU sampling, honest unavailable
  peak results, portable monotonic sampling regression. `61b7b7d` null-device
  fixtures belong to the excluded failure-capture framework; no retained test
  opens `/dev/null`, so a NUL replacement is unnecessary here.

## Output inventory and exclusions

Retained sample fields include frontend/count/workload/phase/readiness/activity;
current and peak resident bytes; optional allocator counts/bytes; actual snapshot
inflight bytes/capacity and peak capacity; V8 usage, live/sample counts and age;
actual tracked GPU buffer/texture/peak bytes; CPU user/system time; basic script,
client-tick and frontend timing aggregates; and requested rendering policy.
Actual per-script progress/paint/inventory/skill data remains in qualification
records at observation start/end. The optional diagnostic sidecar keeps bounded
client frames, logs, requests, and actual request/sent-batch counts.

Detailed navigation history is explicitly `null` in that sidecar because its
capture producers are excluded. It must not be interpreted as a measured empty
history. Advanced scheduling/latency/cohort journals, navigation captures,
failure-capture controller, direct-owner/census accounting, snapshot dedup,
borrowed fingerprints, and tiled navigation are absent. The dense world remains.
No allocation counter is described as RSS and unavailable domains are not
fabricated as zero.

## Verification performed

macOS native, selected client `d5f26e3`, with temporary product dependencies
from `c5e0f03` and root's `751f085` RSS source applied for the combined checks:

- `cargo check -p host-play -p panel -p tui --features memory-profile --offline`:
  passed.
- Same check with `memory-profile-no-alloc`: passed.
- `cargo test -p host-play --features memory-profile memory::tests --offline`:
  all 30 selected tests passed; no live tests ran.
- `cargo test -p scenario --lib --offline`: 78 passed, including terminal
  callback/evidence and cleanup regressions.
- `cargo test -p panel --lib --features memory-profile focus::tests --offline`:
  12 passed, including one/sixteen-slot and legacy rendering-policy regressions.
- `cargo test -p tui --lib --features memory-profile live_prepare_ --offline`:
  both final preparation fixtures passed in 1.86 seconds, without live workers.
- `cargo test -p script --lib --features memory-profile memory_profile::tests
  --offline`: all 3 accounting lifetime/teardown tests passed.
- Bounded diagnostic queue regression: passed.
- `git diff --check`: passed.

These are extraction and functional unit checks, not a new performance result,
live acceptance, or native Windows/Linux execution. Root still owns combined
full validation, native platform availability, the bounded live checks and final
Grok review. The harness branch currently has temporary uncommitted/staged
product/RSS dependencies used for compilation; only the two named commits are
its extraction deliverable.
