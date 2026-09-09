# Frozen nav differential diagnostic

Read `../nav-tiled-differential-report.md` for the actual result, failures and
open real/native gates. This directory is task-owned diagnostic tooling only.
Never point this card at a real pack. No default production input path exists.

## Generated operation

From the campaign checkout:

```sh
python3 docs/memory/nav-tiled-differential/harness.py prepare --run docs/memory/nav-tiled-differential/NEW
python3 docs/memory/nav-tiled-differential/harness.py build --run docs/memory/nav-tiled-differential/NEW
python3 docs/memory/nav-tiled-differential/harness.py generated --run docs/memory/nav-tiled-differential/NEW
python3 docs/memory/nav-tiled-differential/audit.py docs/memory/nav-tiled-differential/NEW
python3 -m unittest discover -s docs/memory/nav-tiled-differential -p 'test_*.py' -v
```

Use a new run name, never overwrite a result. The checked-in immutable archives
preserve the three development coverage stages; final evidence is run-03.
`audit.py RUN --archive-only` creates a new archive, skips target directories,
and reads every archived file back to verify its SHA-256.

The probe template is compiled separately inside frozen dense/tiled workspaces.
Only three storage access expressions differ; all original source bytes are
checked. The router has an appended access-module declaration solely to invoke
its original bounded implementation. Six exact frozen host source spans supply
radius/bank orchestration. Generated input filenames, bytes, selector ordering,
fixed route rows and tool recipes are frozen before either arm.

`generated` also exercises the one-input executable protocol against an owned
copy of generated `routes-open.bin`. Its receipt's executable argument `real`
does not mean production data or native guards were used. This protocol fixture
is labeled separately in result.json and remains under GENERATED limits.

The real wrapper is inaccessible on Darwin. Never treat the binary alone as an
authorization boundary or sandbox: supported execution is through the harness.

## Output framing

Each record is `tag`, one ASCII space, decimal payload byte length, newline,
then exactly that many payload bytes and one newline. Binary cell/canonical
wire payloads can themselves contain arbitrary newline bytes. Do not parse
these streams as JSON-lines or grep them as plain text.

Cell records are 15 bytes per logical cell, in original level/z/x order:

- u8 face, u8 blocked;
- u32le original walk_word_from_parts result;
- u32le queried walkable_word;
- u32le raw flag result;
- u8 with walkable in bit0 and standable in bit1.

Blocked-word records contain complete u64le words, including unused final high
bits. Canonical wire payloads are the complete original encode output. Other
records contain full Rust Debug representations; route records additionally
contain exact f64 tick bits. All enum fields, vector order and graph.at index
vectors are retained. Only inherently unordered map/set keys are sorted.
The final `complete-input-count` record is required; direct byte comparison is
used in addition to hashes and the frame/count inventory.

## Future real authorization (not released by this card)

On the separately authorized native Linux host, first rebuild and qualify
GENERATED inputs there, then run `harness.py guards --run <new-owned-dir>`.
That command produces guards.json only reporting `native_real_qualified: true`
when its full guard test suite passes on Linux with no skips. AS qualification
cannot be imported from Darwin; platform, harness/test hashes and the actual
receipt/stdout/stderr hashes are checked. The positive Linux wrapper admission
path has not been executed by this card and needs native review/qualification.

After root reviews the particular input, startup overlap, limits and exact route
selectors, the root-issued authorization JSON must supply ALL of:

| Key | Required meaning |
| --- | --- |
| mode | `real` |
| released | boolean true, set only by root after separate release |
| launch.json_sha256 | Native generated run's launch.json SHA-256 |
| result.json_sha256 | Native successful generated result.json SHA-256 |
| sources.json_sha256 | That run's sources.json SHA-256 |
| dense_admission_sha256 | dense-admission.json SHA-256 |
| tiled_admission_sha256 | tiled-admission.json SHA-256 |
| guard_report | Absolute path to qualified native guards.json |
| guard_report_sha256 | Its SHA-256 |
| input_path | Absolute path to ONE regular non-symlink v8 pack |
| input_sha256 | Its admitted SHA-256 |
| input_bytes | Exact admitted length, at most 134217728 |
| routes_path | Absolute path to frozen fixed route rows |
| routes_sha256 | Their SHA-256 |
| limits | Exactly `{ "wall": 900, "cpu": 800, "rss": 1073741824, "address": 4294967296, "output": 4294967296 }` |

The future invocation is `harness.py real --run <qualified-native-run>
--authorization <root-issued-json>`. No filled authorization is checked in.
An existing `real-release` directory prevents a second input/retry in that run.
Failures retain receipts and output; they do not authorize looser caps.

Fixed route file: 1..256 lines of ten decimal integers:

    from_x from_z from_level to_x to_z to_level option_bits state_family radius model

Coordinates must fit the checked range -16384..32767; levels 0..3;
option_bits 0..15: bit0 teleports, bit1 wilderness, bit2 bank-fetch, bit3 Aubury
essence session. `radius` 0..4; model 0 running or 1 walking. Each row exercises
full model, teleport-model and original host calculate outputs. The model flag
controls the two explicit model probes; host calculate intentionally retains
its original running-cost behavior. A nonzero radius uses original host
approach/calculate, never a new radius router.

State families: 0 empty; 1 skills `(6,25),(2,3)`, completed quests `Rune Mysteries`
and `é`, varp `(150,160)`; 2 carried `(995,10)` and worn 1712; 3 union of 1 and 2.
Bank supply is fixed `(995,10),(1712,1)`. These are diagnostic facts, not live
account facts. Root must choose meaningful real routes and explicitly retain
unexercised cases; do not call a missing-requirement NoPath a successful
bank/teleport proof. Arbitrary state snapshots need a separately reviewed
extension, not an unrecorded edit to frozen selectors.

## Safety and portability

See the report for all exact caps. CPU/AS are per process, RSS is sampled group
accounting, and owned-child cleanup is process-group based. No protection is
claimed against hostile code escaping the group. Generated decoder admission
caps geometry and allocation-bearing string lengths before either process.
The supervisor does not confuse EOF with process exit and keeps combined output
bounded even when descendants hold or flood both pipes.

Darwin's rejected/ineffective AS behavior is explicitly unsupported for real
runs. `check_limits.py` is a platform diagnostic, not a way to qualify hard AS.
`receipt_summary.py` only reads retained run-01/run-03 build receipts; neither
utility opens a real pack or launches a client. No script modifies registry,
credential, server, runtime cache, prior census or production-source files.

Archive restoration must go into a new owned diagnostic directory; never
extract over original evidence. Binaries/target caches are excluded from archives
and remain local with admitted hashes. Use fresh `prepare/build/generated` for
reproduction; do not replace admitted source or recompile an accepted run in
place. Compiler/OS/SDK differences require a new qualification, not equal binary
hash expectations across machines.
