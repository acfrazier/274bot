Direct-owner lifecycle fixture escalation review

Scope

Independent grok46 review of frozen bb267d1c29a6219c9722fbc941fba143dc815c63 on
codex/memory-diagnostics. Read-only production/test inspection. Writes are this
report and diagnostics/direct-owner-managed-extension/lifecycle-escalation/.
Did not read or change the concurrent t_a6d0e3df working tree. Did not run
native/SSH/private/live/bot/server, and did not claim Linux execution from Mac.

Review input

- Branch: codex/memory-diagnostics (verified).
- Frozen commit: bb267d1 Correct direct-owner lifecycle release binding.
- Original Linux failure: root-native-qualification-02, 148 tests, exactly
  test_linux_direct_lifecycle_keeps_collector_through_stop_c_and_exit
  preflight_failed missing=['release_contract'] at 9344d75.
- 16fdb97 added generated release/receipt materialization but still omitted
  capture_contract.release_contract. bb267d1 added that path. Same-card review
  then found missing cache_dir/unpack_root. This card asks for every remaining
  structural incompatibility, not only that field.
- Production contracts inspected at bb267d1: run_managed_cell.py,
  managed_receipt.py, cache_provenance.py, build_provenance.py,
  server_resources.py, process_accounting.py.

Portable probes

diagnostics/direct-owner-managed-extension/lifecycle-escalation/probe_frozen_bb267d1.py
extracted the frozen files into a new temporary dir and reconstructed the
lifecycle spec plus _direct_preflight_fixtures. Result:
probe-bb267d1-result.json. macOS skip of the Linux-only test was observed; the
dummy /proc launcher and collector were not executed.

Confirmed native-run blockers at bb267d1

1. Missing cache roots, and the helper crashes before validate_spec.

   Frozen lifecycle spec has capture_contract.release_contract and the five
   admission_receipts paths, but never sets spec.cache_dir / spec.unpack_root
   and never calls _make_cache_tree.

   Repro (probe validate_spec_frozen):
   CellError: direct owner cache and unpack roots are required
   at run_managed_cell._validate_direct_contract.

   Repro (probe fixtures_frozen), and this is what the test actually hits
   first because it calls _direct_preflight_fixtures(spec) before
   run_managed_cell:
   KeyError: 'cache_dir'
   at test_run_managed_cell.py:394 cp.capture(spec['cache_dir'], spec['unpack_root']).

   On Linux this is an ERROR in fixture setup, not another preflight_failed.
   Adding the two string fields without a real cache tree is not enough:
   cp.capture requires the eight jags, snapshot files, and directories.

   Minimal correction: before spec_path.write_text and before
   _direct_preflight_fixtures, reuse the existing helper:

     cache, unpack = _make_cache_tree(self.fx.root / 'direct-cache')
     spec.update(cache_dir=str(cache), unpack_root=str(unpack))

   Those keys must be on the spec object that is serialized, because
   run_managed_cell loads spec_path. _test_launcher=True already skips the
   inherited unpack_root canonical check, which is the existing scoped
   fixture allowance.

Confirmed additional incompatibility (not an assertion failure under current mocks)

2. Generated helper sample does not preserve the live start identity.

   9344d75 used sr.sample_process for both game_server and ambient_helper in
   fake_pf. bb267d1 replaced that with fixtures['sample'], which returns the
   real sidecar identity for the server PID and the constant 'ambient-start'
   for every other PID.

   Repro (probe generated_samples): helper generated start_identity is
   'ambient-start'; live sample is the real process identity. Server sample
   matches the sidecar.

   Collector ownership still receives the real helper PID. run_managed_cell
   does not re-sample ambient identities after the mocked preflight, and
   mr.complete remains mocked as it was on the passing 9344d75 test, so this
   would not by itself fail the current lifecycle assertions.

   Minimal correction: keep _direct_preflight_fixtures for the six files,
   cache snapshot, and provenance hashes; restore live
   sr.sample_process(...) for fake_pf server_sample and ambient_helper as
   9344d75 did.

Not a mocked-path launch failure

3. Generated release JSON expiry is the unit-test epoch (expires_unix_s=300.0,
   issued 120, window 100-110). Pre-Popen does not re-parse that JSON; it
   hash-rechecks the six files and compares time.time() to
   fake_pf['direct_preflight']['release_expires_unix_s'] (time.time()+60).
   After a cache-tree correction, probe pre_popen_hash_recheck and
   managed_receipt._validated_bindings both passed on the six real files
   (release_contract plus receipt_conflict/account/population/cache/server_health).
   Aligning the file's expires_unix_s is only required if preflight is un-mocked.

Lifecycle / identity coverage that is not a new defect

- Dummy frontend child still writes linux_proc_start_ticks from /proc stat
  field 22, the same split used by server_resources.parse_proc_stat. Guard
  sampling is real (process_sampler('system')). Collector Popen,
  _request_collector_stop wrapping, observe-end/Stop/C/exit assertions, and
  the complete() receipt mock are unchanged from the Linux-passing 9344d75
  test. root-native-qualification-01 ran that lifecycle ok; this escalation
  did not repeat Linux.
- Scoped preflight mocking remains valid and does not disable collector
  lifetime. Empty 9344d75 direct_preflight={} is no longer sufficient after
  9344d75 admission rechecks; bb267d1's six-entry binding map is the right
  shape once the cache tree exists.
- diagnostic_argv (flags then tui/1/active) parsed and matched the spec
  under _test_launcher=True.
- source_lineage {'fixture': True} and the fixture manifest (no production
  DIRECT_* lineage) are the same stub 9344d75 used; they are only checked by
  real preflight, which this test still mocks.

Coverage limits

- Mac skip cannot validate the Linux dummy/collector path.
- This review did not execute Popen of the dummy launcher or process_accounting.
- Concurrent HEAD after bb267d1 was not reviewed.

Verdict for root

bb267d1 is not ready for another native run. The required correction is the
cache tree plus cache_dir/unpack_root on the serialized spec before fixtures
and launch. Restore live helper/server samples in fake_pf while touching that
spec. Do not treat this escalation as same-card approval or as Linux pass.
The original card still needs same-card reviewer approval and fresh Linux.
