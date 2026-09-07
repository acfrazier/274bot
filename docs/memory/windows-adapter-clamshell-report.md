# Windows explicit-adapter and clamshell diagnostics

Date: 2026-09-07

## Verdict

These five matched, diagnostic-only runs show a discrete closed-lid failure on the requested NVIDIA Vulkan path and recovery after reopening. NVIDIA open-lid measured 49.01265 client ticks/s and 49.01270 GPU callbacks/s; closed-lid measured 28.15629 and 28.28878, a 42.5530% client-tick reduction, with 124 callback intervals in the 250–500 ms bucket and scene/acquire trace p99 values of 423.0419/429.7271 ms. After reopening, NVIDIA returned to 48.89001 ticks/s and 48.89010 callbacks/s, with no 250–500 ms gaps and a 40 ms histogram p99 upper bound.

Intel did not reproduce the closed-lid cadence loss in this bounded comparison: open 48.92854 ticks/s / 48.93663 callbacks/s versus closed 48.90235 / 48.90177 (−0.0535% ticks/s). This is an observed association, not a universal clamshell fix, causal lock claim, physical scanout result, or final performance acceptance. Callback cadence is a diagnostic delivery rate, not physical FPS.

## Matched runs

| run | physical state | selected adapter/backend | client ticks/s | GPU callbacks/s | interval p99 upper bound | 250–500 ms gaps | visual evidence |
|---|---|---|---:|---:|---:|---:|---|
| open-nvidia | open | NVIDIA RTX 5060 / Vulkan | 49.01265 | 49.01270 | 40 ms | 0 | all 3 captures inspected |
| open-intel | open | Intel Graphics / Vulkan | 48.92854 | 48.93663 | 40 ms | 0 | all 3 inspected |
| closed-nvidia | lid closed | NVIDIA RTX 5060 / Vulkan | 28.15629 | 28.28878 | 500 ms | 124 | scene-ready only |
| closed-intel | lid closed | Intel Graphics / Vulkan | 48.90235 | 48.90177 | 40 ms | 0 | scene/bank/return inspected |
| reopened-nvidia | reopened | NVIDIA RTX 5060 / Vulkan | 48.89001 | 48.89010 | 40 ms | 0 | one scene-ready capture inspected |

The p99 values are histogram bucket upper bounds, not precise percentiles. The source histograms use upper bounds [10, 20, 25, 40, 50, 100, 250, 500, 1000, 2000] ms. All five runs completed and qualified with zero final GPU pending/lost/dropped faults in the bound comparison. The workload was the same active diagnostic shape (N=1, focused-one, GPU renderer, diagnostic sidecar, approximately 120 seconds), on the same frozen host binary `36825a9` and client `5ee9b6e`.

The qualification workload also recorded positive managed-work steal gains for every run: open-NVIDIA 9 (`livecc80_0`), open-Intel 8 (`live35e40_0`), closed-NVIDIA 13 (`live305c0_0`), closed-Intel 10 (`live23bc0_0`), and reopened-NVIDIA 21 (`live3d3c0_0`). These are workload diagnostics, not a cross-run savings or capacity claim.

## Trace and scheduling evidence

Open NVIDIA acquire p99 was 0.0384 ms and scene submit p99 0.1098 ms; closed NVIDIA was 429.7271 ms and 423.0419 ms respectively. Every closed-NVIDIA long Chrome submit (132 records) overlapped acquire, and 223/224 long scene submits overlapped acquire. These are emitted call-expression associations only: the trace includes warmup, captures, teardown, and encoder.finish argument evaluation; it does not prove lock causality and does not infer an unemitted tail. Intel closed had acquire/scene p99 0.0258/0.1489 ms; reopened NVIDIA 0.0392/0.1080 ms.

Per-run bound-slot gate results were: open-NVIDIA `available/meet`, p99 bucket 20–21 ms; open-Intel `available/meet`, 20–21 ms; closed-NVIDIA `available/miss`, 250–500 ms; closed-Intel `available/meet`, 20–21 ms; reopened-NVIDIA `available/meet`, 20–21 ms, against the 40 ms target. Decode was `available/meet` for each run (its p99 was bounded in the bindings); input status was `unavailable` for each run, so neither status is a final latency or lifecycle acceptance. Scheduling semantics are limited to the observation-window offline histogram. Responsiveness and fine-responsiveness profiles were present, but no final latency or lifecycle acceptance follows from them. No physical scanout measurement was made.

## Host and power conditions

The same active console session and requested Vulkan adapters were used. The user physically closed the lid before closed-NVIDIA and reopened it before reopened-NVIDIA. AC lid action was already `Do Nothing`; the only power-policy change was AC idle sleep from 1800 s to 0. AC1 and `Ultimate Performance` were verified during closed Intel. Battery settings were unchanged. This means open versus closed original power conditions cannot be treated as identical, and the single power change is not an isolated causal experiment.

The NVIDIA closed issue therefore remains an actionable diagnostic finding, not evidence for a universal clamshell policy. The Intel diagnostic selector is currently gated and is not ordinary production adapter selection.

## Resource evidence and limits

The independent bindings report managed resources as available, with attributable roles for game server, controller, bootstrap, launcher, and collector. Role medians (RSS bytes / enclosing-observation CPU seconds) were, in run order open-NVIDIA / open-Intel / closed-NVIDIA / closed-Intel / reopened-NVIDIA: game server `269969408/2.78125`, `476121088/3.078125`, `342482944/2.765625`, `478203904/2.671875`, `481462272/3.09375`; controller `29782016/0.46875`, `29483008/0.515625`, `29528064/0.359375`, `29544448/0.53125`, `29683712/0.46875`; bootstrap `70828032/0`, `69943296/0`, `70701056/0`, `70934528/0`, `70639616/0`; launcher `23928832/0`, `23228416/0`, `23642112/0`, `24002560/0`, `23080960/0`; collector `23898112/0.265625`, `23924736/0.109375`, `23859200/0.0625`, `24154112/0.109375`, `23924736/1.5625`. These are role observations, not a frontend budget or savings result.

The bindings also provide diagnostic frontend current-RSS medians and CPU-core samples, independently attributable to the five runs: open-NVIDIA `547762176` bytes / `0.2374704802` cores; open-Intel `663781376` / `0.1941546131`; closed-NVIDIA `546148352` / `0.1512792146`; closed-Intel `659632128` / `0.2033638689`; reopened-NVIDIA `527222784` / `0.2338515684`. These are current diagnostic samples, not `ru_maxrss`/peak values. The resources gate remains `unavailable` because of `missing_resource_provenance`; that limits acceptance/provenance, not the existence of these samples or the imported managed roles. Helper overhead is unmeasured. No accepted budget or savings claim is made.

## Provenance and preservation

The five replays from reader `901c2b0` are all `bound` and `qualified`; every binding has `final_acceptance_claim: false`, and before/after full-file hashes prove the original raw evidence remained unchanged. Reader archive SHA-256: `9798259155e03ecd041624802e0247ab21ce2005828538ecb567a745004a5654`. The first reader's historical NVIDIA rejection remains preserved in oldraw. The successful native test attempt exited 0 with one unavailable-local-fixture skip; the earlier PowerShell wrapper interruption was not substituted for that result.

Visual proof is intentionally bounded: root inspected all six open captures, only the closed-NVIDIA scene-ready capture, all three closed-Intel captures, and the one reopened-NVIDIA scene-ready capture (guard-thieving scene 2 / 4 food). No bank or return capture exists for reopened-NVIDIA, so no claim is made for those absent views.

## Next discriminator

Before changing production policy, isolate the NVIDIA closed-lid presentation/scheduling boundary in code and instrumentation: distinguish adapter selection, callback delivery, acquire/submit wait, and display/present behavior. Keep explicit adapter selection diagnostic-only until that discriminator is measured under a reviewable protocol. This report authorizes no new run and does not claim final performance, whole-system capacity, or causal lock attribution.

See `windows-adapter-clamshell-table.json` for the machine-readable rows and independently transcribed histogram counts.
