# N16 stop diagnostic (bounded functional cell)

Task `t_27a3363a`. **Functional diagnostic only** — not a benchmark, not RSS/CPU/p99
acceptance, not an N=16 support claim.

## Verdict

| Result | Detail |
|:---|:---|
| Authorized cell | `docs/memory/diagnostics/20260907T012918Z_tui_n16_active` |
| Process exit | **`-15` (SIGTERM)** — agent foreground tool timeout at ~420 s, **not** a script-requested stop |
| Workload qualification | **`qualified=false`** — process interrupted before clean harness completion |
| Original N16 scriptStop (`20260907T005032Z`) | **Not reproduced** in this cell |
| Cached `stop_reason` | **Not observed** (no script stop; field absent from qualification slots) |
| Full 300 s observe retained? | **Yes** — `observe-start` and `observe-end` boundaries present; observe samples cover ~300 s |
| Teardown | Started; last sample `phase=teardown` `elapsed_s=418.995756542`; killed mid-teardown |
| N=16 supported? | **No** — single interrupted functional diagnostic must not mark support |
| Performance claim | **None** — diagnostics sidecar on; resource numbers excluded from acceptance |

Orch correction (same card): do **not** automatic-rerun after tool timeout. A worker
background relaunch `20260907T013727Z` was killed immediately and is **not** an
authorized cell (receipt only).

## Scope and non-claims

- One bounded PTY TUI N=16 active sustained Thiever cell with **diagnostic sidecar ON**
  (`BOT_MEMORY_DIAGNOSTICS=1` via `run_diagnostic.py` default; no `--no-diagnostics`).
- Stop-reason capture is the reviewed opt-in path (`341a5b8`); this run exercises it only
  if a script stop occurs.
- No Rust/client/JS/catalog/fixture/timeout/policy edits in this card.
- No server start/stop/reset; VNC `memory-ref-amd64-view` and `fr-vault-chroma` left running.
- No stack logging, counting allocator, or `BOT_DEBUG`.
- CPU/RSS/p99/scheduling/responsiveness are **out of acceptance** (sidecar may perturb).

## Commands (exact)

### Preflight

- Branch: `codex/memory-diagnostics` (verified).
- HEAD: `944fb60923b8a2ce533c9ac676396f9bdc929437`
- Client: `451759f2a7df9c57895657d5b8d506172860cee1`
- Crate delta vs freeze base `6345fbc`: **only** reviewed stop-reason capture commit
  `341a5b8` (`load.rs`, `memory_profile.rs`, `load_isolate.rs`). Metrics docs commit
  `944fb60` is docs-only relative to crates.
- Host dirty tree at build: empty (`host_diff_sha256` =
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`).
- Host sources sha256: `9fee9ab6804e283293b2ad2797eff22f170fdd72d8709beb253601123fb587a8`
- Client sources sha256: `27246deba654abac980395b0bbf9ad077d8d3feb66992b4d659512e0630e4a25`
- Server: PID **4719** listen **43594** (existing node engine; not restarted).
- Helpers retained: `memory-ref-amd64-view` `:6080`, `fr-vault-chroma` `:8765`.
- Quiet check before build/run: no competing `cargo`/`rustc`/`tui-play`/`run_diagnostic`.

### Diagnostic candidate build

```bash
cd /Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f
export CARGO_TARGET_DIR=/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f/target-shared-nav-candidate
cargo build --release --locked -p tui --bin tui-play --features memory-profile-no-alloc
# exit 0 — log: docs/memory/diagnostics/_n16_stop_build.log
```

Binary copied (not overwriting matched freeze):

`docs/memory/diagnostics/n16-stop-diagnostic-build-20260907T012907Z/tui-play-n16-stop-diagnostic-system-20260907T012907Z`

| Field | Value |
|:---|:---|
| SHA-256 | `13a1221c3f0d6f03ca3884894b36d6c995c3c7b3e7bcf70567aa50d8223557aa` |
| Size | 80999632 |
| Features | `memory-profile-no-alloc` (system allocator) |
| `nm` non-v8 `CountingAllocator` | 0 lines |
| Differs from frozen candidate `769a9367…` | yes (expected: includes `341a5b8`) |
| Frozen control/candidate SHA verified unchanged | yes (see manifest) |

### Authorized live cell

```bash
python3 docs/memory/run_diagnostic.py tui 16 active --sustain \
  --binary docs/memory/diagnostics/n16-stop-diagnostic-build-20260907T012907Z/tui-play-n16-stop-diagnostic-system-20260907T012907Z \
  --warmup 30 --observe 300
# harness wall ~419.87 s; metadata exit_code=-15 (SIGTERM from agent 420 s foreground limit)
```

Flags: **no** `--no-diagnostics`, **no** `--stack-logging*`, **no** `--debug`, **no**
profile flags. Terminal PTY 120×40. Throwaway harness accounts (`live11e09_*`).

## Window and interruption class

| Phase | Samples | `elapsed_s` min–max |
|:---|---:|:---|
| seed | 73 | 0.054 – 73.236 |
| warmup | 29 | 74.259 – 102.962 |
| observe | 291 | 103.994 – 402.656 |
| teardown | 16 | 403.661 – **418.996** |

| Boundary | `elapsed_s` |
|:---|---:|
| qualification `observe-start` | 103.357 |
| qualification `observe-end` | 403.372 |
| Observe span (end − start) | **≈ 300.014 s** (full requested observe retained) |
| metadata `started_unix` → `ended_unix` | **419.870 s** |
| Tool limit | ~420 s foreground → **SIGTERM** |

**Classification:** `tool_timeout_sigterm` during teardown. Distinct from original
failure class `script requested stop on tick 289; isolate stopping`.

Even though observe-end exists and all slots were `Running`, **final qualification is
false** because the process did not complete teardown/exit 0 under harness control.

## Slot progress (qualification; paint lines)

`stop_reason` key: **absent** on every slot top-level and `runtime` object at both
boundaries (not the same as captured `null` after a stop). No script stop ⇒ capture
path did not run. Do **not** substitute paint text for `stop_reason`.

### observe-start (`elapsed_s=103.357`)

All 16 `Running`, `error=null`. Steals **5–14** (sum **144**). Food 3–4. Bank trips 0.
Example titles: pickpocket / stunned / banking for food. All `ingame=true`,
`scene_state=2`, Ardougne-area positions.

### observe-end (`elapsed_s=403.372`)

All 16 `Running`, `error=null`, `in_flight=null`. Steals **19–51** (sum **594**).
Food **19–20**, bank trips **1** each. Bank closed/empty in runtime snapshot.
Ticks mostly ~661–668 (two slots 575). Titles: pickpocket or stunned — waiting.
No slot matches original failing account name `livefd010_5` (new throwaway set
`live11e09_*`).

| Slot | Steals | Food | Tick | Title (truncated) |
|:---|---:|---:|---:|:---|
| live11e09_0 | 49 | 20 | 668 | stunned — waiting |
| live11e09_1 | 39 | 19 | 663 | stunned — waiting |
| live11e09_2 | 33 | 19 | 664 | Pickpocket Guard (2669,3303,0) |
| live11e09_3 | 31 | 19 | 664 | Pickpocket Guard (2655,3313,0) |
| live11e09_4 | 35 | 19 | 663 | stunned — waiting |
| live11e09_5 | 40 | 20 | 575 | Pickpocket Guard (2655,3313,0) |
| live11e09_6 | 40 | 19 | 664 | Pickpocket Guard (2655,3313,0) |
| live11e09_7 | 41 | 20 | 664 | Pickpocket Guard (2669,3303,0) |
| live11e09_8 | 43 | 19 | 664 | Pickpocket Guard (2659,3306,0) |
| live11e09_9 | 51 | 20 | 663 | Pickpocket Guard (2659,3306,0) |
| live11e09_10 | 33 | 19 | 664 | Pickpocket Guard (2655,3313,0) |
| live11e09_11 | 30 | 19 | 664 | Pickpocket Guard (2660,3296,0) |
| live11e09_12 | 26 | 19 | 664 | Pickpocket Guard (2660,3296,0) |
| live11e09_13 | 38 | 20 | 575 | Pickpocket Guard (2655,3313,0) |
| live11e09_14 | 19 | 19 | 663 | stunned — waiting |
| live11e09_15 | 46 | 20 | 661 | Pickpocket Guard (2652,3304,0) |

Diagnostics sidecar: `samples.diagnostics.jsonl` **409** lines (aligned with sample
count); **zero** `stop_reason` / `stopReason` hits. `run.log`: no
`script requested stop`, no `FAIL: memory tui`.

## Comparison to original failed cell `20260907T005032Z`

| | Original | This diagnostic |
|:---|:---|:---|
| Binary | frozen candidate `769a9367…` @ host meta `bd5dc1e` | diagnostic rebuild `13a1221c…` @ `944fb60` (+stopReason capture) |
| Observe request | 120 s | 300 s |
| Sidecar | **off** | **on** |
| Exit | 1 (script stop) | **-15** (tool SIGTERM) |
| Failure message | `livefd010_5: script requested stop on tick 289` | none (no script stop) |
| `stop_reason` | unavailable (not captured / no sidecar) | no stop ⇒ field never written |
| Qualification | incomplete / failed | boundaries present but **not qualified** (interrupted) |

Original failure reason remains **unknown**. This cell does **not** prove the stop is
fixed, gone, or unrelated to shared-nav — it only shows one longer interrupted run
without a script stop under diagnostic capture.

## Provenance (nav / catalog)

| Asset | SHA-256 |
|:---|:---|
| `~/.274bot/274bot.navpack` | `2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30` |
| `~/.274bot/274bot.navflags` | `92d5dea05c886ac8720be6b47e47cbc68355a8ff42676c0886f5b7ea8343a4cb` |
| `~/.274bot/js-scripts.json` | `3bee852f888a9c383b23d299979a3e898c4b620b73b5c8a6f69e87584399d4f6` |
| rs2b0t | `100adccc037d9f6898080e1cad58fcfc43364775` |

Matches shared-nav freeze / clean-screen provenance.

## Material uncertainty

1. **Tool budget:** setup + 30 + 300 + 60 exceeds a 420 s foreground wait; teardown was
   cut. Future named runner must use background job + finite watchdog **>** full budget.
2. **Non-reproduction ≠ fix:** different binary (stopReason instrumentation), longer
   observe, diagnostics on, new accounts, and external kill confound causal inference.
3. **`stop_reason` path unexercised live:** unit tests cover capture; this cell never
   hit `ScriptRunner.stop`, so live emission into qualification/diagnostics is still
   unproven on a real stop.
4. **Original stop path still unknown:** bank/food/return hypotheses from forensics
   remain candidates only.
5. Resource series in samples are **diagnostic-contaminated**; ignore for budgets.

## ONE next diagnostic / fix proposal (not implemented)

**Next card:** one clean background PTY TUI N=16 active cell on the **same** diagnostic
binary + sidecar, watchdog **≥ 600 s** (or setup-measured + 30 + 300 + 90), no agent
foreground 420 s kill. Success criteria:

1. Process exits under harness control (0 or 1), not SIGTERM.
2. If exit 1 with `script requested stop`, require retained `stop_reason` string (or
   explicit unavailable) on the stopping slot at failure boundary — then map that string
   to the existing ThievingBot/`stopSafely` call site **without** changing stop semantics.
3. If exit 0 with full observe+teardown and all slots progressed, report
   **non-reproduction under adequate watchdog** still without declaring N16 supported or
   resuming the performance batch from this single cell.
4. Still no fixture/timeout relaxation; still exclude CPU/RSS/p99.

No code/fix implementation on this card.

## Artifacts (gitignored diagnostics)

| Path | Role |
|:---|:---|
| `docs/memory/diagnostics/20260907T012918Z_tui_n16_active/` | **Authorized** cell |
| `…/metadata.json` | exit -15, sidecar true, hashes |
| `…/samples.jsonl` | 409 phase samples through teardown 418.996 s |
| `…/samples.qualification.jsonl` | observe-start + observe-end |
| `…/samples.diagnostics.jsonl` | sidecar 409 lines |
| `…/run.log` | PTY log |
| `docs/memory/diagnostics/n16-stop-diagnostic-build-20260907T012907Z/` | binary + receipt |
| `docs/memory/diagnostics/20260907T013727Z_tui_n16_active/` | **Unauthorized** aborted rerun (exit -15, ~13 s) — ignore for claims |
| Original immutable | `docs/memory/diagnostics/20260907T005032Z_tui_n16_active/` |

Machine twin: [n16-stop-diagnostic-manifest.json](n16-stop-diagnostic-manifest.json).
