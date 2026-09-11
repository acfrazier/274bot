# VialFiller East bank-return ownership on frozen B1

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11. Kind: bounded read-only ownership audit for brief 76.
Not implementation, LIVE, fixtures, ledger, or an enabled-status change.
Root owns acceptance and any follow-up repair or dim.

**Root qualification, 07:14 UTC:** the mechanical approach trace below is
retained, but the imported-defect verdict is not accepted. Frozen foreign
Bank.ts470-510 `openNearest` includes reachable counter selection/walking and
an adjacent retry; our bridge maps it to a lower-level geometric-adjacent
OpenBooth attempt. West success and unchanged low-level host code do not prove
an imported caller defect. Brief79 reviews existing native bank-access
composition and any missing ordinary banking capability. East remains
unresolved and enabled; no option dimming or host behavior change follows
from this audit alone.

Read once: `AGENTS.md`, `docs/execution.md`, fail-closed-dispatch, current
STATE imported-script / ancillary-stub rules, and brief 76. Branch checked
first: `codex/rs2b0t-multirevision` (not `main`). Work was read-only except
this report and `docs/compat/evidence/vial-bank-return-ownership/`. Concurrent
working-tree edits on this checkout are not the tested binary. Mid-run orch
note: after `2f1bd9cf` adjacent West pre-Start correction, old-catalog West
PASS on both revisions; newer-catalog West also finished PASS on that binary.
That contrast does not repair or requalify East.

## Verdict

**Old-catalog `vial_filler_east` on both revisions is an imported VialFiller
Falador East option defect, not a host banking regression, not loc-identity,
and not an invalid East fixture.** Do not dim default `vial_filler`
(Falador West). Do not change OpenBooth approach-walk, walk timeouts, nav
radius, or host door dispatch to accommodate the East option.

The East cells fill 28 actual water vials. First bank open from the interior
stand is a real Use-quickly. Return `WalkNear` radius 3 to stand `(3013,3355)`
arrives outside at `(3010,3352)` (Chebyshev 3, within the imported radius).
`Bank.openNearest` then hammers booth `(3011,3354)` id 2213. Host OpenBooth
sees Chebyshev 2 and substitutes `WireCommand::Walk` toward the geometric
adjacent tile; it never sends Use-quickly. No Close, no fresh bank generation,
deadline tile stays `(3010,3352)` with a full water pack.

West is the control: same `openNearest` / OpenBooth / `walkResilient`
radius 3 on the same banking code. After the 2f adjacent-seed fixture fix,
West deposits the 28 water vials, restocks empties, and fills again on both
catalogs and both revisions. Two `could not open` retries on West are the
5 s snapshot wait during approach, not a timeout-equals-bug inference; East
retries until the 150-tick deposit watch. `2f1bd9cf` does not touch
`interact.rs` or `bank.js`.

Recommend dimming only the Falador East option / `vial_filler_east` row,
both frozen catalogs, both revisions. Root changes enabled status.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Tested host (East + West-prep) | `b1cff8a7fdf6d18a5f7a3c7c23c6fa8a020e173b` |
| Tested client | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Isolated export | `.superpowers/review-exports/catalog-root-b1cff8a7` |
| Headless binary (b1) | sha256 `9eac5fa89ca78602368d3bd6d908d2465519dd14f88612316f230fb68ec8b0f5` |
| West contrast host | `2f1bd9cf75bfcff0900eb9e029deb98433f5a6d3` (scenario adjacent seed only) |
| West contrast binary | sha256 `34a4c5f3418898a78832859dab8b268d3129e5a179659b1f2475fc9706cf9d20` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| 274 engine | `4c95f87efe00b068cadbd229d94736626907bd1a` |
| 289 engine | `cc359656b4acd216ca452495874b6beba9a0ac75` |
| VialFiller.ts (both catalogs) | sha256 `58f95dca5a65bfc6048c1da6a3eda8a8153b90979c0a30c237e76ed437c8bd4c` |
| VialFillerLogic.ts (both catalogs) | sha256 `e704f90e6e6b96a44a408edad53fb45c92f64b682505b0b2637627e90767bfeb` |
| Kanban card | `t_715971a2` |
| Brief 76 | `docs/compat/briefs/76-vial-bank-return-ownership.md` |

Host/API/shim line hashes are the export files in
`docs/compat/evidence/catalog-headed/source-b1cff8a7.json` (also copied in
`evidence/vial-bank-return-ownership/extra-hashes.json`). `cmp` of both
catalog VialFiller.ts / Logic.ts is identical (exit 0).

## 1. East cells (host b1cff8a7 / client 9d090ed)

| Cell | Exit | Witness |
|---|---|---|
| r274 / 100adccc vial_filler_east | 1 | fill 28 × 227; `fresh_bank_item_id(227)>=1` not seen in 150 ticks; tick 290; tile `[3010,3352,0]`; no return Close. Log sha256 `48daff95b339…` |
| r289 / 100adccc vial_filler_east | 1 | same family; tick 286; tile `[3010,3352,0]`. Log sha256 `7ae913de1cee…` |

No r274/r289 8e7d965b East cell exists on this binary. Scripts are
byte-identical across catalogs, so that missing cell is not a catalog-script
split.

### Exact call sequence (274 East; 289 matches)

Frozen `VialFiller.ts` `bankLeg`: `walkTo(this.bankStand)` then
`Bank.openNearest('Bank booth', 'Use-quickly')`. `walkTo` early-outs at
Chebyshev ≤ 4 and otherwise `Traversal.walkResilient(dest, { radius: 3,
attempts: 2, timeoutMs: 60_000 })`. `BANK_STANDS['Falador East']` is
`(3013,3355,0)`.

Shim: `Bank.openNearest` is `openBooth(undefined, name, op)`
(`bank.js` 347–348). No stand walk. Named loc filter + distance sort, then
`op: 'open-booth'` with posted `x,z,level,id,name,action`. Wait is
`waitSnapshotAfter(generation, 5000)`.

Host: `InteractReq::OpenBooth` → `open_named_booth_at`
(`host-play` 1585–1597, `interact.rs` 947–1002). If Chebyshev to the loc
> 1, **return `self.walk(dest)`** where dest is the signum-adjacent tile
on the player’s side. That is `WireCommand::Walk`, not Use-quickly.

274 log, start inside the seed stand:

1. Baseline tile `[3013,3355,0]`, `bank_generation` 2, pack empty.
2. Line 19: `OpenBooth { x: 3012, z: 3354, id: 2213, name: Bank booth,
   action: Use-quickly }`. Chebyshev from stand is 1, so this **is** the
   loc op. Line 25 `Close` / `shim-close sent`. Withdraw/close succeeded.
3. Line 27: `WalkNear { 2949,3381, radius: 3 }` → Arrived `(2952,3378)`.
4. Line 197: `filled 28 Vial of water`.
5. Line 198: `WalkNear { 3013,3355, radius: 3 }`. Nav
   `route_with_radius` (`host-play` 3229) aims reachable `(3010,3352)`
   radius 0. Line 304 Arrived `(3010,3352)`. `transport=false`. Chebyshev
   to the stand is 3 ≤ imported radius 3, so walk reports success.
6. Lines 306–414: eleven `OpenBooth { 3011,3354, id: 2213, Use-quickly }`
   interleaved with `could not open the bank — retrying`. **No**
   `[nav-walk]` after those OpenBooths (approach is raw Walk, not nav).
   **No** second Close. Player never leaves `(3010,3352)`.
7. Step 13 FAIL: `fresh_bank_item_id(227)>=1` at tick 290, tile
   `[3010,3352,0]`, inventory still water vials.

Nearest-booth split is geometric, not identity loss: from interior
`(3013,3355)` the nearest matching 2213 is `(3012,3354)`; from
`(3010,3352)` it is `(3011,3354)`. Both rows are `Bank booth` /
`Use-quickly` / id 2213. This is not Flax Wall 980 / first-row loc bind
(`t_c35281b3` remains a distinct colocated-id repair; not these files).

### Approach walk versus bank operation

First interior OpenBooth: Chebyshev 1 → Use-quickly sent → bank actually
opens → Close. That is a bank operation.

Return OpenBooth: Chebyshev 2 → `interact.rs` 990–997 substitutes Walk to
`(3011+signum(3010-3011), 3354+signum(3352-3354))` = `(3010,3353)` and
does **not** send Use-quickly. The actor stays at `(3010,3352)` through
the 5 s generation wait, then the script retries. Eleven retries, zero
bank ops. Do not read the 5 s wait as an imported timeout bug: West uses
the same wait and recovers.

Reusable host semantic: OpenBooth refuses to fire Use-quickly until
Chebyshev ≤ 1, then walks geometrically. Preserve it. Do not send loc-op
from range, do not special-case East tiles, do not clone a foreign door
router, do not widen/narrow radii or waits to make East green.

## 2. West contrast (preparation versus return)

| Cell | Host | Exit | Witness |
|---|---|---|---|
| r274 / 100adccc vial_filler | b1cff8a7 | 1 | **pre-Start** `fresh_bank_item_id(229)>=56` at `[2946,3368,0]` tick 161. No script Start. Log sha256 `f05ec81977da…` |
| r289 / 100adccc vial_filler | b1cff8a7 | 1 | same prep failure (log sha256 `98499dc55da7…`) |
| r274 / 100adccc vial_filler | 2f1bd9cf | 0 | fill 28, return OpenBooth `(2945,3367)` id 2213, two `could not open` then Close, further fill at `[2949,3380,0]`. 96.6 s. Log sha256 `0db5f1dd7cbd…` |
| r289 / 100adccc vial_filler | 2f1bd9cf | 0 | same family. 98.239 s. Log sha256 `55d5c09d105a…` |
| r274 / 8e7d965b vial_filler | 2f1bd9cf | 0 | fill 28, two retries, PASS further fill. 100.612 s. Log sha256 `e1a2ba0c43a5…` |
| r289 / 8e7d965b vial_filler | 2f1bd9cf | 0 | same family. 100.047 s. Log sha256 `d86f07b7e2da…` |

`2f1bd9cf` vs `b1cff8a7`: scenario/docs/tanner/gitlink only.
`crates/api/src/interact.rs` and `crates/script/src/shim/bank.js` are
unchanged. Root’s adjacent West seed (3368 vs a 3369 approach-walk accept)
fixes **preparation**, not East return.

West return `WalkNear { 2946,3369, radius: 3 }` arrives `(2943,3372)` —
also outdoor, also radius 3 — then OpenBooth `(2945,3367)` eventually
opens because those booths are usable from the plaza. Same imported
radius, same host approach-walk, different bank geometry. That is why
West is not an East repair and why East is not a host OpenBooth
regression.

## Dim / reason scope

Dim **only** fixture/scenario `vial_filler_east` (settings
`bank=Falador East`) on both frozen catalogs and both revisions.

Do **not** dim default `vial_filler`. Do **not** alter host OpenBooth
Chebyshev-adjacent Walk, `walk-near` radius handling, door transport, or
`Bank.openNearest` identity posting to make East enter. Do **not**
special-case `(3010,3352)` / `(3011,3354)`. Do **not** gate by revision.
The default card maps supported host operations (West PASS on 2f, both
catalogs, both revisions).

Suggested reason, if root dims:

`imported VialFiller Falador East walkTo radius 3 / openNearest arrives
outside at (3010,3352) and never fires Use-quickly on booth (3011,3354);
first interior open and 28-fill work; default Falador West bank-return
PASS on 2f1bd9cf/9d090ed (both catalogs, both revisions).`

## Falsifiable next proof

None required for ownership. Control already exists: default West PASS
on 2f after adjacent seed, with unchanged OpenBooth/walk. An East cell
that returns inside Chebyshev ≤ 1 of a Use-quickly booth **without** host
OpenBooth/walk/door changes would falsify the foreign assignment. Do not
run that as a repair attempt. A host loc-op-from-range or door-aware
bank approach would be a new authorized capability, not this audit.

## Limits

- No LIVE, no product edits, no ledger/enabled writes, no archive extract.
- 8e7d965b East was not in the b1cff8a7 batch. Scripts are identical, so
  that gap is not a catalog split.
- No loc snapshot dump was taken at the East stall tile; booth ids/names
  are taken from the BOT_DEBUG OpenBooth lines (2213 / Bank booth /
  Use-quickly), which match the successful first open.
- Walk dest `(3010,3353)` after return OpenBooth is inferred from
  `interact.rs` 990–997 plus the logged player/booth tiles. The log does
  not print SendResult; evidence that Use-quickly never fired is the
  absent Close / absent generation bump / stuck tile, not a printed
  refuse reason.
- This card does not implement loc-identity repair `t_c35281b3`.
- Newer-catalog West PASS on 2f is contrast only; it does not requalify
  East.
