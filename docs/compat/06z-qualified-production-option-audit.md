# Remaining options on qualified production cards

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 11:58 UTC. Kind: bounded read-only campaign audit of brief
119. Own only this report and `docs/compat/evidence/qualified-production-option-audit/`.
No code, build, LIVE, STATE, support-matrix, ledger, or new cards. Root owns
acceptance.

Read once: `AGENTS.md`, `docs/execution.md`, brief 119. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Frozen cards+siblings are the two
catalog inputs. Actual settings come from those sources; matrix scanner rows
were compared and are incomplete. Isolated PASS is taken from named harvest
summaries/core-results, not lexicographic last `host_commit` and not giant
logs. Concurrent working-tree edits are not a tested binary.

## Verdict

**Replace “remaining supported settings” on these eleven PARTIAL cards with
three named scenarios (twelve catalog×revision cells). Everything else is
already isolated-PASS, the same generated action on different content, shop-
owned, or paired-owned.** Do not mark final PASS. Do not dim. Do not invent
Steel/Mithril dart tiers, every herb/gem/bar, every Alcher chip, or frontend
clicks per tier.

Still needed:

1. `alcher_defaults` — empty `items=[]` fallback to `DEFAULT_ALCH_ITEMS`.
2. `bank_fletcher_shafts` — catalog default `product=Arrow shafts` / `Logs`.
3. `bank_fletcher_headless` — `workKind=attach` (`Headless arrows`).

Shop `buyVials` stays on the existing shop implementation. RuneCrafter
Runner/Mule Recipient stays on existing paired-runner work. Final N32 /
lifecycle / native controls stay campaign-wide gates.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `4a5ba565c75552bca8cb4076288d4f0eb60995ae` |
| Client gitlink | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 119 SHA-256 | `ca2dd6080eeeed211c5bfa9b52c656f2b58a488d9962eab64ccba6df96d01e00` |
| Kanban card | `t_6f8dec2b` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| Isolated Alcher/Bone/Fletcher core | `f6b9ec4b2e7afe22f4a5bb58a837f3b9a992cb5f` |
| Isolated inventory + stringing | `a4157243817d8606f6c22a5ca2583f106d98bb85` |
| Isolated Superheater Bronze/Steel | `6d750e65a227aa8a39f2bb6fdc1e5ceb5a452338` |
| Isolated Fire battlestaff | `3be68eafffd586d790e72e8095ddcab457795672` |
| Isolated potions | `b1cff8a7fdf6d18a5f7a3c7c23c6fa8a020e173b` |
| Isolated vials West | `2f1bd9cf75bfcff0900eb9e029deb98433f5a6d3` |
| Isolated vials West+East | `060e15a0fb065f5e74405116f4b5498445c6a825` |
| Isolated RuneCrafter Air/Earth | `4cc6cc27d7048c4c60e6b0393ae92132d9b88a1f` |
| Isolated Smelter Bronze/Steel | `91289e9bac158721849209a301dcae5c26988eb9` |

Card/sibling SHA-256 values: `evidence/qualified-production-option-audit/source-hashes.json`.
`cmp` is 0 on both catalogs for every card except BankFletcher and Superheater.

## Method

Meaningful remaining work is default/all vs named filter, alternate
target/tier, bank/source production mode, or a legitimate stop. Not a
Cartesian product of scalars, log types, or chip lists. One default does
not qualify every string name. The same generated action on another content
row is data substitution, not a new cell. Unknown content/API is not
substitution. Matrix `status_detail` that says “remaining supported
settings” or “full supported branches” is the text this audit replaces.
Import/compile/seed is not qualification. Evidence dates and isolated host
hashes stay separate from a later integrated-source refresh.

## Per card

### BoneBurier — none remaining

One setting: free-text `boneName` default `Bones`. Isolated `f6` all four
cells PASS the withdraw/close/further-bury loop (`prayer` 27). No inject;
the catalog default runs. Further names are the same `Bank.withdraw` +
`interact('Bury')` on another row. A name without Bury already stops. Do
not add Big/Dragon bones cells.

### DartFletcher — none remaining

`tier` options Bronze…Rune. Isolated `a4157243` Bronze and Iron, all eight
cells, exact product ids 806/807 and Fletching 180/380. Remaining tiers are
the same Feather-on-tip spam with a different plan row. Stop-when-empty is
the script end condition, not a second production mode.

### HerbCleaner — none remaining

Default `herbs=[]` vs named `['Guam leaf']` is the default/all vs filter
pair. Isolated `a4157243` all eight PASS (full pack, deposit, restock,
further guam, named keeps unselected 201 in bank). Other herb names are
Identify-on-unid-id substitution. Empty eligible-set stop is an error
path, not a production cell.

### GemCutter — none remaining

Same pair: `gems=[]` vs `['Sapphire']`. Isolated `a4157243` all eight PASS.
Named keeps uncut opal 1625 in bank and crushed 1633 at 0. Crush is server
RNG on the same chisel `useOn`, not a setting. Other gems are substitution.

### Alcher — one remaining (`alcher_defaults`)

Covered on isolated `f6`, both catalogs, both revisions:

- `alcher` injects `items=[rune_chainbody]` (not the 11 defaults)
- `alcher_custom` / `_alias` / `_name` (custom sentinel, obj and display)
- `alcher_ordered` (plate then chain, Magic 130)
- `alcher_large_batch` (`alchs=1000`)

`selectedAlchItems` with no `custom` chip and an empty pick list falls
through to `DEFAULT_ALCH_ITEMS`. That fallback is unwitnessed. Other fodder
chips and dragonhide display collisions are `withdrawXById` + High Alch
substitution (custom alias already used generated id 1331/1332). `alchs`
1/27/1000 already exist. Exhausted-stock stop is lifecycle, not a new
option cell.

### BankFletcher — two remaining

`workKind` is `knife` | `string` | `attach` (newer also `cut+string`).
Covered:

- knife Short bow, Willow, isolated `f6` all four (`fletching` 932)
- string String short bow, isolated `a4157243` all four (`fletching` 232)
- cut+string newer catalog only, isolated `a4157243` both revisions
  (`fletching` 133). Old catalog FAIL is unsupported `mode`, not remaining
  work.

Unwitnessed distinct modes:

- Arrow shafts (catalog default product, Logs-only knife)
- Headless arrows (attach; material/knife ignored)

Do not add every log, Long bow (same knife), explicit `mode=cut`/`string`
(auto-from-product already ran), or tipped arrow metals (same attach loop).
Tile/leash/booth scalars are not production modes. Make-X stays a campaign
gate; shafts reuse it rather than a new Make-X task.

### Superheater — none remaining

Bronze (2 ores) and Steel (1 iron + 2 coal) isolated `6d750e65` all eight.
Newer Fire battlestaff isolated `3be68eaf` both revisions; `6d750e65`
Attack-1 FAILs retained. Iron/Silver/Gold are the same `withdrawSet`
loop with one ingredient; Mithril+ are Steel-shaped coal ratios; Blurite
is quest-gated content, not an ordinary cell. `natures` is a scalar.
Other `FIRE_STAVES` are `pickFireStaff` substitution.

### RuneCrafter — none remaining on Solo

`RUNE_OPTIONS` is only Air and Earth. Isolated `4cc6cc27` all eight Solo
cells PASS. There is no third rune. Runner / Mule Recipient / `partner`
are paired trade modes owned by existing paired-runner work. Do not create
a new pair or shop task here.

### PotionMaker — none remaining

Custom Guam+newt vs named Ranarr+snape is custom vs selector. Isolated
`b1cff8a7` all eight PASS (unf then finished ids, deposit 14, restock,
named keeps unselected Guam). Other recipes are the same batched `useOn`
on different ids.

### VialFiller — none remaining on fill

West and East, `buyVials=false`, isolated `060e15a0` all eight (West also
`2f1bd9cf`). Those are the only `BANK_STANDS` keys. `buyVials=true` and
the three shop scalars await existing shop implementation (brief 38). Do
not create a new shop task. Matrix options listing only Falador West is a
scanner drop, not source.

### SmelterBot — none remaining

Bronze and Steel isolated `91289e9b` all eight, including ordered
withdraw, deposit, closed return and further bars. Further bars follow
the Superheater recipe-table equivalence. Location/leash/obstacle strings
are not production modes. Make-X already ran on these cells.

## Minimal named set

| Scenario | Inject | Witness | Cells |
|---|---|---|---|
| `alcher_defaults` | `items=[]` | One `DEFAULT_ALCH_ITEMS` obj (seed only `yew_longbow`), noted withdraw, High Alch, Magic XP. No chainbody. | 2 catalogs × 2 revs |
| `bank_fletcher_shafts` | `material=Logs`, `product=Arrow shafts` | Exact shafts up, logs down, XP, deposit, restock, further shafts | 4 |
| `bank_fletcher_headless` | `product=Headless arrows` | Exact headless up, feather 314 + shafts down, XP, deposit, restock, further headless | 4 |

Twelve cells. Exact item ids from selected 274/289 rows at fixture time; do
not freeze guessed ids here. Use existing `scenario::get` / `names()`. Do
not change frozen catalog source.

## What this is not

- Not final card PASS and not a dim.
- Not a demand to click native/TUI controls on every remaining tier.
- Not a new shop card (`buyVials`) and not a new paired-rune card.
- Not integrated-source N32 refresh; keep those hashes separate.
- Not acceptance from the matrix scanner or from this report alone.

Machine copies: `evidence/qualified-production-option-audit/{refs,source-hashes,frozen-settings,isolated-pass,remaining-cells}.json`.
