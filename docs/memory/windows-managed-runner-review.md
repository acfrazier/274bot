# Windows managed-runner audit — Grok-4.5 review

**Review task:** t_0112cec2  
**Audit under review:** `docs/memory/windows-managed-runner-audit.md` (task t_20fe5d5c)  
**Reviewer profile:** `reviewer` (Grok-4.5) — required profile pass (prior t_20fe5d5c / run 268 @implementer is not accepted as this review)  
**Branch:** `codex/memory-diagnostics` @ `63c8baf` (not `main`)  
**Workspace:** `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`  
**Scope:** read-only verification of the audit against currently referenced sources under `docs/memory/`. No source/git/remote/live/build/UI mutations. This file is the only write.  
**Not claimed:** savings, matched baseline/candidate acceptance, full Windows qualification, or readiness-green beyond the audit’s own bounded verdict.

---

## Verdict

**APPROVED**

The audit’s one-line verdict and ordered blockers match the current managed-path sources. Fail-closed gaps (stop channel, cleanup/SIGKILL, parent ownership/`ps`, diagnostic PTY import, HOME/unpack lag, missing `windows_process_sample.py` module bind, same-host `game_server` PID / cross-host non-support) are source-backed. Next-task graph T1–T10 is actionable and does not weaken gates. Explicit unknowns and “not claimed” boundaries are preserved. No savings or readiness-green overclaim.

Prior implementer self-pass on t_20fe5d5c is superseded by this required-profile review only for audit acceptance; final whole-branch Grok-4.6 remains later.

---

## Method

Cold-read audit, then re-checked the named modules on this branch:

| Artifact | Checked |
|----------|---------|
| `run_managed_cell.py` | import of `run_diagnostic`; `expected_unpack_root`; `_pid_alive`; `_terminate_owned`; `_cleanup_frontend`; `_capture_owned_frontend`; `_build_sampler_config` / module set; collector `send_signal(SIGTERM)`; `game_server_pid` / ambient; `require_argv_consistent_with_spec`; outer `except Exception` cleanup |
| `process_accounting.py` | lazy `resource`; SIGINT/SIGTERM handlers; `stop_event` in-process API; `controlled_stop` / exit 0; children rusage unavailable path |
| `server_resources.py` | `win32` → `_windows_sample`; pressure `unavailable` / `unsupported_pressure_counter` |
| `windows_process_sample.py` | OpenProcess rights; WorkingSetSize; GetProcessTimes; WaitForSingleObject(0); `windows_creation_filetime:`; zero-WS fail closed |
| `managed_receipt.py` / `managed_resource_binding.py` / `process_evidence.py` | sampler exit 0; module expected set; `game_server` identity; backend ∈ {system,libproc}; stop_controlled → controlled_stop; `performance_acceptance` false; pressure unassessed |
| `run_diagnostic.py` / `tui_input_probe.py` | top-level `fcntl`/`pty`/`termios`; PTY path; non-PTY `Popen`; `RS2B0T` default; catalog `HOME`; default binary without `.exe` |
| Backdrop docs (status only) | `windows-native-first-proof.md`, `windows-home-path-report.md`, `windows-process-metrics-report.md` |

No live Windows re-run in this review (task forbids live/build). B2 `os.kill(pid, 0)` acceptability is accepted from the task’s stated root probe on owned WinPython 3.14.6 child (ready-stdout path; earlier init-crash child not used as semantics proof) plus audit wording that limits the claim to modern CPython Windows — not a general “unsupported OS” extrapolation.

---

## Blocker verification (source vs audit)

### B1 — Collector stop / stop_controlled (critical) — CONFIRMED

- Managed runner stops the collector with `collector.send_signal(signal.SIGTERM)` after observe-end pad and on launcher-done (`run_managed_cell.py` ~826, ~853).
- Sampler config always `duration_mode: "stop_controlled"` and `duration_s_requested: None` (`_build_sampler_config`).
- `process_accounting.run` installs SIGINT/SIGTERM handlers; stop_controlled + orchestrator stop → `completion=controlled_stop`, exit 0.
- Library already accepts an in-process `stop_event`, but the **managed child** path does not pass argv/file/stdin/Event IPC that would set a stop in the collector process. Audit phrasing “no stdin/event/file stop channel exists” is correct for the managed path; T1 correctly adds a portable stop channel rather than relying on `send_signal(SIGTERM)` as graceful stop.
- **Inference (not live-proved here):** CPython Windows maps `Popen.send_signal(SIGTERM)` to TerminateProcess, so handlers/summary/`controlled_stop` do not run; receipt then hits `sampler_failed_or_incomplete` (`managed_receipt.complete` requires sampler `exit_code == 0`) and `process_evidence` rejects non-`controlled_stop` series. Mechanism is standard CPython + source contracts; this review did not re-execute a Windows managed cell.

### B2 — Frontend cleanup `SIGKILL` (critical) — CONFIRMED

- `_cleanup_frontend` builds `(signal.SIGTERM, signal.SIGKILL)` then `os.kill(pid, sig)`.
- `signal.SIGKILL` is absent on win32 → `AttributeError` when evaluating that tuple if a live owned frontend is being cleaned.
- Happy-path cleanup sits inside the broad `except Exception` of `run()` → failed report. Exception-path cleanup re-invokes the same helper (~995), so a second `AttributeError` can escape the handler if a frontend PID is still set — slightly harsher than “caught → failed report,” still a cleanup portability blocker, not a false alarm.
- `_pid_alive` via `os.kill(pid, 0)`: audit’s “acceptable on modern CPython Windows” matches the task root probe; do not treat that probe as a general OS-support claim beyond modern CPython.
- Soft `os.kill(..., SIGTERM)` / `send_signal(SIGTERM)` as TerminateProcess-class behavior on Windows is correctly distinguished from a graceful Python signal path.

### B3 — Frontend parent ownership via `ps` (critical) — CONFIRMED

- `_capture_owned_frontend` runs `['ps', '-o', 'ppid=', '-p', str(pid)]` and requires `ppid == launcher_pid`, else `CellError('cannot establish frontend parent ownership')`.
- No Win32 parent-PID path in-tree. T3 (explicit PID→parent, fail-closed child-of-launcher) is the right fix shape.

### B4 — `run_diagnostic` Unix TTY import / PTY (critical) — CONFIRMED (stronger than “child only”)

- `run_diagnostic.py` line 4: unconditional `import errno, fcntl, pty, struct, termios, threading`.
- Terminal path: `pty.openpty`, `TIOCSWINSZ`, `os.setsid`, `TIOCSCTTY`, `preexec_fn`.
- Non-terminal branch is plain `Popen` to log (no PTY) — but **module import still requires** the Unix modules.
- **Amplification (source, consistent with audit cascade):** `run_managed_cell.py` does `import run_diagnostic as rd` at load time (~29) for parser/argv/`rd.__file__` checks. On stock win32 CPython, **the managed controller itself fails import** before preflight — not only the launcher child. Panel is blocked at import until lazy/gated imports land (T4).
- `require_argv_consistent_with_spec` requires launcher argv = `sys.executable` + this `run_diagnostic.py` + declared diagnostic argv — confirmed.
- `tui_input_probe.InputProbe` writes via `os.write` on a master fd — PTY-shaped; T5 correctly scoped.

### B5 — Same-host `game_server` / cross-host (critical) — CONFIRMED

- Spec requires positive local `game_server_pid`; preflight samples that PID and matches server sidecar `{pid, start_identity}`.
- Runner never starts/stops/signals server/ambient (module docstring + cleanup scope).
- Binding requires `roles['game_server']` to match sidecar; process evidence needs one schema-2 stream with all roles; backend only `system|libproc`.
- No remote sample transport or dual-host merge in sources.
- Operator topology (Mac server + Windows client via loopback SSH reverse forwards) is documented in `windows-native-first-proof.md` as cross-host, not six-role same-host accounting — audit uses that correctly as backdrop, not as a silent Mac-PID-on-Windows allowance.
- Options A–D and “Answer: No” on honest Mac-server + Windows-frontend matched cells are correct under current gates. T8 as policy gate before matched cells is appropriate.

### B6 — HOME / unpack_root Python lag (high) — CONFIRMED

- `expected_unpack_root`: only non-empty `os.environ.get('HOME')`, else `cwd/.274bot/unpack` — no `USERPROFILE`.
- `run_diagnostic` catalog: `(Path(env.get('HOME') or '.') / '.274bot/js-scripts.json')`.
- Rust `operator_home_from` (per `windows-home-path-report.md`): on Windows, explicit HOME wins including empty; else USERPROFILE.
- Gap and T6 mirror are correct. Empty-HOME semantics differ (Python → cwd fallback; Rust empty HOME can win as empty prefix) — T6 should keep fail-closed parity explicit when implementing.

### B7 — Sampler module bind omits `windows_process_sample.py` (medium) — CONFIRMED

- `_build_sampler_config`: for `system`, modules = `process_accounting.py` + `server_resources.py` only; `libproc` adds `native_process_sample.py`.
- `managed_resource_binding._process` expected set matches that rule; no `windows_process_sample.py`.
- Actual Win32 counters live in `windows_process_sample.py` imported inside `server_resources._windows_sample`.
- Silent edit would not trip launch/completion module recheck. T7 is the right bind.

### B8 — Hardcoded `RS2B0T` / default binary name (medium) — CONFIRMED

- `env.setdefault('RS2B0T', '/Users/acfrazier/experiments/rs2b0t')` then `git -C` rev-parse in metadata.
- Default binary `target/release/{tui,panel}-play` without `.exe`; managed specs require explicit `--binary` via argv consistency — audit’s “OK if specs written correctly” is fair.

### B9 — Cross-user OpenProcess (unknown) — CONFIRMED as unknown

- Sampler rights: `PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ | SYNCHRONIZE` — source-backed.
- Austen→BotTest success not proved in-tree. Prefer same-user collector; T9 proof-only — correct. No gate weakening proposed.

### B10–B12 — Accepted limitations / out-of-runner context — CONFIRMED

- Children CPU: explicit unavailable/null on platforms without rusage children — source-backed; not a matched per-PID host/server blocker.
- Pressure: win32 unavailable honest; `process_evidence` leaves pressure `unassessed`.
- Full client three failures: backdrop from native proof docs; correctly out of runner blockers.

---

## Already-supported claims

| Claim | Review |
|-------|--------|
| Win32 `system` sample via `windows_process_sample` (identity, WS, CPU, exit wait, fail closed) | Source-backed |
| `process_accounting` loads without hard `resource` import; children CPU honest null | Source-backed |
| Host pressure unavailable on win32 (not zero/healthy) | Source-backed |
| Receipt/binding fail-closed; `performance_acceptance` false end-to-end | Source-backed |
| Rust host WorkingSet/GetProcessTimes separate from multi-role Python accounting | Backdrop report + correct non-substitution claim |
| Panel non-PTY launch body exists but import still blocks | Source-backed |
| Native TUI/panel binaries exist (operator proof, not re-run) | Backdrop only — acceptable as non-re-run |

Standalone local explicit-PID sampling “ready enough” is consistent with prior approved Windows sampler work and e425799 collector repair notes; this review does not re-run those tests and does not upgrade that to managed-cell readiness.

---

## Next-task graph (T1–T10)

| Task | Unblocks | Review |
|------|----------|--------|
| T1 stop channel | B1 | Required; keep Unix SIGTERM; TerminateProcess escalation only |
| T2 portable cleanup | B2 | Required; no bare `SIGKILL` reference on win32 path |
| T3 parent PID | B3 | Required; fail-closed child-of-launcher |
| T4 lazy diagnostic import + panel + RS2B0T | B4 partial, B8 | Required before any managed import on win32 |
| T5 ConPTY TUI | TUI primary | Correctly optional for panel-first path |
| T6 HOME parity | B6 | Required for cache-matched cells |
| T7 bind windows_process_sample | B7 | Required for immutable Win32 sampler bytes |
| T8 server placement | B5 policy | Required before honest matched `game_server` series |
| T9 user access proof | B9 | Proof/report; no gate weaken |
| T10 dry managed cell | integration | After hooks; still no savings claim |

Suggested dependency sketch (T4 early; T1–T3/T6/T7 → T10; T5 for TUI; T8/T9 for matched server honesty) matches source constraints. Panel RDP resource cells after T1–T4,T6–T9 without T5 is a sound sequencing note, not a green claim.

---

## Source claims vs inferences vs unknowns

**Source-backed (code on this branch):** B1–B8 mechanisms and contracts; module bind gap; HOME-only Python paths; required local `game_server_pid`; single-stream binding; PTY/import; SIGKILL tuple; `ps` ownership; pressure/children honesty; performance_acceptance false.

**Inference / well-known runtime (not re-proved live in audit or this review):** SIGTERM→TerminateProcess on Windows CPython for `Popen.send_signal` / cross-process `os.kill(SIGTERM)`; resulting missing `controlled_stop` / non-zero sampler exit in a full managed cell. Acceptable as blocker reasoning; do not treat as a fresh native managed-cell receipt proof.

**Operator / backdrop (not re-audited here):** native TUI/panel binary existence; Mac SSH reverse-forward topology; e425799 73-test Windows collector path; three client failures.

**Explicit unknowns (audit § — still open):** server placement A vs C; headless TUI for CPU/RSS-only cells; BotTest vs Austen OpenProcess; scheduled-task parent/job topology; ConPTY under RDP; ambient helpers Mac-only vs Windows PIDs.

**Correctly absent:** savings claims; full qualification; inventing Mac server PID as Windows `game_server`; weakening enclosure/identity/exclusive-create/performance_acceptance; WSL2 as native Windows substitute.

---

## Non-blocking nits (do not require audit rewrite)

1. B1: library `stop_event` exists in-process; managed child still has no IPC — T1 already the fix; optional one-line clarify if audit is edited later.
2. B2: exception-path cleanup can re-raise `AttributeError` after the outer handler has already started — still blocker-class.
3. B4: parent `import run_diagnostic` failure is the earliest abort on win32; audit cascade already implies it.
4. B1 TerminateProcess outcome is mechanism inference until a Windows dry cell exists (T10).

None of these change blocker ordering, fail-closed posture, or the APPROVED verdict.

---

## Acceptance mapping (t_0112cec2)

| Criterion | Result |
|-----------|--------|
| Review audit vs directly referenced current source | Done |
| Required Grok reviewer profile (not implementer self-pass) | This pass |
| Branch not `main` | `codex/memory-diagnostics` |
| No source/git/remote/live/build/UI mutations; write only this review file | Honored |
| Verify stop/cleanup/ownership/PTY, module bind, home fallback, local server PID, cross-host non-support | Confirmed |
| `os.kill(pid,0)` modern-CPython note without general unsupported-OS claim | Audit OK; root probe noted as version-scoped evidence |
| Flag source vs inference vs unknown | Section above |
| APPROVED or CHANGES REQUESTED actionable | **APPROVED** |
| No savings/readiness-green claim | None |
| Bounded audit only; final branch Grok-4.6 later | Stated |

---

## Files written by this review

- `docs/memory/windows-managed-runner-review.md` only
