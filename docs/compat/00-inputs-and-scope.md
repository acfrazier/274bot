# Frozen integration inputs and scope

Step 1 began on 2026-09-10 after the operator requested implementation of
`docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md`.

The campaign worktree is
`/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision`, branch
`codex/rs2b0t-multirevision`, based on
`b2bd5023489ab2e0b6ba690f228217e8984ac91b`. `git ls-remote origin
refs/heads/main` still returns that base. The primary checkout's unrelated
untracked files were left in place. Its ignored state pointer names this
campaign and `docs/compat/STATE.md`.

The isolated client is `vendor/fr-client-rust`, branch
`codex/bothost-274-289`, based on published
`9b41e6e06b9fd42dc2247fc813a303c3fcb92941`. The bothost remote still publishes
that exact pin on `r274-bh-modular`. Reconciliation input
`c18f3a1148e9caee73426e162677328ca64d1a83` was fetched directly from
`/Users/acfrazier/experiments/FR-client-289`; its working tree has unrelated
untracked diagnostics that were not copied. The source's common ancestor with
the selected 274 changes is `4f2048ea10f75b3bb92ff45610b35ba7313b0308`.

A non-mutating `git merge-tree --write-tree` reports one textual conflict in
`crates/client/src/bot_target.rs`; `client/client.rs` and the GPU backend merge
textually. They still need semantic reconciliation and explicit preservation
tests. The accepted 274 delta includes animation-base sharing and borrowed
delays, boxed appearance packets, sprite recycling, overlay/minimap freeze
corrections, lazy modal chrome, Windows socket/home support, and opt-in resource
accounting. The 289 delta adds revision construction, protocol/decoder fixtures,
outbound mapping, standalone revision selection, tutorial/hint overlays and
bounded input diagnostics. A clean textual merge does not establish preservation.

The named 377 foundation commits were checked directly:
`a172f29b5d0ac512abd33345f37d8d3f2fd24032` (client) and
`a802ce1094f35d1257abc5f8b497f17675af3dde` (host). Their ownership and reusable
construction/guard seams are covered by `architecture-review.md`. Full 377
implementation and gameplay qualification remain outside this campaign.

Catalog inputs are read-only `git archive` exports from the unchanged local
RS2B0T repository, under `.superpowers/inputs/rs2b0t-<full-commit>`:

- `100adccc037d9f6898080e1cad58fcfc43364775`, the clean source checkout HEAD.
- `8e7d965be2071d6ec65c3265e12af797082d720a`, the separately pinned version
  with 289 BankFletcher and Superheater behavior.

The real loader inventory, frozen enabled set, source hashes, options,
capabilities and proof requirements are delivered in `support-matrix.json` and
`catalog-inputs.md`. Dim and import-blocked entries remain explicit. Gameplay
proof status starts pending; no supported option may silently disappear from
the ledger to make it pass.

Fixture identities and exact setup commands are in `fixture-inputs.json` and
`fixture-inputs.md`. Stable discovery IDs are `local-274` and `local-289`;
each proof also records their concrete source/cache/nav hashes at run time.

Hermes step-1 cards are `t_c3fae2e9` (Sol catalog inventory) and `t_192621de`
(Grok 4.6 architecture review). Profile defaults were checked before dispatch.
Actual running model/provider receipts are in
`evidence/input-preflight/worker-models.json`. Review verdicts and the step gate
are recorded in `STATE.md` when complete. Source inventory and architecture
approval do not satisfy client, host, catalog or frontend live acceptance.

The implementation target remains alpha 2 / `0.1.7`: one immutable server and
revision per process, both frontends, qualified local 274 and 289 worlds, and
all frozen enabled cards and supported branches. Release planning follows
acceptance. Packaging, tagging, publication and announcements remain separate.
