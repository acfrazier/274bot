# Windows Python collector native CLI failures — fix report

**Task:** t_5c6d1143  
**Branch:** `codex/memory-diagnostics`  
**Workspace:** `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`  
**Scope:** platform diagnostics correctness only (docs/memory Python collectors/tests + this report).  
**No git commits/pushes/merges.** Native Windows on-box re-proof is **root-owned**.

---

## Failures (from native Python 3.14 Windows: 71 tests / 2 failures / 1 skip)

### 1) `server_resources.py` as `__main__` — SampleError identity split

**Symptom:** Invalid PID (and other Win32 sample failures) escaped `main`/`run` as an uncaught traceback instead of `FAIL:` + exit 1.

**Root cause:** Script entry loads the file as `__main__`, so `except SampleError` binds `__main__.SampleError`. The Win32 path does `from windows_process_sample import …`, and that module does `from server_resources import SampleError`, which loads a **second** module object under the name `server_resources`. The raised exception is `server_resources.SampleError`, which is not an instance of `__main__.SampleError`.

Unix backends define sampling inside the same file, so they never hit the split; the bug is Windows-path + script entry.

**Fix (localized adapter translation only):**

In `_windows_sample` only: import the Windows backend’s actual `SampleError` alongside `sample_process`, catch that exact type, and re-raise **this module’s** `SampleError` (`str(exc)` / `from exc`).

`run` / `main` keep exact `except (SampleError, OSError)` and `except SampleError` — Unix catch paths are unchanged. No name-based matching, no `sys.modules` publish, no global helpers.

### 2) `process_accounting.py --help` — cp1252 `UnicodeEncodeError`

**Symptom:** `--help` crashed on Windows consoles using cp1252 when printing argparse description text containing U+2192 (→).

**Root cause:** `argparse.ArgumentParser(description=__doc__)` embeds the module docstring; three arrows in the Modes section are not encodable as cp1252.

**Fix:** Replace those three arrows in the **module docstring only** with ASCII `->`. No global `PYTHONIOENCODING` / stdout rewrap. Comments and non-CLI docstrings left alone (not printed by `--help`).

---

## Files changed

- `docs/memory/server_resources.py` — `_windows_sample` exact-type SampleError translate; `run`/`main` restored to exact SampleError catches
- `docs/memory/process_accounting.py` — module docstring arrows → `->`
- `docs/memory/test_server_resources.py` — adapter translation unit test; run path FAIL:+exit 1 / no Traceback via `_windows_sample`; keep invalid-PID CLI smoke Traceback absence
- `docs/memory/test_process_accounting.py` — `--help` + `__doc__` must encode as cp1252
- `docs/memory/windows-python-native-fix-report.md` — this report

No production shaders, Rust, or other trees touched.

---

## Tests run (this box — macOS)

```text
cd docs/memory
python3 -m unittest test_windows_process_sample test_server_resources test_process_accounting -v
```

**Result:** **73 tests, 0 failures, 3 skipped** (Windows-only live samples).

Includes targeted regressions:

- `CliSmokeTests.test_windows_sample_translates_backend_sample_error` — backend SampleError class → this module’s SampleError (identity + cause)
- `CliSmokeTests.test_windows_backend_error_through_run_prints_fail_not_traceback` — `_windows_sample` → `run` → `FAIL:` + exit 1, no Traceback, JSONL error row
- `CliSmokeTests.test_required_mid_run_failure_prints_fail` — script entry invalid PID: FAIL: + exit 1, no Traceback
- `InjectedCollectorTests.test_force_flag_absent_from_cli` — help stdout/stderr and `__doc__` encode as cp1252

**Native Windows proof:** not run here. Root should re-run the same three modules on Windows Python 3.14 and spot-check:

```text
python server_resources.py 999999991 out.jsonl --duration 0.1
# expect: exit 1, stderr starts with FAIL:, no Traceback

python process_accounting.py --help
# expect: exit 0, no UnicodeEncodeError
```

---

## Explicit non-claims

- Did not run native Windows; Mac suite is portable/injected coverage only.
- Did not change measurement acceptance, managed runners, or campaign savings math.
- Did not mask failures or weaken FAIL-closed contracts.
- Did not broaden Unix exception policy with name-based SampleError matching or sys.modules registration.
