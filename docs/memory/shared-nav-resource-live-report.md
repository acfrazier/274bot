# Shared navigation N16 clean resource screen

The repeated short screen supports lower candidate RSS with CPU inside the predeclared 5% empirical margin. This is not accepted campaign savings: longer confirmation, latency, actual backend evidence, target hardware and lifecycle remain pending.

## Evidence and protocol

Batch: `diagnostics/shared-nav-resource-screen-20260907T080827Z`; canonical receipts and derived bindings listed in `batch.json`, full fresh analysis in `resource-screen-analysis.json`. Frozen matched manifest: `diagnostics/matched-instrumented-build-20260907T054019Z/build-manifest.json`. Reference TUI SHA `95d7d4d40fab47200ada6f4b303019c3cf700e0ad110e211352b955b63c5ac74`; candidate `b94bef2b437b649418dd3285f657ff77838e16d0696ceff8146aa38f37b27340`.

Predeclared R/C/C/R, real TUI, N16 active sustain, 30s warmup, 120s native observation, existing normal teardown, all hot profiles/counting/verbose diagnostics disabled, real PTY input probes, libproc 1Hz. Same fixtures, settings, server identity, cache hashes and helper configuration independently bound. All four completed exit 0, workload qualified, native qualification and continuous six-role accounting available. All 16 slots progressed in each run. Quiet-before files record no campaign worker running/ready/review at each launch; unrelated user OS activity remained, with no machine-idle claim. Host resource values come from Rust host samples, not controller/helper RSS; helper series are separately retained.

The earlier predeclared `shared-nav-resource-screen-20260907T075803Z` first spec failed preflight because `kind=resource_screen` is invalid; no frontend launched. Its report and batch ledger are preserved. The new explicit batch corrects only the kind to `matched`; no retry overwrote a failed cell.

## Results

| Cell | Raw run | Median RSS MiB | Max sampled RSS MiB | CPU cores | Client loops/slot/s | Minimum steal gain |
|---|---|---:|---:|---:|---:|---:|
| reference_a | 20260907T080839Z_tui_n16_active | 944.500 | 960.766 | 0.224202 | 37.129 | 4 |
| candidate_a | 20260907T081407Z_tui_n16_active | 907.734 | 922.547 | 0.226605 | 37.100 | 4 |
| candidate_b | 20260907T081927Z_tui_n16_active | 893.500 | 930.516 | 0.228991 | 37.118 | 4 |
| reference_b | 20260907T082506Z_tui_n16_active | 963.844 | 974.469 | 0.223014 | 37.094 | 3 |

Reference median RSS range 944.500–963.844 MiB; candidate 893.500–907.734 MiB. Minimum observed reduction 36.766 MiB exceeds the largest within-role spread 19.344 MiB. Pairwise candidate-minus-reference differences are -36.766 and -70.344 MiB. These are observed replicate ranges, not confidence intervals or a universal savings estimate.

Conservative candidate-max/reference-min CPU ratio 1.026804 (+2.6804%); reference spread 0.5328%, candidate 1.0530%. Reviewed reader returns `screen_available`, `candidate_resource_screen_supported=true`, `accepted_rss_saving=false`, `final_acceptance=false`. No profiler/helper subtraction is applied. The earlier independently rebound OFF/ON/ON/OFF experiment still supplies profile-choice evidence, not an extrapolated overhead correction; profiles ON added 13.8–15.6% CPU in that experiment.

All runs remain above the N16 512 MiB RSS target and below the 40 loops/s cadence target on this Mac; CPU remains below 0.5 cores. This Mac is not the specified modest Linux reference device. Profile-off records do not identify actual renderer backend and cannot prove latency; requested policy and native settings are matched. Existing full-pair acceptance remains unavailable.

## Next bounded work

Proceed to longer unprofiled confirmation and profiled latency companions using the same frozen matched binaries. A 600s companion covers the existing 480s N16 focus rotation; verify actual per-slot input samples rather than assuming coverage. Preserve remaining absolute-target, renderer/backend, panel, Linux resource-limit, lifecycle, and whole-branch Grok-4.6 requirements. This result justifies continuing shared-navigation validation, not ending the memory campaign.
