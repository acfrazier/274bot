# Compatibility campaign handoff

Prepared 2026-09-11 at approximately 18:39 UTC at the operator's request.
This is the current resume entry point. `STATE.md` contains dated evidence;
its older next-step paragraphs are superseded by this checkpoint and the live board.

## First actions

1. Read this file, `AGENTS.md`, `docs/execution.md`, and the relevant card brief.
2. Inspect `hermes kanban --board 274bot list --json`, then `show`, `runs`, and
   `log --tail 5000` for the active cards below. Workers were left running;
   automatic review dispatch and dependency release remain enabled.
3. Preserve their files and caches. Finish monitoring the same-card reviews,
   verifying actual model, reviewed source, tests, and verdict.
4. After shared headed witness153 is approved, freeze an exact composed source
   export, build the visible catalog watcher, and resume the bounded failed-cell
   queue. All new LIVE must be headed so the operator can watch.

No root LIVE or build process was active when this document was prepared.
Do not assume a running worker's uncommitted code is reviewed.

## Authorization and scope

The operator authorized full implementation of the primary checkout's local
`docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md`, with continual
verified staging, integration into main, and ordinary remote pushes. No release
tag, packaging release, announcement, or distribution is authorized.

All new live runs must be visible headed (operator clarification17:21 UTC).
BankSorter must remain explicitly unavailable (operator decision17:49 UTC).
Do not import the foreign sorting planner or invent a sorting policy. Four rows
are `UNAVAILABLE_BY_OPERATOR_DECISION`; they are neither PASS nor foreign defect.
Brimhaven has four separately justified `DIM_FOREIGN_DEFECT` rows. Preserve the
original45-card/180-row inventory and all failed evidence. No full campaign
acceptance yet; source review and process exit do not establish full core proof.

The user noticed a first Fire Strike hitch. Captures correlated with72–75ms
synchronous snapshot serialization, but animation-loading causation and a
performance improvement have not been established. See06aq and brief153.

## Checkout and publication

- Primary: `/Users/acfrazier/experiments/274bot`.
- Active: `/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision`.
- Active branch: `codex/rs2b0t-multirevision`; snapshot HEAD
  `6ce32af4195e1d44dbbf6eac8862ad45234c4c25` before this documentation commit.
- Primary main and GitHub main verified at
  `57aee576b9250ccb4c7c8fd1fb5ebd12b4541aa8`.
- Published client: `aef3952d1cd7bb3b93d39c497f0f476b68021c59` on the approved
  FR-client-bothost remote branch `r274-bh-modular`.
- Active client: `52c37f9ce50d1f184656d5b4469c007ec8a5791a`, branch
  `codex/bothost-274-289`, later reviewed NPC projection, still unpublished.

Stage2 main contains the reviewed df2-equivalent product with documentation.
Its Grok4.6 whole-branch review146 approved incremental integration; this is
not final campaign approval. A fresh recursive GitHub checkout of57 passed
`cargo check --workspace --all-targets --locked --features memory-profile`
and `cargo build -p panel -p tui --locked`. No need to repeat without changes.

GitHub initially rejected two raw N32 diagnostic blobs over100MiB. Root
losslessly gzip-packaged them only in unpublished history, retaining individual
commits/topology/metadata and all product bytes; ordinary push succeeded.
Published ancestors were untouched. Mapping, original hashes, clone checks,
and push receipts are in `docs/compat/evidence/publication-compression/`.
Historical review IDs can require mapping for new ancestor checks. Local
backups `codex/stage2-before-compression` and
`codex/campaign-before-compression` preserve old history. Do not blindly merge
the later active branch into main: it has new implementation and pending review.

## Active workers and immediate review concerns

| Card | Task | Worker at checkpoint | Ownership and next action |
| --- | --- | --- | --- |
|127|`t_c6765b1f`|Sol run1390|Committed6ce32af41; isolated checks/report then same-card reviewer. Owns paired_catalog_live.rs and support/paired_catalog.rs.|
|153|`t_8ae5a895`|Sol run1391|Shared full witness extraction and visible bridge; uncommitted WIP, checks underway, then same-card reviewer.|

127 corrects the Mule first raw-load fixture with27 essence and talisman on
the Crafter before Start; requires bilateral essence/Air transfer conservation,
subsequent crafting, bank restock and continued cycle. It does not authorize
seeding after Start or accept two unrelated partial cells as a full cycle.

153 owns host-play/src/lib.rs, new src/catalog_core.rs,
tests/catalog_boundary_live.rs and catalog_core_watch.rs, plus
panel/examples/catalog_watch.rs and panel/src/{app,session}.rs. It moves the
existing full core from the integration test, with unchanged cycle predicates.
Ordinary panel behavior stays opt-out; dedicated catalog_watch enables the core.
Read `docs/compat/briefs/153-shared-headed-core-witness.md` and the three board
comments. Specific root findings already being corrected in WIP:

- Freeze the last prepared production observation immediately before actual
  Start; the first post-Start observation may already contain script work.
- Initial login sets session_boundary before Start. Invalidate the candidate
  baseline and wait for fresh preparation; only post-Start boundaries fail.
- Poll cheap qualification status, not full witness JSON every UI frame.
  The latest WIP separates `status()` and terminal `qualify()`; review concurrency
  between these calls too (a pending-to-qualified transition must not panic in
  the timeout branch's `expect_err`).
- Preserve screenshot plus matching full snapshot and terminal drain semantics.
  A bounded safe serialization change is optional if broader ownership changes
  would expand this card. Do not claim the hitch fixed from source alone.

These corrections still need exact-source tests and reviewer approval. Root
has not run new headed gameplay on this code. The dedicated watcher is expected
to emit combined scenario/core JSON; confirm final CLI/schema before updating
the root runner. Paired core is a later card, not solved by solo153.

## Completed later reviews

-139 preparation: actual Grok4.5 run1387/session20260911_134926_347385 approved.
  Resource preparation, Wildy HP40, combat prerequisites, Coal mining/ballast
  and Rock baseline; retain original failures and deadlines.
-152 async TaskBot validation: actual Grok4.5 run1388/session20260911_135127_fcd2d4
  approved. Promise(false) previously selected ContinueDialog, starving Green.
  Real isolate regressions passed; Green still needs headed full-core proof.
-144 NPC projection: actual Grok4.5 run1389/session20260911_135632_d3016b approved.
  Root verified12 owned export blobs against actual
  `e05ecafb806b5af7f1d729b6f9c23c780956c118` and client52c37f9.
  A copied nonresolving hash suffix in the report was corrected with a retained
  source-verification receipt; no different-code review is implied.
-154 noncombat audit: actual Grok4.6 run1385, eleven scenarios/44 proposed cells.
-158 paired extras audit: actual Grok4.6 run1392/session20260911_141341_411314,
  commit1b8598e51. Three extras/twelve cells; already-implemented boat routes
  verified. Death altar location content remains explicit, not a whole-card dim.

## Dependency queue

Do not recreate cards. Brief numbers below are files in `docs/compat/briefs/`.

| Brief | Existing task | Work |
| --- | --- | --- |
|37|`t_1bf9a22e`|Native spellbook teleport after153; grok46 profile|
|38|`t_1591d140`|Native shop buy/sell after37|
|39|`t_4b04cb5f`|Make-X/smithing after38|
|40|`t_79175534`|Fire lighting after39|
|126|`t_47426de7`|Native trade after40|
|156|`t_6b9a0675`|Native Flax partner exchange after126|
|147|`t_30bc0807`|Ranged/consumables after153+127+145|
|148|`t_9a7d8feb`|Alternate camps/guard response after147|
|149|`t_f8371bea`|Combat banking/return after148|
|150|`t_53f63e04`|Hazardous camp approach/return after149+37|
|155|`t_af1a1d64`|Eleven noncombat scenarios after150+40+153|
|157|`t_587aa6ca`|Visible paired full witness after153+127+155+156|
|159|`t_f9496ae0`|Nature island/stayInAltar, Mule bankFill=false after157|

155 includes aio_teleport/Falador/no-staff, shop_buyout/Aubury,
smithing_bot/platebody, leather_crafter/hard-body, firemaker/oak families.
BuyoutPlan is ancillary while SHOP_DB is empty; native Shop.buy still required.
159 must verify canonical bank essence1436 versus withdrawn note1437 and
continuous full cycles; audit seed recipes are proposals, not established proof.
The unrelated triage card `t_95bef768` is not new campaign work to dispatch.

## Next headed proof and evidence

Use reviewed153 in an exact export with known client identity. Prior root
helper `.superpowers/headed-current-root/run_cell.py` supports only mage/Green
and records scenario diagnostics; adapt it to require the full core receipt.
`.superpowers/headed-current-root/bundle.py` wraps the frozen binary in a native
app for viewing. Read actual screenshots through an image-capable tool.

Priority requalification:139 resource/Ardy/Wildy/Coal/Rock;152 Green;144 Fire NPC
geometry; mage/control on composed153. Both274/289 and frozen catalogs
100adccc037d9f6898080e1cad58fcfc43364775 and
8e7d965be2071d6ec65c3265e12af797082d720a. Preserve180s/scenario clocks and
held-newer-after-old-failure policy; do not rerun until lucky. Exactdf2 had four
full mage PASSs and two supplemental headed scenario PASSs; those do not prove
the new shared observer. All historical failures remain retained.

Later native capabilities, all distinct options, visible paired cycles,
integrated fleet/lifecycle/platform refresh and final Grok4.6 whole-branch
requirement-to-evidence review remain. Review completion is not LIVE acceptance.

## Local preservation and operational details

The primary checkout's unrelated untracked CONTEXT, memory/package files and
other artifacts remain untouched. Active checkout has many retained raw captures,
helpers, source exports and caches. In particular, the old dirty
`docs/compat/evidence/periodic-bank-capabilities/script-lib-periodic-bank.txt`
is unrelated to127/153. Do not blanket add/reset/clean or change the client pin.

Root cache `.superpowers/review-exports/t_a74e9684-combat-target` is reusable for
non-performance integration checks. Active caches include target-t_8ae5a895,
target-t_c6765b1f and target-t_c6765b1f-exact. Free disk was138GiB at18:38 UTC.
Completed cache removals have receipts; never remove raw evidence or frozen
binaries as cache cleanup. Do not restart the gateway just because the CLI
prints its old mixed-module warning while valid workers are running.

Board snapshot: `.superpowers/stage2-root/handoff-board.json` (local, stale as
soon as a worker advances). Re-query live board on resume. No automation was
created for this handoff and no new task was created in Codex.
