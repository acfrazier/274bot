# Coordinate native + exact59 correctness evidence review

**Verdict: APPROVE** (correctness-evidence prerequisite only)

Reviewer profile: `reviewer` (Grok 4.5). Round 1 artifact lens, independent
recompute from immutable local archives. No production code edited. No SSH,
native rerun, private-pack open outside archived bytes, live bot, limits change,
cleanup, or CF1 release.

## Scope and frozen base

| Item | Value |
| --- | --- |
| Workspace | `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f` |
| Branch | `codex/memory-diagnostics` |
| Frozen report commit | `8894e786c80cf158f6f7270581b96c1c0527ef3a` (ancestor of HEAD) |
| Differential tool commit | `a44a930e0b2a8bf546458fb160daa061c22a3dce` |
| Scheduler fixture commit | `113ce05525cd2bd27128bd3f420cafaad76c529c` |
| Claim under review | Fresh generated native qualification + exact59 full-byte correctness |
| Explicitly **not** approved | CF1/CF2/CA, CPU/p99/peak/RSS, Stage B, live actions, owner budgets, final branch review, performance acceptance |

Requirements read: `docs/memory/nav-coordinate-native-result-report.md`,
`docs/memory/nav-coordinate-native-admission-plan.md` (correctness/native
sections), `docs/execution.md`.

## Independent recomputes (passed)

### Source binding `7b101c30…`

Local `docs/memory/nav-tiled-stage-a/coordinate-source-binding.json` SHA256
`7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb` matches
authorization, generated root audit, Concord qualification, and the admission
plan composite:

- dense base `29b7aea779322c8611f83dc193e939ca7d756f75` (host commit present)
- refined base `8385babb23fd15b876506d4a3f6154984a6b2df1` (host commit present)
- client `3456edc8dabf7b25ada78110ffa56327af9f67a4` (present in `vendor/fr-client-rust`)
- sole overlay `crates/nav/src/collision.rs` from `ebf0f30…`
  - old blob `7446a044…` / SHA `688713da…` / 65420 bytes (verified)
  - new blob `07e0b215…` / SHA `760ae7c8…` / 65821 bytes (verified)

### Generated full-byte differential archive

| Claim | Independent result |
| --- | --- |
| Archive SHA | `baa98510e1c4044bf817e9ceffddfcee4dcf3178ce4f94ba3d37d42e291c7524` |
| Members / uncompressed | 13436 / 681424323 |
| Comparison | 328120731 bytes equal (`result.json` + root-archive-audit) |
| Protocol fixture | 80987 bytes equal |
| Inputs | 12954 |
| Source counts | dense 209 / refined 210 |
| Binding | `7b101c30…` |

### Generated real-wrapper (tiny)

Local `generated-wrapper/`: input 337 bytes; dense.out == refined.out at 80987
bytes (SHA `e605a7623ad8de34…`); `result.json` comparison equal; root-audit
`equal_bytes=80987`, verified.

### Fresh Concord after fixture correction `113ce05`

| Claim | Independent result |
| --- | --- |
| Success archive SHA | `41cd71c943b11c3c899fe66693cc98dbe83e2936007eb10698fa83f161b02681` |
| Members / uncompressed | 9999 / 24948143 |
| Progress | guards/clean/counting/integration/scheduler all rc=0 |
| Scheduler guards | Ran 29 tests in 161.868s, OK, zero skips; hard AS active; limits 300 CPU / 360 wall / 512 MiB RSS / 4 GiB AS |
| Linux comparisons | 12 records (2 variants × 3 fixtures × 2 arms); each `raw_calls=840`; refined refs historical `tiled`. Full-file SHA may differ (timing); qualify_sharded asserts behavioral fields (`aggregate`, `logical_cells`, `lookup_checksum`, `narrow_checksum`, `layout`, narrow alloc counters) before writing `qualified=True` |
| GQ children | exactly 4: `000-1-1-dense`, `001-1-1-refined`, `002-1-2-refined`, `003-1-2-dense`; smoke `completed=4`; receipts rc=0 |
| Package | tool_commit `113ce05…`; binding `7b101c30…`; archive `test_sharded.py` SHA matches git `113ce05` (`b39ee71d…`); production stage-a tools unchanged in that commit (only `test_sharded.py` + diagnostic helpers) |
| Four admissions unchanged | root audit flag true; four admission refs in q3 result |

### Failed scheduler vs fresh pass (distinct)

| | Failure | Fresh pass |
| --- | --- | --- |
| Archive | `7c500748ace8bae33fd449452df5f0b9e8377d3ec4c3a70abe8925d2728a68de` | `41cd71c9…` |
| Scheduler | returncode **-9**, wall ~302.8s, scope says failed | returncode **0**, 29/29 OK |
| Private input | none | none |

Failure evidence remains retained; it is not relabeled as a pass.

### Exact59 real full-byte correctness

Streamed all 15 members of `exact59-evidence.tar.gz` without retaining multi-GB
bodies in memory.

| Claim | Independent result |
| --- | --- |
| Archive SHA / size | `27e5f0a8d305a2dcbf4f654e7e42a1630b9af1f3fd5ffb3c141b42bdba59ccce` / 34822129 |
| Files / uncompressed | 15 / 4313089141 |
| Manifest vs stream | every path/bytes/sha256 matches |
| Pack `input.bin` | 73438581 bytes, SHA `2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30` |
| Routes TSV | 59 rows × 10 fields, ordinals 1..59, SHA `49e348ea78806c8278d720d27b171c54a0a209930fa8191ffcd38d2e9bbbc125` |
| dense.out / refined.out | each 2119776634 bytes, SHA `bbb179b7bedf32e8811db730b6fe69e069c67d9f80aca62bf14a0c577e7dd022`, hashes equal |
| Frame coverage (root audit, both arms) | fixed-selector/post-state/fixed-model/fixed-tele-model = 59; host-route+host-outcome = 59; cells=2; geometry=2; row identities match TSV; dense/refined payload SHAs equal per row |
| Result | `qualified=true`, comparison equal, both arms rc=0, hard AS active |
| Limits | wall 900 / CPU 800 / RSS 1 GiB / AS 4 GiB / output 4 GiB (unchanged) |
| Auth SHA | `f8ed2150e9d2c0eda91fdf525165a09a94011c52f1d3504c75821cd0256b038d` |
| Binding | `7b101c30…` |
| 40/19 lineage | not used to fill any current slot (auth/result free of 40-row/19-row fill) |

### Preflight / admission ordering before private input

`prepare_exact59_release.py` (archived) validates source/guards/generated
evidence, runs hardware preflight, writes `exact59-preflight.json`, then first
`admit()`/stat of the private pack and routes. Observed:

- preflight `qualified=true`, idle 100%, steal 0, no conflicts, swap 0, builder node
- preflight SHA `440d9623…` bound into authorization as `root_preflight_sha256`
- preflight `ended_unix_s` 1788998329.48 < root audit `verified_unix_s` 1788998692.25
- preflight JSON contains no private pack path

## Limits of this approval

1. **Correctness only.** Does not authorize CF1/CF2/CA, performance gates, live
   owner actions, or final branch acceptance. Root owns later CF1 release.
2. **Linux reference comparisons** match on behavioral fields required by
   `qualify_sharded.py`, not full output byte identity (timing/raw SHA differs by
   design). Exact59 and generated differential paths **do** claim full-byte equality
   and those were independently re-hashed.
3. **Root audit helpers** (`audit_exact59*.py`, etc.) support reproduction; the
   proving artifacts are the raw archives + streamed hashes + git objects.
4. **Client commit** is not in the host object store; verified in the client
   submodule. Dense/refined/overlay blobs verified in host git.
5. **Cell payload interior** of the two 65,142,784-cell frames was not fully
   re-parsed cell-by-cell; coverage is from root-audit frame inventory + full
   output SHA equality of both arms. Header tags (`geometry`, `cells`, …) present.
6. Generated archive: full 13436-member count/uncompressed + embedded
   `result.json` equality verified; not every intermediate corpus file was
   individually re-hashed beyond membership accounting (exact59 path was fully
   member-hashed).

## Evidence produced by this review

Under `diagnostics/native-nav-differential-preparation/coordinate-native-evidence-review/`:

- `phase1-outer-hashes.json` … `phase9-final-checks.json`
- Independent stream/coverage/concord scripts used for the recomputes
- This report: `docs/memory/nav-coordinate-native-evidence-review.md`

## Conclusion

Root claims at frozen commit `8894e78` for **generated native qualification** and
**exact59 full-byte correctness** under binding `7b101c30…` are independently
corroborated. **APPROVE** as the correctness-evidence prerequisite. CF1 and all
performance/live acceptance remain closed until separately released and reviewed.
