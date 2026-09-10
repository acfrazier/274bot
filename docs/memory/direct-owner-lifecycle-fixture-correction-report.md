Direct-owner lifecycle fixture correction

Scope

Updated docs/memory/test_run_managed_cell.py only. No runtime Python or Rust,
limits, policies, live inputs, or native systems were changed.

Corrections

- Added one shared _direct_lifecycle_fixture constructor used by both the portable
  regression and the Linux-only lifecycle test. It creates the cache/unpack tree,
  serializable validated spec, release contract, five typed admission receipts,
  six-file binding map, and mocked preflight result.
- Restored actual server_resources.sample_process samples for both the real dummy
  game server and ambient helper. The lifecycle preflight no longer substitutes
  the synthetic ambient-start identity. The collector remains created and sampled
  by run_managed_cell; its identity path is not mocked or bypassed.
- Generated receipt/release times now derive coherently from the fixture's supplied
  clock. At the unchanged default now=200.0, existing unit fixtures retain their
  exact times: observation 101.0-102.0 in window 100.0-110.0, issue 120.0, and
  expiry 300.0. The lifecycle passes one current time and copies the generated,
  hashed release expiry into fake_pf instead of inventing a separate +60 value.
- Added portable test_direct_lifecycle_fixture_binds_live_identities_and_release.
  It runs validate_spec on the serialized complete lifecycle input, re-captures the
  cache binding, freshly samples both live dummy processes, checks the exact six
  release/receipt bindings, rehashes them with build_provenance.recheck_files, and
  runs the real validate_direct_admissions at the same finite now=1000.0 clock.
  The test was observed RED before the shared constructor existed, then GREEN.

Preserved lifecycle semantics

- Scoped preflight and managed-receipt mocks remain limited to the Linux lifecycle
  test. Launch creation and six-file pre-Popen rechecks remain real.
- The actual launcher and frontend child still run. The real collector remains
  alive through observe-end, Stop, C, frontend exit, and launcher exit before its
  stop request; collector exit, cleanup, guard output, and final handoff assertions
  are unchanged.
- No cap or timing assertion was weakened.

Verification on macOS

- py_compile of test_run_managed_cell.py, run_managed_cell.py, and
  managed_receipt.py: PASS.
- Six affected portable direct-owner tests: 6/6 PASS in 0.624s, including the new
  shared-construction regression and existing schema/preflight/TOCTOU checks.
- Linux lifecycle test: SKIPPED as Linux-only; this is not reported as a pass.
- Current full module: 56 tests in 41.837s, 13 failures and 1 Linux-only skip.
  Exact 37be3a1 baseline: 55 tests in 40.951s, 12 failures and 1 skip; all 12
  baseline failures recur in current and are macOS collector failures. The one
  current-only full-run failure, test_complete_cache_proof_pre_and_post, also
  fails when rerun alone against exact 37be3a1 (1 test, 1 failure), demonstrating
  it is existing macOS collector behavior rather than this fixture correction.

Evidence

Raw logs and hashes are recorded in
diagnostics/direct-owner-managed-extension/lifecycle-fidelity-correction/verification.json.
The baseline was executed from a git archive of exact
37be3a175ed128cb925172821a3ed24bbcc0a9e4.

Linux qualification boundary

The actual Linux lifecycle test and fresh native full qualification remain
root-owned after review. This portable/macOS correction does not establish native
or live acceptance, and the withheld native03 package is not corrected proof.
