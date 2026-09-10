# Coordinate native evidence — corrective supplement

**Verdict: APPROVE** (corrective independent checks only)

Profile: `reviewer` (Grok 4.5). Corrective card `t_5159d6b4` for verification
gaps withheld under CF1 on closed parent `t_518edea2` / `f13d682`. Parent limited
APPROVE is preserved. No production/test changes, no native/SSH/live/private
input outside archived bytes, no caps or STATE edits.

## Why this card exists

Root accepted original exact59 full-member hashing and recorded scope limits, but
withheld CF1 because:

1. Prior `phase7_concord_detail.py` checked the root-audit `four_admissions_unchanged`
   flag and cited qualifier assertion existence for 12 comparisons rather than
   independently re-parsing every raw pair.
2. Prior `phase3_source_binding.py` verified two collision blobs and commit
   existence, not every effective original-source entry / binary admission.
3. Prior generated-archive review counted members and equality claims but did not
   stream-hash every member against the embedded inventory.

This supplement performs only those missing independent checks.

## Immutable inputs

| Artifact | SHA256 |
| --- | --- |
| Generated archive `coordinate-native-generated-01.tar.gz` | `baa98510e1c4044bf817e9ceffddfcee4dcf3178ce4f94ba3d37d42e291c7524` |
| Concord success `coordinate-concord-qualified-03.tar.gz` | `41cd71c943b11c3c899fe66693cc98dbe83e2936007eb10698fa83f161b02681` |
| Source binding | `7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb` |
| Frozen parent report commit | `8894e786c80cf158f6f7270581b96c1c0527ef3a` |
| Differential tool | `a44a930e…` |
| Scheduler fixture tool | `113ce055…` |

Workspace: `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f` branch
`codex/memory-diagnostics`.

## Checks completed (all PASS)

### (1) Generated archive — every member

Streamed `coordinate-native-generated-01.tar.gz` (`r|gz`) and hashed **all 13436**
members against `coordinate-native-generated-01-archive.json` (path after strip of
prefix `coordinate-native-generated-01/`, size, sha256). Results:

- Outer SHA and 18,765,670 bytes match expected.
- Uncompressed sum 681,424,323; zero missing/extra/mismatch.
- `corpus.json` 12,954 entries: every path/bytes/sha256 matches stream.
- `result.json` output_hashes, dense==refined probe and protocol outs verified
  against stream hashes.
- Launch tools match stream hashes and `git show a44a930:docs/memory/nav-tiled-differential/…`.
- Bodies retained only under 16 MiB for later logical checks (no multi-GB cell payload).

### (2) Concord archive — all 9999 members

Streamed `coordinate-concord-qualified-03.tar.gz` against
`coordinate-concord-qualified-03.manifest.json`:

- Outer SHA `41cd71c9…`; 9999/9999 members; 24,948,143 uncompressed; zero mismatch.
- Receipt SHA/files count and qualification object match.

### (3) Four stage admissions — package + sources + executables

Independently (not via root-audit flag):

| Admission | package-manifest SHA match | source paths hashed | executable |
| --- | --- | --- | --- |
| dense-clean | `a7b41eca…` | 226 / 0 bad | `…/dense/target-clean/release/stage-a-probe` |
| dense-counting | `b94f3f2a…` | 226 / 0 bad | `…/dense/target-counting/release/stage-a-probe` |
| refined-clean | `73366a7e…` | 227 / 0 bad | `…/refined/target-clean/release/stage-a-probe` |
| refined-counting | `d5346685…` | 227 / 0 bad | `…/refined/target-counting/release/stage-a-probe` |

Each admission file bytehash equals both the Concord archive member and the
original `package-manifest.json` entry; `source_binding` equals q3 binding;
every `source[path]` hash matches archived tree bytes; sole `stage-a-probe` under
`target-{variant}/release/` matches `executable_sha256`.

### (4) Every production effective original-source entry from Git

Reconstructed from dense base `29b7aea7…`, refined base `8385babb…`, client
`3456edc8…` (submodule), sole collision overlay blob `07e0b215…`:

- Dense: **209/209** original entries — git blob id, original sha/size, effective
  manifest sha/size, archived payload == effective bytes (+ adapter-only suffixes
  on `crates/nav/Cargo.toml` and `crates/nav/src/router.rs` only).
- Refined: **210/210** with exactly one overlay on `crates/nav/src/collision.rs`.
- Host span extracts verified separately: `original-host-lib.rs` ==
  `git show {base}:crates/host-play/src/lib.rs`; span piece hashes; rebuilt
  `host-probe.rs`.
- Admission source maps (221 dense / 222 refined paths including non-manifest
  materialization files) all match archive stream hashes.
- Orig/eff path sets equal per arm; base commits align with binding.

### (5) Twelve raw comparison pairs — field-by-field

For each of 12 q3 comparisons (2 variants × 2 arms × 3 fixtures):

- new/old `.out` archive bytes match claimed `new_sha256` / `old_sha256`.
- Last JSON line of each pair compared on:
  `aggregate`, `logical_cells`, `lookup_checksum`, `narrow_checksum`, `layout`,
  `narrow_allocations`, `narrow_requested_bytes` — all equal.
- `raw_elapsed_ns` length **840** and `raw_calls==840` on every record.
- `narrow_allocations` and `narrow_requested_bytes` are 0 on every new summary.

Also verified independently (supporting, not flag-only):

- Scheduler guards: rc=0, hard AS active, limits 300/360/512MiB/4GiB, stderr
  `Ran 29 tests` + `OK`, no `skipped`.
- Smoke children order exactly
  `000-1-1-dense`, `001-1-1-refined`, `002-1-2-refined`, `003-1-2-dense`;
  completed=4; record hashes match.
- GQ receipts returncode 0 (12 receipt files observed).
- Progress 5 stages all rc=0.
- Tools in q3 match `git show 113ce05:docs/memory/nav-tiled-stage-a/…`.

## Method note

Root audit scripts (`audit_archive.py`, `audit_coordinate_concord_03.py`) were
inspected for format/schema only after the checks above were designed. Supplement
scripts re-implement the byte/git comparisons; they do not treat
`four_admissions_unchanged=true` or `qualified=True` as proof.

## Limits (unchanged / still closed)

1. Does **not** approve CF1/CF2/CA, performance, live owner actions, or final branch review.
2. Linux reference comparisons remain behavioral-field equality (timing SHAs may differ by design); exact59 full-byte path was already accepted by parent and is out of this card’s corrective scope.
3. Cell payload interiors of multi-GB frames were not re-parsed cell-by-cell (no multi-GB extract); generated path proves member inventory + output equality hashes only.
4. Adapter-only suffixes and host-span extracts are verified separately from production effective original-source entries.
5. No probe reruns.

## Evidence produced

Under `diagnostics/native-nav-differential-preparation/coordinate-native-evidence-supplement/`:

- `check1_and_4_generated_sources.py`, `check2_3_5_concord.py`, `write_verdict.py`
- `check1-generated-members.json`, `check4-source-reconstruction.json`, `check1-and-4-full.json`
- `check2-concord-members.json`, `check3-admissions.json`, `check5-comparisons.json`, `check2-3-5-full.json`
- run logs, `verdict.json`
- This report: `docs/memory/nav-coordinate-native-evidence-supplement.md`

## Conclusion

The five missing independent prerequisite checks from the original correctness
evidence card are now proven directly from archived bytes and Git objects.
**APPROVE** this corrective supplement. Root still owns any later CF1 release
decision; performance and live acceptance remain closed.
