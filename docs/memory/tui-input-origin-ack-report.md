# TUI input origin acknowledgment across focus changes

Task: `t_3ec8a043`  
Branch: `codex/memory-diagnostics` (worktree `t_a1f4796f`)  
Scope: instrumentation-only producer correction in `crates/tui/src/bin.rs` + this report.  
No production UI/action ordering changes, client changes, sleeps/timeouts, wire changes, or reader-gate relaxation. Original failed latency artifacts preserved untouched.

## Problem (source-cited)

Loop order in `run_loop`:

1. `session.pump(&mut app)` — under `memory-profile`, sets `app.focused = Some(run.focus_index())` and `play.focus(...)` (`bin.rs` pump ~943–945).
2. `terminal.draw(...)` then (old) `note_tui_draw_flush(slot_id_for(focused_name))` after success.
3. One event read; on Key/MouseDown, `note_input_start(slot_id_for(focused_name), …)`.

So a start recorded against focus **A** is only completed on a later iteration. Before that draw, pump can advance focus to **B** (`Run::focus_index = (elapsed/30)%N`). Host `note_tui_draw_flush` completes pending only when `p.surface == Tui && p.slot_id == flush slot` (`responsiveness_profile.rs` ~871–877). Flushing **B** leaves **A** open (stranded pending / long completion / overflow class). Distinct from correctly fail-closed metrics reader and from decode edge pending.

Audit parent `t_da2f4634` / `1ee7a93`: hypothesis §A.1; 660/690/720 s boundary tie-in; this task owns the producer fix.

## Correction

Bounded `Option<u64>` `input_flush_origin` in `run_loop`:

| Step | Behavior |
|:---|:---|
| Successful `note_input_start` | `remember_tui_input_origin` stores that slot id |
| `note_input_start` false (disabled/dropped) or no focus | origin unchanged / unset — **no fabricated ack** |
| Successful `terminal.draw` | `flush_tui_input_origin` takes origin and `note_tui_draw_flush` **that** slot |
| Draw error (`?`) | origin not taken; no flush |
| No retained origin | flush is a no-op (does not flush post-pump focus) |

Helpers: `remember_tui_input_origin`, `flush_tui_input_origin`, `note_tui_input_start_for_focus`. One event per loop ⇒ single Option; no per-loop allocation or registry scan. Event processing, draw/pump/dispatch order, terminal errors/exits unchanged. Panel API untouched.

### Restart / Drop (host API investigation)

`Local` Drop (`responsiveness_profile.rs` ~746–766): discards decode bridge for generation; **cuts all `INPUT_PENDING` for `slot_id`** and increments `input_lost_n`; ended flush. Pending is slot-keyed, not generation-keyed. TUI Option stores **slot_id only** — does **not** claim generation coverage beyond existing Drop/lost hooks. After Drop, a stale origin flush is a host no-op (no matching pending).

## Regression evidence (deterministic host probe)

Tests in `bin.rs` `tests` (serialized on `INPUT_ORIGIN_TEST_LOCK`; unique slot names):

| Test | Proof |
|:---|:---|
| `old_focus_at_draw_flush_strands_prior_origin` | Start A; old helper flushes B → A `complete=0` `pending=1`; B `start=0` `complete=0` |
| `origin_flush_completes_a_when_focus_moves_to_b` | Start A via helper; flush retained origin → A `complete=1` `pending=0`; B not fabricated |
| `origin_flush_same_focus_still_completes` | Same-slot path still pairs |
| `no_start_does_not_fabricate_ack_on_draw` | No focus / `started=false` → no origin; flush no-ops; counters stay 0 |
| `origin_not_flushed_until_draw_success` | Origin held without flush call; pending remains until later successful flush |

Old path is exercised via `flush_tui_input_focused_at_draw` (test-only), not field assignment alone.

### Commands

```text
cargo test -p tui --lib --features memory-profile -- --test-threads=1
# result: 92 passed; 0 failed
```

(Also verified the five origin tests individually without feature flag.)

Host crate tests not required (host API unchanged). No new live run/build matrix (orch owns binary freeze/smoke after review).

## Producer-fix evidence and limits

**Fixed here:** TUI start/flush mis-attribution when focus rotates between start and next successful draw — root cause class for stranded A / long open pending under 30 s rotation.

**Not claimed:**

- Decode edge `boundary_pending_incomplete` geometries (still unresolved; out of scope).
- That every historical N16 overflow/open streak was solely this bug (hypothesis alignment + unit proof; live re-measure is orch post-review).
- Generation-scoped input pending beyond existing `Local` Drop lost accounting.
- Reader/gate changes; prior failed artifacts reinterpreted as pass.

## Files

- `crates/tui/src/bin.rs` — origin retain + flush; regressions
- `docs/memory/tui-input-origin-ack-report.md` — this report
