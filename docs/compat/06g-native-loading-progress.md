# Descriptive native loading progress

Task: `t_42263bfd`
Branch: `codex/rs2b0t-multirevision`
Implementation: `79f86973`, lint-only `0bfb28ad`, and monotonic-stage follow-up `d321908c`
Selected-data predecessor review: `8c6689c1`
Related first-run picker source/proof: `1a2f2dcf` / `c7d7a8ff`

## Result

The panel's generic preparation line is replaced by a 20-cell classic text bar, a plain current-stage description, an exact percentage, and a caption stating whether the percentage measures files, bytes, or completed steps. The description and filled `#` cells reuse `theme::ACCENT` (`#FFB000`); the unfilled cells and measurement caption remain dim. The fixed-width ASCII bar fits the existing narrow panel rail without assets, fonts, windows, dependencies, or theme additions.

Displayed stages come from actual worker operations: selecting the server profile; checking the eight cache archives; reading those archives for CRCs; loading and decoding client/interface plus selected-revision metadata; hashing navigation pack/flags bytes; decoding the selected navigation world; and the final resource validation immediately before Unlock/live Play construction. Archive reads and game-data decode use distinct stages, so finishing the CRC pass cannot make the later load stage regress from 100%. Final validation uses the same display with a `Final checks:` prefix. Decode stages advance only when a named decode step finishes. A stage reaches 100% only after its corresponding file hashes, archive reads, digest finalization, or decode step has completed.

## Progress ownership and lifecycle

`host-play::progress` contains only small copyable stage/counter values and a callback observer. Existing direct host-play/TUI/CLI APIs remain wrappers with a no-op observer. The panel creates one `Arc<Mutex<Option<ProfileProgress>>>` per preparation or validation generation. Workers publish with `try_lock`, so they never queue updates or wait for UI consumption; the UI only reads the latest value.

The progress state lives on the existing worker job. Taking a successful or failed completion drops the UI's handle, generation mismatch filtering prevents an old worker from repainting a newer run, and closing the panel drops the receiver/state without joining the worker. Errors continue to replace progress with the existing error banner. Vault, Play, slots, scenario, picker globals, JS stores, and GPU installation remain on the UI thread. The private one-use `ValidatedTemplate` contract is unchanged.

## Resource validation

The three existing cache/navigation identity passes and their order remain intact. `nav::manifest::hash_file` now delegates to a bounded 1 MiB streaming SHA-256 implementation; its no-observer behavior and error prefix are preserved. The progress-aware path emits actual bytes processed and publishes `(total, total)` only after digest finalization. It does not add a resource pass, use path/mtime caching, change the nav format, or alter mismatch/missing-file refusal.

## Verification

TDD red checks first failed on the absent streaming hash/progress APIs and absent bounded panel progress helpers. Focused tests then covered:

- SHA-256 equality across an exact chunk boundary, with no pre-final completion update;
- unchanged unreadable-resource error classification;
- completed cache-file counters and final-validation completion;
- latest-only, generation-scoped panel state;
- clearing on successful final validation and preparation failure;
- no partial-work rounding to 100%, and a 20-cell rail layout.

Exact committed-source export: `.superpowers/task-exports/t_42263bfd-79f86973` with client `56d80272bcbda3eb1e22db096c1c5e21d3497de4`:

- `cargo test -p nav`: 256 library, 7 nav-pack, and 3 resource-manifest tests passed.
- `cargo test -p host-play --lib --features memory-profile`: 156 passed.
- `cargo test -p host-play --test session_profile --features memory-profile`: 12 passed.
- `cargo test -p panel --lib`: 392 passed.

Exact lint-fix export: `.superpowers/task-exports/t_42263bfd-0bfb28ad`, same client commit:

- selected-file `rustfmt --check` passed;
- strict all-target/all-feature Clippy passed for `nav` and `host-play`;
- strict all-target/all-feature panel Clippy passed with only the four unchanged pre-existing `clippy::type_complexity` sites explicitly allowed, matching the accepted startup baseline.

`git diff --check` passed before each task commit. No LIVE run, source-data edit, native loading capture, performance claim, or release action was performed. Root still owns actual Mac/Windows loading captures against full-size resources. The separate first-run picker proof at `c7d7a8ff` records three internal native Mac captures and an exit-0 check of the `1a2f2dcf` UI behavior; it did not unlock a vault or start a script.
