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

`held`, `open-booth`, `open-stand`, `close`, `set-note-mode`, `deposit`,
`withdraw`, `withdraw-load`, `withdraw-x`, `walk`, `walk-near`,
`walk-nearest-bank`, `inspect-route`, `puzzle-move`.

Completion is the next snapshots' seq/result fields, not a Promise.

- `deposit` takes one required `name`: the **bank-side display name**, never
  an inventory row rewritten first. `request` does not pre-resolve it; the
  host matches the first `snapshot.bank_side` row whose resolved obj name
  equals `name` ignoring ASCII case and sends that row's Deposit-All menu
  op. A miss, a closed/unloaded bank, an already-armed bank operation, and
  a changed `bank_generation` never throw out of `request`: they fail
  closed, and completion is `bank_op_result_seq` / `bank_op_result`
  (`false` for any of them). `true` requires the matched row's bank-side
  count to drop within the bounded deposit window.

- Pass `snapshot.bank_generation` into `withdraw-load` / `withdraw-x`. A
  changed generation is stale, not bank exhaustion.
- `puzzle-move` clicks one piece of the posted board. It takes the posted
  row's own `id`, `slot` and `component` plus
  `snapshot.puzzle_board_generation`. The host re-resolves that exact row on
  `snapshot.puzzle_board.items` and sends the held family's OPHELD op
  (`Move` where the obj table has it, else op 5) on the board component —
  never the component-op INV_BUTTON the bank and make panels use. A closed
  board, an observed size other than 25, a stale generation, a row that has
  moved or gone, and an obj table with neither `Move` nor a fifth op all
  fail closed: nothing is sent and nothing is closed. A sent packet is not
  board progress — there is no move-result field and nothing waits on the
  board, so re-read `snapshot.puzzle_board` for the pieces' new slots.
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

## Gather query helpers

Two sync `HelperResult` methods over the selected pin's gather-methods family.
They are not Promises, not `request()` ops, and they do not push `h.interact`.
`bestAxe` and `bestPickaxe` stay `notImpl`.

| Method | OK | Errors |
| --- | --- | --- |
| `gatherMethods(input?)` | `{ rows, coverage }` | `invalid-args`, `missing-selected-data`, `family-unavailable:gather_methods`, `unknown-skill` |
| `gatherResource({ name })` | `{ rows }` | `invalid-args`, `missing-selected-data`, `family-unavailable:gather_methods`, `unknown-resource` |

`gatherMethods()` and `gatherMethods({})` omit the skill. Accepted skills, after
trim and ASCII case-fold, are only `woodcutting`, `mining`, and `fishing`. A
skill filter keeps that bucket's order and does not filter coverage. Coverage
is the loaded pin's full array, beside `rows`. `gatherResource` matches
`resource_key` only (trim, ASCII case-insensitive). One match is `rows` of
length 1. Zero matches is `unknown-resource`, not `{ rows: [] }`. Loc ids stay
`{ alias, id }`. Fishing rows have no loc ids.

Example: `crates/script/examples/gather_methods_v2.ts`.

## Quest query helpers

Two sync `HelperResult` methods over the selected pin's quest-identity family.
They are not Promises, not `request()` ops, and they do not read varp 101.
Both methods read `quest_identity` only. There is no second writer field.

| Method | OK | Errors |
| --- | --- | --- |
| `questIdentity({ name } \| { id })` | landed row | `invalid-args`, `missing-selected-data`, `family-unavailable:quest_identity`, `unknown-quest` |
| `questPrereqs({ id })` | requirements object | `invalid-args`, `missing-selected-data`, `family-unavailable:quest_prereqs`, `unknown-quest` |

`questIdentity` takes exactly one of `name` or `id`, and it must be a string.
`questIdentity()` and `questIdentity({})` are `invalid-args`, not a six-row dump.
Neither, both, a non-object, a non-string, or an array is `invalid-args`. A
number is `invalid-args`. A blank or whitespace-only string is `unknown-quest`
after the family check. `id` matches the seed id only (trim, ASCII
case-insensitive). `name` matches `display` only. One match is the row object,
not `{ rows }` of length 1. Fields stay `id`, `component`, `display`, `varp`,
`varp_id`, `complete`, `quest_points`, `unknown_sides`, and `requirements`.
`complete` is the number. `quest_points` is the static constant.

`questPrereqs` requires a string `id`. A `name` field is not a key. An extra
`name` is ignored. The value is the requirements object only. All six stay
`partial`. Empty `mustHave` is still `ok: true` with `qualification: "partial"`
and `unknown_as_satisfied: false`. Items stay script aliases.

Example: `crates/script/examples/quest_facts_v2.ts`.

## Clue helpers

Four sync `HelperResult` fact reads and one owned-session pump. `row` and
`heldStep` read the selected pin's landed trail membership family; `packPlan`
is pure slot arithmetic over the caller's own numbers and reads nothing at all;
`hardKit` is a pure hard-clue kit status over the caller's own facts and reads
nothing at all. `begin` / `next` are the isolate machine: one token per
isolate, the landed held-step identify over the posted pack page, and its own
search and casket-open steps onto the interact drain. None is a Promise and
none is a `request()` op; the four fact reads push no `h.interact`. `row`,
`heldStep` and the machine read `trails()` only: not `items()`, and not the
challenge answers. `api.clue` holds `row`, `heldStep`, `packPlan`, `hardKit`,
`begin`, and `next`, in that order, and nothing else.

| Method | OK | Errors |
| --- | --- | --- |
| `clue.row({ id } \| { alias })` | landed membership row | `invalid-args`, `missing-selected-data`, `family-unavailable:trails`, `unknown-id` |
| `clue.heldStep()` | the first held membership row | `missing-selected-data`, `family-unavailable:trails`, `none-held` |
| `clue.packPlan(input)` | the published pack targets | `invalid-args`, `no-room` |
| `clue.hardKit(input)` | `{ status: 'ready' }` | `invalid-args`, `attack`, `lost-city`, `dds`, `superantipoison`, `sharks` |
| `clue.begin(input?)` | `{ token }` | `missing-selected-data`, `family-unavailable:trails`, `none-held` |
| `clue.next({ token, resume? })` | one continue step | `invalid-args`, `missing-selected-data`, `family-unavailable:trails`, `none-held`, `stale`, `aborted` |

`clue.row` takes exactly one of `id` or `alias`. Neither, both, a non-object, an
array, or a non-string `alias` is `invalid-args`, and `api.clue.row()` and
`api.clue.row({})` are `invalid-args`, not a row dump. `id` is the packed
integer and a real `i32`: `"3554"`, `3554.5`, a bigint, `new Number(3554)`, and
`2147483648` are `invalid-args`, and nothing is clamped or converted. Negative
zero is not an `i32` to that check either, so `{ id: -0 }` is `invalid-args`
rather than a `0` miss. A present `{ id: 0 }` is a pin, not a missing one, so a
miss is `unknown-id`. `alias` matches the landed alias only (trim, ASCII
case-insensitive); the wrapper does not trim or fold.

The order is `invalid-args`, then `missing-selected-data`, then
`family-unavailable:trails`, then `unknown-id`. Family absence is the method
token, never `{}`, `{ rows: [] }`, or an empty success, and a blank alias does
not turn that absence into `unknown-id`; a blank alias with no selected data is
`missing-selected-data`, and a blank alias with a landed family is `unknown-id`.
One match is the landed row object, not `{ rows }` of length 1. Fields are
`alias`, `id`, `role`, and `params`; `access` is present only when the landed row
carries it, and the one bounded inclusion (packed 3554) is
`access: "constrained"`. A row that omits `access` omits the key: it is never
`null` and never rewritten to `"open"`.

`params` is the landed list, in file order, raw strings: `^true` and `yes` stay
strings, an empty list stays present, and `"Speak to Hazelmere."` stays inside
`params`. Nothing is parsed, coerced, expanded, or joined: no `trail_coord`
coordinate, no item name, no `npc`, no `supported`, and no answer. Membership is
not support, so `3533`, `trail_clue_hard_sextant017_casket`,
`trail_clue_medium_map002`, `trail_clue_hard_sextant026`, and the six challenge
rows are `unknown-id`, and a challenge answer is never suffix-joined onto its
parent. Input keys other than the one pin are ignored:
`api.clue.row({ id: 3554, supported: true, access: "open" })` returns the landed
3554 row with `access: "constrained"` and no `supported` key.

Not a Promise and not a `request()` op: `api.request({ op: 'clue.row' })` stays
`not impl`. `challengeAnswer`, `deposit`, `retry`, and `noteDeath` do not exist
on `api.clue`.

`clue.heldStep()` takes no argument. The page is the already-posted
`host().snapshot.inv` `(id, count)` sequence and nothing else: an argument is
ignored, nothing rebuilds the tab, and a missing or empty page is an empty held
list rather than a snapshot error. A pair is held only when its count is
positive, and the held `id` is matched against the membership rows alone — no
name, noted, cert, or alias match, and no second inventory page. A held id that
is not a membership row is skipped, not `unknown-id`, so a pack of unrelated
items (a challenge row included) is `none-held`, and the challenge answers stay
unread.

The step is the first held casket, else the first held clue: a clue earlier in
the pack does not beat a later casket, and within one role the posted order of
the page decides, not the membership table. The value is the same landed row
object `clue.row` returns, including `access: "constrained"` on the packed 3554
clue; it is not `{ rows }`, not a coordinate, not an npc, and not an answer. The
order is `missing-selected-data`, then `family-unavailable:trails`, then
`none-held`, then the row. Nothing held is a named error, never `ok` with a
`null` value. It is sync `HelperResult`, not a Promise, and
`api.request({ op: 'clue.heldStep' })` stays `not impl`.

`clue.packPlan(input)` is pure slot arithmetic over the caller's own numbers. It
reads no snapshot, no `snapshot.inv` page, no equipment, no bank, and no trail
row, so a missing page or family is not one of its errors, and an extra input
key is neither an inventory override nor a snapshot substitute. It is sync
`HelperResult`, not a Promise, and `api.request({ op: 'packPlan' })` stays
`not impl`.

```text
capped = min(hostWant, 10)
room   = heldFood + max(0, freeSlots - reserveSlots)
food   = max(0, min(capped, room))
```

| Field | If absent | If present |
| --- | --- | --- |
| `hostWant` | `0` | non-negative `i32` |
| `heldFood` | `0` | non-negative `i32` |
| `freeSlots` | `0` | non-negative `i32` |
| `reserveSlots` | `0` | non-negative `i32` |
| `perCast` | no `runeTarget` | non-negative `i32`, including `0` |
| `weaponName` | no `weaponNeeded` | string, including `""` |
| `weaponInBackpack` | `false` | boolean |
| `weaponEquipped` | `false` | boolean |
| `casketAlias` | no `rewardSlots` | string, including `""` |

Presence is `hasOwnProperty`, not truthiness: a present `null`, a wrong type, a
fraction, a negative, `-0`, a string number, a bigint, or a boxed `Number` is
`invalid-args` rather than the omitted default, and nothing is clamped, trimmed,
case-folded, or coerced. `perCast: 0` is a present field with `runeTarget: 0`;
`weaponName: ""` is a present field with `weaponNeeded: false`, and because
nothing is trimmed `" "` is a name; `casketAlias: ""` is a present field with
`rewardSlots: 4`. `packPlan({})` is `ok` with
`{ coordToolSlots: 3, teleportCasts: 20, food: 0 }`.

The ok value carries only the fields the caller asked for, plus the two named
constants and `food`:

| Key | Value |
| --- | --- |
| `coordToolSlots` | `3`, always |
| `teleportCasts` | `20`, always |
| `food` | `0..=10`, always |
| `runeTarget` | `perCast * 20`, only when `perCast` is present |
| `weaponNeeded` | `weaponName !== "" && !weaponInBackpack && !weaponEquipped`, only when `weaponName` is present |
| `rewardSlots` | `6` when `casketAlias` contains `_hard_`, else `5` for `_medium_`, else `4`; only when `casketAlias` is present |

`_HARD_` is not `_hard_`, and the alias is never looked up: it is not an id, a
miss is not `unknown-id`, and an unrecognised string (including `""`) still
reserves the easy-tier `4`. Hard is checked before medium. Runes stack, so
`food` carries no rune-slot term, and the method does not invent `reserveSlots`:
the caller passes the slots the coord trio still needs. `trailFoodCap` is not a
published field.

`no-room` is the whole result, with no value, when `hostWant > 0` and the food
arithmetic is `0`: the caller asked for food, `heldFood` is `0`, and
`freeSlots <= reserveSlots`. It is never `ok` food `0`, and `hostWant` `0` or
omitted with food `0` stays `ok`. A zero rune target is not `no-room`,
`weaponNeeded: false` is not an error, and `rewardSlots` is never `no-room`. A
caller who wants runes without food omits `hostWant` and calls again: the method
is pure and stateless.

The arithmetic widens past `i32`: `perCast: 2147483647` publishes
`runeTarget: 42949672940`, not a wrapped `-20` and not a saturated
`2147483647`, and `heldFood` `2147483647` with `freeSlots` `2147483647` is a
huge room that still caps `food` at `10` rather than wrapping into a false
`no-room`.

`clue.hardKit(input)` is a pure status over the caller's own kit facts. It reads
no snapshot, no `snapshot.inv` page, no quest tab, no equipment, and no bank, so
`snapshot-unavailable`, `quest-tab-unbound`, `not-on-tab`, `none-held`,
`unknown-id`, and `no-room` are not its errors, and an extra input key is neither
an inventory override nor a snapshot substitute. `lostCity` is already resolved
by the caller: a caller who has not checked the quest passes `false` and receives
`lost-city`. It is sync `HelperResult`, not a Promise, and
`api.request({ op: 'hardKit' })` stays `not impl`.

The five kit failures are the whole result, with no value, and neither sum is
ever published. First failure wins:

| Check | Result |
| --- | --- |
| `attack < 60` | `attack` |
| `lostCity` false | `lost-city` |
| no item with id `1231` or `1215` and `count > 0` | `dds` |
| summed superantipoison doses is `0` | `superantipoison` |
| summed id `385` count `< 15` | `sharks` |
| otherwise | `{ status: 'ready' }` |

| Field | Rule |
| --- | --- |
| `attack` | required non-negative `i32` |
| `lostCity` | required boolean |
| `items` | required array; each element is an object with required `id` (a real `i32`) and `count` (a non-negative `i32`) |

`ready` is the only ok value, and it carries nothing else. The constants are the
frozen ones: attack minimum `60`, DDS ids `1231` and `1215`, superantipoison
`2448` × 4, `181` × 3, `183` × 2, `185` × 1, shark id `385`, and fifteen sharks.
`attack: 60` passes and `59` is `attack` even when Lost City is false and the kit
is otherwise ready; a present `attack: 0` is a number, so it is `attack` rather
than `invalid-args`. One dose of any size is enough, ordinary antipoison `2446`
contributes `0` doses, and dragon longsword `1305` is not a DDS. Fourteen sharks
fail and fifteen pass however the stacks are split, and two stacks that sum to
`15` pass. An item id with `count: 0` does not satisfy `count > 0`.

Nothing is defaulted. An omitted `attack` is `invalid-args`, never the status
`attack`, and an omitted `lostCity` is `invalid-args`, never `lost-city`.
Presence is `hasOwnProperty` and `1`/`0` are not booleans, so a present `null`, a
wrong type, a fraction, a negative, `-0`, a string number, a bigint, or a boxed
`Number` is `invalid-args` rather than the omitted default, and nothing is
clamped or coerced. An empty `items` array is a present array: after attack and
Lost City pass, it is `dds`. An item that is not an object, or whose `id` or
`count` is missing, `null`, or the wrong type, is `invalid-args`. Extra keys on
an item are ignored, so `{ id: 1231, count: 1, slot: 3, worn: true }` is a held
dagger, and an id that matches nothing (a negative one included) is a miss rather
than `invalid-args`. `includeBank` is not an input: a present key is ignored and
is not a bank read.

The dose and shark sums widen past `i32`: `count: 1073741824` of `2448` is
`4294967296` doses, not a wrapped `0`, and two `2147483647` shark stacks sum to
`4294967294`, not a wrapped `-2` under the minimum. `hardTrailFoodTarget` and
`hardKitSnapshot` are not published: no `trailFoodCap`, no snapshot gatherer, and
no bank prep.

Example: `crates/script/examples/clue_facts_v2.ts` (row),
`crates/script/examples/clue_held_step_v2.ts` (held step),
`crates/script/examples/clue_pack_v2.ts` (pack targets), and
`crates/script/examples/clue_hard_kit_v2.ts` (hard-kit status).

### `clue.begin` / `clue.next`

One machine per isolate, over the landed held-step identify. It is sync, it is
not a Promise, and it is not a `request()` op: it is the search, casket-open,
unguarded-dig, guarded-dig encounter and trail-end collect slice of a later
clue trail, not the whole dispatcher. It emits search, the held Open, the
unguarded Dig, the guarded walk/Dig/Attack/redig and the collect that follows
the last casket — no talk, puzzle, deposit, or retry — and it never emits the
exact `'clue solved'` string, never restores gear, and never returns a `done`
status. `ownsEquipment` stays false.

`clue.begin(input?)` takes the optional input and ignores every key: nothing
but the token and the wrapper's generation is captured, so `enabled`, the pack
page, food and equipment stay script-owned and are re-read later. It returns
`{ ok: true, value: { token } }` or a refusal. The identify is the landed
order: `missing-selected-data`, then `family-unavailable:trails`, then
`none-held`, then the row. A family absence is not `none-held`, and a refusal
leaves **no** live token — a later pickup needs a new `begin`. A second `begin`
aborts the previous token and emits nothing for it.

`clue.next({ token, resume? })` takes an object argument, not
`(token, resume)`. `resume` is the callback return and is the only answer slot:
`reply` is not read, and a non-boolean `resume` is `invalid-args` (as is a
missing or non-integer `token`, or a non-object argument). The step is
`{ ok: true, status: 'continue', token, kind, … }` or `{ ok: false, error }`;
a dead token is the error object, never `undefined` and never a continue kind.

| `kind` | Meaning |
| --- | --- |
| `wait` | nothing this tick: frozen by pause/hold, or the session idled after `resume: false`, or the identified step was already reported, or — while collecting — the pages are still empty inside the reward window, or the posted pack page carried no `inv_size`; a dig row that has not arrived, or has no `Spade` on the posted pack page, waits the same way, and so does the guarded encounter with no wizard of the row family posted on the npc page — no spawn posted is no click and no Attack |
| `callback.enabled` | re-read the script's `enabled()` and answer with `resume` on the next `next` |
| `callback.log` | perform `log(message)` |
| `callback.setStatus` | perform `setStatus(message)` — a progress string, never `'clue solved'` |
| `walk` | a search row, an unguarded dig row or a guarded row that has not arrived: walk to `{ x, z, level }`, the decoded `trail_coord` |
| `loc` | a search row that has arrived: interact with the picked loc, `{ x, z, level, action, id }` |
| `npc` | a guarded row that has arrived and Dig its spawn: interact with the posted npc, `{ name, action: 'Attack', index }` — the posted scene index it was observed with |
| `if-button` | a guarded fight whose posted overlay does not read up: click the selected Protect from Magic component, `{ component_id }` |
| `held` | a casket row that has been reported: interact with the held item named `name` with `action`, `{ name, action }`; an unguarded or guarded dig row's `Dig` is the same kind with `name: 'Spade'`, and the collect's pack-full Drop is the same kind with `action: 'Drop'` |
| `close-modal` | collecting: close the main modal the page posted open. The step carries nothing else — no interface id, no text |
| `obj` | collecting: interact with the casket overflow on the posted tile, `{ x, z, level, name, action: 'Take' }` |
| `yield` | posted `hold \|\| ours`; the token stays live and this is not trail completion |

The precedence on a live token is frozen clock → `wait`, else posted
`hold || ours` → `yield`, else the identify, else `callback.*`. A frozen call
burns nothing: the pending `enabled` question is still open after the thaw.
`resume: false` idles the session with its token live — it is not `abandon`,
not `done`, and not a completion — and the next gate re-reads instead of
replaying that answer. `none-held` on a live session aborts it: the held
membership went away, so the old token is dead. The one exception is the
trail-end collect below: a `Steady` step whose casket `Open` was already
dispatched survives `none-held` as `Collecting`, and the same `none-held`
abort is how that collect finishes. Reset, stop and a generation
bump abort silently; the machine emits no `h.interact` entry and no request op
for them, and the first thing the caller hears about it is `stale` or
`aborted`. A held step that is neither a casket, a search row, an unguarded dig
row nor a guarded dig row — the packed 3554 `access: "constrained"` clue
included — is identified and then idled: no action and no walk.

A held row is a **search row** only when the selected family carries
`trail_loc=^true` **and** a decodable `trail_coord` on that same row: five
`_`-separated integers `level_mapX_mapZ_localX_localZ`, level in `0..=3`,
locals in `0..64`, no extra part, and `x = mapX * 64 + localX` (`z` likewise).
Coord-only rows (the easy maps), the desc-only riddles, off-contract tokens and
the packed 3554 clue are not search rows: they stay identified then idle, and
an off-contract token is never rounded into an invented coordinate.
`trail_coord` is never published on `clue.row`.

Once the search row has been reported — after its `callback.log` and
`callback.setStatus` — `walk` and `loc` replace the idle `wait`. The wrapper
marshals the posted `here` tile and the posted loc page at `next` call time,
so the machine caches no world copy and `api.snapshot.locs` stays hidden.
`walk` repeats until `here` is on the decoded tile's level and within Chebyshev
1 of it; then the picker takes posted loc rows on that level, within Chebyshev
1 of the decoded tile (not of `here`), whose actions match `Search` then `Open`
case-insensitively — nearest first, then rank, then posted order. The `loc`
step carries the picked row's own tile and its posted scene id, always, so the
host refuses a stale id rather than falling back to a co-located row. Neither
kind is a `V2_OPS` verb: `next` enqueues them onto the interact drain the way
the quest journal enqueues its modal clicks, `api.request({ op: 'loc' })` stays
`not impl`, and the step is still returned as `status: 'continue'`.

Arrived with nothing to search this tick — an unloaded scene, an empty page, or
a loc id the host refused — is `wait`: the token stays live and the pick is
re-read next call. There is no `walkLeg` here: no walk timeout, teleport,
wilderness, bank fetch, detour or `no-searchable-loc` that kills the token, and
no `abandon`.

A held row whose landed `role` is `casket` is the held casket item, and the
landed identify is **casket-first**: with a casket and the clue it belongs to
both held, the casket is the step — the sextant028 casket is the Open and never
3554 play. Once its own report is posted — after its `callback.log` and
`callback.setStatus` — the `held` step replaces the idle `wait`. Its `name` is
the selected item display joined by that row's own id (never the alias, never
an item id) and its `action` is `Open`, so the host resolves the first held
inventory row with that name; no tile and no row id ride along. `held` repeats
while that same casket id stays held, and a different held row re-arms the
gate. The frozen clock and the posted `hold || ours` interrupt still win: a
frozen call and a `yield` emit no `held`. When the casket leaves and nothing
else in the trail family is held, the landed `none-held` still aborts the
token — that is not completion, not `done`, and not `abandon`.

Unlike `loc`, `held` is already a supported author `request` op (`V2_OPS`), and
the machine's own step is enqueued onto the interact drain directly rather than
through `request()`; the step is still returned as `status: 'continue'`.

A held row is an **unguarded dig row** when the selected family carries a
decodable `trail_coord` on a row with **no** `trail_loc`, **no**
`trail_guardian` and an `access` that is not `"constrained"`, which is the
twenty medium sextant rows (`2801`…`2825` odd and `3582`…`3594` even) beside
the twenty coord-bearing map, vague and riddle rows that never pinned a
sextant — the easy maps (`2713`, `2716`, `2719`, `3516`, `3518`), the medium
maps (`2827`, `3596`, `3599`, `3602`), the hard maps (`3520`, `3522`), the
vague `3510` and the hard riddle-with-coord rows (`2774`, `2776`, `2780`,
`2783`, `2786`, `2788`, `2790`, `3580`) — forty rows on both pins. Neither
`trail_sextant` nor `trail_casket` is that membership:
`trail_sextant` is not read by this classify at all, so the
Sextant/Watch/Chart trio is never required, never waited for and never
acquired, exactly as the search walk never bank-fetched its key. Once such a
row has been reported — after its `callback.log` and `callback.setStatus` — it
walks to its decoded tile exactly like a search row (same posted `here`, same
level, same Chebyshev 1, same repetition until it holds), and then, arrived,
dispatches the generic `held` step with `name: 'Spade'` and `action: 'Dig'`:
the selected item display the host resolves by first name match, carrying no
row id and no tile. `Dig` repeats while that same clue stays held — the host
refuses an item it no longer holds — a casket the dig produced is the landed
`Open` that follows, and a `none-held` right after a `Dig` still aborts: only
the casket `Open` reaches the collect below. Arrived with no `Spade` on the
posted pack page — or with no posted `here` at all, or still on the way — is
`wait`: the token stays live and the next call re-reads the page. Nothing is
acquired and nothing is invented: no `ensureSpade`, no bank fetch, no ground
scan, no public `no-spade` error and no `abandon`.

A held row is a **guarded dig row** when the selected family carries a
decodable `trail_coord` on a row with **no** `trail_loc`, `trail_sextant=yes`,
a `trail_guardian` and an `access` that is not `"constrained"`, which is the
thirty hard sextant rows (`2723` among them). The `trail_guardian` value is a
family alias and not an npc name: `trail_hard` is the cap-documented Zamorak
Wizard and `trail_hard2` the Saradomin Wizard, and that mapping is only ever a
filter over the posted npc page — it is never written onto the row, never
published, and never used to invent a scene entity.

The first Dig is the spawn, so this row's walk-then-Dig is only the start. Once
reported, the row walks to its decoded tile exactly like the unguarded sibling
and Digs with the Spade from that same arrived page. From that Dig on the
machine holds the encounter in its own session state — no second scheduler,
never a `Phase::Fighting` — and each following call reads the pages the wrapper
marshals at call time, in that order: the posted npc page is observed first,
the Protect from Magic click is only ever raised behind a posted wizard of the
row family, and only a posted-on overlay leaves the Attack:

| Kind | When |
| --- | --- |
| `if-button` | a wizard of the row family is posted and this call's overlay varp (the selected Protect from Magic row's own varp) does not read `1`, including when the page did not post it: the machine clicks the row's own selected component and waits. An overlay already up skips the click, the click is never raised for a spawn that was not posted, and the fight never Attacks under an overlay that is off or unobserved. Nothing here nests `api.prayerSet`, waits a toggle timeout, or clears the prayer |
| `npc` | the overlay reads up, a wizard of the row family is posted and this token owns no wizard yet: the posted row must be on the posted `here` level, carry `Attack`, and sit inside the frozen radius 12 — by its own posted `distance` when the page posted one, and by its own posted `x`/`z` tile measured from the posted `here` otherwise. The row targeting the player wins, else the nearest match, then posted order. The verb is `{ name, action: 'Attack', index }` with the posted name and the posted scene index, so the host refuses a stale index instead of taking a co-located row |
| `wait` | no posted npc page, nothing of the row family posted, no name-and-action match on the `here` level inside the radius, a wizard that is still posted and alive, or an owned index that left the page only after the frozen grace — the nearest anything is never Attacked, the Attack is not re-issued, and there is no `guardian-lost` |
| `held` | the kill was observed: the walk back to the decoded tile and its Dig, which repeats while that same clue stays held, exactly like the unguarded Dig. A casket it produces is the landed casket-first Open |

The kill is the wizard this token Attacked: that owned index leaving the posted
npc page inside a 6000ms grace the freeze never spends — the thaw reclaims the
frozen interval into the owned last-seen the way the landed hunt fight shifts
its own stamps, so a pause longer than the remaining grace still ends in the
kill — or that index posted at zero health beside a posted maximum while the
page still shows this token's fight on it. An index this token never Attacked
is never a kill — a disappearance without a prior Attack waits rather than
Digging again. While the fight is on, a posted effective `hitpoints` at or
below zero is a `wait` and never a public `dead`, and a page that posted no
hitpoints is not a zero. The encounter survives `yield` and idle waits on its
live token the way the casket `Open` does; a different held row, an abort and a
reset drop it, so the row's next `Steady` walks and Digs its spawn again. All
of those pages are this call's own marshalling of `host().snapshot` — the
posted npc page, the local-player slot and its posted target pair, the posted
overlay varp and the posted stat rows — so the machine caches no world copy,
`api.snapshot.npcs` is never scanned, and `api.snapshot.self_slot` and
`api.snapshot.varps` stay hidden. `npc` and `if-button` are not `V2_OPS`
verbs — `next` enqueues them onto the interact drain directly, so
`api.request({ op: 'npc' })` and
`api.request({ op: 'if-button' })` stay `not impl` — and the guarded row emits
no completion, no `supplies-needed`, no `dead` and no `guardian-lost`.

The rows that are neither caskets, search rows, unguarded dig rows nor guarded
dig rows stay identified then idle: the packed 3554 `access: "constrained"`
clue, the desc-only riddles that carry no decodable `trail_coord` and the
empty-params `2722`. Every coord-bearing row without the `trail_loc` pin is the
dig classify's instead, so it is the `trail_guardian` / `access` half of the
classify — and the coord itself — that keeps these out: no frozen `type` table
is copied, and a row that carries the `trail_loc` pin at all belongs to the
search membership instead.

### Trail-end collect

The held `Open` is also the trail-end seam. The first call whose identify is
`none-held` from that same `Steady`-on-casket step is `Collecting`: the token
stays live and the loot is dispatched one verb per call. The `hard` signal is
the casket alias' own `_hard_`, captured at the `Open` while the row is still
in hand — after `none-held` the row is gone. The one 2000ms reward window is
armed on entry and is freeze-paused; empty pages wait it out and then finish.

| Kind | When |
| --- | --- |
| `close-modal` | a posted `main_modal_id` other than `-1`. An omitted slot is not closed, and no interface id is hardcoded in place of the posted one |
| `obj` | collecting, main closed, a same-tile posted ground row whose actions carry `Take`: `{ x, z, level, name, action: 'Take' }`. The frozen shark id and every id this collect dropped are skipped, posted order decides, and the row's own posted name rides along |
| `held` | collecting with a full pack: one Drop of a food row, `{ name, action: 'Drop' }`, then the Take next call |
| `callback.log` | the settled Take line `took '<name>' from the casket`, or the pack-full `WARNING: …` |

Identify is still made on every collecting call, so a next scroll — or a
leftover casket, which Opens rather than collecting — leaves the collect and
re-arms the landed gate: no close and no Take ever runs under a step that is
still held. The pack's fullness is the posted `inv_size` against the occupied
positive-count rows; a page that did not post it is a `wait`, and the client's
28 is not invented for it. A process that is full with nothing droppable logs
the frozen WARNING and then finishes the same way; that is a log line through
`callback.log`, not a new `HelperResult` error. Finishing is always the landed
`none-held` abort — never `done`, never `'clue solved'`, never `abandon`.
`close-modal`, `obj` and `loc` stay off `V2_OPS`, so
`api.request({ op: 'close-modal' })` and `api.request({ op: 'obj' })` stay
`not impl`, and `api.snapshot.ground` stays hidden.

## Scene projections

Two sync `HelperResult` methods that copy the isolate-posted scene page. They
are not Promises, not `request()` ops, and they do not push `h.interact`. They
read `host().snapshot` directly: `api.snapshot` hides `locs` and `tick`, and a
missing host page is `snapshot-unavailable`, not `{ rows: [] }`.

| Method | OK | Errors |
| --- | --- | --- |
| `sceneLocs({ ids, limit, region? })` | `{ as_of_sequence, scene, rows, truncated }` | `invalid-args`, `missing-ids`, `snapshot-unavailable` |
| `sceneNpcs({ types, actions, limit, region? })` | `{ as_of_sequence, scene, rows, truncated }` | `invalid-args`, `snapshot-unavailable` |

Args are checked before the page, so a bad call is never
`snapshot-unavailable`. `sceneLocs()` and `sceneLocs([])` are `invalid-args`. A
missing or empty `ids` is `missing-ids` even when `limit` is also bad; a
non-array `ids` or a non-integer element is `invalid-args`, and `"2092"` is not
coerced. `sceneNpcs` requires a non-empty integer `types` array and a non-empty
string `actions` array: missing or empty is `invalid-args`, not `missing-ids`,
and an omitted `actions` is not match-any. `limit` is required and must be an
integer in `1..=64`; `65` is `invalid-args` and is not clamped. An omitted
`region` is no spatial filter (`ids`, or `types` plus `actions`, still bounds
the query). A present `region` must be a non-array object with integer `min_x`,
`min_z`, `max_x`, `max_z`, and `level`; `plane`, `null`, and
`{ cx, cz, radius }` are `invalid-args`. An inverted box is zero matches.

Each row is a new object `{ id, x, z, level, actions }` with a new `actions`
array. `id` is the posted loc id, or the posted NPC type including `-1`; a
caller `-1` is legal and matches only a posted `-1` row, not every NPC. A row
matches when its posted `id` equals any requested entry and its posted actions
contain any requested action (string equality, no trim and no case-fold,
any-of not all-of). Posted order is kept, nothing is sorted by distance,
matches are capped at `limit`, and more matches than `limit` set
`truncated: true`. An empty posted array with an available scene is zero
matches: `ok: true` with `rows: []` and `truncated: false`. A missing or
non-array `locs` / `npcs` key, a missing or non-number tick, and a collision
that is absent, non-object, or `available !== true` are `snapshot-unavailable`;
unavailable collision wins over an empty posted array, so `post_base` is not an
empty success.

`as_of_sequence` is `snapshot.tick`, not `api.tick`. A delta that omits the
vector keeps the previous array and stamps the current tick; there is no
per-vector sequence. `scene` copies `{ available, base_x, base_z, level, width,
height }`; collision flags stay on the page. Copies are historical: a retained
value stays usable after a later merge, it is not live, and an action needs a
fresh call.

Example: `crates/script/examples/scene_observe_v2.ts`.

## Quest status

One sync `HelperResult` method that copies one posted quest-tab row's already
resolved status string. It is not a Promise, not a `request()` op, and it does
not push `h.interact`. It reads `host().snapshot` directly: `api.snapshot`
hides `quest_statuses` and `tick`, and its getter substitutes `{}` for a
missing page.

| Method | OK | Errors |
| --- | --- | --- |
| `questStatus({ name })` | `{ status, as_of_sequence }` | `invalid-args`, `snapshot-unavailable`, `quest-tab-unbound`, `not-on-tab` |

Args are checked before the page, so a bad call is never
`snapshot-unavailable` and never `quest-tab-unbound`. `name` is required and
must be a string: no arguments, a non-object, `null`, an array, a missing
`name`, a non-string `name`, and a blank or whitespace-only name are all
`invalid-args`. A present `id` is `invalid-args` even beside a legal `name`;
other extra fields are ignored. `"death"`, `"314"`, and `"219"` are legal
names, not keys: this method does not consult `quest_identity()`, and a number
is `invalid-args`.

Matching is on the posted `name` only: both sides are trimmed and folded A-Z
only (no `toLowerCase`, and no folding of internal spaces). `Death Plateau` and
`death plateau` hit a posted row with that text, `  Death Plateau  ` hits, and
`DeathPlateau` and `death` do not. Posted order is kept and the first legal
match wins: a row whose `name` is not a string and a row whose `status` is not
exactly `notStarted`, `inProgress`, `complete`, or `unknown` are skipped, not
fatal to the page.

A page that was never posted, or whose snapshot is not an object, is
`snapshot-unavailable`; it does not throw and it is not an unbound tab. On a
posted page a missing `quest_statuses` key, and a value that is neither `null`
nor an array, are also `snapshot-unavailable`, while
`quest_statuses === null` is `quest-tab-unbound` (`post_base` is that null
tab). The null check runs before the tick check: a null tab with a missing or
non-finite tick is still `quest-tab-unbound`, and a bound array whose tick is
not a finite number is `snapshot-unavailable` even when a row would match.

Success is `{ status, as_of_sequence }`, not a bare string and not the live
row: `status` is the posted string, copied, and `as_of_sequence` is
`snapshot.tick` (never `api.tick`, and never a stamped `0`). No legal match is
`not-on-tab`, including on an empty posted array: `unknown` is a posted status,
not the answer to a miss, and the identity table is not joined to fill one. No
`component_id` and no colour integer are returned, the quest tab is not opened,
and no `interact` is queued. Copies are historical: a retained value stays
usable after a later merge, it is not live, and an action needs a fresh call.

Example: `crates/script/examples/quest_status_v2.ts`.

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
for a posted `size>=1` NPC, choosing min `(distance, index)` among those
rows, calls existing `api.lineOfSight` plus imported
`Npc.size` / `Npc.networkOrigin()` / `reader.selfTarget()` /
`Reachability.lineOfSight`, then named-stops. No size>=1 row is not
complete.

## Puzzle board

`snapshot.puzzle_board` is the open main modal's first depth-first TYPE_INV
with `obj_ops`: `{ component_id, size, items }`. `component_id` is `-1` and
`size` is `0` for a closed board — a present object, never a missing
property. `size` is the observed `link_obj_type` length and `items` are that
widget's own sparse rows (`name`, `count`, `id`, `ops`, `component_id`,
`slot`), so an empty slot contributes no row. The rows carry the component's
own `ops`, not the held-item table a click is sent with.

`snapshot.puzzle_board_generation` is the broad session generation: it
advances on a session open, a session close or a new board component, never
on a piece move. Pass it into `puzzle-move`; a changed generation is stale,
not a solved board.

## Sync and async tick

A v2 `tick` may be async. The isolate will not re-enter `tick` while that
Promise is pending. Snapshot posts still merge. Prefer a **sync** tick that
polls `api.snapshot` across game ticks.

Pause / Stop / disconnect / reload drop queued actions of the old
generation. Settings updates replace the bag for later ticks. Per-profile
settings stay isolated by card identity.
