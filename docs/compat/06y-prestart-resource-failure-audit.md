# Pre-Start and resource core failure ownership (frozen 04d7b1f4)

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 11:55 UTC. Kind: bounded read-only ownership audit for
brief 116. Not implementation, compilation, LIVE, fixtures, ledger, STATE,
timeout inflation, or a foreign navigator/planner copy. Root owns live
reproduction after any corrected source/review. Fixture109 is in review;
other runtime/paired owners are active. No routine reviewer card. Do not
spawn implementation cards from this run.

Read once: `AGENTS.md`, `docs/execution.md`, brief 116. Branch checked
first: `codex/rs2b0t-multirevision` (not `main`). Campaign HEAD at
write-up is `5dd89374ba50fe3c05fa85c5023b4ec4db7931d7`. LIVE cells were
recorded on frozen host `04d7b1f48180ab9b93bbd6fec45a655df4385233` /
client `9d090ed04957e4efc254f073cda97bc5510ca72b`. Work was read-only
except this report and
`docs/compat/evidence/prestart-resource-failure-audit/`. Frozen source is
`.superpowers/review-exports/catalog-root-04d7b1f4`. Concurrent cook /
paint / teleport / shop / Make-X / remaining-fighter / fixture109 WIP
was not touched.

## Verdict

**These five old-catalog both-rev FAILs are not broken imported
scripts.** Do not dim the cards. Do not raise stats or watch budgets to
hide the receipts. Do not copy GnomeMagicChopper / CoalTrucks / FlaxAIO
controllers.

Two cells die **before Start**:

1. **ChaosDruid Attack40 ack is current-stat after a tele into
   hostiles.** `Proof::Stat` reads `effective`. Chaos druids cast
   Weaken (`mes("You feel weakened.")` + `stat_sub`). Smallest owned
   fixture change: acknowledge Attack/Strength/Hitpoints **before** the
   dungeon tele, or ack `base` not `effective`. Not Attack 99.
2. **HerbloreSecondaries lobster bank ack is a one-shot Edgeville
   booth click that never became a fresh bank of 50×379.** Arrival at
   `3094,3492,0` worked. `open_nearest_booth` returned `Sent` (else
   the driver would have rejected). Chat never left the seed set.
   Split open-UI vs empty-`givebank` vs `bank_loaded` with the next
   snapshot; do not lengthen the 150-tick watch.

Three cells die **after Start**:

3. **GnomeChop reaches Magic tree 1306 and posts `Chop down`.** 274
   chat `You get some magic logs.` with inv still only the steel axe
   and no `stat_xp_gain(8)`. 289 chat is swing-only. First owner is
   chop completion / inv+XP publication, not west-magics pin selection.
   OnStart banking for axe/knife is catalog behavior.
4. **CoalTrucks is death to giant bats.** Script logs the constraint
   (`aggressive below 55 combat`). Fixture seeded Mining 30 / HP 10 /
   combat 1 on the mine tile. 274 mined 453 then `Oh dear you are
   dead!` at Lumbridge with two coal. 289 swung, never mined, same
   death. Not missing `Tools.bestPickaxe`. Smallest owned fixture
   change: seed combat ≥55 (the script's own threshold) or tele to a
   stand the bats do not aggro, then walk in. Not a longer XP watch.
5. **FlaxAIO pick-only: Seers booth `OpenBooth` 2213 works.** 274
   filled (`You can't carry any more flax.`), deposited (empty pack),
   then sat `bank_closed` to the 180s deadline — `bankRun` never calls
   `Bank.close` and no `WalkTo` followed the open. 289 picked flax
   (chat) then banked before `has_item_id(1779)>=28`. Next snapshot
   must dump inv ids/`inv_size`/`isFull` while picking. Do not add a
   close-planner.

Static lead, no Cook causality: `resolve_op_target` loc arm
(`host-play` ~2113) is first `x/z/level` and ignores `target_name`.
CookBot salmon cells are not this audit. The native diagnostic for
colocated locs is a dump of **every** loc at the clicked tile (id,
name, actions), not a Cook harvest.

## Classification against brief 116

| | Question | Applies? |
|---|---|---|
| Chaos first owner | Current Attack after Weaken, tele-before-ack | **Yes.** `Proof::Stat` → `s.effective`. Engine `npc_combat_magic.rs2` `stat_sub` + `You feel weakened.` |
| Chaos | Stats never seeded / scene stale | **Not distinguished.** Evidence `StatRow` is runenergy-only. Items+scene 2 present. Need base vs effective. |
| Chaos | Broken script | **No.** Never reached Start. `core=no Start baseline`. |
| Herblore first owner | Fresh lobster bank never seen at Edgeville | **Yes.** Step 4 `fresh_bank_item_id(379)>=50`. Tile booth stand. Chat seed-only. |
| Herblore | Path/control miss of Edgeville | **No.** Arrived `3094,3492` vs stand `3094,3493` r8. |
| Herblore | Increase timeout / 99 stats | **No.** |
| Gnome first owner | Chop/XP/inv after `Chop down` 1306 | **Yes.** Not missing `Traversal.preload`. |
| Gnome | Wrong tree / west magics empty | OnStart gear walk is catalog. They still clicked 1306. Not the XP owner. |
| Coal first owner | Death at the mine, combat 1 | **Yes.** Both-rev `Oh dear you are dead!` Lumbridge. |
| Coal 274 truck miss | Nav/truck loc | **No.** Died with 2×453 before a truck click. |
| Coal 289 noXP | Missing `bestPickaxe` / no mine click | **No.** Pickaxe held; `Mine` 2096 posted; death before ore. |
| Flax 274 bank_closed | Script never closes; no walk after open | **Yes.** `Bank.close` mapping exists; `banking.ts` does not call it. |
| Flax 289 not full pack | 1779 never held 28 in a snapshot | **Yes as observation.** Split isFull-early vs non-1779 vs missed tick. |
| Foreign catalog regression | | **No.** Named scripts are byte-identical across both catalogs. |
| Dim / hide with stats or timeouts | | **No.** |

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `5dd89374ba50fe3c05fa85c5023b4ec4db7931d7` |
| LIVE host (frozen Root04d) | `04d7b1f48180ab9b93bbd6fec45a655df4385233` |
| Client gitlink | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Isolated export | `.superpowers/review-exports/catalog-root-04d7b1f4` |
| Catalog-boundary binary | sha256 `9ba631c61a28c31b4f661036fd9109f84b0827b50f848642c0cb49393d62e3ef` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 116 SHA-256 | `f9e4d00e481b80fa15355f0647bb9317b287b75c658fd1949a5c87ea0dc6ef9d` |
| Kanban card | `t_c901bad1` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| 274 engine | `4c95f87efe00b068cadbd229d94736626907bd1a` |
| 289 engine | `cc359656b4acd216ca452495874b6beba9a0ac75` |
| ChaosDruidKiller.ts (both) | `51ed34065cb42a85680e122f01bd92903587ca117e3e5ed45e72e75f1a1eb2fa` |
| HerbloreSecondaries.ts (both) | `daa91562ff5993a6b560a7471c62a5c6c69a1f63b5c823c7a2aa3965fec28523` |
| GnomeMagicChopper.ts (both) | `3b0d034c761f9b98361561627817a6d806d359f659000cbccdf22e25772471e1` |
| CoalTrucks.ts (both) | `d20112f0bfb44612ae53e0c56009c6b3e50808375c2a54dc43ed2160e55fcf36` |
| FlaxAIO flaxaio.ts / banking.ts (both) | `6fd81a87…b608a1` / `f7b2b65a…d5f64` |

Machine-readable copies:
`evidence/prestart-resource-failure-audit/{refs,hypotheses,cells}.json`.
Log sha256 values match the per-cell `*.json` receipts.

## 1. ChaosDruid — Attack 40 ack (pre-Start)

| Cell | Exit | Wall s | Runner | Predicate | Tile | Inv |
|---|---|---|---|---|---|---|
| r274 | 1 | 66.784 | FAIL 163 ticks / 63550 ms | `stat(0)>=40` step 4 | `[3110,9934,0]` | 1331 + 12×379 |
| r289 | 1 | 69.219 | FAIL 162 / 66116 | same | `[3111,9935,0]` | same |

Chat (newest first): `I can't reach that!` / `You feel weakened.` / seed
(`Inventory wiped.` / `set tutorial: to 1000` / `Welcome to RuneScape.`).
Scene 2. `core={"error":"no Start baseline"}`. `StatRow` is runenergy 100
only — evidence.rs does not dump combat stats.

`combat_core_scenario` order: `setstat attack/strength/hitpoints 40`,
give scimitar+lobster, **tele `CHAOS_DRUID_FIELD` (3110,9936,0)**, wait
arrived r14, **then** `Proof::Stat { id: 0, min: 40 }`. `stat_value`
maps id 0 to `stats[].effective`. Frozen engine
`content/scripts/skill_combat/scripts/npc/npc_combat_magic.rs2` on
Weaken: `mes("You feel weakened.")` then `stat_sub($stat, $constant,
$percent)` — current, not base.

`I can't reach that!` is consistent with auto-retaliate through dungeon
obstacles after the tele. It is not a booth click (this cell never
opens a bank).

Smallest owned fix: ack the three melee stats on a safe tile **before**
the dungeon tele, or ack `base`. Do not seed Attack 99. Do not treat
missing `StatRow.attack` as proof that `setstat` failed — the next
snapshot must print `stats[0].{effective,base,xp}`.

## 2. HerbloreSecondaries — lobster seed bank (pre-Start)

| Cell | Exit | Wall s | Runner | Predicate | Tile | Inv |
|---|---|---|---|---|---|---|
| r274 | 1 | 111.717 | FAIL 160 / 108553 | `fresh_bank_item_id(379)>=50` step 4 | `[3094,3492,0]` | empty |
| r289 | 1 | 112.585 | FAIL 160 / 109402 | same | `[3094,3492,0]` | empty |

Chat is seed-only. Scene 2. `core=no Start baseline`. Settings
`secondary="Red spiders' eggs"`.

Scenario order: `givebank lobster 50`, tele `EDGEVILLE_BANK`
(3094,3493,0), wait arrived r8, then `tanner_open_seed_bank` →
`open_nearest_booth()` once (`Perform` sets `step_sent`; a `false`
send would `driver rejected the send`, which did not happen). Wait is
`Proof::BankItemId` which requires `fresh_bank`: ingame, scene 2,
`bank_component_id>=0`, `bank_loaded`.

Arrival worked (1 tile south of the stand). The booth click was
posted. The bank never became a fresh 50×lobster snapshot. No
`Use-quickly` / banker chat.

Split with the next snapshot, not a longer watch:

- `bank_component_id`, `bank_loaded`, `bank_ids`
- locs at 3094,3493,0 (id, name, actions) — whether the clicked
  `Use-quickly` loc is the Edgeville booth
- whether `givebank lobster 50` is in the account bank when opened

Do not change host bank-open tolerance. Do not invent a banker Talk-to
fallback (`open_nearest_booth` is Use-quickly only by contract).

## 3. GnomeChop — Magic tree after Start

| Cell | Exit | Wall s | Runner | Predicate | Tile | Inv |
|---|---|---|---|---|---|---|
| r274 | 1 | 89.141 | FAIL 168 / 85934 | `stat_xp_gain(8)>=1` step 13 | `[2433,3409,0]` | 1353 only |
| r289 | 1 | 93.960 | FAIL 168 / 90707 | same | `[2433,3409,0]` | 1353 only |

Prep baseline: tile west magics `[2372,3425,0]`, WC 75, XP 1210421,
steel axe 1353, loc_facts only Gate 1553. `woodcutting` is skill index
8 (`Skill::names`). XP baseline is captured when the StatXpGain watch
begins (`runner.rs` `begin_step`), after Start — a later chop should
count.

Posted commands (both revs, same shape):

1. OnStart: `Traversal.preload()`, then `gear: opening gnome bank for
   best axe / knife` — WalkNear bank stairs 2444,3416, Climb-down 1744,
   then `running to Magic tree @ 2432,3410`.
2. `interact Loc { x: 2432, z: 3410, level: 0, action: "Chop down", id: Some(1306) }`.

274 chat newest: `You get some magic logs.` / `You swing your axe at
the tree.` Inv at FAIL is still only 1353. 289 chat is swing-only —
no logs line.

OnStart gear (deposit-all-except-knife, then withdraw axe) is catalog
GnomeMagicChopper, not a missing `Tools.bestAxe` throw and not a reason
to dim. Isolated28 `Traversal.preload` / `bestPickaxe` notes in the
frozen support-matrix are stale relative to these 04d logs.

First owner of the XP FAIL:

- 274: server chat says logs were granted; host inv and `stat_xp[8]`
  did not move. Next snapshot: `stats[8].xp` vs 1210421, inv 1513,
  ground 1513, `local_animation`.
- 289: swing without logs — chop did not complete. Same loc dump:
  every loc at 2432,3410,0 (id/name/actions), not only 1306.

Do not copy the three-pin hopper. Do not treat west-magics loc_facts
(gate only at prep) as the chop failure; they left that tile for gear
before chopping.

## 4. CoalTrucks — mine then death

| Cell | Exit | Wall s | Runner | Predicate | Tile | Inv | Chat |
|---|---|---|---|---|---|---|---|
| r274 | 1 | 106.070 | FAIL 177 / 102898 | `arrived_near(2575,3486,0,4)` step 11 | `[3222,3217,0]` | 1269 + 2×453 | `Oh dear you are dead!` + mine/swing + `You manage to mine some coal.` |
| r289 | 1 | 98.835 | FAIL 163 / 95641 | `stat_xp_gain(14)>=1` step 9 | `[3221,3219,0]` | 1269 only | `Oh dear you are dead!` + swing, **no** mine-ore line |

Prep baseline both: `[2582,3481,0]`, Mining 30, XP 13363, pickaxe
1269, combat 1 / HP 10, loc_facts empty, not in combat.

Script onStart (both catalogs): `no combat handling: the level-27
giant bats here are aggressive below 55 combat`. Posted `Mine` loc
2096 at 2582,3479 then 2585/2586,3483. 274 later `WalkNear` 2582,3481
(recovery anchor) after death — Lumbridge, not the truck.

274 passed XP and coal-item watches (step 11 is the truck stand named
"after the pack fill" but the arm is only `ArrivedNear` truck r4; it
does not wait for a full pack). Two coal in the Lumbridge inv is
honest mined ore, then death. 289 died during swings, so the XP watch
never saw a gain.

Smallest owned fixture change: seed combat ≥55 so bats are not
aggressive, **or** tele to a non-aggro stand and walk to 2582,3481.
That is the script's documented constraint, not Attack-99 hiding.
Do not lengthen the XP watch. Do not copy CoalTrucksLogic. Truck loc
mapping is not reached.

## 5. FlaxAIO pick-only

| Cell | Exit | Wall s | Runner | Predicate | Tile | Inv |
|---|---|---|---|---|---|---|
| r274 | 1 | 183.185 | FAIL 340 / 180014 deadline | `bank_closed` | `[2721,3493,0]` | empty |
| r289 | 1 | 93.448 | FAIL 163 / 90219 | `has_item_id(1779)>=28` step 8 | `[2721,3493,0]` | empty |

Prep both: field `[2741,3444,0]`, empty pack. Inject `picking=true`,
`spinning=false`. Script log `FlaxAIO (pick)`. Pick loc 2646 posted
at 2739/2738,3442–3440. Then `WalkTo 2721,3493` and
`OpenBooth { x: 2721, z: 3494, id: 2213, name: Bank booth, action: Use-quickly }`.

274 chat newest: `You can't carry any more flax.` then many `You pick
some flax.` — full pack happened. After OpenBooth the log is host
heartbeats only through the 180s deadline. `FlaxAIO/banking.ts`
`bankRun` opens, `depositInventory`, returns **without** `Bank.close`.
Native `Bank.close` exists (`shim/bank.js`). Pick-only `needsBank()`
is `Inventory.isFull()` on level 0; after deposit that is false, so
the next expected act is return-to-field, which never posted.

289 chat is pick-only (no can't-carry). OpenBooth still posted, inv
empty at FAIL. `isFull()` is `inv_size>0 && used>=size`. Early bank
means used hit capacity without the harness ever observing 28×1779.

Next snapshot, not a longer deadline:

- while picking: inv ids+counts, `inv_size`, `isFull`
- after OpenBooth: `bank_component_id`, `bank_loaded`, whether a
  WalkTo/close was queued and refused
- 274: why GoToField never sent after deposit with bank still open

Do not add a close-planner to FlaxAIO. Do not treat missing booth
mapping as the owner — 2213 Use-quickly posted and the 274 full-pack
chat fired.

## 6. Static loc-resolver lead (no Cook claim)

`resolve_op_target` `"loc"` arm is
`.find(|l| l.tile.x == x && l.tile.z == z && l.tile.level == level)`
and does not use `target_name` or loc id. Obj arm does filter name.
This is a static observation on frozen 04d `host-play/src/lib.rs`
~2113. CookBot salmon cells are outside this audit and are not a
failure attribution.

If a later snapshot of gnome 1306 / coal 2096 / flax 2646 shows
**multiple locs on the clicked tile**, the native diagnostic is that
full loc list (id, name, actions) plus which row the posted command
bound. Direct `interact Loc { id: Some(...) }` in these logs may use
a different dispatch than UseOn; do not merge them without that dump.

## Hypotheses

| | Hypothesis | Status |
|---|---|---|
| (1) | Imported scripts are broken; dim the cards | **Rejected.** Frozen callers. Missing fixture order / observation / combat seed. |
| (2) | Raise Attack/Mining/timeouts until the watch passes | **Rejected.** Hides Weaken, bats, and bank-close. |
| (3) | Chaos: ack current Attack after tele into druids | **Accepted owner.** Reorder or ack base. |
| (4) | Chaos: setstat never applied | **Open.** Need base vs effective. Weaken chat does not prove 40 was held. |
| (5) | Herblore: never reached Edgeville | **Rejected.** Tile is the booth stand. |
| (6) | Herblore: one-shot booth Sent, bank never fresh | **Accepted as observed.** Split contents vs UI vs loaded on next snapshot. |
| (7) | Gnome: missing Traversal.preload / bestAxe | **Rejected** for these 04d logs. Preload called; they walked and chopped 1306. |
| (8) | Gnome 274: server granted logs; host XP/inv did not | **Accepted owner** of the XP FAIL. |
| (9) | Gnome 289: swing without logs | **Accepted** as incomplete chop. Same loc dump. |
| (10) | Coal: missing bestPickaxe | **Rejected.** Started; mined on 274; 2096 posted on 289. |
| (11) | Coal: death to bats at combat 1 | **Accepted owner.** Seed combat ≥55 or a safe stand. |
| (12) | Flax 274: Bank.close mapping missing | **Rejected.** Mapping exists; script does not call it; no WalkTo after open. |
| (13) | Flax 289: 1779×28 never in a harness snapshot | **Accepted as observed.** Need inv/`isFull` while picking. |
| (14) | Copy foreign navigator/planner | **Rejected.** |
| (15) | UseOn loc-arm first-match caused Cook FAIL | **Rejected / out of scope.** Static lead only. |

## Owned next (root)

Root owns live reproduction. This run does not implement.

1. Chaos fixture: ack melee stats before dungeon tele, or `Proof::Stat`
   on `base`. Snapshot `stats[0].effective` vs `base` on the current
   FAIL if the reorder is deferred.
2. Herblore: snapshot bank fields + Edgeville Use-quickly locs on the
   existing 150-tick wait. Then one owned click/load/`givebank` fix,
   not a timeout.
3. Gnome: snapshot `stat_xp[8]`, inv 1513, all locs at 2432,3410,0
   when `Chop down` 1306 posts.
4. Coal fixture: combat ≥55 or off-aggro tele. Truck is not this
   cell's first owner.
5. Flax: snapshot inv/`isFull` during pick; after OpenBooth, whether
   walk/close is refused on an open bank. Do not edit FlaxAIO.

Keep `crates/scenario/src/lib.rs` serialized if fixture seed order
changes (hotspot with other catalog cores). Do not edit cook / paint /
shop / Make-X / fixture109 files.
