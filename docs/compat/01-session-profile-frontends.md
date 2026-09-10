# Session profile frontend wiring

Status: frontend commit `7aaa8c39` approved by completed actual Grok 4.5 review
(card `t_879d9607`). Root integration at `a02c9dd5` is approved by actual Grok 4.6;
macOS native proof passed after presentation-only correction `41d896b3`.
See `01-session-profile.md` for the accepted milestone and limits.

## What changed

- `panel-play`, `tui-play`, and `host-play` now parse the shared profile surface (`--profile`, `--revision`, `--prod`, endpoint, asset, cache, nav, content, vault, catalog, and manifest overrides) before frontend-specific flags.
- All three production entry paths resolve and bind one immutable `ServerProfile`, load one `SharedClientTemplate`, and reuse it for initial clients and later slot spawns.
- Revision 289 remains constructible offline but is refused at the host boundary before memory preparation, temporary-vault creation, vault open/create, or account upsert.
- Target-sensitive vault paths, live passphrases, generated passwords, and catalog roots come from the selected/bound profile rather than mutable ambient process state.
- Panel exposes the effective server and revision before session binding. The single persisted revision field migrates to 274. Explicit CLI/environment choices retain precedence. Once bound, revision changes and rebinding return restart-required errors, including after vault lock.
- Panel captures profile environment once, resolves explicit pre-bind catalogs instead of ambient `RS2B0T`, binds the selected navflags path, and records full catalog identity when cards load. Binding refuses root or source-hash changes, including same-root edits after warmup, without clearing custom file cards.
- Panel stress scatter uses the selected shared template's world-specific seed. TUI startup reuses the selected template.
- Both frontends exercise their real parser-to-profile-to-template-to-client path against the synthetic 274 and 289 manifests, each with a private copy of all eight JAGs and explicit public RSA values. Both assert the default game host is `127.0.0.1`.
- `host-play` no longer mutates global target state while parsing and now loads checked assets before touching its vault.

## Verification

- `cargo fmt --all -- --check` — pass.
- `cargo check -p host-play -p panel -p tui --all-targets` — pass.
- `cargo clippy -p panel -p tui --all-targets -- -D warnings` — pass.
- `cargo clippy -p host-play --bin host-play -- -D warnings` — pass.
- `cargo test -p host-play` — pass; production live tests remain intentionally ignored unless `LIVE=1`.
- `cargo test -p panel --features memory-profile` — 384 passed, 0 failed.
- `cargo test -p tui --features memory-profile` — 89 passed, 0 failed.
- `git diff --check` over source/docs excluding raw Cargo logs — pass. The three
  Cargo test logs preserve Cargo's exact trailing blank line at EOF.

Raw logs: `docs/compat/evidence/session-profile/frontends/`.

## Integrated proof

Root completed native macOS panel selection, binding, refusal and local 274
scene-2 proof with a synthetic vault; the real macOS TUI PTY also reached scene
2 and exited 0. The panel clipping found visually was corrected in `41d896b3`.
Receipts and all limits are in `evidence/session-profile/native-macos/proof.json`.
Revision 289 asset initialization passed; its bot operation remains gated for
step 4. Linux/Windows and terminal-window visual proof remain unqualified.

## Root follow-up to the review

The reviewer noted that the debug heading still called the legacy ambient-target
button helper. The local/public send gate was already bound, but an explicit
local profile could display the wrong buttons when the ambient target differed.
Root routed that one presentation call through the bound session target and
checked the existing local and public button tests. This small follow-up is
included in the fresh Grok 4.6 integration review. The completed Grok 4.5 receipt
is `reviews/session-profile-frontends-grok45.json`; it covers `7aaa8c39`.
