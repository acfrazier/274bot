# Descriptive native loading progress

Operator request during this campaign: an external/background loading phase
should show a text loading bar and a description of what is happening, both to
help unfamiliar users and to evoke the game's classic loading presentation.
Implement this in the existing panel startup preparation flow. Root has told
the user the bar will reflect actual work and use plain stage descriptions.

Read AGENTS.md, docs/execution.md and the accepted startup implementation/report
814e5293 / 06d-panel-startup-preparation.md. Verify the campaign branch. Start
after the selected-data consumer's same-card review to avoid its shared files.
Own only `crates/host-play/src/{profile,lib}.rs`, a small progress module there
if needed, `crates/nav/src/manifest.rs` only if needed for hash progress,
`crates/panel/src/{app,session}.rs`, focused related tests, and
`docs/compat/06g-native-loading-progress.md`. Keep scenario and data-generator
workers' files untouched. Root owns native execution and internal capture proof.

Replace the generic Preparing server profile line with a readable classic-style
text bar and current stage. Fit the existing narrow panel rail and theme; avoid
new windows, assets, fonts, logos, dependencies or framework work.
The operator specifically selected the brighter orange used by existing panel
text: reuse `theme::ACCENT` (`#FFB000`, `[1.0, 176.0/255.0, 0.0, 1.0]`) for the
loading description and filled bar. Do not introduce another orange value.
Useful plain labels include Checking game files, Checking navigation files, Loading game
data, Preparing navigation, and Final checks. Do not show engine paths, Rust
types, thread details or hashes to end users as loading descriptions.

Progress must describe real completed work. File/byte progress within the named
stage is appropriate; say what a displayed percentage measures. Do not derive
percentages from elapsed time, fake a steadily advancing total, or show 100%
before that stage has finished. For a decode operation without a meaningful
fraction, retain the honest stage label and completed-step indication. A
stage-aware bar may restart for the next named operation. The long navigation
hash checks should visibly advance from actual processed bytes, rather than
leave the user with the same generic label for the entire wait.

Keep one bounded latest progress state for each preparation generation. The
worker may publish a small stage/counter value; the UI only reads it. Do not
queue per-byte/per-tick events, copy resources/snapshots, or block a worker
waiting for UI consumption. Reset/drop progress on error, generation changes,
close, and completion so stale jobs cannot repaint a newer run. Final Unlock
validation uses the same display. Preserve the existing private one-use
ValidatedTemplate handoff and UI ownership of vault, Play, slots and GPU.

Preserve all three resource-identity checks, their order, actual digest bytes,
missing/mismatch refusals, existing errors, and direct TUI/CLI APIs. Hashing
must stay on the preparation/validation worker in the panel. Prefer no-op
observer wrappers for existing non-panel callers. The existing `hash_file`
currently reads then hashes the file in one call: if adding chunk progress,
keep byte-for-byte SHA-256 results and resource-error behavior. Do not add an
extra pass, cache by path/mtime, skip a check, change the nav format, or turn
this into another memory/startup optimization campaign. No speed/size claim.

Verification should catch real regressions: digest correctness at a chunk
boundary; a changed/unreadable resource still fails; stale generation progress
is ignored; progress stays bounded and is cleared on completion/failure; a
partially processed stage cannot appear complete. Reuse existing startup and
profile tests. Inspect text/bar layout with a focused panel check; root will
read actual native internal captures while real full-size resources prepare.
Run affected formatting/strict checks proportionally. No tests that merely
repeat a list of labels.

Commit only owned source/report/check evidence. Hand this SAME card to profile
`reviewer` via kanban_request_review with exact commits/checks, then STOP. Do not
complete it yourself. Actual Grok 4.5 review is required before the integrated
milestone/native acceptance. No LIVE, fixtures, source-data edits, feature split,
release, backup jobs or unrelated UI redesign.
