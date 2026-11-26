--!strict
-- The stated workload of slice L7 (ADR 0023 item 32, Promotion Condition 2).
-- A cow-field fighter that banks its loot. It uses each kind of binding that a
-- real bot uses on each trip: settings, status, quest, walk, combat,
-- interaction and bank requests, a log line, and a paint overlay on each
-- frame. Its own state is bounded: a ring of the last 256 tiles, a fixed
-- route and a few counters. `support/workload.rs` drives it.
manifest {
    apiVersion = 2,
    name = "L7 Cow Fighter",
    version = "1.0.0",
    author = "274bot",
    description = "Fight cows, take the hides, bank them, walk back.",
    category = "Combat",
    tags = { "combat", "banking" },
    settings = {
        target = { type = "string", default = "Cow", label = "Target" },
        loot = { type = "string", default = "Cowhide", label = "Loot" },
        lootId = { type = "number", default = 1739, min = 1, max = 65535 },
        tripKills = { type = "number", default = 4, min = 1, max = 28 },
        paintRoute = { type = "boolean", default = true },
        quest = { type = "string", default = "Cook's Assistant" },
    },
    capabilities = {},
}

local FIELD = { min_x = 3240, min_z = 3254, max_x = 3266, max_z = 3299, level = 0 }
local ROUTE_LENGTH = 32
local ROUTE = table.create(ROUTE_LENGTH)
for index = 1, ROUTE_LENGTH do
    ROUTE[index] = { x = 3208 + index, z = 3220 + (index * 3) % 60, level = 0 }
end
local LEG = 4
local HISTORY = 256
local history = table.create(HISTORY, 0)
local head = 0

local counters = { ticks = 0, kills = 0, trips = 0, walks = 0, banks = 0, failures = 0 }
local phase = "walk"
local leg = 0
local waypoint = 1
local line = "starting"
local detail = ""
local distinct = 0

local function remember(x: number, z: number, level: number)
    head = head % HISTORY + 1
    history[head] = x * 65536 + z * 4 + level
end

local function countDistinct(): number
    local seen = {}
    local count = 0
    for index = 1, HISTORY do
        local packed = history[index]
        if packed ~= 0 and not seen[packed] then
            seen[packed] = true
            count += 1
        end
    end
    return count
end

local function walkLeg(api)
    local point = ROUTE[waypoint]
    local result = api.walk.to(point.x, point.z, point.level)
    counters.walks += 1
    if result.ok then
        waypoint = waypoint % ROUTE_LENGTH + 1
        leg += 1
        if leg >= LEG then
            leg = 0
            phase = "fight"
        end
    else
        counters.failures += 1
    end
end

local function fight(api, x: number?, z: number?)
    local outcome = api.combat.fight({
        target = { npc = { api.settings.str("target", "Cow") } },
        pick = "lowestHealth",
        area = FIELD,
        radius = 10,
        lostRadius = 18,
        style = "melee",
        meleeMode = "aggressive",
        food = true,
        retaliate = true,
        budgetTicks = 250,
    })
    if outcome.kind == "done" and outcome.value["end"] == "fought" then
        counters.kills += 1
        api.interact.obj(x or FIELD.min_x, z or FIELD.min_z, 0, "Take", api.settings.num("lootId", 1739))
        if counters.kills % api.settings.num("tripKills", 4) == 0 then
            phase = "bank"
        end
    else
        counters.failures += 1
    end
end

local function bank(api)
    local opened = api.bank.openNearest()
    if opened.status == "done" then
        api.bank.depositAll()
        api.bank.close()
        counters.banks += 1
        counters.trips += 1
    else
        counters.failures += 1
    end
    phase = "walk"
end

return {
    tick = function(api)
        counters.ticks += 1
        local x, z, level = api.status.tile()
        if x then
            remember(x, z, level)
        end
        local quest = api.questStatus(api.settings.str("quest", "Cook's Assistant"))
        detail = string.format(
            "%s %s | run %d%% | %s",
            api.settings.str("loot", "Cowhide"),
            quest.ok and quest.status or quest.error,
            api.status.runEnergy(),
            api.status.inCombat() and "in combat" or "idle"
        )
        if phase == "walk" then
            walkLeg(api)
        elseif phase == "fight" then
            fight(api, x, z)
        else
            bank(api)
        end
        if counters.ticks % 64 == 0 then
            distinct = countDistinct()
        end
        line = string.format(
            "%s | kills %d | trips %d | tick %d",
            phase,
            counters.kills,
            counters.trips,
            api.status.tick()
        )
        if counters.ticks % 100 == 0 then
            api.log(line, detail)
        end
    end,
    onPaint = function(ctx)
        ctx.fillStyle = "rgba(0, 0, 0, 0.6)"
        ctx:fillRect(4, 4, 260, 72)
        ctx.fillStyle = "#ffffff"
        ctx.font = "12px sans-serif"
        ctx:fillText(line, 10, 20)
        ctx:fillText(detail, 10, 36)
        ctx:fillText(
            string.format(
                "walks %d | banks %d | failures %d | tiles %d",
                counters.walks,
                counters.banks,
                counters.failures,
                distinct
            ),
            10,
            52
        )
        if api.settings.bool("paintRoute", true) then
            ctx.strokeStyle = "#40c040"
            ctx.lineWidth = 1
            ctx:beginPath()
            for index = 1, ROUTE_LENGTH do
                local point = ROUTE[index]
                local px = 10 + (point.x - 3208) * 7
                local py = 60 + (point.z - 3220) / 6
                if index == 1 then
                    ctx:moveTo(px, py)
                else
                    ctx:lineTo(px, py)
                end
            end
            ctx:stroke()
        end
    end,
    onStop = function()
        api.log("stopped", counters.ticks, counters.kills, counters.trips, counters.failures)
    end,
}
