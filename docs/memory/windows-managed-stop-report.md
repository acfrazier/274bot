# Windows managed runner — portable collector stop and owned-process cleanup (B1–B3)

**Task:** t_f1304afa  
**Branch:** `codex/memory-diagnostics` (verified before edits)  
**Workspace:** `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`  
**Scope:** B1 collector stop IPC, B2 portable cleanup, B3 frontend parent ownership only.  
**Not done here:** B4+ (PTY/diagnostic import, HOME, server placement, live Windows cell). No git mutations. Native Windows proof remains root-owned.

## What changed

| Item | Files |
|------|--------|
| B1 portable graceful stop | `docs/memory/process_accounting.py`, `docs/memory/run_managed_cell.py` |
| B2 portable owned cleanup | `docs/memory/run_managed_cell.py` |
| B3 parent ownership | `docs/memory/windows_process_parent.py` (new), `run_managed_cell.parent_pid` |
| Tests | `test_process_accounting.py`, `test_run_managed_cell.py`, `test_windows_process_parent.py` (new) |

### B1 — Collector stop file IPC

- Collector accepts `--stop-file ABS_PATH`. Path must be absolute, parent dir present, and **not** already exist at start (stale → fail closed).
- Loop polls file presence in the same 50 ms sleep slices as stop_event/signals.
- `stop_controlled` + file create → `status=closed`, `completion=controlled_stop`, exit **0** (no Unix signals required).
- Metadata records `stop_file` and `stop_channels`.
- Managed runner places stop path at `{cell_dir}/collector.stop` (exclusive cell-contained), passes it on collector argv, and exclusive-creates the file on every normal stop and exception cleanup path.
- **Unix:** still sends SIGTERM after creating the stop file (handlers preserved).
- **Windows:** does **not** use `send_signal(SIGTERM)` as graceful stop (that is TerminateProcess). Escalation remains `Popen.kill()` after cleanup wait.

### B2 — Portable cleanup signals

- `_cleanup_frontend` no longer builds `(signal.SIGTERM, signal.SIGKILL)` (SIGKILL undefined on win32).
- Stages: soft `SIGTERM`, hard `SIGKILL` on Unix / `TERMINATE` label + TerminateProcess-class `os.kill(SIGTERM)` on win32.
- Direct owned children: soft TERM then `Popen.kill()`; Windows owned children escalate via `kill()` without referencing SIGKILL.
- Server/ambient never signaled (unchanged).

### B3 — Parent ownership

- `parent_pid(pid)`: Unix keeps `ps -o ppid=`; win32 uses `windows_process_parent.parent_pid` (Toolhelp32 snapshot, explicit PID only — no name scan).
- Capture still requires `parent == launcher_pid`.
- Cleanup: while launcher is **alive**, recheck parent==launcher before each escalation; after launcher death, identity-only (reparent to init is expected for orphan frontend kill).
- Stable `start_identity` still required before every stage.

## Verification (Mac)

```text
python3 -m unittest docs.memory.test_process_accounting \
  docs.memory.test_windows_process_parent \
  docs.memory.test_run_managed_cell
→ Ran 75 tests … OK
```

Notable new coverage:

- stop-file → controlled_stop / exit 0 without signals  
- stale/relative/missing-parent stop path fail closed  
- managed cell writes cell-local `collector.stop` and completes controlled_stop  
- win32 hard-label path never needs `signal.SIGKILL`  
- parent ownership reject foreign launcher; dummy child parent==self  
- cleanup refuses signal when launcher alive and parent mismatch  
- existing TERM-ignoring frontend still escalates to SIGKILL on Unix  

## Explicit limits

- No remote/live Windows managed cell; root owns native proof (T10).
- No `run_diagnostic` / PTY / HOME / B5–B7 work in this card.
- Cross-host Mac server + Windows frontend accounting still unsupported (unchanged policy).
- Windows Toolhelp live path unexercised here; deterministic fake API tests only.

## Handoff

Implementation complete for B1–B3 source + Mac unit tests. Request same-card review (`reviewer`). Do not mark performance acceptance or Windows matched-cell readiness.
