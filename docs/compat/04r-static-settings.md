# Literal catalog settings and list visibility

Implementer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 09:04 UTC. Kind: bounded `crates/script` scanner + visibility
correction for brief 90. Not LIVE, ledger, STATE, runtime/UI, or a claim that
computed `ALCH_OPTIONS` is fixed. Root owns native Params proof.

Read once: `AGENTS.md`, `docs/execution.md`, fail-closed-dispatch, brief 90,
`docs/compat/04p-catalog-parameter-metadata.md`. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Owned files only:
`rs2b0t_registry.rs`, `settings_store.rs`, their focused tests,
this report, and `docs/compat/evidence/static-settings/`. Concurrent
working-tree edits were not compiled; checks used an exact host/client
Git-blob export plus this overlay in a new empty target.

## Verdict

The faithful literal subset now inlines through the existing static scanner:

- sibling literal string-array defaults (`DEFAULT_ALCH_ITEMS` → 11 keys)
- quoted string-constant defaults
- inner `anyOf` identifiers that resolve to quoted constants
  (`CUSTOM_ALCH_KEY` → `'custom'`)
- `string[]` master-value membership in shared `setting_visible`

Mixed/computed arrays no longer report a successful empty or partial parse.
Unknown expressions stay unresolved. Real empty arrays stay valid. Scalar
`showIf` equality, including case sensitivity, is unchanged. 04p's
case-insensitive comparison and FODDER-built `ALCH_OPTIONS` were not
authorized and were not implemented.

## What this does not fix

- `ALCH_OPTIONS` remains empty at Browse. Its RHS is
  `[CUSTOM_ALCH_KEY, ...ALCH_ITEMS.map(i => i.key)]`. That is still a
  computed selected-data choice, not a literal string array. Do not treat
  empty `items.options` as a chip list.
- `optionLabels` Record, richest-first order, File-Load `ALCHER_SETTINGS`
  under the `SETTINGS` name, TUI/panel editors, and isolate/`ITEM_DB`
  evaluation at Browse.

## Checks

Isolated export `/tmp/t_7bb8a58a-src-20260911045854` from host
`78cc99d07046a781fe8cd5ebf66bb63e663d23ea` plus client
`9d090ed04957e4efc254f073cda97bc5510ca72b`, overlay of the three owned
Rust files, empty target `/tmp/t_7bb8a58a-target-20260911045854`.

- `cargo test -p script --locked --test rs2b0t_registry`: 22 passed
- lib `settings_store` visibility/merge tests: 3 passed
- `cargo clippy -p script --locked --no-deps --lib --tests -- -D warnings`:
  clean
- frozen `catalog_inventory` on both catalogs: Alcher
  `source_sha256` `1a09baa1117544a2bb80da145829e69c9ced029ad5af533616afa8e7a36f2032`;
  11 default keys; `customItem.show_if` is `{ key: 'items', anyOf: ['custom'] }`;
  `items.options` still `[]`

Machine-readable copies: `evidence/static-settings/`.
