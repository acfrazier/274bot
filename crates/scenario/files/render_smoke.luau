scenario_header {
    name = "render_smoke",
    revision = 289,
    cases = { "scene2" },
    accounts = { main = { mainland = false, preamble = "tutorial" } },
    deadline_s = 300,
    idle_ticks = 240,
    view = { renderer = true, capture = true },
    visual = { profile = "required" },
    terminal_shot = "scene2",
    proof = { is = "ingame_scene_2" },
    limits = { "Capture the rendered game at the logged-in tile after scene state 2." },
}

local function tick(api: any)
    scenario.phase("product")
    scenario.step("Wait for the rendered game scene", "Confirm scene state 2 at the login tile before capture.")
    scenario.expect.stat { id = 16, min = 0 }
    scenario.expect.ingame_scene_2 { within = 240 }
    scenario.step("Capture scene 2", "Request the presented game frame and wait for it to be written.")
    scenario.shot { label = "scene2" }
    scenario.finish { logout = true }
end

return { tick = tick }
