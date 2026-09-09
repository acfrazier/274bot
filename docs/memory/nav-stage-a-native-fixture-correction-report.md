# Scheduler fixture correction report

Status: corrected locally; native Linux qualification remains required.

Root cause

`qualify_sharded.py` runs both generated scheduler test modules through the existing `QUAL_LIMITS` address cap of 4 GiB. The deadline/descendant test in `test_sharded.py` omitted `address`, so `frozen_support.bounded` used its frozen 64 GiB default and attempted to raise the inherited Linux hard `RLIMIT_AS`. The preserved root reproduction records inherited soft/hard 4294967296, a rejected 68719476736 request, and acceptance of the inherited cap.

Correction

Only the affected scheduler fixture call now passes `address=sh.QUAL_LIMITS['address']`. The test retains the 0.25-second global deadline, forked pipe-holding descendant, cleanup/PID assertions, and existing wall/CPU/output/resource limits. A regression test binds the fixture receipt address to `QUAL_LIMITS['address']`; on Linux it additionally asserts the child reports both RLIMIT_AS values at 4 GiB. macOS explicitly records the platform limitation because it cannot prove Linux hard-RLIMIT_AS inheritance.

Audit

The other new scheduler bounded call in `test_sharded_guards.py` already passes `sh.stage.LIMITS`, whose address cap is 4 GiB. No frozen support, production crate, client, or resource cap was changed. The proposed manifest did not require a content change; its existing hash remains 36fc057b0a371485420715fb339e58d11dab9eac7f4680e80124642a50d3179b.

Verification

- `python3 -m unittest -v test_sharded.Metrics.test_budget_reservation_refund_and_deadline test_sharded.Metrics.test_scheduler_fixture_address_binds_to_qualification_cap`: 2 passed; log SHA256 `669a1c1382ed6ac1f91a9358b82f718375dd3aefc4d3c78b393f06bc7929f812`.
- `python3 -m unittest -v`: 41 passed in 103.633 seconds.
- Source and manifest hashes are recorded in `diagnostics/nav-stage-a-native-fixture-correction/scheduler-fixture-correction.receipt.json`.
- Linux inherited-hard-limit evidence is preserved at `diagnostics/nav-stage-a-native-preparation/root-inherited-as-reproduction.json`; this checkout did not perform native qualification.

Remaining prerequisite

Root must run the fresh generated scheduler qualification on Linux under the unchanged 4 GiB qualification cap and verify the resulting receipt, address guard, tool hashes, and full generated output before any native or real feasibility release.
