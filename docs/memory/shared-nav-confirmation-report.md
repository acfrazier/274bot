# Shared navigation longer confirmation and latency companions

The longer unprofiled resource pair supports the earlier short-screen direction: candidate median host RSS is 77.141 MiB lower, with CPU 2.195% higher. The bounded stage does **not** establish latency non-regression or accepted campaign savings. Input has incomplete boundary accounting, one overflow result, and three available ordinal comparisons above the 2ms diagnostic bound margin. No further live retry is declared.

## Evidence

Batch `diagnostics/shared-nav-confirmation-20260907T083318Z/batch.json` predeclared one longer stage in order reference resource, candidate resource, candidate latency, reference latency. Each used real N16 TUI active sustain, 120s warmup after readiness, 600s observation, existing normal teardown, real PTY input probes, libproc 1Hz and continuous six-role accounting. Resources have hot profiles OFF; latency companions both have scheduling/render/responsiveness/fine ON, GPU-completion OFF. No profiler/helper subtraction. Each launch has a quiet-before snapshot with no campaign workers active; unrelated user OS activity remained. Root performed brief read-only progress monitoring; no machine-idle claim.

All four completed exit 0 and independently re-bound workload/native/cache/process evidence. All 16 slots made positive steal progress. Frozen manifest remains `diagnostics/matched-instrumented-build-20260907T054019Z/build-manifest.json`, reference TUI SHA `95d7d4d40fab47200ada6f4b303019c3cf700e0ad110e211352b955b63c5ac74`, candidate `b94bef2b437b649418dd3285f657ff77838e16d0696ceff8146aa38f37b27340`. The short-screen report was independently approved by Grok4.5 task `t_0e62055e` before this stage launched.

Reproduce the derived diagnostic summary with `python3 docs/memory/diagnostics/shared-nav-confirmation-20260907T083318Z/analyze_confirmation.py`. It re-reads all original receipts through `bind_side`, requires workload/native/managed qualification, checks like-profile match keys, native match keys, distinct runs and helper identity continuity, and maps latency by native ordinal. Output `confirmation-analysis.json` retains resources for every owner and per-ordinal statuses. This batch-specific script is not a new acceptance gate; existing paired fine helper remains unavailable and reports arithmetic only.

## Observed resource result

| Cell | Raw run | Median host RSS MiB | Max sampled host RSS MiB | CPU cores | Loops/slot/s | Minimum steal gain |
|---|---|---:|---:|---:|---:|---:|
| Reference resource | 20260907T084356Z_tui_n16_active | 989.250 | 1040.219 | 0.191005 | 37.079 | 46 |
| Candidate resource | 20260907T085856Z_tui_n16_active | 912.109 | 945.188 | 0.195198 | 37.097 | 47 |
| Candidate latency, profiles ON | 20260907T091408Z_tui_n16_active | 908.688 | 925.234 | 0.226881 | 37.073 | 51 |
| Reference latency, profiles ON | 20260907T092908Z_tui_n16_active | 963.484 | 1001.703 | 0.225980 | 37.146 | 49 |

The unprofiled median difference is 80,887,808 bytes (77.140625 MiB), CPU +2.194991%. Host sampled counter spans are 598.859s reference and 599.514s candidate within the native 600s observations. This one run per role cannot estimate repeat variability on its own; the prior independently reviewed short R/C/C/R had minimum gap 36.766 MiB exceeding its 19.344 MiB within-role spread. Do not pool unlike durations or profiled CPU with unprofiled CPU. Neither longer resource run meets the N16 512MiB or 40loops/s target on this Mac; both remain below 0.5 CPU cores. The Mac is not the specified modest Linux reference device.

## Latency and coverage

Gate top-level `available` does not mean all slots passed. Both runs have all 16 scheduling slots available, p99 interval upper bound 30ms, but all miss the measured cadence target. Decode is available/target-meeting for candidate14/16 and reference12/16; missing slots all report `boundary_pending_incomplete`. Ten ordinals have usable decode bounds in both runs; candidate fine upper minus reference fine lower is 1ms for nine and 2ms for ordinal13. These are diagnostic arithmetic, not a full paired gate pass.

Input is available/target-meeting for 13/16 slots in each run. Candidate ordinals5 and6 report `boundary_pending_incomplete`, ordinal9 reports `overflow_bucket`; reference ordinals5,6,7 report `boundary_pending_incomplete`. Twelve ordinals have usable paired input bounds. Candidate upper minus reference lower exceeds 2ms at ordinal0 (+4ms), ordinal2 (+13ms) and ordinal8 (+21ms); the other nine usable differences are <=2ms. Thus even the usable subset does not establish the declared margin. The overflow does not supply a finite precise percentile. Full focus rotation did not resolve complete boundary accounting, and PTY writes are not substituted for visible acknowledgments.

Grok-supervised bounded audit `t_da2f4634` is examining the raw boundary counters, clocks and overflow in `latency-boundary-confirmation-audit.md`. Its scope is diagnosis and a correction proposal only if a concrete bug is proven; no source changes, favorable window selection or live rerun are authorized by that task.

## Decision and remaining work

Retain the observed resource result; shared-navigation acceptance remains pending, with `accepted_rss_saving=false` and `final_acceptance=false`. This stage has reached its predeclared latency limitation, so investigate the named accounting/overflow evidence or park the candidate rather than repeat runs until favorable. Actual backend evidence for profile-off cells, metric-complete paired latency, absolute targets, Linux resource limits/reference hardware, panel modes, lifecycle/Stop and final whole-branch Grok4.6 remain required. No binary or production behavior changed during this stage.
