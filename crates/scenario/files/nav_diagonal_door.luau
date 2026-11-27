scenario_header {
    name = "nav_diagonal_door",
    revision = 289,
    cases = { "diagonal-door" },
    accounts = { main = { mainland = true, preamble = "tutorial" } },
    deadline_s = 360,
    idle_ticks = 600,
    view = { full_rate = true, nav_paint = true },
    visual = { profile = "functional", nav_paint = "product" },
    terminal_shot = "nav-diagonal-door-terminal",
    proof = { is = "ingame_scene_2" },
    limits = {
        "The fixture teleports into the East Ardougne room sealed by loc_1530 at (2669,3316).",
        "The product walks through the diagonal door to the south bank; no transport is allowed.",
    },
}

local function tick(api: any)
    scenario.phase("fixture")
    scenario.step("Enter the diagonal-door room", "Use a fixture teleport to begin inside the closed door.")
    scenario.seed.teleport { x = 2671, z = 3316, level = 0, why = "Start inside the sealed East Ardougne room." }

    scenario.phase("product")
    scenario.step("Walk through loc_1530 to the south bank", "Open the diagonal door and reach the bank on foot.")
    local route = scenario.mark("diagonal door route")
    api.walk.to(2655, 3286, 0, { allow_teleports = false })
    scenario.expect.action_sent { after = route }
    scenario.expect.tile { x = 2655, z = 3286, level = 0, within = 600 }
    scenario.finish { logout = true }
end

return { tick = tick }
