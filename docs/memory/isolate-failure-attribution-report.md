# Isolate failure attribution

Status: bounded implementation; no live/native run performed.

## Scope

Failure capture now records the first rustyscript error observed at the isolate
call boundary before `cancel_terminate_execution` runs. The record is retained
only on the existing memory-profile counters Arc and is exposed through the
existing `memory_progress` payload when diagnostics or failure capture is
enabled. Capture is absent when both switches are off.

The record contains the observed tick, `sync` versus `async-parked` call path,
a bounded rustyscript debug variant and debug text, whether V8 reported
execution termination before cancellation, and the latest monotonic host
interrupt identity when one exists. Error text is capped at 1024 Unicode
scalars and only the first attribution is retained.

Host slow-tick interruption assigns the monotonic identity immediately before
arming V8 termination. Existing timeout, cancellation location, log strings,
command ordering, and runtime behavior are unchanged. No JS probe is used to
obtain the rustyscript error attribution.

## Interpretation limits

This distinguishes an ordinary thrown/runtime error from an error observed
while host termination was armed when the rustyscript call returns an error.
It does not prove the cause of the preserved Linux slot-12 `Unknown error`,
and it does not alter or reinterpret frozen diagnostic artifacts. Async errors
that are surfaced only through the existing host `lastError` string remain on
the established logging path rather than being re-evaluated for attribution.

## Verification

- `CARGO_TARGET_DIR=target-memory-attribution cargo test -p script
  --features load,memory-profile --test load_isolate failure_capture_ --
  --nocapture` passed (4 tests), covering thrown JS attribution, host
  termination attribution with interrupt identity and post-interrupt isolate
  use, first-failure retention, and capture-off absence.
- `cargo test -p script --features load,memory-profile --lib memory_profile`
  passed, including bounded first-only attribution and capture-off state tests.
- No live frontend, native run, network/VPS action, client edit, or vendor
  dependency change was performed.
