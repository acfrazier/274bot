# Native Linux timing diagnostic observation

Date: 2026-09-08 UTC

Scope: one completed native Linux x86_64 diagnostic, evidence only. This report does not accept a performance result, establish a fix, or attribute the earlier failure.

## Result

The diagnostic completed without reproducing a failure. The controller, launcher, collector, and frontend all exited successfully; teardown was clean; and the run produced both qualification boundaries. This is a non-reproduction, not a zero-duration result and not evidence that the earlier failure was fixed.

The immutable archive is `diagnostics/linux-failure-timing-20260908/archive-fec9793.tar.gz`, SHA-256:

`f4d5ed16c857b281296f5b95d7ac19bbee9706592a4e601bac24295e0d053077`

Its expanded sibling contains 27 files. The archive manifest was independently checked for all 27 entries: every recorded byte length and SHA-256 matched.

Run identity:

- Raw run: `native20260908T040908Z_tui_n16_active` (`raw-run-01`).
- One candidate launch, controller `fec9793`.
- Binary SHA-256: `0695a7bfd31122c17a93b9f79d2ffcfbb2eb0c2296d3fd08edc178bc726d5163`.
- Build provenance: host commit `60ca4b7d523bb255c7c6dbc61f71ebf1a19e6e88`, client commit `3456edc8dabf7b25ada78110ffa56327af9f67a4`, native Hyper-V Linux build, `std::alloc::System`, locked `memory-profile-no-alloc` with allocation counting disabled.
- Build source pre/post: 856 files, aggregate SHA-256 `5b1f33d5a3c6adcbc2069eebacf439e2f6ef62bda2281da90b0bfed9c16c81a9` on both sides.
- The raw metadata's checkout label (`b4b686fd...`) is not substituted for the recorded build provenance; the build manifest and binary identity are authoritative for the workload.

## Execution and qualification evidence

The managed receipt records launcher exit 0, collector exit 0, sampler exit 0, no binding errors, no runner errors, and a controlled collector stop after the observe window. The managed completion is exit 0, with `qualified: false` because this was explicitly diagnostic-only.

The raw stream has 276 rows: 71 seed, 30 warmup, 116 observe, and 59 teardown. The observe samples span 118.265305491 seconds (elapsed `103.645132996` through `221.910438487`). The qualification stream has exactly two 16-slot boundaries:

- `observe-start`, elapsed `102.665305022`: 16/16 slots `Running`, `ingame=true`, `scene_state=2`; no slot errors. Dispatched work was present (per-slot range 56–155), and food inventories were 3–4 (Counter 3×9, 4×7).
- `observe-end`, elapsed `222.700757229`: 16/16 slots `Running`, `ingame=true`, `scene_state=2`; no slot errors. Dispatched work was present (per-slot range 256–355), and food inventories were 21–22. The rendered runtime status shows progress through the thieving/banking flow, including bank-trip completion by the end boundary.

A scan of all raw qualification and observation rows found no `runtime.error`, `failure_attribution`, or other per-slot failure payload. The run log had no error/failure/unknown/exception event; its only matching line was the successful completion marker. Therefore there is no failure boundary from which to measure a failure duration or infer a causal path.

## Descriptive resource/timing observations

Across the 116 ordinary observe samples:

- Median resident bytes: `626327552` (the 512 MiB target is a separate campaign target, not an acceptance gate here).
- Resident range: `561098752` to `635400192` bytes.
- Aggregate process CPU slope: `0.716089622805` core-equivalents under the run's existing accounting convention (the 0.5-core target is descriptive only here).
- Client tick delta: 92,727 over the raw observe sample span, or `49.003699570` client ticks/slot/second across 16 slots.

The qualification boundary elapsed values and raw sample elapsed values have different purposes but use the same `MemoryHarness` elapsed clock; the boundary rows bracket qualification reads, while the raw stream reports sampled observations at different instants/windows. They must not be presented as interchangeable timings. Launcher/process accounting has separate origins, and the new native Hyper-V build/toolchain differs from the earlier Docker-binary evidence; no matched-performance comparison is valid.

## Historical failure retained

The earlier failed archive `diagnostics/linux-failure-attribution-20260908/archive-d4a3ed5` and `docs/memory/linux-failure-attribution-native-observation.json` remain historical failed evidence. That report records ordinal 15, tick 19, synchronous `Runtime("Unknown error")`, and a host-issued interrupt observed before cancellation. The current successful timing run neither reproduces nor disproves that failure, and it does not establish why that earlier tick was slow. No old archive was overwritten and no rerun of the failed cell is implied.

## What this does and does not unblock

This run unblocks only the evidence statement that the reviewed native timing binary and managed pipeline can complete one N16 diagnostic with stable provenance, full qualification boundaries, and descriptive timing/resource samples without producing a failure payload. It does not unblock a failure fix, causal attribution, performance acceptance, a matched control/candidate comparison, or a top-level native binding claim beyond the archived local evidence.

Under the existing investigate-or-park rule, do not repeat the unchanged diagnostic merely to wait for a failure. A bounded next action is to compare a deliberately changed, reviewed Linux candidate against the preserved build/source provenance and the earlier failed evidence, or park the Linux failure question until such a candidate or a specific source-level attribution hypothesis exists. Any future candidate comparison must first establish source/binary/fixture identity and keep clock origins explicit; it must not mix this native Hyper-V result with the older Docker binary as if the workloads were matched. Root separately owns live native binding verification.

No game behavior, timeout, fixture, source, client, VM, Docker, server, network, frontend, STATE file, gitlink, push, or merge was changed by this evidence report.

## Verification performed

- Recomputed observe-row count, elapsed span, median/range RSS, CPU slope, and per-slot client-tick rate from `raw-run-01/samples.jsonl`.
- Scanned all qualification slots and raw rows for errors and failure-attribution fields; checked first/last progress, `ingame`/`scene_state`, and food/bank readiness fields.
- Independently verified all 27 expanded archive-manifest lengths and SHA-256 values.
- Independently verified the immutable tarball SHA-256 above.
