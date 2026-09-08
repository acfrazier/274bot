# Panel memory capture attachment lifecycle

Date: 2026-09-08  
Branch: `codex/panel-memory-capture-arm`  
Scope: source-only repair of the memory harness forced-pane path that stranded
`capture_tx` after a real Game pane close. No native/SSH/launcher/cohort-reader
/client/STATE edits. Not a performance claim.

## Native evidence (root, prior)

Windows candidate input diagnostic (`BOT_DEBUG=1`) with visually confirmed
ingame scene2, slot0 focused, capture ON, Game Image hovered, and reviewed
`f64b81d` helper pulses showed:

- gate seq2: `capture_tx=1`
- gate seq3 onward: `capture_tx=0` through all pulses
- 40 win/imgui/stream arrow entries; every stream `channel=0` with correct slot id
- no drain / metric_start; zero published input counts

That localizes **no capture channel**, not injector failure or missing metric rows.

## Root cause (reproduced)

Frame order in `ui_frame`:

1. `Session::memory_focus` → `focus::memory_draw_policy` (can write `game_pane_open = true` directly)
2. later `game_window` → `Session::set_game_pane_open(built.is_some())`

`set_game_pane_open(false)` correctly runs `capture_off` and drops the sender.
On the next harness frame, `memory_draw_policy` set `game_pane_open = true`
without going through the Session open transition. Then
`set_game_pane_open(true)` saw `was == true` and **skipped** `capture_on`.
Same focused name also skips `select`/`apply_focus` reattach.

Deterministic Session test
`memory_draw_policy_direct_open_strands_capture_without_owned_edge` locks this
in: after pane close + direct policy open + `set_game_pane_open(true)`,
`capture_tx` stays `None` while `capture` and `game_pane_open` remain true.

## Fix

Minimum ownership change in `Session::memory_focus` / new private
`memory_focus_at`:

- Still apply `memory_draw_policy` for renderer/wall/cadence bits.
- For FixedOne / FocusedOne / FocusedPlusBackground, roll back the policy's
  direct `game_pane_open = true` write to the prior value, then call
  `set_game_pane_open(true)` so a real closed→open edge reattaches one channel.
- Already-open + capture on: `set_game_pane_open(true)` is a no-op for the
  channel (no per-frame recreate, queued edges kept).
- Capture pref off: open edge does not attach.
- RotatingAll does not force pane open (unchanged).
- Focus switch still goes through `select` (old drain disabled, current attached).
- Legacy pane close/reopen via `set_game_pane_open` alone unchanged.

Comment on `memory_draw_policy` documents Focus-level flag vs Session edge
ownership. No metric/admission/input synthesis changes.

## Tests

```text
cargo test -p panel --features memory-profile-no-alloc --lib memory_
  → 9 passed (focus policy + new session capture lifecycle)

cargo test -p panel --features memory-profile-no-alloc --lib -- capture
  → 18 passed (legacy close/reopen, stream_capture, prefs, new lifecycle)
```

Covered:

| Case | Result |
| --- | --- |
| Direct policy open after close strands channel (confounder) | asserted |
| capture attached → pane close → memory_focus → pane open → channel + key to SlotInput | pass |
| repeat memory_focus stable channel keeps queued edges | pass |
| capture pref off stays off through memory_focus + open | pass |
| focus switch disables old drain, attaches current | pass |
| legacy close/reopen / stream_capture / pref persist | pass |

No live secrets; no native test claims.

## Files

- `crates/panel/src/session.rs` — `memory_focus` / `memory_focus_at` + tests
- `crates/panel/src/focus.rs` — ownership comment on `memory_draw_policy`
- `docs/memory/panel-memory-capture-arm-report.md` — this report

## Out of scope (root)

Identical application to both native roles, combined Grok 4.6 review, rebuilds,
and one diagnostic rerun justified by this confounder. No performance acceptance.
