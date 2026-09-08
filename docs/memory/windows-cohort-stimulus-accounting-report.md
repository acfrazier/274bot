# Windows cohort stimulus accounting

Status: source-only implementation; no Windows, network, native, or live execution was performed.

## Delivered

`docs/memory/windows-cohort-stimulus-accounting/run_stimulus_accounted.py` is a bounded controller for the reviewed `invoke-panel-input-stimulus.ps1` helper. It accepts a root-produced `native-panel-input-stimulus-accounting-manifest-v1` JSON manifest and fails closed unless the manifest supplies the current cell, frontend PID/start/session/binary/hash, freshly inspected scene-2/capture/slot-0 rectangle and Game Image point, immutable helper path/hash, observe-start publication, source hash, and future receipt/event/envelope paths.

The controller:

- verifies the helper SHA256 (`04822c0e9f5ade2d555e408cc707722c234bc1e7b442676975a16e7efcaebbd1`) and target binary hash before launch;
- validates the observed target start identity before launch and passes the literal session, geometry, point, and identity to the helper;
- waits only for the predeclared monotonic 60–90 second window after observe-start publication, with no retry, catch-up, or late launch;
- invokes PowerShell once through an argv list, with exactly 120 pulses, 1000 ms cadence, and 80 ms holds. Geometry and point are written to a validated per-run JSON launch spec and consumed by a static per-run PowerShell `-File` launcher as typed `[int[]]` values; this avoids PowerShell `-File` positional array truncation while retaining one helper invocation and no shell interpolation of manifest values. The envelope retains SHA256 hashes for both launcher and spec;
- enforces a finite helper lifetime (135 seconds by default) followed by a bounded graceful-terminate cleanup budget and kill fallback. A timeout remains incomplete evidence and never retries or claims that the helper receipt completed;
- retains raw wrapper/helper sampler rows at bounded cadence, including explicit unavailable rows rather than zeroes, final samples, PID/start identities, per-process `cpu_seconds`/`rss_bytes` summaries, and separately reported sampler overhead. A resource status is available only when both helper and wrapper roles have valid finite nonnegative CPU counters (both user and system) and RSS; malformed or partial rows remain evidence but do not qualify. The envelope labels the sampler `root-managed windows_process_sample`; target identity is recorded but excluded from managed totals so frontend cost is not double-counted. Helper UTC is populated only when the producer supplies it in a sample; otherwise it is explicitly null because `subprocess.Popen` has no start timestamp;
- archives the exact helper receipt (including its embedded events) by SHA256 when present; it does not invent a standalone events artifact;
- validates the helper's native receipt fields against this run's target, schedule, label, geometry, and completed 120-pulse outcome;
- emits a post-run `native-panel-input-stimulus-run-receipt-v1` envelope that always has `inputCoveragePass: false` and `performanceAcceptance: false`. Missing bindings, missed trigger windows, sampler loss, helper failure, missing/malformed receipts, and receipt mismatches remain `incomplete`.

The immutable helper was not modified.

## Envelope compatibility note

This wrapper emits the v1 run envelope but intentionally does not pretend to
match the pair-controls validator from `63bfed6`/`1c2d8bd`. Compared with that
validator, `helperStartUtc` is recorded from a valid helper sample when the producer
supplies a `start_utc` field (and remains explicitly null otherwise; Python's
`subprocess.Popen` does not expose one), while
`captureEnabledVerified` and `slotZeroFocusVerified` are recorded from the
fresh binding and gate launch. The top-level `sampler` is a descriptive object,
where the controls envelope expects a `resourceAccounting.sampler` string.
Resource accounting uses explicit `helper` and `wrapper` summaries plus raw
`samples`; it does not reshape them into the controls `processes` list or add a
fabricated target-cost row. Wrapper sampler cost is separate, and target cost
is excluded from managed totals to avoid double counting. `inputCoveragePass`
and `performanceAcceptance` remain false in every envelope, including retained
incomplete failures.

## Verification

`python3 -m unittest discover -s docs/memory/windows-cohort-stimulus-accounting -v`: 16 passed.

`python3 -m py_compile docs/memory/windows-cohort-stimulus-accounting/run_stimulus_accounted.py docs/memory/windows-cohort-stimulus-accounting/test_run_stimulus_accounted.py`: passed.

Tests use fake clock, sampler, and subprocess adapters. They cover wrong target identity, missing session/UI verification, missing session binding with retained envelope, wrong helper hash, no completed receipt, missed trigger/no spawn, exactly one spawn with typed-array launch-spec binding, bounded hung-helper cleanup, unavailable sampler coverage after spawn, missing CPU counters, nonfinite settings, changing process identity, helper failure, receipt mismatch, and receipt archiving without inventing standalone events.

## Remaining integration requirements

Root must produce a fresh manifest from the actual BotTest local-console session, independently verify scene 2, capture enabled, slot-zero focus, physical rectangle and Game Image point, provide the current frontend PID/start/session/binary identity and hashes, publish observe-start, and supply the helper's future output paths. Root must perform the native no-launch contract and any actual Windows execution after review, preserve all raw artifacts, and independently check the resulting cohort evidence. This source-only work makes no host-acknowledgement, input-coverage, latency, resource-performance, or campaign-acceptance claim.
