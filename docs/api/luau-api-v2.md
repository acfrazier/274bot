# Luau API v2 reference

> **Experimental.** The Luau runtime is opt-in and can change in a later
> release. See [writing-luau-scripts.md](writing-luau-scripts.md) for the
> warning, the manifest and runnable examples.

This page lists every member of the `api` table in the Luau runtime and every
table that a member takes or returns. The
[declaration file](luau-api-v2.d.luau) holds the same surface as Luau types. A
test in `script-luau` compares the declaration file and the member tables on
this page with the runtime, so a member that is not here does not exist. The
JavaScript counterpart is [js-api-v2.md](js-api-v2.md).

## Call classes

Each member belongs to one class. The class tells you when you may call it and
whether it waits.

| Class | Behavior |
| --- | --- |
| Read | Reads the game state that the host lends to `tick` and `onPaint`. It never waits. Outside those two entries it raises `game input is lent only to tick and onPaint`. |
| Request | Makes one host call and returns at once. The host sends the queued action before the next game tick. It works only inside `tick`. |
| Await | Makes one host call and suspends `tick` until the host settles it. It works only inside `tick`. |
| Output | Writes script output. It works in every entry. |

`api.settings` members belong to the Request class but work in every entry,
because they read the settings of the run and no game state.

A member refuses more arguments than its signature has, and raises a Luau error
that starts with the member name. A text argument may hold at most 64 KiB.
Every number argument must be a number, never a string. Where the table says
"integer", a number with a fraction is refused.

## Output and control

| Member | Class | What it does |
| --- | --- | --- |
| `api.log(...)` | Output | Writes one log line: the values joined by tabs. A `nil` prints as `nil` and a table as `<table>`. The line may hold 4,096 bytes and 256 values. |
| `api.stop(reason?)` | Request | Stops the script. The entry ends at the call and `onStop` runs once. The reason is text of at most 256 bytes, and the default is `"stopped"`. It works in `tick` and `onPaint`. |
| `api.wait()` | Await | Suspends `tick` until the next game tick that is not held. It returns nothing. |

## Settings

| Member | Class | What it does |
| --- | --- | --- |
| `api.settings.str(key, fallback?)` | Request | The text setting `key`, or `fallback`, or `""`. |
| `api.settings.num(key, fallback?)` | Request | The number setting `key`, or `fallback`, or `0`. |
| `api.settings.bool(key, fallback?)` | Request | The boolean setting `key`, or `fallback`, or `false`. |

Each reader returns the fallback when the saved value has another type. The
settings come from the manifest and the values that the operator saved.

## Status

All status members are Read members with no argument.

| Member | Returns |
| --- | --- |
| `api.status.tick()` | The game tick of the lent state. |
| `api.status.inGame()` | `true` when the client is in the game. |
| `api.status.tile()` | The player's `x`, `z` and `level` as three numbers, or nothing when the host has no tile. |
| `api.status.runEnergy()` | The run energy. |
| `api.status.runEnabled()` | `true` when run is on. |
| `api.status.inCombat()` | `true` when the player is in combat. |
| `api.status.animating()` | `true` when the player animates. |
| `api.status.name()` | The player's name, or `nil`. |

## Walking

| Member | Class | What it does |
| --- | --- | --- |
| `api.walk.to(x, z, level, options?)` | Await | Walks to the exact tile and returns a walk result. |
| `api.walk.near(x, z, level, radius, options?)` | Await | Walks to within `radius` tiles and returns a walk result. |
| `api.walk.nearestBank()` | Request | Queues a walk to the nearest bank stand. |

`options` is a table with the optional booleans `allow_teleports`,
`allow_wilderness` and `allow_bank_fetch`. Each is `false` when you omit it.

A walk result has these fields:

| Field | Meaning |
| --- | --- |
| `ok`, `failed` | `ok` is `not failed`. |
| `x`, `z`, `level`, `radius` | The tile and radius that the walk settled on. |
| `blocked`, `no_path` | The host found the end tile blocked, or found no route. |
| `allow_teleports` | Whether teleports were allowed. |
| `cancel_reason` | `"none"` or `"user-input"`. Moving the character by hand cancels a walk. |
| `request_id`, `generation`, `seq` | The host's numbers for the request. |
| `aborted` | Only on a walk that a session reset ended: `"reset"`, `"superseded"`, `"terminated"`, `"user-input"` or `"parent-aborted"`. The position fields are then missing. |

## Banking

| Member | Class | What it does |
| --- | --- | --- |
| `api.bank.openNearest()` | Await | Opens the nearest bank and returns a bank result. |
| `api.bank.depositAll()` | Await | Deposits every bankable backpack item and returns a bank result. |
| `api.bank.close()` | Await | Closes the bank and returns a bank result. |
| `api.bank.openBooth(x, z, level, id, options?)` | Request | Queues opening the bank booth with loc id `id` at the tile. `options` may hold `name` and `action`. |
| `api.bank.openStand(x, z, level, kind, options?)` | Request | Queues opening a bank stand. `kind` is `"booth"` or `"npc"`. `options` may hold `name`, `stand_op` and `choose`. |
| `api.bank.closeNow()` | Request | Queues closing the bank interface at once. |
| `api.bank.setNoteMode(on)` | Request | Queues the withdraw-as-note switch. `on` is a boolean. |
| `api.bank.deposit(name)` | Request | Queues depositing every copy of the named item. |
| `api.bank.withdraw(name, action)` | Request | Queues withdrawing the named item with the bank's own option text, for example `"Withdraw-All"`. |
| `api.bank.withdrawX(request)` | Request | Queues a counted withdrawal. The table needs `name`, `count`, `bank_item_id`, `lands_as_id`, `action` and `bank_generation`. |
| `api.bank.generation()` | Read | The current bank session generation, for `withdrawX`. |

A bank result has `status`, which is `"done"`, `"failed"`, `"refused"` or
`"aborted"`, and `has_value`. It has `value` when `has_value` is true and
`reason` when the status is `"refused"` or `"aborted"`.

## Interaction

| Member | Class | What it does |
| --- | --- | --- |
| `api.interact.npc(name, action, index?)` | Request | Queues an action on an NPC by name, at an exact index when you give one. |
| `api.interact.loc(x, z, level, action, id)` | Request | Queues an action on the loc with `id` at the exact tile. |
| `api.interact.obj(x, z, level, action, id)` | Request | Queues an action on the ground item with `id` at the exact tile. |
| `api.interact.player(name, action)` | Request | Queues an action on a player by name. |
| `api.interact.held(name, action, slot, target_item_id)` | Request | Queues a held-item action by exact slot and target id. |
| `api.interact.heldByName(name, action)` | Request | Queues a held-item action by item name and action text. |
| `api.loc.interact(tile, action)` | Request | Queues the loc option at the exact tile, with no id fallback. `tile` is `{ x, z, level }`. |
| `api.inventory.option(name, option)` | Request | Queues option 1 to 5 of the named carried item. |
| `api.inventory.useOn(source, target)` | Request | Queues using one carried item on another. |
| `api.equipment.wear(name)` | Request | Queues wearing the named carried item. |
| `api.equipment.remove(name)` | Request | Queues removing the named worn item. |
| `api.dialog.continue()` | Request | Queues the Continue option of the open dialog. |
| `api.dialog.answer(option)` | Request | Queues a dialog option. The first option is `1`. |
| `api.interface.button(component_id)` | Request | Queues a click on an interface component. |

`api.inventory.*` and `api.equipment.*` resolve the item name against the
backpack or worn items that the host lends. A name that matches no item, or
two different items, is refused.

## Routes and puzzles

| Member | Class | What it does |
| --- | --- | --- |
| `api.route.inspect(from, to, options?)` | Request | Queues one route query. A tile is `{ x, z, level }`, and a missing `level` is `0`. |
| `api.route.result()` | Read | The latest route result, or `nil`. |
| `api.puzzle.move(id, slot, component, generation)` | Request | Queues sliding one piece of the open puzzle board. |
| `api.puzzle.board()` | Read | The open puzzle board, or `nil`. |

`options` of `api.route.inspect` may hold `allow_teleports`,
`allow_wilderness`, `allow_bank_fetch`, `max_expansions` and `avoid`. `avoid`
is a list of at most 16 rectangles `{ minX, maxX, minZ, maxZ, level? }`.

A route result has `seq`, `generation`, `request_id`, `ok`, `reason`,
`bank_planned`, `ticks` and `hops`. Each hop has `kind`, `locId`, `locName`,
`action`, `option`, `ticks`, `from` and `to`.

A puzzle board has `component_id`, `size`, `generation` and `items`. Each item
has `id`, `slot`, `count` and `component_id`.

## Sessions

The session members run a native card to its end and return an outcome table.

| Member | Class | What it does |
| --- | --- | --- |
| `api.combat.fight(request)` | Await | Runs one combat session and returns a combat outcome. |
| `api.gather.run(settings?)` | Await | Runs one Gatherer session and returns a gather outcome. |
| `api.questProgress(quest)` | Await | Reads the progress of one quest and returns a quest progress outcome. |
| `api.questStatus(name)` | Read | Reads one row of the quest tab. |
| `api.questPaths()` | Read | Lists the released quest paths. |

Every outcome has a `kind`:

| Kind | Fields |
| --- | --- |
| `"done"` | `value`, a table whose `["end"]` field says how the session ended. |
| `"refused"` | `reason`. The host refused the request, for example with `busy`. |
| `"aborted"` | `reason`: `"reset"`, `"superseded"`, `"terminated"`, `"user-input"` or `"parent-aborted"`. |

Use `value["end"]` with brackets, because `end` is a Luau keyword.

**Combat.** The request table has a required `target` with exactly one of
`npc` (a name or a list of names), `npcId` (an id or a list of ids),
`attackers` (`"npcs"`, `"players"` or `"any"`) and `player` (a name). The other
keys are optional: `pick`, `notTargetingOthers`, `area`, `radius`,
`lostRadius`, `style`, `meleeMode`, `rangedMode`, `spells`, `fallbackSpells`,
`prayer`, `food`, `potions`, `retaliate`, `budgetTicks` and `pvp`. `area` is a
box `{ min_x, min_z, max_x, max_z, level }` or a list of boxes. The value ends
as `"fought"` (with `report`), `"stopped"`, `"interrupted"` (with `cause`),
`"refused"` or `"failed"` (with `reason`).

**Gatherer.** The settings table accepts the Gatherer keys `skill`,
`woodcuttingResources`, `miningResources`, `fishingMethod`, `targetPreference`,
`tickMethod`, `location`, `site`, `customTile`, `radius`, `disposition`,
`bank`, `baitTarget`, `buyTools`, `food`, `foodTarget`, `eatBelow`,
`coinTarget`, `reserveTeleport`, `reserveCasts`, `allowTeleports`,
`allowWilderness`, `allowDangerZones`, `deathPolicy` and `maxDeaths`. An
unknown key is refused. The value ends as `"stopped"`, `"blocked"` (with
`failure` and `counts`), `"refused"` or `"failed"`. `counts` has `yielded`,
`dropped`, `deposited`, `trips` and `xp`.

**Quests.** `api.questStatus` returns `ok` with `status` (`"notStarted"`,
`"inProgress"`, `"complete"` or `"unknown"`) and `as_of_sequence`, or `ok` as
`false` with `error`: `"invalid-args"`, `"snapshot-unavailable"`,
`"quest-tab-unbound"` or `"not-on-tab"`. `api.questPaths` returns a list of
rows with `id`, `display`, `journal`, `draft` and `stages`.
`api.questProgress` ends as `"done"` with a `row` or as `"refused"`.

## Paint canvas

`onPaint(ctx)` receives a context that records drawing operations. Call methods
with a colon. The context works only inside the call, and an old context
raises `canvas: onPaint context has expired`.

| Group | Members |
| --- | --- |
| Rectangles and text | `fillRect(x, y, w, h)`, `fillText(text, x, y)`, `measureText(text)`. |
| Paths | `beginPath()`, `closePath()`, `moveTo(x, y)`, `lineTo(x, y)`, `quadraticCurveTo(cpx, cpy, x, y)`, `arc(x, y, radius, start, end, counterclockwise?)`, `fill()`, `stroke()`, `clip()`. |
| State | `save()` and `restore()`. |
| Gradients | `createLinearGradient(x0, y0, x1, y1)` and `createRadialGradient(x0, y0, r0, x1, y1, r1)` return a gradient. `gradient:addColorStop(offset, color)` adds a stop. |
| Text properties | `fillStyle`, `strokeStyle`, `shadowColor`, `font`, `lineJoin`, `textAlign` and `textBaseline`. They hold strings. `fillStyle` may also hold a gradient of the same call. |
| Number properties | `lineWidth`, `shadowBlur`, `shadowOffsetX` and `shadowOffsetY`. |

`measureText` returns a table with `width`. Colors and fonts are CSS strings,
as in `"#40c040"`, `"rgba(0, 0, 0, 0.6)"` and `"12px sans-serif"`. One frame
holds at most 256 operations, and one text string at most 512 bytes. An error
in `onPaint`, such as a broken bound, does not stop the script. The frame shows
the message in red instead of your drawing. Any other property or method raises
`not impl: Canvas.<name>`. `fillText` with a fourth argument raises
`not impl: Canvas.fillText.maxWidth`.

## What the runtime does not have

The Luau runtime has no `api.hunt` namespace, no packet member, no catalog
helper compatibility and no snapshot or query tables. It has no `api.paint`
chrome (title, rows, buttons): a script draws with `onPaint`. A manifest
cannot ask for a capability. File scenarios use a separate `scenario` table
that is described in [luau-scenarios.md](../dev/luau-scenarios.md).
