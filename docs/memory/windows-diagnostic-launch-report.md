# Windows diagnostic launch portability (panel import / home / RS2B0T)

**Task:** t_9778aec4  
**Branch:** `codex/memory-diagnostics` (verified before edits)  
**Workspace:** `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`  
**Scope owned:** `run_diagnostic.py`, `test_run_diagnostic.py`, new `operator_home.py` + `test_operator_home.py`, this report.  
**Not owned / not edited:** `run_managed_cell.py`, `process_accounting.py`, related managed tests. No git mutations, Rust/client, remote/live/Windows invocation.

**Not claimed:** full Windows managed-runner readiness, ConPTY TUI, matched cells, savings, or native Windows live proof (root-owned after helper integration).

Audit prerequisites covered: **B4** (import/panel only), **B6** (Python home parity helper + diagnostic catalog), **B8** (explicit RS2B0T / no Mac default on Windows). Source: `windows-managed-runner-audit.md`.

---

## Changes

### B4 — Lazy Unix TTY stack; panel import without PTY

- Removed top-level `fcntl` / `pty` / `termios` imports from `run_diagnostic.py`.
- `require_terminal_transport()` loads them only on the real TUI terminal path.
- `sys.platform == 'win32'`: clear `RuntimeError` / argparse error — never silent headless.
- Missing modules on a non-Windows platform flag: clear fail-closed error.
- Non-terminal path (panel, `tui --headless`) remains plain `subprocess.Popen` (unchanged).
- Unix real PTY path (`openpty`, `TIOCSWINSZ` 40×120, `setsid`/`TIOCSCTTY`, drain thread, input probes) preserved when modules exist.

### B6 — Operator home parity (helper)

New pure module `docs/memory/operator_home.py` mirrors Rust `operator_home_from`:

| Platform | Rule |
|----------|------|
| non-Windows | `HOME` only (`USERPROFILE` ignored) |
| Windows | Explicit `HOME` wins **including empty**; else `USERPROFILE`; both absent → unavailable |

Also exposes `bot_home_path` (empty/missing → `"."`) and `unpack_root_path` (non-empty home → `$home/.274bot/unpack`; else `{cwd}/.274bot/unpack`).

`run_diagnostic.catalog_path_for_env` uses `bot_home_path` so Windows sessions with only `USERPROFILE` resolve `.274bot/js-scripts.json` under the profile, matching client catalog defaults. Explicit empty `HOME` does **not** fall through to `USERPROFILE`.

### B8 — RS2B0T provenance

- Historical Mac default `/Users/acfrazier/experiments/rs2b0t` applied via `setdefault` **only when platform ≠ win32**.
- On win32, unset `RS2B0T` is left unset; `resolve_rs2b0t_commit` requires an explicit non-empty path that is a real directory and returns real `git rev-parse HEAD` only — **no fabricated SHA**.
- Missing path / git failure → argparse error before run dir / process start.

---

## Integration required in `run_managed_cell.expected_unpack_root` (other agent / root)

**Do not leave** the current HOME-only body:

```python
def expected_unpack_root(*, cwd: Optional[pathlib.Path] = None) -> pathlib.Path:
    home = os.environ.get('HOME')
    if isinstance(home, str) and home:
        return (pathlib.Path(home) / '.274bot' / 'unpack')
    base = pathlib.Path(cwd) if cwd is not None else pathlib.Path.cwd()
    return base / '.274bot' / 'unpack'
```

**Exact replacement** (import the shared helper; preserve optional `cwd`):

```python
from operator_home import unpack_root_path

def expected_unpack_root(*, cwd: Optional[pathlib.Path] = None) -> pathlib.Path:
    """Client bot_target::unpack_dir via operator_home (HOME/USERPROFILE parity)."""
    return unpack_root_path(cwd=cwd)
```

Optional test knobs (if managed tests inject platform/env without mutating process env):

```python
return unpack_root_path(cwd=cwd, environ=environ, windows=windows)
```

Semantics to preserve after the swap:

1. Non-empty `HOME` → `Path(HOME)/.274bot/unpack` on all platforms.
2. Windows, `HOME` **absent**, `USERPROFILE=C:\Users\op` → `C:\Users\op\.274bot\unpack`.
3. Windows, `HOME=""` (explicit empty) → `{cwd}/.274bot/unpack` (**not** USERPROFILE).
4. Non-Windows, `HOME` absent → `{cwd}/.274bot/unpack` even if `USERPROFILE` is set.
5. Canonical preflight compare against spec `unpack_root` must keep fail-closed inequality (no gate weakening).

`operator_home.unpack_root_path` already implements (1)–(4). Managed-cell agent owns the one-line call-site + any `test_run_managed_cell` matrix updates; this card does not edit those files.

---

## Tests run (Mac)

```text
cd docs/memory && python3 -m unittest test_operator_home test_run_diagnostic -v
Ran 24 tests in ~0.5s — OK
```

Includes:

- HOME/USERPROFILE matrix (explicit empty, profile fallback, non-Windows ignore profile).
- Import under **genuinely blocked** `pty`/`fcntl`/`termios` (meta_path blocker subprocess): panel argv validate + child env; Windows TUI terminal fail-closed; missing-module fail-closed.
- RS2B0T Mac default vs win32 no-default; fail-closed missing path; real git HEAD when path is a repo.
- Existing CLI/validate/env scrub cases unchanged.
- Unix `require_terminal_transport` loads real modules on this host.

---

## Explicit non-claims / remaining blockers

- No ConPTY (audit T5); real Windows TUI terminal remains unsupported by design until a later task.
- No managed-cell stop/cleanup/parent-PID/module-bind work (T1–T3, T7).
- No native Windows process launch or RDP proof.
- Gates on metadata/source/binary/evidence unchanged; no broad new CLI flags.
- Full Windows readiness: **not** claimed.
