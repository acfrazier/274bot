# Linux native failure-attribution evidence report

Status: **failed** diagnostic cell. Not performance acceptance. Not
qualification. No timeout change. No rerun.

Independent read-only diagnosis of the changed-instrument native Linux N=16
run archived under `diagnostics/linux-failure-attribution-20260908/`. Primary
root observation:
`docs/memory/linux-failure-attribution-native-observation.json`.

## Evidence identity

| Item | Value |
| --- | --- |
| Archive tar SHA-256 | `40444f4b6af3ad6bb82b13384dd7ed52e92485e09b6ece3ac3ed247d2efb2750` |
| Manifest files | 25 (all length+SHA verified against expanded tree) |
| Archive status | `failed`, `performance_acceptance: false` |
| Binary SHA-256 | `e451c0164c96142c4da28f9b6dcfe1944823b0c4c3282c54e06804832784fd0a` |
| Build host commit | `57d52293f803a9e382d7ecc3d398d04212cd7a65` (`codex/linux-failure-attribution`) |
| Instrumentation scope | reviewed transplant through `e1cdfbc` (verification notes) |
| Client commit | `3456edc8dabf7b25ada78110ffa56327af9f67a4` |
| Host sources aggregate | `2ce61b110fe97fb22e8a9dd0ce7b21307c7bfba53c86df1a551df59035a78f5f` (852 files, pre=post) |
| Runtime tools root | `workspace-b4b686f` (`host_commit` `b4b686fd…` on metadata; tools only) |
| Controller | `d4a3ed5` (exclusive live-spec bind); prepare path also retains `2f5f3f6` |
| Features | `memory-profile-no-alloc`, System allocator, `failure_capture: true`, `diagnostic_sidecar: false` |
| CLI | `tui 16 active --sustain --warmup 30 --observe 120 --no-diagnostics --failure-capture` |

Source read for this report (isolated tree, not edited):

- `/Users/acfrazier/experiments/274bot/.worktrees/linux-failure-attribution/crates/script/src/load.rs`
  SHA-256 `a3b282226abf7a6386d0ad5e1e5dd5fb2f2ef16514a3551d6796b54fe4bcf701`
- `…/memory_profile.rs`
  SHA-256 `ecfe497fa8e9076002b25c2ece324a22ab5a0982dc84d9ba61b7317019dafa39`

These match the candidate verification document on that branch.

Optional local verifier:
`docs/memory/linux-attribution-verify-archive.sh` (hash/manifest only).

## Keep pre-launch failure distinct

| Event | What happened | Relation to failed cell |
| --- | --- | --- |
| Prepare `2f5f3f6` | Native prepare-only **passed** | Preconditions only |
| First launch attempt (historical) | Stopped **pre-frontend** on stale ambient SSH parent **pid 154234** | **Not** the failed cell; no gameplay failure |
| Corrective `d4a3ed5` | Exclusive `live-spec.json` binds **current** parent; refuses duplicate live spec; does not adopt prepared session PID as live helper | Enables the actual cell |
| Actual cell | Frontend pid **154275** launched once; launcher **154266**, collector **154267**, controller **154263**, ssh **154262**, server **152004** | **This** failure |

Preflight for the actual cell records verified provenance, server identity
`linux_proc_start_ticks:241214967`, and ambient ssh
`linux_proc_start_ticks:244654117`. Completion record:
`qualified: false`, exit 1.

## Actual failure boundary

From `raw-run-01/samples.qualification.jsonl` and the root observation JSON
(identical attribution payload):

- `elapsed_s`: **16.222461943**
- `phase` / `record`: **failure-boundary** (still in **seed**; never reached
  warmup/observe qualification)
- Failure string: `live25aa3_15: tick 19: Unknown error`
- Ordinal **15**, name **live25aa3_15**
- Slot runtime at capture: `dispatched: 2`, `last_completed_tick: 20`,
  `in_flight: null`, paint title `ThievingBot — starting`
- Client: `ingame: true`, `scene_state: 2`, tile `(2661, 3306, 0)`
- Only this slot carries a non-null `error` among 16 slots at the boundary
  (others Running/Idle without attribution)

`failure_attribution` (first retained record):

```json
{
  "tick": 19,
  "call_path": "sync",
  "error_variant": "Runtime",
  "error_debug": "Runtime(\"Unknown error\")",
  "terminating_before_cancel": true,
  "interrupt_id": 1
}
```

Harness exit: frontend/metadata exit 1; managed receipt
`failed_or_unavailable` with binding errors
`unexpected qualification phase`, `missing observation boundary`,
`frontend_failed_or_incomplete`, `launcher_failed`. Wall time frontend
~17.9 s (`started_unix`→`ended_unix`). All 16 periodic
`samples.jsonl` rows remain `phase: "seed"`.

## Source semantics (what the flags mean)

### Budget (unchanged)

`LoadIsolate` keeps `SLOW_TICK = 50ms` and `RUNTIME_TIMEOUT = 50ms`. No
threshold raise in this candidate.

### Who allocates `interrupt_id`

Only the **`on_game_tick` slow-tick path** calls
`counters.next_interrupt_id(tick)` immediately before
`terminate_execution()`. That path fires when a previous `in_flight` tick is
still outstanding and `started.elapsed() > SLOW_TICK`.

`pause`, stop, and other terminate sites call `terminate_execution` **without**
allocating an interrupt identity. Therefore `interrupt_id: 1` is evidence the
host slow-tick interrupt path ran at least once for a matched tick, not merely
that V8 termination was observed somehow.

### Publication path

With `BOT_MEMORY_FAILURE_CAPTURE=1` / failure-capture harness mode, stop-reason
capture is enabled at isolate spawn. On rustyscript `Err` at the tick call
boundary, before `cancel_terminate_execution`, the isolate records
`FailureAttribution` into the per-isolate `Counters` Arc. Host
`memory_progress` republishes it when capture is enabled. First write wins;
later failures on the same counters are ignored. Debug text is capped at 1024
Unicode scalars.

### `terminating_before_cancel` is a **combined** flag

```text
terminating_before_cancel = is_execution_terminating()
                         || interrupt_id_for_tick(n).is_some()
```

Comments in `load.rs` state rustyscript/V8 may clear the observable terminating
flag while converting an interrupt into `Runtime("Unknown error")`, so a
tick-matched host interrupt is treated as bounded pre-cancel evidence.

Implications:

- **`terminating_before_cancel: true` does not by itself prove** the raw V8
  `is_execution_terminating()` bit was still set at observation time.
- With **`interrupt_id: 1` present and tick-matched**, the combined flag is
  expected true even if the raw V8 flag had already cleared.
- The separate fields still allow: interrupt identity present vs absent;
  call path `sync` vs `async-parked`; variant/debug text.

### Tick match and Relaxed atomics

`next_interrupt_id(tick)` stores `interrupt_tick = tick` and bumps
`interrupt_id` with **`Ordering::Relaxed`**. `interrupt_id_for_tick(n)` returns
the latest id only when `interrupt_tick == n`.

Limits:

- Identity is **latest** id for that tick slot, not a full interrupt history.
- Relaxed loads/stores do not provide an explicit acquire/release handshake;
  the design assumes host writer + isolate reader ordering around the same
  tick’s terminate/return path. Do not over-read concurrency guarantees.
- `record_failure_attribution` keeps **only the first** failure; later ticks’
  errors on that isolate are not retained in the progress payload.

### What this run’s attribution supports

Supported for **this instrumented cell only**:

1. Failure occurred on the **synchronous** `__rs_tick` path (`call_path: sync`),
   not `async-parked`.
2. rustyscript surfaced **`Runtime("Unknown error")`** at the isolate call
   boundary for **tick 19**.
3. Host allocated **interrupt identity 1** for that tick via the **slow-tick**
   `on_game_tick` path before cancel — i.e. this failure is attributed to
   **host-issued slow-tick interruption**, not an ordinary thrown JS error
   without host interrupt identity.
4. Client was **in-game, scene_state 2** at the boundary.

Not supported:

- **Why** tick 19 exceeded the 50 ms host in-flight budget (JS work, GC,
  scheduling, lock contention, CPU interference, etc.).
- That raw V8 terminating was still observable (combined OR).
- Retroactive proof of **older uninstrumented** Linux failures (e.g. historical
  slot-12 `Unknown error`).
- That every slow tick always becomes this error shape.
- Performance, RSS, or N=16 support claims.

## Process CPU / memory / pressure / timing bounds

### Host conditions at cell start

- Linux x86_64, Ubuntu 24.04, **2** logical CPUs
- `MemTotal` ~1.92 GiB; `MemAvailable` ~920 MiB; **no swap**
- Memory guard: `MemAvailable >= 128 MiB`
- PSI memory at host-conditions snapshot: some/full avg10 **0.00**
- Nightly timer disabled; purpose marked diagnostic / no performance acceptance

### Managed process accounting (roles only)

`process_accounting.jsonl`: 48 samples, 0.5 s interval, stop-controlled
(~23.5 s span). Roles: collector, controller, game_server, launcher,
ssh_session. **Frontend pid 154275 is not a role** — descendants of launcher
are explicitly **not measured**. Do not treat launcher RSS/CPU as the TUI
process.

Around the failure window (elapsed 15.0–17.0 s):

| elapsed_s | launcher RSS | launcher cores_delta | PSI mem some avg10 | game_server RSS |
| --- | --- | --- | --- | --- |
| 15.0–17.0 | ~19.6 MiB | **0.0** | ~0.08–0.09 | ~626 MiB |

Broader sampler bounds:

- Launcher (python wrapper) RSS min/max ~18.5–110.6 MiB; peak cores ~1.02 early
  (~1.5 s), not at the failure instant
- PSI memory some avg10 max **1.69** (later, ~19 s / teardown region), full max
  **1.44** — elevated **after** the failure boundary, not a smoking gun at
  t≈16.2 s
- Game server identity stable `241214967`; launcher identity stable
  `244655210` across samples
- Sampler summary: `controlled_stop`, `grid_complete: false`, not full
  observation coverage

### Frontend self-metrics (`samples.jsonl`)

These are the TUI process counters (pid 154275), still all **seed** phase:

| elapsed_s | resident_bytes | peak_resident | CPU user+sys (s) | script_tick_count | script_tick_max_ns | v8_live | active/ready |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 14.75 | 370.2 MiB | 369.9 MiB | 6.56 | 6 | 4.76 ms | 1 | 1/14 |
| **15.98** (last) | **460.6 MiB** | 460.4 MiB | **8.06** | 22 | **5.90 ms** | **12** | **12/14** |

Notes:

- Last periodic sample is **~0.24 s before** the failure-boundary timestamp.
  Aggregate `script_tick_max_ns` **5.9 ms ≪ 50 ms** on completed ticks
  published before the failure; the interrupting tick **is not present** in
  periodic samples and cannot be timed from them.
- Between 14.75 s and 15.98 s: isolates jump 1→12 live, RSS +90 MiB, script
  tick count 6→22 — concurrent script start wave during seed, on a **2-CPU /
  ~2 GiB / no-swap** host already holding ~626 MiB game server RSS.
- This bounds **context** (memory/CPU tight, multi-isolate seed ramp) but does
  **not** identify the single-tick root cause or justify changing SLOW_TICK.
- `client_tick_max_ns` reaches ~316 ms in seed samples; that is client mainloop
  accounting, not the script isolate slow-tick interrupt identity.

### Fixture / qualification state

- Fixture identity tuple frozen in build-manifest (nav/catalog/server/cache/
  server_start_identity); cache verified before launch and after completion.
- Qualification lines: **1** (the failure-boundary only).
- **Not qualified.** Warmup 30 s / observe 120 s **never entered**.
- `performance_acceptance` false everywhere (observation, manifest, receipt,
  completion).

## Interpretation (strict)

The instrumented native cell **failed in seed** when slot `live25aa3_15` hit
`tick 19: Unknown error`. Failure-capture attributes that error to a
**sync-path** rustyscript `Runtime("Unknown error")` with a **tick-matched host
slow-tick interrupt id (1)** and combined `terminating_before_cancel: true`.
That is sufficient to classify **this** failure as **host slow-tick
interruption → Unknown error**, under unchanged 50 ms budgets.

It is **not** sufficient to:

- explain the mechanical reason the tick was still in flight past 50 ms,
- treat PSI/CPU/RSS as root cause,
- promote the run to qualified/passed,
- or map the result onto older uninstrumented failures.

Preserve **failed** status.

## One bounded next diagnostic (only if pursued later)

**Proposal:** one follow-up **failure-capture** cell (same binary/budgets/N, no
timeout raise) that additionally records, per interrupt, the host-side
`in_flight` elapsed at `terminate_execution` and the isolate-side
`start.elapsed()` at attribution time into the existing progress payload (or a
single bounded counter field pair). Goal: close the gap that periodic samples
missed the interrupting tick’s duration, without claiming performance
acceptance.

Out of scope for that proposal: threshold changes, blind reruns for green,
matched RSS matrices, or rewriting old archives.

## Non-actions taken by this review

No source edits, no archive mutation, no native/VM/Docker/network/process
actions, no implementation, no timeout raise, no blind rerun.
