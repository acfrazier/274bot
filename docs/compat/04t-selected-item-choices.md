# Selected-item choices in native Params

Implementer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 09:45 UTC. Kind: bounded 04s implementation (brief 94).
Not LIVE, File Load, gp chip formatting, or a V8 browsing evaluator.

## What landed

Catalog `ALCH_OPTIONS` is no longer a free-text hole. The scanner attaches a
typed high-alchemy `ItemOptionSpec` on `SettingDef` when the sibling blob
matches `[prefix, ...items.map(i => i.key)]` whose `items` RHS is the frozen
`FODDER.flatMap` / `ITEM_DB.find` / `Math.floor(cost * 0.6)` / `.sort` chain.
Unknown maps, wrong rates, and malformed FODDER stay unresolved. Options stay
empty; FODDER is not copied into a Rust static.

`resolve_setting_options` borrows `Option<&SelectedGameData>` at Params.
No selected facts publishes only the prefix `custom`. With facts, missing
aliases drop, remaining keys sort by `floor(cost * 0.6)` descending then
identity label (UTF-8 order matching the demonstrated localeCompare ties),
and explicit FODDER labels distinguish shared client names. Schema is not
mutated. Inline literals and `optionsFrom: 'loadouts'` still win.

TUI ParamsPane and panel parameter editors pass the bound template /
`selected_game_data()` Arc and paint labels while persisting keys.

## Identities

| Role | Exact |
|---|---|
| Campaign HEAD at implementation | `7c2eef6eb8db6b01933c333aa64cf5f68169d8d2` |
| Parent brief-90 commit | `8bd34675b61c94707e6c5ed07a943102bf06cac5` |
| Client gitlink | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Branch | `codex/rs2b0t-multirevision` |
| Kanban card | `t_f8ca5ed7` |

Machine-readable copies:
`docs/compat/evidence/selected-item-choices-ui/{refs,isolated-checks}.json`
and `isolated-checks.txt`.

## Tests

- Registry: Alcher-shaped sibling extracts prefix `custom` plus FODDER
  `{obj,label?}` in source order; `options` empty. SHOP_PRESETS.map, ALCH_RATE
  0.5, and mixed FODDER literals refuse the descriptor.
- Resolver: 274 and 289 yield the same 37 keys, first fodder `rune_platebody`;
  no facts → `[custom]`; missing alias dropped; rune 2h before platelegs;
  Air/Earth/Fire/Water battlestaves; `dragonhide_body` label `Green d'hide body`;
  rebound facts do not mutate `options`; literals/loadouts unchanged.
- TUI: identity label paints; persisted value is the key. Existing params
  controls still pass.
- Panel: loadout combo control still lists store names.

## Not claimed

- Cosmetic gp `toLocaleString` suffixes
- LIVE Start-after-choice alchemy (root falsifier)
- File Load `ALCHER_SETTINGS` / `optionLabels` Record
- register_rs2b0t game-data overload
