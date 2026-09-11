# 05z: native retaliate control identity

## Result

`reader.retaliateControls()` now returns the exact Rust-observed auto-retaliate toggle pair as `{onComId, offComId}`. The host reads `GameSnapshot::retaliate_controls`; the isolate FlatBuffer appends the two component ids, fingerprints them per slot, delta-posts changes, and posts `-1/-1` to clear an unavailable pair. The thin adapter returns `null` for absent or malformed transport state and never derives identity from `retaliate_enabled`.

The Host JS declaration now publishes the internal nullable `ToggleControls` snapshot shape. Existing native hint/overhead callers remain source-compatible through `NativeFactsInput::default()`.

Implementation commit: `15e779cad1a7fb9cb148554857352ae0c7507726` (parent `21446414d21b50088aabf100df4392c3674b89fa`). Client remained `aef3952d1cd7bb3b93d39c497f0f476b68021c59` with no client edits.

## Verification

An exact archive of implementation commit `15e779cad` plus client `aef3952d` was checked at `.superpowers/review-exports/native-retaliate-controls-t_25d98ac4-15e779cad`, reusing task-owned compiler cache `target-t_25d98ac4`.

- script library: 83 passed
- native retaliate reader/transport: 4 passed
- existing native hint/overhead transport: 4 passed
- Host JS declaration: 2 passed, 1 ignored regeneration helper
- host-play `script_snapshot` group: 9 passed
- API native controls source: 1 passed
- total executed tests: 103 passed
- strict Clippy passed for script lib + native-retaliate test and host-play lib
- rustfmt check and scoped commit diff check passed

The focused tests cover exact on/off identity independent of enabled state, absent old buffers, a later clear, separate-isolate state, and the live host snapshot producer's use of the native control pair.

## Limits

This is source/transport verification, not LIVE acceptance. No scenario, catalog, fixture, navigation, opcode, or client files were changed. Root retains the Wildy full-lap reruns and evidence acceptance after review.
