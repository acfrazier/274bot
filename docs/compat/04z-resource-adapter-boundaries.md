# Resource adapter boundaries: Traversal.preload and native tool facts

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 10:05 UTC. Kind: bounded read-only architecture/fidelity
design for brief 102. Not implementation, LIVE, fixtures, ledger, STATE,
dim, timeout, or a foreign Navigator/Tools/Gatherer copy. Root owns
acceptance, any LIVE snapshot, and commit of this evidence. No routine
reviewer card.

Read once: `AGENTS.md`, `docs/execution.md`, brief 102. Branch checked
first: `codex/rs2b0t-multirevision` (not `main`). Frozen source is
`.superpowers/review-exports/catalog-root-28d97f38` (host
`28d97f38f01b854b6a3c62fdd09c40caf29fcf99`, client
`9d090ed04957e4efc254f073cda97bc5510ca72b`), not campaign WIP
`7c2eef6eb`. Concurrent cake/named-bank/UI/station workers were not
touched. Work was read-only except this report and
`docs/compat/evidence/resource-adapter-boundaries/`. Raw 28 cells stay
in `docs/compat/evidence/catalog-harness/`.

## Verdict

**First owner for all six old-catalog GnomeMagicChopper cells is shim
`Traversal.preload` throwing `not impl`. First owner for both CoalTrucks
cells is shim `Tools.bestPickaxe` throwing `not impl`.** Neither is a
foreign-script regression, missing nav pack, or missing loc/item opcode.
No resource action was accepted. Do not dim. Do not clone `Navigator`,
`WalkExecutor`, or `Tools.ts`. Do not copy a foreign policy table.

`Traversal.preload` is a void, unawaited `Navigator.start()` in the
frozen catalog. The host already binds `NavWorld` at profile/template
construction, before isolate Start. A void no-op is faithful to that
already-performed native preparation. It must not start a worker, import
a foreign router, or invent arrival. Unbound nav is already fail-closed
on `ScriptWalkArm::route_with_radius` (`world` `None` → false). Preload
must not fake readiness there either.

`bestPickaxe` / `bestAxe` / `canWieldTool` / `AXES` are selected-revision
tool facts with thin JS callback mapping. Existing `tools.js` is not that:
`AXES` is bronze-first with `id: 0`, `bestAxe` ignores the availability
callback's exclusivity, `bestPickaxe` and `canWieldTool` throw. Native
Rust owns identity and use-versus-wield. JS only forwards the caller's
predicate and returns `string | null` / `boolean`.

Generated `game-data/{274,289}.json` has item id/name/`wear_position: 3`
and no skill fields. Use and wield live in selected content, not ObjType
and not the foreign `Tools.ts` numbers when they disagree with content.

## Classification against brief 102

| | Question | Applies? |
|---|---|---|
| Gnome first owner | Shim `Traversal.preload` | **Yes.** `onStart` line 667, before status log or gear. |
| Coal first owner | Shim `Tools.bestPickaxe` | **Yes.** `heldPickaxe` from `onStart`/`loop` `hasPickaxe`. |
| Preload no-op faithful to native prep | | **Yes**, when `NavWorld` already bound. Not a readiness wait. |
| Native readiness opcode required | | **No** for this contract. Foreign `start()` is void and unawaited. |
| Copy foreign Navigator/worker | | **No.** Unauthorized. |
| Copy foreign `PICKAXES`/`AXES` tables | | **No.** Native selected content owns tiers. |
| Held tool implies wieldable | | **No.** Seeded Attack 1 cannot wield steel (Attack 5). |
| Missing loc-chop / mine opcode | | **Not first.** `Loc.interact` already queues. Never reached. |
| Dim | | **No.** |

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Frozen host | `28d97f38f01b854b6a3c62fdd09c40caf29fcf99` |
| Frozen client | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Isolated export | `.superpowers/review-exports/catalog-root-28d97f38` |
| Campaign HEAD at write-up | `7c2eef6eb8db6b01933c333aa64cf5f68169d8d2` (WIP; not the audit source) |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 102 SHA-256 | `4a2213250788a763a47ef90d14969d9b31326ed3d68ab40d576f56dc4ed874a1` |
| Kanban card | `t_44e1ad0c` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| 274 engine / content | `4c95f87efe00b068cadbd229d94736626907bd1a` / `000c19997e07206131bcb3c884265840efce416d` |
| 289 engine / content | `cc359656b4acd216ca452495874b6beba9a0ac75` / `92649430fcbc83538d8c4367ecb96cee1a67a944` |
| 274 ObjType.ts | sha256 `61e07de093787c88a00a7224ed9ecffa9be9c1770176650b30926e69fd0ea42b` (manifest decoder) |
| Traversal.ts (both catalogs) | sha256 `984fda1becb12276c46e61e8588f4bd5d51085f968849ff202034b49c44f5d6d` |
| Navigator.ts (both) | sha256 `9dd2a86fcdc2178785a30212e8159732ca44ac3fdd35dd8de8d6f220edca61a4` |
| Tools.ts (both) | sha256 `34302c46870de39103e2ecbb698a2406122cde765b42157e6b085a52aed2af4e` |
| GnomeMagicChopper.ts (both) | sha256 `3b0d034c761f9b98361561627817a6d806d359f659000cbccdf22e25772471e1` |
| CoalTrucks.ts (both) | sha256 `d20112f0bfb44612ae53e0c56009c6b3e50808375c2a54dc43ed2160e55fcf36` |
| Frozen `traversal.js` | sha256 `5b272833520f908b02e1aae3c75577829ab1c8ad843082659aca0b6148980274` (byte-identical to 78) |
| Frozen `tools.js` | sha256 `d8e107799c38f011567291c02f0169292f678a3fc3b20b3aabc1a2447e24bde8` |
| 274 game-data JSON | sha256 `d310498c02aff231d4bd3a45095c9bb5d89007f899ada70f7b02863410b322c1` |

`cmp` 0 on Traversal/Navigator/Tools/Gnome/Coal between catalogs.
Machine-readable copies: `evidence/resource-adapter-boundaries/{refs,hypotheses,facts}.json`.

## 1. Eight isolated cells (immutable receipts)

Old catalog only, as retained. Newer-catalog resource cells were not in
this batch.

| Cell | Tick | Error | Baseline |
|---|---|---|---|
| r274 gnome_chop | 35 | `not impl: Traversal.preload` | 2372,3425,0; WC 75; steel axe 1353 in pack; equipment empty |
| r289 gnome_chop | 33 | same | same seed; 289 models.bin skip is unrelated |
| r274 gnome_fletch_short | 34 | same | |
| r289 gnome_fletch_short | 37 | same | |
| r274 gnome_fletch_long | 38 | same | |
| r289 gnome_fletch_long | 34 | same | |
| r274 coal_trucks | 30 | `not impl: Tools.bestPickaxe` | 2582,3481,0; Mining 30; Attack 1; steel pick 1269 in pack; equipment empty |
| r289 coal_trucks | 34 | same | Attack 0 in the 289 snapshot; still Mining 30 / pick 1269 |

Gnome never printed `GnomeMagicChopper, magics at …`. Coal printed the
mine/truck/Seers banner and the giant-bat note, then died on
`hasPickaxe()` → `bestPickaxe`. No chop, mine, truck, or bank action.

## 2. Traversal.preload contract

Frozen `Traversal.preload(): void { Navigator.start(); }`.
`Navigator.start()` is idle-only, fire-and-forget: spawn
`navworker.js`, load `collision.lcnav.gz`, post `init`. It does not
await `ready`, does not return a boolean, and `findPath` later fails
closed if the worker never comes up. Scripts do not await preload.

Host start paths, all before isolate spawn:

| Path | When `NavWorld` binds |
|---|---|
| Panel | `ServerProfile::load_nav` during bind; `SharedClientTemplate::load` copies `profile.world()`; `script_start_selected` / `script_start_load` afterwards |
| TUI | Same template load (`bin.rs` “single `load_pack`”); `script_start` afterwards |
| host-play catalog | `selected_profile` → template world; isolate on Start |
| Legacy `Play::new` | `NavWorld::load_pack(&default_pack_path()).ok().map(Arc::new)` at construction |

Honest unbound / failed prep (already exist; do not invent new ones):

- Missing non-bundled pack → `NavAvailability::Unavailable`, `world: None`
- Missing bundled pack → `Err`
- Legacy `load_pack` failure → `world: None` via `.ok()`
- `ScriptWalkArm::route_with_radius`: `self.world.as_ref()` `None` → `false` (comment: `ctx.walk` refuses to arm)

A void no-op therefore matches foreign `start()` and the host's
already-performed bind. Throwing `not impl` is the defect. A native
“nav ready” fact is not required for this call. Do not wait, do not
tighten WalkNear, do not lengthen timeouts, do not import the foreign
worker.

## 3. Selected tool use vs wield

ObjType (274 and 289) decodes `wearpos` only. Generated rows: steel
pick 1269 and steel axe 1353 both `wear_position: 3` (righthand), no
skill. Use/wield are selected **content**, identical on both revisions
for these objects (`pickaxes.obj` / `axes.obj` / `pickaxe_checker.rs2`
/ `tier5.rs2` `cmp` 0). 289 `woodcut.rs2` differs elsewhere; the live
axe-checker still has no woodcutting gate.

**Use (gather):**

- Pickaxes: `pickaxe_checker` compares `stat(mining)` to
  `oc_param(*, levelrequire)` and accepts worn-or-inv, best-first
  rune→bronze. Steel `levelrequire` is **6**. Bronze/iron params are 0
  (foreign `Tools.ts` wrote 1; at real Mining ≥ 1 both agree).
- Axes: `woodcutting_axe_checker` comments “no wc req in 2004”, WC
  gates are commented out, live path is worn-or-inv best-first with
  **no woodcutting test**. `axes.obj` `param=levelrequire` (steel 6,
  mithril 21, adamant 31, rune 41) is **not** a WC use gate and **not**
  the Attack wield number.

**Wield (opheld2, not `try_equip`):**

`try_equip` does not read `levelrequire`. Wield is
`content/scripts/levelrequire/scripts/tier*.rs2`:

| Item | id | Use | Wield Attack |
|---|---|---|---|
| Bronze pickaxe | 1265 | mining param 0 | tutorial equip, not Attack-gated |
| Iron pickaxe | 1267 | 0 | 1 |
| Steel pickaxe | 1269 | **6** | **5** |
| Mithril pickaxe | 1273 | 21 | 20 |
| Adamant pickaxe | 1271 | 31 | 30 |
| Rune pickaxe | 1275 | 41 | 40 |
| Bronze axe | 1351 | none | tutorial equip |
| Iron axe | 1349 | none | 1 |
| Steel axe | 1353 | none | **5** |
| Black axe | 1361 | none | 10 |
| Mithril axe | 1355 | none | 20 |
| Adamant axe | 1357 | none | 30 |
| Rune axe | 1359 | none | 40 |

Do not treat `oc_param(levelrequire)` as wield for picks or as use for
axes. Do not assume a held steel tool is wieldable at Attack 1. Coal
never calls `canWieldTool`; mining from the pack is legal. Gnome calls
it and must keep the steel axe in the pack when Attack < 5.

Foreign `AXES` omits black axe. Native woodcut checker includes it.
Do not copy that omission.

## 4. Existing `tools.js` is not a valid mapper

- `AXES` bronze→rune with `id: 0`. Gnome `gearHasSteelOrBetter` uses
  `AXES.findIndex`; bronze-first **inverts** steel-or-better (accepts
  bronze/iron, skips mith/addy/rune).
- `bestAxe` walks reverse names (accidentally best-first) then falls
  back to inventory when the callback misses. Foreign
  `bestFromTiers` uses **only** `available(name)`. Gnome's bank-only
  callback at the Bob restock path would leak a held axe.
- `bestPickaxe` / `canWieldTool` throw. Coal dies immediately.
- `declared_surface.js` still throws `bestAxe`; live `Tools.js` is
  `shim/tools.js` via `mod.rs`. Do not “fix” the unused declared stub
  instead of `tools.js`.

JS remains thin: iterate native best-first facts, call the supplied
predicate, return the first usable name or `null`. Preserve argument
order `(level, available)` / `(name, attackLevel)`.

## 5. Next ordinary required operations (bounded)

After these two first owners, ordinary core does **not** need a
Gatherer rewrite. Already mapped and reached next: `walkResilient` /
`walk-near`, `Locs.query`+`Loc.interact`, `Equipment.equip`/`contains`,
`Bank.*` used by gnome gear, `Inventory.useOn`, `GameMessages`,
`Game.animating`, `Shop.isOpen`, `ChatDialog.canContinue`/`isMakeMenu`.

| Required for ordinary gnome_chop | Status |
|---|---|
| `Traversal.preload` void no-op | **this slice** |
| `bestAxe` + `AXES` best-first real ids | **this slice** (exists, wrong) |
| `canWieldTool` | **this slice** |
| Upstairs booth open, deposit, withdraw, close, stair climb, magic Chop | already queued/mapped; not reached |
| Death / Port Sarim / Bob / SneakyArdougne / missing-knife stop | **ancillary**; stub per user policy |

| Required for ordinary coal_trucks (mine/truck/further, not Seers) | Status |
|---|---|
| `bestPickaxe` | **this slice** |
| `canWieldTool` | not called on this path |
| Mine loc, `useOn` truck, `walkResilient` radius 1, further mine | already mapped; not reached |
| Seers haul/bank/return | **not** this fixture's ordinary core |

Fletch Make-X (`ChatDialog.makeX` still `not impl`) stays on the existing
Make-X runtime queue. `Traversal.remaining` / `requestRepath` /
`toolRestockPlan` / `bankHasBetterGatherTool` are not first ordinary
calls. Do not infer foreign breakage from those gaps.

## 6. Bounded implementation proposal

1. `traversal.js` `preload()`: return void. No worker, no ready wait, no
   WalkNear change. Optional debug only if nav is already bound; never
   throw; never claim a path.
2. Unique `crates/api/src/gather_tools.rs` (not `content.rs`, not a
   foreign table): per-revision best-first pickaxe/axe facts from the
   selected content above (id, display name, use skill+level, wield
   Attack). Slot isolation is the existing selected `Arc` at isolate
   start; do not copy onto snapshots.
3. Rewrite `tools.js` as callback mapping over those facts.
   `bestPickaxe`/`bestAxe` must not consult inventory except through
   `available`. Export `AXES`/`PICKAXES` best-first with real ids.
4. Publishing the fact JSON through `content_json` in `shim/mod.rs` is a
   **serialized insertion after cake97** (that function owns
   `__rs2b0t_host.content`). `traversal.js` preload does not need that
   hop. Do not land gather facts in `api/content.rs`.
5. Minimum tests (behavior, not source snapshots):
   - `bestPickaxe(30, steel)` → `"Steel pickaxe"`; `bestPickaxe(5, steel)` → `null`
   - `bestPickaxe(1, bronze+steel)` → `"Bronze pickaxe"`
   - `bestAxe(1, rune+steel)` → `"Rune axe"` (no WC gate)
   - bank-only callback does not return a held-only name
   - `AXES` steel-or-better index includes mith/addy/rune, excludes bronze
   - `canWieldTool("Steel pickaxe"|"Steel axe", 1)` false; `(5)` true
   - preload does not throw
   - 274 and 289 facts agree for these ids

Do not claim live acceptance. Root decides dispatch.
