scenario_header {
    name = "walk",
    revision = 289,
    cases = { "courtyard" },
    accounts = { main = { mainland = true, preamble = "tutorial" } },
    deadline_s = 180,
    idle_ticks = 180,
    proof = { is = "ingame_scene_2" },
    limits = { "Ground walks across the open Lumbridge courtyard only; no transport." },
}

local function tick(api: any)
    scenario.phase("product")
    scenario.step("Walk to the courtyard midpoint", "Use a first normal walk inside the open courtyard.")
    local midpoint = scenario.mark("midpoint")
    api.walk.to(3220, 3216, 0, { allow_teleports = false })
    scenario.expect.action_sent { after = midpoint }
    scenario.expect.tile { x = 3220, z = 3216, level = 0, within = 90 }

    scenario.step("Walk to the courtyard south", "Prove a second independent walk reaches the requested tile.")
    local south = scenario.mark("south")
    api.walk.to(3220, 3212, 0, { allow_teleports = false })
    scenario.expect.action_sent { after = south }
    scenario.expect.tile { x = 3220, z = 3212, level = 0, within = 90 }
    scenario.finish { logout = true }
end

return { tick = tick }
