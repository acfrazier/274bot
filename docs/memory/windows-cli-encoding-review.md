# Review: Windows CLI help encoding correction (968846c)

**Task:** t_f74f3c59  
**Reviewer:** Grok-4.5 (profile `reviewer`)  
**Round:** 1 (artifact lens)  
**Branch:** `codex/memory-diagnostics`  
**Commit:** `968846ca8945827d81540e313e6d99f3c6d318fa`  
**Verdict:** APPROVED

## Scope checked

Two-file diff only (matches task bound):

| File | Change |
| --- | --- |
| `docs/memory/run_diagnostic.py` | Help strings: `→` → `-to-` words; `≤` → `<=` |
| `docs/memory/test_run_diagnostic.py` | New `test_help_survives_windows_redirected_output_encoding` |

No parser dest/flag names, validation, env wiring, ConPTY/process, or runtime behavior changes. Out of scope for this card: `410eba6` ConPTY/process work (already reviewed elsewhere).

## Claim → evidence

| Claim | Evidence |
| --- | --- |
| Native Windows redirected cp1252 help failed on U+2192 (and ≤ U+2264) | Parent tree under `PYTHONIOENCODING=cp1252`: `--help` exits 1 with `UnicodeEncodeError: 'charmap' codec can't encode character '\\u2192' ...` from argparse `print_help` → `file.write` |
| ASCII replacements fix help without behavior change | Current help lines 52–53 use `decode-to-script` / `input-to-UI` and `<=1ms`; flag names and `action='store_true'` unchanged |
| Regression test uses cp1252 + real help exit 0 | Test sets `env={**os.environ, 'PYTHONIOENCODING': 'cp1252'}`, runs `[sys.executable, SCRIPT, '--help']` with binary capture, asserts `returncode == 0` and `b'--responsiveness-fine' in stdout` |
| Twenty CLI tests pass on Mac | `cd docs/memory && python3 -m unittest test_run_diagnostic -v` → **Ran 20 tests … OK** (includes new test) |

## Independent checks (this review)

1. Cold read of `git show 968846c` — only help text + one test; no runtime path edits.
2. Parent reproduction: copy of `968846c^` `run_diagnostic.py` run from `docs/memory` with `PYTHONIOENCODING=cp1252` → exit 1, exact U+2192 encode failure.
3. Current: same env → exit 0; help contains `decode-to-script`, `input-to-UI`, `<=1ms`; `iconv -f cp1252 -t cp1252` OK; no UTF-8 multibyte lead bytes in help stdout.
4. Full suite: 20/20 OK from `docs/memory`.
5. `py_compile` on both files OK.

## Residual notes (non-blocking)

- Pre-existing U+2014 em dash remains in a **comment** near ConPTY spawn (~L296). Comments are not printed by argparse help; not part of this failure mode or this commit’s scope.
- New test asserts exit 0 + flag presence under cp1252; it does not pin the replacement phrasing. Sufficient for the encode-crash regression.
- This host is macOS; native Windows Python 3.14 was not re-run here. The portable `PYTHONIOENCODING=cp1252` path reproduces the reported codec failure and fix.

## Conclusion

Minimal, correct fix for Windows code-page help crashes. Behavior-preserving; regression is real and green. **Approve.**
