--!strict
-- The Luau workload of the release memory A/B (`BOT_MEMORY_SCENARIO=luau_gatherer`).
-- Each bot runs the native Gatherer session through `api.gather.run` with the
-- settings of the `gatherer` scenario, and paints an overlay on each frame.
manifest {
    apiVersion = 2,
    name = "Memory Luau Gatherer",
    version = "1.0.0",
    author = "274bot",
    description = "Chop the Draynor trees through the native Gatherer session.",
    category = "Woodcutting",
    tags = { "memory" },
    settings = {},
    capabilities = {},
}

-- The settings that the `gatherer` scenario injects into the native card.
local GATHER = {
    skill = "Woodcutting",
    location = "Start",
    woodcuttingResources = { "normal" },
    radius = 12,
    disposition = "Power",
}

local sessions = 0
local line = "starting"

return {
    tick = function(api)
        sessions += 1
        line = string.format("session %d: gathering", sessions)
        local outcome = api.gather.run(GATHER)
        line = string.format("session %d: %s", sessions, outcome.kind)
        api.stop("the Gatherer session ended: " .. tostring(outcome.reason or outcome.kind))
    end,
    onPaint = function(ctx)
        local x, z = api.status.tile()
        ctx.fillStyle = "rgba(0, 0, 0, 0.6)"
        ctx:fillRect(4, 4, 220, 40)
        ctx.fillStyle = "#ffffff"
        ctx:fillText(line, 10, 20)
        ctx:fillText(string.format("tick %d at %d, %d", api.status.tick(), x or 0, z or 0), 10, 36)
    end,
}
