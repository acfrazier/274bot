scenario_header {
    name = "file-seed-pack",
    revision = 289,
    cases = { "pack-give" },
    accounts = { main = { mainland = true, preamble = "tutorial" } },
    deadline_s = 180,
    idle_ticks = 120,
    seeds = "confirmed-only",
    limits = { "Pack operation is own-account and established-state only." },
}

local function tick(api)
    scenario.step("Prepare", "A reviewed pack operation seeds carried coins.")
    scenario.seed.teleport { x = 3222, z = 3218, level = 0, why = "Courtyard start." }
    scenario.seed.ext { op = "give_item", item = "coins", count = 5, why = "Exercise the typed pack path." }

    scenario.phase("product")
    scenario.step("Walk", "A normal root that closes by its own arrival.")
    local m = scenario.mark("walk")
    api.walk.to(3221, 3218, 0, { allow_teleports = false })
    scenario.expect.action_sent { after = m }
    scenario.expect.tile { at = { x = 3221, z = 3218, level = 0 } }
    scenario.expect.item_count { item = "coins", where = "inventory", eq = 5 }
    scenario.finish { logout = true }
end

return { tick = tick }
