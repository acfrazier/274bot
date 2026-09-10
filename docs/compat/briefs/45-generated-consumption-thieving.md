# Generate consumption and pickpocket facts for compatibility

Use the existing campaign checkout and verify branch
`codex/rs2b0t-multirevision`. Read AGENTS.md and docs/execution.md. The operator
explicitly includes server-derived data tables required by enabled scripts in
this campaign. This task extends the accepted brief-44 pipeline at 3b53ce2c.

## Scope and ownership

Own only `tools/game-data/`, generated game-data JSON assets and your report
`docs/compat/04-generated-consumption-thieving.md` plus scoped evidence. No Rust,
shim, frontend, server, account, LIVE, publication or acquisition-planner edits.
Another worker will consume these assets after source review. Keep existing
item facts and cache identities stable while extending the payload/schema.

Generate revisioned consumption and pickpocket facts from each pinned 274/289
engine/content input using the same roots and provenance contract as brief 44.
Prefer the existing offline DbTableType/DbRowType decoders where practical;
otherwise a bounded config parser must preserve repeated rows, typed columns,
aliases and counts. Bind every consumed table/config/decoder/semantic helper
to its actual hash, enforce pinned refs and reject relevant dirty inputs.
Preserve environment-root overrides and reproducible output. Do not introduce
handwritten food-heal, NPC-level, loot, or alias tables in the generator.

Known sources to inspect (relative to content):
- `scripts/player/configs/consumption/consume.dbtable` and relevant `.dbrow`
  files, including consume_normal. Keep stat/constant/percent tuples distinct,
  including multiple effects, partial servings and aliases. Preserve raw server
  units and label them; do not flatten variable healing into a fixed maximum.
- `scripts/player/scripts/consumption/effects/scripts/consume_effects.rs2`
  and relevant consumption dispatch distinguish ordinary fixed food from other
  conditional effects. The consumer needs a trustworthy fixed-HP-heal view.
- `scripts/skill_thieving/configs/pickpocking/pickpocket.dbtable` and `.dbrow`:
  NPC IDs/aliases, level, raw XP, stun facts, success-chance tuple and loot
  item/quantity/weight tuples. Include identity needed to join NPC aliases and
  display names without collapsing distinct IDs. Curated training stands and
  leashes remain host policy, outside the generated facts.

## Verification and deliverable

Re-run generation twice and compare bytes. Extend verification to real source
and output hashes, cache identity, alias/ID validity and coherent joins. Check
both revisions for Lobster=12, Bread=4 and Anchovies=3 fixed HP healing and
Guard required Thieving=40 against actual selected input. Confirm multi-bite
foods describe one consumption action, not an invented whole-item total.
Confirm repeated NPC and loot rows survive parsing and unhandled/conditional
facts remain explicitly unqualified. Test behavior of any new parsing logic
with a small meaningful fixture rather than mirroring a constant list.

Inventory mismatches against current `api::content::FOOD_HEALS` and thieving
levels. Root has already found Bread=5 and Anchovies=1 in the handwritten
table; these differ from selected server facts. Report actual values and exact
derivation so the runtime worker can correct them without guessing. Document
the final JSON shape and qualification rule for fixed food healing.

Keep this bounded to consumption and pickpocket facts; spell/production/shop
tables have their own upcoming capability briefs. Commit only your scoped
files, hand this same card to profile `reviewer` using kanban_request_review
with exact commits and verification, then STOP. Do not complete it yourself.
