scenario_header {
    name = "file-two-roles-relog",
    revision = 289,
    cases = { "relog-and-signals" },
    accounts = {
        main = { mainland = true, preamble = "tutorial" },
        peer = { mainland = true, preamble = "tutorial" },
    },
    deadline_s = 300,
    idle_ticks = 200,
    seeds = "confirmed-only",
    proof = { is = "ingame_scene_2" },
    limits = {
        "Ground movement and a mid-run relog only; signals prove order, not shared game state.",
    },
}

local function walk_and_check(api, x, label)
    local m = scenario.mark(label)
    api.walk.to(x, 3218, 0, { allow_teleports = false })
    scenario.expect.action_sent { after = m }
    scenario.expect.tile { at = { x = x, z = 3218, level = 0 } }
end

local function tick(api)
    if scenario.role == "main" then
        scenario.step("Prepare main", "Own position and a carried-item witness.")
        scenario.seed.teleport { x = 3222, z = 3218, level = 0, why = "Courtyard start." }
        scenario.seed.give { item = "coins", count = 3, why = "Carried state that must survive the relog." }
        scenario.signal("main-ready")
        scenario.expect.signal { name = "peer-ready", within = 400 }

        scenario.phase("product")
        scenario.step("Relog mid-run", "The VM, dock and run identity survive a new session.")
        scenario.relog { why = "T15: a real logout and login inside one File run." }
        scenario.expect.item_count { item = "coins", where = "inventory", min = 3 }
        scenario.step("Walk after relog", "A normal root in the new session closes by its own arrival.")
        walk_and_check(api, 3221, "after-relog")
        scenario.signal("main-done")
    else
        scenario.step("Prepare peer", "Own position only.")
        scenario.seed.teleport { x = 3223, z = 3219, level = 0, why = "Courtyard start." }
        scenario.signal("peer-ready")
        scenario.expect.signal { name = "main-ready", within = 400 }

        scenario.phase("product")
        scenario.step("Peer walk", "The second role sends its own normal root.")
        local m = scenario.mark("peer-walk")
        api.walk.to(3224, 3219, 0, { allow_teleports = false })
        scenario.expect.action_sent { after = m }
        scenario.expect.tile { at = { x = 3224, z = 3219, level = 0 } }
        scenario.expect.signal { name = "main-done", within = 600 }
    end
    scenario.finish { logout = true }
end

return { tick = tick }
