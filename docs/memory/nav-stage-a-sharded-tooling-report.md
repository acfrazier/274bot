# Bounded singleton routing tooling — implementation report

Card `t_500f341e`, branch `codex/memory-diagnostics`. Implements the tooling-only
release of `nav-stage-a-routing-resource-plan.md` (`6a2c566`), based on the
reviewed `f24de7c` diagnostic. No native/SSH/real-pack/live execution, production
optimization, cap increase, remote change or acceptance release occurred.
Independent same-card review is required.

The original implementation snapshot below was submitted as `5324f41` and
REJECTED for an unguarded setup-CPU prefix. Its evidence/hashes remain historical;
the **Idle-window correction** section at the end is the current tooling freeze.
The intervening `5c0ab7f` setup-CPU fix passed that regression but was rejected
in round 2 because the separately required fixed idle-window correction was missing.

## Delivered

- `nav-tiled-stage-a/stage_probe.rs`: preserves ordered elapsed samples by
  formatting the existing preallocated duration vector AFTER the timed batch,
  before its original sort/drop. Adds a versioned raw summary; original elapsed
  endpoint, CPU reads, warmup, eight sweeps, three distinct calls, aggregate and
  owner-drop assertions are unchanged. The formatting buffer is extra post-route
  observer memory and can affect post-route/drop RSS. It is not subtracted from
  measurements or presented as instrumentation-free equivalence.
- `sharded.py`: separately versioned root authorization, original 59-ordinal
  binding, fixed F1/F2/six-pair schedule, actual global alarm cleanup, cumulative
  CPU/wall reservations/refunds, unknown-CPU charge/stop, headroom and projection
  gates, original-provenance checks across continuations and final aggregation,
  exact pooled/per-lane raw p99, weighted batch CPU, baseline-noise rules and
  first-evaluable row peak failure. Claims/checkpoints prevent automatic restart.
- One shared read-only full input copy for the release, 59 small selectors,
  compact per-child output/records and constant-size checkpoints. The whole
  release cap is 512 MiB, with 1 GiB free reserve plus remaining declared capacity.
  No evidence deletion. See `README-sharded.md` for schemas, command arguments,
  precise storage/accounting limits, native prerequisite structure and caveats.
- `test_sharded.py`, `test_sharded_guards.py`, `qualify_sharded.py` and
  `sharded-proposed-manifest.json`. The proposed manifest is NOT an authorization.

`stage_a.py`, its legacy `real` semantics, existing proposed manifest, allocator,
all three frozen support files and completed historical evidence are unchanged.
No independent replacement runner/router/decoder was introduced: new scheduling
uses the original source/admission/preflight functions, decoder allocation guard,
`bounded()` owned process runner, phase parser and `paired()` noise classifier.
The frozen `ccff4bb` helper was not edited.

## Frozen builds and original-object readback

All four new builds used independently materialized original Git objects, never
the concurrently edited host/client path dependencies. Final independent readback
resolved every manifest path through its original Git tree and compared blob
bytes: **209 dense and 210 tiled original files passed**, including pinned client
`3456edc8dabf7b25ada78110ffa56327af9f67a4`. Host helper spans also match their original
source manifests and hashes. Source arms remain dense
`29b7aea779322c8611f83dc193e939ca7d756f75` and tiled
`8385babb23fd15b876506d4a3f6154984a6b2df1`.

Fresh run: `docs/memory/nav-tiled-stage-a/sharded-run-01/`.
All build return codes were zero; admission and resolved lock/source/compiler
bindings are preserved individually, not rewritten historical admissions.

| Arm / variant | Build wall seconds | Binary bytes | SHA256 |
|---|---:|---:|---|
| dense / clean | 68.661165 | 620752 | `0daa6e96e2cedbd06bf3bce9a8c5d46a209eeb87f6fc99fc6cebbbfbc24399d4` |
| dense / counting | 68.682941 | 636768 | `d56647ff64c914f1a80198abe72938053e34983bf3d0324e088c5f18058acc59` |
| tiled / clean | 75.491742 | 637696 | `3269817125f8e99fe24a4f15b77b69883d6ad6a822fd25fe4f86be9da970f36a` |
| tiled / counting | 74.658674 | 637728 | `12ff92ffd0133545d309ca8c1aee4ba402627bc5a1de4f1c93f2c4f3dde7efb2` |

New probe SHA256:
`1c3fb5f7674d59c3f13140e76db92b42ae01d76973884e21d0e86ffc2c118c12`.
Scheduler SHA256:
`55c9366b93003cff411ff7aacd3f488143c58f8de3be6438f406214a5b125ef8`.
Complete tool hashes, four admission hashes, source manifests, compiler/locks and
host-span hashes are frozen in the archived readback and admissions.

Unchanged probe spans, independently compared against `f24de7c`:

- Three original route calls:
  `2baed5ffea20d7f6aa63d003ae1d085a37612be9fefba2b070b5e0a97b1e98dd`.
- Request construction, lookup, warmup and timed loop:
  `d2fbb0849f3db5958792612fbd1372cfd00fdadbb5c1d81425f5a95dae21f975`.
- Last-owner/drop assertion interval:
  `abf83cbc36409a8c0e0df94c7b0ed475d0470d52f9592cdd18b5b12484aa33d7`.

## Commands and verified outcomes

Commands were run from the campaign checkout; `RUN` below expands to the absolute
path of `docs/memory/nav-tiled-stage-a/sharded-run-01`. Complete logs, individual
child receipts and commands are in `diagnostics/nav-stage-a-sharded-tooling/`
and the corresponding fresh run directories.

1. `python3 docs/memory/nav-tiled-stage-a/stage_a.py prepare --run RUN`: exit 0.
2. Same driver `build --run RUN --variant clean`, then `--variant counting`:
   exit 0 each, four fresh frozen binaries/admissions.
3. Same driver `generated --run RUN --variant clean`, then `--variant counting`:
   exit 0 each; all-uniform/all-dense/gated fixture qualification passed.
4. Same driver `guards --run ABSOLUTE_STAGE/sharded-guards-01`: exit 0,
   **17 original guard tests passed**, including between-repetition mutation,
   output/CPU/RSS/EOF and pipe-holding descendant cleanup.
5. `python3 docs/memory/nav-tiled-stage-a/test_stage_a.py --run RUN`: exit 0;
   both-arm source/lock/binary/tool mutation rejection, four clean/counting
   self-tests, malformed inputs, phase rejection and generated legacy release
   protocol passed. Counting and clean timings were not mixed.
6. `python3 docs/memory/nav-tiled-stage-a/qualify_sharded.py RUN singleton-qualification-02`:
   exit 0. Shared bounded runner executed **17 new tests**, all passed, no unittest
   skips, in 90.573 s. Then four ACTUAL new clean-probe children exercised F1 and
   the entire new probe-to-driver schema on a tiny generated pack. **12 old/new
   fixture comparisons** (both arms, both variants, three fixtures) matched
   aggregate/layout/read checksums; every new raw array reproduced its own
   summary p99 (not the historical timing value). Historical `run-05`
   outputs were read only. Its temporal/phase helper comparison also passed.
7. `python3 diagnostics/nav-stage-a-sharded-tooling/export_readback.py`: exit 0;
   independently rechecked original Git blobs, all admissions, raw aggregation,
   counts, single-copy storage and every archive member.

The final new-suite matrix uses explicit deterministic mocked child output for
**4 F1 + 114 F2 + 708 fresh six-pair slots = 826 slots**, not 826 expensive router
executions. It uses 59 distinct ordinal selector rows. Its final receipt contains
92.235442459 charged active wall seconds and 88.89523 charged CPU seconds,
including three prepaid publication tails. These are TEST CONTROLLER costs,
not routing performance or a native feasibility prediction. Its complete release
uses 1,962,838 bytes and exactly one `input.bin`.

Validation includes clustered/spread 1416-sample rank-1402 and 472-sample rank-468
quantiles with retained actual arrays; weighted CPU `1/101` rather than averaging
row percentages; exact 5% / 2 ms / 8 MiB boundaries and noise/straddle cases;
negative values, incomplete/reordered coverage and raw arrays; original/loaded
authorization, input/copy/full TSV/shard/tool/admission gates; continuation root/
receipt/output/checkpoint budgets; final-aggregation output mutation; reservation
refusal and refund, unknown-CPU charge, headroom stop, setup interruption and
restart refusal. Actual processes establish both supervisor CPU-alarm execution
and global driver deadline killing/reaping a group with a pipe-holding descendant.
No existing path is interpreted as permission to resume a failed phase.

Source/lock/executable/native-qualification after-child dispatch tests inject the
shared prerequisite rejection; the existing admitted-binary integration separately
mutates actual source/lock/executable/tool files. They are distinct proof layers,
not a claim that a mocked native receipt was Linux-tested.

## Failed attempts and corrections retained

Logs `01` through `20` retain the development red/green sequence. Initial missing
quantile/schedule/budget/driver/receipt/native-gate assertions failed as intended
before implementation. Additional adversarial tests found two real intermediate
holes: a negative carried CPU budget could continue, and an output mutated after
aggregation could still publish completion. Both failing receipts/logs remain;
nonnegative/bounded continuation validation and rebinding previous/current output
records at final publication correct those cases. Negative row CPU and missing
supervisor CPU/interrupt gates were also added and verified through red/green tests.

A terminal call containing an unnecessary `python3 -c` print was denied by the
headless approval policy. The ordinary test command then ran without that print;
no policy/configuration bypass occurred. Two patch applications failed on ambiguous
context before changing files; subsequent bounded patches used exact context.
An initial four-child new-driver smoke and earlier matrix passes remain retained
as earlier-tool snapshots, not the final freeze. Final qualification is the
numbered `singleton-qualification-02` result below.

Early unit fixtures used temporary directories, retaining their full unittest
failure logs rather than each scratch file. Subsequent adversarial/final fixtures
are retained without cleanup and included in the archive. No historical failed
real or cold-screen evidence was modified. Qualification may overlap other local
agent work and supplies no clean performance claim. This task ran diagnostic
build/tests, not a workspace-wide formatter/linter or live campaign suite.

## Evidence freeze and remaining release gates

Final qualification result SHA256:
`ce740914415e9c33bd09e06c22d97b5f06b65983acc6e4744c57e6f96c1162ca`.
Proposed manifest SHA256:
`490f8e23d73352ff0d02ef40a21353889ca6ad920337aa3811958f5616eb287e`.

Archive:
`diagnostics/nav-stage-a-sharded-tooling/tooling-evidence.tar.gz` — **6,700,772 bytes**,
SHA256 `bd3d00bd4365b4f7b2e76457820e03cf6ff467ad39e16c0c01f8ce9e5f2c3444`.
All **26,973 members** were independently reread and hashed: 26,972 payload files
plus the archive manifest, covering 26,615,564 payload bytes. It includes fresh
materialized sources, four admitted executables (not compiler caches), all new
retained fixtures, successes/failures, protocols and tooling. Nothing was deleted.
Readback SHA256: `c24d17d6aef7d91f43a28fa7f8698dc9c07316811bef68061f0b6c1724861700`.
`archive-readback.json` records the verified inventory and archive identity.

Platform is macOS 15.7.9 arm64. Both old and new qualification explicitly report
`native_hard_as_qualified=false`. There are no hidden unittest skips, but Linux
hard-AS/native preflight/CPU-idle/provenance qualification is still **NOT DONE**.
A later native qualification must use this exact reviewed tool, qualify both old
and new guard paths and the new smoke, and freeze its native receipts. Root owns
packaging and native execution; no macOS result may stand in for it.

Root then separately reviews/releases real F1, real F2 and (only after complete
feasibility/projection and explicit temporal-method acceptance) clean six-pair
routing. The original all-row temporal schedule stays unqualified. No routing
CPU/p99 acceptance, deployed RSS/per-bot saving, Stage B, lifecycle/absolute gate
or whole-branch Grok 4.6 acceptance is claimed here.

## Setup-CPU correction — same-card review round 2

Root and reviewer independently demonstrated the same defect in `5324f41`:
`check_release`, source/input/contract and continuation setup ran before the
supervisor CPU alarm. A 0.03-second test cap permitted 0.15 seconds of setup CPU.
The wall-only outer guard did not enforce the CPU ceiling. The original root and
reviewer reproductions, first-round archive and qualification are preserved.

The correction arms wall AND CPU guards before reading authorization. A bounded
bootstrap admits only small hash-bound root/parent receipts and the durable
non-restartable phase claim. The parent budget is then validated before heavy
work. Both alarms use the remaining cumulative allowance, including all measured
bootstrap work, before release admission, source/input hashing, native checks,
contract construction or ancestor output validation. Existing child reservation,
reap, stop, final aggregation and publication accounting stay intact. Failed setup
now also records the known cumulative budget when it has been established.

New tests exercise actual process CPU, not mocked clocks: F1 setup alarm; F2 with
0.29 seconds previously spent under a scaled 0.32 cumulative cap; and an exhausted
0.32-second continuation that must refuse heavy admission entirely. Both tests
fail on the original Git source and pass on the correction. They also assert no
child launch and no restart. Minimal synthetic parent metadata in these prefix
tests is never treated as complete native/output qualification.

Commands from the checkout (absolute `RUN` remains `sharded-run-01`):

1. `python3 diagnostics/nav-stage-a-sharded-tooling/setup_cpu_regression.py --baseline`:
   expected exit 1, three failing cases; logs `22` and `26`. Log `26` fixes only
   Python traceback source-line lookup so the printed lines come from the old
   Git source. Log `22` retains the first execution and its misleading current-file
   displayed source lines; the executed code was the original Git blob in both.
   Log `21` retains the initial failing F1 test before the implementation edit.
2. Same command without `--baseline`: exit 0, two tests pass, log `23`.
3. `python3 docs/memory/nav-tiled-stage-a/qualify_sharded.py RUN singleton-qualification-03`:
   exit 0, **19 tests pass in 90.495 s**, no unittest skips. This includes the
   complete 4/114/708 generated mocked schedule, mutation/quantile/CPU/noise
   tests and actual pipe-holding descendant cleanup. Four actual generated clean
   probe children then complete new-driver F1; all 12 historical aggregate/layout
   comparisons and unchanged `f24de7c` helper intervals pass. Log `24`.
4. `python3 docs/memory/nav-tiled-stage-a/stage_a.py guards --run ABSOLUTE_STAGE/sharded-guards-02`:
   exit 0; **17 legacy tests pass**, including between-repetition mutation.
   Receipts and stdout/stderr are in that fresh guard directory; log `25` is empty.
5. `python3 diagnostics/nav-stage-a-sharded-tooling/export_setup_correction.py`:
   exit 0; readback verifies all **209 dense / 210 tiled original Git files**, all
   four existing admissions/binaries, helper spans, complete raw aggregation,
   single-copy storage and every new archive member. Log `27`.
6. `python3 -m py_compile docs/memory/nav-tiled-stage-a/sharded.py docs/memory/nav-tiled-stage-a/test_sharded_guards.py`
   and `git diff --check`: exit 0.

The qualification's F1 CPU alarm returns after 0.037015 process CPU seconds; the
near-exhausted F2 after 0.037397 seconds. This includes signal delivery and bounded
failure serialization, not an assertion of sub-tick timer precision. The exhausted
continuation refuses heavy setup in 0.000882 seconds. The complete mock matrix
records 92.067721917 charged wall / 88.899 CPU seconds and 1,962,811 release bytes,
with exactly one input copy. These are generated controller tests, NOT router
performance, native feasibility or acceptance measurements.

Only Python supervision/tests and documentation/manifest changed. The probe,
production/helper bytes, compiler settings and four admitted binaries are unchanged;
the fresh builds and generated clean/counting qualifications from round 1 are
reused and revalidated, not rebuilt or re-admitted in place. The new qualification
binds the corrected scheduler/test hashes. The original archive and qualification
hashes above were independently rechecked unchanged. An attempted convenience
`execute_code` readback was denied by headless policy; normal `read_file` of the
already-produced readback log supplied the facts instead. No policy was changed.

Current freeze:

- Scheduler SHA256 `0794118e775f93ef27f120d41e8482d09ae27736a3bfa87970691b486c96b00e`.
- Guard tests SHA256 `d1abe14589aed5128f37bd170b35d1466866edb7c45e408a28643c293b163ae1`.
- Qualification `singleton-qualification-03/result.json` SHA256
  `d67fce681b534a7bcdcc5a6d657cafdc4bf4459754a9019eac8dfb10871e358a`.
- Proposed manifest SHA256 `1114272818a17be5b8408a7c913c15c2a0b68de6d835285bf080375be12f7c9b`.
- Correction archive `diagnostics/nav-stage-a-sharded-tooling/setup-correction-01/evidence.tar.gz`:
  **589,227 bytes**, SHA256 `57d9dd3bb608788e46ac7a6f956a7b387047e4702cdd18079cdac5ac3fb768ec`.
  All **5,538 members** rehashed (5,537 payloads, 3,279,726 bytes). It supplements,
  rather than replaces, the original source/build/evidence archive.
- Correction `readback.json` SHA256
  `02c21318f6b133d1743bcee924053dfba96fd74348b1b72ee68870d37ec54d69`.

macOS qualification still explicitly reports `native_hard_as_qualified=false`.
No native, SSH, real-pack or live execution occurred. Same-card Grok 4.5 review,
root native qualification and separate F1/F2/new-method/acceptance releases remain
required. The correction does not close any performance or whole-branch gate.

## Idle-window correction — same-card review round 3

Round 2 correctly found `native_preflight` still sleeping 0.05 seconds. The root's
preserved counter observations show why that is inadequate on its two-CPU/HZ100
target: 6 of 60 short samples fell below 90% idle, while the three fixed one-second
samples had 195/199, 194/199 and 197/199 idle ticks with zero steal. These are
root-produced historical host observations, not native work performed by this task.
Both original JSON files remain untouched; hashes and counts are in this readback.
The rejected first archive, setup correction archive and both old qualification
results were rehashed against their historical identities and remain unchanged.

The new scheduler uses fixed `NATIVE_IDLE_SECONDS=1.0`. Thresholds and method are
unchanged: a single delta of the first eight `/proc/stat` counters, idle plus
iowait at least 90%, no changed steal ticks, positive total elapsed ticks. A bad
sample fails immediately; there is no retry-until-idle or cross-sample average.
Version `stage-a-singleton-v2` binds `contract.native_idle_sample` with the exact
window, `stage-a-native-idle-v1` method schema and thresholds. Old v1 authorization
files are not upgraded or accepted in place. Raw probe schema stays v1 and all
compiled probe/helper/production bytes stay unchanged. The contract reads the
live window constant so an in-memory mutation also rejects at the next binding.

### Actual sampling cost and unchanged ceilings

Source control-flow readback found the review's one-sample-per-child estimate
understated the existing frequency. Each successful phase has one fresh entry
check, an initial bind, two binds per child, and two final aggregation binds:
`2*N+4` fixed sleeps. No call site was removed. Computed nominal idle costs are
12s F1, 232s F2, and 1420s acceptance; F1+F2 is 244s, total 1664s. Nominal room
after idle alone is 468s under the F1 wall cap, 1556s under cumulative F1+F2,
5780s under acceptance, and 7336s under the whole-release cap. The stricter F2
projection permits 960s diagnostic wall total, leaving 716s for non-idle work.

Idle alone fits, but this does NOT establish that routing plus hashing fits.
Scheduling delays, setup, repeated binding, aggregation and all supervisor/helper
CPU continue to count; there is no allowance increase or observer subtraction.
The real F2 projection can still refuse acceptance. All sampling remains inside
the already-corrected setup/global alarms, outside child route timing. A real
one-second sleep with synthetic `/proc` charged 1.073978375s wall and positive
supervisor CPU; a separate 0.08s alarm interrupted the window. This is generated
budget proof, not a native CPU-idle measurement or performance claim.

### Verification and preserved corrections

Commands ran from the campaign checkout; `RUN` is the absolute
`docs/memory/nav-tiled-stage-a/sharded-run-01` path.

1. `PYTHONPATH=docs/memory/nav-tiled-stage-a python3 -m unittest -v test_sharded_guards.Guards.test_native_idle_fixed_window_contract`:
   expected exit 1 before correction, `sleep(0.05)` instead of `sleep(1.0)`;
   log `28-idle-window-red.log`. The same command passed after correction,
   log `29-idle-window-green.log`.
2. New loaded-window mutation regression initially failed because a cached
   contract dictionary did not observe the changed execution constant. Log
   `30-idle-mutation-red.log` and its complete generated fixture are preserved.
   Contract construction now reads that constant directly. All four new tests
   passed in 1.209s; exact test names/command operands are in log
   `31-idle-tests-green.log`. They cover fixed schema/window, after-child loaded
   window mutation stopping before the partner, single-shot exact-90% boundary /
   below-threshold / zero-tick / steal rejection, and actual sleep/alarm accounting.
3. `python3 docs/memory/nav-tiled-stage-a/qualify_sharded.py RUN singleton-qualification-04`:
   exit 0, **23 tests passed in 113.196s**, no skips. The existing 19 tests include
   setup CPU, near/exactly exhausted continuations, owned descendant cleanup,
   complete 4/114/708 mocked matrix, mutation/raw/quantile/weighted CPU/noise
   coverage. Then four ACTUAL tiny-generated clean-probe children and all 12
   old/new clean/counting fixture comparisons passed. Log `32`.
4. `python3 docs/memory/nav-tiled-stage-a/stage_a.py guards --run ABSOLUTE_STAGE/sharded-guards-03`:
   exit 0, **17 legacy guard tests passed**, including between-repetition mutation.
   Fresh retained receipts; log `33` is empty.
5. `python3 diagnostics/nav-stage-a-sharded-tooling/export_idle_correction.py`:
   exit 0; log `34` records both arms' 209/210 pinned original Git files, all four
   unchanged binary/admission bindings, helper spans, raw aggregation readback,
   one input copy, AST-checked bind frequency and computed cost table. The complete
   mocked matrix used 1,962,809 release bytes and charged 113.438858458 wall /
   106.778379 CPU seconds; these are controller fixture costs only.
6. `python3 -m py_compile docs/memory/nav-tiled-stage-a/sharded.py docs/memory/nav-tiled-stage-a/test_sharded_guards.py`
   and `git diff --check`: exit 0. A convenience arithmetic `python3 -c` command
   was blocked by headless policy; the ordinary readback script computed the cost
   table instead. No policy/configuration was changed.

Only scheduler Python, tests, README, mutable proposed manifest and this report
change in the commit. The new evidence/readback script and archive live under
the task's diagnostics directory. The first-round four fresh clean/counting
builds and generated fixture results are reused/revalidated, not rebuilt or
re-admitted in place: this correction changes no compiled byte or build input.

### Current freeze and remaining limitations

- Scheduler SHA256 `8bfda30e744379d55a6abaef3dda31192487a534d0508cc41fd3a6ff0152267d`.
- Guard tests SHA256 `45dd91f94b7c9128647e26d243666fd7ea658a82dce56c4dd19b6d2b01cae15b`.
- Qualification `singleton-qualification-04/result.json` SHA256
  `9244415c4f021acd840842b724288089cf3d5ef414f47efed8f3e0c2224552ae`.
- Proposed manifest SHA256 `36fc057b0a371485420715fb339e58d11dab9eac7f4680e80124642a50d3179b`.
- Fresh archive `diagnostics/nav-stage-a-sharded-tooling/idle-correction-01/evidence.tar.gz`:
  **578,958 bytes**, SHA256 `72506e5f3626f04012eb082d2efc4d6db66469a7a8e8a574674c7db31975de7c`.
  All **5,699 members** verified, including the archive manifest; 5,698 payloads
  total 3,148,760 bytes. This supplements, never replaces, prior archives.
- Readback SHA256 `97398aec6eed79e70d0d50d035294634e6a394a48f87bdb2713a90c029c154d8`.

The platform remains macOS and `native_hard_as_qualified=false`. Synthetic
`/proc` fixtures are explicitly not Linux host qualification. No native, SSH,
real-pack, account, server or live access occurred. Root must still package and
qualify the reviewed native tool, review/release each feasibility phase, accept
the changed temporal method before any clean acceptance release, and own all
remaining performance/lifecycle/whole-branch gates. Same-card Grok 4.5 review is
requested; this report does not claim its verdict.
