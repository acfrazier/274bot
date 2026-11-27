scenario_header {
    name = "render_smoke",
    revision = 289,
    cases = { "scene2" },
    accounts = { main = { mainland = true, preamble = "tutorial" } },
    deadline_s = 300,
    idle_ticks = 240,
    view = { renderer = true, capture = true },
    visual = { profile = "required" },
    terminal_shot = "scene2",
    proof = { is = "ingame_scene_2" },
    limits = { "Capture the rendered game after it reaches scene state 2 at the mainland landing." },
}

local function tick(api: any)
    scenario.phase("product")
    scenario.step("Wait for the rendered game scene", "Confirm scene 2 at the mainland landing before capture.")
    scenario.expect.ingame_scene_2 { within = 240 }
    scenario.expect.tile { x = 3220, z = 3220, level = 0, within = 60 }
    scenario.step("Capture scene 2", "Request the presented game frame and wait for it to be written.")
    scenario.shot { label = "scene2" }
    scenario.finish { logout = true }
end

return { tick = tick }
