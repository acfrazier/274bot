# Windows managed operator-home integration review

**Task:** t_42587faf  
**Reviewer:** Grok 4.5 (`reviewer` profile)  
**Lens:** Round 1 artifact (diff-first) + independent focused Python re-run  
**Host commit:** `0dc87d7` — *Use the shared operator home resolver for managed unpack paths*  
**Branch:** `codex/memory-diagnostics`  
**Scope reviewed:** exactly `git diff 0dc87d7^..0dc87d7`  
**Verdict:** **APPROVED**

Final whole-branch `branchreviewer` remains pending (out of scope here).

---

## Diff under review

Single file, nine-line change:

| Path | Change |
| --- | --- |
| `docs/memory/run_managed_cell.py` | Import `unpack_root_path` from approved `operator_home`; `expected_unpack_root` becomes a thin wrapper |

Before (HOME-only):

```python
home = os.environ.get('HOME')
if isinstance(home, str) and home:
    return (pathlib.Path(home) / '.274bot' / 'unpack')
base = pathlib.Path(cwd) if cwd is not None else pathlib.Path.cwd()
return base / '.274bot' / 'unpack'
```

After:

```python
from operator_home import unpack_root_path
...
def expected_unpack_root(*, cwd: Optional[pathlib.Path] = None) -> pathlib.Path:
    """Client unpack directory with the same HOME/USERPROFILE selection as launch."""
    return unpack_root_path(cwd=cwd)
```

No other paths in the commit. Shared `operator_home.py` / ConPTY-owned `run_diagnostic` / inputprobe were **not** edited here (inspect-only).

---

## Acceptance criteria → evidence

| Criterion | Result | Evidence |
| --- | --- | --- |
| Wire managed unpack expectation through shared `operator_home.unpack_root_path` | **PASS** | Diff is only import + wrapper; body of `unpack_root_path` unchanged from prior reviewed helper |
| Rust HOME/USERPROFILE empty precedence parity | **PASS** | See matrix + Rust/`operator_home` comparison below |
| Spec canonical-path gate unchanged | **PASS** | Gate still `expected_unpack_root(cwd=cwd).resolve()` vs `Path(spec["unpack_root"]).resolve()` + same `CellError` text; call site not in diff hunk body beyond line shift |
| No non-Windows behavior change | **PASS** | Synthetic matrix: all non-Windows cases identical to old HOME-only helper |
| Do not edit shared/ConPTY-owned code | **PASS** | Commit touches only `run_managed_cell.py` |
| Write this review with explicit APPROVED/CHANGES | **PASS** | This file |

---

## Rust vs Python precedence

### Rust (`client::operator_home` / `unpack_dir`)

`vendor/fr-client-rust/crates/client/src/bot_target.rs`:

- Windows: `Ok(home)` (including `Ok("")`) wins; `USERPROFILE` only on `Err(_)` (absent / non-Unicode).
- Non-Windows: HOME result only; USERPROFILE never consulted.
- `unpack_dir`: non-empty home → `$home/.274bot/unpack`; empty or missing → relative `.274bot/unpack` (cwd-relative when resolved).

### Python helper (already approved; not modified in this commit)

`docs/memory/operator_home.py`:

- `operator_home_from` / `operator_home`: same empty-HOME-wins / USERPROFILE-only-when-HOME-absent rules; non-Windows ignores USERPROFILE.
- `unpack_root_path`: non-empty home → `$home/.274bot/unpack`; else `{cwd or Path.cwd()}/.274bot/unpack`.

### Managed cell after `0dc87d7`

`expected_unpack_root` delegates to `unpack_root_path(cwd=cwd)` with default environ/platform → same selection as launch-side Python and Rust unpack defaults.

Empty explicit HOME still does **not** fall through to USERPROFILE (Rust parity). Missing HOME on Windows **does** use USERPROFILE (closes prior B6 lag from `windows-managed-runner-review.md`).

---

## Behavioral matrix (old inline vs new wrapper)

Synthetic environ, fixed `cwd=/tmp/cell-cwd`:

| Case | windows | same as pre-`0dc87d7`? | Notes |
| --- | --- | --- | --- |
| HOME set | no | yes | `$HOME/.274bot/unpack` |
| HOME empty | no | yes | cwd fallback |
| HOME absent, USERPROFILE set | no | yes | USERPROFILE ignored |
| both absent | no | yes | cwd fallback |
| HOME set + USERPROFILE | yes | yes | explicit HOME |
| HOME empty + USERPROFILE | yes | yes | empty wins → cwd fallback, not profile |
| HOME absent + USERPROFILE | yes | **no (intentional)** | old: cwd; new: `$USERPROFILE/.274bot/unpack` |
| both absent | yes | yes | cwd fallback |

Only intentional Windows delta: absent HOME with USERPROFILE present — matches Rust `unpack_dir` / prior B6 finding. Non-Windows rows all `same=True`.

---

## Spec canonical-path gate

Still in `run_cell` path when `cache_dir` is set and not `_test_launcher`:

```text
expected = expected_unpack_root(cwd=cwd).resolve()
actual_unpack = pathlib.Path(spec["unpack_root"]).resolve()
if actual_unpack != expected:
    raise CellError(f"unpack_root canonical {actual_unpack} != inherited {expected}")
```

`validate_spec` body is outside the diff change set (only the following function was rewritten). Pairing rule (`cache_dir`/`unpack_root` both set or both omitted), non-empty string checks, and fail-closed `CellError` remain. Gate still compares **resolved** paths; only the *source* of `expected` gained Windows USERPROFILE awareness via the shared helper.

---

## Tests

Task handoff claimed 108 combined root Python tests. Independent Mac re-run in this review workspace:

```text
cd docs/memory && python3 -m unittest \
  test_operator_home test_run_managed_cell test_run_diagnostic \
  test_process_accounting test_windows_process_parent test_managed_resource_binding -q
→ Ran 108 tests in ~33s — OK
```

`test_operator_home` covers empty-HOME wins, USERPROFILE when HOME absent, non-Windows ignore USERPROFILE, and `unpack_root_path` shape. No git/native/live/remote/UI/build actions performed.

---

## Scope / residual notes (non-blocking)

- Thin wrapper adds one import edge from managed cell → `operator_home`; acceptable and preferred over duplicated HOME logic.
- Rust `unpack_dir` empty/missing returns a *relative* `.274bot/unpack`; Python builds an absolute base via `cwd`/`Path.cwd()`. Gate `.resolve()` equalizes when `cwd` is the process working directory (existing contract; not introduced here).
- `run_diagnostic` already used `bot_home_path` from the same module (prior reviews); this commit aligns managed unpack expectation with that shared resolver.
- Whole-branch `branchreviewer` still required separately.

---

## Verdict

**APPROVED**

`0dc87d7` correctly closes the managed-cell HOME/USERPROFILE lag by routing `expected_unpack_root` through the reviewed `operator_home.unpack_root_path`, preserves empty-HOME precedence, leaves the canonical unpack gate structure intact, and does not change non-Windows behavior. Focused 108-test suite OK on independent re-run.
