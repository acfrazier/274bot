# Native script paint buttons through both frontends

Task `t_9b6fbcdf` implements brief 113 / design `04ad-paint-buttons-design.md`
sections 4–verification on `codex/rs2b0t-multirevision`. Client gitlink
`9d090ed04957e4efc254f073cda97bc5510ca72b`. No live actor. Root owns LIVE
and native/TUI controls proof.

## Mapping

Foreign `Paint.buttons({id,label}[])` is a host-owned descriptor plus a
one-shot isolate command. JS records `{id,label}` on `ScriptPaint`, returns
the pending id only when this `buttons()` call advertised it, otherwise
`null`. Headless never fabricates a click. Labels are display-only.

`IsolateCmd::PaintClick { id, generation }` is processed like Pause: dropped
when paused or when isolate `work_generation` mismatches. Unconsumed
`paintClick` is cleared after each paint eval, on Pause, and on ResetSession.
Hold still paints and may consume; Pause does not start painting.

Rendered-frame generation is host-owned and is **not** on the FlatBuffer.
Each isolate spawn takes a unique `paint_generation`; session reset bumps it
and re-stamps the last forwarded frame. Panel and TUI pass that generation
with the id. `Play::script_paint_click(name, id, generation)` rejects before
forwarding when the generation does not match the current frame, even if the
new script advertises the same id.

## Frontends

Panel overlay draws a real ImGui button per advertised label inside the
chatbox paint, not on Pause/Stop chrome. Collapse remains view-local and
hides the widgets. Click returns `(id, generation)` to
`Session::script_paint_click`.

TUI paint-as-chat renders `[n] label` with j/k+Enter and digits `1..=9`.
Modal still wins. `ChatAction::PaintButton(index)` maps through the rendered
`script_paint` generation; it is not `WireCmd`. Host Pause/Stop keys stay
`script_toggle_pause` / `script_stop`.

## Checks

Exclusive target
`.superpowers/review-exports/native-paint-buttons-t_9b6fbcdf-target`.
Isolated export
`.superpowers/review-exports/native-paint-buttons-t_9b6fbcdf`.
Not LIVE.

- `cargo test -p script --test paint_buttons`: 11 passed (no throw, null
  default, one-shot, unadvertised clear, pause no replay, hold consume,
  two isolates, generation skip, stale-frame same-id, two `buttons()`
  calls, stepper still missing).
- `cargo test -p script --lib paint`: FB round-trip + truncated err.
- `cargo test -p script --test load_isolate isolate_paint`: empty buttons
  default preserved.
- `cargo test -p panel --lib paint::tests`: overlay button click + collapse.
- `cargo test -p tui paint`: digit/Enter PaintButton, modal wins, no WireCmd.
- `cargo test -p host-play --lib script_paint`: Idle/Paused/unadvertised
  no-op; label toggle publishes.
- `cargo clippy --no-deps -D warnings` on `script` lib, `script --test
  paint_buttons`, `host-play` lib.
- `rustfmt --edition 2021 --check` on owned Rust.

Panel/TUI whole-crate clippy `-D warnings` still hits pre-existing
`type_complexity` on traveller maps; not introduced here.

## Limits

Root owns LIVE paired Air / NatureCrafter `gobank` click, catalog harness
auto-click refusal, and cache cleanup. Not LIVE acceptance. Headless Air can
proceed once `buttons()` stops throwing (null). Operator park still needs
the frontend click path. `stepper`/`list`/`fill`/`wrap`/`cols` stay
`not impl`. Host Pause/Stop stay host-owned.
