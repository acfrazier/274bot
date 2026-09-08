# Fixed-window cohort reader report

Scope: Python offline reader only (`docs/memory/cohort_reader.py`, additive
gates in `reference_metrics.py`, focused tests, this report). No Rust, native,
SSH, launcher, STATE, or archive rewrite. This is **not** a native latency or
performance acceptance claim.

Corrective card `t_25c2eb89` after `t_829c6804` / ec5dda2: close remaining
endpoint/population and clock/bounds provenance gaps reproduced by root
(`diagnostics/cohort-reader-ec5dda2-root-probes.json`).

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
7. Strict typed bounds: u16 schema version; u64 fields in `0..=2**64-1`
   (bool is not an integer); capacity as bounded usize/u64; valid enum
   shapes for surface/outcome/loss reason. Values such as `sequence=2**64`
   are rejected (`malformed_event_identity`).

### Endpoint and population (source policy)

Supported frontends only: `panel` | `tui`. Headless and other labels are
`unsupported_frontend` (no UI endpoint → never meet with Panel/Tui events).

Input population is **derived from source policy**, not trusted from arbitrary
header claims. Header must match the derived shape exactly:

| frontend | render policy | header `input_population` |
| --- | --- | --- |
| panel | `fixed-one`, `focused-one`, `focused-plus-background` (`RenderPolicy::pins_focus`) | `{"kind":"focused-one","slots":[0]}` |
| panel | `rotating-all` (or unset → rotating) | `{"kind":"all-run-slots","n":N}` |
| tui | rotating-all only | `{"kind":"tui-endpoint","note":"TUI flush endpoint; not panel texture present"}` |

- pins_focus always fixture **ordinal 0** → FNV `responsiveness_slot_id`;
  alternate/duplicate/empty ordinals → `input_population_ordinal_mismatch` (etc.).
- Decode population remains `all-run-slots` with `n == meta.n`.
- Panel vs Tui surfaces are distinct; input gate selects `Panel` or `Tui` by
  frontend. Mixed/forged endpoint success is rejected.
- Meta `render_policy` and qual `render_policy_requested` must agree when both set.

### Process-mono clock and sample lifecycle

Harness `elapsed_s` and process mono are **independent clocks**. Do not equate
them or invent float tolerances against mono when brackets exist.

Every cohort-bearing sample must publish:

- `responsiveness_clock` with `domain=responsiveness_process_mono`
- valid sample/read mono brackets (`lo<=hi`, non-unset), `elapsed_mono_ns` u64
  contained in the sample bracket; read contained in sample
- per-population-slot capture brackets for the gate prefix
  (`decode_capture_mono_*` / `input_capture_mono_*`) contained at/before
  sample/read upper

Observe lifecycle (source publisher):

- ≥2 observe samples with clocks (first + final publication)
- final observe sample mono bracket **encloses** immutable `end_mono_ns`
  (forced final observe at END before drain)
- observe samples not outside `[START, END]` mono domain
- drain samples after END, not past `END+tail`
- mono elapsed non-decreasing across cohort-bearing samples

Missing/malformed/inverted/domain-mismatched clocks, missing capture brackets,
and misaligned final observe fail closed. Aggregate edge `pending==0` is **not**
required (cohort identity/loss/terminal accounting is).

Positive fixtures use source-faithful 600s observe mono window, first/mid/final
observe + drain samples, capture brackets, and harness qualification 120..720
independently of mono.

### Provenance and path binding

- Reads only files inside the bound run directory:
  `samples.cohort.jsonl`, `samples.jsonl` (caller-provided),
  `samples.qualification.jsonl`, `metadata.json` (via analyze_run)
- Never opens a recorded native path; never basename-only or relative refs
- Recorded sidecar claims must be absolute (POSIX or `X:\...`), end with
  `<run_dir.name>/samples.cohort.jsonl`, and agree exactly (separator-normalized)
  across header + sample + qualification cohort refs
- Qualification requires **both** `observe-start` and `observe-end` boundary
  rows (phase / `phase_tag`), each with settings + cohort ref + finite harness
  `elapsed_s` with `end > start`
- Qualification cohort refs also require
  `qualification_elapsed_s_is_harness_not_cohort_mono=true` and a string
  `phase_tag` (harness elapsed is not cohort mono)
- Terminal `observe_end_elapsed_s` must equal qualification observe-end
  harness elapsed (self-consistent harness clock; not mono)
- Cohort-bearing sample harness `elapsed_s` must not precede observe-start
- Sample `cohort.cursor` must be a uint in the set of emitted batch
  `next_cursor` values (plus 0), and `max(sample cursors) == final batch HWM`

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
- `ended=True` on a cohort-bearing profile row → `sample_slot_ended`
- Empty / disappeared profile on a cohort-bearing sample →
  `sample_slot_disappeared` (one good sample is not whole-population lifetime)
- Every cohort-bearing sample must expose the same slot set; declared gate
  population slot ids must be a subset of each sample's profile
- Interior `observe`/`drain` samples without a present cohort ref →
  `cohort_sample_ref_missing_interior`
- Header `focused-one.slots:[0]` means **fixture ordinal 0**, resolved through
  qualification `responsiveness_slot_id` (nonzero FNV), not `EventId.slot_id=0`

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
- Preserve counted loss vs retained receipts; pending0/finalized≠success;
  per-slot p99 (no fleet pool-away)

### Meta / settings agreement

Requires **meta and every qualification settings object**:

- `meta.responsiveness_profile is True`
- `meta.responsiveness_fine is True` (no meta-True override of silent qual False)
- every qual settings: `responsiveness_profile_enabled is True`
- every qual settings: `responsiveness_fine_enabled is True`
- critical settings keys agree across **all** qualification boundary rows
  (`qualification_settings_disagree` otherwise)
- Frontend and N must agree across meta/header/settings when present

### p99 math (frozen)

`p99_ns` SHA-256 (function source, trailing newline normalized):

`ade46732007625b4ddd83f7895e38350646c7799879ed8f9030c8ceb41a9d8f3`

Root independent check (640 deterministic cases, seed 274) validated this
exact hash. Verified **unchanged** after endpoint/clock harden. Compare
durations in **nanoseconds** to inclusive integer ms upper edges (do not floor
1.1 ms into 1 ms). Coarse bounds `(5,10,20,25,40,50,100,250,500,1000)`; fine
`1..100`. Overflow bucket has lower bound only (`upper_ms=None`). Per-slot fine
p99 drives the ≤100 ms verdict; a slow slot cannot be pooled away in a fleet
average. Target verdict:

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

Prior codes retained. Additive / tightened for this card:

`unsupported_frontend`, `input_population_ordinal_mismatch`,
`input_population_policy_mismatch`, `input_population_duplicate_ordinal`,
`input_population_mismatch`, `render_policy_meta_qualification_disagree`,
`tui_render_policy_invalid`, `unsupported_render_policy`,
`sample_clock_missing`, `sample_clock_malformed`, `sample_clock_domain_mismatch`,
`sample_clock_bracket_*`, `sample_elapsed_mono_malformed`,
`sample_read_outside_sample_bracket`, `sample_elapsed_outside_sample_bracket`,
`sample_mono_time_regression`, `capture_bracket_missing` / `_malformed` /
`_inverted` / `_unset`, `capture_outside_sample_bracket`,
`observe_samples_missing`, `observe_boundary_samples_missing`,
`observe_first_before_start`, `observe_final_missing_or_misaligned`,
`observe_sample_outside_window`, `drain_sample_before_window`,
`drain_sample_past_tail`, `drain_before_observe_end`, …

## Verification commands and counts

```
cd /Users/acfrazier/experiments/274bot/.worktrees/latency-cohort-reader
python3 -m unittest discover -s docs/memory -p 'test_cohort_reader.py' -v
# → 52 tests, 0 fail, 0 err

python3 -m unittest discover -s docs/memory -p 'test_reference_metrics.py' -v
# → 100 tests, 0 fail, 0 err

python3 -m unittest discover -s docs/memory -p 'test_matched_evidence_adapter.py' -v
# → 46 tests, 0 fail, 0 err, 1 skip

python3 -m unittest discover -s docs/memory -p 'test_instrumentation_overhead.py' -v
# → 29 tests, 0 fail, 0 err, 1 skip
```

p99 freeze hash verified unchanged:
`ade46732007625b4ddd83f7895e38350646c7799879ed8f9030c8ceb41a9d8f3`

cohort_reader SHA-256 after this card:
`573cf29ab7397254bbf8f925d94f89c574848fe4b4b4ec4d5450d3a9008af20a`

### Root false-meet probes (now fail-closed)

From `cohort-reader-ec5dda2-root-probes.json` remaining false meets:

1. headless frontend + Panel events → `unsupported_frontend`
2. focused-one `slots:[1]` → `input_population_ordinal_mismatch`
3. `EventId.sequence=2**64` → `malformed_event_identity`
4. missing `responsiveness_clock` → `sample_clock_missing`

Prior R2 six + interior ref still fail-closed. Control happy path meets with
source-faithful clocks/observe publication.

### R2 false-meet probes (retained)

1. qual `responsiveness_fine_enabled=False` while meta fine true →
   `qualification_fine_disabled`
2. missing observe-end qualification row →
   `qualification_observe_end_missing`
3. cohort-bearing sample `ended=True` → `sample_slot_ended`
4. empty-profile disappearance on cohort-bearing sample →
   `sample_slot_disappeared`
5. forged `sample.cohort.cursor=99999` → `sample_cursor_mismatch`
6. observe-end `responsiveness_profile_enabled=False` while start true →
   `qualification_settings_disagree`
7. interior observe without cohort ref →
   `cohort_sample_ref_missing_interior`

## Mandatory scenario coverage (tests)

Full synthetic N=16 archived RUN + `analyze_run` + CLI `--require`;
legacy archive without sidecar; tail completion; start-at-END / pre-start;
late completion beyond tail; out-of-order start IDs valid; cancel; overflow;
counter-only loss; trailing batch after terminal; missing terminal;
finalized-unavailable pending0; duplicate identities; cursor gap/stale;
missing generation; empty profile disappearance; no input events → population
incomplete; basename-only and relative paths rejected; profile/fine required;
slow slot not pooled; header-only; producers_joined; final batch complete;
focused-one ordinal0→FNV id; qualification shape is `slots` not
`qualification_slot_rows` / no generation on qual slots; FNV matches Rust;
bool rejected as uint; pre-arm seed gen ignored; in-cohort generation reset;
quiet-inner-span not selected; malformed surface and fuzzed scalar types →
unavailable without exception; N=16 single-slot cannot meet; ns boundary /
exact 100ms / 100ms+1ns overflow; all six R2 provenance probes + interior ref;
**headless unsupported; wrong ordinal; u64 overflow sequence; missing/inverted/
domain-mismatched clock; missing capture; final observe misaligned; pins_focus
fixed-one + focused-plus-background meet; TUI endpoint distinct from Panel.**

## Limitations (honest)

- No native/live publisher run in this isolated checkout; fixtures are
  source-faithful synthetic envelopes plus path-binding of Windows-style
  absolute claims onto the bound archive leaf
- Untracked diagnostic fixture symlinks (if present) are local test reuse only
  and must not be committed as reader artifacts
- Not a native latency pass, matched-pair proof, or campaign acceptance
