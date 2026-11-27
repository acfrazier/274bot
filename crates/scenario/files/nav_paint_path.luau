scenario_header {
    name = "nav_paint_path",
    revision = 289,
    cases = { "courtyard-paint" },
    accounts = { main = { mainland = true, preamble = "tutorial" } },
    deadline_s = 180,
    idle_ticks = 120,
    view = { full_rate = true, nav_paint = true },
    visual = { profile = "functional", nav_paint = "product" },
    terminal_shot = "nav-paint-path-terminal",
    proof = { is = "ingame_scene_2" },
    limits = { "Paint only the product walk path across the open Lumbridge courtyard." },
}

local function tick(api: any)
    scenario.phase("product")
    scenario.step("Walk south through the courtyard", "Keep the product-owned route visible in the headed capture.")
    local route = scenario.mark("painted route")
    api.walk.to(3220, 3212, 0, { allow_teleports = false })
    scenario.expect.action_sent { after = route }
    scenario.expect.tile { x = 3220, z = 3212, level = 0, within = 90 }
    scenario.finish { logout = true }
end

return { tick = tick }
