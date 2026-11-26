scenario_header {
    name = "file-ground-walk",
    revision = 289,
    cases = { "ground-walk" },
    accounts = { main = { mainland = true, preamble = "tutorial" } },
    deadline_s = 180,
    idle_ticks = 120,
    seeds = "confirmed-only",
    proof = { is = "ingame_scene_2" },
    limits = { "Ground movement only; no door, transport or silent negative action." },
}

local function tick()
    scenario.step("Prepare the account", "Establish independent inventory, stat and position input.")
    scenario.seed.teleport { x = 3222, z = 3218, level = 0, why = "Start on the courtyard approach." }
    scenario.seed.give { item = "coins", count = 7, why = "Exercise the carried inventory source witness." }
    scenario.seed.set_stat { skill = "woodcutting", level = 12, why = "Exercise the exact same-skill stat source witness." }
    scenario.expect.stat_value { skill = "woodcutting", base = 12, effective = 12 }

    scenario.phase("product")
    scenario.step("Walk away from the seeded tile", "Use normal navigation instead of teleporting to the expected tile.")
    local outbound = scenario.mark("outbound")
    api.walk.to(3221, 3218, 0, { allow_teleports = false })
    scenario.expect.action_sent { after = outbound }
    scenario.expect.tile { at = { x = 3221, z = 3218, level = 0 } }

    scenario.step("Walk back past the marked tile", "Require an independent second root and a near-arrival witness.")
    local inbound = scenario.mark("inbound")
    api.walk.near(3223, 3218, 0, 1, { allow_teleports = false })
    scenario.expect.action_sent { after = inbound }
    scenario.expect.tile { at = { x = 3223, z = 3218, level = 0 }, radius = 1 }
    scenario.finish { logout = true }
end

return { tick = tick }
