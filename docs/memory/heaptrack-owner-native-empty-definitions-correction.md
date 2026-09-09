# Native replay empty-descriptor schema correction

## Scope

This correction changes only native `interpreted()` result construction. The
Python reference remains unchanged. When no `a` descriptor records occur, the
native `first.definitions` object now omits `a`, matching the Python
`Counter({'s': 1, 'i': 1, 't': 1})` contract. A nonzero descriptor count is still
published exactly as before; allocation/free replay, marks, suppression
metadata, snapshots, canonical output, and the deferred last-event result are
otherwise unchanged.

## Red/green evidence

The new differential test was first run against the pre-correction native
binary:

    python3 -B -m unittest discover -s tests -p 'test_event_record.py' -v

It ran 5 tests and failed the two empty-descriptor subtests with
`AssertionError` at the differential result assertion. The retained failure
was the native/reference mismatch for `first.definitions` (`a: 0` emitted by
native while Python omitted `a`). The other three event-record tests passed.

After removing the unconditional native `a` insertion, the same command ran 5
 tests and passed. The passing cases include descriptor-free no-event replay
with a mark, the same replay with suppression metadata, an unused descriptor
with count 1, an ordinary free-event tail, and malformed-tail rejection. Each
uses the actual fixture child and full-output differential path, not a JSON
helper.

## Verification

- `cargo build --release --manifest-path docs/memory/heaptrack-owner-native/Cargo.toml`: passed.
- Native Rust core tests, serial (`cargo test ... --release -- --test-threads=1`): 16/16 passed. Existing dead-code warnings remain.
- Native/Python focused event-record differential suite: 5/5 passed.
- Native/Python full suite was attempted and exercised 41 tests: 24 passed before 1 Linux address-space skip and 16 setup errors because the historical `diagnostics/owner-capture-evidence-2015/owner-capture-evidence-2015-manifest.json` fixture is absent from this checkout. The same missing historical fixture also affects the mapped original-fixture wrappers; these are environment/setup failures, not relabeled passes.
- Original Python suite: 21 run, 19 passed, 2 explicit Linux native guard skips.
- `cargo fmt --manifest-path docs/memory/heaptrack-owner-native/Cargo.toml -- --check`: passed.
- `git diff --check`: passed.

The existing three-pass fixture remains exercised by
`test_three_pass_baseline_and_first_peak` in both the unchanged Python suite
and the native-mapped suite. Full output and exact first/snapshot/canonical
comparisons are retained by the differential adapter. No timing, Linux
qualification, production replay, or performance claim is made here.

## Unresolved gates

Linux native address-space/resource qualification, production capture/replay,
throughput qualification, capacity, lifecycle acceptance, and final
whole-branch review remain unresolved. The absent historical diagnostic fixture
must be restored or supplied by the root owner before the setup-dependent
native guard/parity tests can be rerun cleanly. The prior last-event performance
correction remains in place and is not remeasured by this schema fix.
