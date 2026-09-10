# Compatibility campaign state

Updated 2026-09-10. Implementation authorized by the operator.
Plan: `docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md`.
Host branch: `codex/rs2b0t-multirevision`; base `b2bd5023489ab2e0b6ba690f228217e8984ac91b`.
Client branch: `codex/bothost-274-289`; base `9b41e6e06b9fd42dc2247fc813a303c3fcb92941`.
Both published refs were rechecked and match these bases.

Step 1 is active: frozen catalog inventory, fixture/source identities, and
274/289/377 architecture and preservation audit. Source catalogs are immutable
archives under `.superpowers/inputs/rs2b0t-<commit>` at `100adccc` and `8e7d965b`.
289 source commit `c18f3a1148e9caee73426e162677328ca64d1a83` has been fetched into
the isolated client; integration has not started.

All configured Hermes profile models/providers match AGENTS.md, and sol/grok46
use high reasoning. No required review is yet complete. The Hermes CLI reports
an update/restart warning; verify real worker runs and actual review models.

Next: finish step 1 ledger and independent architecture review, then reconcile
the client. Full 377 qualification, native rewrites and release publication
remain outside this campaign's implementation scope.
