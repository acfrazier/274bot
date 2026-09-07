# TUI nav-capture correlation report

Branch: `codex/memory-diagnostics`  
Scope: bounded diagnostic instrumentation only — no live run, no behavior fix, no client/JS changes.

## What landed

### Enable / drain
- `BOT_NAV_CAPTURES=1` enables capture for **panel and TUI** (`memory.rs` no longer panel-gates `enable`).
- Launcher: `--nav-captures` allowed on **TUI** as data-only mode; panel still requires `--single-renderer` or `--focused-one`.
- TUI drain: `nav_capture::drain_json_files()` writes checkpoint JSON under `274BOT_SMOKE_DIR` (or `~/ .274bot/smoke`) with `"drain":"tui-data-only"`. No renderer select, no GPU screenshots, **no 25s terminal wait**.
- On TUI harness `Err`: immediate drain of existing ready checkpoints, print honest missing-capture note if empty, **preserve original error + exit 1**.
- Panel screenshot focus/timing remains in `panel::nav_capture` (unchanged).

### Correlation ring (enabled only)
- Per-slot ring cap **128** compact JSON rows; path sample cap **16** (head+tail); string cap **128**.
- Retain once per native snapshot/player edge **or** meaningful guardian/path/chat/route/walk-pending change; identical re-observations counted in `observed` but not retained.
- Counters: `observed`, `dropped`, `truncated_strings`, `truncated_paths`, `first_retained_tick`, `last_retained_tick`, `pending_walk_replaced`.
- Default-off: no path sample, no nav lock, no string build until `nav_capture::enabled()`.

### Path / tryMove honesty
- Call site uses `BoundedPath::sample(&c.try_move_path, nearest)` — **no full path clone/scan** into `ObserveInput`.
- Signature is over **original_len + nearest + head/tail sample only**.
- Field: `bounded_sample_last_change_observed_tick` (native snapshot tick when bounded sample sig last changed).
- Per row: `full_path_change_detection = !truncated` (false when truncated — middle-only changes undetectable); `resend_detection = "unavailable"`.
- `scene_base_for_path` = observe-time scene base (send-time base unavailable).
- Does **not** claim last tryMove/send time or server ACK. Walk acceptance = outbound encoding only.

### Guardian / route / walk
- Guardian from previous-frame `RandomStatus` (`status_lag: previous_frame`); strings bounded at `GuardianObs::bounded`.
- `npc_slot`, hop cursor, `ticks_waited`: **unavailable** (no extra Guardian/nav private accessors).
- Route: `route_generation`, `route_dest`, `current_aim` via existing `Traveller::current_aim` (not `remaining_walk_tiles`).
- `WalkAttempt` → `pending_walk` with replace counter; included in failure correlation **even before next observe**.

### First failure
- `failure()` once: builds `correlation_history` **once into checkpoint detail** (immutable JSON owner). No separate `sealed_at_failure` twin clone of the ring.
- Live ring may continue after first failure; checkpoint Value is independent.
- Full `GameSnapshot` still only at checkpoints (not every tick).

## Fields / limits (exact)

| Item | Limit / value |
|------|----------------|
| `RING_CAP` | 128 |
| `PATH_POINTS_CAP` | 16 head+tail |
| `STRING_CAP` | 128 chars |
| `READY_CAP` | 5 checkpoints |
| Guardian lag | previous_frame |
| tryMove note | bounded sample change tick ≠ send tick; no ACK |
| TUI failure | immediate drain + original Err |

## Remaining missing capability
- Private hop cursor / ticks_waited (not on public surface).
- Guardian NPC slot (not on `RandomStatus`).
- Full-path middle change + identical-path resend detection without client hooks.
- Send-time scene base for path points.
- No performance measurement on this card.

## Tests

```text
cargo test -p host-play --lib --features memory-profile nav_capture -- --test-threads=1
# 12 passed

cargo test -p host-play --lib --features memory-profile -- --test-threads=1
# 163 passed

python3 docs/memory/test_run_diagnostic.py -v
# 12 passed

cargo check -p tui --features memory-profile
# ok
```

No client suite (client unchanged). No live/perf acceptance.

## Files
- `crates/host-play/src/nav_capture.rs`
- `crates/host-play/src/lib.rs` (observe/walk/failure hooks only)
- `crates/host-play/src/memory.rs` (enable for all frontends)
- `crates/tui/src/bin.rs` (immediate data-only drain)
- `docs/memory/run_diagnostic.py` / `test_run_diagnostic.py`
- `docs/memory/tui-nav-capture-report.md`
