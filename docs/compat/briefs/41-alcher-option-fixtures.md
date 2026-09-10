# Alcher custom, ordered targets and large noted withdrawals

Use configured luna defaults for a bounded harness extension on branch
codex/rs2b0t-multirevision. Read AGENTS.md, docs/execution.md and the full plan
step 7. Root runs LIVE; this task prepares reviewable fixtures and predicates.
The native 1947741f/client 56d8027/catalog 100adccc Alcher runs now consume
30 chainbodies across 27+3 withdrawals, deposit 900000 coins and Stop cleanly
on both revisions. Existing independent core proof covers 65 Magic XP,
stock acquisition and consumption. Other settings still need concrete proof.

Extend the shared scenario and catalog_boundary_live harness with three named
Alcher variants, using the real frozen Alcher and settings schema:
- custom item selection with batch size 1 (a known notatable item, two stock),
  requiring exact requested item, withdrawal, rune/item consumption and XP/coins;
- two distinct selected items with deliberately bounded stock (one or two each),
  requiring the higher-value target exhausted first, then actual acquisition and
  a cast of the second item. Verify actual values/names from selected content;
- batch size 1000 with sufficient noted stock and Nature runes seeded before
  Start, requiring an observed 1000-note and 1000-rune stack followed by a cast.
  Do not wait for 1000 casts or extend an existing deadline. The ordinary
  restock transition already has its separate 27+3 native proof.

All fixture changes occur before Start and baselines follow confirmed seed
acknowledgements. Reuse scenario seed helpers and actual profile/world bindings.
Give variants stable scenario names usable through both the existing native
script_<scenario> path and the headless harness; map all to the same Alcher card.
Keep production script source frozen and unchanged. Do not hide unavailable
operations, use post-start fixture actions, substitute seed XP for progress,
or loosen existing core witnesses to accommodate a variant. Keep c933f37c's
client display-name comparison for minted baseline actors. Record source root,
merged settings, fixture variant and independent deltas in the evidence.

Allowed files: crates/scenario/src/{lib,proof}.rs as required and
crates/host-play/tests/catalog_boundary_live.rs, focused existing harness tests,
05b-alcher-option-fixtures.md and evidence/alcher-option-fixtures/. Preserve
BoneBurier's full-loop predicate and other first-five cases. No api/script
product source, host-play/src, client, frontend source, profile, external engine,
operator configuration, support-matrix or STATE edits. Root owns those files.
Bank worker is concurrent; never restore/stash/copy over its working files.
Use an exact committed source export for builds; link only the two immutable
.superpowers/inputs catalog roots into its expected fixture location.

Use meaningful fixture/predicate regressions: seeded outcomes fail, first item
alone does not satisfy ordered multi-item proof, and small stacks fail the
large-batch witness. Run the affected scenario and harness checks once, preserving
failed receipts. Commit only scoped files in a new commit, request same-card
profile reviewer with the frozen candidate and evidence, then stop. No agents,
LIVE, commit amendments, gitlink, merge, remote or release actions.
