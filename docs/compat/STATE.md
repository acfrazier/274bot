# Compatibility campaign state

Updated 2026-09-10. Implementation authorized by the operator.
Plan: `docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md`.
Campaign checkout: `/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision`.
Host branch: `codex/rs2b0t-multirevision`; base `b2bd5023489ab2e0b6ba690f228217e8984ac91b`.
Client branch: `codex/bothost-274-289`; published base `9b41e6e06b9fd42dc2247fc813a303c3fcb92941`.

## Current milestone

Step 1 is complete. Step 2 source reconciliation and macOS regression checks
are accepted after corrective Grok 4.5 and fresh whole-client Grok 4.6 review.
The complete campaign is not accepted: host revision/profile binding, live
gameplay, catalog options, frontend/fleet and Linux/Windows checks remain.
Historical platform failures have not been freshly reproduced.

Reviewed client candidate: `58120f28ee5208553ca07f41cb364f2cf98ea280`.
Client product: `d14755da758c64d971c1103b2d7703f6fd8379fb`; the final candidate
commit adds report/evidence only. The campaign host gitlink pins that candidate
locally. Its publication and fresh recursive fetch/build proof belong to step 9.
No public integration, release tag or package publication has occurred.

The client combines the published 274 improvements with 289 source
`c18f3a1148e9caee73426e162677328ca64d1a83`. Reconciliation preserves boxed
appearance storage, sharing, sprite reuse and Windows/home handling. One GPU
overlay comparison now preserves hint updates alongside scene-state-1 minimap
freeze and lazy uploads. Review exposed a stale input-frame limit during login;
root also found that incoming report handlers completed existing 274 stubs.
Both corrections have failing-before/passing-after regressions. Details:
`client-integration-milestone.md` and the client's
`docs/revision-289/bothost-integration-report.md`.

| Current check | Result |
|---|---|
| Published 274 client baseline | 774 passed, 0 failed, 0 ignored |
| Corrected client workspace | 992 passed, 0 failed, 2 ignored GPU tests |
| Both explicit GPU tests on the same product | 2 passed, 0 failed, actual GPU |
| Client format and strict all-target Clippy | Passed |
| Final API/host/host-play with `host-play/memory-profile` | 441 passed, 0 failed, 7 live tests ignored |
| Host format | Passed |

The host check exposed a pre-existing fixture-name truncation collision. The
scoped correction preserves changing serial digits and passed an extended
existing uniqueness test plus the affected suites. This is fixture maintenance;
the completed memory harvest has not been reopened. Raw failures and command
receipts remain in `evidence/client-milestone/` and the client evidence folders.
No local gameplay acceptance is claimed by these offline tests.

## Inputs and reviews

Read-only catalog archives at `100adccc` and `8e7d965b` remain under
`.superpowers/inputs/rs2b0t-<commit>`. The real loader exports freeze 45 enabled
cards per catalog and 180 source/revision rows, with 9 dim declarations,
2 import-blocked cards, 3 shape-omitted entries and native WalkTo recorded
separately. All required gameplay rows remain pending or explicitly blocked.
Initial ledger SHA-256:
`70eca8b8545471af704d2f90c5a743c4ff32aa36b176bfa0cee810818cfb919e`.
Fixture/cache/content identities and setup commands are in
`fixture-inputs.json` and `fixture-inputs.md`.

Architecture task `t_192621de` completed under actual Grok 4.6. Initial
implementation card `t_e8e16acd` completed its same-card Grok 4.5 review.
The required independent trial then compared actual Grok 4.6 and fresh Astra
against identical frozen inputs. Astra's reproduced login failure overrode the
earlier Grok approval; root added the report-control correction. The trial and
timing/tool limits are recorded in `reviews/client-trial-reconciliation.md`.

Corrective same-card `t_706ce8a5` was approved by actual Grok 4.5 / xai-oauth
(session `20260910_103233_3506ce`). Fresh whole-client follow-up `t_4bed943f`
was approved by actual Grok 4.6 / xai-oauth (session
`20260910_103533_55a2fa`) against candidate `58120f28`. Both runs completed;
no worker or review remains active for this milestone. Their exact model/run
receipts and report are in `reviews/`. The final whole-campaign Grok review is
still required after steps 3–8.

## Next implementation

Step 3: bind one immutable server profile through host, panel and TUI, including
revision, game/asset endpoints, RSA/CRC, shared cache/interface resources and
nav/content/catalog identity. Read `briefs/05-session-profile-preparation.md`:
the shared constructor's HTTP default and ambient login/transport/unpack
settings must be bound before actual spawn and reconnect. No step-3 product
implementation has begun. Do not expose 289 bot operation using current direct
274 host writers or fall back to 274 navigation.

Then follow steps 4–9 in order: host boundary and local live actions, world and
guardian binding, required Rust capabilities, every enabled card/option proof,
frontend/fleet/native preservation, and reviewed public integration. Full 377,
mixed fleets, tutorial/audio parity, named native rewrites, public live
qualification and performance-budget claims remain outside the agreed scope.
Alpha 2 / 0.1.7 release planning follows implementation acceptance.
