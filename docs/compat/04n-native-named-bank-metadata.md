# Native named-bank metadata for catalog routes

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 08:19 UTC. Kind: bounded read-only architecture/fidelity
design for brief 85. Not implementation, LIVE, fixtures, ledger, STATE,
or an enabled-status change. Root owns acceptance and any follow-up hop.

Read once: `AGENTS.md`, `docs/execution.md`, fail-closed-dispatch, current
STATE imported-script rules, brief 85, `04k`/`04m`/`04-generated-game-data`,
and the named host/catalog/packed files at HEAD. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Work was read-only except this
report and `docs/compat/evidence/named-bank-metadata/`. Concurrent
working-tree edits on this checkout are not the tested binary. Shared
host-play/runtime files belong to hostile observation `t_eeea20f0` then
special/teleport/shop/Make-X/fire. `crates/script/src/shim/bank.js` belongs
to named-approach `t_4948af53`. They were not edited.

## Verdict

**Both isolated675 old-catalog MuleCrafter Air cells fail after Start
because our `BANK_LOCATIONS` export is an empty array.** Catalog
`MuleCrafterLogic.bankTile('Falador East')` does `BANK_LOCATIONS.find`
and throws `bankTile: unknown bank 'Falador East'`. That is a missing
host mapping, not a foreign defect. Do not dim the mule cells. Do not
fabricate a free-floating Falador row. Do not clone `BankLocations.ts`.

The smallest native source is a **host-owned alias table** of catalog
argument names onto **packed `bankbooth` loc tiles plus a distinct
walkable stand tile**, posted once on `__rs2b0t_host.content` before
catalog modules evaluate. `BANK_LOCATIONS` readers see `{name, tile}`
where `tile` is the stand. Unknown or packed-missing names stay absent
so `find` returns undefined and `bankTile` keeps throwing. Packed
`world.banks()` and live `nearest_booth` stay unnamed `"Bank booth"`
facts. Closest-bank policy does not change.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `9407ba77c57dad5e8603d051142e9c7c62c3ef39` |
| Client gitlink | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 85 SHA-256 | `9282b2e8012ae07ba40b3e6e79ca04f292ce891813029fb7c2f3e25f9a789442` |
| Kanban card | `t_0541f9be` |
| Frozen B1 harvest | host `6750713b80ecfb8ad5036f3ffb0564459f67a856` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| 274 content | `000c19997e07206131bcb3c884265840efce416d` |
| 289 content | `92649430fcbc83538d8c4367ecb96cee1a67a944` |
| 274 nav pack | `05db24743e9f549ced16c1f00b87c30a390d3aaec391815da3f3563130b3bcd4` (68 stands) |
| 289 nav pack | `131db92e32eddcb08148909d477589544320fe7e34a422e8004e97e888032924` (68 stands) |
| Packed booth tiles 274 vs 289 | `cmp` equal (`input-audit-*.json` baked_stands tiles) |
| `bank_locations.js` | sha256 `b0f9116d402236f40763ad577ed376ae8580853c54c8de67554ee805140d684d` |
| BankLocations.ts (both; `cmp` 0) | sha256 `03ef4db685db466c4607f39824761f73c8c8d782f23301cc3761e1e2506625c6` |
| MuleCrafterLogic.ts (both; `cmp` 0) | sha256 `d9cc408c1857a02332e3338e1ae1e956dea51c7c191d4a66af81be6e5108251a` |
| MuleCrafter.ts (both) | sha256 `bf745db4c0a3df22406b49c8a8b716b10e0594302853d80ca6f38862d05dc1f9` |
| RuneCrafter.ts (both; `cmp` 0) | sha256 `a944049b42ee5adf882aaf119fb7b7f4604b63e2d07ff34e2db6dbf9e3954843` |
| runeCraftLocations.ts (both; `cmp` 0) | sha256 `40db500c2c081a0978738796fcce62965bd39ac70d0995345e15443d4042f221` |
| r274 mule 100adccc log | sha256 `760234bca946e403d83a79f099f76f140207aab3410ae580698ce87b06c4202d` |
| r289 mule 100adccc log | sha256 `4bc3e3fb63c0e6ae6ede6f39c383dcc72f10c2833f203fc8e4993228162a8f9a` |

Machine-readable copies: `evidence/named-bank-metadata/{refs,packed-booths-274-eq-289,alias-verify}.json`.

## 1. What failed

Isolated mule cells (old catalog, both revisions, harvest `6750713b`):

1. Identity binds the selected nav pack (274 `05db2474…` / 289 `131db92e…`).
2. Preparation teles to Falador East plaza. Baseline tile is `[3012,3355,0]`,
   `ingame && scene_state==2`.
3. Start loads catalog `MuleCrafter.ts` plus sibling `MuleCrafterLogic`.
4. `onStart` line 86: `this.bankTile = bankTile(this.conf.bank)` with
   `conf.bank === 'Falador East'` from remapped `runeCraftLocations.js`.
5. Tick 44 (274) / 45 (289): `bankTile: unknown bank 'Falador East'` then
   FAIL + exit 1.

`bankTile` (catalog, both trees):

```
const loc = BANK_LOCATIONS.find(b => b.name === bankName);
if (!loc) throw new Error(`bankTile: unknown bank '${bankName}'`);
return loc.tile;
```

Our `crates/script/src/shim/bank_locations.js` exports `BANK_LOCATIONS = []`
and keeps `nearestBank` on `snap().nearest_booth`. Empty-array `find` is
exactly this throw. Root retains FAIL; no dim.

RuneCrafter Air/Earth do **not** import `BANK_LOCATIONS`. They inline
`new Tile(3013, 3355, 0)` and `new Tile(3253, 3420, 0)`. That is why
solo Rune cells can Start. Those tiles are the same stands this mapping
must publish under the alias names Mule looks up. 05o Air deadline is
a different path (`openBooth(stand)` vs pack-empty watch) and is not
solved here.

## 2. What already exists (do not reuse as the named list)

| Fact | Owner | Name on the wire | Tile meaning |
|---|---|---|---|
| Packed `world.banks()` / `snap().banks` | `derive_banks` from content `bankbooth` placements | always `"Bank booth"` | **booth loc tile** |
| Scene `nearest_booth` / `nearestBank()` | live Use-quickly on the player's plane | loc display name, usually `"Bank booth"` | **booth loc tile** |
| `WalkNearestBank` / closest packed booth | packed Chebyshev | n/a | nearest **booth loc**, not a named alias |
| `COOK_STANDS` / `FIRE_PLOTS` | `api::content`, posted on `handle.content` before module eval | Catherby / Varrock East / … | **stand** tiles for those skills |
| `BANK_LOCATIONS` | empty shim | — | callers expect a **stand** named Falador East |

`derive_banks` (`pack.rs` 711–736) never looks up `bankboothclosed` or
`newbiebankbooth`. NPC tellers are deferred. Both selected worlds bake
**68** booth stands, every row `name="Bank booth"`, `Booth { op: 2 }`.
Keyframe posts them; unchanged deltas omit them (`force_banks` on
NavWorld identity change). That reuse must stay. Do not deep-copy the
world on read. Do not rename packed stands to Falador East — loc
identity for `open-booth` is `"Bank booth"`.

`content_json` is already posted onto `__rs2b0t_host.content` **before**
catalog modules evaluate (`load.rs` 1719–1725). Cook/fire/cow/item
tables use that channel so import-time `const` arrays are populated
without a snapshot. Named banks belong there, not on `snap().banks`.

## 3. Foreign table we refuse to copy

Frozen `BankLocations.ts` (173 lines, identical on both catalogs) is an
18-row JS world table with `requires` (quest/skill/`useMageBank`),
`access` (Shantay chest, Duel Arena openFirst), `npcAccess` (Shilo,
Gundai), `approach` for ranking, plus `nearestBanks` /
`bankUnlocked` / `approachOf`. Foreign `CookLocations.ts` builds
**one cook location per bank** from that list. Brief 17 / 04-banking
already forbade copying `BankLocations` / `resolveBankOpenRoute`.

Do not import ranking, gates, chest/NPC access, or the 18-row set.
Location **names** that catalog arguments already use may be
compatibility aliases to verified packed booths.

`MuleCrafterLogic.bankTile` only reads `.name` and `.tile`. That is the
published JS shape for this hop. Optional `requires`/`access`/`npcAccess`
stay unpublished so callers cannot pretend those policies exist.

## 4. Smallest alias set

Enabled Mule uses remapped `crates/script/src/shim/rune_craft_locations.js`
(`RUNES.bank` strings) plus `bankTile(name)`. First hop aliases are
exactly those five F2P names, each backed by packed booths on **both**
revisions (tile lists equal):

| Alias | Stand (walk / `BANK_LOCATIONS.tile`) | Packed booth cluster (loc tiles) | Stand is a packed booth? |
|---|---|---|---|
| Falador East | 3013,3355,0 | 3011–3015,3354,0 | no |
| Varrock East | 3253,3420,0 | 3252/3253/3254/3256,3419,0 | no |
| Edgeville | 3094,3493,0 | 3095,3491,0 and 3096,3493,0 | no |
| Draynor | 3093,3243,0 | 3091,3242/3243/3245,0 | no |
| Al Kharid | 3269,3167,0 | 3268,3164–3169,0 | no |

Stand provenance is host/catalog/live already using that plaza tile, not
the foreign table as authority. Falador East stand matches RuneCrafter
Air, the MuleCrafter field initializer that `bankTile()` then overwrites,
and the live mule baseline `[3012,3355,0]` on the same row. Representative
Falador East booth for later named open remains live `(3011,3354)` id
2213; this hop does **not** post that as `BANK_LOCATIONS.tile`.

Catherby `[2809,3441,0]` is already `COOK_STANDS.bank` with packed booths
at z=3442. Do not add it to `BANK_LOCATIONS` here (not a shim `RUNES.bank`
name). Falador West / Seers / Ardougne East have packed booths; no
enabled Mule/Rune/cook `BANK_LOCATIONS.find` caller. Shilo / Mage Arena
are NPC; Shantay / Duel Arena are chests. Out of this mapping.

## 5. Native module and resolve rule

New independently owned `crates/api/src/named_banks.rs` (api already
owns `WorldTile` and `content.rs`; script depends on api; nav depends on
api and can test resolve against packed stands). Do not append rows to
dirty `content.rs`.

Each alias is `{name, stand: WorldTile, booth: WorldTile}`.

```
resolve(aliases, packed_booth_tiles) -> Vec<alias>
```

Keep an alias only when **the representative booth tile is present** in
the packed booth set (exact x/z/level, booth access). Refuse (omit) when
the booth is missing, when stand equals any packed booth tile, or when
stand is not packed-walkable / not Chebyshev-adjacent to at least one
packed booth of that cluster. Omitted names do not appear in
`BANK_LOCATIONS`, so `bankTile` still throws. That is fail-closed, not a
guessed coordinate.

Bind identity: resolve against the profile's `world.banks()` when that
slice is in hand at isolate spawn. 274 and 289 packed booth tiles are
byte-equal today, so the five aliases resolve on both selected packs.
A future pack that drops Falador East booths must omit the alias.
`content_json` today does not take `NavWorld`; extending it is **gated**
(see ownership). Until that gate opens, post the five aliases only
together with unit tests that fail if `input-audit` packed tiles no
longer contain those booths. Do not ship aliases that those tests cannot
see.

Do not generate this table from ObjType game-data. Named banks are host
compatibility aliases, not server fact rows (`04-generated-game-data`
already separates curated stands from generated items).

## 6. How `BANK_LOCATIONS` callers see facts

1. Isolate spawn writes `handle.content.named_banks` **once**, before
   shim/catalog evaluate (same slot as `cook_stands`).
2. `bank_locations.js` builds

   `export const BANK_LOCATIONS = (host().content.named_banks || []).map(...)`

   with `name` and `tile: new Tile(stand.x, stand.z, stand.level)`.
   No snapshot read. No mutation of a process-global across resets.
3. A new isolate (profile bind, Stop, `reset_session` that respawns the
   isolate) re-evals the module against that profile's content. Do not
   fill the array from later `snap().banks` deltas — those omit unchanged
   packed rows and are named `"Bank booth"`.
4. `nearestBank(_hint)` stays `snap().nearest_booth`. A live nearest
   booth is not the named whole-world list.
5. `BANK_LOCATIONS.filter` / `.find` / `.map` see only resolved aliases.
   `bankUnlocked`, `nearestBanks`, `approachOf` stay honest stubs / not
   impl if they still throw; this hop does not implement them.

Cook: `cook_locations_data.js` already maps `host().content.cook_stands`
(Catherby only). Do **not** wire foreign `buildCookLocations(BANK_LOCATIONS)`.
Filling named banks must not expand cook options.

## 7. File ownership (implementation hop, not this card)

Independently owned (this capability):

- `crates/api/src/named_banks.rs` (new)
- `crates/script/src/shim/bank_locations.js` (currently empty; unique)
- `crates/script/tests/named_bank_metadata.rs` (new; do not pile onto
  `load_isolate.rs`)
- `crates/nav` tests that call `named_banks::resolve` against packed
  booth tiles (new file)

Minimal gated integration:

- `crates/api/src/lib.rs`: `pub mod named_banks;` (file is currently clean)
- `crates/script/src/shim/mod.rs` `content_json`: one `named_banks` field.
  **Hotspot:** this file is already dirty from concurrent WIP. Do not
  `--only` it while another owner has hunks. Serialize or compose a
  scoped index.

Do not touch: `bank.js`, `isolate.fbs`, `host-play`, `pack.rs` /
`derive_banks`, `content.rs`, `game_data.rs`, `snapshot.rs`,
`declared_surface.js`, catalog trees, fixtures, STATE, ledger, LIVE
harness. Do not add opcode/API/client methods.

## 8. Tests (behavior, not source snapshots)

Meaningful unit/isolate tests:

1. Isolate with named content, **no snapshot posted**:
   `BANK_LOCATIONS.find(b => b.name === 'Falador East').tile` is
   `(3013,3355,0)`. Proves import-before-snapshot.
2. `bankTile`-shaped find of `'No Such Bank'` is undefined (caller throw
   preserved). Empty content → empty array.
3. Each first-hop alias: stand ≠ every packed booth tile; representative
   booth ∈ packed 68; packed 274 tiles == packed 289 tiles.
4. Packed walkable/can_step: stand is walkable; booth loc tile is not
   used as `BANK_LOCATIONS.tile`. Use the selected nav grid / existing
   `SceneQuery::can_step` packing, not a JS flood.
5. `nearestBank` still follows posted `nearest_booth`, not the alias list
   (control: nearest booth at a non-alias tile).
6. `COOK_LOCATIONS` names remain the Rust `COOK_STANDS` set (Catherby),
   not the named-bank names.
7. Resolve omits an alias whose booth was removed from the packed slice.

Do not assert foreign length 18, quest keys, or NPC access objects.
Do not snapshot `named_banks.rs` source text.

## 9. One LIVE proof root can run

After the implementation hop is reviewed and built isolated:

`LIVE=1` catalog cell `mule_crafter`, revision 274, catalog `100adccc`,
same seed as the retained FAIL (Falador East r8, Air rune, blank partner).

Pass for **this mapping**: Start does not throw `unknown bank 'Falador East'`.
The script may still FAIL later on craft/bank/gold; that is not this
card. Same throw on 289/old mule is the same mapping; one 274 cell is
enough proof that the alias resolved. Do not lengthen
`SCRIPT_GOLD_DEADLINE`. Do not treat a mule gold PASS as implied.

## 10. Still unsupported (and why)

| Capability | Why this mapping does not provide it |
|---|---|
| `bankUnlocked` / quest / skill / `useMageBank` | Foreign policy; not packed facts |
| `nearestBanks` distance ranking / `approachOf` | Foreign router; closest packed booth already exists |
| NPC banks (Shilo, Mage Arena) | `derive_banks` is booth-only |
| Chest banks (Shantay, Duel Arena) | Not `bankbooth` |
| Cook Auto nearest-bank pairing | Would clone `buildCookLocations` |
| RoguesPurse `rankBanksByDetour` | Ranking + fare policy |
| `Bank.openNearest` approach | 04m / `bank.js` hotspot |
| Low-level `open_named_booth_at` geometry | 04k: preserve |
| Rune Air pack-empty deadline | 05o; Rune does not use `BANK_LOCATIONS` |
| Members Law `Catherby` via catalog RUNES | Not on shim F2P `RUNES`; cook stand already exists |
| All 18 foreign names | Would clone the table |

Incomplete host mapping remains **BLOCKED** until the implementation hop
lands. Honest mule FAIL stays on the ledger until Start gets past
`bankTile` and any later gold is proven separately.
