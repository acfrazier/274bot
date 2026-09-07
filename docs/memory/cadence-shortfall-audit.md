# N1 cadence shortfall audit (~36 loops/s vs ≥40)

**Scope:** read-only investigation. No code, build, test, live, or server changes.
**Branch:** `codex/memory-diagnostics` (not `main`).
**Owned file:** this report only.

## Targets (approved plan)

From `docs/memory/performance-finish-plan.md` (Simulation gate):

- Preserve the **20 ms** client-loop scheduling target.
- At least **40 iterations/slot/s** in the steady active fixture.
- **p99** start-to-start interval **≤ 40 ms**.

Server game ticks, client iterations, and presented frames remain distinct metrics.

## Frozen candidate evidence (repeated shortfall)

| Item | Value |
|:---|:---|
| Reviewed pipeline | `docs/memory/diagnostics/managed-pipeline-qualification-20260907T062612Z/` |
| `bound-side.json` | `qualified: true`, managed resources available, role candidate |
| Run dir | `docs/memory/diagnostics/20260907T062620Z_tui_n1_active` |
| Match | TUI, N=1, workload active, render_policy none, `scheduling_profile: true` |
| Host | macOS 15.7.9 arm64, unrestricted development Mac |
| Observation window | elapsed ≈ 53.767 s → 172.755 s (wall span 118.988 s; adapter `measured_duration_ms` 119037) |
| Workload tick rate | `client_ticks_per_slot_s` **36.130** |
| Per-slot scheduling | `observed_iterations_per_s` **36.123**, `target_verdict: miss` (≥40) |
| p99 start interval | bucket **29–30 ms** upper bound 30 ms (`target_interval_ms` 40 → **meet** on p99 alone) |
| Prior failed pipeline | `managed-pipeline-qualification-20260907T061239Z` → run `20260907T061512Z_tui_n1_active`: **36.226** ticks/s, **36.224** observed iterations/s, same p99 29–30 ms shape |

Both recent N1 TUI active cells land near **36.1–36.2** loops/s. The shortfall is stable across the failed and the qualified pipeline cells, not a one-off contamination spike in the reviewed candidate.

## What drives the loop (source)

### Production TUI / host-play path (this cell)

`host-play` slot threads call `Host::run_client` (`crates/host-play/src/lib.rs` ~3766).

Busy / frame-cadence path in `crates/host/src/lib.rs`:

| Lines | Behavior |
|:---|:---|
| 51–52 | `FRAME_MS = Duration::from_millis(20)` — host frame budget |
| 257–286 | If `frame_cadence \|\| busy`: mark tick `start`, run `client_tick`, then leftover sleep |
| 273–275 | Comment: Java GameShell sleeps the leftover of 20 ms **after** work; fixed pre-tick sleep made period = 20 ms + work |
| 276–280 | `work = start.elapsed()`; `rest = FRAME_MS.checked_sub(work)`; `thread::sleep(rest)` if Some |
| 278–284 | Scheduling profile (opt-in): measure actual sleep around that call; `record(start, draw, work, requested=rest, slept, budget=FRAME_MS)` |
| 292+ | Idle path: `parked()` then park 200 ms / 600 ms / 1 s — **not** the active fixture path when `busy` keeps the 20 ms loop |

Active fixture keeps the 20 ms body via observe-hook **busy** (and/or input cadence), not via `Client::run`.

**Requested period arithmetic (host):**  
`requested_sleep = max(0, 20 ms − work)`.  
**Intended start-to-start period if sleep is exact:** ≈ 20 ms (work + leftover), independent of work duration while work ≤ 20 ms.  
If work > 20 ms, sleep is skipped (`checked_sub` → None).

Cadence measurement (`crates/host/src/cadence.rs`) does **not** change sleep budget, sleep calls, or loop structure (module docs lines 36–37; `per-slot-scheduling-report.md`).

### Alternate path (not this TUI host cell)

`Client::run` / `GameShell` (`vendor/fr-client-rust/.../game_shell.rs`, `client.rs` ~10457+):

- `deltime` default **20**, `mindel` default **1**.
- Sleeps `Duration::from_millis(delta)` **before** mainloop catch-up, with Java-style ratio/otim bookkeeping (`frame_bookkeeping` ~222–258).
- Host multi-slot TUI does **not** use this driver for the measured N1 cell; do not attribute the shortfall to GameShell `delta` without a cell that runs `Client::run`.

## Observation-window counter deltas (candidate run)

Source: `samples.jsonl` `scheduling_slots[0]` rows at elapsed **53.766961375** (cycle_n 1550) and **172.754761083** (cycle_n 5850). Same generation 2, drawing false (TUI / no resident renderer).

| Metric | Delta / mean |
|:---|---:|
| `cycle_n` / `interval_n` | **4300** / **4300** |
| park / mode_break / anchor_miss delta | **0** / **0** / **0** |
| `work_overrun_n` delta | **0** |
| mean **work** | **0.281 ms**/cycle |
| mean **requested_sleep** | **19.719 ms**/cycle |
| mean **actual_sleep** | **27.401 ms**/cycle |
| mean **sleep excess** (actual − requested) | **≈ 7.682 ms**/cycle |
| mean **start-to-start interval** | **27.683 ms** |
| loops/s from interval Δ / elapsed Δ | **36.138** |
| equivalent period | **27.672 ms** |

Identity check: mean work + mean actual sleep ≈ 0.281 + 27.401 = **27.682 ms** ≈ mean interval. The period is almost entirely **work + measured sleep**, not hidden post-record gap.

### Sleep excess histogram (bounds 1, 2, 5, 10, 20 ms + overflow)

Observe delta `sleep_excess_buckets`: **[175, 120, 575, 3426, 4, 0]** over 4300 non-zero-request sleeps.

| Excess over requested | Count | Share |
|:---|---:|---:|
| ≤ 1 ms | 175 | 4.1% |
| (1, 2] ms | 120 | 2.8% |
| (2, 5] ms | 575 | 13.4% |
| (5, 10] ms | 3426 | **79.7%** |
| (10, 20] ms | 4 | 0.1% |
| > 20 ms | 0 | 0% |

Process-wide non-drawing group in `bound-side.json` matches the same excess shape (`sleep_excess_buckets_delta` / `interval_excess_buckets_delta`).

### Absolute interval histogram (1 ms steps 18–42)

Observe delta `interval_buckets` (bounds include …, 19, **20**, 21, …, **30**, 31, …):

- Counts in all buckets with upper bound **≤ 20 ms**: **0**
- Mass sits in 21–30 ms; single largest bucket **≤ 30 ms**: **2252 / 4300 (52.4%)**
- Only **4** samples in ≤ 31 ms beyond the ≤30 bucket; nothing in higher finite buckets in this window

Conservative per-slot `interval_p99_upper_bound_ms`: **30** (meets ≤40 ms p99; **fails** mean ≥40 Hz).

Prior failed cell process-wide excess was the same pattern: sleep/interval excess p99 band **5–10 ms** over the 20 ms baseline (`analysis.json` for `20260907T061239Z`).

## Requested vs actual (distinction)

| Concept | Evidence |
|:---|:---|
| **Requested loop period** | Host designs **20 ms** from tick start via leftover `thread::sleep(FRAME_MS − work)` |
| **Requested sleep duration** | ~**19.72 ms** mean (20 ms minus ~0.28 ms work) |
| **Actual sleep duration** | ~**27.40 ms** mean |
| **Actual loop period** | ~**27.68 ms** mean → ~**36.1** loops/s |
| **Work path** | Sub-millisecond; **zero** work-over-budget events in the window |
| **Park / mode breaks** | None in the observe interval set (steady same-drawing pairing) |

So the shortfall is **not** explained by mainloop/work overrunning 20 ms, idle parking, or drawing-mode flips in this cell.

## What the evidence supports vs what it does not

**Supported by source + raw counters:**

1. Production scheduling for this TUI N1 cell is **relative leftover sleep** after work against a fixed **20 ms** budget (`host/src/lib.rs` 273–280).
2. The sleep call is **`std::thread::sleep`** on the leftover `Duration` (same lines).
3. Instrumentation records **requested** vs **actual** sleep; actual systematically exceeds requested by a mean ~**7.7 ms**, with **~80%** of excess samples in the **(5, 10] ms** bucket.
4. Start-to-start intervals track work + actual sleep; rate ~**36.1/s** is the reciprocal of that period.
5. p99 start interval ~**30 ms** can pass the **≤40 ms** gate while mean rate still **misses ≥40/s** — both gates are required by the plan; only the rate gate fails here.
6. The same ~36 loops/s band appears on the prior failed N1 pipeline cell.

**Not established as fact (do not treat as proven root cause):**

- A specific **macOS timer quantum**, power-management coalescing policy, or kernel HZ value (no OS timer trace, `dtrace`/`latency`, or bare-nanosleep cell in these artifacts).
- That scheduling-profile Instant bookkeeping **causes** the 7.7 ms gap (docs state measurement does not change sleep; overhead of the profile itself is **unmeasured** here; even generous bookkeeping would not reclassify zero work-overruns into a 7–8 ms sleep excess concentrated in OS-style buckets).
- That GameShell `delta` / `mindel` arithmetic is on the hot path for this cell.
- That Linux or modest-hardware deployments would show the same shortfall (host conditions: unrestricted Mac; `target_device_validation: false`).

Prior campaign note (`docs/memory/scheduling-profile.md`): partial n32 panel profile already reported mean sleep excess ~2.7–3.6 ms and called **late wakeups** a cadence limiter without naming a kernel cause. This N1 TUI cell shows a **larger** mean excess (~7.7 ms) with the same “work cheap, sleep long” structure.

## Gate split (important)

| Gate | Result on candidate N1 |
|:---|:---|
| ≥ 40 iterations/slot/s | **Miss** (~36.1) |
| p99 start interval ≤ 40 ms | **Meet** (upper bound 30 ms) |
| Work ≤ 20 ms (overrun count) | **Meet** (0 overruns) |

Fixing or accepting the rate miss is separate from p99 headroom already present.

## Smallest next diagnostic (do not implement here)

**Preferred smallest diagnostic cell (no production behavior change):**

1. **Bare `thread::sleep` probe** on the same Mac, quiet host policy matching managed cells, no client/server:
   - Loop N times: request sleep **19 ms** and **20 ms**; record requested vs `Instant` actual; emit the same 6-bucket excess schema as `cadence.rs` (`EXCESS_BOUNDS_MS` 1,2,5,10,20).
   - Compare mean excess and bucket shape to this audit’s **[175,120,575,3426,4,0]** pattern.
   - If bare sleep reproduces ~5–10 ms excess and ~36 Hz effective period for a 20 ms request, the limiter is **outside** client/host work (OS/runtime sleep), and game-path optimization will not recover 40 Hz under the current sleep API.
   - If bare sleep is near-exact and only the host loop shows excess, instrument **pre-sleep / post-sleep / post-record** splits inside `run_client` (diagnostic build only) at `crates/host/src/lib.rs:278–284`.

2. Optional second diagnostic (still non-shipping): one-shot **absolute deadline** sleep behind an env flag (`sleep_until = start + FRAME_MS`) vs current relative leftover, same N1 fixture, compare `requested_sleep` / `actual_sleep` / interval means. Measurement-only if the flag defaults off and tick work path is unchanged.

Do **not** use JSONL row Δt as the interval window; keep counter deltas + endpoint rules from `per-slot-scheduling-report.md`.

## Smallest safe correction candidate (propose only — not implemented)

If the bare-sleep probe confirms OS/runtime overshoot on relative sleeps:

| Proposal | Detail |
|:---|:---|
| **What** | Keep **20 ms** period intent and leftover-after-work policy; replace **relative** `thread::sleep(rest)` with **sleep until tick-start + FRAME_MS** (absolute deadline), still skipping sleep when `work ≥ FRAME_MS`. |
| **Where** | `crates/host/src/lib.rs` **273–284** (only production sleep site for this path). Optionally mirror later only if a cell uses `Client::run` / GameShell sleep (`game_shell.rs` 182–185, `client.rs` 10491–10493) and that path is shown short. |
| **Why** | Relative leftover sleep **re-bases** every cycle on “sleep N ms from now,” so each wakeup overshoot **adds** to the period. Absolute deadline from `start` absorbs overshoot into the next wait when the OS wakes early/late within the period (platform APIs permitting). |
| **Preserve** | Tick body, observe/script order, park bounds, `FRAME_MS == 20 ms`, no invented tick-end opcode, no busy-spin default. |
| **Risks** | Platform-specific absolute wait (`clock_nanosleep`, mach continuum, etc.); must not change behavior when work overruns; need Linux + macOS evidence; CPU spin-tail hybrids need explicit CPU-budget review on the low-end plan. |
| **Not proposed without new evidence** | Shortening `FRAME_MS`, raising script load, disabling profiles to “pass” gates, or treating p99-only as full simulation acceptance. |

If bare sleep does **not** overshoot, do not land an absolute-deadline change; chase host-loop time outside `client_tick` first.

## Files / lines reference

| Path | Role |
|:---|:---|
| `crates/host/src/lib.rs:51–52, 257–286, 292–301, 640–645` | `FRAME_MS`, leftover sleep, park path, `frame_cadence` |
| `crates/host/src/cadence.rs` | requested/actual sleep, excess + absolute interval histograms; no policy change |
| `crates/host-play/src/lib.rs` (~3766) | `Host::run_client` entry for slots |
| `crates/host-play/src/memory.rs` (~671, 1089–1161) | `BOT_SCHEDULING_PROFILE` enable + JSON emit |
| `vendor/fr-client-rust/.../game_shell.rs:51–56, 168–185, 222–258` | Alternate GameShell 20 ms machine (not this cell’s driver) |
| `docs/memory/performance-finish-plan.md:76–78` | 20 ms / ≥40 Hz / p99 ≤40 ms |
| `docs/memory/scheduling-profile.md` | Earlier late-wakeup note |
| `docs/memory/per-slot-scheduling-report.md` | Endpoint / flush semantics |
| `docs/memory/diagnostics/20260907T062620Z_tui_n1_active/samples.jsonl` | Raw counters |
| `docs/memory/diagnostics/managed-pipeline-qualification-20260907T062612Z/bound-side.json` | Bound qualification + scheduling gate |

## Bottom line

The repeated N1 **~36 loops/s** result is the reciprocal of a **~27.7 ms** mean start-to-start period. The host **requests** ~**19.7 ms** sleep after **~0.28 ms** work toward a **20 ms** budget; **actual** sleep averages **~27.4 ms**, with excess dominated by the **(5, 10] ms** bucket and **no** work overruns. That is sufficient evidence that **leftover relative `thread::sleep` overshoot**, not simulation work cost, accounts for the rate gate miss on these Mac TUI cells. It is **not** sufficient to name a specific macOS kernel timer mechanism without a bare-sleep or OS-trace diagnostic. Smallest next step: bare sleep excess cell; smallest behavior-preserving correction to evaluate after that: absolute deadline sleep at `host/src/lib.rs` 273–284.
