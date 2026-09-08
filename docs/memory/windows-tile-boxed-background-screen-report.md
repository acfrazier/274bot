# Native boxed-tile N16 focused-plus-background ABBA screen

Status: independently recomputed screening evidence only. This report grants no retention, provisional acceptance, final acceptance, absolute RSS/CPU budget pass, lifecycle acceptance, or visual proof.

## Inputs and verification

Only these four immutable directories were read:

| cell | archive tar SHA-256 |
|---|---|
| baseline-forward | `06972a6d54e2294ab224b4f25348b78bc5b25c0db7a894377ce167ec6db2317f` |
| candidate-forward | `29f1bccab7b2056ed2b6502405920478fcee729fbec8f54c56ba83e5ed6d0650` |
| candidate-reverse | `dcae9b20593b8d8cdbc92e84eb5490463eee28439e0ee03074b09a4c70a040c8` |
| baseline-reverse | `10230c08b6c4faf37340e006d62555c6cb2b34cc8045e5f64350b415e6164f28` |

Each archive's 22 manifest-listed files matched its recorded length and SHA-256. The JSON table records the complete input hash manifest. Original Windows paths remain literal evidence and were not reopened on macOS. All four native-bound records are `status=bound`, `binding_ok=true`, `qualified=true`, exit 0, with Intel Vulkan startup. Exact frozen build/source/binary provenance is retained; checkout labels are not used as binary identity. Baseline is host926/clientabb/binary e2deb; candidate is hostfb/clientfd/binary a9b581.

All cells are panel N=16 active, 30 s warmup, 120 s requested observation, focused-plus-background, with allocation counting, diagnostic sidecar, owner census and nav PNG disabled. Scheduling, render, GPU-completion, responsiveness-fine and failure capture are enabled identically. Original qualification was independently rerun with `qualify_control` (`counting=False`, `diagnostics=False`); every boundary has 16 running/error-free slots and selected samples have ready=active=16.

## Whole observations

| cell | elapsed endpoints (s) | duration (s) | samples | RSS median/max MiB | peak-field max MiB | process CPU cores | GPU tracked median/max MiB |
|---|---:|---:|---:|---:|---:|---:|---:|
| baseline forward | 103.2073198..221.7025809 | 118.4952611 | 119 | 2172.9805 / 2484.7852 | 2486.5703 | 0.6357058 | 142.8885 / 143.0433 |
| candidate forward | 137.3414382..255.9378206 | 118.5963824 | 119 | 1398.5625 / 1659.1016 | 1731.1406 | 0.6056519 | 142.8182 / 142.9449 |
| candidate reverse | 112.2408380..231.9926042 | 119.7517662 | 120 | 1533.6484 / 1718.8633 | 1722.3555 | 0.6199021 | 143.4410 / 143.4885 |
| baseline reverse | 103.2066542..221.7968501 | 118.5901959 | 119 | 2139.7813 / 2490.9063 | 2494.6914 | 0.6405989 | 143.2087 / 143.5669 |

CPU is `(last process_cpu_user_s + system_s - first process_cpu_user_s - system_s) / (last elapsed_s - first elapsed_s)`. Current RSS median/max, peak RSS field, private commit and GPU tracking are separate and never subtracted.

## Pairwise and common ranges

The independent forward AB intersection is `137.3414382..221.7025809` (84.3611427 s): baseline selected `137.3841274..221.7025809` (85 samples), candidate `137.3414382..220.7727661` (84 samples). The independent reverse BA intersection is `112.2408380..221.7968501` (109.5560121 s), not the all-four range: candidate selected `112.2408380..220.9088534` (109 samples), baseline `113.2260029..221.7968501` (109 samples). The all-four intersection is `137.3414382..221.7025809` (84.3611427 s). Exact per-side durations, counts and metrics are in the JSON table under `ranges` and `forward_AB`/`reverse_BA`.

Candidate-minus-reference RSS median deltas are -424.8457 MiB (AB) and -507.8984 MiB (independent BA). Whole-observation deltas are -774.4180 MiB (forward) and -606.1328 MiB (reverse). This difference, plus readiness/startup-age offsets and falling RSS, is a material confounder. Two repeats are not a noise distribution.

## Reviewed per-slot gates

For every pairwise and all-four selected range, the script invokes the reviewed readers in `reference_metrics.py`: `evaluate_scheduling`, `evaluate_scheduling_slots`, `evaluate_decode`, `evaluate_input`, and `evaluate_gpu`. The JSON stores per-slot status/reason, counter deltas, histogram bucket counts/sums, p99 bounds, endpoint coverage, roles and cadence fields; raw histograms remain in the immutable inputs.

Scheduling is explicitly unavailable as a per-slot acceptance proof when only process-wide groups are available. The stricter per-slot reader is unavailable on baseline-forward in AB and all-four (`no_complete_qualified_per_slot`) but available on the other selected cells/windows. Decode is available diagnostically, with incomplete full decode/transition coverage retained where the reader reports it. Input is unavailable (`no_slot_with_available_input_p99`): no valid input sample distribution is established. GPU completion is available diagnostically where the reader returns it; it is callback completion after prior submit, not physical presentation/scanout, and does not establish rendered-image proof or frame cadence acceptance. Endpoint coverage limits and histogram resolution are not promoted to silent passes.

Resource provenance is not blanket-missing: managed process/cache resource evidence is `available` in the native records. Instrumentation-overhead calibration is unavailable, and renderer/cache metadata required by the resource acceptance gate is incomplete; those are distinct gates. Startup/render inspection in the table parses the actual `panel-render-attribution` startup JSON from each original `raw-run-01/run.log` (and separately walks managed metadata/receipts). All four runs report requested 1120x580, actual 1680x870, scale factor 1.5, Intel(R) Graphics requested/actual adapter, Vulkan backend, visible=true, focused=true, minimized=false, offscreen_target=false, and present_mode=Fifo; no mismatch was found. These are startup/render records, not rendered-image proof, and GPU attribution remains callback timing rather than scanout.

## Classification and bounded next action

- Intended RSS reduction: **inconclusive**. Both independent pair windows are lower for the candidate, but readiness/startup age and resident decay prevent attribution to the boxed fields.
- CPU 5% non-regression: **inconclusive**, not a pass. CPU deltas are descriptive and clean overhead calibration plus repeated variation evidence is insufficient.
- p99 2 ms non-regression: **unavailable**. Complete matched fine endpoint coverage and a paired proof are not established.
- Next action: run the focused-one regression pair with the same frozen provenance and controls. Permit at most one longer confirmation stage only if that pair supports retention. Do not claim absolute resource budgets or provisional/final acceptance.

Reproduce with:

`python3 docs/memory/windows-tile-boxed-background-screen-analysis.py`

Machine-readable results, archive/source/input hashes, exact pair/common ranges, startup/render inspection and reviewed per-slot outputs are in `docs/memory/windows-tile-boxed-background-screen-table.json`.