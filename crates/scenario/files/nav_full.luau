scenario_header {
    name = "nav_full",
    revision = 289,
    cases = { "whole-world" },
    accounts = { main = { mainland = true, preamble = "tutorial" } },
    deadline_s = 360,
    idle_ticks = 600,
    view = { full_rate = true, nav_paint = true },
    visual = { profile = "functional", nav_paint = "product" },
    terminal_shot = "nav-full-terminal",
    proof = { is = "ingame_scene_2" },
    limits = {
        "Walk north from the Lumbridge courtyard across the audited z=3264 map-square boundary.",
        "Ground navigation only; teleports are disabled for the product walk.",
    },
}

local function tick(api: any)
    scenario.phase("product")
    scenario.step("Walk north across the map-square boundary", "Reach the audited destination without a teleport.")
    local route = scenario.mark("whole-world route")
    api.walk.to(3220, 3264, 0, { allow_teleports = false })
    scenario.expect.action_sent { after = route }
    scenario.expect.tile { x = 3220, z = 3264, level = 0, within = 600 }
    scenario.finish { logout = true }
end

return { tick = tick }
