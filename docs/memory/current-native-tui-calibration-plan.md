# Current-source native TUI N16 calibration boundary

Captured 2026-09-08 for the bounded design/provenance audit. This document
releases no build, live run, candidate comparison, or acceptance decision. It
defines one possible current-source diagnostic screen for root review under
`performance-finish-plan.md` sections 1, 3, and 5.

## Decision boundary

The old Linux N1/N16 screen is not current-source calibration evidence. It used
host `b4b686f`, client `3456edc`, and binary
`f00e7fb18e28c013fc173e78d956bd4db4b819fd28a3a39b8784de71a7ed2546`, with
profile-enabled short observations. Its N16 median was 610.68 MiB and CPU
0.72958343 cores, but that source lineage predates subsequent host/client
changes (`native-platform-resource-screen-report.md`, lines 24–35, 57–76).
That lineage mismatch is a named calibration confounder. It does not authorize
re-running the parked terminal/shared-navigation, borrowed-fingerprint, or
other candidate comparisons.

This screen has one purpose: determine whether a current, production-default,
native Linux TUI N16 active run can be bound to a complete source/binary/cache/
server/terminal/accounting record strongly enough to choose the next measured
owner. It may expose a current baseline miss, a current calibration failure, or
a bounded owner lead. It cannot accept a saving, close the final matrix, or
turn an old result into a current result.

The result must answer only:

1. Did the current source and client gitlink actually produce the executable
   that ran on the target?
2. Did 16 declared slots become ready and remain active with the preserved
   script/fidelity/cadence contract?
3. What are the separately measured frontend RSS, CPU, progress/cadence,
   server, terminal/controller, and collector costs over the declared window?
4. Which next owner is indicated by a measured discrepancy: source/binary
   calibration, harness/accounting, fixed host ownership, per-slot growth, or
   workload/fidelity?

It must not answer whether snapshot dedup, terminal cleanup, borrowed
fingerprints, or another parked candidate is accepted. It must not claim final
TUI budgets, fleet savings, capacity, or RSS savings from malloc/allocation
counts.

## Frozen screen proposal

Root may release exactly one current-source screen after design, integration,
and prerequisite review. The proposed frozen declaration is:

| Field | Frozen value or rule |
|---|---|
| Frontend | native Linux `tui-play`, TUI mode, no renderer attached; the TUI source forces every spawned profile to `RasterMode::Off` (`crates/tui/src/bin.rs`, lines 525–544) |
| Scale | `BOT_MEMORY_N=16`; all 16 active slots are required to become ready and make progress |
| Workload | `BOT_MEMORY_WORKLOAD=active`; preserve the existing active fixture, script, server, account population, and cadence. Do not substitute idle, seeded-idle, lifecycle, or an inert fixture |
| Timing | `BOT_MEMORY_WARMUP_S=120`, `BOT_MEMORY_OBSERVE_S=600`; these are the harness defaults and are explicit in the declaration (`crates/host-play/src/memory.rs`, lines 150–165) |
| Harness/build | Build `tui-play` with `memory-profile-no-alloc` so the reviewed `BOT_MEMORY_N=16` harness, qualification publisher, and `Run` receipt path are present; this selects `System` and disables allocation counting. It is an allocation-unprofiled diagnostic binary, not the ordinary no-feature product binary |
| Runtime instrumentation | Hot profiling, stack logging, verbose allocator instrumentation, and debug output OFF; external managed process sampling remains required and is separately accounted |
| Snapshot feature | feature-off production default; do not pass `snapshot-dedup` |
| Target | existing Concord native Linux host: Ubuntu 24.04.4 x86_64, 2 logical CPUs, 1,967 MiB RAM, no swap. This is below the approved 4 GiB VPS reference and must be reported as a capacity-limited target, not silently called a 4 GiB validation (`concord-native-environment.md`, lines 1–6) |
| Server | existing qualified co-located fixture/server, sampled as its own role and never added to frontend RSS/CPU |
| Terminal | real PTY-backed managed TUI path, with terminal geometry captured and bound; no pipe-only or inert terminal fixture |
| Renderer | none; TUI is not a panel/GPU result |
| Source | current root host checkout and its exact client submodule gitlink frozen immediately before build; do not reuse the historical `b4b686f`/`3456edc` artifact |
| Order/repeats | one attempt only. No baseline/candidate pair, parked-candidate comparison, catch-up input, or favorable retry is included |

The executable should be a current-source locked release build of `tui-play`
with `memory-profile-no-alloc`. This feature is required for the harness and
publisher, but its `System` allocator means allocation counting is off; it must
not be mislabeled as the ordinary production binary. Build it on the current
native Hyper-V Ubuntu 24.04 amd64 builder: the readiness proof records a
Generation-2 VM with 4 vCPU, 8 GiB static memory, no swap, native Rust/Cargo
1.98.0, and passing compile/run smoke checks (`hyperv-builder-native-proof-report.md`,
lines 15–21, 32–59). That is a build environment, not the 2-CPU/1,967-MiB
Concord runtime target. The older Docker image/digest and its `f00e7fb` binary
remain historical evidence only (`concord-linux-build-report.md`, lines 7–35);
do not use them as the current screen artifact. The native builder must produce
a new hash, source manifest, feature manifest, and ELF receipt.

The managed execution path must reuse `docs/memory/run_managed_cell.py` and its
reviewed PTY binding, receipt, role sampler, bounded-output, and teardown
controls. The Rust harness publisher supplies the N16 qualification fields;
the managed collector supplies external process evidence. Do not replace this
with a manual product-UI collector or a pipe-only launcher. Any missing proof
from the reviewed path remains `unavailable` and blocks qualification.

The TUI command must use explicit current cache and vault paths, target host and
port, and the authorized vault/account fixture. The CLI defaults are derived
from `client::cache_dir()`, the default vault, and the current bot target
(`crates/tui/src/bin.rs`, lines 90–124), but defaults alone are insufficient
provenance. Record the resolved values without recording passwords or secrets.
The TUI always disables per-slot rasterization, while profile defaults still
include low-memory true, auto-login false, GPU raster, random events true, and
lamp auto-use true (`crates/vault/src/lib.rs`, lines 36–49, 87–97). The screen
must preserve the fixture's existing account settings rather than normalize
these values to improve memory.

## Current feature/default audit

`snapshot-dedup` is not a production default. `crates/tui/Cargo.toml` declares
it as an opt-in feature forwarding to `api`, `host`, and `host-play`; the
ordinary `[features]` default is empty and `memory-profile` is separate. The
same opt-in shape is present in `crates/host-play/Cargo.toml`, while
`crates/api/Cargo.toml` explicitly documents feature-off as the original Vec
storage. Therefore the clean current screen must use feature-off production
semantics. A future dedup measurement would be a separately approved candidate
comparison, not this calibration. “Feature-off” here means the production
snapshot-dedup family is off; the diagnostic harness feature is deliberately
present so the declared N16 fixture and receipts can actually be produced.

`memory-profile-no-alloc` is not production behavior. It enables the benchmark
harness and selects `System` rather than `CountingAllocator`
(`crates/host-play/src/memory.rs`, lines 26–34); `tui/Cargo.toml` forwards it
through `memory-profile`, which is why the harness/publisher exists. It is the
coherent executable for this structured diagnostic, but its harness/observer
overhead is not automatically zero. The old Linux screen was also
profile-enabled and therefore cannot be treated as an unprofiled baseline.

### Profiled versus unprofiled metrics

The one released screen is unprofiled for allocation attribution, not
harness-free: it uses `memory-profile-no-alloc` and `BOT_MEMORY_N=16` solely to
activate the reviewed fixture, fixed phases, and qualification publisher. It
must not enable `memory-profile` counting semantics, allocator counters,
allocation stack tracing, hot profiling, or debug output. External managed
collection may sample the named frontend,
server, terminal/controller, launcher, and collector roles, but collector
self-cost and instrumentation overhead must be reported separately. The clean
screen's resource and cadence values are diagnostic observations, not final
acceptance until all gates are met.

If root later authorizes a counting/profile-enabled attribution run, it must use the
same frozen source, binary manifest, cache, server, geometry, and fixture, and
must be labeled profiled. Its Rust/V8/snapshot counters can identify ownership
leads only. They cannot be subtracted from RSS, used as RSS, or converted into
an accepted saving. The resource harness itself states that metric fields are
separate domains and must not be summed or equated with RSS
(`crates/host-play/src/memory.rs`, lines 1–6). No such paired profile run is
released by this document.

## Required provenance and prerequisites

Root must verify all of the following before launch. A missing item is a
precondition failure, not a reason to launch and repair the record afterward.

### Source and binary

- Current root commit, worktree cleanliness for source/build inputs, and exact
  client submodule gitlink; record both hashes and the submodule worktree state.
- Complete source-file manifest/hash set for host crates and client inputs,
  including the current TUI/host/api/script/nav code and any generated source
  consumed by the build.
- Locked dependency identity and native-builder state/provenance, OS/architecture,
  compiler/Cargo versions, target triple, release profile, and exact feature
  list. The feature list must show `snapshot-dedup` absent and
  `memory-profile-no-alloc` present for this allocation-unprofiled harness
  screen; it must also show counting/stack/debug switches OFF.
- Final ELF hash, size, interpreter, architecture, dynamic dependencies, and
  read-only staged path. Do not use the old `f00e7fb` artifact.
- Required host and client tests/build checks must be completed separately
  before release. If client code or the client gitlink changes, run actual
  native client integration tests separately; host tests do not substitute for
  client integration tests. This screen itself does not authorize executing
  those checks.

### Cache, navigation, server, and account binding

- Immutable cache/catalog fingerprint and resolved path bindings, including
  version list, snapshot files, jag files, backing-store presence/absence, and
  content hashes. The existing cache utility records configuration and content
  but does not prove which path a running client selected
  (`cache-provenance-report.md`, lines 1–31); the run receipt must bind the
  launched settings to that fingerprint.
- Current nav-pack/catalog/world hashes and the exact server fixture hash. Do
  not infer nav identity from a launcher default.
- Server process PID, start identity, fixture/engine commit, readiness, loopback
  endpoints, and separately sampled server RSS/CPU. The Concord fixture is
  based on engine `4c95f87...`, with 688 hash-checked fixture files and a
  separately identified server PID in the existing environment report
  (`concord-native-environment.md`, lines 14–27).
- Exact 16-account/profile population, settings, script name and revision,
  scenario parameters, vault identity/path, and a redacted account binding.
  No passwords, tokens, or production players enter the report.
- Readiness proof for all 16 slots: `ready=16`, `active=16`, ingame/scene
  state, per-slot progress, no loss or pending terminal records, and the
  preserved 20 ms logical client scheduling target. Server ticks, client
  iterations, and presented frames remain distinct metrics.

### Terminal and accounting

- Real PTY allocation, terminal dimensions, TERM/locale, PTY owner, and a
  managed reader/controller with bounded output and explicit EOF/join receipts.
  Record the PTY, launcher, controller, and collector as distinct roles.
- Continuous PID plus start-identity binding for frontend, server, launcher,
  controller, and collector; no PID reuse or foreign descendant is silently
  folded into the frontend.
- Per-role RSS and interval/cumulative CPU with actual sample brackets and
  observation span. The existing accounting design measures named-role RSS and
  CPU but not a complete process tree or live-child RSS; waited-child CPU is
  distinct and child current RSS is unavailable (`process-accounting-report.md`,
  lines 140–151). Report those limits explicitly.
- Collector overhead measurement or an explicit `unmeasured` status. Do not
  report helper RSS/CPU as frontend cost, and do not double-count the managed
  frontend. The recent Windows pair demonstrates why helper cost and missing
  resource provenance must remain visible (`windows-cohort-pair-1430-report.md`,
  lines 23–43).
- A fixed observation-start receipt, actual warmup/observation timestamps,
  stop/teardown receipt, archive hashes, and a complete chronological event
  chain. A configured duration is not an observed duration.

## What to record and how to interpret it

The screen records frontend-only and separately owned values:

- steady median RSS and observed lifetime peak RSS;
- frontend CPU as CPU seconds divided by wall seconds, with interval brackets;
- ready/active counts, client iterations per slot per second, p99 start interval,
  decode-to-dispatch latency if complete, and terminal input-to-visible
  acknowledgement only if a real TUI receipt covers it;
- server RSS/CPU, terminal/controller/collector RSS/CPU, and pressure/swap state
  as separate roles;
- per-slot progress, navigation state, script actions, and error/loss counts;
- source/binary/cache/nav/server/terminal/accounting provenance booleans;
- missing-data statuses, not zero substitutions.

The approved TUI targets remain <=256 MiB steady median RSS for one bot,
<=512 MiB for 16 active bots, <=8 MiB additional idle bot, <=16 MiB additional
active bot, and <=0.5 average process CPU core at 16 bots
(`performance-finish-plan.md`, lines 58–75). This one N16 screen cannot measure
idle finite difference or accepted N+15 growth, so it must mark those gates
`not measured`. A current N16 miss may point to fixed cost or scale, but cannot
separate them without a matched N1/N16 design.

A malloc/allocator count, V8 heap field, snapshot-inflight field, or native
allocation census is an ownership clue only. It is not RSS, does not prove
resident savings, and must never be subtracted from the process RSS row. The
historical Linux N1/N16 arithmetic was explicitly unlike-N descriptive
arithmetic, not a fitted scaling law or matched saving
(`native-platform-resource-screen-report.md`, lines 64–76).

## One-attempt stop and qualification rule

This is one bounded attempt. Stop before or during launch on any prerequisite
failure, missing source/binary/cache/server/terminal/accounting binding,
non-finite or incomplete receipt, wrong feature/default, wrong PID identity,
missing terminal geometry, or failure to obtain 16 ready/active slots in the
predeclared startup window. Preserve the partial archive and classify it as
`calibration_failed` or `declared_population_incomplete`; do not retry, send
catch-up input, or change the fixture.

After readiness, observe exactly the declared active window once. Stop and
classify the attempt as diagnostic-only/incomplete if any slot loses progress,
terminal records are incomplete, role identity changes, the server is not
separately accounted, the observation span is shorter than configured, or
resource provenance/collector overhead is unavailable. A clean process exit is
not a pass. A missing metric is `unavailable`, never zero.

Qualify the screen only as `current-source-calibrated-diagnostic` when all 16
slots remain active with complete progress/terminal receipts, the source and
binary manifest binds, cache/nav/server/settings/geometry bind, role accounting
is complete within its declared limits, and the observation window is complete.
Even then, report it as diagnostic evidence, not final performance acceptance:
no absolute budget pass, incremental saving, candidate retention, final matrix,
32-slot lifecycle, or capacity claim follows from this one screen.

Use the result to choose exactly one next measured owner:

- source/binary mismatch or default drift -> fix provenance/release wiring;
- missing terminal/resource/accounting proof -> fix the managed collector and
  repeat only after review, without calling the first result performance data;
- complete current screen with large fixed RSS -> root may design one measured
  fixed-owner attribution task;
- complete screen with per-slot growth or progress/cadence regression -> root
  may design one matched N1/N16 owner task;
- fidelity, navigation, script, or account divergence -> behavior/integration
  owner, not memory optimization.

Do not infer the owner from RSS alone. Do not claim that this screen validates
or rejects snapshot dedup, terminal cleanup, borrowed fingerprints, or any
other parked candidate.

## Release gate and retained evidence

No live release is implied. Root should release the screen only after this
plan is reviewed, the current-source binary/build prerequisites are independently
verified, and the native client integration requirement is satisfied when the
client gitlink/source is affected. Reversible preparation already authorized by
the campaign needs no extra permission flow, but execution waits for the
integration/provenance/prerequisite review.

Retain the old Linux result and its failed/limited evidence unchanged. Retain
all current screen partial artifacts, missing-proof classifications, raw role
series, archive hashes, and exact command/environment manifests. Do not rewrite
old reports to make them current and do not merge this design into the final
campaign matrix until root performs the later evidence review.

References: `performance-finish-plan.md` §§1,3,5; `behavior-contract.md`;
`current-finish-line-gap-audit.md`; `native-platform-resource-screen-report.md`;
`borrowed-fingerprint-decision-analysis.md`; `architecture-synthesis.md`;
`windows-cohort-pair-1430-report.md`; `concord-linux-build-report.md`;
`concord-native-environment.md`; `cache-provenance-report.md`;
`process-accounting-report.md`.
