# Fixed-window cohort reader report

Scope: Python offline reader only (`docs/memory/cohort_reader.py`, additive
gates in `reference_metrics.py`, focused tests, this report). No Rust, native,
SSH, launcher, STATE, or archive rewrite. This is **not** a native latency or
performance acceptance claim.

## Frozen reader contract

### Entry points

- Module: `docs/memory/cohort_reader.py`
- API: `read_cohort(run_dir, meta, samples, gate)` with `gate in {"decode","input"}`
- Wired into `reference_metrics.analyze_run` as additive gates `decode_cohort`
  and `input_cohort` (legacy `decode`/`input`/resources/scheduling/gpu unchanged)
- CLI: existing `--require` selector accepts the new names (no new flag)

```
python3 docs/memory/reference_metrics.py RUN --require decode_cohort,input_cohort
```

`final_acceptance_claim` remains `false`. Archives without `samples.cohort.jsonl`
report `cohort_schema_missing` while legacy gate results stay visible.

### Fail-closed boundary

`read_cohort` never raises on malformed archives. Unexpected faults map to
`status=unavailable`, `target_verdict=unavailable`, reason
`malformed_cohort_input` (with exception type in `detail`).

### Sidecar envelope (publisher schema 1)

Required shape (source: `host::responsiveness_cohort` + host-play publisher):

1. Exactly one leading `cohort-header`
2. Zero or more `cohort-batch` records
3. Exactly one final `cohort-terminal` (no trailing records)
4. Final batch must have `complete=true`
5. `schema_version=1`, `clock_domain=responsiveness_process_mono`,
   `tail_name=DEFAULT_TAIL_NS` (5_000_000_000 ns), immutable boundaries,
   `observe_ns == end - start`
6. `producers_joined=true`, finite `observe_end_elapsed_s` on terminal

### Provenance and path binding

- Reads only files inside the bound run directory:
  `samples.cohort.jsonl`, `samples.jsonl` (caller-provided),
  `samples.qualification.jsonl`, `metadata.json` (via analyze_run)
- Never opens a recorded native path; never basename-only or relative refs
- Recorded sidecar claims must be absolute (POSIX or `X:\...`), end with
  `<run_dir.name>/samples.cohort.jsonl`, and agree exactly (separator-normalized)
  across header + sample + qualification cohort refs
- Qualification cohort refs also require
  `qualification_elapsed_s_is_harness_not_cohort_mono=true` and a string
  `phase_tag` (harness elapsed is not cohort mono)

### Identity mapping (source-faithful)

- Qualification JSON top-level `slots[]` (from `serialize_qualification_boundary`),
  each: `ordinal`, `name`, `responsiveness_slot_id` (FNV-1a of name), **no**
  `generation`
- Generations come only from **cohort-bearing** sample rows
  (`sample.cohort.present is True`); seed/warmup/teardown without cohort attach
  are ignored so a legitimate pre-arm seed restart does not look like an
  in-cohort reset
- Multiple generations per slot **within** the cohort-bearing span →
  `sample_generation_reset`
- Header `focused-one.slots:[0]` means **fixture ordinal 0**, resolved through
  qualification `responsiveness_slot_id` (nonzero FNV), not `EventId.slot_id=0`
- Decode population: `all-run-slots` with `n == meta.n` — every slot identity
- Input `focused-one`: declared ordinals only; other slots are outside the input
  population (not failures/passes)
- Input `all-run-slots` and `tui-endpoint` supported when declared; TUI requires
  `frontend=tui` and uses surface `Tui` (not panel texture-present)

### Membership and accounting

- Include event iff `START <= start_mono_ns < END`
- Completion: `complete >= start` and `complete <= END + tail` (equality allowed)
- Out-of-window starts are malformed (not silently dropped)
- EventId.sequence / LossReceipt.sequence ≠ journal cursor
- Completions may arrive out of start order; no start-ID sort requirement
- Cursor: inclusive high-water; empty batch keeps cursor; non-empty advances by
  `len(records)+len(losses)` exactly (no silent gaps/stale cursor)
- Counter-only loss gaps (`loss_count`/`losses_n` without receipts) keep the
  cohort unavailable
- Any loss receipt, overflow, `available=false`, or non-completed member →
  unavailable
- Terminal `pending_n` is post-finalize (often 0); nonzero → unavailable
- `records_n` must equal serialized EventRecords

### Meta / settings agreement

Requires `responsiveness_profile=true` and fine enabled (`responsiveness_fine`
or qualification `responsiveness_fine_enabled`). Frontend and N must agree
across meta/header/settings when present. Focused-one header requires
`render_policy`/`render_policy_requested` == `focused-one` when set.

### p99 math (frozen)

`p99_ns` SHA-256 (function source):

`ade46732007625b4ddd83f7895e38350646c7799879ed8f9030c8ceb41a9d8f3`

Root independent check (640 deterministic cases, seed 274) validated this
exact hash. Compare durations in **nanoseconds** to inclusive integer ms upper
edges (do not floor 1.1 ms into 1 ms). Coarse bounds
`(5,10,20,25,40,50,100,250,500,1000)`; fine `1..100`. Overflow bucket has lower
bound only (`upper_ms=None`). Per-slot fine p99 drives the ≤100 ms verdict;
a slow slot cannot be pooled away in a fleet average. Target verdict:

- `meet` if every declared slot’s fine upper_ms ≤ 100
- `miss` if available but upper exceeds 100
- `unavailable` on overflow/empty/malformed

### Output shape (available)

```
status, gate, target_verdict, reason,
population_n, event_n, records_n, losses_n,
boundaries{start_mono_ns,end_mono_ns,tail_ns},
p99, fine_p99, slots[{slot_id,generation,sample_n,p99,fine_p99,target_verdict}],
events
```

Unavailable results always include `status=unavailable`, `target_verdict=unavailable`,
and a stable `reason` string (see tests for the enumerated set).

## Reason codes (non-exhaustive, stable)

`cohort_schema_missing`, `cohort_header_missing`, `multiple_cohort_headers`,
`unknown_cohort_schema`, `malformed_cohort_boundaries`, `observe_duration_mismatch`,
`sidecar_provenance_mismatch`, `cohort_frontend_mismatch`,
`responsiveness_profile_disabled`, `responsiveness_fine_disabled`,
`qualification_missing` / `_malformed` / `_slots_*` / `_unexpected_generation`,
`sample_generations_missing`, `sample_generation_reset`,
`cohort_sample_ref_missing` / `_mismatch`, `cohort_qualification_ref_*`,
`decode_population_mismatch`, `input_population_*`, `missing_generation_for_slot`,
`cursor_regression`, `cursor_accounting_mismatch`, `loss_counter_regression`,
`record_after_terminal`, `cohort_terminal_missing`, `producers_not_joined`,
`final_batch_incomplete`, `cohort_accounting_unavailable`,
`declared_population_incomplete`, `unexpected_slot_identity`,
`noncompleted_member`, `event_start_out_of_window`,
`malformed_completion_timestamp`, `overflow_bucket`, `empty_cohort`,
`malformed_cohort_input`, …

## Verification commands and counts

```
cd /Users/acfrazier/experiments/274bot/.worktrees/latency-cohort-reader
python3 -m unittest discover -s docs/memory -p 'test_cohort_reader.py' -v
# → 34 tests, 0 fail, 0 err

python3 -m unittest discover -s docs/memory -p 'test_reference_metrics.py' -v
# → 100 tests; 1 fail + 1 err — both missing EXTERNAL fixtures not present in
#   this isolated worktree (not regressions from this change):
#   - missing .../diagnostics/low-end-reference-screen-20260906T220129Z/tui_n1_active
#   - missing .../diagnostics/clock-adapter-review-20260907T022630Z/input-interior-conservation-mismatch.json
#   Those paths exist under primary-linked worktree
#   /Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f/docs/memory/diagnostics/
#   and should be re-run there; they are not cohort-reader artifacts.

python3 -m unittest discover -s docs/memory -p 'test_matched_evidence_adapter.py' -v
# → 46 tests, 0 fail, 0 err, 1 skip

python3 -m unittest discover -s docs/memory -p 'test_instrumentation_overhead.py' -v
# → 29 tests, 0 fail, 0 err, 1 skip
```

p99 freeze hash verified unchanged at report time (matches root 11:34 UTC check).

Root archive generation scope (read-only
`t_a1f4796f/diagnostics/windows-input-smoke-0942-archive`, 381 samples):
without cohort refs → `sample_generations_missing`; observe-only with synthetic
`cohort.present` → single generation 2; tagging all phases would reset — confirms
pre-arm seed gen1 must stay outside cohort span.

## Mandatory scenario coverage (tests)

Full synthetic N=16 archived RUN + `analyze_run` + CLI `--require`;
legacy archive without sidecar; tail completion; start-at-END / pre-start;
late completion beyond tail; out-of-order start IDs valid; cancel; overflow;
counter-only loss; trailing batch after terminal; missing terminal;
finalized-unavailable pending0; duplicate identities; cursor gap/stale;
missing generation; no input events → population incomplete; basename-only and
relative paths rejected; profile/fine required; slow slot not pooled; header-only;
producers_joined; final batch complete; focused-one ordinal0→FNV id; qualification
shape is `slots` not `qualification_slot_rows` / no generation on qual slots;
FNV matches Rust offset basis; bool rejected as uint; pre-arm seed gen ignored;
in-cohort generation reset; quiet-inner-span not selected; malformed surface and
fuzzed scalar types → unavailable without exception; N=16 single-slot cannot meet;
ns boundary / exact 100ms / 100ms+1ns overflow.

## Limitations (honest)

- No native/live publisher run in this isolated checkout; fixtures are
  source-faithful synthetic envelopes plus path-binding of Windows-style
  absolute claims onto the bound archive leaf
- External reference-cell diagnostics are absent from this worktree (exact
  paths listed above); primary checkout holds them under `t_a1f4796f`
- Not a native latency pass, matched-pair proof, or campaign acceptance
