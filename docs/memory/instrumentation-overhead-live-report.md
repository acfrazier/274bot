# N16 instrumentation overhead screen — 2026-09-07

All four predeclared OFF/ON/ON/OFF cells completed normally and independently passed workload, native boundary, cache, and continuous process binding. The same frozen candidate binary and requested UI workload were used throughout. This is an instrumentation screen, not a control/candidate optimization comparison.

Evidence: `diagnostics/instrumentation-overhead-20260907T064437Z/` contains the declared specs, batch ledger, quiet checks, immutable cell receipts/raw process series, bound-side results, `cell-summary.json`, and independently recomputed `overhead-analysis.json`. The shared sidecars were frozen before the first cell; actual process identity was rechecked by each runner. No campaign builds, tests or review workers ran during observation. Unrelated OS/user activity was present and is documented in preparation-os-load.json; this was not an idle-machine claim.

| Cell | Host CPU cores | Median RSS MiB | Sampled max RSS MiB | Client loops/slot/s | Probe writes |
| --- | ---: | ---: | ---: | ---: | ---: |
| off_a | 0.229341 | 883.344 | 911.500 | 37.097 | 113 |
| on_a | 0.260949 | 919.031 | 931.156 | 37.083 | 112 |
| on_b | 0.262001 | 895.453 | 908.188 | 37.103 | 112 |
| off_b | 0.226550 | 927.078 | 953.094 | 37.139 | 112 |

Every cell qualified all 16 active bots, with positive steal gains in every slot (minimum gains 4,4,3,5 respectively). All launcher/collector exits were zero; no probe error was reported. The observation window was nominally 120s after 30s warmup; ordinary host sampled spans were about 118.65s, distinct from native wall and helper acquisition envelopes. Normal 60s teardown completed.

The OFF CPU range is 0.226550–0.229341 cores; ON is 0.260949–0.262001. Replicated spreads are 1.232% OFF and 0.403% ON. The entire observed ON/OFF ratio range, 1.137819–1.156480, exceeds the 5% empirical screen margin: profiling cost was 13.8–15.6% in this short screen. These are observed ranges, not confidence intervals or a production regression caused by the shared-nav candidate. The result argues for profile-off CPU comparisons with separate latency companions; profile-on CPU is not treated as an unperturbed deployment number.

RSS ranges overlap (OFF 883.344–927.078 MiB; ON 895.453–919.031 MiB), so this screen does not establish a profile-related RSS increase or reduction. All measured medians exceed the 512 MiB N16 target on this development Mac; target Linux/resource-limited validation is still required. CPU remains below 0.5 cores here, while ~37.1 client loops/s misses the >=40 target. ON scheduling histograms remain available and flag that miss. This is not final resource or simulation acceptance.

| Separately accounted role | OFF CPU seconds range | ON CPU seconds range | OFF median RSS MiB range | ON median RSS MiB range |
| --- | ---: | ---: | ---: | ---: |
| collector | 0.126440–0.133020 | 0.124627–0.125944 | 14.938–15.141 | 14.906–14.938 |
| controller | 0.515106–0.517200 | 0.493263–0.498535 | 28.531–29.016 | 29.391–35.062 |
| game_server | 4.119163–4.427457 | 3.900166–3.959916 | 1055.062–1055.406 | 1055.141–1055.156 |
| gateway | 0.211166–0.220021 | 0.209275–0.211742 | 244.688–244.938 | 244.688–244.688 |
| launcher | 0.601849–0.615158 | 0.615096–0.649605 | 178.906–179.391 | 177.375–177.641 |
| server_supervisor | 0.000759–0.000899 | 0.000943–0.001033 | 49.172–49.172 | 49.172–49.172 |

Role CPU seconds use each independently validated enclosing acquisition interval; full intervals and endpoint excesses are in the raw/bound evidence. Controller is Python run_managed_cell, not the Rust host. Helper/server resources are never subtracted from host RSS/CPU. Continuous helper accounting measures their resource use, not sampler causal perturbation of the host. Unsupported macOS pressure remains unavailable.

The real PTY input probe worked: ON cells contain native input/draw samples. However each ON cell has only one slot with a qualified input window, four slots rejected for publisher_clock_bracket_incomplete, and eleven with no_input_samples. The aggregate input target is unproven. In ON_A the accepted slot has 16 matched starts/completions, no lost/canceled/dropped/pending inputs, and a fine p99 upper bound of 1ms. This does not waive rejected slots or prove all injected inputs. Focused-period and publisher-cut coverage diagnosis is pending. OFF cells intentionally lack native latency histograms, so no p99 instrumentation-overhead claim follows.

The reviewed reader returns resource_deltas_available=true, but instrumentation_overhead_measured=false, pair_eligible=false and no accepted RSS saving. Remaining latency/sampler perturbation limitations stay explicit. No cell was retried. These results preserve the earlier failed N1 pipeline cell and successful N1 qualification; they do not upgrade those older artifacts into paired performance evidence.
