# Native replay identity-test correction

Task `t_20b7fef4`, branch `codex/memory-diagnostics`, base `ade95e0`.
Local diagnostic test/documentation correction, submitted for same-card review.
No production reader behavior, resource limit, pass count or runtime changed.
No Linux build, remote command, native qualification or production retry occurred.

## Root cause and preserved evidence

The first Linux qualification remains **failed and unreleased**. Its core gate
ran 11 tests: nine passed; `frozen_identity_hash_suffix_and_midpass_mutation`
failed its `finish.is_err()` assertion; `interpreted_mutation_between_passes`
then failed acquiring the poisoned shared mutex. No later qualification cells
ran. The original Markdown/JSON and raw evidence were not edited. Local rehash of
`diagnostics/replay-native-f097fdc-qualification-1/qualification.json` still gives
`47ab6b8515b244b82d3fbda7bd2e4365090abc27cb216f9d811a11d0da9e7f74`.
Earlier failed capture/replay results also remain unchanged.

The old migration fixture fits completely in the native 65,536-byte BufReader.
The first `next` fills and hashes the whole original file, returns its first
line, and retains the rest in the buffer. The test then writes a same-length
change to the file's first byte. Subsequent parser reads and their SHA still
refer to original bytes. Rejection consequently depends on a metadata change,
not a content-hash mismatch. An immediate write does not portably guarantee a
new filesystem mtime/ctime, even with nanosecond-valued stat fields.

The task's upstream diagnostic evidence reports a separate Ubuntu 24.04 builder
Python probe: 32/32 immediate same-size rewrites retained the exact
`dev/ino/size/mtime_ns/ctime_ns` tuple despite changed disk bytes and original
buffered bytes. That is supplied upstream evidence, not a new remote observation
by this task. Here the original exact native test passed on macOS before editing;
we do not claim to have reproduced Linux filesystem granularity locally.
The new explicitly simulated equal-metadata tests reproduce its mechanism
without depending on this machine's clock or filesystem resolution.

Ranked explanations were: metadata timestamp collision (matches supplied probe
and buffering); hashing altered bytes incorrectly (contradicted by the actual
buffer/hash path and new consumed-byte comparisons); shared test state corruption
(explains the second failure only). No evidence establishes that the production
parser consumed changed bytes or produced a successful wrong result.

## Exact contract, not a universal concurrent-write detector

Source authorities:

- `existing-heaptrack-owner-replay-capability.md:263-267,300-306`: owned immutable
  saved inputs, frozen sizes/SHA-256, identical full hashes and counts in both
  interpreted passes. Input immutability is an execution precondition, not a
  promise that a portable reader can prevent arbitrary writers.
- `heaptrack-owner-replay-throughput-design.md:241-246,266-270`: hash each byte in
  its consuming pass, permitted batching, no prehash/extra raw read, compare
  dev/inode/size/mtime/ctime around each pass and length/full SHA at EOF.
- Original `heaptrack_owner_replay.py:122-153`: regular-file/no-follow/nonblocking
  open, exact initial declared size, line/byte bounds, SHA updated from yielded
  lines, before/after fstat tuple equality, final exact length and expected hash.
- Native `heaptrack-owner-native/src/core.rs`, `identity` and `Input`: the same
  checks; native hashes newly filled bounded buffers once and requires exact
  consumed size before success. `replay.rs:583-613` runs one raw/two interpreted
  streams, validates definitions and compares complete interpreted results.

For a successful consuming pass, SHA-256 binds its ordered consumed byte stream
to the manifest (subject to SHA-256's collision-resistance assumption). Every
observed metadata-tuple change rejects; every consumed-byte hash mismatch rejects.
Both interpreted streams must meet the same expected hash and agree in semantic
results. Neither reader proves the backing file was unchanged at every instant,
is still byte-identical after the last check, or that the pathname was never
replaced while the open descriptor continued referencing the original file.
A write to already consumed/buffered bytes can be invisible to that stream; if
metadata also compares equal, success says the original bytes were consumed,
not that no writer existed. A changed unread byte still fails the full hash even
when metadata compares equal. A stream assembled across reads is accepted only
if its complete hash matches; timestamps are not substituted for hashing.

Decision: correct the test's unstated timestamp assumption, retaining the existing
promised checks and immutable-input prerequisite. Do not add a sleep, reread,
prehash, mmap, locks, filesystem-specific watcher or weakened success check.
A reread exceeds the approved raw-one/interpreted-two stream budget and still
cannot prove absence of a transient write restored between observations. If root
requires protection against arbitrary concurrent writers rather than identity of
consumed bytes, it needs a separate reviewed input-isolation decision: e.g. a
read-only snapshot with a separately bound manifest and no remaining writable
alias to that snapshot. Merely chmodding a path or taking an advisory lock does
not establish that guarantee. No such mechanism is implemented or authorized here.

## Correction and discriminating tests

`src/migration_tests.rs` now gives each fixture a PID-plus-atomic-counter directory;
there is no shared mutex to poison. Test-only stream-open counters are thread-local,
matching the single-threaded replay; they still count real `Input::open` calls and
assert `[1, 2, 1]` for raw/interpreted/oracle. Production builds omit the counters.
The only non-test source addition is an explanatory comment at `Input::finish`.
The test mapping now names the separated tests rather than the removed combined
one; all 21 original mapping keys and every target were verified mechanically.

Separate cases prove:

- Wrong frozen expected SHA fails specifically with `input hash mismatch`.
- Lost suffix fails initial `input identity/size` independently of other cases.
- Mid-pass same-size write plus deliberately set mtime (epoch + one second)
  yields a verified different metadata tuple. The buffer still hashes to the
  original SHA; finish must report `input changed during pass`, not a hash error.
  This establishes the branch's precondition without sleeps or clock timing.
- Same-size, grammar-valid `X fixture` → `X altered` before raw replay reaches
  EOF and fails specifically on the frozen SHA, not parser/accounting rejection.
- Fully buffered write with simulated equal metadata consumes exactly the original
  bytes, hashes them correctly, and differs from the disk's changed SHA. Success
  documents the actual contract limit, not a new production exception.
- A 70,000-byte fixture changes a byte beyond the verified first buffer. With
  simulated equal metadata, `finish` rejects specifically on the content hash.
- Between interpreted passes, a same-size, legal metadata record change rejects
  specifically on the frozen hash. The second pass receives real first-pass
  peak/last-mark checkpoints, like its actual caller.

The two native collision simulations replace only the private stream's saved
metadata in tests; production checks are unchanged. The bounded local-check script
also exercises the unchanged Python reference with explicitly mocked equal fstat
results for buffered/unread changes, and real explicit-mtime mutation. All three
reference probes pass, confirming the same distinction across both readers.

During test development, strengthening the interpreted mutation from an early
allocation-underflow rejection to an EOF hash rejection exposed the old fixture's
empty checkpoint map: `replay.rs:417` panicked on its required `peak` key. That
intermediate release run was 15 passed/one failed, not a passing result. The test
now derives the real required checkpoints; the production caller already did so.
Other independent cases ran and passed despite this panic, demonstrating the
removal of the observed mutex-poison cascade. No production panic-path change
was needed or made.

## Executed local verification

Host: macOS 15.7.9 / aarch64-apple-darwin, rustc 1.98.0
(`88d9e12ae178fab0fb5cc050a94da85685d449ea`), Python 3.9.6.
Only existing saved small fixtures and bounded synthetic fixtures were read.

Commands from the campaign checkout:

    cargo build --release --offline --locked --manifest-path docs/memory/heaptrack-owner-native/Cargo.toml --bins
    cargo test --release --offline --locked --manifest-path docs/memory/heaptrack-owner-native/Cargo.toml --bin heaptrack-owner-child
    cargo fmt --manifest-path docs/memory/heaptrack-owner-native/Cargo.toml -- --check
    python3 -B -m unittest discover -s docs/memory/heaptrack-owner-native/tests -v
    python3 -B -m unittest discover -s docs/memory -p test_heaptrack_owner_replay.py -v
    python3 -B diagnostics/heaptrack-owner-native-identity-correction/local-check.py
    git diff --check

Final results:

| Check | Actual result |
|---|---|
| Isolated release binaries | Build succeeds; existing dead-code warnings only |
| Release native core | 16 passed, zero failed/skipped; 1.11 seconds |
| Cargo formatting | Pass |
| Python/native differential and guard suite | 36 run: 35 pass, one Linux AS skip; 9.192 seconds |
| Original unchanged Python suite | 21 run: 19 pass, two Linux skips; 6.037 seconds |
| Migration parallel repeat check | 20 repetitions, nine passes/zero failures each, eight test threads |
| Original mapping coverage | 21 keys; every Python/native core target exists |
| Python buffered/unread/metadata probes | All three pass; collision cases explicitly simulated |
| Diff whitespace | Pass |

The additional check is reproducible at
`diagnostics/heaptrack-owner-native-identity-correction/local-check.py`; its real
stdout receipt is `local-check.json` in that directory. It does not invoke
`qualify.py`, accept arbitrary trace paths or overwrite the old build provenance.
A programmatic tool call and an inline Python command were denied by headless
execution policy; the bounded file-based check ran instead, without permission
or configuration changes.

The native suite skips `test_native_address` (Linux native RLIMIT_AS). The
original suite skips `test_linux_native_as_is_bytes_in_owned_child` and
`test_linux_native_memory_guard_small_child` (Linux RLIMIT_AS and /proc).
The native RSS test's portable result is not native Linux monitor qualification.
No local result grants Linux throughput, production capacity, owner attribution,
savings, new capture, or retry permission. Root still owns exact reviewed-source
Linux staging and a separately released native proof, with all guard caps,
ownership, termination, pass bounds and prior failures preserved. Required
same-card Grok 4.5 review and final campaign Grok 4.6 acceptance remain separate.
