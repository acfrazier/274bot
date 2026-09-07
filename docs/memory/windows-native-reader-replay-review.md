# Native Windows cache identity fixture + panel reader replay — review

**Review task:** t_af39df26  
**Change under review:** `a0bca8d` — Exercise native Windows cache identity without symlink privileges  
**Reader commit referenced:** `2539cab4d9a6b2bb2c0db97d26765a28962e4f73`  
**Reviewer profile:** `reviewer` (Grok-4.5)  
**Branch:** `codex/memory-diagnostics` (a0bca8d is ancestor of HEAD `cf263a0`)  
**Workspace:** `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`  
**Lens:** round 1 artifact (cold read of commit + diagnostics; focused Mac BindingTests re-run only)  
**Scope:** root test fixture correction for filesystem cache identity on hosts without symlink privilege; independent reader replay of frozen panel b4 receipt. No production edits expected. No git mutations, full builds, or new live runs in this review.  
**Owned artifact:** this file only.

---

## Verdict

**APPROVED**

Commit `a0bca8d` is a minimal, test-only fix to `test_cache_directory_uses_filesystem_identity`: on `win32` it feeds the real extended-length (`\\?\`) spelling of the cache directory into `cache_dir_canonical` instead of creating a directory symlink (which fails with WinError 1314 for the standard BotTest account). Non-Windows keeps the symlink alias. Positive same-directory and negative distinct/missing checks remain. Native BotTest evidence shows the same 49-test run moving from 1 ERROR to OK with 1 skip. Separate reader-tools replay of frozen panel `b4b686f` / `native-panel-n1-a` with reader commit `2539cab` independently bounds with `qualified=true`, `match_keys_missing=null`, and `managed_resources.status=available`, without claiming performance acceptance. Original panel receipt bytes are unchanged; derived binding is a separate artifact.

---

## What failed (original native suite under reader-tools-2539cab)

Preserved under `docs/memory/diagnostics/windows-native-reader-2539cab/reader-replay-2539cab/` (PowerShell UTF-16 LE logs):

| Field | Value |
|-------|--------|
| `readerCommit` | `2539cab4d9a6b2bb2c0db97d26765a28962e4f73` |
| `binarySource` | `b4b686f` |
| `user` | `BotTest` |
| `testExitCode` | `1` |
| `replayExitCode` | `0` (panel replay still succeeded in this window) |
| Suite | Ran 49 tests in 5.475s — FAILED (errors=1, skipped=1) |
| Failing test | `BindingTests.test_cache_directory_uses_filesystem_identity` |
| Error | `OSError: [WinError 1314] A required privilege is not held by the client` on `alias.symlink_to(...)` |

Root cause: fixture assumed symlink creation; BotTest cannot create symlinks.

---

## What landed (a0bca8d)

**Commit paths:**

| Path | Role |
|------|------|
| `docs/memory/test_managed_resource_binding.py` | Fixture fix (+7 / −2 on the cache-identity test) |
| `docs/memory/conpty-native-fixture-review.md` | Prior ConPTY fixture review write-up (docs only; not this card’s subject) |

**No production modules** in the commit (`managed_resource_binding.py` unchanged here).

**Working-tree / commit blob SHA-256** of `docs/memory/test_managed_resource_binding.py`:

`a6c182b0e3f9c16e7f1366635ae24027fa82eb36e6a5629fbd5d0cd795ddc87d`

Matches `docs/memory/diagnostics/windows-native-reader-2539cab/provenance.json` field `corrected_test_sha256`. Worktree file equals `git show a0bca8d:...` bytes.

**Behavioral change in the fixture:**

1. `win32`: `alias = pathlib.Path('\\\\?\\' + str(self.cache.resolve()))` — source bytes confirmed via `od -c` as four/two backslash pairs → runtime prefix `\\?\` (extended-length path), not a symlink.
2. else: unchanged `cache-alias` directory symlink.
3. Positive: bind with aliased `cache_dir_canonical` → `available`.
4. Negative: distinct existing directory → `unavailable`; missing path → `unavailable`.

Production binder still compares directories with `os.path.samefile` via `_same_directory_identity` (`managed_resource_binding.py`), so the fixture continues to exercise filesystem identity rather than string equality. On Windows it additionally exercises the extended-path producer spelling the native stack actually emits.

---

## Corrected native suite (path fixture only; no second panel replay)

`docs/memory/diagnostics/windows-native-reader-2539cab/reader-replay-native-path-fixture/`:

| Field | Value |
|-------|--------|
| `testExitCode` | `0` |
| `replayExitCode` | `null` (no second replay; intentional) |
| `readerCommit` | `2539cab4d9a6b2bb2c0db97d26765a28962e4f73` |
| `binarySource` | `b4b686f` |
| `user` | `BotTest` |
| Window | `2026-09-07T18:04:59Z` → `18:05:04Z` |
| Suite | Ran 49 tests in 5.132s — OK (skipped=1) |

Provenance note matches: original failed suite + first panel replay preserved; second suite is corrected fixture tests only.

---

## Reader replay of frozen panel b4 (2539cab tools)

**Derived output:**  
`docs/memory/diagnostics/windows-native-reader-2539cab/reader-replay-2539cab/independent-binding-reader-2539cab.json`

**Replay log summary** (`replay.log`, UTF-16 LE):

```text
status=bound, qualified=true, match_keys_missing=null, final_acceptance_claim=false
native_qualification status=available
managed_resources status=available
```

**Cross-check vs original frozen panel binding**  
(`docs/memory/diagnostics/windows-native-panel-b4b686f/managed-panel-b4b686f-a/independent-binding.json`):

| Field | Original panel binding | Reader-2539cab derived binding |
|-------|------------------------|--------------------------------|
| `receipt_id` | `native-panel-n1-a` | `native-panel-n1-a` |
| `run_identity.identity_sha256` | `4264c458c95ad2608cd68ffc97ca0a8872d8dba8469a8473fea21000b2a8572d` | same |
| raw artifact hashes (metadata/samples/qualification) | present | same triple |
| `binary_sha256` | `8ccdc44b5c2ebe4b3e0b7030b4087d4e5a1d55842a7dcc8e466f410e54b8d512` | same |
| `status` / `binding_ok` / `qualified` | bound / true / true | bound / true / true |
| `match_keys_missing` | `"terminal_size"` | `null` |
| `managed_resources.status` | `unavailable` (`native cache directory differs from fingerprint`) | `available` |
| `final_acceptance_claim` / `performance_acceptance` | false | false |

**Original receipt unchanged:**  
`cells/native-panel-n1-a/receipt.json` SHA-256  
`333125262360dee1bb627da2790d71c99eb4c475ac8e4ea9a03f0cb05a3d1a0f`  
(commit `a0bca8d` does not touch panel diagnostics). Derived binding is a separate file; it does not rewrite the receipt.

Interpretation: the frozen run’s *original* independent-binding left managed resources unbound on cache path identity and reported a panel `terminal_size` missing-key note. The later reader-tools replay (same receipt / run identity / binary) independently re-binds with full match keys and available managed resources, still without performance acceptance. That is exactly the claimed “independent binding” outcome, not a mutation of the original receipt.

---

## Provenance / archive

`docs/memory/diagnostics/windows-native-reader-2539cab/provenance.json`:

| Field | Value | Verified |
|-------|--------|----------|
| `reader_commit` | `2539cab4d9a6b2bb2c0db97d26765a28962e4f73` | matches completion JSON + git |
| `corrected_test_commit` | `a0bca8d` | matches |
| `corrected_test_sha256` | `a6c182b0…ddc87d` | matches file and `git show a0bca8d` blob |
| `archive_sha256` | `6f2efbb96b9617a4a8d05484f54c67cf49ac14fd034e8b8ce159541519130d1f` | **not** re-derived from a local tar/zip of the in-tree directory (same class of non-blocking gap as prior native reviews) |

On-disk evidence files (fail log, corrected log, replay log, derived binding, completions) were content-checked directly. Logs are UTF-16 LE PowerShell captures as claimed.

---

## Local verification (this review)

- Cold-read `git show a0bca8d` and full post-change `test_cache_directory_uses_filesystem_identity`.
- Confirmed extended-path string literal bytes (`od -c`) → `\\?\` prefix.
- Confirmed blob SHA-256 matches provenance `corrected_test_sha256`.
- Read original fail + corrected suite + replay logs and both independent-binding JSON artifacts.
- Confirmed production `_same_directory_identity` still uses `os.path.samefile`.
- Focused Mac re-run (not a full build or live cell):

```text
python3 -m unittest test_managed_resource_binding.BindingTests -v
→ Ran 10 tests in 0.078s — OK
```

(Matches task claim that the Mac 10-test BindingTests module passes; initial wrong class name would have run nothing.)

No full cargo builds, no new live managed runs, no git mutations, no remote actions. Reviewer did not edit production or test implementation files.

---

## Scope / production check

| Check | Result |
|-------|--------|
| Fixture fix is test-only | Pass |
| `managed_resource_binding.py` / runners / client in `a0bca8d` | No |
| Positive + two negative cache identity asserts retained | Pass |
| Non-win32 symlink path retained | Pass |
| Panel replay claims performance acceptance | No (`final_acceptance_claim=false`, `performance_acceptance=false`) |
| Second suite re-ran panel replay | No (`replayExitCode=null`) |

**Non-blocking notes:**

1. `a0bca8d` also lands `conpty-native-fixture-review.md` (prior card’s review doc). Harmless docs bundling; not part of the cache-identity contract.
2. Claimed diagnostics `archive_sha256` not recomputed from a packaged blob in this pass; file-level contents verified.
3. Original panel independent-binding’s `managed_resources.unavailable` / `match_keys_missing=terminal_size` is historical bind output; the approved claim is the *derived* reader replay, not that the first bind already had managed resources.

---

## Conclusion

Approve `a0bca8d` as the correct, minimal, test-only Windows portability fix for cache filesystem-identity coverage without symlink privileges, with consistent native evidence (49-test ERROR→OK/skip=1) and a successful independent 2539cab reader replay of frozen panel b4 (`bound` / `qualified` / no missing match keys / `managed_resources` available, no performance acceptance). Mac BindingTests (10) re-confirmed OK.
