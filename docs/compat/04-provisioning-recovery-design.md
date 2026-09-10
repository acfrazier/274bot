# Step-6 provisioning and recovery capability design

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-10. Kind: bounded pre-implementation design/audit of
`docs/compat/briefs/20-provisioning-recovery-design.md` against frozen
enabled callers and current host shims. Not runtime evidence, not live
acceptance, not first-family banking source review, not campaign release.

Read once: `AGENTS.md`, `docs/execution.md`, fail-closed-dispatch, plan
steps 6–7 in `docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md`,
`docs/compat/support-matrix.json`, `docs/compat/04-banking-design.md`,
brief 18, and the named brief. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Work was read-only except this
report. No product edits, fixtures, STATE, matrix, subagents, commit,
stash, reset, restore, checkout, merge, remotes, live sessions, or suite
reruns. Concurrent first-family bank WIP was inspected read-only and is
labeled below; it is not accepted product.

## Verdict

**Proceed to scoped provisioning/recovery implementation after the first
banking family is reviewed**, under the constraints below. Design
approval is not source acceptance and not live acceptance.

Enabled cards already construct PeriodicBank / DeathRecovery and select
loadouts. Construction with `bankStrategy=Off` is harmless. Every other
named option in this family currently no-ops or throws `not impl`. That
is an honest ledger BLOCKED, not permission to shrink the enabled set.

Do not clone `PeriodicBank.ts`, `DeathRecovery.ts`, `Banking.bankNearest`,
`AcquireTask`, `LoadoutPanel`, or a universal job framework. Reuse Rust
navigation, the pending first banking family (fresh snapshot / Withdraw-X
/ named booth open), existing deposit/wear/walk, and posted chat. Scripts
load and start irrespective of revision; missing operations refuse at
host/client boundaries.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `71a5ac0adc9792a29c86c62af71dc9bf69436ebd` |
| Client pin (submodule HEAD) | `6cb5a0b17aeef74da6b57b205e916681daee4f76` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief SHA-256 | `1c2055d9b65b3023d4ff27a79545d36eb2f8a75b13ecbe9217c8ed2ad166b12c` |
| First-family design SHA-256 | `09e924b847ab07dafa5af4f1fdad7ecf917ef0f05cb2236909ae83ff53edfce2` |
| Audit SHA-256 | `a954c7bf50d1bcbc3d0d245f291348dad6bd9b7d3a4f48ce2528de4d3601385f` |
| Catalog roots | `.superpowers/inputs/rs2b0t-100adccc037d9f6898080e1cad58fcfc43364775` and `…/rs2b0t-8e7d965be2071d6ec65c3265e12af797082d720a` |
| PeriodicBank.ts / DeathRecovery.ts / bankRules.ts / loadoutPlan.ts | identical SHA across both catalogs (`5c20fc2f…`, `33106a67…`, `bd9b6d08…`, `3406fee1…`) |
| Kanban card | `t_23695d9d` run 1118 |

Current bank WIP (uncommitted, Sol `t_e52e0a03`; **not** this report's
product): `crates/api/src/snapshot.rs` already consults client
`inventory_packet_state`; worktree `bank.js` still aliases
`snapshotReady()` to `ready()` (`isOpen && loaded`) and still throws
`Bank.withdrawLoad`. Root integrates that family's reviewed commit
separately. This design assumes the first-family contracts in
`04-banking-design.md`, not the unfinished alias.

## Caller / capability / proof table

Catalog sources for these APIs are identical. Enabled set only.

| Caller | Required options | Harmless disabled | Missing host capability | First live proof |
|---|---|---|---|---|
| ChickenKiller | `bankStrategy` Off/Loot count/Time/Either; `bankEveryItems`/`Minutes`; PeriodicBank deposit-except-keep + `afterDeposit` ammo/rune restock; DeathRecovery radius 3 | Off (default): constructor + `validate===false` is correct | PeriodicBank execute; death detect + walk-back | Off: combat/loot/bury with **no** bank trip. Loot count: deposit keep-list remainder, restock, return, further XP |
| RockCrab | PeriodicBank + `bankCommonJunk` (default true) + optional `bankTile` destination; DeathRecovery radius 4; loadout food | Off | PeriodicBank; common-loot OR into deposit; named stand when `bankTile` set; death walk-back; loadout food withdraw | `bankTile` walk-near + booth open; common junk deposited iff setting true |
| ArdyFighter | PeriodicBank loot-name deposit + `commonJunk`; DeathRecovery radius 6; loadout **gear only** (food ignored; eats stolen cakes) | Off | PeriodicBank; common-loot deposit; death walk-back; gear wear/withdraw | Loot count banks stolen loot + gems; return to market; death returns to `ANCHOR` |
| ArdyThiever | same PeriodicBank/commonJunk; DeathRecovery; `depositMatcher(..., BANK_COMMON)` on BankRun | Off | same as ArdyFighter | `bankCommonJunk=false` deposits only named loot |
| ArdyCakes | DeathRecovery radius 6; Bank.openBooth / depositInventory | n/a (no Off) | death detect + walk-back | death → stall `STAND` |
| AutoFighter | DeathRecovery; `scriptFood` + `foodWithdraw` | blank loadout uses Trout fallback | death walk-back; food name from loadout carry | named loadout food withdrawn to `foodWithdraw` |
| HillGiant | DeathRecovery; loadout food; `weapon` string; `bankCommonJunk` pickup | blank weapon = already-worn | death; food/weapon withdraw+wear; common-loot **pickup** | `bankCommonJunk=false` does not pick gems |
| MossGiant / FireGiant / GreenDragon | DeathRecovery; loadout food; `matchesCommonBankLoot` on ground loot; GreenDragon also `suppliesOf` potions | `bankCommonJunk=false` skips common pickup | common-loot matcher; death; food; GreenDragon carry qty | GreenDragon death anchors at `BANK_TILE`, not the field |
| WildyAgility | DeathRecovery with **`walkBack`**; loadout food; `foodWithdraw` after death | `minFood=0` skips startup food check | death + script `walkBack` (food-only bank then ridge) | death → bank food → `RIDGE_APPROACH`; queued walk is not recovery |
| ChaosDruidKiller / Thiever / HerbloreSecondaries / BrimhavenAgility | loadout food (+ gear for some) | blank loadout | `scriptFood` already maps; `gearOf`/`suppliesOf`/`weaponOf` still throw | food fallback vs named loadout |
| BankFletcher / CookBot / SmithingBot | `Bank.withdrawLoad(name)` fill | n/a | `withdrawLoad` | one Withdraw-All or one Withdraw-X(free slots); inv used increases or pack full or bank count 0 |

Dim `RoguesPurse` / import-blocked `BrimhavenMossGiants` / quest
`gearOf`/`weaponOf` callers stay deferred. Clue `SolveClue` stays
pre-Start refusal, not this family.

## Exact contracts (callers, not planner bodies)

### Common-loot matching

Frozen `bankRules.ts` (both catalogs):

| Fact | Contract |
|---|---|
| `COMMON_BANK_LOOT` | `uncut`, `sapphire`, `emerald`, `ruby`, `diamond`, `opal`, `jade`, `topaz`, `strange fruit`, `beer`, `kebab` (substring, lowercased name) |
| `RANDOM_EVENT_CASKET_ID` | `405` |
| `matchesCommonBankLoot(name, id=-1)` | `id===405` **or** name contains a list entry |
| `depositMatcher(own, includeCommon)` | `own(name) \|\| (includeCommon && matchesCommonBankLoot(name, id))` |
| `PERIODIC_BANK_SETTINGS.bankCommonJunk` | boolean, default **true** |

Shim today: `COMMON_BANK_LOOT = []`, casket id `-1`,
`matchesCommonBankLoot` throws, `depositMatcher(..., true)` throws
`depositMatcher.includeCommon`. `bankCommonJunk=false` already works
because the includeCommon arm is skipped.

Pickup callers (MossGiant / FireGiant / GreenDragon / HillGiant) OR the
matcher into the loot set when the setting is true. Deposit callers
(ArdyFighter / ArdyThiever / RockCrab PeriodicBank, ArdyThiever BankRun)
pass `commonJunk: () => BANK_COMMON`. ChickenKiller does **not** pass
`commonJunk`; its deposit is `depositAllExcept(keepList())`, so the
foreign `?? true` default is redundant there.

Host owns the predicate. Do not restore the JS table. Verify obj id 405
against both content identities before baking it; a missing 289 id is
recorded evidence, not a silent drop of casket matching. Posted bank and
ground rows already carry `name` and often `id`.

### Loadout gear / supplies / weapon

Frozen `Loadout` is `{ name, worn: Partial<Record<Slot,string>>, carry: {item, qty}[] }`
with slots `hat back front righthand torso lefthand legs hands feet ring quiver`.

| Accessor | Contract |
|---|---|
| `selectedLoadout(bag)` | case-insensitive name, else first, else null |
| `foodOf` | first carry whose content record is `consumable==='eat'`, else fallback |
| `scriptFood(bag, fallback)` | `foodOf(selectedLoadout(bag), fallback)` |
| `gearOf` | `Object.values(worn)` |
| `suppliesOf` | copy of `carry` including **qty** |
| `weaponOf` | `worn.righthand ?? fallback` |

Host today: persist `{ name, worn: Vec<String>, carry: Vec<String> }`.
`selected_compat_loadout` wraps carry as `{item}` and leaves `worn` as a
JSON array, so `weaponOf(...righthand)` cannot work and qty is lost.
`__rs2b0t_food_of` already walks `carry[].item` against
`api::content::FOOD_HEALS` (the eatable subset). That food path stays.
`gearOf` / `suppliesOf` / `weaponOf` throw `not impl`.

Required store change (host, not JS panel clone): persist slot-keyed
`worn` and `{item, qty}` carry. Blank loadout remains "use script
fallback". ArdyFighter help text: loadout food is ignored.

Provisioning is host withdraw + wear after a **fresh** bank snapshot.
Do not invent a loadout planner. Script still decides *when* to bank;
host resolves names to obj ids through existing content/snapshot rows.

### PeriodicBank

Frozen options: `strategy`, `itemsThreshold`, `minutesThreshold`,
`countLoot`, `deposit`, optional `afterDeposit`, `destination`,
`commonJunk` (default true in `bankNearest`), `returnTo`, `setStatus`,
`log`.

Foreign `validate`: false when strategy `off`, in combat, or inside a
**3 minute** failure backoff; else `shouldBankNow` (`items` / `time` /
`either`, and `lootCount > 0`). Foreign `execute` calls
`Banking.bankNearest` then walk `returnTo` radius 6 timeout **120_000 ms**.
Do not copy that router or raise walk timeouts. Use existing **60_000 ms**
traversal bound from the first family.

Shim `parseBankStrategy('Loot count')` returns `'loot'`; foreign returns
`'items'`. Host must parse the **labels** `Off` / `Loot count` / `Time` /
`Either` itself. Do not reuse the shim token.

`afterDeposit` stays a script callback (ChickenKiller restock ammo/runes
via existing withdraw). Destination, when supplied (RockCrab `bankTile`),
is walk-near that stand then named booth open from family 1 — not a
fallback to a different loc.

### DeathRecovery

Frozen options: `anchor`, `radius` (default 6; ChickenKiller 3, RockCrab 4),
`needs?`, `onDeath?`, `onRecovered?`, `walkBack?`.

No enabled card passes `needs`. Do not implement `AcquireTask`. Dim
RoguesPurse `walkBack` stays deferred. WildyAgility **does** pass
`walkBack` (`recoverAndReturn`: food-only bank then ridge) — required.

Death detect: `/oh dear.*you are dead/i` on game chat. Client
`MESSAGE_GAME` already fills `chat_text[0]` / `chat_lines`; isolate
`chat.message` fires on `chat_text` change. That is the observation
seam. Host latches death from a **new** posted line, not from a queued
script flag. `validate` is false until death; once at anchor within
radius and (no needs), it clears and calls `onRecovered`.

Foreign `execute`: wait `ingame && tile` **20_000 ms**, then **3 ticks**,
then `walkBack` or `Traversal.walkResilient(anchor, radius)`. Preserve
those bounds. Pause freezes the clock; Stop / isolate drop /
`reset_session` / logout abort and drop the latch.

### withdrawLoad

Frozen: backpack must be bank-ready; find bank row by name; if
Withdraw-All op exists, send it and wait **4000 ms** until
`Inventory.used` grew **or** pack full **or** bank count 0; else
`withdrawX(name, Inventory.free())`; free 0 is success without a send.
Depends on first-family snapshotReady + Withdraw-X. Shim throws.

## Ownership split

```
client  existing per-component inv packet state (first family)
        MESSAGE_GAME → chat ring (already)

host    matches_common_bank_loot(name, id)
        loadout store: slot worn + {item,qty} carry
        food/gear/supplies/weapon accessors
        withdrawLoad pending (All or X)
        PendingPeriodicBank: due? → walk/open/deposit/afterDeposit/return
        PendingDeathRecovery: chat latch → wait respawn → walkBack or walk
        Pause/Stop/reset/reconnect abort

shim    marshal settings and callbacks
        PeriodicBank/DeathRecovery construction ABI
        throw not impl for destination/NPC/obstacles still out of family
```

Fail-closed: missing facts stay `not impl` or `validate===false` only
for **Off**. Non-Off PeriodicBank and all DeathRecovery must not stay
silent no-ops.

## Recommended task order

Do **not** start these until first-family source is reviewed
(`t_e52e0a03` / brief 18). Root still owns live 274/289 bank fixtures.

1. **Common-loot predicate** — smallest. Host `matches_common_bank_loot`
   + shim `matchesCommonBankLoot` / `depositMatcher.includeCommon` /
   nonempty `COMMON_BANK_LOOT` observation. No planner. Unblocks
   `bankCommonJunk=true` pickup and deposit. Independent of PeriodicBank
   execute.
2. **withdrawLoad** — host-owned fill using family-1 snapshotReady and
   Withdraw-All / Withdraw-X. BankFletcher / CookBot / SmithingBot
   restock. Do not fold into PeriodicBank.
3. **Loadout slot/qty + accessors + provision** — persist foreign shape
   in the host store; implement `gearOf` / `suppliesOf` / `weaponOf`;
   wear missing gear and withdraw carry qtys after a fresh open. Food
   keeps `FOOD_HEALS`. No quest loadout engine.
4. **PeriodicBank pending op** — after 1 and family 1. Host evaluates
   Off/items/time/either from posted loot count + elapsed minutes.
   Sequence: walk (optional destination tile) → named open → wait
   snapshotReady → `depositAllMatching` **settled** → script
   `afterDeposit` → close → walk `returnTo` radius 6. 3 min backoff on
   false. Combat suppresses. Do not extend `PendingBankFetch` (Open still
   pops on nonempty `bank_loaded` in committed `step_bank_fetch_on_bot`).
5. **DeathRecovery pending op** — after walk + chat publication. Host
   latches on the death line, waits 20 s + 3 ticks, then script
   `walkBack` if provided else walk-to-anchor. `onDeath` / `onRecovered`
   remain JS. No `needs` / AcquireTask in this family.

Each task is one implementer card, same-card `reviewer`, then stop.
Do not merge 4 and 5.

## API shape (append only)

No new isolate opcode. Reuse Interact `open-booth`, `walk-near`,
`deposit`, `withdraw` / `withdraw-x`, `wear`, `close`.

| Addition | Where | Notes |
|---|---|---|
| `matches_common_bank_loot` | host content/interact helper, thin JS | names + verified casket id |
| loadout JSON | `crates/script/src/loadouts_store.rs` | slot map + qty; migrate existing flat files as "unknown slot" names into carry/gear lists without dropping items |
| `withdraw-load` | existing Interact name field | one pending latch, 4000 ms settle |
| `bank_snapshot_ready` / generation | first family | PeriodicBank and withdrawLoad wait on it |
| posted `player_dead` (bool) | Snapshot append after `shop_stock` | optional; pending op may read host snapshot directly. Do not repurpose `chat_text` |

`PendingBankFetch` stays the nav BankBudget path. PeriodicBank /
DeathRecovery / withdrawLoad are separate optional latches on the script
slot, pumped from the same observe path as `dispatch_script_interact`.
One latch of each kind. Pause freezes clocks and does not send. Stop /
reset / reconnect abort false.

## Existing timeouts (do not change)

| Bound | Value |
|---|---|
| Walk / walk-near | 60_000 ms |
| Bank waitReady / withdraw settle / withdrawLoad settle | 4_000 ms |
| Count-dialog wait | 3_000 ms |
| Death respawn wait | 20_000 ms |
| Death extra delay | 3 ticks |
| PeriodicBank failure backoff | 180_000 ms |
| Isolate RUNTIME / JOIN / SLOW_TICK | 50 ms / 2 s / 50 ms |
| FRAME / IDLE_PARK | 20 ms / 600 ms |

Foreign PeriodicBank return timeout 120 s is **not** adopted.

## Meaningful tests (source; no live run here)

Must traverse script call → IPC → Rust send/pending → posted result.
Must not snapshot source text.

- `matchesCommonBankLoot('Uncut sapphire')` true; `'Rune scimitar'` false;
  id 405 true iff content evidence; `depositMatcher(own, false)` never
  calls common; `includeCommon=true` ORs it.
- `bankStrategy=Off`: PeriodicBank.validate false; **zero** open/deposit
  sends. `Loot count` with lootCount≥threshold, not in combat: open +
  settled deposit + return walk. In combat: false. Failed open: backoff,
  no retry-spam inside 3 min.
- `bankCommonJunk=false`: gem name is not deposited/picked. true: it is.
- withdrawLoad: All-op present → one Withdraw-All → used↑ or full or
  bank 0. No All-op → one Withdraw-X(free). free 0 → true, no send.
  Stale bank → false, no send.
- `weaponOf` reads `worn.righthand`; `suppliesOf` preserves qty;
  `gearOf` lists every named slot; blank loadout food fallback.
  Wear/withdraw happen only after snapshotReady.
- Death: posted `Oh dear, you are dead!` latches; wait bounds; walk to
  anchor within radius; `onRecovered` once. Duplicate chat does not
  re-latch. WildyAgility `walkBack` is invoked instead of default walk.
  Pause during wait does not walk; Stop/reconnect drop the latch.
- First-family empty/stale/wrong-container tests stay green.
- `needs` / NPC `openNpcAccess` / obstacles / clue still `not impl` or
  pre-Start refusal.

## Live proof that could disprove this (later)

Design approval does not run this. Isolated local 274 and 289. Wait
`ingame && scene_state==2`. No public 289. FAIL + exit 1.

1. ChickenKiller default Off: combat/loot progress, **no** booth open.
2. ChickenKiller `Loot count` + small `bankEveryItems`: script-caused
   deposit of non-keep items, ammo/rune restock if ranged/mage, return,
   further XP. Queued open without snapshotReady fails the cell.
3. ArdyThiever or RockCrab `bankCommonJunk` true vs false: gem/casket
   present only on true.
4. BankFletcher `withdrawLoad`: pack fills from one named log/bar.
5. Named loadout: food and righthand appear in inv/worn after provision.
6. Forced death (local fixture): chat line → respawn wait → tile within
   radius of the card's anchor (GreenDragon: bank tile). WildyAgility:
   food withdrawn then ridge approach. Seeded position is not recovery.
7. Pause/Stop/reconnect during PeriodicBank deposit wait or death wait:
   no late send.

Both catalog hashes of these API files are identical, so one source
proof covers both catalogs. Card-level 8e7d965b BankFletcher mode and
Superheater staff alternatives remain first-family / production proofs.

## Distinctions

| Claim | This document |
|---|---|
| Ownership and task split are implementable | Yes — design approval |
| PeriodicBank / DeathRecovery / common-loot / loadout already correct | No |
| First-family empty-bank / Withdraw-X / named open | Prerequisite, separate card |
| Off PeriodicBank may remain idle | Yes |
| Non-Off PeriodicBank or DeathRecovery may remain idle | No |
| Quest / clue / gatherer / market-maker / NPC banker / AcquireTask | Out of family |
| Source review of future patches | Later same-card `reviewer` |
| Local script → IPC → Rust → posted result, both revisions | Later live qualification |
| Campaign / 0.1.7 | Not accepted |

## Remaining product decisions

None outside authorized scope. Casket id 405 needs content-table
evidence at implementation time; if 289 disagrees, record the id and
keep name-list matching. Loadout file migration is in-scope host store
work, not an operator fork.
