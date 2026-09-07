# Native ConPTY fixture portability — review

**Review task:** t_3a5a9918  
**Change under review:** `2539cab` — Use native process readers in ConPTY wiring fixture  
**Reviewer profile:** `reviewer` (Grok-4.5)  
**Branch:** `codex/memory-diagnostics` @ `2539cab4d9a6b2bb2c0db97d26765a28962e4f73`  
**Workspace:** `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`  
**Lens:** round 1 artifact (cold read of commit + receipts; one focused Mac unit re-run)  
**Scope:** test-only fixture portability for `test_windows_conpty_starts_once_after_helpers_are_bound`. No production edits expected. No git/remote/full-build/live ConPTY managed run in this review.  
**Not claimed:** real Windows ConPTY managed-cell proof; whole-branch acceptance; archive-bit-for-bit re-hash of operator packaging outside the preserved receipt tree.

---

## Verdict

**APPROVED**

Commit `2539cab` is a single-file, test-only fix that removes hard-coded Darwin/`ps` identity sampling from the Windows delayed-start ConPTY wiring fixture. Native Windows BotTest evidence shows the same 160-test suite moving from 1 failure (this test) to OK with 7 skips after applying the blob whose SHA-256 matches the committed file. Fixture assertions still target delayed collector start and `conpty_helper_0` role wiring. Production managed-runner / ConPTY ownership code is untouched by this commit.

---

## What failed (original native e3188e2)

Preserved under `docs/memory/diagnostics/windows-helper-fixture-native/original-e3188e2/` (PowerShell UTF-16 LE logs; decoded read-only):

| Field | Value |
|-------|--------|
| `hostCommit` | `e3188e2522c4681a47e0127c8202caaa0c3b81a8` |
| `clientCommit` | `3456edc8dabf7b25ada78110ffa56327af9f67a4` |
| `testExitCode` | `1` |
| Suite | Ran 160 tests in 33.402s — FAILED (failures=1, skipped=7) |
| Failing test | `ManagedCellTests.test_windows_conpty_starts_once_after_helpers_are_bound` |
| Report status | `preflight_failed` |
| Error | `required macOS ps sample unreadable: [WinError 2] The system cannot find the file specified` |

Root cause in the pre-fix fixture (`e3188e2`): while the runner was patched to `win32`, `portable_sampler` forced `sys.platform = "darwin"`, so identity sampling took the macOS `ps` path on a Windows host. The fake parent reader also shell-called Unix `ps`.

---

## What landed (2539cab)

**Commit paths:** only `docs/memory/test_run_managed_cell.py` (+12 / −4). No production modules in the commit.

**Working-tree blob SHA-256** of that file (matches `git show 2539cab:docs/memory/test_run_managed_cell.py`):

`435bef68098816919efc41ad227a4aa372761da5d955d79cbc87b2d4f92929d9`

(Matches corrected native receipt field `testOnlyPatchSha256`.)

**Behavioral change in the fixture:**

1. Capture `native_platform = sys.platform` before mocks.
2. Bind `read_native_parent` once: real `windows_process_parent.parent_pid` on win32, else `rmc.parent_pid` (POSIX `ps`).
3. `fake_parent.parent_pid` → `portable_parent` that temporarily restores `sys.platform` to `native_platform` while calling that reader (so a later `sys.modules` mock of `windows_process_parent` does not poison the already-bound native callable).
4. `portable_sampler` sets `sys.platform = native_platform` instead of hard-coded `"darwin"`.
5. Runner still patches `rmc.sys.platform` to `"win32"` so production delayed-start / helper-required branches run.

This matches the failure mode: identity sampling and parent lookup must follow the host OS while the *under-test* Windows control flow remains simulated.

---

## Corrected native receipt

`docs/memory/diagnostics/windows-helper-fixture-native/corrected-fixture/`:

| Field | Value |
|-------|--------|
| `exitCode` | `0` |
| `sourceCommit` | `e3188e2` |
| `testOnlyPatchSha256` | `435bef68098816919efc41ad227a4aa372761da5d955d79cbc87b2d4f92929d9` |
| `user` | `BotTest` |
| Window | `2026-09-07T17:56:25Z` → `17:56:59Z` |
| Suite | Ran 160 tests in 34.424s — OK (skipped=7) |

The corrected log no longer contains the ConPTY delayed-start FAIL block present in the original log. Other lines are expected intentional FAIL/INCOMPLETE cases from negative unit tests (cadence, identity, overwrite refusal, etc.), consistent with the original log’s non-failing body and with exit 0 / skipped=7.

**Archive SHA** claimed in the task body (`1c056e57…05c6f`) was not independently reconstructed from a local tar/zip of this tree in this review. The receipt files themselves were read and content-checked; that gap is non-blocking for fixture acceptance.

---

## Fixture still meaningful?

Yes, for the contract this test owns:

| Assertion / behavior | Still present |
|----------------------|---------------|
| Simulate Windows runner path (`rmc.sys.platform = win32`) | Yes |
| `conpty_fake` launcher emits `terminal_transport=conpty` + helper meta | Yes (unchanged fixture launcher) |
| Single collector / accounting Popen after launcher | Yes (`len(accounting_calls)==1`, index order) |
| Helper role on collector argv | Yes (`conpty_helper_0=` in accounting argv) |
| Helper in `role_identities` / `conpty_helpers` receipt | Yes |
| Identity sampling uses host-native backend under the win32 runner patch | Yes (`native_platform` restore) |

**Scope boundaries (correct, not defects for this card):**

- `_capture_conpty_helpers` is still mocked with `capture_helpers`; live parent/image/identity fail-closed checks are covered by sibling unit tests (`test_conpty_helper_handoff_*`), not by this delayed-start integration fixture.
- `portable_parent` is therefore largely defensive on the current mock surface; the load-bearing fix for the native Windows failure is the sampler platform restore. Both changes are coherent and test-only.
- This unit fixture is **not** proof of a real ConPTY managed run on Windows. Root remains responsible for that separately.

Production path cross-check (unchanged by `2539cab`): Unix starts collector after launcher ownership; Windows delays until metadata; ConPTY requires helper handoff then `start_collector()` once (`run_managed_cell.py` ~1024–1062). Fixture still exercises that ordering under simulation.

---

## Production / scope check

| Check | Result |
|-------|--------|
| Commit touches only test file | Pass |
| `run_managed_cell.py` / `windows_process_parent.py` / `process_accounting.py` / `server_resources.py` in commit | No |
| HEAD is `2539cab` on `codex/memory-diagnostics` | Pass |
| Patch blob SHA matches corrected Windows receipt | Pass |

---

## Local verification (this review)

- Cold-read `git show 2539cab` and full post-change test method.
- Decoded original + corrected BotTest receipts/logs (UTF-16 LE / UTF-8 BOM).
- Confirmed blob SHA-256 of `docs/memory/test_run_managed_cell.py` = claimed patch SHA.
- Focused Mac re-run (not a full suite or live managed cell):

```text
python3 -m unittest \
  test_run_managed_cell.ManagedCellTests.test_windows_conpty_starts_once_after_helpers_are_bound -v
→ ok (1 test, ~1.1s)
```

No full builds, no live ConPTY managed runs, no git mutations, no remote actions.

---

## Caveats (non-blocking)

1. Real Windows ConPTY managed-cell proof remains an orchestrator follow-up; unit fixtures are not that proof.
2. Claimed diagnostics archive SHA not re-derived from a packaged archive in this pass; on-disk receipt contents were verified directly.
3. Mocked `_capture_conpty_helpers` means this one test does not re-validate Toolhelp parent ownership; sibling handoff tests still do.

---

## Conclusion

Approve `2539cab` as the correct, minimal, test-only portability fix for the Windows-native failure of the ConPTY delayed-start fixture. Evidence chain (pre-fix fail → patch blob → post-fix 160/OK/7 skips → Mac focused ok) is consistent. Proceed with the separate real ConPTY managed run when root schedules it.
