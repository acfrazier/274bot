# Compatibility campaign state

Updated 2026-09-10. Implementation authorized by the operator.
Plan: `docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md`.
Host branch: `codex/rs2b0t-multirevision`; base `b2bd5023489ab2e0b6ba690f228217e8984ac91b`.
Client branch: `codex/bothost-274-289`; base `9b41e6e06b9fd42dc2247fc813a303c3fcb92941`.
Both published refs were rechecked and match these bases.

Step 1 is complete: frozen catalog inventory, fixture/source identities, and
274/289/377 architecture and preservation audit. Source catalogs are immutable
archives under `.superpowers/inputs/rs2b0t-<commit>` at `100adccc` and `8e7d965b`.
289 source commit `c18f3a1148e9caee73426e162677328ca64d1a83` is included in the
intermediate client merge described below; that candidate is not yet accepted.

All configured Hermes profile models/providers match AGENTS.md, and sol/grok46
use high reasoning. Architecture review t_192621de completed under actual grok-4.6 / xai-oauth
(session 20260910_090652_9f1de4): proceed with GPU freeze and host-send constraints. The Hermes CLI reports
an update/restart warning; verify real worker runs and actual review models.

The current published 274 client baseline passed cargo test --locked --workspace:
774 passed, zero failed/ignored, including GPU freeze/modal regressions. Receipt:
docs/compat/evidence/client-baseline-274/. Compiler cache: target/client.

Catalog task t_c3fae2e9 was reclaimed after its worker stalled in context
compression; parent finished the saved support tool, fixed one closure capture,
and validated both actual exports plus all 180 source/revision rows. The fixed
set is 45 enabled cards per catalog, 9 dim declarations, 2 import-blocked cards,
3 shape-omitted entries, and native WalkTo. All gameplay proof remains pending.
Initial support-matrix SHA-256: 70eca8b8545471af704d2f90c5a743c4ff32aa36b176bfa0cee810818cfb919e.

Next: finish client reconciliation, regression checks and the required reviews,
then bind the immutable server profile through the host and frontends. Full 377 qualification, native rewrites and release publication
remain outside this campaign's implementation scope.

Step 2 active: client merge 4098a50fb4803f5d6ec9131af38d18e0ba6a395f prepared inline by root.
Card t_e8e16acd is assigned to sol for semantic reconciliation, tests,
and same-card reviewer handoff. This intermediate client is unqualified;
host gitlink, public refs and live acceptance remain unchanged.

The raw merge required boxed appearance assignments in the 289 actor decoder.
A combined production-NPC-hint / held-minimap GPU test now reproduces the
expected overlay invalidation defect before the correction (exit 101). Keep
that failure receipt; it is an offline GPU regression, not a live world run.
