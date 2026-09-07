# Windows sampler module binding in managed evidence

**Task:** t_af9534fd  
**Branch:** `codex/memory-diagnostics` (not `main`)  
**Workspace:** `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`  
**Scope:** B7 from `windows-managed-runner-audit.md` — pin actual Win32 sampler bytes in launch config and fail-closed binding. No git mutations, no remote Windows, no live cells, no Rust, no stop/cleanup/PTY/home/server topology changes. `windows_process_sample.py` APIs frozen (e425799); read-only.

**Not claimed:** memory savings, Windows managed-runner readiness, matched baseline/candidate acceptance, or that Mac tests prove native Windows cells.

---

## Gap (B7)

For `process_backend=system`, launch/recheck/binder previously bound only:

- `process_accounting.py`
- `server_resources.py`

On Windows, actual counters are implemented in `windows_process_sample.py` and imported by `server_resources._windows_sample`. A silent edit to that file escaped module recheck and exact dependency set checks. Mac/Linux `system` logic stays inside `server_resources.py`, so the gap is Windows-specific.

---

## What changed

### `run_managed_cell.py` — launch pin

- `DEFAULT_WINDOWS_PROCESS_SAMPLE` points at `windows_process_sample.py`.
- `_build_sampler_config`: when **producer** `sys.platform == 'win32'` and `process_backend == 'system'`, bind `windows_process_sample.py` path + sha256 (same pattern as libproc → `native_process_sample.py`).
- Non-Windows producers do **not** add that module (unused dependency stays out of the set).
- Completion `_recheck_sampler_modules` already iterates all bound modules — no separate path needed once the pin is present.

### `managed_resource_binding.py` — reader validation

- Exact expected module set still fail-closed (`set(modules) == expected`).
- `libproc` still adds `native_process_sample.py` only.
- For `system`, if runtime role identities prove Win32 production via the real identity prefix  
  `windows_creation_filetime:` (from frozen `windows_process_sample.creation_identity`),  
  expected set **requires** `windows_process_sample.py`.
- Detection uses **producer evidence in the receipt** (role `start_identity`), **not** the reviewing machine’s `sys.platform`. A Mac/Linux reader still rejects an older Windows receipt that lacks the Windows module pin.
- Extra `windows_process_sample.py` on non-Windows identity receipts fails the exact set check (no silent blessing of unrelated pins).
- Output enclosure, role identities, matched gates, backend enum, and legacy non-Windows behavior unchanged.

### Tests

- `test_managed_resource_binding.py`: Windows included (cross-platform reader), missing (historical unbound), tampered hash, non-Windows rejects extra windows module, libproc path unchanged.
- `test_run_managed_cell.py`: `_build_sampler_config` under mocked `win32` includes windows module + correct path/sha; under mocked `darwin` excludes it. Existing Mac system module set test unchanged.

### Out of scope (honored)

- No edits to `windows_process_sample.py` APIs.
- No B1–B6/B8+ stop/cleanup/ownership/PTY/HOME/server work.
- No git commit/push; root owns native Windows proof.

---

## Tests run (this box — macOS)

| Command | Result |
|---------|--------|
| `python3 -m unittest test_managed_resource_binding test_run_managed_cell.ManagedCellTests.test_sampler_modules_bound_and_backend_identity_consistent test_run_managed_cell.ManagedCellTests.test_windows_sampler_module_bound_when_producer_is_win32 -v` | **OK** — 11 tests |
| `python3 -m unittest test_managed_resource_binding test_run_managed_cell test_managed_receipt test_process_evidence -v` | **OK** — 54 tests, ~32s |

Portable/injected cases ran on Mac. Live Win32 managed cell launch pin and native recheck are **root-owned / pending**.

---

## Limitations (exact)

1. **Launch pin uses producer `sys.platform` at cell run time.** Correct for a Windows controller; a Mac controller never pins the Windows module (and must not invent Windows identities).
2. **Binder Windows requirement is identity-prefix based** (`windows_creation_filetime:`). That is the real Win32 sampler identity format. Fabricated prefixes without a coherent series still fail later enclosure/hash gates; missing pin with real Windows identities fails at dependency set.
3. **No new free-form platform metadata field** was added to receipts — minimal, source-backed identity evidence only.
4. **Native Windows managed-cell end-to-end** (launch → recheck → bind on real Win32) not executed here.
5. **Other Windows managed blockers (B1–B6, B8+)** remain; this task only closes B7 module binding completeness.
6. **No performance_acceptance / savings / readiness-green claim.**

---

## Files touched

| Path | Change |
|------|--------|
| `docs/memory/run_managed_cell.py` | Bind `windows_process_sample.py` on win32 system producer |
| `docs/memory/managed_resource_binding.py` | Require that pin from Windows role identity evidence |
| `docs/memory/test_run_managed_cell.py` | Producer win32 vs darwin module-set unit test |
| `docs/memory/test_managed_resource_binding.py` | Windows include/missing/tamper + non-Windows/libproc regressions |
| `docs/memory/windows-sampler-binding-report.md` | This report |
