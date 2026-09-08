# Focused-one GPU completion coverage audit

Status: bounded, read-only audit of the six completed short GPU archives. This is
not a performance, visual, lifecycle, latency, or final acceptance result. No
raw samples were changed and no new samples were manufactured.

## Evidence inspected

The one focused-one baseline/candidate AB pair and the two background pairs were
independently recomputed with the existing readers:

- `baseline-focused-one-nativecheck-20260908-0103`
- `candidate-focused-one-nativecheck-20260908-0103`
- `baseline-focused-plus-background-nativecheck-20260908-0103`
- `candidate-focused-plus-background-nativecheck-20260908-0103`
- `baseline-focused-plus-background-reverse-20260908-0116`
- `candidate-focused-plus-background-reverse-20260908-0116`

`python3 docs/memory/windows-tile-boxed-focused-screen-analysis.py` completed
with both focused archive manifests valid and both cells qualified 16/16.
`python3 docs/memory/windows-tile-boxed-background-screen-analysis.py` completed
with all four background archive manifests valid. The archived native-bound
records are `bound`, exit 0, and qualified; these records remain artifact
provenance, not a local Windows-path re-open.

All six cells use panel N=16 active, Intel Vulkan, GPU completion and render
profiles enabled, and `render_policy=focused-one` for the focused cells. The
focused-one metadata declares `single_renderer=false` (the policy, rather than
this metadata boolean, is the source of the one-renderer contract), and the
background cells contain the same 16-slot fleet under the focused-plus-
background workload.

## Finding: real aggregation defect, not missing renderer proof

`evaluate_gpu_completion_intervals` in `docs/memory/reference_metrics.py:2067`
through `2247` currently treats every stable slot in `renderer_profile` as an
expected GPU-completion slot. It first pairs the complete 16-slot identity set
(`:2099-2114`), then rejects a slot when `renderer_present`/backend/completion
is absent (`:2130-2131`). That is correct for background mode, where every slot
is required to render at 1 fps, but it is too strict for focused-one mode.

The focused archives make the mismatch observable rather than hypothetical:

| cell | expected slot rows | actual focused GPU | actual expected headless | current GPU result |
|---|---:|---:|---:|---|
| baseline focused-one | 16 | 1 | 15 | unavailable; 15 `gpu_backend_or_completion_unavailable` |
| candidate focused-one | 16 | 1 | 15 | unavailable; 15 `gpu_backend_or_completion_unavailable` |

At the selected observation boundary in both focused cells, the raw rows have
exactly one row with `renderer_present=true`, `backend_kind=gpu`, `draw=true`,
and `full_rate=true`; the other 15 have `renderer_present=false`,
`backend_kind=null`, `draw=false`, and `full_rate=false`. The focused renderer
is `(generation=17)` in both runs (with run-specific slot IDs), and all 16
rows are present through generation 32 at the inspected end boundary. The
qualification boundaries independently show 16 Running slots with no errors.
This is stable role/identity evidence for one GPU plus 15 deliberately
headless slots, not evidence that fifteen required renderers disappeared.

The current focused result therefore reflects a genuine aggregation defect in
policy interpretation. It is not safe to fix it by dropping every row with no
renderer: that would hide a missing required focused renderer and would also
weaken background coverage. The background recomputation is the control: all
four background cells produce `available`, with 16/16 available slots and the
roles split `1 focused_full_rate + 15 background`.

The focused renderer itself has valid diagnostic completion data in both
focused cells: `stable_completed_n_delta` is 5,815 (baseline) and 5,808
(candidate), with zero dropped/lost completions, zero pending at both
boundaries, and p99 upper bound 25 ms. This remains callback-delivery timing,
not hardware presentation, scanout, or rendered-image proof.

## Required fail-closed policy correction (proposal only)

The adapter should derive an explicit expected-role contract from the declared
render policy and preserve the full stable slot identity set:

| declared mode | required role set | non-rendered rows |
|---|---|---|
| `focused-one` | exactly 1 `focused_full_rate` GPU row | exactly 15 `expected_headless` rows, each identity-stable and explicitly declared headless |
| `focused-plus-background` | 1 `focused_full_rate` GPU row + 15 `background` GPU rows | none |

For focused-one, a row may be classified `expected_headless` only when the
policy declaration and row evidence agree: stable slot ID/generation mapping,
`renderer_present=false`, `backend_kind` absent/null, `draw=false`,
`full_rate=false`, and the observed headless completion object is consistent
with that role: `gpu_completion` is present with `enabled=true`,
`stable_completed_n=0`, `pending_n=0`, and the complete counter-key shape, but
with `backend_kind=null`, `draw=false`, and `full_rate=false` and no completion
activity. The
adapter must fail closed for any unexpected renderer, two or zero focused GPU
rows, a missing required focused row, a role transition anywhere in the
selected span,
changed slot set, or a headless row that has draw/full-rate/backend/completion
signals inconsistent with the declaration. The focused GPU row must still pass
all existing completion, pending, loss, counter, coverage, histogram, and
callback-versus-scanout checks.

The role contract must be evaluated separately for each stable mode epoch and
must not infer focus from slot ordinal or from the presence of the first row.
Stability evidence must inspect interior rows throughout the selected span, not
only its endpoints, so an away-and-back focus change cannot pass as stable.
Focus-switch, reset, attach/detach, and pending-boundary cases must remain
unavailable unless the selected endpoints are in one stable declared epoch;
nonzero endpoint pending remains unavailable. A focused renderer disappearing
must fail, while an intentionally headless slot remaining non-rendered must
pass its role check. Focused-plus-background mode must continue to require all
16 GPU completion rows: one `focused_full_rate` row and 15 `background` rows.

A useful regression fixture contract is therefore:

1. Positive focused-one: 16 stable identities, exactly one GPU/full-rate row,
   15 explicit headless rows with the observed zero-counter headless completion
   shape, complete focused completion counters and zero
   boundary pending => focused GPU gate available when the focused row meets
   its target.
2. Negative focused-one: remove or disable the focused GPU row, or mark a
   headless row `draw=true`, `full_rate=true`, or `backend_kind=gpu` =>
   unavailable; never pass by filtering that row.
3. Positive background: exactly one focused_full_rate row plus 15 stable
   background GPU rows => available only when
   every row has complete cadence/completion evidence.
4. Negative background: one missing backend/completion row => unavailable.
5. Negative transitions: change a generation/slot identity, switch focused
   identity inside the selected span, reset a counter, or leave pending at an
   endpoint => unavailable.

These fixtures should test the policy-aware adapter, not be used as product
performance evidence.

## Remaining latency-reader limits

The focused screen's current diagnostic results separate the issues:

- Decode is `available` for all 16 slots in the reviewed focused windows, but
  this is a contained single-run reader result. It does not establish the
  paired <=2 ms fine-p99 non-regression contract; the existing responsiveness
  contract explicitly keeps `paired_fine_p99_margin` diagnostic and unavailable
  until matched-run artifact/slot/endpoint binding exists.
- The process-wide scheduling reader is unavailable with
  `process_wide_only_cannot_satisfy_per_slot`; this is an accounting-shape
  limitation, not evidence that the client has no scheduling activity. In
  contrast, `evaluate_scheduling_slots` is `available`, `target_verdict=meet`,
  and has 16/16 available slots for both focused baseline and candidate (for
  both `root` and `common_window`). That availability is still not the paired
  fine-p99 contract: coverage and margin/identity-binding limits must remain
  separate from reader availability, and no producer fields are inferred.
- Input is unavailable on both focused sides with
  `no_slot_with_available_input_p99`. The raw focused cells contain no actual
  input events sufficient for a p99 bound; no input samples are inferred from
  clocks, decode, or draw activity.
- Managed process/cache resource accounting is available in the native-bound
  records. Instrumentation-overhead calibration and the standalone
  missing-resource-provenance acceptance gate remain unavailable. Those are
  distinct resource/calibration gaps, not substitutes for missing latency
  observations.

A minimal reader correction should therefore be limited to policy-aware GPU
role selection plus explicit role validation as above. It must not fabricate
fine-decode, scheduling, or input rows. Separately, a future responsiveness
reader change would need to publish/consume actual per-slot scheduling and
input capture brackets/counters (with identity and contained-window checks),
then retain the existing fail-closed histogram conservation and pending/
boundary rules. Matched baseline/candidate artifacts must be bound before
turning fine-p99 arithmetic into a <=2 ms verdict.

## Conclusion

The focused GPU `no_complete_qualified_stable_interval` result is currently
caused by aggregating 15 policy-expected headless rows as if they were required
renderers. The evidence is sufficient to specify a narrow policy-aware,
fail-closed correction, but no correction is implemented here and no focused
GPU acceptance is claimed. Even after that correction, overall p99/input
acceptance remains unavailable until matched fine-latency evidence and real
input captures exist; calibration/resource accounting does not close those
gaps.
