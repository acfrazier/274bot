# TUI script paint button visibility and hit testing

Task `t_193e97fb` fixes the TUI-only follow-up to native paint buttons. No runtime, client, scenario, JavaScript, panel, or LIVE behavior changed.

## Result

The app now gives an active paint-with-buttons pane a bounded preferred height instead of the fixed six-row chat allocation. The observed NatureCrafter title, three status rows, spacer, and `Go bank` control fit in a 140x40 render. Dialogue retains the ordinary chat height and input priority.

`Chat` computes one paint layout shared by rendering and mouse dispatch. Button rows are pinned below a reserved spacer, their hit rectangles cover only visible rendered label cells, and constrained layouts keep the focused button in the visible window. Wrapped or clipped body text cannot shift a click target. Title, body, spacer, trailing blank cells, borders, and nonvisible rows do not dispatch `PaintButton`.

## Regression coverage

Actual `TestBackend` renders cover the realistic NatureCrafter frame at 140x40 and 48x18, locate the rendered `Go bank` label, and click its real buffer coordinates. A compact 24x7 pane exercises wrapped-body clipping and rejects clicks on title, body, spacer, border, and trailing blank cells. Existing digit/Enter, modal-priority, Pause/Stop, paint-toggle, and script-generation paths remain covered by the TUI suite.

Verification used the frozen `cd49e147ade94bc5b06e7798e08ed4ad05a5d245` host export with client `aef3952d1cd7bb3b93d39c497f0f476b68021c59`, overlaid only with `crates/tui/src/{app,chat}.rs`, and exclusive target `target-t_193e97fb-r2`.

- Owned-file `rustfmt --check`: pass.
- `cargo test -p tui`: 114 passed, 0 failed.
- Strict all-target TUI Clippy baseline: blocked only by four pre-existing `clippy::type_complexity` findings in out-of-scope `crates/tui/src/bin.rs`.
- All-target TUI Clippy with that exact baseline lint allowed and every other warning denied: pass.

Raw logs and source hashes are under `docs/compat/evidence/native-paint-buttons/tui-visibility/`. Root still owns the real PTY rerun; this task did not run LIVE.
