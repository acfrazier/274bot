# Isolated Air RuneCrafter bank-return deadline (frozen 6750713b)

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 08:01 UTC. Kind: bounded read-only architecture/fidelity
audit for brief 84. Not implementation, LIVE, fixtures, ledger, STATE, dim,
timeout, or a coordinate exception. Root owns acceptance and any follow-up
diagnostic or repair.

Read once: `AGENTS.md`, `docs/execution.md`, brief 84. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Work was read-only except this
report and `docs/compat/evidence/rune-air-deadline/`. Frozen source is
`.superpowers/review-exports/catalog-root-6750713b`, not the concurrent
working tree. Shared scenario/catalog files belong to Ardy `t_464d9c4c`.
`bank.js` belongs to named-approach `t_4948af53`. Original four Air receipts
were not copied or edited.

## Verdict

**The 274/newer Air FAIL is not a foreign source regression, not a missing-op,
and not an `openNearest` / 04k / 04m defect.** Do not dim `rune_crafter`. Do
not lengthen `SCRIPT_GOLD_DEADLINE`. Do not add a Falador East coordinate
exception. Do not treat a lucky rerun as proof.

The serial scenario died on pack-empty (`has_item_id(556)<=0`) after
`fresh_bank_item_id(556)>=1` must already have passed. The script logged a
first-cycle deposit of 27 Air runes and a trip-2 essence restock, then a
second Craft-rune. Chat contains two temple bind/portal sequences. Passing
second-craft chat does not overrule the failed scenario. The FAIL line has
runner Evidence and **no CoreWitness**. That gap is what still separates
"GameSnapshot never showed 556==0" from "the serial watch missed a real
empty-pack window."

The remaining `nav-walk` to stand `(3013,3355)` radius 0 while named
OpenBooth targets booth `(3011,3354)` is real on the unfinished second
return. It is also present on the **first** return of the same FAIL cell,
which then posted SetNoteMode, deposited, and restocked. Overlap alone does
not establish that the bank never opened.

## Classification against brief 84

| | Choice | Applies? |
|---|---|---|
| (a) | Native bank/route interference as the FAIL cause | Not established. First return opened. Second return ran out of wall-clock with the same overlap the first return survived. |
| (b) | Fixture observation timing/ordering | **Primary remaining hypothesis.** Pack-empty was the armed watch after a first-cycle `fresh_bank_item_id` pass, across a window where the script had already cleaned Air runes out of the pack. |
| (c) | Legitimate existing bound (180s / radius / openBooth stand) | The 180s deadline is the original gold bound. It fired. That is not proof the bound is wrong. |
| (d) | Foreign catalog source regression | No. `RuneCrafter.ts` is byte-identical (`cmp` 0, sha `a944049b42ee`). Location tiles live in that file. |
| (e) | 04k named `Bank.openNearest` mapping | No. Primary caller is `Bank.openBooth(bot.bankTile(), …)`. `openNearest` is fallback only. 04m does not cover this path. |

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Frozen host | `6750713b80ecfb8ad5036f3ffb0564459f67a856` |
| Frozen client | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Isolated export | `.superpowers/review-exports/catalog-root-6750713b` |
| Headless catalog-boundary binary | sha256 `9bd499e7d977ad14b9a96ec54d79a2bef9c9d62b5ed7e669b54f1717915c4450` (178455304 bytes) |
| Campaign HEAD at write-up | `047fe5869a9dcdf686c6d1165c25417d53461381` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 84 SHA-256 | `c9422be7b7799022361d403a87899be9cbc187a7c81fb49078bae4f9856deae6` |
| Kanban card | `t_8a8e6ef3` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| 274 engine | `4c95f87efe00b068cadbd229d94736626907bd1a` |
| 289 engine | `cc359656b4acd216ca452495874b6beba9a0ac75` |
| RuneCrafter.ts (both catalogs) | sha256 `a944049b42ee5adf882aaf119fb7b7f4604b63e2d07ff34e2db6dbf9e3954843` |
| Frozen `bank.js` | sha256 `31d0380098d9c9d1b759447ce832461c3dd9b52069ceb9b6bd96d5181cfb1aeb` |

Line hashes match `docs/compat/evidence/catalog-headed/source-6750713b.json`.
Extra hashes: `docs/compat/evidence/rune-air-deadline/hashes.json`.

## 1. Four Air cells (immutable receipts)

| Cell | Exit | Wall s | Runner | Predicate | Tile | CoreWitness |
|---|---|---|---|---|---|---|
| r274 old `100adccc` | 0 | 141.452 | PASS 243 ticks / 138340 ms | `has_item_id(556)>=1` | temple `[2843,4832,0]` | full, `further=true` |
| r289 old `100adccc` | 0 | 140.881 | PASS 244 / 137716 | `has_item_id(556)>=1` | temple `[2843,4832,0]` | full, `further=true` |
| r289 newer `8e7d965b` | 0 | 142.601 | PASS 248 / 139481 | `has_item_id(556)>=1` | temple `[2843,4832,0]` | full, `further=true` |
| r274 newer `8e7d965b` | 1 | 183.106 | FAIL 334 / 180010 | `has_item_id(556)<=0` | bank `[3011,3355,0]` | **absent** |

FAIL inventory at deadline: Air talisman 1438 ×1 + Air rune 556 ×27. Scene 2.
No `not impl`, no `missing-op`, no `could not open`.

PASS terminal predicate is the **further-craft** arm, not pack-empty. Those
three cells already observed pack-empty, restock, close, ruins return, and
the second craft. FAIL never left pack-empty.

## 2. Ordered scenario watches after Start

Frozen `rune_craft_variant` (`scenario/src/lib.rs` 5909–5970), deadline
`SCRIPT_GOLD_DEADLINE` = 180s, each watch `SCRIPT_GOLD_WATCH_TICKS` = 150.
A watch-budget miss would read `not seen within 150 ticks`. FAIL reads
`deadline 180s exceeded` with the current arm still pack-empty.

After Start, serial arms are:

1. pack essence `has_item_id(1436)>=1`
2. arrived_near ruins `(2988,3294,0)` r4
3. crafted `has_item_id(556)>=1`
4. Runecraft XP gain
5. essence gone `has_item_id(1436)<=0`
6. portal exit, same ruins r4
7. **`fresh_bank_item_id(556)>=1`** (`Proof::BankItemId`; `fresh_bank` =
   ingame && scene 2 && `bank_component_id>=0` && `bank_loaded`; **no**
   generation check despite the name)
8. **`has_item_id(556)<=0`** pack-empty
9. restock essence
10. `bank_closed`
11. return to ruins
12. another crafted `has_item_id(556)>=1` (also the scenario `proof`)

CoreWitness is a **parallel** latch (`RuneCrafterCycle`), not this serial
list. `deposited` there additionally requires `bank_generation > baseline`,
pack 556==0, and bank 556>=1.

## 3. What the script actually posts

Frozen `openBank` (RuneCrafter.ts 96–108):

1. `walkTo(bankTile(), 3)` → `WalkNear` `(3013,3355)` radius **3**
2. `Bank.openBooth(bankTile(), 'Bank booth', 'Use-quickly')` → `WalkNear`
   the **supplied stand** radius **1** (120s adjacent wait), then named
   `open-booth` for loc `(3011,3354)` id 2213
3. only then `Bank.openNearest` fallback

04k/04m compose approach for `openNearest` without a stand. This path
already supplies the stand. Preserve `openBooth(stand)` and low-level
geometric OpenBooth. Do not retarget this FAIL onto that hop.

`BankTrip` validates `!inTemple() && essCount()===0`, then `openBank`,
`cleanPack([talisman])`, log `deposited N`, withdraw essence.

## 4. Command counts (all four Air logs)

Posted isolate `interact` lines:

| Cell | OpenBooth | WalkNear | Loc | Craft-rune | Use | SetNoteMode | heading-to-bank | deposited 27 |
|---|---|---|---|---|---|---|---|---|
| r274 old | 3 | 4 | 8 | 2 | 6 | 2 | 2 | 1 |
| r289 old | 3 | 4 | 9 | 2 | 7 | 2 | 2 | 1 |
| r289 newer | 3 | 4 | 9 | 2 | 7 | 2 | 2 | 1 |
| r274 newer FAIL | **5** | **6** | **15** | **2** | **13** | 2 | **3** | 1 |

All four posted **two** Craft-rune. FAIL's extras are a second bank
approach (`WalkNear` r3 then r1, two more OpenBooth) and more portal Use
clicks. Seed OpenBooth is the first of the three/five.

PASS WalkNear order: ruins r1, bank r3, stand r1, ruins r1.
FAIL appends bank r3, stand r1 after the second Craft-rune.

## 5. FAIL timeline versus first-return success on the same cell

Script lines on `r274-rune-crafter-8e7d965b-6750713b.log`:

- 20 / 24 / 31 / 33: seed bank, SetNoteMode, talisman, 27 essence trip 1
- 168: Craft-rune 2478
- 189 / 190: heading to bank, WalkNear stand r3
- 303: WalkNear stand r1
- 330 / 354: OpenBooth 2213 while `nav-walk` still aims `(3013,3355)` r0
  from `(3012,3355)` then `(3011,3355)`
- **364 SetNoteMode, 373 `deposited 27 Air runes`, 379 trip 2 essence**
- 383: WalkNear ruins r1
- 516: second Craft-rune
- 534 / 535 / 646: heading to bank, WalkNear r3 then r1
- 694 / 718: OpenBooth 2213, same remaining r0 walk, player `(3012,3355)`
  then `(3011,3355)`
- **no second SetNoteMode, no second deposit log**
- 725: deadline, pack-empty predicate, inv still 27 Air

Chat (newest first) has two `bind … Air Runes` / portal / ruins-hold
blocks. `You don't have enough inventory space to withdraw that many`
is also on every PASS cell (Withdraw-All against 200 banked essence).

No FAIL item snapshots were invented. The only FAIL inventory row is the
deadline Evidence. Script `deposited 27` is a script log, not a
GameSnapshot.

## 6. Why pack-empty can still be the armed watch after two crafts

`Proof::BankItemId` needs a loaded open bank with 556>=1. The second
return never posted SetNoteMode, so it did not show a loaded bank in the
log. Therefore `fresh_bank_item_id(556)>=1` passed on the **first**
return, which did load and deposit.

Pack-empty is the next serial arm. It was therefore armed from first
deposit until the 180s deadline, including:

- script-claimed empty pack of Air runes
- restock (essence, still 556==0 if the deposit held)
- walk to ruins
- second Craft-rune (556 becomes 27 again)
- second bank approach

If any polled GameSnapshot in that empty window had `inv_id_count(556)==0`,
the watch would have advanced. It did not. PASS CoreWitness latches
exactly that empty-pack + banked-27 observation (e.g. r274 old deposited
tick 159, pack `{1438:1}`, bank `{556:27,1436:173}`, gen 5; restock tick
161; further craft in temple tick 211, gen 6, XP 270).

Two alternatives remain, and FAIL output cannot split them:

1. Serial GameSnapshot never showed 556==0 (script Inventory ≠ runner
   snapshot). Do not guess the missing rows.
2. Snapshots did show 556==0 and the serial watch missed them (poll /
   dirty / ordering). CoreWitness on PASS is a different latch; FAIL never
   printed it.

Second-craft chat is not a substitute for (1) or (2).

## 7. 04k / 04m do not cover this caller

Frozen `openBooth(stand)` walks the supplied stand radius 1, then named
open-booth, wait 5s on generation. `openNearest` is
`openBooth(undefined, name, op)` and is only the fallback after that
returns false. 04m's new `openNearest` WalkNear-to-loc composition is
not this path. A current bank-approach patch is not proof it covers
RuneCrafter.

Low-level `open_named_booth_at` geometric Walk when Chebyshev > 1 stays
as specified. At the posted OpenBooth player tile `(3012,3355)` vs booth
`(3011,3354)` Chebyshev is 1, so that geometric substitution is not the
logged remaining walk. The remaining r0 walk is the **stand** `WalkNear`
from `openBooth(stand)`, still armed.

## 8. Ownership and what not to do

- Root owns any diagnostic or repair. This card owns only `05o` and
  `evidence/rune-air-deadline/`.
- Do not edit `bank.js` (`t_4948af53`) or scenario/catalog
  (`t_464d9c4c`) from this audit.
- Do not dim Air, do not raise 180s, do not special-case `(3013,3355)`,
  do not recommend a rerun as acceptance.
- Keep the four raw receipts immutable.

## 9. Remaining uncertainty

Unknown without FAIL CoreWitness: whether `RuneCrafterCycle.deposited`
latched, whether `further` latched, and whether runner GameSnapshot ever
had 556==0 after first deposit. Earth cells were not this card's four
Air receipts; they were not used as a substitute verdict.

## 10. One bounded falsifier (root-owned)

On the **same** isolated 6750713b binary and the single
`r274` / newer `rune_crafter` cell, change only the harness
`RunnerStatus::Failed` arm in frozen
`catalog_boundary_live.rs` (2914–2918) so it emits the accumulated
`witness.qualify()` JSON the outer-timeout arm already emits
(`error` + `witness`, including `rune_crafter_cycle`). Do not change
success policy, deadline, OpenBooth, dim, or scenario watches. Do not
build a tracing system.

Read the new FAIL line:

- `deposited` Some with pack 556==0 at a first-cycle tick, `further`
  true: CoreWitness saw the bank cycle and second craft; the serial
  pack-empty watch missed it.
- `deposited` Some, `further` false: Observation stream saw the empty
  pack; serial still failed pack-empty anyway (same split, no second-craft
  latch).
- `deposited` None: Observation stream never saw empty pack + banked
  556 together. Script `deposited 27` then remains an unconfirmed
  Inventory view. That still does not authorize a timeout or dim.

That is the smallest root-owned diagnostic that distinguishes the
alternatives on this reviewed isolated source.
