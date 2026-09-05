# memory-per-bot T4 — live baseline matrix

**Branch:** `memory-per-bot-t4`  
**Commit (harness):** `e1ebd59` (T3 wiring; measurement-only hop — no T5+ opts)  
**Host:** local engine `127.0.0.1:43594`, `LIVE=1`, throwaway vaults  
**RS2B0T:** `/Users/acfrazier/experiments/rs2b0t`  
**Protocol:** warmup 120s, observe 600s, teardown ≥60s; 1 Hz jsonl (`Sample`, null ≠ 0)  
**Workload:** idle = no script; active/lifecycle = Thiever scenario seed + XP proof  
**Allocator:** counting Rust global alloc installed only under `--features memory-profile`  
**Do not** sum RSS + rust_live + V8 + GPU. **Do not** equate `rust_live_bytes` with RSS.

## Dispatch outcome

**Stopped fail-closed on honest FAIL** (seed/proof, not incomplete scale).

```
FAIL: memory tui: Thiever seed/proof failed for live773e0_17:
  step 6 (watch the script pickpocket guards):
  stat_xp_gain(17)>=1 not seen within 150 ticks
```

Cell: **tui × N=32 × active × r2** — exit 1. Matrix runner halted. No retry.

Remaining cells **not run** (not blocked-scale): tui N=32 active r3; tui N=32 lifecycle ×3; 32-slot 3600s lifecycle soak; all N=128 × {panel,tui} × {idle,active,lifecycle} ×3.

## Cell status matrix

| frontend | N | workload | r1 | r2 | r3 | notes |
|---|---|---|---|---|---|---|
| panel | 1 | idle | PASS | PASS | PASS | full 3 |
| panel | 1 | active | PASS | PASS | PASS | Thiever |
| panel | 1 | lifecycle | PASS | PASS | PASS | |
| tui | 1 | idle | PASS | PASS | PASS | |
| tui | 1 | active | PASS | PASS | PASS | |
| tui | 1 | lifecycle | PASS | PASS | PASS | |
| panel-cpu | 1 | idle | PASS | — | — | `BOT_CPU=1` one-shot |
| panel | 32 | idle | PASS | PASS | PASS | ready=32 |
| panel | 32 | active | PASS | PASS | PASS | ready=32 |
| panel | 32 | lifecycle | PASS | PASS | PASS | ready=32 in observe (r1 teardown ready_last=31) |
| tui | 32 | idle | PASS | PASS | PASS | ready=32 |
| tui | 32 | active | PASS | **FAIL** | not run | fail-closed stop |
| tui | 32 | lifecycle | not run | not run | not run | |
| panel / tui | 128 | * | not run | not run | not run | |
| soak 3600s | 32 lifecycle | — | not run | | | would use panel (all 32-lifecycle 600s PASS) |

Jsonl roots: `docs/memory/samples/<frontend>_n<N>_<workload>_r<k>.jsonl`  
Run log: `docs/memory/samples/matrix_summary.tsv`  
PASS line: `PASS: memory panel/tui observation complete`

## Method notes

- Mid-observe metric = median of middle third of `phase=observe` samples (avoids edge of warmup/teardown).
- Cell median = median across the three (or fewer) fresh-process runs that reached observe.
- **Bytes per additional ready bot** = `(median_N32 − median_N1) / 31` for that frontend×workload. Process fixed cost stays in N=1; this is **not** a promise that 128 will scale the same.
- Peak figures are `peak_resident_bytes` (ru_maxrss-derived). Current RSS is `resident_bytes`.
- Teardown last = last jsonl line in `phase=teardown` (≈60s after script stop).

## Mid-observe medians (PASS cells)

| cell | runs w/ observe | RSS mid | rust_live mid | peak RSS (any) | start peak | teardown last RSS |
|---|---|---|---|---|---|---|
| panel N=1 idle | 3 | 514.7 MiB | 423.0 MiB | 603.0 MiB | 470.3 MiB | 602.6 MiB |
| panel N=1 active | 3 | 649.1 MiB | 515.5 MiB | 736.1 MiB | 613.3 MiB | 734.8 MiB |
| panel N=1 lifecycle | 3 | 567.4 MiB | 515.2 MiB | 643.2 MiB | 624.4 MiB | 637.7 MiB |
| panel-cpu N=1 idle | 1 | 488.6 MiB | 323.6 MiB | 534.7 MiB | 450.7 MiB | 533.1 MiB |
| tui N=1 idle | 3 | 249.1 MiB | 143.0 MiB | 249.3 MiB | 248.5 MiB | 249.3 MiB |
| tui N=1 active | 3 | 375.0 MiB | 224.4 MiB | 381.6 MiB | 375.8 MiB | 370.5 MiB |
| tui N=1 lifecycle | 3 | 369.6 MiB | 223.6 MiB | 381.9 MiB | 376.2 MiB | 370.5 MiB |
| panel N=32 idle | 3 | 3.56 GiB | 3.11 GiB | 3.73 GiB | 3.43 GiB | 3.73 GiB |
| panel N=32 active | 3 | 7.39 GiB | 6.73 GiB | 7.65 GiB | 7.22 GiB | 7.38 GiB |
| panel N=32 lifecycle | 3 | 7.26 GiB | 6.30 GiB | 7.57 GiB | 7.23 GiB | 7.41 GiB |
| tui N=32 idle | 3 | 872.3 MiB | 528.0 MiB | 872.4 MiB | 869.9 MiB | 872.4 MiB |
| tui N=32 active | 1 (r1 only) | 3.99 GiB | 3.10 GiB | 4.06 GiB | 4.00 GiB | 3.77 GiB |

## Bytes per additional ready bot (N=1 → N=32)

| frontend | workload | ΔRSS / 31 | Δrust_live / 31 |
|---|---|---|---|
| panel | idle | **98.2 MiB** | 86.6 MiB |
| panel | active | **217.6 MiB** | 200.4 MiB |
| tui | idle | **20.1 MiB** | 12.4 MiB |
| tui | active | **116.6 MiB** (1× N32 run) | 92.7 MiB |

Lifecycle N=32 panel ΔRSS/31 ≈ (7.26 GiB − 567.4 MiB) / 31 ≈ **216.7 MiB** (same order as active).

## Startup / transition peaks

- **Startup peak** (seed+warmup `peak_resident_bytes` max, cell median): panel N=1 idle ~470 MiB; panel N=32 idle ~3.43 GiB; panel N=32 active ~7.22 GiB; tui N=1 idle ~248 MiB; tui N=32 idle ~870 MiB.
- **Observe-window peak** tracks steady growth under counting alloc + game load; lifecycle observe shows stop/start half-cycles (`active` flips 32↔0) without dropping ready count while established.
- Panel N=32 lifecycle r1 ended teardown with `ready_last=31` after a full observe at ready=32 — counted PASS (observation completed).

## Retained RSS after teardown

Teardown last RSS stays near observe peak for most cells (OS does not return all dirty pages immediately). Examples (cell median):

- panel N=1 idle: ~603 MiB retained vs ~515 MiB mid-observe  
- panel N=32 idle: ~3.73 GiB retained ≈ peak  
- tui N=1 idle: ~249 MiB (flat)  
- tui N=32 active r1: teardown last 3.77 GiB vs mid 3.99 GiB (slight drop after script stop)

`rust_live_bytes` often **rises** into teardown on active/lifecycle (allocator accounting + deferred free) — treat as a separate domain from RSS.

## Renderer compatibility (`BOT_CPU=1`)

One panel N=1 idle with `BOT_CPU=1`: PASS. Mid RSS 488.6 MiB / rust_live 323.6 MiB vs GPU panel idle median 514.7 / 423.0 MiB. Not a full CPU factorial.

## Null fields (T2 contract still open)

Every sample: `snapshot_inflight_bytes`, `snapshot_inflight_capacity`, `v8_used_bytes`, `v8_total_bytes`, `gpu_tracked_bytes` are JSON **null** (not zero).

## Incomplete / not attempted

| item | status |
|---|---|
| tui N=32 active r2 | **FAIL** (Thiever XP proof timeout on one slot) |
| tui N=32 active r3 | not run (fail-closed) |
| tui N=32 lifecycle ×3 | not run |
| panel/tui N=128 ×3 workloads ×3 | not run |
| 3600s lifecycle soak | not run (intended frontend: **panel**, all three 600s lifecycle PASS) |

No N=128 blocked-scale evidence this hop — scale fill was never attempted after the FAIL stop.

## Artifacts

- Samples: `docs/memory/samples/*.jsonl` (33 files; includes FAIL partial `tui_n32_active_r2.jsonl`)
- Logs: `docs/memory/samples/*.log`
- Summary TSV: `docs/memory/samples/matrix_summary.tsv`
- Aggregation scratch: `docs/memory/samples/agg_out.txt`, `docs/memory/agg_cells.py`

## Interpretation (baseline only)

- Headless **tui** idle marginal RSS/bot (~20 MiB) is far below headed **panel** idle (~98 MiB). Active Thiever multiplies both (panel ~218 MiB/bot RSS, tui ~117 MiB/bot on the single completed N=32 active).
- Fixed process cost is large relative to one bot; use N=1 vs N=32 deltas, not N=1 alone, for “per bot” talk.
- No savings claim. No percentage. This is the pre-opt baseline for T5–T9.

## Fail line (operator interrupt)

```
FAIL: memory tui: Thiever seed/proof failed for live773e0_17: step 6 (watch the script pickpocket guards): stat_xp_gain(17)>=1 not seen within 150 ticks
```

Queued/completed before stop: all N=1 (panel+tui ×3 workloads ×3), panel-cpu idle, all panel N=32, tui N=32 idle ×3, tui N=32 active r1 PASS then r2 FAIL. Suspected: concurrent Thiever seed race / guard XP not observed on one of 32 slots within 150 ticks — not a missing host verb.
