# Bounded singleton routing tooling — implementation report

Card `t_500f341e`, branch `codex/memory-diagnostics`. Implements the tooling-only
release of `nav-stage-a-routing-resource-plan.md` (`6a2c566`), based on the
reviewed `f24de7c` diagnostic. No native/SSH/real-pack/live execution, production
optimization, cap increase, remote change or acceptance release occurred.
Independent same-card review is required.

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
