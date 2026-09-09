Direct-owner lifecycle fixture correction

Scope

Updated docs/memory/test_run_managed_cell.py only for the Linux-only direct-owner lifecycle fixture. The fixture now declares its generated private-root-release.json release_contract path and materializes that reviewed release contract plus five typed admission receipts through _direct_preflight_fixtures, then passes real hash-bound files and a six-entry admission binding map (release_contract plus receipt_conflict/account/population/cache/server_health) through the mocked preflight result. The test-only expiry is finite and starts 60 seconds from fixture setup, so the real pre-Popen expiry and file rechecks execute without weakening production rules.

Preserved checks

- Real dummy frontend child and launcher identity handoff.
- observe-end, Stop, C, and launcher-exit lifecycle markers.
- Collector remains alive when the stop request is issued after C and frontend exit.
- Collector stop and exit status, exact owned output paths, and final exited handoff.
- Existing scoped preflight and managed-receipt mocks remain limited to this lifecycle test; launch creation, binding rechecks, expiry checks, collector startup, collection, and cleanup remain real.

Verification on macOS

- py_compile of test_run_managed_cell.py, run_managed_cell.py, and managed_receipt.py: PASS.
- Four affected direct-owner contract/preflight/TOCTOU tests: 4/4 PASS after the release_contract correction.
- Linux lifecycle test: SKIPPED (Linux-only; macOS cannot qualify it).
- Full test_run_managed_cell module: 55 tests, 1 Linux-only skip, 10 pre-existing macOS collector failures; failures are unrelated default/system/libproc collector timing behavior and were preserved.

Linux qualification boundary

The actual Linux-only lifecycle test and fresh native qualification remain for the root Linux run. This macOS fixture correction and its portable tests do not establish native or live acceptance.
