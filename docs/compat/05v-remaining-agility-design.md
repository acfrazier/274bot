# Full-core fixtures for remaining agility cards

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 11:28 UTC. Kind: bounded read-only design for brief 110.
Not implementation, compilation, LIVE, engine, STATE, ledger, or a
foreign planner/route/guardian. Root owns acceptance, any LIVE
snapshot, serialized implementation insertion, and implementation
handoff. No additional cards from this run. Own only this report and
`docs/compat/evidence/remaining-agility-design/`.

Read once: `AGENTS.md`, `docs/execution.md`, brief 110. Branch checked
first: `codex/rs2b0t-multirevision` (not `main`). Campaign HEAD at
write-up is `c5749bbe29eaa0ff313f25169e3d101865e217a9`. Client gitlink
`9d090ed04957e4efc254f073cda97bc5510ca72b`. Concurrent cook / combat /
tick / tools / paint workers must not be touched. Work was read-only
except this report and the unique evidence dir.

`docs/compat/fixtures/remaining-combat-world.md` §§14–15 are dated
leads. Ridge-only / one-XP / fee-only / one queued obstacle are not
full cores.

## Verdict

**WildyAgility full core is a native five-obstacle circuit plus further
work, not ridge entry.** Seed Agility 52 and food in pack, tele the
south ridge stand, skip startup bank. Ordered witness: ridge success
onto `z>3931`, then pipe → ropeswing → stepping stone → log → rocks
with the selected lap bonus, then further pipe progress. Sections 14's
ridge-only cell is a diagnostic, not acceptance.

**BrimhavenAgility full core is arena movement + obstacle XP +
script-caused dispenser/ticket progression with subsequent work, not
the 200gp fee.** Tele the surface entrance with coins and food. Ordered
witness: Pay/Talk fee (coins 995 down, varp 309 bit 1), Climb-Down onto
plane 3, a real hop that awards agility XP and changes platform, Tag
on the hinted dispenser, then a second Tag that adds ticket 2996, then
further hop/XP. First Tag never grants a ticket. Sections 15's
XP-or-ticket-or is too weak.

**Do not stretch `SCRIPT_GOLD_DEADLINE` 180s or `SCRIPT_GOLD_WATCH_TICKS`
150.** If a full cell cannot finish, record the measured phase and keep
smaller diagnostics distinct from acceptance. No post-Start teleports,
stat awards, tickets, or fake failures.

Two missing host members block Brimhaven chase, and are not imported
script defects:

1. `reader.hintTile` — client already stores `HINT_ARROW` tile
   (`hint_type`/`hint_tile_x`/`hint_tile_z`); snapshot does not post
   it; the reader proxy throws `not impl: reader.hintTile`.
2. `actions.setRun` — isolate already decodes `set-run` and host-play
   already calls `Interactions::set_run`; the JS `actions` proxy does
   not expose `setRun`.

Map those thinly. Do not port the foreign arena BFS, DeathRecovery,
Shop.sell, or a boat helper. User-permitted ancillary stubs stay stubs.

| | Hypothesis | Status |
|---|---|---|
| (1) | Ridge XP/message + z>3931 is enough Wildy core | **Rejected.** Brief: whole native circuit + further work. |
| (2) | Entrance fee or one arena XP/ticket is enough Brimhaven core | **Rejected.** Need hop + XP + script Tag progression + subsequent work. |
| (3) | Dim the cards; imported scripts are broken | **Rejected.** Frozen constructors are the documented callers. Missing hint/setRun are host gaps. |
| (4) | Implement a host arena planner / guardian / route | **Rejected.** Frozen JS already owns `nextHop`/`ARENA_EDGES`. Host owes loc/npc/hint/run/bank. |
| (5) | Expand DeathRecovery, Shop.sell, or a boat helper so options pass | **Rejected.** User permits those stubs. Core leaves them idle. |
| (6) | Seed tickets / agility / teleports after Start, or fake a fail | **Rejected.** |
| (7) | Widen 180s/150 tick so a lap/ticket cycle fits | **Rejected.** Unchanged gold. Diagnostics ≠ acceptance. |
| (8) | Thin hint post + `actions.setRun` over existing isolate/host run; reuse Gnome ordered-witness shape | **Accepted mapping.** |

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `c5749bbe29eaa0ff313f25169e3d101865e217a9` |
| Client gitlink | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 110 SHA-256 | `8b49322ed139b90fe9aac400f33c9ec08ca279b1d48bbe3652fb206a7003bf8a` |
| Kanban card | `t_33794ec0` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| WildyAgility.ts (both catalogs) | `71170d4bfe844161fe87d36e411ded9e99424ebadac875e11db9f781043c405f` |
| WildyAgilityLogic.ts (both) | `8e38f53a01cea150f1e9fa67ece422ea4f642c748f52326c3a0512125bc77942` |
| BrimhavenAgility.ts (both) | `771daff07bd4b3d6f2826ab1300d4fd66bcbae0f9d7a76e4a2ad07a4d050e859` |
| BrimhavenAgilityLogic.ts (both) | `fedf5f8e05fd43642efb0370352e71e5feebf258a784bc503a2e145c33b46d4d` |
| AgilityBot.ts (GnomeCourse; both) | `dee7c8885685724b0453097a0cf6a66d36a4d081e24157f29d35c742c518028f` |
| 274 wilderness_course.rs2 | `fddf3605c923f61585edf45c2edc0377b13be9e668ce745d648c59a008062572` |
| 289 wilderness_course.rs2 | `db69683bd6b23f73241a4274334e40b3e9ebae954e853869c45b0d6cab5c3fe9` |
| 274 agilityarena.rs2 | `c33bf541b9e5e557dd607b27bfe7e065d5c9bfe2c949a242585a6dde787335d5` |
| 289 agilityarena.rs2 | `dadc3599740ced8b9cd9db5feb4314163c0c63fbdc300fec60f10d99bdf1a9a7` |
| wilderness_course.loc (274=289) | `90e9d6bf69eae77c18c12ab53d13e45387a9de5c810f4e6f348ac6c56d053991` |
| Native `client_adapter.js` | `53cf56b989a0de6c69ba7104f331d050f1be7b10eba1130da5a66e0a5a7a03d7` |
| Native `shop.js` | `0c532288168fb7ede92258d2da7f6f4ce158d5ea4bfe9eba1394a313b13ac682` |
| Native `shim/mod.rs` (has `set-run`) | `7b78764a188bf552311b5e53b83a2b5be38bfada6e1719c25b6853fda8d6a573` |

Machine-readable copies: `evidence/remaining-agility-design/{refs,hypotheses}.json`.

274 engine `4c95f87efe00b068cadbd229d94736626907bd1a` and 289 engine
`cc359656b4acd216ca452495874b6beba9a0ac75` / content pins in
`fixture-inputs.md` were inspected read-only. No server was started or
changed.

## 1. Dated lead vs this design

Reuse GnomeCourse's already-qualified shape (`gnome_course`: log dest,
ground return, pipe, further XP ≥ 94, second log dest; radius-8 is a
dimmed foreign search option). Do not revive `gnome_course_radius`.

| Lead cell | Dated witness | This design |
|---|---|---|
| `wildy_agility` | Ridge XP **or** success line **and** z>3931 | Whole circuit + lap bonus + further pipe |
| `wildy_agility_food` | Startup Edgeville withdraw | Keep as option cell; not the core |
| `brimhaven_agility` | Any arena XP **or** ticket 2996 | Hop + XP + first Tag + ticket 2996 + further hop |
| `brimhaven_agility_bank` | Deposit + re-enter | Keep as option; 180s likely cannot fit |

## 2. Selected content facts (not guesses)

`stat_advance` units are tenths. Gnome lap 86.5 + next log 7.5 = 94 is
already the gold further-work bar.

### Wildy (m46_61 / wilderness_course.rs2)

Loc names/ops match the frozen course list. Pack ids are identical on
274 and 289:

| Step | Loc | Id | Name | Op | XP | Skill |
|---|---|---|---|---|---|---|
| Entry | `loc_2309` | 2309 | Door | Open | 15.0 | 52 (mesbox else) |
| 1 | `obstical_pipe2` | 2288 | Obstacle pipe | Squeeze-through | 12.5 | 49 |
| 2 | `obstical_ropeswing2` | 2283 | Ropeswing | Swing-on | 20.0 | fail → high-z pit |
| 3 | `wilderness_stepping_stone` | 2311 | Stepping stone | Jump-from | 20.0 | lava knockback, same scene |
| 4 | `wilderness_log_balance1` | 2297 | Log balance | Walk-across | 20.0 | fail → high-z pit |
| 5 | `wilderness_rocks` | 2328 | Rocks | Climb | 0 then **498.9** if progress=5 | none |

Lap from on-course: 12.5+20+20+20+498.9 = **571.4**. Ridge adds 15.0.
Further pipe adds 12.5 → **598.9** from a south-stand Start that
crosses the ridge and completes one lap plus the next pipe.

`~update_wilderness_varp(n)` only advances when `progress+1 >= n`, so
skipping an obstacle withholds the 498.9 bonus. That is why one XP is
not a circuit.

Inner gate `loc_2307`/`2308` name Gate at `(2998,3931,0)`. Course
membership is `z>3931`, `|x-2998|≤24`, same plane. Wolf pit is
same-scene (`RIDGE_FAIL`); obstacle pits teleport `z` by >2000.

Approach must stay south of Door 2309. Walking a north tile Opens the
door as a multi-tile transport and steals the ridge click.

289 wilderness differs only by stepping-stone `p_arrivedelay` and last
jump delay 1→0. Same XP, names, ids, skill gates.

Frozen approach tiles (not dests): pipe `(3004,3937,0)`, ropeswing
`(3005,3952,0)`, stone `(3002,3960,0)`, log `(3002,3945,0)`, rocks
`(2994,3937,0)`. Dests are rs2-relative to the selected loc tile
(pipe `loc_coord` z+9, and so on). Implementer must read those loc
tiles from selected maps or a one-shot snapshot dump. Source-only
absolute dests are not fresh proof.

No quest. Edgeville bank `(3094,3493,0)`. Default food Lobster 379.

### Brimhaven (m43_149 / agilityarena.rs2)

Varp **309** = `agilityarena_varbit` (`varp.pack` both worlds,
`transmit=yes`). Bits: 0 tagged, 1 paid, 2 eligible, 3 first-tag.
Clerk `Cap'n Izzy No-Beard` npc 437, op1 Talk-to, op3 Pay, fee 200
coins 995. Ladder down loc 3617 name Ladder / Climb-Down; dest packed
`3_43_149_53_54` → `(2805,9590,3)` (script platform 24, no dispenser).
Ladder up packed `0_43_49_56_57` → `(2808,3193,0)`. Entrance
`(2809,3194,0)`.

Ticket obj **2996**. Dispenser loc 3608 and poisondarts both name
`Ticket Dispenser` / Tag. First Tag on the hinted pillar sets bits 0+2+3
and mesbox "Tag the next pillar"; **no ticket**. Later Tags add
`agilityarena_ticket` 1. Timer rotates every **98** ticks. Entering the
mapzone clears tagged/eligible/first-tag; paid remains.

Pillar enum 0–23 packed coords match frozen `PILLARS` (e.g. 0 =
`3_43_149_9_10` = 2761,9546). Obstacle XP examples: ledge 16.0, log
12.0, wall 8.0, swing 20.0, plank 6.0, monkey 14.0, handholds 22.0
(needs 20). Several edges are level 1. Seed Agility 1 is enough to
leave platform 24 via Rope swing / Plank.

289 arena/clerk diffs are parrot chat only. Same fee, varp, ticket,
98-tick cycle, loc ids.

No quest. Steal restock is Thieving 20 stall / 40 guards. Boat fare 30.
Ardougne bank `(2655,3283,0)`.

## 3. Host APIs vs missing members

Already mapped and sufficient for core when the two gaps below are
filled: loc query/interact, npc Pay/Talk-to, ChatDialog continue/choose,
Bank open/deposit/withdraw/withdrawX, Skills xp/level/effective,
GameMessages, varp table (whole `Client.var`, so 309 is posted),
Traversal.walkResilient / walkTo, DirectNavigator.walkTo,
Reachability.walkable/canReach, ContinueDialog, Inventory Eat, Game
runEnabled via varp 173.

DeathRecovery `validate()` is idle until a new `oh dear` line. Core
does not induce death. Do not change it.

`Shop.sell` still throws. Steal/boat-sell cells stay out of this wave.

`reader.retaliateControls` is missing. Wildy `ensureRetaliateOff`
logs and returns. Non-fatal. Do not invent controls here.

**Missing, in scope, thin:**

1. Post existing client hint fields on the isolate snapshot. `reader.hintTile()`
   returns `{x,z}` when `hint_type==2` (server `hint_coord` types 2–6
   collapse to 2), else `null`. Never fabricate a pillar. Headless and
   native share the same post.
2. `actions.setRun(on)` queues existing `{op:'set-run', on}`. Do not
   duplicate host auto-run policy.

If LIVE throws `not impl: reader.hintTile` or `not impl: actions.setRun`,
that is a missing host capability, not a broken imported script and not
an invalid fixture.

If Agility 51 at the ridge, 0 coins at Izzy, or a 289 loc **name**
differs while the id exists under another name: fixture/content. Record
the 289 id; do not silently retarget.

If the loc is present, the click is sent, and XP never arrives: loc/settle
capability (same split as Gnome log).

Flax-style first-row loc bind remains a separate identity defect. These
course locs should be unique tiles; if LIVE shows a colocated wall,
reuse the existing loc-identity repair, do not write a planner.

## 4. Exact setups (before Start only)

Shared: `script_live_seed_steps`, mainland, `gold_script_nav` (600ms),
`start_catalog_step`, drain `advancestat` chat. Cheats are `give` /
`advancestat` / `tele`. No `give` of ticket 2996. No agility award after
Start.

### `wildy_agility` (core)

Inject: `acquireFoodAtStart=false`, `minFood=0`. Seed:
`advancestat agility 52`, drain, `give lobster 5`, tele
`(2998,3916,0)`. Baseline: south of Door, Agility 52, lobster 379 ×5,
agility XP = post-seed value, not on-course.

Card `WildyAgility`. DeathRecovery constructed but idle.

### `brimhaven_agility` (core)

Inject: `stealRestock=false`, `bankAtTickets=1000`. Seed: `give coins
1000`, `give lobster 10`, tele `(2809,3194,0)`. Agility 1 is enough;
do not seed 20/40 unless running steal. Baseline: surface entrance,
coins ≥ 200, tickets 2996 = 0, agility XP at Start, varp 309 paid bit
clear.

Card `BrimhavenAgility`.

## 5. Compact ordered witnesses

Reuse Gnome's chained `bank_fletcher_watch` arms under unchanged 180s
total. Each arm 150 ticks. Queued loc op without the world change fails.

### Wildy core

1. Ridge: agility XP ≥ 15 **or** chat `You skillfully balance across the ridge`,
   **and** tile `z>3931` same plane. Wolf-pit fail line is not a pass.
2. Pipe dest (selected loc 2288 z+9, radius 3) **and** XP > ridge.
3. Ropeswing dest **and** XP > pipe.
4. Stepping-stone dest **and** XP > ropeswing.
5. Log dest **and** XP > stone.
6. Rocks dest **and** agility XP ≥ **586** above Start (15+571.4, floor).
7. Further work: pipe dest again **and** XP ≥ **598** above Start.

Catalog independent witness: XP ≥ 598 and second pipe dest. Scenario
proof is that further-work arm.

Wolf-pit, high-z pit, `wrong side`, or timeout-without-XP fail. Script
`advance()` skip after two retries is not a lap.

### Brimhaven core

1. Fee: coins 995 down by 200 **and** varp 309 bit 1. Fee alone is not
   enough to stop.
2. In arena: tile in `x 2752–2815, z 9536–9599`, level ≥ 3 (plane 3
   platforms).
3. Movement: platform index changes after a named edge loc interact
   (or a walk-trap stand), **and** agility XP ≥ 1 from that hop.
4. First Tag: varp 309 bit 0 set **and** tickets 2996 still 0, with
   mesbox/chat about tagging the next pillar.
5. Ticket: tickets 2996 ≥ 1 after a later Tag (not a seed).
6. Subsequent work: another platform change **or** further agility XP
   after the ticket.

Catalog independent witness: ticket 2996 ≥ 1 **and** agility XP ≥ 1
**and** a post-ticket platform/XP delta. Scenario proof is that last arm.

A Tag while the hint is elsewhere (`You can only get a ticket when the
flashing arrow…`) is not progression.

## 6. Gold clock

Unchanged 180s / 150 ticks. Relog + tele already consume part of 180s.

Wildy anims are longer than Gnome. A full ridge+lap+pipe may miss 180s.
Brimhaven first Tag plus a 98-tick (~58.8s) wait plus chase plus second
Tag plus a further hop is the tightest cell.

If LIVE times out:

- Record the last ordered arm that passed and the tick/ms.
- Keep the gold bar.
- Do not treat a diagnostic as the core PASS.

Diagnostics (not acceptance):

| Name | Witness | Why it exists |
|---|---|---|
| `wildy_agility_ridge` | Arm 1 only | Clock/content split for ridge |
| `wildy_agility_first` | First on-course XP + dest | Loc interact without claiming a lap |
| `brimhaven_agility_enter` | Fee + plane 3 | Clerk/ladder without chase |
| `brimhaven_agility_hop` | Arm 3 only | Movement+XP without hint |
| `brimhaven_agility_tag` | First Tag, tickets 0 | Dispenser without 98-tick wait |

Implementer still **authors** the full core scenarios. Root LIVE decides
whether they finish. A timeout is FAIL, not a dim.

## 7. Option groups (inventory, do not permute)

### WildyAgility

| Setting | Default | This wave |
|---|---|---|
| loadout | blank | Idle. `scriptFood` fallback Lobster. |
| food | blank → Lobster | `wildy_agility_food_override`: `food=Shark` withdrawn. Not core. |
| foodWithdraw | 20 | Exercised only on food/death cells. |
| minFood | 1 | Core injects 0. |
| acquireFoodAtStart | true | Core injects false. `wildy_agility_food`: true, pack 0, `givebank lobster 40`, tele Edgeville, witness lobster 379 up then optional ridge. Bank withdraw is the option, not the circuit. |
| obstacleTimeoutTicks | 24 | Do not permute. |

Death + `walkBack` (food bank then `RIDGE_APPROACH`) is a supported
branch. Leave it idle here. Do not implement new recovery. Queued walk
is not recovery if that cell is ever run.

### BrimhavenAgility

| Setting | Default | This wave |
|---|---|---|
| loadout | blank | Idle. |
| foodWithdraw | 25 | Core seeds 10 in pack; no startup bank. |
| bankAtTickets | 1000 | Core leaves high. `brimhaven_agility_bank`: inject 1 after a real ticket (not a seed). Leave arena, Ardougne booth, tickets 2996 pack→bank, withdraw food/coins, re-enter (paid bit stays), another Tag/XP. Likely exceeds 180s; diagnostic/FAIL, not a timeout stretch. |
| stealRestock | false | Core false. True throws at Start if Thieving < 20; guards need 40. Cake/pickpocket/Shop.sell stay out. |

Do not permute every arena edge.

## 8. Mandatory current operations (core)

Wildy: `Locs` Door Open; five course loc first-actions; Skills.xp agility;
GameMessages ridge/wrong-side/pit; Traversal to approach/start tiles;
Inventory Eat if HP drops; PitEscape Climb-up only if a high-z fail
happens (not required to pass). DeathRecovery idle.

Brimhaven: Npc Pay (preferred) or Talk-to + choose `use the Agility Arena`
/ `Okay, here's 200`; Ladder Climb-Down; `reader.varp(309)`;
`reader.hintTile`; loc Tag; loc edge ops from `ARENA_EDGES`;
DirectNavigator stage; `actions.setRun`; Skills.xp; Inventory ticket
count. Climbing rope / exit ladder / Bank / Shop / steal idle on core.

## 9. Tests, matrix, native/TUI, ownership

Meaningful tests (behavior, not source snapshots):

- `crates/scenario`: names, inject, `start_script`, ordered proofs,
  `give` of 2996 is absent, agility 52 / coins 1000 / tele tiles.
- `crates/host-play --test catalog_boundary_live` (not LIVE): Wildy cycle
  rejects ridge-only and 571-without-further; accepts 598+second pipe.
  Brimhaven cycle rejects fee-only, first-Tag-without-ticket, ticket
  without hop XP; accepts ticket + subsequent hop/XP.
- `crates/script`: `hintTile` null without post, `{x,z}` when type 2;
  `setRun` queues `set-run`; missing member still throws rather than
  faking a pillar.

Minimum honest matrix: both catalogs × both revisions for
`wildy_agility` and `brimhaven_agility` (8 cells). Scripts are
byte-identical across catalogs; 274/289 content XP/ids/names match.
Option/diagnostic rows are extra and must not complete the cards.

Native/TUI: root after catalog PASS, same as Gnome. Screenshot only if
a tool actually reads the capture. Headless gold is not native proof.

Implementation file ownership (one handoff; orch serializes):

| File | Own |
|---|---|
| `crates/scenario/src/lib.rs` | **hotspot** — new scenarios/names only |
| `crates/host-play/tests/catalog_boundary_live.rs` | **hotspot** — cycle + CORE_SCENARIOS |
| `crates/script/src/shim/client_adapter.js` | `hintTile` + `setRun` |
| `crates/api/src/snapshot.rs` | **hotspot** — post hint fields only |
| isolate snap encode (`isolate_fb.rs` / `load.rs`) | hint fields only |

Do not edit DeathRecovery, shop.js, nav routers, frozen catalogs,
GnomeCourse, or STATE/ledger.

## 10. Support-matrix note

Matrix still lists WildyAgility `BLOCKED: missing death recovery
execution`. That row is stale relative to `04-capabilities-recovery.md`
(walkBack ABI exists; `validate()` false until death). Core does not
need death. Do not flip enabled status from this design. Root updates
the matrix after LIVE.

## Limits

Source dest math is not a live loc dump. Implementer must read selected
pipe/ropeswing/stone/log/rocks tiles before wiring dest proofs.
Historical single-step ridge/XP cells are not this proof. No LIVE was
run here.
