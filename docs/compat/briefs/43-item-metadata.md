# Complete host item metadata used by enabled catalog settings

Use the existing campaign checkout and verify branch `codex/rs2b0t-multirevision`.
Read AGENTS.md, docs/execution.md and the fail-closed-dispatch skill. This is
step 6 capability work, after the startup worker releases shared files.

## Confirmed trigger and scope

Root's frozen 41d7a1e3/client 56d8027 ordered Alcher LIVE cell failed on 274:
`evidence/catalog-harness/live/r274-alcher-ordered-100adccc-41d7a1e3.log`.
Requested settings were rune_platebody plus rune_chainbody, but startup selected
only chainbody. `api::content::ITEMS` is a single hardcoded chainbody row.
AlcherLogic evaluates ITEM_DB at module initialization and silently filters absent
rows. The fixture is intentionally unchanged; do not select easier items or
claim the ordered option is supported from a single cast.

Operator update: game tables must be programmatically generated from pinned
server code/data as versioned JSON and consumed with serde. Use the artifacts
and schema delivered by brief 44 (generated-game-data), not hand-written Rust
item records or aliases. Deserialize once per immutable profile and share the
loaded data; native runtime must not require a server checkout or generator.
This supersedes the earlier suggestion of maintaining an audited alias table.

Implement complete host-owned metadata for enabled Alcher choices and custom
items that the selected client cache knows, for both frozen catalogs and both
274/289 profiles. Preserve name/id/cost semantics, including several dragonhide
variants with the same display name. Resolve obj aliases from audited local
content/cache provenance; do not invent an underscore normalization that merges
distinct same-name variants. Inspect existing host `ObjNames`/ItemDefView fields
and immutable per-Play sharing before choosing a bounded design. Prefer the
selected cache's id/name/cost facts. Missing content must remain absent; never
fallback to foreign 274 facts for unknown 289 items. Do not clone foreign API
bodies or import its ITEM_DB as the runtime authority. Alias data comes from generated server assets with explicit provenance and
cache validation; do not encode it manually in Rust.

Publish metadata before actual catalog module evaluation. Preserve offline
catalog/schema behavior where no cache exists honestly; test that supported
choices are exposed on a real selected-cache path. Do not serialize/deep-copy
a whole item database on every snapshot/tick or add it to every per-bot world
copy. Keep immutable data reusable for the process profile, while the JS module
may necessarily own its one-time read. No client bot API, new revision gate,
foreign gameplay policy, server edit, release or LIVE run.

Scope: serde asset loading and api content/item-definition access; script static metadata publication,
loading context and thin data shim; minimal host-play wiring if required. Do not
change bank routing/transfer semantics or panel startup decisions. Preserve all
already reviewed lifecycle and startup changes. Before source edits, write a
short ownership/design note in `docs/compat/04-item-metadata.md`; implementation
and review may cover that bounded design together.

Meaningful regressions must exercise the compiled frozen AlcherLogic/card on
host-supplied selected metadata: both requested items survive selection and sort
richest first, custom item outside the current gold chainbody resolves by alias
and display name, same-name dragonhide IDs remain distinct, unknown item remains
unavailable, and no profile cross-contamination. Do not use tests that merely
mirror a new constant table. Run affected crate checks, fmt and strict Clippy
proportional to edits. Root owns live ordered/custom checks and option ledger.

Commit only scoped source/report/evidence. Hand this same card to profile
`reviewer` via kanban_request_review, with exact commits and verification, then
STOP. Do not complete the card yourself. Review must use actual Grok 4.5.

## Operator data-table expansion (2026-09-10)

The operator explicitly includes the game data tables needed by enabled scripts
in compatibility work. After reviewed brief 45, use the same immutable serde
asset loading path for generated consumption and pickpocket facts. Replace
handwritten FOOD_HEALS and required_thieving values with the selected revision's
qualified generated values, including loadout food detection, static shim
publication and the nav audit consumer. Keep curated stands, names/order,
leashes and selection policy separate; do not copy an entire world per tick.
Bread and Anchovies currently disagree with server facts (4 and 3 respectively)
and should follow the verified server values. Dynamic or conditional effects
must not masquerade as fixed healing. Preserve unsupported-operation behavior
and current catalog option names/order. Add meaningful selected-revision checks
for these lookups and fail-closed behavior, alongside the item regressions above.
Do not implement a general acquisition planner or Quester in this task.
