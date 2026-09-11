# Resource diagnostic ownership (frozen 1eba-r2 overlay)

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 12:40 UTC. Kind: bounded read-only ownership audit for
brief 121. Diagnosis, not acceptance. Not implementation, compilation,
LIVE, fixtures, ledger, STATE, timeout inflation, or a foreign
navigator/planner copy. Root owns implementation and acceptance. No
routine reviewer card. Do not spawn implementation cards from this run.

Read once: `AGENTS.md`, `docs/execution.md`, brief 121, audit 06y. Branch
checked first: `codex/rs2b0t-multirevision` (not `main`). Campaign HEAD at
write-up is `b6013ba36ddb8bb0f5a56172be726d3b09a60622`. LIVE cells are
private diagnostic overlays on exact host `1eba4b1e11daf6b97e79ac3c393413a7540ab002`
/ client `9d090ed04957e4efc254f073cda97bc5510ca72b`, binary sha256
`82dea8dfddaadbf8d11530f6274d3a8a81ef2198819177871a2ed0fef5e4a36c`.
Overlays (logging only): `catalog_boundary_live.rs` and `shim/bank.js`.
Frozen source: `.superpowers/review-exports/resource-diag-1eba-r2`. Work
was read-only except this report and
`docs/compat/evidence/resource-diagnostic-ownership/`. Concurrent dirty
`crates/scenario/src/lib.rs` / `snapshot.rs` / STATE were not used.

## Verdict

**Do not dim these three cards.** Do not raise clocks, RNG, XP
multipliers, or seed product after Start. Do not turn `givebank` into a
fake pass. Do not add a FlaxAIO close-planner. Overlay loc_facts is
door/gate within distance 8 only — it cannot dump flax, trees, or
Edgeville booths.

1. **HerbloreSecondaries: fixture one-shot Edgeville open never became a
   bank.** Both revs: `started=0`, `bank_component_id=-1`, `bank_loaded=false`,
   `bank_generation=0` at FAIL, empty `bank`/`bank_ids`/`bank_side`,
   `hold=false`, widgets `[3213,3214]`, chat seed-only, tile
   `[3094,3492,0]`, `core=no Start baseline`. Smallest owned fixture
   correction: open a packed Edgeville booth identity
   (`named_banks` `EDGEVILLE_BOOTHS` `3095,3491` / `3096,3493`) from stand
   `3094,3493` and wait `bank_loaded`, instead of one-shot
   `open_nearest_booth`. Not host bank semantics.

2. **GnomeChop: honest no-success on steel Magic, not an XP/inv
   publication bug.** `client.stat_xp[8]` equals snapshot
   `xp.woodcutting` at `1210421` for the whole post-setstat run. Chat is
   swing-only. Inv stays `{1353:1}`. Native `magic_tree_table` steel
   `4,12` at level 75 is `STAT_RANDOM` 10/256 per 4-tick attempt. 27
   empty slots cannot fill inside 180s. Recommend rune axe (native no WC
   req; catalog `AXES` WC 1) plus nonproduct ballast so one or two
   script-made logs fill, then deposit / closed return / further product.
   That is not capacity proof. Same first-log rate applies to
   `gnome_fletch_short` / `gnome_fletch_long`.

3. **FlaxAIO pick: deposit succeeded; return never posted.** Both revs
   bank `1779×28`, empty pack, `hold=false`, `depositInventory`
   enter+return, OpenBooth `2213` at `2721,3494`, then no later `WalkTo`.
   FAIL is `bank_closed` (274 deadline 180s; 289 150-tick watch).
   `GoToFieldTask` requires `nearestFlax()===null`, not `!atField()`.
   Missing fact that prevents a host-vs-foreign close: Flax loc rows +
   `Reachability.canReach` at the FAIL snapshot. Overlay loc_facts cannot
   answer. Do not infer a close-planner.

## Classification against brief 121

| | Question | Applies? |
|---|---|---|
| Herblore first owner | One-shot Edgeville open never fresh | **Yes.** Component -1, loaded false, gen 0, empty side, no Start. |
| Herblore | Change host bank / clocks / fake givebank | **No.** |
| Herblore | Path miss of Edgeville | **No.** Arrived stand `3094,3493` then `3094,3492`. |
| Gnome first owner | Unrealistic 180s / steel 10/256 fixture | **Yes.** Raw XP matches snapshot; swing without logs. |
| Gnome | Host XP/inv publication gap | **Rejected on this overlay.** 06y 274 `You get some magic logs.` is not in these chats. |
| Gnome | Seed product/XP or raise timeouts/RNG | **No.** |
| Gnome fletch siblings | Same first-log consideration | **Yes.** Do not treat 180s as 27-log throughput. |
| Flax first observation | 28 flax banked, deposit logged, bank left open, no WalkTo | **Yes.** |
| Flax | Foreign trap: GoToField vs Pick vs atField/scope 12 | **Code yes; live nearestFlax unknown.** |
| Flax | Host/shim walk refused on open bank | **Open.** Needs loc/canReach at FAIL. |
| Flax | Add close-planner / dim | **No.** |
| Foreign catalog regression | | **No.** Named scripts byte-identical across both catalogs. |

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `b6013ba36ddb8bb0f5a56172be726d3b09a60622` |
| LIVE host (exact 1eba + overlays) | `1eba4b1e11daf6b97e79ac3c393413a7540ab002` |
| Client gitlink | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Isolated export | `.superpowers/review-exports/resource-diag-1eba-r2` |
| Overlay binary | sha256 `82dea8dfddaadbf8d11530f6274d3a8a81ef2198819177871a2ed0fef5e4a36c` |
| Overlays | `catalog_boundary_live.rs` `388ccbce…fa6558f`, `bank.js` `8c20365b…36d091` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 121 SHA-256 | `dc024c4a22448ad6257ff843cb698089743566b8863f1f69a30ff44f24d2955b` |
| 06y SHA-256 | `5a7835c7f9ca00efeb710679d50e89e821c23ce798f159a93b5353f6798d581d` |
| Kanban card | `t_f7fa09db` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| 274 engine | `4c95f87efe00b068cadbd229d94736626907bd1a` |
| 289 engine | `cc359656b4acd216ca452495874b6beba9a0ac75` |
| HerbloreSecondaries.ts (both) | `daa91562…c28523` |
| GnomeMagicChopper.ts (both) | `3b0d034c…2471e1` |
| FlaxAIO flaxaio.ts / banking.ts / picking.ts (both) | `6fd81a87…b608a1` / `f7b2b65a…d5f64` / `c0955fac…124b5` |

Machine-readable copies:
`evidence/resource-diagnostic-ownership/{refs,hypotheses,cells,receipts}.json`.
Log sha256 values match the six live receipts.

## 1. HerbloreSecondaries — lobster seed bank (pre-Start)

| Cell | Exit | Wall s | Runner | Predicate | Tile | Bank |
|---|---|---|---|---|---|---|
| r274 | 1 | 110.866 | FAIL 160 ticks / 107677 ms | `fresh_bank_item_id(379)>=50` step 4 | `[3094,3492,0]` | component -1, loaded false, gen 0, side [] |
| r289 | 1 | 47.261 | FAIL 160 / 44045 | same | `[3094,3492,0]` | same at FAIL |

Chat seed-only. Scene 2. `started=0`. Widgets inventory-only. r289 unique
`bank_generation` values include 1 and 2 during login frames with
`tile=None`; they never pair with `bank_loaded` or component 5382. Last
frame is gen 0.

Scenario (frozen 1eba `herblore_secondaries_scenario`): `givebank lobster 50`,
tele `EDGEVILLE_BANK` `3094,3493,0`, one-shot `tanner_open_seed_bank` →
`open_nearest_booth()` (`Use-quickly` nearest, refuse if none). Packed
booths are `3095,3491` and `3096,3493`. Native `bankbooth` op2 is
`Use-quickly`. Player walked stand → `3094,3492` (diagonal of `3095,3491`)
and the UI never opened.

Smallest owned fixture change: `open_booth_at` / named Edgeville booth
identity, then `Proof::BankItemId` on `bank_loaded`. Retry that identity.
Do not change `open_nearest_booth`. Do not lengthen 150 ticks. Do not
Talk-to a banker. Missing loc dump of every loc at `3094,3492`/`3093`
(overlay loc_facts filters doors/gates) is not required to own the
one-shot.

## 2. GnomeChop — steel Magic, 10/256, 180s

| Cell | Exit | Wall s | Runner | Predicate | Tile | Inv | Raw XP / snap XP |
|---|---|---|---|---|---|---|---|
| r274 | 1 | 90.45 | FAIL 167 / 87325 | `stat_xp_gain(8)>=1` step 13 | `[2433,3409,0]` | 1353 only | 1210421 / 1210421 |
| r289 | 1 | 43.333 | FAIL 169 / 40011 | same | `[2433,3409,0]` | 1353 only | 1210421 / 1210421 |

Prep: west magics `[2372,3425,0]`, WC 75, steel axe. OnStart opens gnome
bank (component 5382, gen 1), deposits then withdraws 1353, closes (gen 2),
walks to `2433,3409`, `Chop down` loc 1306 at `2432,3410`. Chat:
`You swing your axe at the tree.` Anim 875. No logs line. No 1513.

Native both worlds (`trees.dbrow` `[magic_tree_table]`,
`woodcut.rs2` `@get_logs`, `PlayerOps.STAT_RANDOM`):

```
value = floor(low*(99-lvl)/98) + floor(high*(lvl-1)/98) + 1
success if value > floor(U*256)
```

Steel `4,12` at 75 → **10/256**. Rune `7,21` at 75 → **17/256**. Cycle is
`%action_delay = map_clock+3` (4 ticks). Expected time per log at 600 ms:
steel 61s, rune 36s. 27 logs: ~28 min steel, ~16 min rune, plus 1/8
deplete and 400-tick respawn. 180s cannot hold a 27-log fill. This run
spent ~20–30 ticks at the tree after arrival; ~16 attempts, E[logs]≈0.6.

Catalog `needsBankTrip` banks when `!fletch && isFull() && logCount()>0`
(1 axe ⇒ 27 products). `AXES` rune WC level 1; native axe checker has no
WC req and accepts inv_total.

Bounded full-core fixture (preserve zero seeded products):

- Keep steel out. Seed **rune axe** as the retained tool (best legal).
- Add **nonproduct ballast** (junk tools, not 1513/72/70) so 1–2
  script-made logs fill, then upstairs deposit, closed ground return,
  further 1513.
- Do not claim 28-slot throughput. Do not raise `SCRIPT_GOLD_DEADLINE`
  or `STAT_RANDOM`. Dedicated longer observation is the other legal
  comparison; it is a separate private diagnostic, not this 180s card.

Fletch siblings chop the same tree before converting; they inherit the
first-log 10/256 (steel) problem. Same axe/ballast choice. Unresolved
publication issue: **none on this overlay**.

## 3. FlaxAIO pick — deposit then freeze

| Cell | Exit | Wall s | Runner | Predicate | Tile | Bank / pack |
|---|---|---|---|---|---|---|
| r274 | 1 | 183.227 | FAIL 341 / 180001 deadline | `bank_closed` | `[2721,3493,0]` | 1779×28 loaded gen 1; pack empty; hold false |
| r289 | 1 | 133.176 | FAIL 278 / 130044 | `bank_closed` 150-tick | `[2721,3493,0]` | same |

Prep field `[2741,3444,0]`. Inject pick-only. Both fill 28, `WalkTo 2721,3493`,
`OpenBooth 2213 Use-quickly` at `2721,3494`. Overlay:
`[private-resource-diagnostic] depositInventory enter` then `return`.
Witness `flax_aio_pick_cycle.first_pack=true` and `deposited` set;
`returned=false`. No `WalkTo` after OpenBooth. `main_modal=5292`.

Frozen both catalogs (`picking.ts` identical sha `c0955fac…`):

- `atField`: distance to `FIELD (2741,3444)` ≤ `FIELD_SCOPE 12`.
- `nearestFlax`: name Flax + Pick, loc distance to field centre ≤ 12,
  first `Reachability.canReach(..., {adjacentOk:true, maxSteps:400})`.
  `canReach` needs the dest in the loaded local scene.
- `PickTask`: picking && !full && atField && nearestFlax≠null.
- `GoToFieldTask`: picking && !full && nearestFlax===null (not !atField).
- `bankRun`: open, `depositInventory`, return **without** `Bank.close`.
- `needsBank` pick-only: `Inventory.isFull()` on level 0.

After deposit the pack is empty so `needsBank` is false and Pick is false
(`atField` false at chebyshev 49). GoToField fires only if `nearestFlax`
is null. Field flax at ~49 tiles can still sit inside a 104-scene; if
`canReach` succeeds, GoToField never walks. That is a real foreign trap
on paper.

**Missing diagnostic fact:** `snapshot.locs()` Flax rows (id, tile,
actions) plus `canReach` at the FAIL tile. Overlay `keep_bounded_loc` is
door/gate ≤8, so loc_facts stayed empty even while picking. Without that
row, do not call host walk-refusal and do not call the foreign trap
proved on this cell. Do not add a close-planner. A missing optional
native close is not a broken-script verdict.

## Hypotheses

| | Hypothesis | Status |
|---|---|---|
| (1) | Dim the three cards as broken imports | **Rejected.** Fixture / observation / rate. Byte-identical catalogs. |
| (2) | Herblore: never reached Edgeville | **Rejected.** Stand then `3094,3492`. |
| (3) | Herblore: one-shot nearest booth, bank never fresh | **Accepted owner.** Pin packed booth + `bank_loaded`. |
| (4) | Herblore: change host open / clocks / fake givebank | **Rejected.** |
| (5) | Gnome: host XP/inv desync (06y 274 logs chat) | **Rejected here.** Raw===snap; swing-only. |
| (6) | Gnome: steel 10/256 + 27-slot 180s is an unrealistic qualification fixture | **Accepted owner.** |
| (7) | Gnome: seed logs/XP or raise timeout/RNG | **Rejected.** |
| (8) | Gnome fletch siblings share the first-log rate | **Accepted.** |
| (9) | Flax: deposit works; Bank.close not called; no WalkTo | **Accepted as observed.** |
| (10) | Flax: foreign GoToField/Pick freeze when in-scope flax remains reachable | **Code trap yes; live nearestFlax unproven.** |
| (11) | Flax: host/shim refuses WalkTo on an open loaded bank | **Open.** Needs loc/canReach. |
| (12) | Flax: add close-planner | **Rejected.** |
| (13) | Copy foreign navigator/planner | **Rejected.** |

## Owned next (root)

Root owns live reproduction. This run does not implement.

1. Herblore fixture: exact Edgeville booth `3095,3491` or `3096,3493` from
   `3094,3493`, wait `bank_loaded`. Optional loc dump at the click tile.
2. Gnome fixture: rune axe + nonproduct ballast; 1–2 script-made 1513,
   deposit, close, return, further 1513. Same for fletch siblings' first
   chop. Do not advertise capacity.
3. Flax: dump Flax locs + canReach at the open bank FAIL. Then either
   own the foreign `GoToField` predicate or a host walk-on-open-bank
   refusal. Do not edit FlaxAIO in that step.

Keep `crates/scenario/src/lib.rs` serialized if fixture seed order
changes. Do not edit cook / paint / shop / Make-X files.
