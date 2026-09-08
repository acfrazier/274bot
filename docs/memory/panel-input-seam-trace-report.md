# Panel input seam trace — diagnostic capability report

Date: 2026-09-08  
Branch: `codex/panel-input-seam-trace`  
Scope: independently removable, bounded Left/Right arrow delivery observation for the unresolved native 0942 input-zero-start result. **Not** a native delivery proof or root-cause claim.

## Goal

Enable one later root-controlled native run (`BOT_DEBUG=1`) to distinguish, via independent stage counts and truthful identities:

1. `WindowEvent` Left/Right received before platform forwarding  
2. ImGui Left/Right edges (even when GameImage gate fails)  
3. capture / hover / draw / slot-focus / window-focus / channel gate  
4. `stream_capture_for` entry (channel + slot Option + slot_id)  
5. host `SlotInput` drain of Left/Right keys  
6. metric `note_input_start`: admitted vs row_match vs live_row_match vs counter_bump + row_gen  
7. mailbox gen bind (`bound_n`) and host-texture present (`removed_n` vs `published_n`)

## Source hooks (removal boundary)

| Stage | Location | Notes |
| --- | --- | --- |
| Module | `crates/host/src/input_seam_trace.rs` | Delete module + call sites |
| Export | `crates/host/src/lib.rs` (`pub mod input_seam_trace`, `debug_flag`) | |
| Window | `crates/panel/src/window.rs` | Left/Right `KeyboardInput` before winit→ImGui forward |
| App gate/ImGui | `crates/panel/src/app.rs` | `observe_input_seam_arrows_and_gate`; **only when** `input_seam_trace::enabled()` — preserves original `capture && is_item_hovered` short-circuit when off |
| Stream | `crates/panel/src/session.rs` `stream_capture_for` | Entry for Left/Right keys only |
| Drain | `crates/host/src/slot_io.rs` | Left/Right `InputEv::Key` after enabled drain |
| Metric | `crates/host/src/responsiveness_profile.rs` | `note_input_start` / `bind_input_to_mailbox_gen` / `note_panel_present` |

No host-play `memory.rs`, cohort module, client, input helper, Python reader, or STATE edits.

## Gate and disabled-path invariants

- **Opt-in:** cached `BOT_DEBUG=1` env once, or host `set_debug` latch via `debug_flag()` (non-allocating). No new runtime flags.  
- **Disabled:** one `enabled()` check only at each call site / note entry — no timestamps, allocations, locks, IO, or changed evaluation ordering beyond that check.  
- **Responsiveness path:** `live_generation_for` remains **cohort-enabled only** (restored). Trace facts come from the same `with_live_slot_from` row mutation (`LiveSlotHit`: `matched`, `live`, `generation`) — not a second later REGISTRY probe.  
- **bind/present:** no extra REGISTRY locks for tracing; zero-work calls do not burn the actionable stage cap (separate heartbeat budget).  
- **App:** when tracing is off, `ui.is_item_hovered()` stays inside the original short-circuit (`capture && …` / `is_focused && capture && …`).

## Output contract

One stderr line per emit (native wrapper already preserves stderr):

```text
[input-seam] seq=<u64> mono_ns=<u64> stage=<tag> k=v …
```

| `stage` | Cap | Body fields (stable keys) |
| --- | --- | --- |
| `win_arrow` | 48 | `key=left\|right` `down=0\|1` `repeat=0\|1` |
| `imgui_arrow` | 48 | `key` `down` `edge=press\|release` |
| `gate` | 24 (≤8 non-transition initials) | `draw` `capture` `hover` `slot_focused` `win_focused` `pane` `capture_tx` |
| `stream` | 48 | `key` `down` `channel` `slot_opt` `slot_id=0x…\|-` |
| `drain` | 48 | `key` `down` `enabled` |
| `metric_start` | 48 | `slot_id` `admitted` `row_match` `live_row_match` `counter_bump` `row_gen` |
| `gen_bind` | 48 actionable; 4 zero-work HB | `slot_id` `gen` `bound_n` `heartbeat=0\|1` |
| `present` | 48 actionable; 4 zero-work HB | `slot_id` `gen` `removed_n` `published_n` `row_match` `live_row_match` `row_gen` `heartbeat` |
| `saturate` | 48 | `of=<stage>` `emitted` `suppressed` (first overflow per stage) |

Parse: whitespace-separated `k=v`; require `seq`, `mono_ns`, `stage` (`parse_record`).

**Semantics**

- Stages are **independent counts** — no claim of 1:1 cross-stage event correlation.  
- `row_match` ≠ `live_row_match`: ended-row fallback can match without a live row.  
- `admitted` can be 1 with `row_match=0` (pending push without registry row; no published `input_start_n` bump).  
- `removed_n` = queue removals; `published_n` = registry complete counter bumps (0 if no row).  
- Present endpoint is **host texture present**, not display scanout.  
- Not a performance measurement.

`force_for_test` / `reset_for_test` are `cfg(test)` only.

## Tests (exact)

```text
cargo test -p host --lib input_seam -- --test-threads=1
  → 7 passed (module unit + producer path)

cargo test -p host --lib
  → 205 passed; 1 ignored

cargo test -p panel --lib stream_capture
  → 3 passed

cargo test -p panel --lib capture_keys
  → 1 passed
```

Covered: disabled silence/zero stage work; parse format + STAGE_CAP saturation + explicit saturate; gate initial reserve + transitions; bind/present heartbeat vs actionable; metric field contract; producer `note_input_start` orphan admitted-without-row vs live-row counter bump + bind/present.

## Native procedure (later root-controlled)

Prerequisites: same capture-enabled, slot-0 focused, GameImage point as 0942; build with this branch; `BOT_DEBUG=1`; responsiveness profile enabled if metric/bind/present stages are required (those hooks sit behind existing profile `ENABLED`).

Limitations: panel must actually receive OS keys; ImGui observation does not force hover/focus; stderr volume is capped — after cap only suppress counts grow; warmup bind/present use ≤4 heartbeats each so controlled pulses remain visible.

## Known unresolved cause

Unchanged from `panel-input-zero-start-audit.md`: 0942 proves injector SendInput success and zero published input starts; it does **not** localize among WindowEvent delivery, ImGui translation, hover/capture gate, channel attachment, drain, or metric identity mismatch. This task only prepares the diagnostic.

## Removal

1. Delete `crates/host/src/input_seam_trace.rs` and `pub mod` / `debug_flag` if unused elsewhere.  
2. Revert call sites in `window.rs`, `app.rs`, `session.rs`, `slot_io.rs`, `responsiveness_profile.rs`.  
3. Delete this report if desired.
