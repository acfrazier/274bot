# Integrated world, first banking and frontend session review

Use configured `grok46` defaults after first-banking, matching/fill, BoneBurier bank routing and frontend-session
same-card reviews complete. This is a bounded integration review before loadout/provisioning
acceptance; the final whole-campaign branchreviewer pass remains.
Read applicable instructions and docs/execution.md once. Campaign branch:
codex/rs2b0t-multirevision. Root owns repo hygiene and live acceptance.

Read-only source review. Write only the new report named below; root commits it.
Never restore, stash, reset, copy over or otherwise modify shared working source
to run a build. Concurrent workers own it. Use a separate git archive/export of
the exact committed host plus exact gitlink client source if a check needs a
clean view. Do not create/alter client worktrees or branches. Ask root for an
export location if it is not clear; do not touch someone else's files.

Review the combined committed candidate after both parents, including:

- Previously accepted host action/session boundary (fede3c0d and its review),
  selected-world nav manifests/bake (80720784), exact profile/resource binding,
  frontend ScenarioRunner world sharing and controlled host/world harnesses.
- First-bank client facts 56d8027 and host 8a60eef7 plus all corrective commits
  on the same banking card t_e52e0a03. Inspect actual source, not just verdicts.
  Freshness must handle empty/nonempty, close/reopen in one drain, wrong/stopped
  containers and session resets. Named stand/name/op and exact stale identity
  refusal must survive script-to-Rust dispatch. Withdraw-X must preserve real
  3000/4000 ms bounds, fixed operations, noted delivered IDs, rejected requests,
  Pause/Guardian freeze and Stop/session abort with posted outcomes.
- Root specifically found the 75514ee3 shim's added 8000 ms delayUntil is a
  competing clock: performance.now continues during Pause/hold while Rust
  freezes. Confirm the parent's correction closes that through a composed
  pause/hold/resume outcome, and abort still resolves if a bank generation is
  reused after session replacement. An unresolved known counterexample fails
  this review even if a previous same-card reviewer overlooked it.
- Matching/fill task t_41b2f50e also owns the known Pause correction above.
  Review its Rust common-loot matching and withdrawLoad observed settlement,
  including stale/no-op/All/X behavior, and the combined pending service.
- Include brief 31's corrective bank routing and actual Bank.withdraw result.
  The headed BoneBurier first-burial PASS hid an off-scene bank failure at
  Lumbridge. Require the Rust route and honest dispatch result through the
  real script call path; inspect root's strengthened scenario/core witnesses.
- Frontend source 5eeff8f0/c9521d2f plus any corrections: actual grant watermark,
  clearing external WalkArm/BankBudget/latches at logout/reconnect and same-name
  slot lifetime changes, no false clearing on scene reload or Guardian hold,
  incremental snapshot ownership and lock order.
- Root profile permit change 87084cbc and its existing positive unarmed-slot
  test: 289 host operations are enabled only after the selected host/world
  proofs. Unsupported revisions/resource mismatches and public fixture-cheat
  restrictions remain. Script imports/Start remain revision agnostic.
- Catalog harness 78c8cf21: actual production Play/start/loader, frozen source,
  selected-world runner, post-preparation baseline and independent core deltas.
  It supports the initial five cases; this is not all 180 ledger rows/options.
  Include root's strengthened ordered BoneBurier bank-cycle witness and new
  bank proof predicates. First-burial-only must fail. Root also fixes the small
  Game.combatStyleResolution return shape consumed by describeCombatStyle;
  the original headed ChickenKiller recovered and completed three cycles, but
  the caught mismatch requires a clean rerun. Broader style fallback remains
  subsequent combat capability work.

Check 02-host-boundary.md, 03-world-capabilities.md, 04-capabilities-banking.md,
05a-catalog-live-harness.md and 06a-frontend-session-observations.md plus relevant
raw evidence. Eight world cells passed on the exact frozen candidates listed;
bank return used fixed transfers and cannot qualify Withdraw-X or named shim
options. Initial catalog diagnostics may be added while you review; only the
exact recorded source/hash/predicates count. No public or native/fleet/all-card
acceptance is implied. The temporary shared-source restore incident is retained
in evidence/catalog-harness/concurrent-restore-audit.json; owners audited their
files and focused checks. Do not repeat that method.

Inspect meaningful seams and independently run focused existing checks for any
uncertain behavior. Do not rerun broad suites solely to restate receipts. Report
unresolved lifecycle/protocol/ownership defects with concrete triggers and
paths/lines. Record the exact host and client hashes you reviewed, actual model,
commands and scope limits in reviews/world-banking-frontend-integration-grok46.md.
Return approved or requested changes with the required fixes. No source edits,
new agents, LIVE, external fixture actions, merge, remotes or release work.
