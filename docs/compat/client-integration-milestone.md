# Initial client integration milestone

2026-09-10. Plan steps 1 and 2 are under implementation. Catalog/input freeze
is complete; the reconciled client is committed and awaiting required reviews.
No host revision-profile or live gameplay acceptance is claimed.

Client product commit: `cf386ae5c82071ef661f4a1eb4d38e22b41718d3`.
Client report/evidence candidate: `8b1a80918f87089e4eb339f1d0e216c1b163348d`.
The published client base remains `9b41e6e06b9fd42dc2247fc813a303c3fcb92941`.
See `vendor/fr-client-rust/docs/revision-289/bothost-integration-report.md` for
the preservation mapping, source provenance and retained failures.

## Current verification

| Check | Result |
|---|---|
| Published client baseline | 774 passed, 0 failed, 0 ignored |
| Reconciled client workspace | 990 passed, 0 failed, 2 ignored GPU tests |
| Explicit client GPU tests | Both passed with real GPU |
| Client format / strict all-target Clippy | Passed |
| API, host, host-play with `host-play/memory-profile` after fixture correction | 441 passed, 0 failed, 7 live tests ignored |
| Host format check | Passed |

The host command was `cargo test --locked -p api -p host -p host-play --features
host-play/memory-profile`. Its raw logs, exact commands and exit receipts are
in `evidence/client-milestone/`. Root independently recomputed counts. The
197-file client input manifest matches the committed product. Host live tests
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

## Review and next work

Client implementation card `t_e8e16acd` is in the same-card reviewer flow.
Root finished its saved report/commit after reclaiming the worker during an
unfinished additional delegate review. That extra pass is not review evidence.
After actual Grok 4.5 acceptance, freeze the exact client/host/evidence manifest
and run the required Grok 4.6 whole-client review plus independent Astra trial.
Include the small host fixture correction and its evidence in both contracts.

Next implementation is the immutable session profile. Preparation is in
`briefs/05-session-profile-preparation.md`; HTTP/RSA/transport/unpack/nav inputs
need binding alongside revision. Direct host sends, 289 nav/content, enabled
card/options proofs, frontend/fleet/native checks and final whole-campaign
review remain pending. No release or public integration has occurred.
