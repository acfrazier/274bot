# Shared-nav clean screen (failed short batch)

Task `t_54b3b727`. Measurement only on branch `codex/memory-diagnostics`.
**No performance acceptance, no absolute-budget pass, no accepted RSS/CPU saving.**

## Outcome

| Result | Detail |
|:---|:---|
| Status | **Failed screen — stopped early** |
| Stop cell | `candidate_n16_profiles_on` (3rd of 8 authorized) |
| Functional failure | `FAIL: memory tui: livefd010_5: script requested stop on tick 289; isolate stopping` |
| Process exit | 1 |
| Qualification | `qualified=false` — missing boundary qualification; observation incomplete (~110.56 s); process failed or incomplete |
| Qualified cells retained | control N1 profiles-on; candidate N1 profiles-on |
| Not run (do not invent) | control N16; four candidate N16 overhead cells |
| Retry in this card | **No** — one functional/scale failure stops the batch |

Orch investigates the N16 stop separately. This card makes **no source, Docker, metric-tool, or rebuild** changes.

## Scope (explicit non-claims)

- Initial short screen only (30 s warmup / 120 s observe / ~60 s teardown).
- Not final absolute budgets, three-run matrix, native owner removal, modest-hardware validation, or campaign acceptance.
- Single forward N1 pair is **diagnostic only**; `accepted_saving=false`.
- Strict `--require resources` remains **unavailable** (overhead unknown; match provenance incomplete). Numerical RSS/CPU under `--inspect` are still reported.

## Quiet host and contamination

| Item | Value |
|:---|:---|
| Contamination start (UTC) | `2026-09-07T00:40:29Z` |
| Contamination end (UTC) | `2026-09-07T00:54:51Z` |
| Preflight | no cargo/rustc/frontend benchmark; load on 16-core M4 Max |
| In-process `quiet_check` | cargo/rustc only in the **running** batch interpreter (disk patch after start did not reload) |
| Manual stronger receipts | `quiet_boundary_post_cell01_pre_cell02.json` (00:46:11Z), `mid_batch_005148` (00:51:48Z), `mid_batch_005600` (00:56:00Z) — all `quiet=true` |
| Manual needles | foreign frontends, native profilers, agent test workloads, other board running tasks |
| Local code tests during observe | none |
| Orch monitoring | read-only OK |

Diagnostic root (gitignored):

`docs/memory/diagnostics/shared-nav-clean-screen-20260907T004029Z/`

## Ambient helpers (retained, not started/stopped)

| Helper | Identity | Preflight resources |
|:---|:---|:---|
| Game server | PID **4719**, `macos_lstart:Wed Aug 26 15:57:11 2026`, listen **43594** | RSS 1 236 713 472 B; argv/env **not** recorded |
| `memory-ref-amd64-view` | image `sha256:5158583a…`, `:6080` | ~0.53% CPU, 169.2 MiB |
| `fr-vault-chroma` | image `sha256:5c8c39dc…`, `:8765` | ~0% CPU, 5.0 MiB |
| macOS pressure | **unavailable** (`unsupported_pressure_counter`) | unavailable ≠ healthy |

`server_resources.py` consumed the explicit PID only. Sampler overhead is **unmeasured**. Helper `sample_self` snapshots are startup/residual only — **no** measured helper CPU delta or attributable overhead claim.

## Binary and data provenance

Frozen builds from reviewed `docs/memory/shared-nav-build-manifest.json` (`shared-nav-build-20260906T235815Z`):

| Binary | SHA256 | Role |
|:---|:---|:---|
| `tui-play-control-system-20260906T235815Z` | `53ddeda0eadd241913a2b05c01eb35c90af5af8d70122387a95fea72364b3eb2` | control (d373ea0; 606c93b reverted) |
| `tui-play-candidate-system-20260906T235815Z` | `769a9367e8c2808ebf7b1feb9f2d4830804524613721c9082e22056d7ac61928` | candidate (rust base 6345fbc) |

- Feature: `memory-profile-no-alloc` / `std::alloc::System`. No stack logging, counting allocator, verbose sidecar, or native attribution.
- Both binaries share per-slot timing instrumentation; historical `57cbfc7` reference was **not** used.
- Preflight verified nav pack / nav flags / `js-scripts.json` hashes match the manifest.
- Checkout at run: host `bd5dc1e`, client `451759f2`. **`run_diagnostic` metadata `host_commit` is the current checkout even when the binary is older; `binary_sha256` is the binary provenance authority.**

## Cell protocol

- Frontend: actual PTY TUI **120×40** (not headless).
- Workload: sustained active level-50 Thiever; loadouts/random events unchanged.
- `run_diagnostic.py tui N active --sustain --no-diagnostics --binary … --warmup 30 --observe 120`
- Enabled cells add `--scheduling-profile --responsiveness-profile --render-profile` (TUI has **no** GPU completion flag).
- Authorized order: controlN1 → candidateN1 → candidateN16 → controlN16 → candidate N16 overhead OFF/ON/ON/OFF.

## Cell results

### 1. control N1 profiles ON — qualified

| Field | Value |
|:---|:---|
| Run dir | `docs/memory/diagnostics/20260907T004143Z_tui_n1_active` |
| Exit / qualified | 0 / true |
| Steal proof | `livef7c70_0`: 7 |
| Observe span | 118.209 s |
| Median RSS | **384 958 464 B (367.125 MiB)** — absolute TUI N1 median budget 256 MiB → numerical **miss** |
| Lifetime peak RSS | 392 527 872 B (peak budget 384 MiB → numerical meet) |
| CPU cores | **0.037826** (user+system Δ / observe wall) |
| Scheduling | **available / meet** — 41.943 iter/s, p99 **[25, 26] ms**, full fleet 1/1 |
| Decode | **unavailable** — `coverage_lost_or_incomplete`, `decode_canceled_n=11`, lost/drop/pending 0 |
| Input | **unavailable** — `no_input_samples` |
| Resources strict | **unavailable** — `missing_resource_provenance`, `overhead=unknown` |
| `--require scheduling,decode` | exit 1 (decode) |
| Server sampler | 260 samples OK; median RSS 1 236 713 472 B; median CPU cores ~0.040 |

### 2. candidate N1 profiles ON — qualified

| Field | Value |
|:---|:---|
| Run dir | `docs/memory/diagnostics/20260907T004608Z_tui_n1_active` |
| Exit / qualified | 0 / true |
| Steal proof | `livefa6c0_0`: 9 |
| Observe span | 118.336 s |
| Median RSS | **310 394 880 B (296.016 MiB)** — still misses 256 MiB absolute median |
| Lifetime peak RSS | 318 308 352 B |
| CPU cores | **0.040906** |
| Scheduling | **available / meet** — 42.163 iter/s, p99 **[25, 26] ms**, full fleet 1/1 |
| Decode | **unavailable** — `decode_canceled_n=14` (same conservative end-snapshot class) |
| Input | **unavailable** |
| Resources strict | **unavailable** — same provenance/overhead class |
| `--require scheduling,decode` | exit 1 (decode) |
| Server sampler | 260 samples OK; same server RSS band |

### 3. candidate N16 profiles ON — functional failure (batch stop)

| Field | Value |
|:---|:---|
| Run dir | `docs/memory/diagnostics/20260907T005032Z_tui_n16_active` |
| Exit / qualified | 1 / false |
| Fail line | `livefd010_5: script requested stop on tick 289; isolate stopping` |
| Partial observe | ~110.56 s (incomplete) |
| Partial median RSS | 947 363 840 B (unqualified; not for acceptance) |
| Partial CPU cores | ~0.238 |
| Scheduling on partial window | diagnostic available/meet 16/16, worst p99 upper 25 ms — **does not override functional failure** |

## Matched N1 comparison (diagnostic only)

| Metric | Control | Candidate | Notes |
|:---|---:|---:|:---|
| Median RSS | 367.125 MiB | 296.016 MiB | Δ ≈ **−71.1 MiB** (candidate lower) |
| Peak RSS | 374.3 MiB | 303.6 MiB | lower on candidate this pair |
| CPU cores | 0.037826 | 0.040906 | **+8.14%** on candidate (>5% provisional non-regression margin) |
| Sched p99 | [25, 26] ms | [25, 26] ms | both meet; conservative 2 ms regression rule not used as a pass |

**No accepted fixed-RSS or CPU saving** from this single forward pair:

1. `compare_matched_runs` treats a single pair as inconclusive.
2. Strict resources gate unavailable (`overhead=unknown`, missing match provenance fields).
3. Profiles-on instrumentation overhead not attributable.
4. Reverse-order pair and N16 matched control never collected.
5. Both medians still miss the absolute TUI N1 256 MiB engineering budget.

Intended fixed-cost RSS movement is **observed directionally** (shared Nav) but **not accepted** here.

## Tooling gaps (honest)

- Attributable overhead-evidence reader not implemented → resources gate permanently unavailable until it exists; do not unlock with metadata labels.
- Decode lifetime cancel counters block decode gate (known core limitation; later observation-window adapter may separate warmup if raw evidence supports).
- Input visible endpoint never generated → unavailable, not pass.
- Scheduling `scene_transition_separation` unavailable.
- macOS pressure unavailable.
- Server/helper sampler overhead unmeasured; do not subtract helper RSS from client.
- Running-batch quiet_check narrower than card text; stronger checks are the manual boundary receipts above.

## Next requirements (downstream; not this card)

1. Root-cause and fix/re-fixture candidate N16 `livefd010_5` script stop (orch), then re-attempt N16 functional cells.
2. Finish **control N16** profiles-on after N16 is functionally clean.
3. **Reverse matched order** (candidate then control) for N1 and N16 short pairs.
4. **N1 overhead** profiles OFF/ON (and authorized N16 overhead OFF/ON/ON/OFF) with sampler aligned; profiles-off has **no timing proof**.
5. Keep native diagnostics out of clean CPU/latency cells until after clean phase.
6. Do not promote this failed short screen to final budgets, three-run matrix, modest hardware, or campaign acceptance.

## Deliverables committed

| File | Role |
|:---|:---|
| `docs/memory/shared-nav-clean-screen.md` | This report |
| `docs/memory/shared-nav-clean-screen.json` | Structured twin |

Raw run dirs, server JSONL, receipts, quiet boundary files, and batch logs remain under the gitignored diagnostic root named above.
