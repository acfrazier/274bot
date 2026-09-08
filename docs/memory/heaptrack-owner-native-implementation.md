# Native Heaptrack replay — implementation handoff

Task `t_98f58617`, branch `codex/memory-diagnostics`. This section supersedes
the historical investigation below; that evidence remains preserved at
`698cc6a`. Root approved the whole-stack reference correction in
`heaptrack-owner-reference-defect-decision.md`; it is now implemented in both
engines. This is an exact-code review handoff, NOT Linux qualification, a
production-attempt release, a successful capture, or accepted memory savings.

## Implemented scope

`heaptrack-owner-native/` is an isolated Cargo workspace with no game crates.
Its pinned lock uses `libc` for child limits/resource clocks, `sha2` for streaming
identity and normalized digests, and `serde`/`serde_json` for duplicate-rejecting
contract parsing and deterministic semantic output. `heaptrack-owner-child`
is executed directly by the existing Python supervisor: explicit
`--engine native --native-executable /absolute/path --native-sha256 SHA256`.
There is no automatic native selection, fallback, Python wrapper child, FFI,
or production path in the fixture driver. On Linux the supervisor executes the
retained, hashed executable descriptor through `/proc/self/fd`; portable macOS
uses the checked pathname and rejects identity changes before publication.

The native modules implement:

- Strict manifest, receipt, stderr, provenance, duplicate-key and file identity
  validation; raw-pointer audit, metadata/events/trace digests; raw once,
  interpreted twice and full positive canonical oracle reconciliation.
- Byte-correct grammar, checked arithmetic, sentinel zero, multiplicities,
  temporaries, first peak/time/EOF snapshots, all inline functions and original
  unknown-symbol coverage without changing first-useful-family selection.
- Compact definition arenas and seeded bounded maps with tombstones, charged
  old+new growth, rehash and 4096-probe failure. A global capacity allocator
  charges all selected-output and temporary state against the same 256 MiB
  ceiling; this includes more runtime state than Python's estimated charges.
- Buffered input SHA and 64 KiB normalized-digest batches; cheap work pulses
  with expensive clock/resource checks every 1024 records/inner operations or
  roughly 100 ms, external monitoring at 100 ms, and forced phase-end checks.
  Render, map probe, ancestry, canonical aggregation and sort paths participate.
- Deterministic lossless TSV/JSON, frozen conservative source classification,
  complete selected graphs, output hashes, and pending-only publication. No
  successful partial rankings or change to the failed capture status.
- Existing input/count/depth/line/numeric, CPU180/wall300 per pass, total900,
  RSS512/AS768 MiB, table256/output32/scratch64 MiB and admission caps remain.
  OS rlimits are child-only; the parent retains its limits and owns kill/reap.
  Failures may include bounded numeric line/offset/record/time progress, never
  raw pointer/command text. Abrupt failure can legitimately omit progress.

The separate `heaptrack-owner-fixture` executable only accepts bounded fixture
bytes over stdin or a finite negative-guard name; it is not a production engine.
`qualify.py` has no trace/manifest input argument or production branch. It runs
the staged native core tests, the complete fixture suite, and nine explicit
generated cases with three repeats each (27 timed native runs), using lower
CPU30/wall60 limits and an overall 900-second budget. Saved smoke hashes are
allowlisted; generated raw/interpreted/oracle members are each <=2,000,000 bytes.
The cases cover short/wide streams at three sizes, definition/descriptor
diversity, long symbols and a near-line-cap record. All successes compare
digests, snapshots, canonical state and exact output content to the corrected
reference. Non-simple cases report rates without pretending to meet a
production-capacity threshold.

## Executed local commands and results

Working directory is the active campaign checkout. No Linux qualification or
remote/build staging was executed.

    cargo fmt --check --manifest-path docs/memory/heaptrack-owner-native/Cargo.toml
    cargo build --release --locked --manifest-path docs/memory/heaptrack-owner-native/Cargo.toml
    python3 -B docs/memory/heaptrack-owner-native/build_provenance.py
    python3 -B docs/memory/test_heaptrack_owner_replay.py

All exited 0. Original Python suite: 21 tests, 19 pass / 2 explicit Linux skips,
6.739 seconds. The provenance builder uses `cargo test --release --locked
--offline --bin heaptrack-owner-child --no-run --message-format=json`, stages
the actual test executable, checks all 21 mapping keys and resolves every
mapped Python/native-core test name. It recorded 36 Python/native tests and
11 native core tests. The build has only dead-code warnings for symbols used
by the companion fixture binary rather than the main child.

    python3 -B docs/memory/heaptrack-owner-native/qualify.py \
      --portable-probe \
      --output docs/memory/heaptrack-owner-native/local-probe-2 \
      --expected-child-sha256 ef72a0f204e720dccc58afa41a26fcaefc26cdf7a2605cea5e3703fd67348df0 \
      --expected-fixture-sha256 332f1a2e1ea74f21aa5cd9f87f091e77dbaa530790153164b5fa756462bf0220 \
      --expected-core-test-sha256 7e3bbc3b512482348f071a234622e0926cbec07207484a359547fdb64464dd0a

Exit 0, `portable_probe_passed`, 19.031362125 seconds total. The immutable
local result is `heaptrack-owner-native/local-probe-2/qualification.json`:
11/11 native core tests pass; Python/native suite 36 tests, 35 pass and one
explicit Linux RLIMIT_AS skip in 9.190 seconds. This suite includes the three
reviewed reference regressions, and separately exercises their native parity,
zero/empty inline names and a fully resolved control. CPU/wall/RSS failures
execute the compiled fixture's shared native guard, not a Python failure proxy;
AS is packaged but not claimed on macOS. Protocol corruption/abnormal exit,
wrong executable identity/no fallback, output/table/scratch rejection,
parent-limit preservation and owned reaping are exercised.

All nine cases and 27 repeats passed semantic comparisons. Simple-case median
rates in decimal MB/CPU-second ranged raw 64.908–140.640, interpreted
17.267–17.654, and selected-output pass 11.873–16.493. These exceed the fixed
necessary gates (11.938945 raw, 4.503128 interpreted/selected) on this portable
fixture run only. Diversity, long-symbol and near-line-cap rates are not
substituted for simple-rate gates. They do not establish production capacity.
The first local probe predates final migration/provenance additions and is not
the submitted verification result.

`git diff --check` passed. `git diff --exit-code 6bee87a --
docs/memory/heaptrack-owner-replay-native-result.json
docs/memory/heaptrack-owner-replay-native-result.md` passed: original failed
capture/replay evidence remains unchanged. No host/client/runtime files or
root STATE were edited; affected verification is this isolated instrumentation,
not an application build/live session.

## Full original-test mapping

Machine-checked source of truth: `heaptrack-owner-native/test-mapping.json`.
Names below omit the `test_` prefix and common module path. The mapping is not
a claim that running the Python originals alone exercises native code.

| Original test | Native coverage |
|---|---|
| strict_records_preserve_string_bytes | NativeSeams record adapter; byte/UTF-8 chunk splits |
| three_pass_baseline_and_first_peak | OriginalFixtures native analyze differential |
| saved_smoke_full_canonical_multiset | OriginalFixtures plus NativeParity direct engine |
| lossless_output_and_fail_closed_output_budget | Exact NativeParity files; native output cap failure |
| owned_portable_runner_smoke | NativeParity hash-bound direct child |
| synthetic_realloc_multiplicity_zero_and_survivor | OriginalFixtures native analyze differential |
| time_rejections_and_unbounded_event_tail | OriginalFixtures native analyze differential |
| record_adversaries | NativeSeams invokes original adversaries through Rust adapter |
| reference_and_session_adversaries | OriginalFixtures native rejection differential |
| conversion_hash_suffix_and_canonical_not_sum | OriginalFixtures plus native frozen identity/midpass/between-pass mutation tests |
| each_table_numeric_input_and_depth_cap | OriginalFixtures; native zero/depth and checked numeric core tests |
| suppression_does_not_change_population | OriginalFixtures native analyze differential |
| normalization_collisions_new_stop_inline_unresolved | NativeSeams original normalization plus zero/depth |
| resource_guards_and_parent_unchanged | Existing external sampling/admission contract plus actual native CPU/output/table/scratch |
| real_owned_wall_kill_reap_and_bounded_failure | NativeGuards compiled wall failure and reap |
| resource_failure_reports_exact_guard_and_usage | NativeGuards actual lower resource failures |
| ownership_unknown_callers_are_not_lost | NativeSeams ownership plus corrected whole-stack exact output differential |
| real_owned_cpu_guard | NativeGuards compiled CPU exhaustion |
| manifest_and_receipt_rejections | NativeContract direct child, bypassing parent validation |
| linux_native_memory_guard_small_child | NativeGuards compiled RSS failure (portable getrusage here; Linux monitor later) |
| linux_native_as_is_bytes_in_owned_child | NativeGuards actual native AS test, explicit local skip |

Additional native checks cover actual stream opens `[1,2,1]`, normalized hash
batch identity, chunk splits inside UTF-8/numeric/record fields, 24 seeded random
pairs, overflow, capacity/transient growth, rehash/tombstone churn, forced hash
collisions, output stability, phase-end/inner-work/render/sort stalls, frozen
file mutation, duplicate JSON keys, provenance/receipt counter/warning failures,
and finite malformed/duplicate/out-of-order/abnormal child protocols. Exact
diffs exclude only resources/timing, executable/path identity, and engine-specific
charged-capacity measurements; canonical/unknown-symbol fields are compared.

## Build identity and remaining release gates

Every owned source/test/runner/reference SHA, manifest and lock SHA, compiler
identity and executable SHA is recorded in the tracked
`heaptrack-owner-native/build-provenance.json`. Important identities:

- Cargo.lock: `87adb225752ab53230c3f48ace04da3bd651f6815189a056c4d4430f17bf336e`.
- Compiler binary: `a11618eca0956a8aa4372c2bc898690b513cbdfa2cb9125b2a5301e360ed5b49`.
- rustc 1.98.0, commit `88d9e12ae178fab0fb5cc050a94da85685d449ea`,
  `aarch64-apple-darwin`, LLVM 22.1.8; Cargo 1.98.0
  `797e8a9bc 2026-08-05`.
- Native child, fixture and core-test executable hashes are the exact three
  arguments in the successfully executed command above.

These are macOS artifacts, NOT deployable Linux provenance. After exact-code
Grok 4.5 review, root must select a Linux builder/toolchain/target, build with
the pinned lock, run the provenance helper and stage all three hash-bound
executables. The fixture driver does not require Cargo on the executor once
the artifacts are staged. Root must explicitly release the finite qualification
command without `--portable-probe`, supplying the new Linux hashes. Its gate
rejects Linux skips and tests real AS/RSS/CPU and sampling. No retry or cap raise
is authorized automatically on failure.

Limitations retained explicitly: warm-cache tiny local fixtures do not prove
Concord throughput, cache behavior, allocator capacity, actual production unique
pointer/trace cardinality or deadline headroom. Native capacity charging includes
allocator/runtime/output state rather than mirroring Python's estimate exactly.
Portable macOS executable execution has pathname revalidation rather than the
Linux descriptor execution guarantee. macOS lacks the Linux `/proc` external
RSS/AS/thread samples, so those are absent, not estimated. The generated fixture
adapter's bounded staging/JSON overhead is outside replay phase-rate numerators;
whole-child wall and RSS remain recorded/bounded separately. Linux compatibility,
exact-code approval, fixture-only qualification, root's separate one-attempt
production decision and final campaign/whole-branch review are still required.

No production traces, remote machines, capture/reinterpretation or game/app
launches occurred. No accepted memory saving or final campaign completion is
claimed.

---

## Historical reference-contract blocker (preserved evidence; resolved above)

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
