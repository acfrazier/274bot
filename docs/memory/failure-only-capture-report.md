# Failure-only capture report

Commit: `cd4df60` on `codex/memory-diagnostics`  
Scope: opt-in failure-boundary evidence without the periodic diagnostic stream.

## What landed

- Env: `BOT_MEMORY_FAILURE_CAPTURE=1` (resolved once at isolate / harness setup).
- Script stop-reason cache enables when diagnostics **or** failure-capture is set.
- Host-play: `Run.failure_capture` + latch `failure_boundary_attempted`.
- Shared production path `handle_harness_failure` (poll/advance Failed + script_last_error):
  - latch **before** slots producer and I/O
  - at most one best-effort failure-boundary qualification row
  - original `Err` preserved when capture is on (incl. writer / diag write fail)
  - when capture is off and diagnostics is on, legacy diag `?` still wins
- Launcher: `--failure-capture`; shared `build_child_env` used by `main` and tests; scrub when flag off; metadata `failure_capture`.
- Help: pair with `--no-diagnostics` for failure-only mode; flags remain independent.

## Not changed

- `mint_live_names` / live account identity (pre-existing PID+serial truncation stays separate).
- No live run, no frozen binary overwrite, no ScriptRunner / client / routing edits.
- No new per-tick JS reads, channel messages, or world deep-copies.
- Does not fix the original N16 stop cause; no performance acceptance.

## Tests (pipefail; exit codes from `echo EXIT:$?`)

```text
python3 docs/memory/test_run_diagnostic.py -v
# EXIT:0  (11 tests OK)

set -o pipefail
cargo test -p host-play --lib --features memory-profile -- --test-threads=1 2>&1 | tee /tmp/hp-full.log
# EXIT:0  (152 passed)

set -o pipefail
cargo test -p script --features memory-profile --test load_isolate 2>&1 | tee /tmp/script-li.log
# EXIT:0  (153 passed)
```

## Storage / work limits

- Failure row: one JSONL qualification line on the error path only (cached slot/client fields already on Play).
- No extra owners retained; writer fail does not allocate a second attempt.
- Failure-helper unit tests construct `Run` stubs with fixed `unit0` names + unique temp qualification paths (no mint/vault allocate except one minimal `prepare` env integration).

## Known separate issue (out of scope)

- `mint_live_names` truncates `{pid:x}{serial:x}` into a short token; after many mints in one PID, uniqueness / vault-already-exists can fail under heavy prepare test load. Not fixed here.
