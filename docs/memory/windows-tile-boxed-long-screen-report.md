# Single longer focused-one N16 screen

Status: descriptive diagnostic only. `accepted_rss_saving`, `performance_acceptance`, and `final_acceptance` are false. Candidate recommendation: park the candidate for this focused-memory claim; do not add another longer, reverse, or short run.

## Scope and reproducibility

This report uses exactly the two predeclared archives from the single allowed longer stage:

- `diagnostics/windows-tile-boxed-long-20260908/baseline-focused-one-long-native-20260908-0250.tar.gz`
  - SHA-256 `b580284cd6ef1eb5766877e8d4543b853c0e6c55a4c8daa3612932856f5907cf`
- `diagnostics/windows-tile-boxed-long-20260908/candidate-focused-one-long-native-20260908-0250.tar.gz`
  - SHA-256 `f651fbda0fe19539512bf81cc2ad2b665aa686844c011af124c76bf50a3374ad`

Each extracted sibling has 22 manifest files. `python3 docs/memory/windows-tile-boxed-long-screen-analysis.py` independently hashes both tarballs and every manifest file, recomputes the metrics from `raw-run-01/samples.jsonl`, and writes `docs/memory/windows-tile-boxed-long-screen-table.json`. Both tar hashes and all 22-file manifest checks pass.

The archived native-bound records independently show exit code 0, `binding_ok=true`, `qualified=true`, no missing match keys, and the intended workload `active`, N=16, focused-one panel. `qualify_control` was rerun locally with `--no-write` semantics for both cells and returned qualified with no errors. Both qualification boundaries contain 16/16 Running slots without errors.

The comparison preserves recorded Windows paths as evidence; it does not reopen them or manufacture a local binding. Native provenance is retained exactly: original restored measurement runtime, controls `7a9eb03`, Intel(R) Graphics/Vulkan, focused visible geometry 1120x580 requested / 1680x870 actual at scale 1.5, Fifo, and one focused GPU renderer. The run logs identify the same Intel adapter/driver (`32.0.101.8991`) and Vulkan backend. The archive-backed side-by-side provenance is: baseline role binary SHA `e2deb1db…`, client commit `abb811bd…`, client-source SHA `ea640270…`, manifest build commit `9268890…`, and manifest-source aggregate `84054d9c…`; candidate role binary SHA `a9b581ba…`, client commit `fd956c91…`, client-source SHA `fad2e792…`, manifest build commit `fb3589ac…`, and manifest-source aggregate `189dac14…`. Thus binaries, client sources, and build commits differ as intended role inputs; they are not claimed identical. Shared fixture/catalog/cache, server identity/configuration, renderer settings, allocator, adapter/backend, geometry, scale, Fifo, and workload conditions are compared from archived `match_keys`/`run.log` evidence and retained in the long table rather than normalized or inferred.

## Elapsed windows and recomputed metrics

The whole observation is the `observe` phase only, not seed, warmup, or teardown. The requested common elapsed intersection is 124.7266300–702.9757121 s. Actual selected endpoints differ slightly by sample cadence: baseline 124.7266300–702.5677778 s (576 samples), candidate 125.3165882–702.9757121 s (576 samples). The selected durations are 577.8411478 s and 577.6591239 s. CPU is computed from matched user+system deltas over each selected elapsed span.

All deltas below are candidate minus baseline. RSS is current resident RSS; peak-field, private commit, and GPU tracking are separate fields and are not subtracted from RSS.

| Window | Baseline RSS median | Candidate RSS median | RSS delta | Baseline CPU | Candidate CPU | CPU delta |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Whole observation | 565.2148438 MiB | 569.2597656 MiB | +4.0449219 MiB (+0.716%) | 0.5188668 | 0.5110003 | -1.516% |
| Common intersection | 565.3652344 MiB | 568.7128906 MiB | +3.3476563 MiB (+0.592%) | 0.5187953 | 0.5097892 | -1.736% |
| Final 300 s | 512.0273438 MiB | 553.0820313 MiB | +41.0546875 MiB (+8.02%) | 0.5174851 | 0.4959294 | -4.169% |

The final-300 endpoints are baseline 403.3442026–702.5677778 s (299 samples) and candidate 403.6954652–702.9757121 s (299 samples). The predeclared 100 s bins are anchored at the common lower bound, with each side's actual contained sample endpoints retained in `windows-tile-boxed-long-screen-table.json`:

| Bin | Baseline RSS median | Candidate RSS median | Candidate minus baseline |
| --- | ---: | ---: | ---: |
| 124.7266300–224.7266300 s | 805.1835938 MiB | 772.3554688 MiB | -32.8281250 MiB |
| 224.7266300–324.7266300 s | 643.1132813 MiB | 649.0683594 MiB | +5.9550781 MiB |
| 324.7266300–424.7266300 s | 560.5058594 MiB | 563.5312500 MiB | +3.0253906 MiB |
| 424.7266300–524.7266300 s | 565.7187500 MiB | 548.2363281 MiB | -17.4824219 MiB |
| 524.7266300–624.7266300 s | 509.9648438 MiB | 568.1054688 MiB | +58.1406250 MiB |

The bins do not converge to a stable candidate RSS advantage. Both traces fall sharply from startup, and the late predeclared window reverses to a materially higher candidate median. The common RSS result is therefore not a focused retained-boxing win.

Other separately recomputed common-window deltas are: current RSS maximum -60.0078125 MiB, peak RSS field maximum -58.8164063 MiB, private-commit median -33.9023438 MiB, private-commit maximum -58.8554688 MiB, and GPU-tracked median/maximum +0.0749130 MiB. These are distinct measurements and do not establish an RSS saving.

## Per-slot reviewed readers and limits

The actual reviewed readers were rerun on the contained common-window rows for both sides and retained in compact form in `windows-tile-boxed-long-screen-table.json`:

- Scheduling per-slot reader: available for 16/16 slots.
- Process-wide scheduling reader: unavailable because it only has process-wide evidence and cannot satisfy the per-slot gate. This is not evidence that per-slot scheduling is unavailable.
- Fine decode reader: baseline available for 16/16 slots; candidate available for 12/16, with 4 unavailable (`boundary_pending_incomplete`). The top-level candidate status remains `available`, but that does not make every slot available.
- Input reader: available for 0/16 slots; aggregate unavailable (`no_slot_with_available_input_p99`); no input samples are invented.
- GPU reader: unavailable as a complete qualified stable interval (`no_complete_qualified_stable_interval`); one slot has available diagnostic data, while the focused mode intentionally has 15 headless slots. The aggregation does not apply the pending-audit headless-slot expectation to call the reader passed, and no reader fix is proposed or accepted here.

Conservation, lost/dropped/pending counters, per-slot identities, ordinals, and p99 bucket fields are preserved in `windows-tile-boxed-long-screen-table.json`. Histogram arrays are compacted to `*_count` and `*_bucket_count`; the raw arrays are not claimed to be present. GPU callback timing remains CPU callback-delivery evidence, not physical presentation, scanout, or visual proof. Managed resource accounting is `available`; that does not mean instrumentation-overhead calibration or the separate resource-provenance gate is complete. Calibration missing is not treated as managed-resource accounting missing.

## Decision and remaining gates

The earlier approved short background `e415ade` and focused `0501fed` screens justified exactly this one longer confirmation because startup/readiness decay could confound the short focused comparison. This longer pair does not prove a background steady-state benefit and does not prove a focused RSS benefit: it shows substantial decay, a +0.592% common RSS median, and a +8.02% final-300 RSS median for the candidate. CPU is descriptively within the 5% screen margin, but that does not rescue the memory result or close missing input/fine-gate evidence.

Park the candidate for this focused claim. Do not claim a final budget pass, accepted RSS saving, retention, lifecycle acceptance, or whole-campaign completion. Fine/input, calibration, visual/last-FBO, lifecycle, scaling, absolute budget, and final whole-branch Grok gates remain pending.
