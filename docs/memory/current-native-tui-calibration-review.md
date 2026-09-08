# Independent review: current native TUI N16 calibration

Reviewer profile: `reviewer` (Grok 4.5 / xai-oauth).  
Reviewed commit: `adeed88` on `codex/memory-diagnostics`  
(`adeed88286b78d71e7d349ea140eb07b4da16309`).  
Task: `t_d6305d97` — evidence-stage recomputation, not source re-review of `82057de`.

## Verdict

**Usable as diagnostic calibration** for the frozen current TUI N16 cell, with
the limits the report already states. No numerical or identity blocker was found
that would invalidate the calibration ledger. This is **not** matched-pair
acceptance, overhead proof, fixed-vs-incremental ownership proof, or final
budget acceptance.

## Scope and method

- Exact HEAD `adeed88` report + evidence JSON + build report.
- Immutable local archives under `diagnostics/current-tui-n16-1616/`.
- Independent recomputation from `samples.jsonl`,
  `samples.qualification.jsonl`, managed `process_accounting.jsonl`, and
  `binding-replay-82057de/binding.json`.
- No new live runs, no raw modifications, no code changes.

## Archive and identity

| Claim | Independent result |
|---|---|
| Original archive SHA256 `7612191c…941c4` | Match on `current-tui-n16-1616-final.tar.gz` |
| Replay archive SHA256 `3a7f5bbe…3fc87` | Match on `binding-replay-82057de.tar.gz` |
| 93 verified files | `archive-manifest.json` has **93** path entries; every listed file hash verifies. On-disk/tar file count is **94** because the manifest itself is the extra file. The “93-file” claim is the verified payload set, not a tar member count bug. |
| Binary SHA256 `a0c6eb0b…591f9` | Match in run `metadata.json`, binding `binary_sha256`, and build report |
| Host `c0709ab…`, client `3456edc8…` | Match in binding `side_provenance` / metadata |
| System allocator, `memory-profile-no-alloc`, no scheduling/responsiveness/render/gpu profiles, no diagnostic sidecar, no allocation counting, terminal 120×40, N=16, workload active | Match in metadata and binding `match_keys`; all observe samples keep `diagnostic_sidecar=false` and `allocation_counting=false` |
| Raw run hashes in binding | `metadata.json`, `samples.jsonl`, `samples.qualification.jsonl` SHA256 all verify |
| Reader replay provenance | All 58 listed `reader-replay-82057de` file hashes match `replay-provenance.json` |
| Original vs corrected reader | Archive `reader-source` differs from `reader-replay-82057de` only in `matched_evidence_adapter.py` and `test_matched_evidence_adapter.py` — expected for `82057de`; raw run artifacts unchanged |

## Workload qualification (16 slots)

Qualification boundary rows are the two `samples.qualification.jsonl` lines:

- `elapsed_s` **209.154143169** → **809.196809023**
- span **600.042665854** s (exact match to evidence/report)
- both boundaries: phase observe intent, all 16 slots `state=Running`, `error=null`

Steal gains recomputed from inventory **Coins** delta ÷ 30 (30 coins per successful steal):

| | min | max | n | all > 0 |
|---|---:|---:|---:|---|
| Recomputed | 49 | 82 | 16 | yes |
| Evidence / report | 49 | 82 | 16 | yes |

Exact map match to `steal_gains` in evidence JSON and binding qualification.
During observe samples, `active` is uniquely **16** for all 580 window samples.

## Client resource recomputation

Observation resource window from raw `observation_window` /
observe-phase samples (`elapsed_s` 209.771027928 … 808.682896518):

| Quantity | Report / evidence | Independent recompute | Match |
|---|---:|---:|---|
| Sample count | 580 | 580 | yes |
| Sample bracket | 598.91186859 s | 598.91186859 s | yes |
| Actual phase span | 600.042665854 s | 600.042665854 s from qualification endpoints | yes |
| Phase vs bracket | phase not shortened by sampling | phase − bracket ≈ 1.131 s; first/last observe *samples* sit inside the wider qualification phase | yes |
| Median RSS | 546076672 B (520.779 MiB) | 546076672.0 B (520.779296875 MiB) | yes |
| Sampled max RSS | 626692096 B | 626692096 B | yes |
| Native peak RSS | 627003392 B (597.957 MiB) | max `peak_resident_bytes` over observe (and full run) = 627003392 | yes |
| Peak ≠ sample max | distinct | peak − sample max = 311296 B | yes |
| CPU seconds | 336.436373 | Δ(`user`+`system`) first→last observe sample = 336.436373 | yes |
| CPU cores | 0.561746… | 336.436373 / 598.91186859 = 0.5617460441918807 | yes |
| Mean client ticks / slot / s | 49.47900359658157 | Δ`client_tick_count` / (span × 16) = same | yes |
| Frontend exit | 0 | metadata/binding/completion/managed launcher path exit 0; VPS log “Calibration completed exit 0” | yes |
| Working targets | median 512 MiB, CPU 0.5, peak 768 MiB | median **miss**, CPU **miss**, peak **meet** | yes |

CPU denominator is the **sampled monotonic bracket**, not the 600.043 s phase —
as the report states. Using phase span as denominator would understate cores;
the report does not do that.

## Teardown (single sample)

Last sample (`phase=teardown`):

- `active=0`, `v8_live_isolates=0`, `v8_total_bytes=0`, `v8_used_bytes=0`,
  `snapshot_inflight_bytes=0`
- retained `resident_bytes=433799168` (exact claim match)
- `ready` remains 16 — report correctly claims zero *active* slots, not zero ready
- One teardown sample is not a lifecycle plateau proof (report limit stands)

## Helper / server accounting (separate intervals)

Managed `process_accounting.jsonl` roles: `game_server`, `controller`,
`ssh_parent`, `launcher`, `collector`. PIDs and `start_identity` values match
evidence continuously across samples.

Filtered to the evidence `cpu_wall_envelope`
`[1788884438.749683, 1788885039.250407]`:

| Role | Median RSS | Peak (window or full-run as noted) | CPU s (first→last cumulative) | Cores interval consistency |
|---|---:|---|---:|---|
| game_server | 654934016 (match); ~0.05484 cores | sampled peak 672890880 match in window | 32.93 match | cores = cpu/duration endpoints, independently sorted min/max — consistent |
| controller | 18649088 match | full-run peak 19304448 match; window peak can differ slightly | 3.05 match | ok |
| ssh_parent | 2359296 match | full-run peak match | 0.0 match | ok |
| launcher | 19951616 match | full-run peak match | 0.45 match | ok |
| collector | 16252928 match | full-run peak match | 2.6 match | ok |

Notes:

- Evidence `resident_sample_count: 1200` vs wall-filtered row counts 1201–1202
  is an edge-inclusion off-by-one; **medians still match exactly**, so this is
  not a calibration blocker.
- Helper peaks in evidence align with full-run sampled maxima for non-server
  roles; server peak matches inside the wall window.
- These series are **not** subtracted from client RSS/CPU and are **not** an
  OFF/ON overhead comparison — report discipline is correct.

## Binding vs resource gate (do not relabel)

| Surface | Result |
|---|---|
| Original independent binding in archive | `unavailable` / `host_conditions_invalid` / `binding_ok=false` — explicit `frontend_processes: []` |
| Managed host-conditions | `frontend_processes: []` confirmed |
| Replay binding (`82057de` reader) | `status=bound`, `binding_ok=true`, `qualified=true` |
| Managed cache/process resources | `available` |
| `pair_eligible` | `false` |
| `shaped_for_compare` / analysis resources gate | `unavailable` / **`missing_resource_provenance`** (unchanged) |
| Overhead | `unavailable` / `helper_overhead_accounting_missing`; shaped overhead `unknown` |
| Performance / final acceptance | `false` everywhere checked |

Managed sidecar binding success does **not** clear the raw/shaped resource
provenance gate. The report keeps that separation; this review agrees and does
not relabel.

## Challenge of report claims

| Claim class | Assessment |
|---|---|
| Diagnostic cell only; not accepted saving/performance | Supported by gates and numbers |
| All 16 qualified and progressed | Supported |
| Steady RSS and CPU exceed working targets | Supported (520.779 > 512; 0.5617 > 0.5) |
| Peak under 768 MiB working target | Supported |
| Phase 600.043 s vs sample bracket 598.912 s | Supported; sampling does not redefine phase length |
| Native peak ≠ sample max | Supported |
| Original binding reject vs replay bind | Supported; raw archive still carries reject receipt |
| Overhead unknown; helpers not client cost | Supported |
| Teardown counters logical only; not RSS savings | Supported |
| One N16 cannot separate fixed vs incremental | Supported |

No material overclaim found. The only pedantic notes are (1) “93 files” means
manifest entries, not tar members including the manifest, and (2) helper sample
count 1200 vs 1201/1202 window edges — neither changes medians or conclusions.

## Blockers for *using this as diagnostic calibration*

**None** for the narrow purpose: a frozen, workload-qualified, profile-off TUI
N16 diagnostic ledger with verified archives and honest gates.

Remaining **non-blockers that still forbid stronger claims** (already in report):

- `missing_resource_provenance` on shaped/raw resource gate
- overhead unmeasured
- `pair_eligible=false` / no matched comparison
- no profiled latency, per-slot cadence, input latency, or GPU completion
- single teardown ≠ lifecycle acceptance
- median RSS and mean CPU already miss working targets

## Ownership experiment: one bounded current N1 / N16 same-build?

**Supported as the next attribution step**, owned by root for integration —
not authorized by this review as a live launch order.

Rationale:

- This N16 cell is a stable same-build baseline (binary/manifest/commits fixed).
- Client RSS/CPU and qualification are independently reproducible from raw.
- Helpers are already separated, so a second N on the same build can attribute
  client fixed vs per-bot incremental without pretending overhead OFF/ON.
- A single N cannot identify the intercept; N1 + this N16 (or fresh N1+N16) can.

### Minimum useful attribution evidence

1. **Same frozen build** as this cell: binary SHA `a0c6eb0b…`, host `c0709ab`,
   client `3456edc8`, profiles off, System allocator, no sidecar/counting,
   real 120×40 terminal, same warmup/observe intent (120 s / 600 s).
2. **One N=1** active Thiever cell and **this N=16** (reuse allowed if still
   the sole calibration) or a fresh pair on that same build only.
3. Per cell, the same ledger fields as here: qualification (16 or 1 slots,
   steal gains), phase span, sample bracket + n, median RSS, sample-max RSS,
   native peak RSS, CPU seconds and cores over the sample bracket, tick mean,
   exits, binding/gate statuses without relabeling.
4. **Attribution math only**: e.g. incremental ≈ (N16 − N1) / 15 and fixed ≈
   N1 (or regression intercept) for **RSS and CPU separately**; publish
   uncertainty from one pair (no false precision).
5. Keep server/helper intervals **separate**; do not subtract into client.
6. Explicitly **not** claimed: matched saving, performance acceptance,
   overhead OFF/ON, lifecycle plateau, cadence/GPU, speculative optimization.

### Not required for that minimum

- Full matrix across allocators, sidecars, or profile-on variants
- Instrumentation overhead campaign
- Per-slot p99 / input / GPU proofs
- Immediate optimization work before ownership numbers exist

## Files reviewed (primary)

- `docs/memory/current-native-tui-calibration-report.md`
- `docs/memory/current-native-tui-calibration-evidence.json`
- `docs/memory/current-native-tui-build-report.md`
- `docs/execution.md` (post-measurement independent recompute requirement)
- `diagnostics/current-tui-n16-1616/current-tui-n16-1616-final/` (+ tar SHA)
- `diagnostics/current-tui-n16-1616/binding-replay-82057de/` (+ tar SHA)
- `diagnostics/current-tui-n16-1616/reader-replay-82057de/` (provenance hashes)

## Conclusion

`adeed88`’s calibration report and evidence JSON match an independent
recomputation from the immutable raw and replay artifacts. Treat
`current-tui-n16-1616` as the **diagnostic N16 baseline**. Next useful
measurement work is **one bounded same-build N1↔N16 ownership split** for
memory and CPU; root owns that experiment design and any live run. Do not
promote this cell to acceptance, matched savings, or overhead conclusions.
