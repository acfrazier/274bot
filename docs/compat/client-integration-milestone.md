# Initial client integration milestone

2026-09-10. Catalog/input freeze is complete. The reconciled client source and
macOS regression milestone is accepted after corrective Grok 4.5 and fresh
whole-client Grok 4.6 review. Host revision-profile, live gameplay and fresh
Linux/Windows acceptance remain pending.

Client product commit: `d14755da758c64d971c1103b2d7703f6fd8379fb`.
Client report/evidence candidate: `58120f28ee5208553ca07f41cb364f2cf98ea280`.
The published client base remains `9b41e6e06b9fd42dc2247fc813a303c3fcb92941`.
See `vendor/fr-client-rust/docs/revision-289/bothost-integration-report.md` for
the preservation mapping, source provenance and retained failures.

## Current verification

| Check | Result |
|---|---|
| Published client baseline | 774 passed, 0 failed, 0 ignored |
| Corrected client workspace | 992 passed, 0 failed, 2 ignored GPU tests |
| Explicit client GPU tests | Both passed with real GPU |
| Client format / strict all-target Clippy | Passed |
| API, host, host-play with `host-play/memory-profile` after fixture correction | 441 passed, 0 failed, 7 live tests ignored |
| Host format check | Passed |

The host command was `cargo test --locked -p api -p host -p host-play --features
host-play/memory-profile`. Its raw logs, exact commands and exit receipts are
in `evidence/client-milestone/host-crates-final.json` and its raw log. Root
independently recomputed counts. The original 197-file manifest describes the
initial candidate; `corrected-review-inputs.json` and the client's correction
receipts name the final product and evidence. Host live tests
remain pending for the later profile/action stages; default-suite ignores are
not live acceptance.

## Existing fixture-name collision exposed by the host check

The first host check failed with `vault already exists` in
`prepare_reads_focused_one_policy_and_pins_focus`; four following tests failed
because its panic poisoned the environment lock. API and host tests passed.
The underlying `mint_live_names` implementation was unchanged from host base
`b2bd5023`: it concatenated a PID and serial, then kept only the first token
characters. This discarded the changing serial for larger fleets or after
serial growth, reusing account names and their temporary-vault path.

Root extended the existing public-helper test to mint 24 rounds of 1, 2, 32
and 128 names and check invocation uniqueness and the engine's name limit. It
failed before the correction (`live-name-red.log`, one failed test, exit 101).
The helper now uses a randomly seeded process counter and fixed-width low
base-36 digits, retaining the changing part within the name budget. Account
prefix, slot suffix and the 12-character limit remain. This removes deterministic
within-process truncation collisions; later live preparation must still verify
its actual initial account state.

The affected suites then passed once with the correction. This is scoped
fixture maintenance required for reliable later live proofs, not a reopening
of the completed memory-optimization campaign. Failed artifacts remain intact;
no timeout, success predicate, vault overwrite, or live account was changed.

## Review, corrections and next work

Client implementation card `t_e8e16acd` completed its same-card Grok 4.5 review.
Root finished its saved report/commit after reclaiming the worker during an
unfinished additional delegate review; that extra pass was not review evidence.
The independent whole-client trial then used the same frozen contract for
actual Grok 4.6 and fresh Astra. Astra reproduced a stale game-frame limit
causing the next login seed read to panic. Root reproduced that through a real
short socket frame followed by cold login, reconnect or adoption on both
revisions, then cleared the bound at the login-seed transition.

Root also found incoming 289 report handlers were active on 274, completing
previously idle controls. Revision guards restore those stubs, and the new
test checks all twelve report reasons and mute while preserving 289 ordered
frames. Both tests fail before their corrections and pass afterward. The
full corrected client suite and both actual GPU tests passed on product
`d14755da`; the final host check above then passed against candidate `58120f28`.

Corrective same-card `t_706ce8a5` was approved by actual Grok 4.5; fresh
whole-client follow-up `t_4bed943f` was approved by actual Grok 4.6 with no
remaining findings. Exact sessions, frozen inputs and root reconciliation are
in `reviews/client-trial-reconciliation.md` and
`reviews/client-grok46-corrected.md`. Earlier approvals describe their older
inputs and do not substitute for these corrected reviews.

The host campaign pins the reviewed client locally. The candidate is not yet
published to the bothost remote, and fresh recursive fetch/build verification
remains a step-9 gate.

Next implementation is the immutable session profile. Preparation is in
`briefs/05-session-profile-preparation.md`; HTTP/RSA/transport/unpack/nav inputs
need binding alongside revision. Direct host sends, 289 nav/content, enabled
card/options proofs, frontend/fleet/native checks and final whole-campaign
review remain pending. No release or public integration has occurred.
