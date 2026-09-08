# Native panel input zero-start audit (0942)

Date: 2026-09-08

## Result

The 0942 native injector completed successfully, but the run provides no host
input coverage. This is an evidence-boundary failure, not proof that Windows
`SendInput` failed and not proof of a Rust host drain defect.

The immutable archive directory is `diagnostics/windows-input-smoke-0942-archive`.
SHA-256 `c7ab1ae3bcfd3ceb7f373090a1e133e7e1d0dc08d6042641caa8bfc441e52fb6`
is the hash of its `diagnostics/windows-input-smoke-0942.tar.gz` archive.
That archive's manifest verifies all 29 files. The root receipt reports 381 sample rows
and maxima of zero for every input field across the whole run:

- `input_start_n=0`
- `input_complete_n=0`
- `input_pending_n=0`
- `input_canceled_n=0`
- `input_lost_n=0`
- `input_dropped_n=0`
- `input_latency_n=0`

The native receipt records 20 requested and 20 completed alternating Left/Right
pulses. Every down/up `SendInput` call returned 1, every release succeeded,
and actual pre-send lateness was 9--26 ms. Therefore the external stimulus
receipt proves only that the guarded injector called Windows successfully; it
does not establish delivery to the panel's ImGui input state or to the focused
host slot.

## Frozen visual and configuration evidence

The three captures use the same PID 11236, session 2, binary SHA-256
`a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f`, and
physical window rectangle `[228,228,2326,1154]`:

- `initial-scene2-0942`: active Thiever scene 2 is visible.
- `general-config-0942`: slot 0 is selected, capture is checked, and focused
  50 is checked.
- `after-twenty-arrows-0942`: active Thiever remains visible after the pulse
  schedule.

The helper's point `[628,528]` is `[400,300]` relative to the window and is
inside the Game Image. These facts establish the intended target and visual
state, but a screenshot cannot establish ImGui hover state, a received
`WindowEvent`, or host-slot acknowledgement. The before/after row-B records
remain the same slot/generation in observe phase with all input counters zero.

## Source trace at the frozen binary source

`a90747a` (the responsiveness instrumentation) is an ancestor of `fb3589a`,
so the source path below is present in the frozen binary's source history.
The relevant path is:

1. `crates/panel/src/window.rs:1269-1350` forwards ordinary `WindowEvent`s to
   `dear_imgui_winit::WinitPlatform::handle_event`. It does not directly send
   native key events to a host slot.
2. `crates/panel/src/app.rs:1093-1155` computes `should_capture`, requires the
   Game Image to be drawn, and calls `stream_capture_for` only inside
   `capture && ui.is_item_hovered()`.
3. `crates/panel/src/app.rs:1355-1376` derives capture keys from ImGui's key
   state (`is_key_pressed_with_repeat` / `is_key_released`) and maps the
   resulting keys to GameShell character values.
4. `crates/panel/src/session.rs:396-443` returns immediately when
   `capture_tx` is absent, stamps `input_start` only for left/right-down or
   key-down edges, then sends `Move`, `Down`, `Up`, and `Key` events.
5. `crates/panel/src/session.rs:2328-2370` attaches `capture_tx` and enables the
   focused slot only when capture is enabled and a focused slot exists.
6. `crates/host/src/slot_io.rs:230-273` drains the receiver only when the
   slot input is enabled. `Down` and key-down are the actionable host edges.
7. `crates/host/src/lib.rs` binds still-unbound focused starts to a mailbox
   generation on mailbox store (including stores after a drain), while
   `crates/panel/src/app.rs` completes the panel metric only after
   `GameView::present` of that generation. The published endpoint is a host
   texture bind/upload, not display scanout.

The same source also handles the MultiBox focused cell through the analogous
`app.rs:1221-1236` capture call. `memory_draw_policy` changes draw/focus cadence
and renderer selection; it does not enable or disable capture.

## What the evidence supports

The strongest source-supported conclusion is bounded:

> The external key schedule produced no published panel input-start samples.

The first intended observable seam is `stream_capture_for`, not Windows
`SendInput`, but the zero counter does not prove that this helper was never
called. `note_panel_input_start` returns early for an empty slot name, and
`note_input_start` can admit/push a pending start even when its live registry
lookup finds no matching row; the published counter is bumped only for a
matching live row. Thus the remaining possibilities include native event
delivery/platform translation, ImGui key-state production, the Game Image
hover gate, capture-channel attachment, or a missing/mismatched slot identity
at the instrumentation seam. The evidence does not distinguish those
boundaries.

This is not established as a `wScan == 0` defect. The pinned dependency set is
`dear-imgui-winit 0.15.1` and `winit 0.30.13`; the winit native mapping path
has a zero-scancode fallback via `MapVirtualKeyExW`, and the client-side
winit path maps `KeyEvent.logical_key`. Thus zero `wScan` alone is not a
supported root cause.

The old item-clobber hypothesis is also unproven and should not be used as the
root cause: current `crates/panel/src/paint.rs:51-97` draws the script overlay
with the Game window draw list, not a second ImGui window. The queue and paint
overlays therefore do not, from source alone, explain the missing starts.

## Bounded next diagnostic

Do not repeat the native run as part of this audit. The next diagnostic should
add one temporary, independently removable observation at the panel seam (or
use an existing debug hook if available) and distinguish these cases in one
controlled run:

1. native `WindowEvent::KeyboardInput` received by the panel window loop;
2. corresponding ImGui key transition visible to `capture_keys` while the
   Game Image is hovered;
3. `stream_capture_for` called with an actionable edge;
4. `InputEv::Key`/`Down` drained by the focused slot;
5. at the instrumentation seam, record the slot name, `slot_id_for(name)`,
   whether `with_live_slot_from` matched a live row, and whether
   `note_input_start` admitted the start versus bumped a published counter.

Keep the existing capture-enabled, slot-zero-focused, fixed Game Image point
and record the four event counts plus the identity/live-row outcome separately.
A native-event count with zero ImGui
transitions localizes the issue to platform translation; ImGui transitions with
zero seam calls localize it to hover/capture gating; seam calls with zero host
drain localize it to channel/slot attachment. A drain count alone is not full
coverage: require start, generation bind, present, and closed counts (with
identity/live-row status) before claiming input coverage. Do not alter UI
behavior to force the metric, and do not treat `GameView::present` as
display-visible acknowledgement.

## Limits

No Rust or helper code was changed. No new native/SSH/input/build run was
performed. The report uses the frozen 0942 receipts, the completed immutable
archive, and source inspection only; performance acceptance remains false.
