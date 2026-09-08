# Native Heaptrack replay — reference-contract review blocker

Task `t_98f58617`, branch `codex/memory-diagnostics`, starting HEAD `ff162db`.
This is a partial investigation receipt, NOT an implemented native engine, a
qualification result, or an implementation-completion handoff.

## Decision required before changing differential semantics

The task and approved throughput design (lines 388–390) require a minimal
fixture and explicit review for an uncovered reference defect before contract
semantics change. That condition was reached during reference inspection.

`heaptrack_owner_replay.py:555` checks only the first function of each IP for
unresolved symbols. `classify` returns at lines 587–588 on the first useful Rust
function. Consequently unresolved inline function entries after that function,
or in a subsequent caller IP, are never examined. The loop at lines 560–565
would notice them only if execution reached them.

This contradicts the approved whole-original-stack unresolved-frame rule in the
throughput design lines 277–280. A symbol ID of zero is absent; the reference
accepts it in a legal inline triple. The current existing test
`test_ownership_unknown_callers_are_not_lost` covers an unresolved primary caller,
not either inline case. Existing normalization tests have a resolved inline
function and likewise do not cover this omission.

## Minimal generated end-to-end reproduction

New file: `heaptrack-owner-native/tests/test_reference_contract.py`.
It accepts no input path or manifest and creates only tiny generated fixtures in
a temporary directory. It runs all three reference passes, exact canonical
comparison, classification and lossless output, then deletes the scratch files.
It is explicitly a reference regression, not a native test adapter.

All cases allocate ten requested bytes at the frozen
`nav::world::NavWorld::load_pack`, source `/frozen/crates/nav/src/world.rs:50`,
with a caller trace. They preserve the same correct source family and total.

- Control: caller IP has no function. Classification says unknown, receipt
  reports ten unknown-symbol bytes, and the test passes.
- Inline caller: caller IP has known primary function `caller` and an inline
  triple `0 0 0`. Classification incorrectly says no unknown symbol and receipt
  reports zero unknown-symbol bytes.
- Inline owner: the known allocation-side owner IP itself has a following inline
  triple `0 0 0`. The same incorrect classification and zero-byte receipt result.

Both failing cases pass raw/interpreted conversion, full canonical peak
comparison, frozen family classification and ten-byte population assertions.
The failure is missing symbol-coverage disclosure, not allocation accounting or
an inferred change in actual ownership. No production prevalence is claimed.

Recommended review decision: retain first-useful-frame family selection but
compute `unknown_symbol` across every primary and inline function of every
original stack IP before returning a family. Preserve zero-IP/trace behavior.
Explicitly approve the necessary correction to the Python reference and native
parity baseline; do not silently copy this defect or exclude the unknown-symbol
field from differential comparisons. No correction has been applied here.

## Executed local verification

From the active checkout:

    python3 -B docs/memory/heaptrack-owner-native/tests/test_reference_contract.py -v

Exit 1; three tests in 0.009 seconds. Control passed; two inline cases each
failed their classification and receipt subtests (four assertion failures).
The failures are intentionally retained for explicit review, not marked xfail.

    python3 -B -m unittest discover -s docs/memory -p test_heaptrack_owner_replay.py -v

Exit 0; 21 tests in 6.065 seconds, 19 passes and two explicit Linux skips.
This unchanged suite does not exercise a native engine and does not cover the
new inline unknown-symbol cases.

    git diff --exit-code 378e634 -- docs/memory/heaptrack_owner_replay.py docs/memory/heaptrack_owner_runner.py
    git diff --exit-code 6bee87a -- docs/memory/heaptrack-owner-replay-native-result.json docs/memory/heaptrack-owner-replay-native-result.md

Both exit 0. Tracked state was clean before writing these two new owned files.
A source-cache search was denied by local filesystem policy; no workaround or
permission change was attempted. The reproduction requires only the reviewed
local Python source, generated inputs, and the approved contract.

SHA-256 from `shasum -a 256`:

- Python reference: `fa4c3ef5384055745d38e80b3e01d37a28a8d28774d175ddb0d82ef81ca18ce0`
- Python runner: `d3d7d034019c705e6930db5bec8bcd6999344f48ccf2f6f8c17a1d5e29e00f21`
- New regression: `092c99c5e78fb0b7abf095f0fc15d8717b7b6883917657ed2182007798fe6c5c`

Local compiler discovery only: rustc 1.98.0
`88d9e12ae178fab0fb5cc050a94da85685d449ea`, host
`aarch64-apple-darwin`, LLVM 22.1.8; cargo 1.98.0
`797e8a9bc 2026-08-05`. No Cargo package, lockfile or native executable has been
created; source/lock/compiler-binary/executable artifact hashes for a native
build are therefore unavailable. No native compilation, dependency fetch or
qualification occurred.

## Remaining work and boundaries

The requested native package, full state machine, bounded arenas/maps, native
supervisor integration, complete per-test native parity mapping, migration
suite, fixture-only qualification driver and implementation review all remain
unfinished. These three tests are not a substitute for any of those gates.
The full native implementation must resume after the explicit reference-defect
review decision. The same task is blocked for that decision rather than marked
complete or submitted as a finished implementation.

No production raw/interpreted input, remote system or game was opened or run.
STATE, original failed capture and original failed replay reports are untouched.
All caps, root-selected Linux build/qualification, exact-code review, separately
released production-attempt decision and final campaign gates remain unchanged.
