# Make existing catalog parameter types editable in the TUI

Use grok46 defaults. Scope is the authorized frontend supported-options gate,
not new settings types or script semantics. Root CAF6b809 Concord289 actual
PTY proof exposes the issue: Params opens for selected Alcher but Items to alch
and Alchs per trip cannot be edited. script_params.rs on_key supports only bool
and optioned string. Existing native panel supports number, text, tile, list,
string[] and optioned string[] through shared setting coercion/store helpers.
The TUI overlay also leaves map/status glyphs visible beneath the form.
Raw interpreted form and ANSI: evidence/platform-isolated/concord-tui-caf6b809/
r289-controls/controls/params-open.*. Root owns LIVE and ledger claims.

Own crates/tui/src/script_params.rs and minimal params-specific integration in
crates/tui/src/app.rs, related focused tests, docs/compat/06m-tui-parameter-editors.md
and unique evidence/tui-parameter-editors/. Do not edit bin.rs, panel, script
runtime/shared schema/coercion/store, client, fixtures, catalog scripts or ledgers.
Preserve CAF deferred PASS output and all existing controls/profile behavior.

Implement usable keyboard editing for the existing types supported by the native
panel using shared coerce_setting_value, format_setting_value and resolved
options. Keep bool toggle, optioned string selection, visible-row rules/grouping,
loadout choices and per-source/per-card persistence. Add explicit entry editing
with Enter to save and Escape to cancel; make numeric/free-text/tile/list entries
and string[] choices accessible. Do not invent a JSON settings format or foreign
settings editor. If shared coercion permits an invalid input, do not silently
replace a previous setting with an accidental value; show a bounded local error
where appropriate without changing the shared accepted settings contract.

Large schemas/option lists must scroll or page and keep the cursor visible at
ordinary terminal sizes. Clear the overlay area before drawing so labels and
values are readable. Show concise keyboard hints for current mode; preserve
normal TUI keys when the form is closed and consume edit keys while it is open.
Edits persist to the existing store and flow through the normal Start merge.
Save errors must be visible and must not claim persistence. No host actions
from the editor, no direct ScriptRunner API, no unrelated UI redesign.

Tests must cover meaningful editing/persistence/cancel and selection/scroll cases,
including Alcher string[] and numeric fields, tile/list coercion and isolation
between cards. Use existing TestBackend for clipping/background/long schema
regressions where helpful. Exact Git-blob source export and client bytes, new
empty target, affected TUI tests and Clippy. Four preexisting type_complexity
errors were retained in CAF review; report them honestly rather than expanding
scope or calling an allowed-lint check strict-clean. Scoped commit, SAME-card
reviewer handoff, then STOP. Root will conduct native PTY editing, reopen,
actual Start/settings effect and lifecycle proof after review and isolated build.
