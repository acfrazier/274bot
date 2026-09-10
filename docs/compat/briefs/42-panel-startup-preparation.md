# Keep native startup responsive during profile validation

Use configured sol defaults after bank routing t_90b60f12 completes review,
when host-play/src/lib.rs is released. Campaign branch codex/rs2b0t-multirevision.
Read applicable AGENTS.md, docs/execution.md, plan step 8 and the accepted
06c-panel-startup-design.md (actual Grok 4.6 run 1142). Root's repeated native
runs confirm the same UI-thread hashing path; 06b retains the sample.

Implement the bounded design: sequential worker preparation and final resource
validation, UI-thread installation and Play/slot/scenario assembly. Keep Session,
IsolatedEnv/JS stores, vault operations, catalog transpile, picker globals and
GPU/slot ownership on their existing threads. Preserve all three resource
validation passes; no pathname/mtime shortcut, changed-resource bypass, new
runtime, streaming-hash campaign or login/script timeout changes. Both normal
Unlock and deferred BOT_VAULT_PASS/live boot must stay responsive. An off-thread
bind/load with the third validation still on the UI thread is incomplete.

Root refines one design seam: do not expose a bare unchecked public
run_prepared_template that any caller can invoke without validation. Prefer a
small consuming preparation ticket with private fields, produced only after
validate_resources on the same Arc<SharedClientTemplate>, then consumed by the
UI-side Play constructor. This is a one-use validation handoff, not a second
profile/identity model. Existing run_with_template stays the checked convenience
entry and TUI/CLI behavior stays unchanged. The handoff has the same bounded
hash-to-construction race class as the existing synchronous code; do not add a
fourth UI-thread hash or claim it eliminates filesystem TOCTOU races.

Before bind, capture exact options/environment/revision and reject stale worker
completions after a user selection/config change. Once installed, the immutable
process profile still requires restart to change. No half-bound profile, vault
creation, Play or slots on failure. Errors remain the actual current messages.
Closing drops the receiver and never joins the hasher on the event-loop thread.
Keep painting a plain preparation status with the existing redraw loop. Do not
add implementation details to user flows. Protect passphrases and preserve
existing error/log behavior; no passphrase in worker logs or proof artifacts.

Allowed: panel app.rs/session.rs, narrow host-play profile.rs/template builder
seams in lib.rs, and focused existing tests. No banking/script dispatch, client,
nav format/hash_file, TUI event loop, external fixtures, operator settings,
source ledger, gitlink or repository integration. Preserve concurrent Alcher
scenario/harness work and use exact committed exports for checks. Coordinate
any unexpected shared-file overlap with root before staging it.

Extend meaningful profile and lifecycle regressions: cached template plus changed
pack/flags/cache refuses at the final validation; checked convenience entry still
refuses changes; only successful validation can produce the handoff ticket;
stale generation drops; failure leaves no Play/slot/vault; successful normal
Unlock and live continuation avoid rebinding; GPU-first ordering remains. Use
small fixtures, not GB-scale unit inputs. Do not weaken existing tests or add
mirrored tests for mechanical wrappers. Run affected checks/Clippy once.

Deliver 06d-panel-startup-preparation.md and evidence/panel-startup-preparation/.
Record the exact source and limitations. Root owns the Mac/Windows native
responsiveness and mismatch proof on disposable copied resources; never mutate
the operator's or campaign's real nav/cache files as a test. Commit scoped source
in a new commit, request same-card profile reviewer, then stop. No agents, LIVE,
commit amend, merge, remote, gitlink or release work.
