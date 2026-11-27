# Writing Luau scripts

> **Experimental.** Luau scripts are off until you turn on **Use experimental
> Luau script runtime** in Settings. Until then no Luau script appears in the
> script picker and no Luau code runs. A script runs in a sandbox: it has no
> file, operating-system, network or process access, only the 274bot script
> API. 274bot stops a script that uses more than 16 MiB of memory or that does
> not yield in time, and the card shows the reason. This runtime can change in
> a later release, and scripts may not behave as expected. A script controls
> your character, so run only scripts that you wrote or that you trust.

Use this guide to write a script for the Luau runtime. It shows the file shape,
the manifest, the entry functions and five runnable examples. The
[Luau API v2 reference](luau-api-v2.md) lists every `api` member. The
[declaration file](luau-api-v2.d.luau) types the whole surface for a type
checker. For the JavaScript and TypeScript runtime, see
[writing-scripts.md](writing-scripts.md).

## Turn on the runtime and load a file

1. Open Settings in the panel or the terminal app.
2. Turn on **Use experimental Luau script runtime**. A first-use warning
   appears. Read it and confirm.
3. In the **Scripts** window, use **Load** and pick your `.luau` file.

The script appears as a file card beside the JavaScript file cards. The card
shows the name, description, category and settings from your manifest. The host
does not load a script whose manifest it refuses. Turn the setting off to hide
every Luau card. 274bot keeps your file references and settings.

A build without the Luau runtime shows the message "This build does not include
the Luau script runtime." Turning the setting off shows "Luau scripts are
turned off. Turn on 'Use experimental Luau script runtime' in Settings to run
this script." when you start a saved Luau card.

## File shape

A Luau script file has three parts, in this order:

1. The statement `manifest { ... }`. It must be the first statement of the file.
2. Your own code: locals, helper functions and state.
3. A `return` of one table. The table has a `tick` function, and may have an
   `onPaint` function and an `onStop` function.

<!-- ci-example: quickstart-luau -->
```luau
manifest {
    apiVersion = 2,
    name = "Quick start",
    version = "1",
    author = "Example author",
    description = "Log a setting value and stop.",
    category = "Examples",
    tags = {},
    settings = {
        count = {
            type = "number",
            default = 3,
            min = 1,
            max = 10,
            label = "Count",
        },
    },
    capabilities = {},
}

return {
    tick = function(api: Api)
        local count = api.settings.num("count", 3)
        api.log("Count:", count)
        api.stop("Quick start complete. Count=" .. count)
    end,
    onStop = function(reason: string)
        api.log("Stopped:", reason)
    end,
}
```

The names `Api` and `PaintContext` in the examples come from the
[declaration file](luau-api-v2.d.luau). Luau removes type annotations before
a script runs, so the names need no import. The scripts also run without them.

A Rust test extracts every example on this page, reads its manifest, checks the
setting schema and runs the example on the real Luau runtime. It needs no game
data. See
[`authoring_guide.rs`](../../crates/script-luau/tests/authoring_guide.rs).

## The manifest

The host reads the manifest without starting a Luau VM, so a card can show your
script before it runs. The reader accepts a closed grammar of literals only.
The host replaces the statement with spaces before it compiles the file, so
error line numbers stay the numbers in your file. Do not read `manifest` at run
time: it does not exist there.

`manifest` is followed by one table in braces. The table may hold only
strings, decimal numbers, `true`, `false` and nested tables. It may not hold
`nil`, names, calls, functions, operators, backtick strings, long-bracket
strings or hexadecimal numbers. A key is a name or a quoted string in brackets.
A table has `key = value` fields or positional items, not both. Comments,
`;` separators and both quote styles are allowed.

All nine fields are required:

| Field | Required value |
| --- | --- |
| `apiVersion` | The integer `2`. |
| `name` | Non-empty text. |
| `version` | Non-empty text. |
| `author` | Non-empty text. |
| `description` | Non-empty text. |
| `category` | Non-empty text. |
| `tags` | A list of unique, non-empty strings. Use `{}` for none. |
| `settings` | A table keyed by setting id. Use `{}` for none. |
| `capabilities` | The empty table `{}`. |

Text has no space, tab or line break at either end and no control character.
The file may hold at most 256 KiB. `capabilities` must be empty: the Luau API
has no packet member, so `"host-packet-protocol"` is refused.

### Settings

Each setting is a table with a `type` and a `default` of the same type. The
types are `"boolean"`, `"number"`, `"string"`, `"string[]"` and `"tile"`. A
tile default is `{ x = 3222, z = 3218, level = 0 }` with integer values and a
level from 0 to 3.

| Field | Use |
| --- | --- |
| `label`, `group`, `help` | Display text. |
| `min`, `max` | Bounds of a `number` setting. `min` may not exceed `max`, and the default must lie inside them. |
| `options` | A non-empty list of unique strings for a `string` or `string[]` setting. Each default must be one of them. |
| `optionLabels` | Display text for options. Each key must be an option. |
| `optionsFrom` | `"loadouts"` or `"named-banks"`. Do not combine it with `options`. |
| `showIf` | `{ key = "other", anyOf = { "a", "b" } }` shows the setting when the string setting `other` has one of those values. |

An unknown field is refused. A script reads its settings with `api.settings.str`,
`api.settings.num` and `api.settings.bool`. These three readers return the
fallback when the saved value has another type. There is no reader for
`string[]` and `tile` values.

### When the manifest is refused

The host refuses a bad manifest with a stable code, the field path and the line
and column of the cause. These three examples show the most common mistakes.

<!-- ci-refusal: luau-manifest-nonliteral -->
```luau
manifest {
    apiVersion = 2,
    name = "Bad " .. "name",
}
```

<!-- ci-refusal: luau-capability-unsupported -->
```luau
manifest {
    apiVersion = 2,
    name = "Bad capability",
    version = "1",
    author = "Example author",
    description = "Asks for a capability that Luau does not have.",
    category = "Examples",
    tags = {},
    settings = {},
    capabilities = { "host-packet-protocol" },
}
```

<!-- ci-refusal: manifest-required-field -->
```luau
manifest {
    apiVersion = 2,
    name = "No author",
    version = "1",
    description = "The author field is missing.",
    category = "Examples",
    tags = {},
    settings = {},
    capabilities = {},
}
```

## The entry functions

`tick(api)` is required. `onPaint(ctx)` and `onStop(reason)` are optional.
`api` is also a global, so `onPaint` and `onStop` can use it.

| Entry | When it runs | What it can call |
| --- | --- | --- |
| `tick(api)` | Once for each game tick that is not held. | Every `api` member. |
| `onPaint(ctx)` | After `tick`, once for each game tick, when you define it. | Reads, `api.settings`, `api.log`, `api.stop` and the canvas. |
| `onStop(reason)` | Once when the script ends. The reason is the text of `api.stop`, or `"stopped"`. | `api.log` and `api.settings`. |

A `tick` can wait. An awaited member such as `api.walk.to` or
`api.bank.openNearest` suspends the tick until the host settles the request,
and `api.wait()` suspends it until the next game tick. While a tick is
suspended, the host does not start another one. When the tick function returns,
the next game tick starts it again from the top, so keep state in locals
outside the function.

`api.stop(reason)` ends the script. It does not return, so no later statement
of that entry runs. An error that you do not catch in `tick` ends only that
tick. The card shows `tick <number>: <message>` with the line, and the host
starts `tick` again at the next game tick. An error in `onPaint` ends only that
frame: the frame shows the message in red. Call `api.stop` to end the script.

### Walk and read the player

This example reads the player tile, walks near a tile and stops when it arrives.
The walk is awaited: the script resumes with a result table.

<!-- ci-example: walk-status-luau -->
```luau
manifest {
    apiVersion = 2,
    name = "Walk to a tile",
    version = "1",
    author = "Example author",
    description = "Walk near a tile, then stop.",
    category = "Examples",
    tags = { "walking" },
    settings = {
        x = { type = "number", default = 3222, min = 0, max = 16383, label = "Tile x" },
        z = { type = "number", default = 3218, min = 0, max = 16383, label = "Tile z" },
        radius = { type = "number", default = 2, min = 0, max = 20, label = "Radius" },
    },
    capabilities = {},
}

return {
    tick = function(api: Api)
        if not api.status.inGame() then
            return
        end
        local x, z, level = api.status.tile()
        if x == nil or z == nil or level == nil then
            return
        end
        local targetX = api.settings.num("x", 3222)
        local targetZ = api.settings.num("z", 3218)
        local radius = api.settings.num("radius", 2)
        if math.abs(x - targetX) <= radius and math.abs(z - targetZ) <= radius then
            api.stop("Arrived")
        end
        local result = api.walk.near(targetX, targetZ, level, radius)
        if result.ok then
            api.log("Walk finished at", result.x, result.z)
        else
            api.log("Walk failed. No path:", result.no_path, "Blocked:", result.blocked)
        end
    end,
}
```

### Draw on the game image

`onPaint(ctx)` receives a canvas context. Call its methods with a colon, as in
`ctx:fillRect(4, 4, 220, 44)`, and set its properties with a dot, as in
`ctx.fillStyle = "#ffffff"`. The canvas draws on the game image, which is
765 by 503 pixels. One frame records at most 256 drawing operations, and one
text string may hold at most 512 bytes. `fillText` takes no width limit. The
context is valid only inside the call: do not store it.

<!-- ci-example: paint-overlay-luau -->
```luau
manifest {
    apiVersion = 2,
    name = "Status overlay",
    version = "1",
    author = "Example author",
    description = "Draw the player tile and run energy on the game image.",
    category = "Examples",
    tags = { "overlay" },
    settings = {
        showEnergy = { type = "boolean", default = true, label = "Show run energy" },
        accent = { type = "string", default = "#40c040", label = "Accent colour", group = "Colours" },
    },
    capabilities = {},
}

local line = "Waiting for the game"
local energy = 0

return {
    tick = function(api: Api)
        local x, z, level = api.status.tile()
        if x ~= nil and z ~= nil and level ~= nil then
            line = string.format("Tile %d, %d (level %d)", x, z, level)
        end
        energy = api.status.runEnergy()
    end,
    onPaint = function(ctx: PaintContext)
        ctx.fillStyle = "rgba(0, 0, 0, 0.6)"
        ctx:fillRect(4, 4, 220, 44)
        ctx.fillStyle = "#ffffff"
        ctx.font = "12px sans-serif"
        ctx:fillText(line, 10, 20)
        if api.settings.bool("showEnergy", true) then
            ctx.fillStyle = api.settings.str("accent", "#40c040")
            ctx:fillText(string.format("Run energy %d%%", energy), 10, 38)
        end
    end,
}
```

The [reference](luau-api-v2.md#paint-canvas) lists every canvas method and
property.

### Bank and interact

A bank trip uses awaited bank members and queued requests. `api.bank.openNearest`,
`api.bank.depositAll` and `api.bank.close` return a result table with a
`status` of `"done"`, `"failed"`, `"refused"` or `"aborted"`. A request member
such as `api.bank.withdraw` or `api.interact.heldByName` returns at once. The
host sends it before the next game tick, so call `api.wait()` before the next
step that depends on it.

<!-- ci-example: bank-interact-luau -->
```luau
manifest {
    apiVersion = 2,
    name = "Bank and bury",
    version = "1",
    author = "Example author",
    description = "Bank the backpack, take out bones and bury them.",
    category = "Examples",
    tags = { "banking", "prayer" },
    settings = {
        item = { type = "string", default = "Bones", label = "Item to bury" },
    },
    capabilities = {},
}

return {
    tick = function(api: Api)
        local item = api.settings.str("item", "Bones")
        local opened = api.bank.openNearest()
        if opened.status ~= "done" then
            api.stop("Bank did not open: " .. opened.status)
        end
        api.bank.depositAll()
        api.bank.withdraw(item, "Withdraw-All")
        api.wait()
        local closed = api.bank.close()
        if closed.status ~= "done" then
            api.stop("Bank did not close: " .. closed.status)
        end
        api.interact.heldByName(item, "Bury")
        api.wait()
        api.stop("Banked and buried " .. item)
    end,
}
```

### Run a gatherer session and read a quest

`api.gather.run` runs one native Gatherer session and returns when it ends. The
table you pass uses the same keys as the Gatherer settings, and every key is
optional. `api.questStatus` reads one row of the quest tab and never waits.

<!-- ci-example: gather-quest-luau -->
```luau
manifest {
    apiVersion = 2,
    name = "Gather and report",
    version = "1",
    author = "Example author",
    description = "Run a Woodcutting session and report the result.",
    category = "Examples",
    tags = { "gathering" },
    settings = {
        radius = { type = "number", default = 12, min = 2, max = 64, label = "Radius" },
        bank = { type = "boolean", default = true, label = "Bank the logs" },
        quest = { type = "string", default = "Cook's Assistant", label = "Quest to report" },
    },
    capabilities = {},
}

return {
    tick = function(api: Api)
        local quest = api.questStatus(api.settings.str("quest", "Cook's Assistant"))
        if quest.ok then
            api.log("Quest", quest.status)
        else
            api.log("Quest unknown:", quest.error)
        end
        local outcome = api.gather.run({
            skill = "Woodcutting",
            radius = api.settings.num("radius", 12),
            disposition = if api.settings.bool("bank", true) then "Bank" else "Power",
            buyTools = false,
        })
        local ended = outcome.value
        if outcome.kind == "done" and ended ~= nil then
            local counts = ended.counts
            if counts ~= nil then
                api.log("Gatherer", ended["end"], "yielded", counts.yielded, "xp", counts.xp)
            else
                api.log("Gatherer", ended["end"], ended.reason)
            end
        else
            api.log("Gatherer did not run:", outcome.kind, outcome.reason)
        end
        api.stop("Session ended")
    end,
}
```

A combat session works the same way: `api.combat.fight` takes a request table
and returns an outcome table. The [reference](luau-api-v2.md#sessions) shows the
fields.

## The sandbox

A script runs in a Luau VM that has only a fixed list of globals: `api`,
`assert`, `bit32`, `buffer`, `error`, `getmetatable`, `ipairs`, `math`, `next`,
`pairs`, `pcall`, `print`, `rawequal`, `rawget`, `rawlen`, `rawset`, `select`,
`setmetatable`, `string`, `table`, `tonumber`, `tostring`, `type`, `typeof`,
`utf8` and `xpcall`. There is no `io`, `os`, `debug`, `coroutine`, `require`,
`load`, `loadstring`, `dofile` or `collectgarbage`. `print` writes a log line,
like `api.log`.

Limits that the host enforces:

| Limit | Value |
| --- | --- |
| Source file | 256 KiB. |
| Memory of the VM | 16 MiB. |
| Top-level code when the script loads | 50 ms. |
| One `tick` and `onPaint` together, between two waits | 600 ms. |
| `onStop` | 50 ms. |
| One log line | 4,096 bytes. A script may write 256 lines for each entry. |
| One stop reason | 256 bytes. |

A script that breaks a limit stops. The card shows a code such as `luau-memory`
or `luau-watchdog` and the reason. A script that never yields, for example in a
`while true do end` loop, is stopped by the 600 ms limit.

Arguments are checked. A number argument must be a number: the string `"3200"`
is refused, and so is `3200.5` where an integer is required. A refusal raises a
Luau error that names the member, for example `api.walk.to: x must be a 32-bit
integer`. Use `pcall` to catch it.

## Check a script with types

The [declaration file](luau-api-v2.d.luau) declares `manifest`, `api`,
`PaintContext` and every result type. A Luau language server that loads
definition files can use it. Annotate the entry functions (`api: Api`,
`ctx: PaintContext`) so the checker knows their types.

The checker does not flag every manifest mistake: it accepts a capability
entry or an unknown setting field that the host then refuses. The runtime has
no `api.hunt` and no packet member. A call to a member that is
not in the reference raises an error when it runs. A test in `script-luau`
compares the declaration file with the member table of the runtime, so the
file lists exactly the members that exist.
