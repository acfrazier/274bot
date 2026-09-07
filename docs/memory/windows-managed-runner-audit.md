# Windows matched-measurement managed-runner readiness audit

**Task:** t_20fe5d5c  
**Branch:** `codex/memory-diagnostics` (not `main`)  
**Workspace:** `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`  
**Scope:** read-only source audit of `docs/memory/run_managed_cell.py` and **directly imported** accounting / receipt / binding / diagnostic / latency helpers. No git mutations, no source edits outside this file, no live/build/remote/UI invocation.  
**Not claimed:** savings, full Windows qualification, matched baseline/candidate acceptance, or that Mac/Linux pipeline green proves Windows.

Prior context used only as status backdrop (not re-audited archives): native Windows TUI+panel compile/run and Python explicit-PID sampling proven at host `e425799` (`GetProcessMemoryInfo` / `GetProcessTimes`); full client still three known failures; Mac managed pipeline qualification exists; game server currently Mac via loopback SSH reverse forwards.

---

## Verdict (one line)

Standalone Windows PID sampling is **ready enough for local explicit PIDs**, but the **managed matched-cell runner is not Windows-ready**: stop/cleanup, TUI PTY launch, frontend ownership, HOME/unpack binding, and **same-host `game_server` continuous accounting** still fail closed. A Mac server PID must not be invented as a Windows role.

---

## Audit surface (what was read)

| Module | Role in managed path |
|--------|----------------------|
| `run_managed_cell.py` | One-cell orchestrator: spec, preflight, launch, collector, cleanup, receipt |
| `process_accounting.py` | Schema-2 continuous multi-role collector (direct import + child argv) |
| `server_resources.py` | `system` backend process/pressure sample (`process_sampler('system')`) |
| `windows_process_sample.py` | Win32 ctypes path used only when `sys.platform == 'win32'` inside `server_resources` |
| `native_process_sample.py` | Optional `libproc` backend (macOS only) |
| `managed_receipt.py` | Immutable launch + completion receipt |
| `managed_resource_binding.py` | Post-receipt cache + continuous process bind |
| `process_evidence.py` | Schema-2 enclosure validation vs native wall envelope |
| `run_diagnostic.py` | Required launcher target (PTY TUI / panel) |
| `tui_input_probe.py` | Optional TUI PTY keystroke stimulus (imported by diagnostic) |
| `build_provenance.py` / `cache_provenance.py` | Binary/fixture and cache fingerprint (path-portable in principle) |
| `instrumentation_overhead.py` | Downstream quartet consumer (not required to *run* a cell; gates matched overhead later) |

Not audited as managed blockers: archive trees, `STATE.md` plans, unrelated worktrees, live diagnostic JSON dumps.

---

## Already supported on Windows (source-backed)

These pieces do **not** need reinventing for local Win32 PIDs:

1. **`system` process sample on win32**  
   `server_resources.sample_process` → `_windows_sample` → `windows_process_sample.sample_process`  
   Identity: `windows_creation_filetime:<u64>`  
   RSS: current `WorkingSetSize` only  
   CPU: `GetProcessTimes` user/kernel FILETIME durations  
   Exit: zero-timeout `WaitForSingleObject`  
   Fail closed on missing / access denied / exited / zero WS.  
   Proven on native Python 3.14 per `windows-native-first-proof.md` / collector repair `e425799` (71→73 tests path); this audit does not re-run it.

2. **`process_accounting` loads on Windows**  
   Lazy `resource` import; waited-children CPU returns explicit `unavailable` with **null** counters (not fabricated from parent).  
   Schema-2 exclusive create, role identity stability, required-grid / controlled-stop contracts remain fail-closed.

3. **Host pressure honesty**  
   `sample_pressure` on win32 → `unavailable` / `unsupported_pressure_counter` (not zero/healthy). Same class as macOS.

4. **Receipt / binding fail-closed contracts (OS-agnostic logic)**  
   - Exclusive cell dir, no overwrite  
   - Launch hashes binary/manifest/server_identity/host_conditions (+ optional cache)  
   - Completion requires launcher exit 0, sampler exit 0, raw files, metadata envelope, module recheck  
   - `managed_resource_binding` requires continuous enclosure of **every** role identity including `game_server`  
   - `process_evidence` requires backend ∈ `{system, libproc}` only; duration_mode match; no silent role drop  
   - `performance_acceptance` stays false end-to-end  

5. **Rust frontend counters (separate from Python collector)**  
   Host-play Windows WorkingSet / GetProcessTimes already implemented (`windows-process-metrics-report.md`). Useful for host `samples.jsonl` once a managed diagnostic actually runs; **does not** replace multi-role Python accounting.

6. **Panel non-PTY launch path in `run_diagnostic`**  
   Non-terminal branch is plain `subprocess.Popen` (stdout/stderr to log) — no `pty`/`fcntl`. **Panel RDP cells are not blocked by PTY**, only by the runner/collector/server issues below and by operator RDP/BotTest setup.

7. **Native TUI+panel binaries exist on Windows** (operator proof, not re-run here)  
   TUI primary for matched cells is still blocked at the **Python launcher** layer (PTY), not necessarily at the Rust binary layer.

---

## Exact Windows blockers for real matched baseline/candidate cells

Ordered by how early they abort a completed receipt. TUI is primary; panel called out when different.

### B1 — Collector stop is TerminateProcess, not a graceful controlled stop (critical)

**Where:** `run_managed_cell.py` requests collector stop via `collector.send_signal(signal.SIGTERM)` after observe-end pad / launcher exit; `process_accounting.run` relies on SIGTERM/SIGINT handlers for `duration=None` → `completion=controlled_stop` / exit 0.

**Windows fact:** CPython maps `Popen.send_signal(SIGTERM)` to **`TerminateProcess`**. That does not run the collector’s Python signal handler, does not flush a normal summary, and typically yields non-zero / missing `controlled_stop`.

**Effect:** Even with perfect samples mid-run, receipt path records `sampler_failed_or_incomplete` (`managed_receipt.complete` requires sampler `exit_code == 0`). `process_evidence` then rejects non-`controlled_stop` stop_controlled series.

**Not a workaround in current code:** managed cells always use `duration_mode: stop_controlled` with `duration_s_requested: None` (`_build_sampler_config`). No stdin/event/file stop channel exists.

### B2 — Frontend cleanup references `signal.SIGKILL` (critical on cleanup path)

**Where:** `_cleanup_frontend` loops `(signal.SIGTERM, signal.SIGKILL)` then `os.kill(pid, sig)`.

**Windows fact:** `signal.SIGKILL` is **not defined** on win32. First cleanup after a live frontend raises `AttributeError` (caught by outer handler → failed report). Escalation kill is not portable as written.

**Related:** `_pid_alive` via `os.kill(pid, 0)` is acceptable on modern CPython Windows; soft TERM via `os.kill(SIGTERM)` is still TerminateProcess semantics for non-child PIDs.

### B3 — Frontend parent ownership uses Unix `ps` (critical for TUI and any captured child)

**Where:** `_capture_owned_frontend` runs  
`['ps', '-o', 'ppid=', '-p', str(pid)]`  
and requires `ppid == launcher_pid`.

**Windows fact:** No stock `ps` with that ABI. Capture fails → `CellError('cannot establish frontend parent ownership')` while frontend still alive → failed cell / no honest owned-frontend identity for cleanup.

**Needed:** Win32 parent PID (e.g. `CreateToolhelp32Snapshot` / `NtQueryInformationProcess`) with the same fail-closed “must be child of owned launcher” rule — not name discovery.

### B4 — Real TUI diagnostic launcher is Unix PTY-only (critical for TUI primary)

**Where:** `run_diagnostic.py` imports `fcntl`, `pty`, `termios` at **module import** time; terminal path uses `pty.openpty`, `TIOCSWINSZ`, `os.setsid`, `TIOCSCTTY`, `preexec_fn`.

**Windows fact:** Import of `run_diagnostic` fails before any cell runs if those modules are absent (standard win32 CPython). Even with lazy import, there is no ConPTY/winpty path.

**Cascade:**  
- Managed `require_argv_consistent_with_spec` demands launcher argv = `sys.executable` + this `run_diagnostic.py` + declared diagnostic argv.  
- `--tui-input-probes` and `tui_input_probe.InputProbe` assume a PTY master fd (`os.write` to master).  
- Mac pipeline qualification used “real default TUI PTY”; that exact path is not portable.

**Panel difference:** `terminal=False` avoids PTY **if** the module can import. Today import is unconditional → **panel is also blocked at import** until `pty`/`fcntl`/`termios` are lazy/gated.

### B5 — Same-host `game_server` continuous sampling (critical for matched cells)

**Contract (current, fail-closed):**  
- Spec requires positive local `game_server_pid`.  
- Preflight samples that PID with the selected backend and matches `server_identity_path` `{pid, start_identity}`.  
- Collector continuously samples `game_server` every interval with stable `start_identity`.  
- Binding: `roles['game_server']` must equal server sidecar identity; series must enclose native observation wall (`process_evidence`).  
- Runner **never** starts/stops/signals the server.

**Operator topology today:** game server on **Mac**, Windows client reaches it through **loopback SSH reverse forwards**. That is valid for functional login/smoke (`windows-native-first-proof.md` explicitly: cross-host, not local Windows server, not six-role same-host accounting).

**What the existing model does *not* support:**  
- Putting the Mac server’s PID into a Windows cell spec and sampling it with Win32 `OpenProcess` — wrong host; PID namespaces are not shared.  
- Inventing a “same-host Windows server PID” that does not exist.  
- Treating reverse-forward listener PIDs (sshd/ssh) as `game_server` — different binary/identity; would falsify server resource series.  
- Splitting roles across two collectors on two OSes into one receipt — `managed_resource_binding` / `process_evidence` expect **one** schema-2 artifact with **all** roles.

**Honest options (policy, not implemented here):**  

| Option | Fits current receipt model? | Notes |
|--------|----------------------------|--------|
| **A. Native Windows game server** + local PID/sidecar on the measurement box | **Yes** | Cleanest match to today’s six-role same-host design. Requires Windows server bring-up (out of runner scope). |
| **B. Keep Mac server; run entire managed cell (controller+collector+frontend) on Mac** | **Yes** (already proven path) | Does not produce **Windows** host RSS/CPU matched cells. |
| **C. Cross-host receipt protocol** (Windows frontend roles + remote Mac server sampler, dual identity sidecars, merged enclosure rules) | **No — not present** | Would be new schema/binding work; must not be faked by weakening gates. |
| **D. Drop `game_server` from required continuous roles on Windows** | **No under current gates** | Would weaken evidence; forbidden by this task. |

**Conclusion:** Existing cross-host *connectivity* (SSH reverse forwards) is fine for gameplay. Existing cross-host *receipt/accounting* model does **not** honestly support Mac-server + Windows-frontend matched CPU/currentRSS cells. Either run a **native Windows server** for same-host sampling, or design an explicit multi-host accounting extension — do not claim Mac PID sampling from Windows.

### B6 — HOME / unpack_root Python binding lags Rust operator_home (high for cache-matched cells)

**Where:**  
- `run_managed_cell.expected_unpack_root`: only `os.environ.get('HOME')`, else `cwd/.274bot/unpack`.  
- `run_diagnostic` catalog: `(Path(env.get('HOME') or '.') / '.274bot/js-scripts.json')`.  
- Rust client now has Windows `HOME`/`USERPROFILE` selection (`windows-home-path-report.md`).

**Windows fact:** Typical session has `USERPROFILE`, often no `HOME`. Spec `unpack_root` checked against Python `expected_unpack_root` will disagree with the binary’s real unpack dir unless the operator forces `HOME` to match Rust.

**Effect:** Cache provenance cells fail preflight (`unpack_root canonical != inherited`) or fingerprint the wrong tree → binding `native cache directory differs from fingerprint`.

### B7 — Sampler module binding omits `windows_process_sample.py` (medium — evidence completeness)

**Where:**  
- `_build_sampler_config` / `managed_resource_binding._process`: for `process_backend=='system'`, bound modules are only `process_accounting.py` + `server_resources.py`.  
- `libproc` additionally binds `native_process_sample.py`.  
- On Windows, **actual counters** live in separate `windows_process_sample.py` imported by `server_resources`.

**Effect:** Launch receipt does not pin the Win32 sampler bytes. A silent edit to that file would not trip module recheck. Mac/Linux `system` path inlines OS logic inside `server_resources.py`, so this gap is Windows-specific for the ctypes backend.

**Also:** `process_backend` enum is only `system|libproc`. No `win32` backend name; routing is implicit via `sys.platform` inside `system`. Acceptable if documented and if `windows_process_sample.py` is bound whenever used.

### B8 — Hardcoded Mac paths in diagnostic metadata (medium for provenance on Windows box)

**Where:** `run_diagnostic.build_child_env` / `main`:  
`env.setdefault('RS2B0T', '/Users/acfrazier/experiments/rs2b0t')` then `git -C env['RS2B0T'] rev-parse`.

**Effect:** On a machine without that path, metadata construction throws unless `RS2B0T` is pre-set. Not a sampler issue but blocks stock launcher.

Default binary path `target/release/{tui,panel}-play` lacks `.exe` when `--binary` omitted; managed specs require explicit `--binary` so production cells can pass a full Windows path — OK if specs are written correctly.

### B9 — Access rights for BotTest vs Austen admin sampler (unknown until proved)

**Operator plan:** standard RDP user `BotTest`; SSH admin `Austen` remains.

**Source behavior:** sampler uses `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ | SYNCHRONIZE)`. Cross-user open often works for admins but can return **access denied** (fail closed — good). Sampling BotTest children from an Austen-run collector is **not proved** in-tree.

**Implications:**  
- Prefer running controller+collector **in the same user session** as the frontend (BotTest RDP) for matched cells.  
- If Austen must sample BotTest PIDs, prove OpenProcess on those PIDs and record access model in host_conditions — do not assume.  
- Do not treat “admin SSH can see Task Manager” as evidence of QUERY_LIMITED + VM_READ success.

### B10 — Waited-children CPU always unavailable (accepted limitation, not a matched-host blocker)

Documented null/unavailable on Windows. Matched host/server role CPU uses per-PID counters, not RUSAGE_CHILDREN. Do not invent children CPU. Instrumentation overhead protocols that *depend* on waited-child deltas stay unavailable on Windows until redesigned.

### B11 — Host memory pressure unavailable (accepted; same class as macOS)

Do not treat as healthy/zero. Matched RSS/CPU cells do not currently require available pressure rows (`process_evidence` marks pressure `unassessed`).

### B12 — Full client three known failures (out of runner, still qualification context)

Native client suite still fails shade/modal/crc cases. Does not block TUI managed accounting once launcher works, but blocks any “full client green on Windows” claim. Panel GPU matched claims stay separate.

---

## Cross-host receipt model (explicit answer)

**Question:** Does the existing cross-host receipt model support Mac game server + Windows frontend matched cells honestly?

**Answer: No.**

- Connectivity via SSH reverse forwards is orthogonal and already used for smoke.  
- Accounting requires a **locally samplable** `game_server` PID with stable `start_identity` in the **same** schema-2 stream as controller/launcher/collector/(ambient) on the collector’s host.  
- There is no remote sample transport, no dual-host merge binder, and no allowance to omit `game_server`.  
- Therefore: **native Windows server (option A)** or **new multi-host protocol (option C)** — not a silent Mac PID on a Windows spec.

---

## What remains intentionally out of scope / non-blockers for this audit

- Implementing any of the above (task forbids runtime implementation).  
- Weakening enclosure, identity, exclusive-create, or performance_acceptance gates.  
- Cross-OS RSS equality (WorkingSet ≠ Linux RSS ≠ macOS footprint).  
- WSL2 Linux lane as substitute for native Windows matched cells.  
- Savings / paired baseline-candidate acceptance claims.

---

## Concrete next implementation tasks (small, ordered)

Do not sprawl into a redesign. Each task should keep fail-closed contracts and add tests first.

### T1 — Windows-safe collector stop channel (unblocks B1)

**Files:** `process_accounting.py`, `run_managed_cell.py`, `test_process_accounting.py`, `test_run_managed_cell.py`  
**Scope:**  
- Add an explicit stop mechanism that works on win32 without relying on `send_signal(SIGTERM)` as graceful stop (e.g. stop file path, stdin byte, or Windows Event name passed on argv; handler thread or poll in existing sleep slice).  
- Keep Unix SIGTERM path working.  
- stop_controlled + successful stop still → exit 0 / `completion=controlled_stop` / `status=closed`.  
- TerminateProcess remains escalation only after timeout, same as today’s kill path.  
**Tests:** dummy multi-role stop on injected clock; assert summary controlled_stop without requiring Unix signals; Windows-skipped live only if needed.  
**Unknowns:** none for contract; pick one IPC style and document in metadata.

### T2 — Portable owned-child cleanup (unblocks B2)

**Files:** `run_managed_cell.py`, `test_run_managed_cell.py`  
**Scope:**  
- Replace `(SIGTERM, SIGKILL)` with a portable escalate helper: soft request where available, then `Popen.kill()` / TerminateProcess for owned children; identity recheck before each stage preserved.  
- Never signal server/ambient/controller.  
**Tests:** existing TERM-ignoring frontend fixture behavior on Unix unchanged; unit test that win32 code path does not reference missing `SIGKILL` (can run on Mac via branch).

### T3 — Windows frontend parent ownership (unblocks B3)

**Files:** new small helper e.g. `windows_process_parent.py` or functions in `windows_process_sample.py`; `run_managed_cell._capture_owned_frontend`; tests  
**Scope:**  
- Explicit PID → parent PID on win32 without name/argv scan.  
- Same rule: parent must equal launcher PID or fail closed.  
- Unix keeps `ps` or moves both under a `parent_pid(pid)` abstraction.  
**Tests:** injected parent map; reject foreign PID; missing process fail closed.  
**Unknowns:** whether CREATE_BREAKAWAY_FROM_JOB / elevated launcher changes parent topology for scheduled-task launches — prove under BotTest launch method.

### T4 — Gate/lazy-import diagnostic Unix TTY stack; panel path on Windows (unblocks B4 partially)

**Files:** `run_diagnostic.py`, `test_run_diagnostic.py`  
**Scope:**  
- Lazy-import `pty`/`fcntl`/`termios` only on terminal TUI path.  
- Panel and `--headless` TUI must import and run on win32 without those modules.  
- Fail closed with clear error if real TUI terminal requested on win32 until T5 exists.  
- Fix `RS2B0T` default to require env or skip rs2b0t commit field fail-closed rather than hardcoding a Mac path.  
- Align catalog/HOME with operator home rules (see T6).  
**Tests:** import smoke under mocked `sys.platform`; panel argv validate without pty.

### T5 — Windows TUI terminal transport (unblocks TUI primary matched cells)

**Files:** `run_diagnostic.py`, `tui_input_probe.py`, tests  
**Scope:**  
- ConPTY or equivalent for real TUI (not headless) with fixed winsize 120×40 semantics.  
- Input probe writes through that transport; still “write only ≠ latency proof”.  
- Receipt still requires native qualification boundaries from the binary.  
**Tests:** fixture PTY-like duplex if possible; skip live console if CI lacks ConPTY.  
**Unknowns:** RDP ConPTY behavior; whether headless TUI is acceptable for *resource* matched cells (CPU/RSS) while input latency remains separate — product decision; default remains real terminal for parity with Mac qualification unless orch decides otherwise.

### T6 — Python unpack/catalog home parity with Rust (unblocks B6)

**Files:** `run_managed_cell.py` (`expected_unpack_root`), `run_diagnostic.py` (catalog_path), tests  
**Scope:** Mirror `operator_home_from(HOME, USERPROFILE)` rules from `windows-home-path-report.md` (Windows: explicit HOME wins including empty; else USERPROFILE).  
**Tests:** matrix of HOME/USERPROFILE presence on fake platform flag.

### T7 — Bind `windows_process_sample.py` when system backend runs on win32 (unblocks B7)

**Files:** `run_managed_cell._build_sampler_config`, `managed_resource_binding._process`, `process_evidence` if backend label changes, tests  
**Scope:**  
- When `sys.platform=='win32'` and backend `system`, include `windows_process_sample.py` path+sha256 in `sampler.modules` exactly like libproc’s native module.  
- Binding expected set must match.  
- Do not add silent fallback backends.  
**Tests:** module set equality; tamper → recheck failure.

### T8 — Server placement decision gate (unblocks B5 policy)

**Files:** docs only until chosen — then server identity sidecar tooling as needed  
**Scope (choose one before matched Windows cells):**  
1. **Native Windows server** install/run recipe + local `server_identity.json` writer using Windows sampler; ambient helpers local; OR  
2. Design task for **multi-host accounting** (explicit schemas) — separate epic, not a silent tweak.  
**Tests:** preflight must fail if sidecar OS/provenance disagrees with sampler host (add host marker to identity if missing).  
**Unknowns:** Windows server binary availability, license, port binds vs existing reverse-forward smoke, BotTest vs SYSTEM service account for server PID.

### T9 — Same-user vs cross-user sampling proof (unblocks B9)

**Files:** report under `docs/memory/`; optional tiny probe script later  
**Scope:** On real box, from Austen and from BotTest, sample known PIDs (self, other-user child). Record OpenProcess errors. Decide collector user for matched cells.  
**No gate weakening** if access denied — run collector as the frontend user.

### T10 — End-to-end dry managed cell on Windows after T1–T7 (+ server choice)

**Files:** cell specs under diagnostics; no production code required if hooks exist  
**Scope:**  
- TUI (or panel if TUI terminal still pending) diagnostic kind first, then matched kind.  
- Require completed receipt + `managed_resource_binding.bind` → process available.  
- Still **no** savings claim; still fail on client GPU failures if those workloads selected.  
**Tests:** existing dummy suite green on Mac; Windows live is root/operator receipt.

---

## Suggested task graph (minimal)

```
T4 (diagnostic import + panel)
T1 (collector stop) ──┐
T2 (cleanup signals) ─┼─► T10 dry cell
T3 (parent PID) ──────┤
T6 (HOME parity) ─────┤
T7 (module bind) ─────┘
T5 (ConPTY TUI) ───────► TUI matched cells
T8 (server placement) ─► any matched cell with honest game_server series
T9 (user access) ──────► choose collector identity
```

Panel RDP matched resource cells can proceed after **T1–T4, T6–T9** without T5. TUI primary matched cells need **T5** as well.

---

## Source-backed blockers vs already supported (summary table)

| Concern | Status on Windows managed path |
|---------|--------------------------------|
| Explicit local PID RSS/CPU sample (Python) | **Supported** (`windows_process_sample` via `system`) |
| Continuous multi-role schema-2 accounting logic | **Supported** if stop works |
| Graceful stop_controlled collector stop | **Blocker** (SIGTERM→TerminateProcess) |
| Frontend cleanup SIGKILL | **Blocker** |
| Frontend parent ownership (`ps`) | **Blocker** |
| `run_diagnostic` import / TUI PTY | **Blocker** (panel fixable via lazy import) |
| Receipt/binding fail-closed gates | **Supported** (keep) |
| Immutable bind of Win32 sampler module bytes | **Gap** |
| HOME/unpack Python vs Rust | **Gap** |
| Mac server PID as Windows `game_server` | **Unsupported / dishonest** |
| Native Windows server same-host accounting | **Supported by model, not stood up** |
| Cross-user OpenProcess (Austen→BotTest) | **Unknown** |
| Pressure / children rusage | **Unavailable (honest)** |
| Rust host WorkingSet in samples.jsonl | **Supported** in binary; needs managed launch |
| Matched savings / full qualification | **Not claimed** |

---

## Explicit unknowns

1. Final server placement: native Windows vs multi-host protocol epic.  
2. Whether headless TUI is acceptable for CPU/RSS-only matched cells (input latency separate).  
3. BotTest vs Austen OpenProcess rights for frontend/collector PIDs.  
4. Scheduled-task vs interactive RDP parent/job topology for ownership checks.  
5. ConPTY availability/stability under the operator’s RDP path.  
6. Whether ambient helpers (gateway, supervisor) will exist as Windows PIDs or remain Mac-only (if Mac-only, they cannot appear as Windows ambient roles).

---

## Files this audit authorizes writing

- `docs/memory/windows-managed-runner-audit.md` only (this file).

No runtime implementation, no evidence gate weakening, no live invocation performed for this task.
