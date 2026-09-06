# Low-end performance finish line — approved plan

**Status: approved by the operator on 2026-09-06.** The three workload modes,
resource targets, measured expansion into fixed/per-bot owners, regression
margins, validation schedule and stop rule are approved for execution. Repository
review and integration rules continue to apply. Actual modest-hardware claims
remain dependent on suitable hardware; no paid infrastructure is authorized.

## Purpose

Make the application reasonably usable on low-end hardware. The current
16-core, 128 GiB Mac is a development, profiling and load-generation resource;
it is not the minimum user specification. A budget that merely accommodates
current consumption on this machine would preserve too much avoidable cost.

The historical 128 MB comparison motivates a careful accounting of the modern
application's costs. It is not a measured requirement for this Rust/V8/GPU host.
Every substantial modern overhead needs an identified owner and purpose; neither
"modern runtime" nor "shared cache" is sufficient justification for its size.

Optimize the cost of starting **one useful client** as well as additional bots.
Measure total cost at N and N+x, and the incremental cost divided by x, for both
memory and CPU. Efficient single-client operation and inexpensive growth are
parallel design requirements. Shared fixed costs, renderer residency, contention
and nonlinear work must remain visible; unlimited capacity is not a measurable
acceptance target.
Then finish the remaining compatibility and product plan. Preserve game behavior,
script APIs, deliberate errors, numeric results, action ordering, logs, and
configured rendering cadence. Do not obtain a saving by quietly disabling
features, dropping observations, reducing fidelity or weakening proofs.

## 1. Approved deployment profiles and budgets

These are **working engineering targets accepted as reasonable**, not demonstrated results
or promises of feasibility. They intentionally demand improvement over today's
roughly 600 MiB one-bot control. If a budget proves incompatible with a required
capability, bring back the measured ownership and tradeoff for a decision; do not
silently raise it to match the implementation.

| Mode | Reference hardware | Intended use |
|:---|:---|:---|
| VPS TUI | 2 CPU cores, 4 GiB RAM, Linux, no GPU | 1–16 active bots; real terminal, focused inspection, bounded logs and structured evidence |
| Panel, one renderer | 4 CPU cores, 8 GiB RAM, integrated GPU | One full-rate GPU renderer; remaining bots simulate without resident per-bot renderers |
| Panel, focused plus background renderers | 4 CPU cores, 8 GiB RAM, integrated GPU | Focused GPU renderer at full rate; remaining rendered bots use skip-paint at 1 fps, with simulation continuing at its normal rate |

Both panel modes are separate measurement and qualification cells. The same
16-bot total resource budgets below are the working finish line for each;
background renderers must fit within that budget. Measure their additional
resident CPU/GPU memory and 1 fps work explicitly. The renderer-count-fixed
incremental target applies to the one-renderer mode; in the background-renderer
mode, report the combined incremental bot-plus-renderer cost against the same
per-bot target. Record actual renderer counts and paint cadence in every result.

The local game server is outside these client deployment budgets. Measure its
resource use separately and ensure it is not starving the test. A co-located
server would need an explicit additional budget.

| Memory metric | TUI target | Panel target |
|:---|---:|---:|
| One active bot: steady median RSS | ≤256 MiB | ≤384 MiB, GPU renderer |
| One active bot: startup/transition peak RSS | ≤384 MiB | ≤512 MiB |
| 16 active bots: steady median RSS | ≤512 MiB | ≤768 MiB, either panel mode |
| 16 active bots: startup/transition peak RSS | ≤768 MiB | ≤1 GiB |
| Additional idle bot, measured finite difference | ≤8 MiB | ≤8 MiB, per mode above |
| Additional active bot, measured finite difference | ≤16 MiB | ≤16 MiB, per mode above |

RSS budgets apply to the host process; account for any new helper processes
explicitly. Record shared/private and GPU costs separately without double
counting. These are engineering budgets, not historical game figures.

Other approved gates:

- **CPU at 16 active bots:** TUI ≤0.5 average process CPU core; each panel
  mode ≤1 core on the reference deployment. CPU seconds/wall seconds is the
  measure. A core on this Mac is not equivalent to a core on an older CPU.
- **Simulation:** preserve the 20 ms client-loop scheduling target; at least
  40 iterations/slot/s in the steady active fixture, p99 start interval ≤40 ms.
  Server game ticks, client iterations and presented frames are distinct metrics.
- **Responsiveness:** p99 decoded update → script dispatch and focused input →
  visible UI acknowledgement ≤100 ms in the reference workload.
- **Rendering:** preserve requested cadence, last-FBO scene freeze and overlays.
  Measure completed/presented frames, not just UI construction time. For a
  full-rate focused view, target at least 40 completed frames/s in steady state
  and p99 frame interval ≤40 ms. Report scene transitions separately.
- **Lifecycle:** all requested slots become ready and make progress; Stop clears
  isolates and in-flight snapshots; repeated start/Stop reaches a stable
  post-Stop plateau without accumulating live owners. Nonzero RSS is not a leak.
- **Resource pressure:** no swapping/OOM or persistent backlog in the reference
  workload. Keep useful headroom for the OS, SSH/desktop and operator tools.

GPU rendering is the panel optimization target. Preserve the CPU-renderer
fallback and run affected compatibility/visual checks, but do not establish a
CPU-renderer performance budget or spend this campaign optimizing it. Preserve
its configured behavior and cadence.

### How to test lower-end hardware using this machine

Use the Mac for allocation attribution and controlled before/after experiments.
Add a Linux environment with explicit CPU/RAM limits for the SSH profile and
report quota/throttling effects. A fast CPU under a quota does not reproduce an
older processor; validate final claims on genuinely modest hardware when
available. Record OS, CPU entitlement, RAM, renderer/GPU and storage conditions.
Do not provision paid infrastructure or claim target-device validation from a
Mac-only result. Hardware availability is an explicit final-validation dependency.

## 2. What the current measurements tell us

- Corrected one-bot controls were about 595 MiB idle and 633 MiB active. The fixed
  startup/shared working set is therefore a first-class target, not background
  cost to amortize across a large fleet.
- Earlier corrected finite differences were 18.9 MiB/additional idle bot and
  31.8–38.6 MiB/additional active bot, before terminal runner cleanup. They include
  remaining harness ownership and are not pure client object sizes.
- Production navigation was already shared. Benchmark runner sharing removed
  31 duplicate packs. Two packs still exist in that harness; distinguish this
  fixed overhead from production before judging the one-bot deployment budget.
- Native widget/location snapshot vectors span four owners, totaling about
  179 MiB at 32. Terminal runner cleanup removed 46.1 MiB on one path. The latest
  clean pair showed about 63 MiB less fleet RSS, but CPU/work moved upward and
  one pair does not establish regression acceptance.
- Other native leads include client construction, scene ground data and renderer
  lighting. Their function names do not yet establish exact fields or redundancy.
- Heightmap duplication is only about 0.168 MiB of payload per client. It remains
  a candidate, but cannot solve the fixed-cost or per-bot budget by itself.

Sources: [clean pair](runner-clean-pair.md),
[controls](corrected-controls-reassessment.md),
[owners](targeted-allocation-owners.md),
[consumer audit](snapshot-consumer-audit.md).

## 3. Establish a trustworthy low-end reference

1. Finish the reverse-order short comparison for terminal runner cleanup:
   corrected then previous, same 30s warmup/120s observation/60s teardown.
   Preserve the earlier forward pair and any failures.
2. Add 16 to the harness as an additional supported scale. Keep existing modes
   and measurement meanings intact. Establish 1/16 controls for actual PTY-backed
   TUI and both GPU panel modes; retain 32 as a scaling regression case. Use
   adjacent or intermediate N/N+x points when needed to distinguish fixed costs
   from incremental growth or locate a nonlinear bottleneck. Ensure the harness
   can qualify actual background 1 fps paint before using that mode as evidence.
3. Separate harness/setup ownership from deployed host ownership. Compare a
   matched standalone client-play run when useful to isolate client fixed cost.
   Match cache, scene, memory mode, audio and renderer settings. Do not compare
   unlike defaults or subtract malloc bytes from RSS to fabricate a baseline.
4. Add the missing responsiveness measurements with bounded per-slot/window
   histograms and timestamps. Measure their overhead separately. No verbose
   per-tick logging, global allocation counting or native stack logging in clean
   CPU/latency cells.
5. Freeze source/binary provenance, cache/catalog/nav-pack hashes, settings,
   server configuration, frontend geometry and renderer policy. Record actual
   ready/active/rendering counts and per-bot progress.

**Deliverable:** a reference table showing which target is missed, by how much,
which owner or work path explains it, and which claims need target hardware.

## 4. Optimize in order of contribution to the target

### A. Fixed working set and ownership

Profile a single ready TUI client and a matched single rendered client. Attribute
large live allocations and mapped/retained memory to concrete fields and users:
shared decoded assets, navigation representation, cache residency, client tables,
world/scene structures, renderer state, script runtime and frontend infrastructure.

For each owner record: purpose, actual consumers, sharing boundary, lifetime,
allocated/live/resident evidence, and possible replacement cost. Investigate
why a shared object is large as well as whether it is shared. Investigate why
simulation retains rendering-only data before assuming it is required.

Use this attribution to choose one bounded proposal at a time. Representation,
cache residency, renderer ownership and entity-table changes were outside the
initial batch; **this plan authorizes opening those areas when measured attribution
shows they are necessary to meet the approved low-end budgets.** Each needs a
concrete design and behavior tests before implementation. No blanket rewrite.

### B. Per-bot snapshots and script traffic

Complete the already-defined candidates independently:

- Borrowed fingerprint comparison with exact equality/order/omission semantics,
  forced bank keyframes, public API compatibility and restart regressions.
- A per-isolate pool of at most two consumed buffers, returned only after
  decoding/materialization. Preserve delivery/order/skip/timeout behavior;
  verify actual allocation reuse and earlier packet lifetime.
- Further snapshot ownership work only after defining immutable publication and
  consumer contracts. Panel navigation genuinely uses widgets, side tabs and
  dialogue state; the script observation snapshot supports the whole API.
- Heightmap CoW at the existing publication boundary if its measured benefit
  justifies transition cost. Keep clients independent.

Terminal runner cleanup is already implemented; resolve its acceptance rather
than rewriting it. Preserve existing shared ownership and scalar animation-delay
improvements. Evaluate each candidate as keep, park or reject based on evidence.

### C. CPU, scheduling and rendering

Profile production-representative work without allocator-instrumentation noise.
Attribute repeated scans, copies, locks, wakeups, animation and rendering costs.
Separate work duration from scheduling delay and GPU completion. Compare N
against N+x for CPU work, resident memory, GPU memory and responsiveness in each
mode; inspect shared locks and repeated fleet-wide work when cost grows faster
than the added useful workload. Preserve the
20 ms scheduling target, packet/tick semantics and configured renderer modes.

Address a measured bottleneck in a small change. Do not redesign scheduling or
rendering merely because a mean timing changed. Test cold start, quiet state,
active scripts, map builds, focus changes and explicit Stop as distinct phases.

## 5. Experiment and acceptance rules

Use short paired runs for screening, not a long soak after every edit. Begin
with 30/120/60s, alternate order, then use a longer confirmation stage for a
candidate worth retaining. Start native attribution only after the clean phase
and keep its results out of CPU/latency comparisons.

A candidate must improve its intended metric beyond measured variation, or
establish a concrete allocation removal, while preserving behavior and meeting
resource/responsiveness gates. Approved clean-run non-regression margins are
5% CPU and 2ms p99 latency, with the absolute budgets also satisfied. Allocation removal is not automatically an RSS saving.

If noise exceeds the margin, classify the result as inconclusive. After paired
screens and one longer confirmation stage, investigate a named confounder or
park the candidate; do not rerun indefinitely looking for a favorable sample.
Any resource tradeoff outside the approved margins needs an explicit decision.

Run affected crate tests with required features and client integration tests
separately when client code changes. Exercise slow/malformed scripts, guardian
hold, pause/resume, restart, bank publication, scene changes and retained packet
lifetimes. Preserve deterministic packet/action traces where fixtures permit;
use per-bot progress qualification for stochastic gameplay.

## 6. Final validation and finish line

Approved validation schedule, replacing blanket matrix reruns after
individual edits:

- Final 1/16 idle and active profiles in all three modes: three fresh runs each,
  120s warmup and 600s observation, after binaries/settings settle.
- 32 active in all three modes: repeated qualified confirmation of scaling and
  resource cost, with a one-hour lifecycle run per mode. Inspect the last
  three post-Stop windows against the same-state repeat-noise band and any growth
  in live owners.
- 128: a gated capacity check after the low-end targets pass. Perform full
  repeated qualification before calling 128 supported; unavailable or incomplete
  results remain blocked evidence. The operator expects this Mac may support
  128 bots with background skip-paint; test that as a capacity hypothesis, with
  one focused full-rate renderer and the rest at 1 fps, rather than assuming it.
  Do not make a large Mac load test a substitute
  for meeting the low-end profile.
- Separate focus/watch, overlays, minimap freeze and CPU-renderer visual checks.
  Validate actual terminal interaction/resize/error visibility on the TUI.
- Final measured target-hardware/resource-limited results, including startup,
  transitions and responsiveness; disclose the limits of VM-only validation.

Finish with a target/result/evidence/pass-or-unresolved table and the required
whole-branch **grok-4.6** review. An unavailable reviewer is not approval. Follow
repository review and integration rules; do not duplicate Git policy here.

**Stop optimizing when the approved low-end profiles, behavior/lifecycle gates
and final review pass, with all listed candidates resolved.** Then resume the
remaining JavaScript compatibility plan and exceptions through Rust host APIs,
and sequence the existing TUI/product plan from its current state. Do not keep
hunting smaller allocations merely because optimization remains possible.
If a required target is missed, present a specific blocker/tradeoff; do not call
that completion or silently substitute this Mac's capacity for the target.

## 7. Larger fleet and renderer goals

Keep both N and N+x inexpensive even after the low-end targets are met; those
targets bound this campaign, rather than define an acceptable scaling defect.
Do not introduce avoidable per-bot duplication just because a small fleet fits.
Further optimization depth after the finish line is a separate prioritization
decision based on measured benefit and implementation cost.

Design for roughly 1,000 resource-dependent SSH bots and 50 renderers without
making both immediate prerequisites for the low-end finish line. At 16 MiB per
additional active bot, the incremental memory alone at 1,000 bots is about 15.6
GiB; fixed costs, headroom, CPU and server capacity still need measurement.
This arithmetic is a planning estimate, not a validated capacity claim.

Before those milestones, specify the VPS CPU/RAM/network entitlement and the
renderer count, resolution, refresh rate and backend. Fifty resident renderers
and fifty continuously drawing renderers are different workloads. Do not silently
substitute thumbnails or reduced refresh rates to meet the latter goal.

## Operator approval and current execution

Approved on 2026-09-06, including both GPU panel modes within the same total and
incremental budgets, evidence-driven expansion into fixed client/cache/nav/
renderer costs, and the stop rule and measurement schedule above.

The operator identified interest in running roughly 1,000 bots on a 16 GB VPS.
This is a later capacity ambition, not an accepted capacity claim or a revision
to the current finish line. At the 16 MiB incremental ceiling, 999 additional
bots alone require about 15.6 GiB; the first bot, OS, runtime, memory headroom
and CPU/network entitlement still need room. Record whether a provider means
16 GB decimal or 16 GiB. Continue reducing useful N and N+x costs where justified;
do not infer that passing the small-fleet budgets makes this deployment feasible.

First execution step: reverse-order short runner-cleanup comparison (corrected
binary followed by the previous binary), retaining the forward pair and failures.
Then establish the three-mode low-end reference and proceed by measured ownership.
