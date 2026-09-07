# Windows managed-native follow-up (test failures at host 0dc87d7)

**Task:** t_9a82e7d4  
**Branch:** `codex/memory-diagnostics`  
**Workspace:** `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`  
**Frozen evidence:** `docs/memory/diagnostics/windows-managed-native-0dc87d7/python-managed-0dc87d7/`  
(UTF-16 `tests.log`, UTF-8 `tests-decoded.txt`, `receipt.json` — 108 ran, 12 fail, 2 errors, 3 skips on WinPython 3.14.6)

## Scope

In scope: `run_managed_cell.py`, `windows_process_sample.py` (liveness helper), `test_run_managed_cell.py`, `test_managed_resource_binding.py`, `test_windows_process_sample.py`, this report.

Out of scope (unchanged / root-owned): git/remote/native Windows re-run, live/server/process actions.  
`run_diagnostic.py` / `test_run_diagnostic.py` / ConPTY / inputprobe — owned by t_f5c57aba (6 of the 12 failures were CLI help/stdout empty; not fixed here).

## Failure classes (from frozen log) → disposition

| Class | Tests | Root cause | Fix |
|-------|-------|------------|-----|
| `_alive` reports dead launcher alive + temp rmtree WinError 32 on `launcher.log` | `test_successful_observation_then_delayed_teardown`, `test_deadline_cleanup_owned_children_only` (+ ERROR cleanup) | `_pid_alive` / test `_alive` used `os.kill(pid, 0)`. On Windows, signal value **0 is `CTRL_C_EVENT`** and routes through `GenerateConsoleCtrlEvent` (console-group side effects); it is not a POSIX no-op liveness probe and does not match Win32 exited-but-handle-open semantics. Stale “alive” also left child handles on `launcher.log` → sharing violation on temp teardown. | `windows_process_sample.process_is_alive`: `OpenProcess(QUERY_LIMITED\|SYNCHRONIZE)` + zero-timeout `WaitForSingleObject`. `run_managed_cell._pid_alive` uses it on win32. Tests call `rmc._pid_alive`. |
| Orphan fixture expects `SIGKILL` | `test_launcher_exit_does_not_leave_term_ignoring_frontend` (+ ERROR file lock) | Production already labels hard stage `TERMINATE` on win32 (`SIGKILL` undefined). Soft stage on Windows is already `TerminateProcess` (fatal); Unix-only `SIGTERM` ignore fixture is inappropriate. | Fixture: ignore SIGTERM only when `sys.platform != 'win32'`. Assert hard label via `rmc._hard_signal_label()` (SIGKILL on Unix; SIGTERM/TERMINATE on Windows). |
| Early frontend exit 42 lost in receipt | `test_early_frontend_fail_stops_collector_no_retry` | Likely cascade from `os.kill(0)` CTRL_C hitting launcher console group before final `metadata.json` write (`exit_code` never persisted → receipt `None`). | Same liveness fix; no receipt-schema change. Mac still asserts `exit_code==42`. |
| Module set Mac-only literal | `test_sampler_modules_bound_and_backend_identity_consistent` | On win32 producer, `_build_sampler_config` correctly pins `windows_process_sample.py`; test expected only accounting+server_resources. | Expected set includes Windows module when `sys.platform=='win32'`. |
| Binder assert reader ≠ win32 | `test_windows_dependency_included_binds_on_any_reader_platform` | Test skipped itself on Windows via `assertNotEqual(sys.platform,'win32')` despite “any reader platform” contract. | Removed platform guard; binding still requires Windows identity modules from producer evidence. |

## Production rules preserved

- Identity + parent-before-escalation while launcher alive; identity-only after launcher death.
- Clean **owned** children only; never signal game_server / ambient helpers.
- No broadened name/argv process matching.
- Stop-file graceful collector path unchanged; Unix still may SIGTERM after stop-file; Windows does not use `send_signal(SIGTERM)` as graceful (TerminateProcess).
- Existing stop-file / evidence gates / timeouts unchanged.

## CLI diagnostic failures (hypothesis only — not fixed)

All six `test_run_diagnostic.RunDiagnosticCli.*` failures show empty `help_proc.stdout` and/or non-zero return on `--help`. Hypotheses for t_f5c57aba / root:

1. Import-time hard dependency on `pty`/`fcntl`/`termios` still aborts before argparse help on stock win32 (B4 from prior audit), **or**
2. ConPTY/path work partially landed but help subprocess captures wrong stream / encoding, **or**
3. Subprocess invocation under PowerShell remote wrapper swallowed stdout (receipt ran under BotTest workspace).

Do not treat managed-cell green as diagnostic CLI green.

## Verification (this host — Mac)

```text
cd docs/memory && python3 -m unittest \
  test_windows_process_sample test_windows_process_parent \
  test_run_managed_cell test_managed_resource_binding -v
→ Ran 69 tests in ~48s — OK (skipped=3 live Windows-only)
```

Native Windows re-validation remains **root-owned** after reviewed change (do not claim matched acceptance).

## Files changed

- `docs/memory/windows_process_sample.py` — `process_is_alive`
- `docs/memory/run_managed_cell.py` — win32 `_pid_alive`
- `docs/memory/test_run_managed_cell.py` — portable asserts, orphan fixture, `_alive`, new liveness unit test
- `docs/memory/test_managed_resource_binding.py` — drop Mac-only platform assert
- `docs/memory/test_windows_process_sample.py` — injected liveness cases
- `docs/memory/windows-managed-native-followup.md` — this report

## Limits / non-claims

- No performance acceptance, no matched baseline/candidate Windows cell claim.
- No native Windows re-run in this card.
- No edits to `run_diagnostic.py` / ConPTY helpers.
- Report limits: no matched acceptance.
