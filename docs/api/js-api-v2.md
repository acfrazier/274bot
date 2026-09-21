# JS API v2 (native host)

JS API **v2** is the native host authoring contract. It is independent of
application release 0.1.8 and of client 274/289. **v1** remains the rs2b0t
compatibility load path.

Declare both:

```ts
export const apiVersion = 2;
export function tick(api: NativeApi): void;
// or: export async function tick(api: NativeApi): Promise<void>;
```

`export const apiVersion = 2.0` is the same version as `2`. Comments and
string mentions of `apiVersion` are ignored. An explicit version that does
not match the file's bot shape is rejected (`api-version-conflict`,
`api-version-malformed`, `api-version-unsupported`,
`api-version-missing-tick`) and never silently falls back.

Unversioned `export function tick` files keep today's loader: `api.tick` is
the only member.

## NativeApi

Generated types live in `crates/script/host-js/index.d.ts` (`NativeApi`,
`NativeOp`, `NativeSnapshot`). The public handle is:

- `tick` — current game tick
- `snapshot` — read-only view of the host snapshot (do not mutate; copy if
  retaining past this tick)
- `settings.str` / `num` / `bool` — per-identity bag (`export const SETTINGS`
  is still the schema)
- `log(message)`
- `stop(reason?)`
- `paint.begin().title().row().gap().end()`
- `request(op)` — enqueue one supported op; **returns void**

Do not import rs2b0t modules or touch `__rs2b0t_host`. Unsupported
`request` ops throw `not impl: request.<op>`.

### Supported requests

`held`, `open-booth`, `open-stand`, `close`, `set-note-mode`, `withdraw`,
`withdraw-load`, `withdraw-x`, `walk`, `walk-near`, `walk-nearest-bank`,
`inspect-route`.

Completion is the next snapshots' seq/result fields, not a Promise.

- Pass `snapshot.bank_generation` into `withdraw-load` / `withdraw-x`. A
  changed generation is stale, not bank exhaustion.
- `!bank_loaded` is unavailable contents, not exhaustion.
- Generic `walk` / `walk-near` take `x`, `z`, `level` (and `radius` for
  `walk-near`) plus optional host `FindOptions`: `allow_teleports`,
  `allow_wilderness`, `allow_bank_fetch`. Omitted fields default false,
  matching `ctx.walk_with`. Explicit `true`/`false` are preserved.
  Optional `request_id` is forwarded for wait correlation.
- `walk-nearest-bank` uses host packed nav with default-false FindOptions.
  Watch `walk_outcome_seq` / `walk_outcome_failed` and `here` vs `banks`.
  No packed stand fails closed in Rust.
- `inspect-route` is a pure preview. Required `from` / `to` tiles. Omitted
  `allow_teleports` / `allow_wilderness` / `allow_bank_fetch` default
  false. Optional `avoid` rects. Use `api.inspectBegin` for an isolate
  token and `inspectSettled` / `inspectValue` to query it. `request_id: 0`
  is snapshot-only: a real host preview still publishes `route_inspect_*`
  and creates no waiter. Invented nonzero ids are isolate-stale and are
  dropped before a host job is queued. The isolate emits `inspect-ack`
  on the existing FlatBuffer interact path after applying a published
  terminal; that ack is not a public `api.request` op and cannot be
  piggybacked on `inspect-route`. Host admission counts unobserved
  terminals only (2-deep ring + 1 held). A registered token the host
  cannot reserve is posted in `route_inspect_refused_id{,_2,_3}` and
  settles `stale`; the ring/held accepted results stay until ACK.
  `route_inspect_unobserved` is last-seen fullness, not a reservation.
  Snapshot-only `0` shares the same admit budget and does not take a
  refuse-mailbox slot. Conditional bank preview never actions or
  latches a bank session; `bank_planned` requires a PRE-state stand
  proof.

User script owns bury/restock business logic. Navigation, action sequencing,
random handling and recovery stay in Rust.

## Worked example: BoneBurier v2

`crates/script/examples/bone_burier_v2.ts` is the authoritative small
author-facing example. `bone_burier_v2.js` is the checked-in normal-build
JavaScript form with the same logic and loadable v2 export declarations; it is
not a second implementation. Either file can be loaded as a NativeApi v2 card.
The example uses only the public `NativeApi` surface and demonstrates the
intended synchronous, snapshot-polled shape:

- `SETTINGS.boneName` selects the unnoted inventory and bank item while
  unrelated inventory is left untouched;
- a supported nearby bank is opened, the current `bank_generation` is passed
  to `withdraw-load`, the bank is closed, and inventory change is observed
  before the next bury request;
- a loaded/current bank row with no matching unnoted supply is a clean,
  confirmed exhaustion stop, while missing, stale, or unloaded contents are
  unavailable (never exhaustion);
- walk, open, close, withdraw, and bury operations have bounded observation
  waits with useful refusal/stall reasons; paint reports phase, burials, and
  observed Prayer XP.

The corresponding `bone_burier_v2` integration tests are synthetic isolate
tests (not LIVE tests). They load both the TypeScript source and bundled
JavaScript, and verify per-isolate settings and bounded failure behavior.

## Prayer helpers

Eight named methods. Queries (`prayerPoints` / `Max` / `Full` / `Known` /
`Available` / `Active`) are sync `HelperResult`. `prayerSet` and
`prayerClear` are the named async private-lifecycle exception
(`Promise<HelperResult<…>>`). Callers never see the private Step.

**Before:** a second overlapping `prayerSet` / `prayerClear` overwrote the
single private pump. Rust abort of the old token did not settle the first
Promise. A sync tick that fired Set/Clear without returning that Promise
did not advance the admitted job.

**After:** if an operation is already admitted, the second Set/Clear
returns `{ok:false, error:'busy'}` before Rust begin or click. The original
keeps ownership and must settle. Sequential `await` inside a returned
async tick remains the preferred example. Fire-and-forget still progresses
on later eligible NativeTicks (not paused, held, or unready). Matching
snapshot completion can enqueue the next clear click only through the
existing Rust machine. Additional public error: `busy`.

See `crates/script/examples/prayer_v2.ts`.

## Supply helpers

Five sync `HelperResult` methods over selected-revision facts (food count/heal,
combat keep list, runes per cast, escape teleport requirements). v1 compat
exports `foodCount`, `foodHealAmount`, `combatKeepNames`, and `runesPerCast`
keep their existing JSON helper paths where applicable; `foodCount` and
`foodHealAmount` marshal through typed native hooks with Rust policy.

| Method | OK | Errors |
| --- | --- | --- |
| `foodCount({ items, foodName })` | slot count | `invalid-args`, `missing-selected-data` |

`foodCount` takes a real `ItemRow[]`. `api.snapshot.inv` is the host array view
(`Array.isArray` is true); pass it directly. Arbitrary `{ length }` objects are
`invalid-args`. `Array.from(api.snapshot.inv)` is optional, not required.
| `foodHealAmount({ foodName })` | fixed heal | `unknown-food`, `missing-selected-data` |
| `combatKeepNames({ food, … })` | string[] | `invalid-args`, `missing-selected-data` |
| `runesPerCast({ spellName, wielded })` | costs or `null` | `invalid-args`, `missing-selected-data` |
| `escapeRunesFor({ id })` | `{ runes, level, label }` | `unknown-id`, `missing-selected-data` |

`escapeRunesFor` resolves `id` with exact `magic_spell_teleport_{id}` rows (no
cast `available()` filter). Public v1 `escapeRunesFor` export remains W5 hunt
integration; the Rust fact module is shared when that lands.

Example: `crates/script/examples/supply_helpers_v2.ts`.

## Loadout and potion helpers

Eight sync `HelperResult` methods. They recommend food, worn names, carry
rows, a weapon, a dart/bow shape, and a melee flask pair. They do not drink,
equip, withdraw, or invent `snapshot.loadout`. Callers supply a loadout
literal and read `api.snapshot.inv` / `api.snapshot.stats`.

v1 JSON helpers (`__rs2b0t_food_of`, `__rs2b0t_gear_of`,
`__rs2b0t_supplies_of`, `__rs2b0t_weapon_of`, `__rs2b0t_boost_potions_step`)
and the current ranged.js shim stay unchanged, including callback/identity
quirks. New typed calls do not go through that JSON dispatch.

| Method | OK | Errors |
| --- | --- | --- |
| `foodOf({ loadout, fallback })` | first `fixed_food_heals` carry name, else fallback | `invalid-args`, `missing-selected-data` |
| `gearOf({ loadout })` | hat-first `WORN_SLOTS` then unassigned | `invalid-args` |
| `suppliesOf({ loadout })` | `{item, qty}[]`; omitted qty defaults to 1 | `invalid-args` |
| `weaponOf({ loadout, fallback? })` | trimmed righthand or fallback/`null` | `invalid-args` |
| `rangeLoadoutOf({ weapon, ammo })` | dart shape or raw bow-shaped | `invalid-args`, `missing-selected-data` |
| `boostFaded({ base, effective, floor? })` | floor-band boolean; omitted floor `0.1` | `invalid-args` |
| `plannedPotions({ carry })` | Super attack then Super strength value copies | `invalid-args` |
| `potionToSip({ plans, held, levels })` | first due plan or `null` | `invalid-args`, `missing-observation` |

`foodOf` does not restore ambiguous eat-table foods. Missing levels are not
treated as zero: a held plan without a matching level row is
`missing-observation` and is not skipped for a later due plan. Explicit
`base: 0` remains a known observation. Carry `qty` / plan `want` must be a
positive integer `<= 4294967295` when supplied; planned carry `qty` is
required.

Example: `crates/script/examples/loadout_potion_v2.ts`.

## Line of sight

`snapshot.collision` is the posted one-plane raw `i32` grid. `flags` is a
readonly view (`length`, `at(index)`). Index is `lx * height + lz`. `0` is
clear, not absent. `at` returns `undefined` for a non-finite or non-integer
index, a negative index, or an index `>= length`. Do not treat it as a
TypedArray.

`api.lineOfSight({ from, to, size? })` is the typed v2 helper. Size omitted
is `1`; a present size must be an integer in `1..=104` even when the family
is absent (`width === 0`). Different planes return `{ok:true, value:false}`
without a family. Same-plane absence is `{ok:false, error:'missing-observation'}`.
A destination footprint that extends past the scene is not rejected when both
origins and the sampled ray stay valid.

v1 `Reachability.lineOfSight(from, to, size?)` marshals the same Rust ray and
returns `false` for invalid tiles, missing family, or a non-integer / `0` /
`>104` size.

Example: `crates/script/examples/line_of_sight_v2.ts`. The File witness
selects two observed cardinal adjacent pairs from raw `flags.at` (open:
start not `WALK_SCENERY` and entering projectile V-bit clear; blocked:
entering V-direction bit set). `VIS_SCENERY` at the destination is not a
blocked pair. It then calls real `api.lineOfSight` and imported
`Reachability.lineOfSight` on those pairs. Pair selection is qualification
fixture logic, not API policy.

## Actor observation

`snapshot.npcs` is the packed NPC_INFO array. Each row's `x,z` is rendered
SW. `nx,nz` is path-head network SW. `size` is tiles along one side.
`size < 1` means observation absent (old buffer or non-NPC family); do not
invent `1`. World `0,0` with `size >= 1` is a real tile, not absence.
Facts are packet-time at the last NPC_INFO rebuild. Copy the array if
retaining past this tick.

`snapshot.self_target_kind` is `0` none, `1` npc, `2` player.
`snapshot.self_target_index` is `-1` when kind is none; `0` is a legal NPC
index. Consume dest SW + dest size with existing `api.lineOfSight`. There
is no v2 `Npc` class.

Example: `crates/script/examples/actor_observation_v2.ts`. The File waits
for a posted `size>=1` NPC, calls existing `api.lineOfSight` plus imported
`Npc.size` / `Npc.networkOrigin()` / `reader.selfTarget()` /
`Reachability.lineOfSight`, then named-stops. No size>=1 row is not
complete.

## Sync and async tick

A v2 `tick` may be async. The isolate will not re-enter `tick` while that
Promise is pending. Snapshot posts still merge. Prefer a **sync** tick that
polls `api.snapshot` across game ticks.

Pause / Stop / disconnect / reload drop queued actions of the old
generation. Settings updates replace the bag for later ticks. Per-profile
settings stay isolated by card identity.
