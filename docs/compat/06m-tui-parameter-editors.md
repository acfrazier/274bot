# TUI editors for existing catalog parameter types

Task: `t_32400aa2`
Brief: `docs/compat/briefs/82-tui-parameter-editors.md`
Branch: `codex/rs2b0t-multirevision`
Kind: TUI params overlay. Not LIVE, not a harness rewrite, not new settings types.

## Result

The Params overlay now edits the catalog types the native panel already
supports, using shared `coerce_setting_value` / `format_setting_value` /
`resolve_setting_options` and the existing per-card store. Bool toggle and
optioned-string cycling stay immediate. Number, free-text string, tile, list
and unoptioned `string[]` use Enter to edit, Enter to save, Escape to cancel.
Optioned `string[]` opens a checklist; Space toggles, Enter saves, Escape
cancels. Invalid tile scratch and failed saves stay local errors and do not
replace the previous bag value. The overlay `Clear`s its popup so map/status
glyphs no longer bleed through labels, long schemas scroll with the cursor
kept visible, and open-form keys are consumed (`q` does not quit).

CAF6b809 Concord PTY (`Items to alch` / `Alchs per trip` uneditable, glyph
bleed) is the issue this covers. Root owns native PTY editing, reopen, and
Start/settings effect after review.

## Ownership

- `crates/tui/src/script_params.rs` owns keyboard editing, overlay paint,
  persist/cancel, and focused tests.
- `crates/tui/src/app.rs` resets editor state on open, consumes keys while
  the form is open, and covers numeric persist through Start merge.
- Shared schema/coercion/store, panel, bin.rs, client, fixtures, catalog
  scripts and ledgers were not edited.

## Verification

Peer WIP was not restored or stashed. Checks ran on an exact Git-blob export
of committed host `bca3429ed36dfb5b8e35fcfff0a0880c55d768a6` plus this card's
overlays:

`.superpowers/review-exports/t32400aa2-tui-params`

Target dir (fresh empty): `.superpowers/review-exports/t32400aa2-tui-params-target`

- Host `bca3429ed36dfb5b8e35fcfff0a0880c55d768a6`
- Client `9d090ed04957e4efc254f073cda97bc5510ca72b`
- Overlay `crates/tui/src/script_params.rs` sha256 `5181bf5a4bbc39b847cdd20f9a4f98865a3a8e9ae58112682528e286512bf9b1`
- Overlay `crates/tui/src/app.rs` sha256 `4e6bf126fd5d560c50f872d3e18232fd23b7a3b4ff15905536d9f3ba7eb5586d`
- `rustfmt --check crates/tui/src/script_params.rs crates/tui/src/app.rs`: passed
- `cargo test -p tui --lib`: 106 passed (8 new params editor tests + previous 98)
- `cargo clippy -p tui --lib --tests -- -D warnings`: 4 pre-existing
  `clippy::type_complexity` hits in frontend slot helpers / `frontend_fixture`
  (`bin.rs` 177, 188, 201, 1633). None in the params editor change.
- `cargo clippy -p tui --lib --tests -- -D warnings -A clippy::type_complexity`: passed

No LIVE run. Root owns Concord TUI editing proof after review and isolated build.

## Out of scope

New settings types, script semantics, bin.rs, panel, shared coercion/store,
catalog scripts, and actual headed PTY re-proof.
