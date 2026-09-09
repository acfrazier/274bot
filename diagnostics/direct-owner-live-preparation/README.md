# Direct-owner live preparation (offline only)

This directory is a root-release contract, not a live launcher. `prepare_invocation.py` never starts, stops, or signals a process and never opens an account, vault, cache, server, frontend, or PTY. It accepts only explicit absolute paths to public/provenance artifacts and root-produced qualification receipts; missing, placeholder, or mismatched identities are refused.

## Offline check

Run from the later root-admitted checkout, with the real paths supplied explicitly:

```text
python3 diagnostics/direct-owner-live-preparation/prepare_invocation.py \
  --source-archive /ROOT/frozen-direct-owner-source.tar.gz \
  --source-manifest /ROOT/frozen-source-manifest.json \
  --derivative-admission /ROOT/direct-owner-derivative-admission.json \
  --qualification /ROOT/linux-qualification/qualification.json \
  --build-manifest /ROOT/direct-owner-build-manifest.json \
  --binary /ROOT/tui-play \
  --controller /ROOT/n1-controller-e707e2d/run_current_tui_calibration.py \
  --controller-manifest /ROOT/n1-controller-e707e2d/install-manifest.json \
  --nav-pack /ROOT/274bot.navpack \
  --output /ROOT/direct-owner-release-contract.json
```

`/ROOT/...` is documentation only; it is deliberately rejected as a placeholder. The command must be rerun with actual root-owned paths. It writes one new JSON contract and does not launch.

The generated contract requires the exact frozen archive SHA, original and reviewed H/C provenance, and a separate root-supplied `direct-owner-derivative-admission-v1` receipt. That receipt binds the archive provenance to clean checkout identities used for the derivative; those identities must not be relabeled as the original H/C commits. The real build-manifest shape is required (`candidate.commit`, `candidate.client.commit`, non-empty branch, client source digest, `binaries.candidate_tui_play`, string `features.requested`, allocator, and structured feature fields), along with controller review `e707e2d` plus the reviewed controller file SHA-256 `8634665855d87aa93d27f10bc386f87d04be4c522b24e5379174f45cc2f37312`, nav-pack SHA, a native generated qualification with `linux_generated_qualified: true`, and a direct-owner feature pair of `memory-profile-no-alloc` and `memory-owner-capture` with locking enabled, allocation counting disabled, and `snapshot_dedup: false`. It records `releaseable: false` and `native_qualified: false` because this preparation does not perform native or live admission.

The reviewed controller currently accepts only the original `memory-profile-no-alloc` feature contract, and its `check_source` requires clean Git HEADs equal to the expected original H/C commits. A narrow, separately reviewed controller/launcher extension is still required to accept the owner-capture pair and the validated derivative checkout admission while preserving the original locked/no-allocation/snapshot-dedup guards; this preparation intentionally does not edit or claim that extension.

The separate `docs/memory/validate_direct_owner_capture.py` validator remains the JSONL protocol check. It does not certify native qualification, account/cache/server admission, RSS, or cleanup. Do not add those claims to owner JSONL.
