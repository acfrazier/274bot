# Compatibility campaign state

Updated 2026-09-10. Implementation authorized by the operator.
Plan: `docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md`.
Campaign checkout: `/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision`.
Host branch: `codex/rs2b0t-multirevision`; base `b2bd5023489ab2e0b6ba690f228217e8984ac91b`.
Client branch: `codex/bothost-274-289`; published base `9b41e6e06b9fd42dc2247fc813a303c3fcb92941`.

## Current milestone

Steps 1–3 are accepted on macOS within their stated scope. Step 3 binds the
immutable profile through host, panel and TUI, with completed actual Grok 4.5
task reviews and Grok 4.6 integration approval. Native panel selection/binding
and local 274 scene-2 login passed; the real macOS TUI PTY also reached scene 2
and exited 0. Both final 274/289 asset constructors initialized successfully.
Host product: `41d896b3007e476c76a9c01352b64d5afd5ed901`; current client pin:
`2be1697060e4d2b8b709ad4d5e54d12513b38333` (product `c8e61557`).

The complete campaign is not accepted: step-4 host action/snapshot qualification,
289 navigation, catalog options, frontend/fleet and Linux/Windows checks remain.
Production 289 bot operations still refuse before mutations. Historical platform
failures have not been freshly reproduced. No workers remain active for step 3.
Read `01-session-profile.md` and its native evidence receipt for acceptance.

## Accepted step-2 snapshot

Step-2 reviewed client candidate: `58120f28ee5208553ca07f41cb364f2cf98ea280`.
Client product: `d14755da758c64d971c1103b2d7703f6fd8379fb`; the final candidate
commit adds report/evidence only. The step-2 host gitlink pinned that candidate
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

## Accepted step-3 milestone

Step 3 was authorized by the operator's 2026-09-10 follow-up: bind one immutable server profile through host, panel and TUI, including
revision, game/asset endpoints, RSA/CRC, shared cache/interface resources and
nav/content/catalog identity. Read `briefs/05-session-profile-preparation.md`:
the shared constructor's HTTP default and ambient login/transport/unpack
settings must be bound before actual spawn and reconnect. Architecture card
`t_bfd3339c` approved the concrete design with binding constraints under actual
Grok 4.6. Its completed review is in `reviews/session-profile-design-grok46.md`.
Client card `t_1b93f768` and root host backend card `t_30007871` have completed
actual Grok 4.5 reviews with approval. The reviewed client candidate is
`2be1697060e4d2b8b709ad4d5e54d12513b38333` (product `c8e61557`), pinned locally
for step-3 integration. Full client workspace: 1,001 passed, no failures, two
ignored GPU tests; both explicit GPU tests passed separately. Affected host
backend: 450 passed, no failures, seven live tests ignored.
Frontend card `t_879d9607` completed its actual Grok 4.5 review with approval
on source `7aaa8c39`. Root follow-up `a02c9dd5` binds the remaining fixture-button
presentation call. The 384 panel and 89 TUI tests passed; current integrated
binaries build successfully. Nine real binary negatives created no vaults.
Current implementation, asset checks and review receipts are recorded in
`01-session-profile.md`. Integration card `t_80dd7fd0` completed approval under
actual Grok 4.6 / xai-oauth, session `20260910_124351_55201a`, at `a02c9dd5`.
Root presentation correction `41d896b3` fixed two clipped profile labels and
passed format/build/diff plus native visual checks. Ten original panel captures
cover default/explicit selection, 289 refusal before vault writes, restart-required
binding, local 274 scene 2 and title-screen return. TUI reached scene 2 in an
actual macOS PTY and exited 0. Its first literal-text reader failure is retained;
CUA denied iTerm2, so no native terminal-window visual proof is claimed.
Both final asset constructors passed with shared resources and expected nav
availability. Complete receipts: `evidence/session-profile/native-macos/proof.json`.
Preferences were restored exactly and proof apps/owned 289 engine stopped;
existing 274 engine PID1852 remained listening. No memory/performance claim.

## Next implementation

Operator correction, 2026-09-10: rs2b2t now serves revision 289. Public 289
requires the known `w1.rs2b2t.com:443` game/asset pairing; public 274 is
unavailable. This supersedes the public-profile assumption in the historical
step-3 design and review receipts. Card `t_96dc78da` is correcting resolution,
resource defaults, help and focused tests. Source `04d3e02f` passed same-card
Grok 4.5 review (run 1099, actual session `20260910_133757_d52004`), including
an independent rerun of ten profile tests, frontend/CLI tests, check and format.
The public correction is accepted; no public live login was performed.
The accepted local 274/289 macOS observations remain valid.

Do not expose 289 bot operation using current direct 274 host writers or fall
back to 274 navigation. Step 4 qualifies the complete host action/snapshot
boundary and the controlled local action sequence on both revisions. Design
card `t_d6761a15` approved the design under actual Grok 4.6 (run 1096,
session `20260910_132555_633342`). Root corrected its numeric blacklist mistake:
legality must follow named packets, since ids 51/224 have valid R289 meanings.
Implementation cards `t_b22abf57` (outbound) and `t_0be9adbf` (snapshots/reset)
are running under configured Sol, with same-card reviewer handoffs required.
Root owns the narrow local proof while ordinary 289 slots/guardians remain
gated for step 5. Briefs 12/13 give disjoint implementation ownership.

Operator platform clarification: validate step 3 on macOS first. Linux and
Windows do not have a 289 engine configured; prepare those isolated engines
before their later platform checks. No Linux/Windows qualification is implied
by the macOS milestone.

Use the established memory-campaign machines: Windows `austen@10.0.0.205`,
Hyper-V Linux builder through SSH alias `274bot-builder`, and Concord through
SSH alias `concord`. Concord is TUI-only for this campaign; compile Linux on
the Hyper-V builder because the VPS has about 2 GB RAM. Concord is currently
reachable and its isolated 274 service uses
`/home/acfrazier/274bot-campaign/server-4c95f87`. Windows SSH initially timed
out; the operator corrected the firewall rule tied to the Mac's previous IP.
Windows is now reachable (`DESKTOP-SL99R6C`). Root started the existing powered-off
Hyper-V VM; `274bot-builder` now responds with Rust 1.98.0, 8 GB RAM and 18 GB
free disk. No SSH trust/key configuration was changed.

Separate 289 fixtures are being prepared under each machine's existing campaign
root at `multirevision-20260910/server-289`. Bundle SHA-256
`d8361199f7f3af851c175e030b1e787cb62137de1ce62b6701bcde07cd75f2e8`
contains the pinned engine/cache/scripts and content maps; no account/key/database
material was copied. Concord and Windows extracted and verified all 1,212 files,
installed locked dependencies and generated fresh local RSA keys and SQLite
schemas. The builder is staging the same bundle. These are setup receipts;
engine readiness and platform client qualification are still pending. Existing
274 fixtures remain unchanged. Concord remains TUI-only with Linux compilation
on the builder.

Then follow steps 4–9 in order: host boundary and local live actions, world and
guardian binding, required Rust capabilities, every enabled card/option proof,
frontend/fleet/native preservation, and reviewed public integration. Full 377,
mixed fleets, tutorial/audio parity, named native rewrites, public live
qualification and performance-budget claims remain outside the agreed scope.
Alpha 2 / 0.1.7 release planning follows implementation acceptance.
