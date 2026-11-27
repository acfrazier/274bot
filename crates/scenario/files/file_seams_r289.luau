scenario_header {
    name = "file-seams-r289",
    revision = 289,
    cases = { "walk-and-subject", "diagonal-door" },
    accounts = { main = { mainland = true, preamble = "tutorial" } },
    deadline_s = 360,
    idle_ticks = 240,
    subject = "sealed-body",
    visual = { profile = "functional", nav_paint = "product" },
    terminal_shot = "file-seams-r289 terminal",
    proof = { is = "ingame_scene_2" },
    limits = {
        "Ground walks inside the audited revision-289 map squares and the contracted East Ardougne door.",
        "The subject buries three seeded bones; banking is not exercised here.",
    },
}

-- Three normal bones; each Bury gives 4.5 prayer XP (bury_bone.rs2, bones.obj).
local BONES = 3
local PRAYER = 5

local function wait_ticks(api: any, ticks: number)
    local start = api.status.tick()
    while api.status.tick() < start + ticks do
        api.wait()
    end
end

local function diagonal_door(api: any)
    scenario.phase("fixture")
    scenario.step("Enter the diagonal-door room", "The room's only exit is the closed diagonal door loc_1530.")
    scenario.seed.teleport { x = 2671, z = 3316, level = 0, why = "Start inside the sealed room." }
    scenario.phase("product")
    scenario.step("Walk through the door to the bank", "The route opens loc_1530 and walks on.")
    local door = scenario.mark("door")
    api.walk.to(2655, 3286, 0, { allow_teleports = false })
    scenario.expect.action_sent { after = door }
    scenario.expect.tile { x = 2655, z = 3286, level = 0, within = 600 }
    scenario.finish { logout = true }
end

local function tick(api: any)
    if scenario.case == "diagonal-door" then
        diagonal_door(api)
    end
    scenario.phase("product")
    scenario.step("Walk north into the next map square", "The arrival crosses z=3264 and runs its mapzone trigger.")
    local north = scenario.mark("north")
    api.walk.to(3220, 3264, 0, { allow_teleports = false })
    scenario.expect.action_sent { after = north }
    scenario.expect.tile { x = 3220, z = 3264, level = 0, within = 240 }

    scenario.step("Walk south past the courtyard", "A second root ends outside the old courtyard box.")
    local south = scenario.mark("south")
    api.walk.to(3220, 3212, 0, { allow_teleports = false })
    scenario.expect.action_sent { after = south }
    scenario.expect.tile { x = 3220, z = 3212, level = 0, within = 240 }

    scenario.phase("fixture")
    scenario.step("Give the subject its bones", "The subject needs carried bones to bury.")
    scenario.seed.give { item = "bones", count = BONES, why = "Stock the free-running subject." }

    scenario.phase("product")
    scenario.step("Observe the free-running subject", "Sequential watches observe; they never drive the subject.")
    local before = scenario.mark("burials")
    local buried = scenario.watch.stat_xp_gain {
        id = PRAYER,
        min = 13,
        since = before,
        within = 400,
        label = "three burials",
    }
    scenario.watch.item_at_most { name = "bones", count = 0, follows = buried, within = 60, label = "bones gone" }
    scenario.seal { logout = true }

    -- From here the body is the product subject: ordinary api only.
    for _ = 1, BONES do
        api.interact.heldByName("Bones", "Bury")
        wait_ticks(api, 4)
    end
    while true do
        api.wait()
    end
end

return { tick = tick }
