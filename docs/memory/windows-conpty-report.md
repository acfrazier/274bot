# Windows ConPTY TUI terminal transport (audit T5)

**Task:** t_f5c57aba  
**Branch:** `codex/memory-diagnostics` (verified before edits)  
**Workspace:** `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`  
**Scope owned:** `run_diagnostic.py`, `tui_input_probe.py`, new `windows_conpty.py` + `test_windows_conpty.py`, probe/diagnostic test updates, this report.  
**Not owned / not edited:** `run_managed_cell.py`, `process_accounting.py`, ownership/stop modules. No git mutations, Rust/client, remote/live/Windows invocation.

**Not claimed:** native Windows live ConPTY smoke, RDP ConPTY stability, matched TUI cells, or latency proof from probe writes. Root owns actual Windows smoke.

**Review round 1 fix:** `UpdateProcThreadAttribute` lpValue is now the HPCON handle value (`ctypes.c_void_p(hpcon)`), not `byref(local HANDLE)`. Attribute list freed after successful CreateProcess and always before ClosePseudoConsole on failure/close. Native Windows still unproven until root.

Source: Microsoft Learn — [Creating a Pseudoconsole session](https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session), [CreatePseudoConsole](https://learn.microsoft.com/en-us/windows/console/createpseudoconsole), EchoCon sample; audit T5 in `windows-managed-runner-audit.md`.

---

## Approach

Native Windows ConPTY via a small auditable **ctypes** helper (`windows_conpty.py`). No third-party package, no silent install, no winpty dependency.

Lifecycle (matches Microsoft docs / EchoCon):

1. Two synchronous pipes (input + output channels).
2. `CreatePseudoConsole(COORD{cols,rows}, inputReadSide, outputWriteSide, 0, &hPC)` — default **120×40**.
3. `InitializeProcThreadAttributeList` (size query + init) + `UpdateProcThreadAttribute(PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE=0x00020016, lpValue=HPCON handle bits, cbSize=sizeof(HPCON))` — **not** `byref` to a stack HANDLE (EchoCon / node-pty / Rust std; avoids pointer-to-temporary ABI bug).
4. `CreateProcessW` with `EXTENDED_STARTUPINFO_PRESENT` | `CREATE_UNICODE_ENVIRONMENT`, `bInheritHandles=FALSE`, `STARTF_USESTDHANDLES` with NULL std handles (avoid parent-console bypass).
5. **Immediately** `DeleteProcThreadAttributeList` after successful CreateProcess (EchoCon), before any later `ClosePseudoConsole`.
6. Close PTY-side pipe ends after create so refcounts drop.
7. Host write end → child input; dedicated drain thread on output read end (UTF-8 terminal bytes).
8. `WaitForSingleObject` / `GetExitCodeProcess` for **true** exit; `TerminateProcess` for stop.
9. After wait: `ClosePseudoConsole` so drain observes EOF (avoids documented ClosePseudoConsole deadlock), then free remaining handles. Abort/`close()` still deletes any retained attr list **before** `ClosePseudoConsole`.

Partial-failure paths close every handle still owned (attr list before HPCON, then pipes, process/thread if any).

---

## Integration

### `run_diagnostic.py`

- `require_terminal_transport()`:
  - **win32** → `('conpty', windows_conpty)` after `require_conpty_support` (fail closed if API cannot bind; never silent headless).
  - **unix** → `('unix', fcntl, pty, termios)` (unchanged lazy import).
- Real TUI terminal on Windows: `windows_conpty.spawn([binary], cwd, env, cols=120, rows=40)`.
- Metadata: `terminal_size=[120,40]`, `terminal_transport='conpty'|'unix-pty'`, `pid` from real process id.
- Drain thread continuous; probe optional via `session.input_writer()`.
- Unix PTY path (`openpty`, winsize 40×120 packed rows/cols, setsid/TIOCSCTTY, drain EIO) **unchanged**.
- Panel / `tui --headless` still plain `Popen` (no ConPTY).

### `tui_input_probe.py`

- Accepts PTY master **fd** (Unix, `os.dup` + `os.write`) **or** object with `write(bytes) -> int` (ConPTY input writer).
- Same `b'o'` stimulus bytes; cadence still gated on native `samples.qualification.jsonl` observe-start/end.
- Endpoint wording: terminal write only — **not** latency evidence (native counters remain required).

---

## Tests (Mac)

```text
cd docs/memory && python3 -m unittest test_windows_conpty test_tui_input_probe test_run_diagnostic -v
Ran 35 tests — OK
```

Coverage:

| Area | What |
|------|------|
| Constants / env / quoting | `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`, UTF-16LE env block, `list2cmdline` |
| Spawn contract (injected API) | 120×40, flags, inherit false, STARTF_USESTDHANDLES, **lpValue=HPCON bits**, attr delete after CreateProcess, PTY-side close, pid/wait/write/read |
| Failure ownership | CreatePseudoConsole / UpdateAttribute / CreateProcessW failures close pipes+HPCON+attrs; **DeleteProcThreadAttributeList before ClosePseudoConsole** |
| close() order | Retained attr list deleted before ClosePseudoConsole |
| InputProbe | existing Unix PTY tests + ConPTY-style writer writes `b'o'` / restore |
| Diagnostic | lazy Unix import, win32 selects ConPTY helper (or fail-closed off-Windows), unix path loads |

No live ConPTY process on this Mac host — by design. Root runs Windows smoke.

---

## Explicit non-claims / remaining

- **Native Windows unproven** until root smoke under BotTest/RDP.
- No managed-cell / stop / parent-PID edits (other cards).
- Probe write ≠ latency; qualification boundaries still from binary.
- Gates (metadata, binary, provenance, headless rejection for real TUI) not weakened: unsupported ConPTY still errors; headless remains explicit `--headless` only.
