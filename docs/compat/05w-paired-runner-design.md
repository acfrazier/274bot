# Remaining paired runner coverage

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 11:26 UTC. Kind: bounded read-only design for brief 111.
Not implementation, compilation, LIVE, fixtures, ledger, STATE, cache
cleanup, or a trade-runtime rewrite. Root owns acceptance, any LIVE
snapshot, serialized implementation insertion, and implementation
handoff. No routine reviewer card. Do not spawn implementation cards
from this run.

Read once: `AGENTS.md`, `docs/execution.md`, brief 111. Branch checked
first: `codex/rs2b0t-multirevision` (not `main`). Campaign HEAD at
write-up is `c5749bbe29eaa0ff313f25169e3d101865e217a9`. Client gitlink
`9d090ed04957e4efc254f073cda97bc5510ca72b`. Concurrent cook / combat /
tick / tools / special workers must not be touched. Work was read-only
except this report and `docs/compat/evidence/paired-runner-design/`.

## Verdict

**Three actual pair cells, one harness, no fake partner.** Solo Mule
Air does not qualify trade. Root912 Air both-started then died on
`Paint.buttons` — design 108 owns that seam; do not dim NatureCrafter
or treat the imported Runner as broken. Remaining paired work is
script-caused offer + two-stage accept, exact counterpart identity,
item conservation, then bank/deposit/restock/closed return and a
further exchange. Seeded first packs and first-transfer-only outcomes
are not full.

Reuse `paired_catalog_live.rs` / `support/paired_catalog.rs` and the
brief-103 shared Start barrier. Do not invent a second two-slot
runtime or edit `catalog_boundary_live.rs`. Do not clone foreign
`drivePartnerTrade.ts`. Do not rewrite `Trade`.

| Cell | Roles | Core loop | Blocker now | Full cycle also needs |
|---|---|---|---|---|
| NatureCrafter Air (exists) | Master + Runner | unnoted 1436 → Master, Air 556 + RC XP, Falador East restock, second transfer/craft | 108 `Paint.buttons` | gold window; already encoded |
| MuleCrafter Air pair (new) | Crafter + Mule | 1436 mule→crafter and crafted Air 556 crafter→mule at ruins, mule bank deposit, restock, further exchange | none on Trade/bank/inventory for Air | same 180s gold; bankFill=true |
| FlaxRunner (new) | Runner + Spinner | Pick 1779, meet (2719,3471,0), flax transfer, spinner Make-X 1777 + Crafting XP, Seers deposit, second delivery | missing `driveActivePartnerTrade` | Make-X `t_4b04cb5f` for conversion |

Nature island / Jiminua / `Shop.sell`, `stayInAltar`, Mule `bankFill=false`,
and non-Air Mule runes are extra gameplay or shop-owned, not this
minimum set.

## Hypotheses

| | Hypothesis | Status |
|---|---|---|
| (1) | Root912 Air FAIL means imported NatureCrafter is broken; dim it | **Rejected.** Tick 27 `not impl: Paint.buttons` after both-started. 108 owns the seam. |
| (2) | Solo Mule Air four-cell PASS qualifies mule_pair | **Rejected.** Solo never opens Trade. Ledger rows stay PARTIAL for paired trade. |
| (3) | Add a harness/fake-partner controller that trades for the catalog | **Rejected.** Brief forbids a hidden controller. Harness prepares before shared Start only. |
| (4) | Broad Trade rewrite / implement offer qty+id filter arity here | **Rejected.** Air already uses name-only `offerAll` because `shortRouteWithdraw` caps at 25. Seed unnoted-only. |
| (5) | Clone foreign `drivePartnerTrade.ts` or revive a JS trade guardian | **Rejected.** Ancillary stubs stay stubs. Flax needs a **thin host-owned driver** over existing Trade members, later, not a foreign controller copy. |
| (6) | Count seeded first 25 essence / first flax pack as restock | **Rejected.** 05r/05s already distinguish seed from script `BankRestock`. |
| (7) | Nature default island, stayInAltar, Mule Mind/Nature, Flax minFlaxCapacity as extra core cells | **Rejected** for this minimum. Air/Falador and Seers flax are the pair representatives. Extra branches are genuine extra gameplay after shop/Make-X. |
| (8) | Three pair cases on the current harness: keep Air, add Mule Air pair, add FlaxRunner after its missing op | **Accepted.** |

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `c5749bbe29eaa0ff313f25169e3d101865e217a9` |
| Client gitlink | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 111 SHA-256 | `0f1ff0dcde751da5782c045480430aebb8b7f8156a568eda8f93a6371c45606d` |
| Kanban card | `t_3caf0280` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| NatureCrafter.ts (both) | `025ac395b25d64ef818cc0321478f0a2c84a051b79f99decbbfec5a9a2f0812a` |
| NatureRunnerLogic.ts (both) | `7a81b75a4cc4fde41d12565f5fe999de7931d88f2529da6d2d5e6a31f82d82f0` |
| MuleCrafter.ts (both) | `bf745db4c0a3df22406b49c8a8b716b10e0594302853d80ca6f38862d05dc1f9` |
| MuleCrafterLogic.ts (both) | `d9cc408c1857a02332e3338e1ae1e956dea51c7c191d4a66af81be6e5108251a` |
| FlaxRunner.ts (both) | `6edae2ae773b73b907b5a3d4c020052075f7b32f9c2872ca78246eead9dfff34` |
| FlaxRunnerLogic.ts (both) | `05a07311383d8ab8121b45e0dcf56a3eefef27d94d8c23565d80fec2462cc32e` |
| runeCraftLocations.ts (both) | `40db500c2c081a0978738796fcce62965bd39ac70d0995345e15443d4042f221` |
| Foreign Trade.ts (both) | `b0985721491802d734a2044c07b1d39a560254764c7a70b508a0deb1e678efa5` |
| Foreign drivePartnerTrade.ts (both) | `bfb144d6d8ededf87f96d98276d3e7b43950ccf9d06bea7b55ab98370c8de9f9` |
| Native `trade.js` | `fff835ca3b7c3316be66d3a7016c9223bf4f100cd72d8efcbe2cbbaf7b040e6c` |
| Native `drive_partner_trade.js` | `7cf38227ca410a4accd0e4579e95f2490fa5f97cb5a26b7ff954fd121ac00c76` |
| Native `shop.js` | `0c532288168fb7ede92258d2da7f6f4ce158d5ea4bfe9eba1394a313b13ac682` |
| Native `chat_dialog.js` | `a572b5f8ca12ce4df9903bec1e49286c1eeec1ca74cf9c328879089cf118b6ec` |
| r289 Air Root912 log | `d8646933d4e6165fee74b7baee3047daf8558106a3ea034158819cd05991ce1a` |

All six script files plus `runeCraftLocations.ts` are byte-identical on
both frozen catalogs. Machine-readable copies:
`evidence/paired-runner-design/{refs,hypotheses,settings}.json`.

## 1. Frozen modes (not a catalog rewrite)

### NatureCrafter

Settings: `rune` (`Nature runes` default, `Air runes`), `mode`
(`Master` default / `Runner`), `partner` (required; empty throws),
`bankEvery` default 60, `withdrawEss` default 0, `withdrawCoins`
default 10000, `stayInAltar` default false. No Solo.

Air: ruins `(2983,3288,0)`, both banks Falador East `(3013,3355,0)`,
`unnote=null`, TRADE_CAP 25, unnoted essence 1436, Air 556, talisman
1438, RC 1. Runner `Bank.openBooth` on the hardcoded tile, not
`BANK_LOCATIONS`. DriveTrade offers unnoted 1436 (`offerAll` when
held ≤ 25, else `offer` qty) then both accept screens.

Nature: Ardougne East `(2655,3283,0)`, Jiminua `(2767,3122,0)`,
Captain Barnaby ship, Shilo master bank commented quest-gated.
`UnNoteEssence` calls `Shop.sell` / `Shop.buy`. Native `Shop.sell`
throws. Air is the trade-only representative.

Runner `onPaint` calls `p.buttons([{id:'gobank',...}])`. Master does
not. Root912: shared Start succeeded; runner tick 27 `not impl:
Paint.buttons`. 108 maps that. Do not auto-click `gobank` in the
harness.

Master `chat.message` TRADEREQ filter wants `e.type` and `e.username`.
Host `chat.message` currently posts `{ text }`. That degrades
multi-runner ask tracking. Air 1:1 still has `AcceptRunner` /
`DeliverEssence` `Trade.request` on the visible partner. Do not block
Air on chat TRADEREQ. Do not expand chat IPC here.

`bankEvery=60` will not fire inside 180s. Do not wait for Shilo.

### MuleCrafter

Settings: `rune` default `'Air rune'` (singular) from
`runeCraftLocations` (Air/Mind/Water/Earth/Fire/Body/Cosmic/Chaos/Nature/Law/Death),
`mode` `Crafter` default / `Mule`, `partner` optional for Crafter
(blank = solo, forces `bankFill`), required for Mule (throws),
`bankFill` default true.

Solo Crafter Air is the Root9cc four-cell PASS. It never qualifies
trade. Mule mode walks Falador East via `bankTile('Falador East')`
(`BANK_LOCATIONS`), withdraws unnoted 1436 cap 27, `Trade.request` at
ruins, `offerAll('Rune essence', id===1436)`, two-stage accept.
Crafter crafts, exits portal, offers non-talisman (the Air runes)
when `classifyMuleState` sees essence, accepts, later banks if
`bankFill`.

`essCount()` uses `reader.inventory()` filtered by id 1436. That
reader is now mapped; solo PASS used it. Pair still needs Trade.

### FlaxRunner

Settings: `mode` default `Runner` / `Spinner`, `partner`,
`minFlaxCapacity` default 24. Two profiles required. Same Seers tiles
as FlaxAIO: field `(2741,3444,0)`, meet `(2719,3471,0)` TRADE_RANGE 2,
bank stand `(2725,3493,0)`, wheel `(2711,3471,1)`.

Runner: Pick flax 1779 → full pack → meet → `Trade.request` →
`driveActivePartnerTrade({role:'giver', productNamesToOffer:['Flax']})`.
Spinner: empty pack at meet → request →
`driveActivePartnerTrade({role:'receiver'})` → climb →
`ChatDialog.makeX('Flax', flaxCount())` → deposit all at Seers →
return.

Native `drive_partner_trade.js`: if `Trade.active()` then
`throw notImpl('driveActivePartnerTrade')`. The first real trade
window kills both sides. That is a **missing operation**, not a
broken imported script, not a fixture gap, not a Trade rewrite.

Flax does not call `Paint.buttons`. `walkOpening` is already a
`Traversal.walkResilient` map; `towardDest` / `isOpenableObstacle`
remain unused stubs. `Reachability.canReach` / `canStep` read posted
`snap().reach`. `Bank.depositInventory` maps. `Inventory.isFull` /
`free` map. `Game.animating` maps.

FlaxAIO 98 is a **different card** (one actor pick+spin). Do not
substitute concatenated FlaxPicker/FlaxSpinner or FlaxAIO for this
pair.

## 2. Equivalence vs extra vs ancillary

**Pair roles (not independent options).** Nature Master/Runner, Mule
Crafter/Mule, Flax Runner/Spinner. Each cell is two minted actors
with exact counterpart names. Both-Runner or Mule-without-partner is
a fixture miss.

**Equivalence groups (one representative).** Mule Air vs other F2P
runes (Mind/Water/Earth/Fire/Body) is the same Trade+altar+named-bank
loop at different tiles/levels. Air is the pair representative. Do
not also run Mule Nature (NatureCrafter owns ship/Jiminua). Members
Cosmic/Chaos/Law/Death are content-risk extras, not core.

**Not equivalent.** Nature Air vs Nature island. Shop/ship/unnote vs
Falador short hop. Air does not accept Nature.

**Genuine extra gameplay (existing schema; later cells, not this
minimum).** Nature `stayInAltar=true` (trade inside temple; runner
needs talisman). Mule `bankFill=false` (mules bring all essence).
Nature default island after shop. No new native/TUI controls. Mode,
partner, rune, stayInAltar, bankFill already exist on the frozen
schema.

**Ancillary, do not vary.** Nature `bankEvery` / `withdrawEss` /
`withdrawCoins` (Air unused coins; 0 withdraw = TRADE_CAP). Flax
`minFlaxCapacity` (empty-pack core never junk-banks). Do not revive
deferred foreign guardians. `towardDest` / `isOpenableObstacle` stay
stubs.

## 3. Host gates (no Trade rewrite)

Nature Air (already in `air_operation_gates`): `Trade.request` mapped
to `{ op:'player', name, action:'Trade' }`. `accept`/`decline` mapped
to posted ids; throw if id `< 0`. `offerAll`/`offer` extra arity
ignored; Air cap 25 uses name-only. `Bank.openBooth` Falador East
hardcoded. Shop/named-bank/teleport unused by Air.

Mule Air pair: same Trade family. `Bank.openBooth` after
`bankTile('Falador East')` — named-bank already posted. `reader.inventory`
mapped. Seed **unnoted 1436 only** so ignored id-filter cannot offer
noted 1437. `Trade.request` does not return a boolean (Mule logs it);
not a blocker. `bankFill=true`.

Flax: `Trade.request` mapped, but the open window is driven by
**missing** `driveActivePartnerTrade`. Later smallest map (not this
card): host-owned driver over existing `Trade.active` / offer /
accept / decline / partner / myOffer / theirOffer matching the frozen
Flax call shape (`role`, `partners`, `productNamesToOffer`,
`inventoryMetric`, `onComplete`, `onDecline`, `onMissingPartner`).
Do not copy foreign `drivePartnerTrade.ts`. Do not implement offer
qty/filter arity. Do not have the harness click trade. Spinner
conversion is `ChatDialog.makeX`; `makeX` throws when no qty `-1`
button — owned by Make-X `t_4b04cb5f`. A first flax transfer without
spin is an explicit **partial** only.

Nature island: `Shop.sell` throws. Shop work owns that. Unused by Air.

Do not propose posted-id Trade as insufficient. Two-stage accept with
posted `trade_accept_id` is the preserved contract.

## 4. Minimum coherent fixtures

Reuse the current ignored LIVE pair harness. Same env:
`PAIRED_CATALOG_REVISION` / `NAV_PACK` / `ROOT` / `COMMIT`. Same
180s prep, 180s gold, 150 watch ticks. One shared observation window
after both `ingame && scene_state==2`. Shared Start exactly once
after both prepared (05s). Relog admission unchanged. Harness may
teleport/seed/ack bank **before** Start only. No post-Start
loot/coins/teleports or unsupported player aliases.

### A. NatureCrafter Air — keep, do not replace

Already encoded. Prep: runner Falador East `givebank blankrune 200`
open+loaded+close+generation, then firstload 25 unnoted at ruins;
master talisman, no essence. Settings `rune=Air runes`, exact minted
names, `stayInAltar=false`. Witnesses already distinguish
`FirstTransferCraft` vs `BankReturnSecondCycle`. Seed 25 is not
restock. After 108, root LIVE may PASS partial or full; do not
reclassify partial as full. Do not auto-click `gobank`.

### B. MuleCrafter Air pair — new `PairCase::Mule`

Two profiles, same catalog card. Crafter: `mode=Crafter`,
`rune='Air rune'`, `partner`=mule IGN, `bankFill=true`, RC 1, Air
talisman 1438, no essence, at ruins. Mule: `mode=Mule`, same rune,
`partner`=crafter IGN, empty pack except after prep. Prep mule like
Air runner: Falador East booth ack `givebank` unnoted 1436 ×200,
close/generation, then firstload 27 (TRADE_CAP) at ruins. Do not
seed Air 556 or RC XP. Do not seed noted 1437.

Partial (`FirstExchangeCraft`): both offer+confirm with exact
counterpart; 1436 left the mule; crafter 556 ≥ 1 and RC XP ≥ 1 from
script; conservation of transferred 1436 (crafter in == mule out, or
crafter already converted to 556). Seeded runes fail.

Full (`MuleBankReturnSecondCycle`): mule opened loaded Falador East
after the first exchange, deposited received 556, withdrew a **new**
1436 load (pack refill after min), returned to ruins, second
transfer, further crafter craft. First 27 is seed. If gold expires
after partial, print partial — do not fake full.

Offline witnesses: empty mule partner (throws — fixture, not this
LIVE), both Crafter, wrong partner, one-sided confirm, seed-only 556,
stale open trade after claimed craft, missing conservation, no mule
bank restock, no second transfer.

### C. FlaxRunner — new `PairCase::Flax` after missing op

Runner: `mode=Runner`, `partner`=spinner IGN, field `(2741,3444,0)`,
empty pack, `minFlaxCapacity=24`. Spinner: `mode=Spinner`,
`partner`=runner IGN, wheel house / meet, empty pack, Crafting 1.
No bank seed required for core pick (runner gathers). Spinner starts
empty so `spinnerReadyForHandoff`. No seeded 1777.

Partial (`FirstFlaxTransfer`): both offer+confirm with exact
counterpart; 1779 left the runner and appeared on the spinner.
`Trade.request` without `active()` fails. This partial is only
honest **after** `driveActivePartnerTrade` no longer throws.

Full (`SpinBankSecondDelivery`): spinner 1777 ≥ 1 with Crafting XP ≥
1 from Make-X (not seed), Seers booth deposit of strings, closed
return to meet, runner second pick+delivery. Full waits on Make-X.
If Make-X is not posted, keep the cell partial or unrun — do not
click a guessed comId.

Do not start both as Runner. Do not run FlaxAIO here.

## 5. Sequencing

1. **Paint 108** — Nature Air cannot survive runner `onPaint` until
   `Paint.buttons` exists. Headless returns null and still records
   descriptors; do not fabricate `gobank`.
2. **Root re-run existing Air** — first-transfer then full if 180s
   allows. This is the Trade-family proof Mule pair reuses.
3. **Tick 106** — Duel `addTickListener` only. Nature/Mule/Flax do
   not subscribe. Do not wait on 106 for these three cards.
4. **Mule Air pair fixtures** — after Air first-transfer is a real
   Trade path (or in parallel as offline witnesses only). LIVE after
   Air Trade is observed, not after solo Mule PASS.
5. **Thin `driveActivePartnerTrade`** — Flax-shaped host driver over
   existing Trade. Separate later card, not a Trade rewrite, not
   foreign file copy. Files: `crates/script/src/shim/drive_partner_trade.js`
   plus a new focused `crates/script/tests/drive_partner_trade.rs`.
   Do not pile `load_isolate.rs`. Do not edit catalog sources.
6. **Flax pair fixtures** — after (5). Conversion/full cycle after
   Make-X `t_4b04cb5f`. Shop is unused by Flax.
7. **Nature island / stayInAltar / Mule bankFill=false** — after
   shop and after Air full cycle. Not this minimum.

Unchanged timeouts, generation, memory constraints, guardian
lifecycle. No post-Start cheats. FAIL + exit 1. PASS only the named
witness.

## 6. Exact files (later implementers)

**Pair fixtures (reuse, do not fork):**

- `crates/host-play/tests/paired_catalog_live.rs`
- `crates/host-play/tests/support/paired_catalog.rs`
- unique evidence under `docs/compat/evidence/paired-catalog-fixtures/`
- report owned by that fixture card, not this design

Add `PairCase::{Mule,Flax}`, role enums, operation gates, conservation
predicates, prep sequences (Falador East ack for Mule; no bank ack
required for Flax pick). Keep Duel and Air. Do not edit
`catalog_boundary_live.rs`, `support/two_slot_isolation.rs`, scenario
`lib.rs`, API, nav, client, engine, or catalog TS.

**Flax missing op (later, after this design):**
`drive_partner_trade.js` + new focused test file only.

**Hotspot:** `paired_catalog.rs` / `paired_catalog_live.rs` already
owned by Air/Duel/103. Serialize fixture edits. Do not also drop
Flax/Mule prep into `catalog_boundary_live.rs`.

Focused offline tests: frozen hashes on both catalogs, wrong partner,
one-sided confirmation, seed-only inventory/XP, missing conservation,
no restock, no second transfer, shared Start / WaitAck / stale relog
already present — extend them per new case. `rustfmt` + `cargo test
-p host-play --test paired_catalog_live` + strict Clippy on that
test. No LIVE from the fixture worker. Root owns both-rev/catalog
LIVE.

## 7. Falsifiers

A later implementation is wrong if any of these hold:

- Solo Mule PASS cited as mule_pair acceptance.
- NatureCrafter dimmed for `Paint.buttons`.
- Harness performs Trade/craft/spin after Start, or a fake partner
  script replaces a role.
- Seeded 25/27 essence or seeded flax/strings/Air 556 counted as
  script restock or conversion.
- First-transfer printed as full cycle.
- Nature island accepted as Air, or Mule Nature substituted for
  NatureCrafter.
- Foreign `drivePartnerTrade.ts` copied, or Trade.offer arity
  rewritten as part of these fixtures.
- `Paint.buttons` auto-clicked `gobank` to force a bank trip.
- Gold/watch/prep constants changed.
- `catalog_boundary_live.rs` or engine/client/nav edited for this.
- Post-Start teleports, givebank, or player aliases.

Root inserts fixture/mapping work after this design. Not LIVE
acceptance.
