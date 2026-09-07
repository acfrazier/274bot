# Windows explicit-PID Python resource sampling — implementer report

**Task:** t_95fecae0  
**Branch:** `codex/memory-diagnostics`  
**Workspace:** `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`  
**No git mutations** (implementer). Native Windows on-box Python proof is **root-owned / pending**.

---

## What changed

### `docs/memory/windows_process_sample.py` (new)

Standalone Win32 ctypes backend (no third-party packages, no PowerShell/subprocess, no name/argv/env discovery):

| Concern | Implementation |
|---------|----------------|
| Open | `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION \| PROCESS_VM_READ \| SYNCHRONIZE)` |
| Identity | `GetProcessTimes` **creation** FILETIME → `windows_creation_filetime:<u64 ticks>` (100 ns since 1601-01-01 UTC) |
| CPU | Same call: user + kernel FILETIME as **CPU durations** (100 ns ticks ÷ 10_000_000 → seconds) |
| Resident | `GetProcessMemoryInfo` → **WorkingSetSize only** (not PeakWorkingSetSize, PagefileUsage, PrivateUsage) |
| Exit | `WaitForSingleObject(handle, 0) == WAIT_OBJECT_0` → exited; STILL_ACTIVE not trusted alone |
| Fail closed | missing / access denied / exited / API fail / zero WorkingSetSize → `SampleError` |
| Handles | Owned handle closed on every path; close failure does not mask a prior sample error |
| Layout | `DWORD` forced to `c_uint32` (Windows ABI); `SIZE_T` pointer-width; sizes recorded in provenance |
| Inject | Optional `api=` / per-call wrappers for deterministic tests without Win32 |

### `docs/memory/server_resources.py`

- `sample_process`: `win32` → `_windows_sample` → `windows_process_sample.sample_process` (no silent fallback to another OS after failure).
- `sample_pressure` on Windows: `status=unavailable`, reason `unsupported_pressure_counter`, explicit note that unavailable is **not** zero/healthy; Linux/macOS paths unchanged.

### `docs/memory/process_accounting.py`

- Top-level `import resource` removed; lazy `_get_resource_module()` so the module loads on Windows.
- Default children snapshot without inject: if `resource` / `RUSAGE_CHILDREN` missing → `status=unavailable`, **null** cumulative counters, note that children CPU is not fabricated from parent CPU.
- Injected `rusage_children_fn` path and Unix behavior preserved.
- Schema / exclusive-create / required-grid / cadence / identity invariants untouched.
- No claim that a standalone sampler result is complete managed accounting.

### Tests

- `docs/memory/test_windows_process_sample.py` (new): FILETIME math, structure widths, injected happy path / identity / counter reset / exit-wait / open failures / zero WS / handle close / routing + pressure wiring / children-unavailable portability; **Windows-only live** own-PID + missing PID skipped on Mac.
- Existing `test_server_resources.py` / `test_process_accounting.py` unchanged and still pass.

### Out of scope (honored)

- No Rust/Cargo, managed runner/launcher, receipts/schema/gates, native-image binding, measurement acceptance, cross-host server identity.
- No git commit/push, no Windows remote actions, no live game runs.
- Native Windows managed/live readiness remains a separate track.

---

## Tests run (this box — macOS)

| Command | Result |
|---------|--------|
| `python3 -m unittest test_windows_process_sample test_server_resources test_process_accounting` | **OK** — 71 tests, **3 skipped** (Windows live) |

Portable/injected Windows cases **ran and passed** on Mac. Live Win32 own-PID / missing-PID / system-backend self-sample are **source-gated** and were **not** executed here.

---

## Windows on-box — **PENDING root**

Python 3.14 Windows x64 is installed and reachable per task note, but this implementer did not run remote Windows. Until root supplies a receipt, treat native proof as **pending**. Do not treat Mac mock results as Windows success.

Suggested root commands (from `docs/memory` on the Windows host):

```text
python -m unittest test_windows_process_sample test_server_resources test_process_accounting -v
python server_resources.py <own_pid> out.jsonl --duration 0.2 --interval 0.05
python process_accounting.py collector=<own_pid> out2.jsonl --duration 0.2 --interval 0.05 --no-collector-self
```

Expect: nonzero WorkingSetSize, stable `windows_creation_filetime:*` identity, non-decreasing cumulative user/kernel, missing PID → FAIL, pressure `unavailable` (not healthy/zero), children CPU unavailable with null counters.

---

## Unsupported / limitations (honest)

1. **Host memory pressure** on Windows: unavailable only; no PSI substitute.
2. **Waited-children CPU/RSS** via `getrusage(RUSAGE_CHILDREN)`: unavailable on Windows; null counters; not mapped from parent.
3. **No process-tree walk**; explicit PIDs only.
4. **No managed complete-accounting claim** from this standalone sampler.
5. **Native Windows unit/live receipt**: pending root.
6. Working set ≠ Linux RSS ≠ macOS footprint; no cross-OS equality claim.

---

## Files touched

- `docs/memory/windows_process_sample.py` (new)
- `docs/memory/test_windows_process_sample.py` (new)
- `docs/memory/server_resources.py`
- `docs/memory/process_accounting.py`
- `docs/memory/windows-python-sampler-report.md` (this file)
