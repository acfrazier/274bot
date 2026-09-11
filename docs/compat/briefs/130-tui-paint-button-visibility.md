# Brief130: keep actual script paint buttons visible and clickable

Read AGENTS.md and docs/execution.md once. Campaign codex/rs2b0t-multirevision.
Independent bounded TUI follow-up to reviewed113/04af. Own only
crates/tui/src/{app,chat}.rs and uniquely scoped tests/report04ag/evidence
under native-paint-buttons/tui-visibility. Do not edit runtime, client, scenario,
foreign source, other frontends, STATE/matrix/ledger or run LIVE. Root owns LIVE.

Root private exact cd49/aef UI export `.superpowers/review-exports/paint-ui-cd49e147`
starts unchanged frozen NatureCrafter in Runner mode through ordinary TUI.
Real140x40 PTY capture `evidence/paint-input-ui-cd49/tui289-input/before-button.txt`
has script pane rows17..22, title +3 status rows. Go bank is clipped. Source
app.rs uses Length(6); chat.rs appends all paint text then a blank and buttons.
Digit1 does reach the real callback, but an invisible control is not qualified.
Native panel Go bank/Resume already works in a macOS bundle with actual CUA.

Second source defect: ChatPane::on_click uses row.saturating_sub(buttons_start)
without testing row >= buttons_start, so clicking above a button can dispatch
index0. Wrapped body lines also invalidate the assumed unwrapped hit offset.

Implement a bounded layout that reserves visible space for advertised paint
buttons at ordinary terminal sizes, with exact hit rectangles derived from the
same visible layout used for rendering. Clicking title, text, spacer, borders,
or clipped/nonvisible rows must not dispatch a button. Keep focused buttons
reachable when space is constrained; no broad terminal UI redesign. Preserve
modal precedence, game chat toggle, Pause/Stop keys and script generations.
Use native TUI layout only, no changes to JS content or click identities.

Meaningful tests must render a realistic NatureCrafter title + rows + button in
140x40 and a compact terminal, assert label visibility, click actual rendered
row, reject body/spacer/border clicks, and preserve modal priority. Existing
paint tests plus affected TUI strict Clippy. Exact committed export + owned
file overlay in an exclusive task cache; keep source/hash evidence and never
borrow another active target. Report focused; no extra full workspace campaign.
Same-card reviewer handoff then stop. Root will rerun the real PTY proof.
