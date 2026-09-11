# Flax274 and Gnome radius-8 ownership after client9d loc refresh

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11. Kind: bounded read-only ownership audit for brief 73.
Not implementation, LIVE, fixtures, ledger, or an enabled-status change.
Root owns acceptance and any follow-up repair.

Read once: `AGENTS.md`, `docs/execution.md`, fail-closed-dispatch, current
STATE imported-script / ancillary-stub rules, and brief 73. Branch checked
first: `codex/rs2b0t-multirevision` (not `main`). Work was read-only except
this report and `docs/compat/evidence/flax-course-ownership/`. Concurrent
working-tree edits on this checkout are not the tested binary.

## Verdict

Two separate findings. Do not merge them.

1. **Revision-274 old-catalog FlaxPicker is a host loc-identity dispatch
   defect, not a foreign-script defect and not a proven stale loc-list
   defect.** Do not dim FlaxPicker. JS selects Ground Flax 2646 / Pick;
   `InteractReq::Loc` then binds the first snapshot row at that `(x,z,level)`.
   On the LIVE-stuck tile that first row is Wall 980 with no actions, so
   `ActionSpec::Label("Pick")` refuses `InvalidAction` while flax remains
   published and visible. Root's queued identity repair `t_c35281b3` matches
   this cause. Reachability is not the stall: 48 in-scope flax remain, and
   the overlay is picking the colocated fence tile.

2. **Old-catalog `gnome_course_radius` (`searchRadius=8`) is an imported
   AgilityBot search/resync option defect.** Do not dim default GnomeCourse.
   Do not invent a host walkable/walkTo/distance gap to hide it. Default
   radius-20 now completes XP94 plus the next log on both revisions and both
   catalogs on this binary. Radius-8 still re-syncs log → pipe after lap 1
   and hammers the exit side. Native distance freshness is established by
   that default-cell contrast. Recommend dimming only the radius-8 option
   row, both catalogs, both revisions, with the source/evidence below.
   Root changes enabled status.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Tested host | `b1cff8a7fdf6d18a5f7a3c7c23c6fa8a020e173b` |
| Tested client | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Isolated export | `.superpowers/review-exports/catalog-root-b1cff8a7` (2548 files) |
| Headless binary | sha256 `9eac5fa89ca78602368d3bd6d908d2465519dd14f88612316f230fb68ec8b0f5` |
| Campaign HEAD at write-up | `4b69c0e965333c55d5d67322b2ec6e425d9b1f30` (gitlink already client9d) |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| 274 engine | `4c95f87efe00b068cadbd229d94736626907bd1a` |
| 289 engine | `cc359656b4acd216ca452495874b6beba9a0ac75` |
| FlaxPicker.ts (both catalogs) | sha256 `e24b0d794e4bb83a0616a841999b28f036da47a48a3bd1e8825316616bbfd83a` |
| AgilityBot.ts (both catalogs) | sha256 `dee7c8885685724b0453097a0cf6a66d36a4d081e24157f29d35c742c518028f` |
| Kanban card | `t_26092c52` |
| Brief 73 | `docs/compat/briefs/73-flax-course-ownership-audit.md` |

Host/API/shim line hashes are the export files in
`docs/compat/evidence/catalog-headed/source-b1cff8a7.json`. Client
`world.rs` at gitlink 9d090ed is sha256
`eb8520694e4d4b7f…` / 47397 bytes (`extra-hashes.json`).

## 1. FlaxPicker 274 old-catalog

### LIVE cells (host b1cff8a7 / client 9d090ed)

| Cell | Exit | Witness |
|---|---|---|
| r274 / 100adccc flax_picker | 1 | `has_item_id(1779)>=28` not seen in 150 ticks; tick 161; tile `[2738,3441,0]`; six inv 1779; seven "You pick some flax." chats. Log sha256 `21814df4787d…` |
| r289 / 100adccc flax_picker | 0 | fill, `banked 28 Flax`, return WalkTo field, further Pick. Log sha256 `638cb640b606…` |
| r289 / 8e7d965b flax_picker | 0 | same family. Log sha256 `8f1af478a6d1…` |

No r274/8e7d965b flax cell exists on this binary. Scripts are byte-identical
across catalogs, so that missing cell is not a catalog-script split.

274 headless log after the first few unique tiles only retries
`(2737,3440)` and `(2738,3441)`. Overlay on the paired native diagnostic
is `picking Flax at (2737, 3440, 0)`, Held 6 / Free 22.

### Native diagnostic (root, preserved as observation only)

Path:
`docs/compat/evidence/catalog-headed/r274-flax-picker-100adccc-b1cff8a7-gpu-diagnostic/shots/2026-09-11T06-21-11_29226/`

- PNG sha256 `36499950540d621e…` (1004818 bytes). Read here: Seers flax
  field, actor visible, remaining plants visible, wooden fence along the
  field edge, inventory six flax, overlay targeting `(2737, 3440, 0)`.
  Last-frame counters were recorded by root (`pixmap0 tex3949`); this
  audit does not re-derive them.
- Snapshot sha256 `23ef181e8c0ba70c…` (3437484 bytes). Player tile
  `(2738,3441,0)`. 2241 loc rows. 48 flax tiles inside field scope 12.
  One flax+wall colocation in that scope: `(2737,3440,0)`.

Exact rows at the overlay target, sweep order wall then ground
(`rebuild_loc` `snapshot.rs` 1962–1976):

1. Wall id **980**, name empty, `actions: []`, `block_walk: true`, distance 1
2. Ground id **2646** name **Flax**, `actions: [null, "Pick"]`,
   `block_walk: true`, distance 1

Player tile `(2738,3441,0)` is Ground Flax 2646 only. Inv is six `[1779,1]`.
Flax is still in the native loc list. This frame is not “deleted flax still
published”.

### Dispatch chain (export b1cff8a7)

- Foreign `nearestFlax` (`FlaxPicker.ts` 115–127): `Locs.query().name('Flax').action('Pick')`
  within 12 of `(2741,3444,0)`, sort by player Chebyshev, first
  `Reachability.canReach(tile, { adjacentOk: true, maxSteps: 400 })`.
- Shim `Loc.interact` (`locs.js` 30–39): checks **this** row’s actions,
  then queues `{ op: 'loc', x, z, level, action }` with **no id / layer**.
- `InteractReq::Loc` (`shim/mod.rs` 740–747) has only `x,z,level,action`.
- Host-play (`lib.rs` 1842–1857):
  `snapshot.locs().iter().find(|l| l.tile.x == x && l.tile.z == z && l.tile.level == level)`
  — first row, no id, no action, no layer. Contrast: `OpenBooth` carries
  `id`; `Obj` filters name; `Npc` matches name.
- `Interactions::interact` (`interact.rs` 785–790): `operation_of(Wall, "Pick")`
  is `None` → `SendReason::InvalidAction`. `wrote` stays false. JS already
  returned true, so `Pick.execute` waits up to 6s on count/loc-gone and retries.

That is a dropped selected identity: the isolate chose Flax 2646 / Pick;
the host applied the wall sitting on the same tile. Fail-closed-dispatch:
this is our mapping bug, not a missing verb and not a broken import.

### What is not the 274 stall

- **Unrefreshed loc list.** Client `static_loc_generation` bumps on
  successful `add_scenery`, `del_loc` that removes a ground sprite, and
  `reset_map` (`world.rs` 531, 653, 157). API `rebuild_loc` dirties on
  scene gen, tile stamp, or that generation (`snapshot.rs` 1946–1955) and
  rebuilds all four layers. The diagnostic snapshot still contains 48
  in-scope flax including the target Ground 2646. Ghost-loc is not shown.
- **Missing coordinate reach.** `Reachability.canReach` / `walkable` /
  `canStep` are mapped on this binary (`reachability.js` 64–102). Loc
  tiles do not use entity row bits (`entityOnTile` only npc/ground); they
  fall through to posted flood bits. The script kept selecting the fence
  flax, so flood `adjacentOk` at that tile is true enough to win nearest.
- **Foreign FlaxPicker.** Byte-identical on both catalogs. 289 both
  catalogs fill/deposit/return/further-pick on the same host/client
  binary. 289 also clicks `(2738,3441)` and `(2737,3440)`, but 289 is not
  proven to colocate Wall 980 on that tile (no 289 paired snapshot). Do
  not invent a 274-only script defect or a revision gate. 274 always-delete
  content (brief / 03-world input-audit) can make a fence-corner trap
  visible sooner; it does not own the first-row dispatch.
- **Paint `Flax picked: 0`.** Overlay counter is a separate
  `inventory.changed` filter (`e.id !== -1`). Held is 6. Not the stall.

### Residual reachability (not the stall)

Unproven, and not required to explain this LIVE cell:

- Player is standing on a `block_walk` flax at distance 0 while the overlay
  targets the distance-1 fence flax. Own-tile `canReach` may be false
  because flood `reachable`/`reachable_adj` is not a loc-occupancy bit.
- If identity is repaired and 274 still only retries `(2737,3440)` after
  that Ground 2646 is actually gone from the snapshot, then re-open loc
  refresh / flood. That is a later proof, not this cause.

### Falsifiable next proof (root / identity card)

After selected-id or selected-action loc dispatch (JS already has `this.snap.id`):

- Repeat isolated 274 / 100adccc `flax_picker` on a hash-verified empty
  target of the repaired source. Expect `1779>=28`, Seers deposit, return,
  further 1779.
- If it still fails: dump the host-selected loc id/layer for each
  `InteractReq::Loc` versus the JS-selected 2646. First-row Wall 980 after
  repair would mean the repair missed. First-row Flax 2646 with no inventory
  change would move the question to engine delete / collision, not identity.

No host timeout, fixture, or FlaxPicker.ts change. Do not dim FlaxPicker.

## 2. GnomeCourse radius-8

### LIVE cells (same binary)

Default `gnome_course` (`searchRadius=20`) **PASS** all four catalog/revision
cells: ordered log / nets / rope / pipe, `lap 1 complete`, next
`Walk-across` at `(2474,3435,0)`, XP94 at `[2474,3429,0]`.

Old-catalog `gnome_course_radius` (`searchRadius=8`) **FAIL** both revisions:

| Cell | Exit | Sequence |
|---|---|---|
| r274 / 100adccc | 1 | full lap, `re-sync: step 0 (log balance) -> 6 (obstacle pipe)`, five `Squeeze-through` at `(2484,3435,0)`, fail tile `[2484,3437,0]`, chat `You can't enter the pipe from this side.` Predicate `stat_xp_gain(16)>=94` not seen. Log sha256 `027916c1b540…` |
| r289 / 100adccc | 1 | identical sequence. Log sha256 `dcd7d15a10ab…` |

No `not impl: Reachability.walkable` on b1cff8a7. Historical
`50f2be8a` radius-8 cells threw that missing mapping **after the same
re-sync** (r274 log sha256 `2f1a4f806b4b…`). Walkable is now mapped; the
remaining failure is the resync itself. `DirectNavigator.walkTo` is not
on this path (no reposition log).

Newer-catalog radius-8 was not re-run on b1cff8a7. AgilityBot.ts is
byte-identical across catalogs; 3be68eaf already failed radius-8 on both
catalogs with the same re-sync. Treat the option as broken on both
catalogs until a LIVE cell shows otherwise.

### Foreign source

`AgilityBot.ts` 34, 57, 82–83, 102–110, 139–180 (hash above):

- `DoObstacle.find` keeps locs with `l.distance() <= searchRadius` and
  `l.actions().length > 0`. `Loc.distance()` is the posted Chebyshev
  scalar (`locs.js` 22–23), refreshed on player tile change
  (`refresh_loc_distances` / `rebuild_loc`).
- After `lap 1 complete`, `step` wraps to 0 (log). `find('log balance')`
  fails at radius 8.
- `resyncTo` walks `new Set(courseNames())` in declaration order and
  accepts the first still-visible name. That is the pipe.

LIVE tiles: log last clicked `(2474,3435,0)`; fail/exit `(2484,3437,0)`.
Chebyshev `max(10,2) = 10`, which is `> 8`. Pipe click `(2484,3435,0)` is
Chebyshev 2 from the exit, so it remains visible. Default radius 20 sees
the log and walks it. That contrast is the distance-freshness proof
05e still required before assigning the option to the import
(`05e-location-world-fixtures.md` 28–32). It is now satisfied.

05e’s older “chebyshev 13” used packed dests; the b1cff8a7 fail tile
measures 10. Either value exceeds 8.

### Dim / reason scope

Dim **only** fixture/scenario `gnome_course_radius` (settings
`searchRadius=8`) on both frozen catalogs and both revisions.

Do **not** dim default `gnome_course`. Do **not** alter host distance,
flood, walkable, or walkTo to make radius 8 see a Chebyshev-10 log. Do
**not** special-case gnome tiles. Do **not** gate by revision. The
default card is a working mapping of supported host operations.

Suggested reason, if root dims:

`imported AgilityBot searchRadius=8 cannot see log (2474,3435) from pipe
exit (2484,3437) (Chebyshev 10); resyncTo selects obstacle pipe; host
loc/walkable/walkTo are not the failing verb (default radius-20 PASS on
b1cff8a7/9d090ed).`

### Falsifiable next proof

None required for ownership. Control already exists: default radius-20
PASS on this binary. A radius-8 cell that `Walk-across` `(2474,3435)`
after lap 1 without host changes would falsify the foreign assignment.
Do not run that as a repair attempt.

## Limits

- No LIVE, no product edits, no ledger/enabled writes, no archive extract.
- 274/8e7d965b flax and 8e7d965b radius-8 were not in the b1cff8a7 batch.
- 289 flax was not given a paired loc snapshot; 289 PASS is not a claim
  that 289 lacks Wall 980.
- Engine flax always-delete versus probabilistic delete was not re-hashed
  from engine trees in this audit; the native snapshot already shows flax
  2646 present at the 274 stall tile.
- This card does not implement `t_c35281b3`.
