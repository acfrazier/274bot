# Step-6 combat, casting, shop, trade and production design

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-10. Kind: bounded pre-implementation design/audit of
`docs/compat/briefs/24-combat-production-design.md` against frozen
enabled callers and committed host shims. Not runtime evidence, not live
acceptance, not first-family banking or provisioning source review, not
campaign release.

Read once: `AGENTS.md`, `docs/execution.md`, fail-closed-dispatch, plan
steps 6–7 in `docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md`,
`docs/compat/support-matrix.json`, `docs/compat/04-banking-design.md`,
`docs/compat/04-provisioning-recovery-design.md`, `03-world-capabilities.md`,
and the named brief. Branch checked first: `codex/rs2b0t-multirevision`
(not `main`). Work was read-only except this report. No product edits,
fixtures, STATE, matrix, subagents, commit, stash, reset, restore,
checkout, merge, remotes, live sessions, or suite reruns. Concurrent
first-family bank WIP was inspected read-only and is labeled below; it
is not accepted product.

## Verdict

**Proceed to scoped combat/cast, hostile-player, shop/trade and
production implementation after the first banking family is reviewed**,
under the constraints below. Design approval is not source acceptance
and not live acceptance.

Do not clone `CombatStyleLogic.ts`, `spelldb.ts`, `Autocast.ts`,
`Special.ts`, `Shop.ts`, `Trade.ts`, `Firemaking.ts`, `DuelInterface.ts`,
or a universal action framework. Reuse posted snapshot rows, existing
`Interactions` (`use_widget_on`, `use_item_on`, `shop_buy`, `press`,
player/npc/held ops, count-dialog), and thin shims. Scripts load and
start irrespective of revision; missing operations refuse at host/client
boundaries.

Banking (fresh snapshot / Withdraw-X / named booth open) and
provisioning/recovery (common-loot, withdrawLoad, loadout,
PeriodicBank, DeathRecovery) already have designs. This report does not
redesign them. `Bank.withdrawLoad`, `Banking.open`, `setNoteMode`,
PeriodicBank and DeathRecovery stay those families' proofs.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `367af78fd3b6e4d7bcfc2ff1ae0cdc6f62f479ee` |
| Client pin (submodule HEAD) | `6cb5a0b17aeef74da6b57b205e916681daee4f76` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief SHA-256 | `0c3cf9dddd72da263c14622546bb8dc55c6749e73e54843768d8020e4fb631ed` |
| First-family design SHA-256 | `09e924b847ab07dafa5af4f1fdad7ecf917ef0f05cb2236909ae83ff53edfce2` |
| Provisioning design SHA-256 | `22b80f5447d263afec31a6c55cb78caccbfe6ce2d3547cbf046814f5553c3cdd` |
| Audit SHA-256 | `a954c7bf50d1bcbc3d0d245f291348dad6bd9b7d3a4f48ce2528de4d3601385f` |
| Catalog roots | `.superpowers/inputs/rs2b0t-100adccc037d9f6898080e1cad58fcfc43364775` and `…/rs2b0t-8e7d965be2071d6ec65c3265e12af797082d720a` |
| Kanban card | `t_9f1baf44` run 1125 |

API files used by these families (`Game.ts`, `Autocast.ts`, `Special.ts`,
`spelldb.ts`, `Shop.ts`, `Trade.ts`, `targets.ts`, `LightFire.ts`,
`Firemaking.ts`) are byte-identical across both catalogs. Only
`BankFletcher.ts` / `BankFletcherLogic.ts` and `Superheater.ts` /
`SuperheaterLogic.ts` differ. That difference is source evidence, not
permission to collapse the later option set.

Current bank WIP (uncommitted, Sol `t_e52e0a03`; **not** this report's
product): `crates/api/src/{interact,snapshot}.rs`,
`crates/host-play/src/lib.rs`, `crates/script/src/shim/{bank.js,mod.rs}`.
Combat/cast/shop/trade/production shims were read from the worktree and
match committed behavior for these families (shop/trade/cast files were
not in that diff). This design assumes first-family contracts in
`04-banking-design.md`, not the unfinished alias.

## Caller / branch / gap / proof

Enabled set only. Banking/provisioning callers are listed only where
this family still owes a non-bank operation.

| Caller / branch | Required Rust operation | Current gap | First live proof |
|---|---|---|---|
| ChickenKiller / RockCrab / MossGiant / FireGiant / AutoFighter **mage** | `castsAvailable` / `runeWithdrawList` / `Autocast.arm` from host spell+staff facts + posted combat-tab coms | shim `SPELL_DB` is settings keys only; those three throw; `Autocast.arm` throws unless choose/toggle/spell coms are posted | Wind Strike (or the set spell): autocast varp 108==3, then combat XP with **no** inventory elemental runes while a providing staff is wielded |
| GreenDragon / AutoFighter **special** | `Special.cost` / `ready` / `arm` from content param + posted spec-bar com | energy/armed need posted varps 300/301; cost/ready/arm/bar throw | dragon dagger: energy ≥ 250, one bar click, next hit spends `%sa_attack` |
| GreenDragon wilderness | `Game.attackedByPlayer` = local `face_entity` ≥ 32768 | throws; isolate does not post self face | posted player-face while a player is the combat target; flee/teleport path runs; a queued walk is not that fact |
| ArdyCakes / ArdyThiever `guardResponse=Fight` | `isHostileAttacker` over posted NPC rows | `HOSTILE_NAMES=[]` and the predicate throws | Guard/Knight/Paladin/Hero in combat, not targeting another player, ≤ radius, Attack op present → Fight; Flee still walks `FLEE_TILE` |
| Duel Arena Combat Trainer | player Challenge/Fight + duel modal accept/cancel | `Input.interactPlayer` maps; `reader.ifText` throws so partner/waiting are blind; modal IDs are catalog-local | Challenge → select modal 6575 (or the 289 id if different) → accept → Fight XP. Seeded modal is not a duel |
| Alcher | `Game.castOnItem('High level alchemy', item)` settle | queues `use-widget-on` only if magic-tab `spell_buttons` posted; no XP wait in the op | Magic XP + target/rune down + coins up, including noted stock |
| Superheater 100adccc | same cast onto primary ore; wield **Staff of fire** | cast same as Alcher; staff path uses existing equip after bank close | Magic+Smithing XP, ore/nature down, bars up through a bank cycle |
| Superheater 8e7d965b | every `STAFF_RUNES` fire provider, bank re-check, wait Wield after close | shim `spelldb.js` does not export `STAFF_RUNES`; later source imports it | each accepted fire staff on the applicable content identity; supporting only Staff of fire fails the later catalog |
| AIO Teleport / FireGiant / GreenDragon `Game.teleport` | press named spellbook teleport (not a nav walk) | throws; `spell_buttons` are `button_type==2` target spells only | Magic XP **and** tile change to that teleport's landing. Queued if-button is not arrival |
| ShopBuyout / GnomeMagicChopper / HerbloreSecondaries / TannerBot / VialFiller `Shop.buy` | host-owned Buy 10/5/1 inv-ops, ≤5 user events/tick, settle on held-count | shim `if-button` + optional `answer-count`; `Interactions::shop_buy` exists but is unused and does not loop/settle | named stock count down, inv count up by the requested qty (or remaining stock). Queued click fails the cell |
| NatureCrafter / BrimhavenAgility `Shop.sell` | same pending on the **player** shop pack | `Shop.sell` throws; `ShopView` has stock only | named inv count down, shop stock or coins move. 289 shop player container must be the posted one |
| RuneCrafter / NatureCrafter / MuleCrafter / FlaxRunner Trade | request / offer / accept / decline on posted trade IDs | shims exist; accept throws if `trade_accept_id<0` | partner window, offered items appear, accept, inv transferred. `Trade.request` without `active()` fails |
| BankFletcher **knife** (both catalogs) | `ChatDialog.make` / `makeX` after knife `useOn` log | `make` clicks a posted qty; `makeX` answers count after **1 tick** without waiting `count_dialog_open` | Fletching XP + shafts/unstrung in pack. Count dialog must be observed before Answer-Count |
| BankFletcher **string/attach** | item-on-item `useOn` | already queued `use-on` | unstrung+string → strung, or headless+tips → arrows |
| BankFletcher 100adccc | product-driven kind; `Bank.openBooth` / `openNearest`; `leashRadius` | open is first family; production as above | default Arrow shafts + Logs: cut cycle, bank return |
| BankFletcher 8e7d965b `mode=cut\|string\|cut+string` | catalog owns mode; host owes `Banking.open({stand,boothName,boothOp})` + make/useOn | open is first family; do **not** drop cut+string | `cut+string` Long bow: logs → unstrung → strung in one trip. Collapsing to auto/product-only fails this catalog |
| SmithingBot | main-modal anvil panel `isMainMakePanel` / `mainMakeProducts` / `makeFromPanelMax` | all three throw; isolate has chat `make_products` only | Smithing XP, bars down, named product up |
| CookBot `surface=Range` (default) | loc interact + chat make | make as above | Cooking XP on the named range |
| CookBot `surface=Fire` / Firemaker | `lightFire` completion + next tile in posted `FIRE_PLOTS` AABB | `lightFire` is useOn without XP/`CANT_LIGHT` wait; `findBurnLane` / `inFirePlot` throw | Firemaking XP, a Fire loc on the plot, then cook/burn continues. Walking the bank tile is not a light |
| GemCutter / HerbCleaner / PotionMaker / LeatherCrafter / DartFletcher | item-on-item (+ DartFletcher continue) | `useOn` exists | skill XP + product in pack |
| SmelterBot / FlaxSpinner / FlaxAIO spin | chat make / makeX | same makeX count-dialog gap | Smithing/Crafting XP + product |
| FlaxPicker / CoalTrucks / DoorOpener / GnomeCourse | loc interact | existing loc op | skill or door change. Not this family's missing API |

Dim / import-blocked / quest `castOnLoc` / `gearOf` stay deferred. Clue
`SolveClue` stays pre-Start refusal. A normal resource-gathering loop
(FlaxPicker, CoalTrucks) is not the excluded gatherer rewrite.

## Exact contracts (callers, not planner bodies)

### Spell accounting and targeted cast

Frozen `spelldb.ts` (both catalogs, generated):

| Fact | Contract |
|---|---|
| `SPELL_DB` | 16 autocast combat spells, `ssb` 0..15, level, per-cast rune names/counts |
| `STAFF_RUNES` | 14 staves; Staff of fire / Fire battlestaff / Lava battlestaff / Mystic fire staff / Mystic lava staff provide Fire rune (lava also Earth) |
| `spellButtonCom(name)` | `1830 + ssb`, or -1 if unknown |
| `castsAvailable` | min floor(held/cost) after staff substitution; no remaining cost → +∞; unknown spell → 0 |
| `runeWithdrawList` | remaining costs × requested casts |

Shim today: `SPELL_DB` empty objects, no `STAFF_RUNES`, the three helpers
throw. 8e7d SuperheaterLogic **imports** `STAFF_RUNES`; that catalog
cannot even construct `FIRE_STAVES` on the current shim.

Host owns rune/staff substitution from the **selected** cache, not a JS
restore of `gen-spelldb.ts`. Verify ssb layout and staff params on both
content identities before baking 1830+ssb; a 289 mismatch is recorded
and the posted staff-spell panel children win over a hardcoded base.

`Game.castOnItem(spell, item)`: open magic tab if needed, resolve the
spell's target button (`button_type==2` + `target_base`),
`use_widget_on` held item. Return is not success. Callers wait Magic XP
or item change. Preserve Pause/Stop/reconnect abort. Do not implement
`castOnNpc` / `castOnLoc` — no enabled caller.

Alcher / Superheater also `Game.openSideTab(6)` (exists) and
`Equipment.equip` (exists).

### Autocast

Frozen `Autocast.arm`: staff combat-tab root 328, choose 353, spell panel
1829, toggle 349, varp 108 armed==3. Those literals are **cache
evidence to verify**, not shim constants. Host posts choose / spell-grid
/ toggle component ids from the selected combat tab. `arm` is three
`press`es with the existing 3000 ms step waits. Missing coms stay
`not impl`, never a fake armed=true.

Melee/range `Game.setCombatStyle` / `setCombatMode` already map posted
`combat_styles`. Keep them. Mage still needs this family.

### Hostile player facts

| Fact | Contract |
|---|---|
| `Game.attackedByPlayer` | local `face_entity >= 32768` (`PLAYER_FACE_BASE`) |
| `Npc.targetsAnotherPlayer` | already `target_kind==2 && target_index != self_slot` |
| `isHostileAttacker` | name in {Guard, Knight of Ardougne, Paladin, Hero} **and** inCombat **and** !targetsAnotherPlayer **and** distance ≤ max **and** actions includes Attack |
| Duel Challenge / Fight | player op slot 1 / 2 (`Input.interactPlayer`) — already mapped |
| Duel partner / waiting | `reader.ifText` on catalog coms 6671 / 6684 / 6571 |

Post `attacked_by_player` (or self `face_entity`) from
`decode_target` already in `snapshot.rs`. Host owns the four hostile
names (same pattern as `matches_common_bank_loot`); do not restore
`targets.ts`. `reader.ifText(id)` is a thin lookup of posted
`WidgetView.text` — widgets already carry text. Verify duel modal roots
6575 / 6412 / 6733 and accept 6674 / 6520 in both selected interface
archives; a 289 id change is posted from that archive, not assumed 274.

### Shop

Frozen `Shop.buy`/`sell`: while open and remaining > 0, resolve the
stock (buy) or player-pack (sell) row, emit Buy/Sell 10 then 5 then 1
ops, **at most 5 user-event packets per tick**, wait held-count change
**3000 ms**, then **1 tick** settle, add the delta. `Shop.open` is NPC
Trade, wait `isOpen` **3000 ms**, 3 attempts. Close waits `!isOpen`
3000 ms.

`ShopView` today is `{open, stock}`. Sell needs the player container
(catalog 3823; world audit: shop root/inventory decode at the audited
ids with equal shape on both archives — confirm the **player**
inventory, not only stock).

Do not use shim `if-button` on stock `component_id`. Wire to a host
pending that loops existing `shop_buy`-style item ops (fix it to
10/5/1 remaining, not a single 1/5/10 guess, and do not answer-count
unless a Buy-X dialog is actually open). Do not copy `shopOpBatch` into
JS. `Shop.buyById` has no enabled caller — leave `not impl`.

### Trade

Posted `trade_offer_open` / `trade_confirm_open` / `trade_partner` /
`trade_mine` / `trade_theirs` / `trade_side` / accept / decline ids.
World audit: trade side/main, other-player label, main and confirm roots
decode equal across selected archives. Shim request/offer/accept exist.
Proof is transferred items, not a queued `player Trade`. First-family
Pause/Stop/reset abort applies. No second trade protocol table.

### Production make menus

Chat make-menu (`make_products`) is already posted. `ChatDialog.make`
clicks the largest posted qty>0. `makeX` must wait posted
`count_dialog_open` (family-1 bound **3000 ms**), one `answer-count`,
wait dialog close **3000 ms**, then `!isMakeMenu` **5000 ms**. Do not
keep the 1-tick race. Reuse Withdraw-X's count-dialog latch; do not
extend `PendingBankFetch`.

Smithing anvil is a **main** skill-multi panel, not the chat make-menu.
Post those item rows (name, ops, component_id, slot, id) from the open
main modal. `makeFromPanelMax` presses the Make-N op with the largest
N. Quest/gatherer `makeFromPanel` stays out.

Item-on-item `InvItem.useOn` already queues `use-on`. Proof is XP/item
delta.

### Fire

Rust `FIRE_PLOTS` already posts Varrock East/West, Draynor, Seers.
`lightFire` must tinderbox→logs, then wait Firemaking XP **or**
`/can't light a fire here/i` on game chat, with the frozen start/light
tick bounds from catalog `Firemaking.ts` (do not invent new timeouts;
read them at implementation from that identical file). Return
`lit` / `blocked` / `stalled`.

`findBurnLane` is a foreign ranker — do not clone it. Host next-tile:
inside the posted plot AABB, walkable, no existing Fire loc, not the
refused set for this attempt. `inFirePlot` is the AABB test. Firemaker
`toolRestockPlan` for tinderbox is named withdraw after snapshotReady
(provisioning/first family), not a Tools planner. `bestPickaxe` /
`canWieldTool` stay `not impl` (no enabled caller here; gatherer is
deferred).

## Ownership split

```
client  widgets (text, target_base, button_type), face_entity,
        shop/trade/make-menu/count-dialog already published

host    spell/staff rune facts from selected cache
        autocast choose/grid/toggle coms from selected combat tab
        spec bar com + sa_energy cost from selected content
        attacked_by_player / is_hostile_attacker
        widget ifText lookup
        shop player pack + pending buy/sell
        smithing main-panel rows
        lightFire pending + next burn tile in FIRE_PLOTS
        Pause/Stop/reset/reconnect abort

shim    marshal names; throw not impl when posted facts are missing
```

Fail-closed: empty `spell_buttons`, missing shop player pack, missing
anvil rows, missing STAFF_RUNES stay honest `not impl`. Do not idle
mage/Fire/cut+string branches.

## Recommended task order

Do **not** start these until first-family source is reviewed
(`t_e52e0a03` / brief 18). Superheater staff withdraw and BankFletcher
open/withdrawLoad also wait that family (and withdrawLoad waits
provisioning). Root still owns live 274/289 fixtures.

1. **Spell facts + targeted cast** — host `STAFF_RUNES` / rune costs /
   `castsAvailable` / `runeWithdrawList` / `spellButtonCom`;
   `castOnItem` settles on Magic XP or item change. Unblocks Alcher,
   Superheater (both catalogs once STAFF_RUNES exists), and mage
   restock lists. No autocast yet.
2. **Autocast arm** — after 1. Posted combat-tab coms, varp 108==3.
3. **Hostile facts** — `attacked_by_player`, `isHostileAttacker`,
   `reader.ifText`. Independent of magic.
4. **Special arm** — posted spec bar + content cost. Independent of 1.
5. **Spellbook teleport** — post non-target magic buttons; `Game.teleport`
   is press+tile/XP, not nav.
6. **Shop buy/sell pending** — player pack + 10/5/1 settle. Independent
   of combat.
7. **Make-X count-dialog + smithing panel** — reuse family-1 count latch;
   post anvil rows.
8. **Fire light + next-tile** — after chat publication (already) and
   loc snapshot.

Each task is one implementer card, same-card `reviewer`, then stop.
Do not merge 1 with 2, 6 with 7, or 7 with 8. BankFletcher 8e7d
cut+string and Superheater fire-staff alternatives are **proofs** on
1+7 plus first-family/provisioning, not extra host APIs.

## API shape (append only)

No new isolate opcode. Reuse Interact names; append snapshot fields.

| Addition | Where | Notes |
|---|---|---|
| staff/spell rune tables | `api::content` from selected cache | export onto `__rs2b0t_host.content`; thin JS |
| `castsAvailable` / `runeWithdrawList` / `spellButtonCom` | host helpers, thin JS | no gen-spelldb clone |
| `attacked_by_player` | Snapshot append | from local face_entity |
| `is_hostile_attacker` | host predicate | four frozen names |
| `if_text(component_id)` | reader over posted widgets | Duel partner/waiting |
| `shop_player` rows | Snapshot beside `shop_stock` | sell |
| shop-buy / shop-sell pending | script slot latch | 10/5/1, 5/tick, 3000 ms |
| smithing panel rows | Snapshot when main skill-multi | not chat make_products |
| makeX | existing answer-count | wait `count_dialog_open` first |
| `light-fire` pending | script slot | XP or CANT_LIGHT |
| next-burn-tile | host helper | AABB + collision + Fire locs |

`PendingBankFetch` stays the nav BankBudget path. These latches are
separate, one of each kind, pumped with `dispatch_script_interact`.
Pause freezes clocks and does not send. Stop / reset / reconnect abort
false.

## Existing timeouts (do not change)

| Bound | Value |
|---|---|
| Walk / walk-near | 60_000 ms |
| Bank waitReady / withdraw settle | 4_000 ms |
| Count-dialog wait | 3_000 ms |
| Shop open / buy settle / close | 3_000 ms + 1 tick |
| Autocast step | 3_000 ms |
| Chat make menu close after makeX | 5_000 ms |
| Side tab open | 2_000 ms |
| Special arm confirm | 2 ticks |
| Isolate RUNTIME / JOIN / SLOW_TICK | 50 ms / 2 s / 50 ms |
| FRAME / IDLE_PARK | 20 ms / 600 ms |

Foreign Shop has no Buy-X in the 10/5/1 batch; do not invent one.
Foreign PeriodicBank 120 s return is still not adopted.

## Meaningful tests (source; no live run here)

Must traverse script call → IPC → Rust send/pending → posted result.
Must not snapshot source text.

- `castsAvailable('Wind Strike', ['Staff of air'], held)` ignores air;
  unknown spell 0; Fire Wave with mystic fire staff still needs blood+air.
- 8e7d `pickFireStaff` sees Staff of fire first, then every fire
  `STAFF_RUNES` entry; Staff of air is not a candidate.
- `Autocast.arm` with posted choose/grid/toggle: three presses, armed
  when varp 108==3. Missing coms throw, no send.
- `Game.castOnItem` without spell_buttons throws; with posted Superheat
  button + copper: one `use-widget-on`, Magic XP or item change.
- `Game.attackedByPlayer` true iff posted face ≥ 32768. Duplicate snapshot
  does not re-latch a script flag.
- `isHostileAttacker` Guard in combat targeting self true; targeting
  another player false; Man false.
- `Shop.buy('Feather', 25)` is 10+10+5, never a 6th user-op in one tick;
  held feathers +25 or stock exhausted. `Shop.sell` without player pack
  throws.
- `ChatDialog.makeX` with count dialog closed: no Answer-Count. Open:
  one answer, menu closes.
- `makeFromPanelMax('Bronze dagger')` clicks the largest Make N on the
  posted anvil row.
- `lightFire('Logs')`: tinderbox useOn logs; XP → lit; CANT_LIGHT →
  blocked; timeout → stalled. `findBurnLane` does not appear as a JS
  table.
- First-family empty/stale/wrong-container and Off PeriodicBank tests
  stay green. `castOnNpc` / `buyById` / `bestPickaxe` still `not impl`.

## Live proof that could disprove this (later)

Design approval does not run this. Isolated local 274 and 289. Wait
`ingame && scene_state==2`. No public 289. FAIL + exit 1.

1. ChickenKiller melee default: combat XP, no bank. Mage: autocast armed
   then XP without packing elemental runes.
2. Alcher: High Alch XP + coins, noted and unnoted.
3. Superheater 100adccc Staff of fire. Superheater 8e7d965b: at least
   Staff of fire **and** one other FIRE_STAVES identity present in that
   content; Wield observed after close; bars produced.
4. BankFletcher 100adccc product-driven shafts. BankFletcher 8e7d965b
   `mode=cut+string` longbows. Banking.open arguments honored when the
   loc exists.
5. ShopBuyout: stock and inv move by requested qty.
6. NatureCrafter or BrimhavenAgility sell: inv down.
7. RuneCrafter or MuleCrafter trade: items leave one pack and appear in
   the partner's.
8. SmithingBot: anvil product, not chat make-menu.
9. Firemaker or CookBot Fire: Fire loc + Firemaking XP inside the plot.
10. GreenDragon: attackedByPlayer true only with a player face; Special
    spends energy.
11. Pause/Stop/reconnect during shop buy wait, makeX count wait, or
    lightFire wait: no late send.

Catalog API hashes are identical, so one API proof covers both catalogs.
Card-level 8e7d965b BankFletcher mode and Superheater staffs remain
separate rows.

## Distinctions

| Claim | This document |
|---|---|
| Ownership and task split are implementable | Yes — design approval |
| Mage/autocast/special/shop sell/anvil/fire-lane already correct | No |
| Melee setCombatStyle / NPC Attack / item-on-item / trade IDs posted | Present; still need live proof |
| First-family empty-bank / Withdraw-X / named open | Prerequisite, separate card |
| PeriodicBank / DeathRecovery / withdrawLoad / loadout | Separate provisioning design |
| 8e7d BankFletcher modes and Superheater staves may be dropped | No |
| Quest / clue / gatherer / market-maker / castOnLoc | Out of family |
| Source review of future patches | Later same-card `reviewer` |
| Local script → IPC → Rust → posted result, both revisions | Later live qualification |
| Campaign / 0.1.7 | Not accepted |

## Remaining product decisions

None outside authorized scope. Autocast/spec-bar/ssb/duel/shop-player
component ids must be read from each selected interface/content archive
at implementation; if 289 disagrees with 274, post the selected ids and
keep the operation. Do not shrink the enabled set or the 8e7d option
lists to avoid that check.
