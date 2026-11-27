scenario_header {
    name = "bone_burier_luau",
    revision = 289,
    cases = { "bank-and-bury" },
    accounts = { main = { mainland = true, preamble = "tutorial" } },
    deadline_s = 600,
    idle_ticks = 320,
    subject = "sealed-body",
    proof = { is = "ingame_scene_2" },
    limits = {
        "Seed five carried bones and twenty-eight unconfirmed bank bones.",
        "Walk to Draynor Bank, withdraw the stack, and bury all thirty-three bones.",
    },
}

local CARRIED = 5
local BANKED = 28
local REMAINING = BANKED - 1
local PRAYER = 5

local function wait_ticks(api: any, ticks: number)
    local start = api.status.tick()
    while api.status.tick() < start + ticks do
        api.wait()
    end
end

local function tick(api: any)
    scenario.phase("fixture")
    scenario.step("Seed carried and banked bones", "Give the subject five carried bones and a separate unconfirmed bank stack.")
    scenario.seed.give { item = "bones", count = CARRIED, why = "Stock the initial burial run." }
    scenario.seed.give_bank { item = "bones", count = BANKED, why = "Stock the bank for a real withdrawal." }
    wait_ticks(api, 4)

    scenario.phase("product")
    scenario.step("Bury and bank without blocking observers", "Follow the subject from its first burial through the seeded bank trip.")
    local first_five = scenario.mark("first five burials")
    local carried_xp = scenario.watch.stat_xp_gain {
        id = PRAYER,
        min = 22,
        since = first_five,
        within = 150,
        label = "prayer XP from the first five burials",
    }
    local carried_empty = scenario.watch.item_at_most {
        name = "bones",
        count = 0,
        follows = carried_xp,
        within = 150,
        label = "carried bones exhausted",
    }
    local bank_loaded = scenario.watch.bank_item {
        name = "bones",
        count = BANKED,
        follows = carried_empty,
        within = 320,
        label = "banked bones loaded",
    }
    local inventory_loaded = scenario.watch.item {
        name = "bones",
        count = BANKED,
        follows = bank_loaded,
        within = 150,
        label = "banked bones withdrawn",
    }
    local bank_empty = scenario.watch.bank_item_at_most {
        name = "bones",
        count = 0,
        follows = inventory_loaded,
        within = 150,
        label = "banked bones exhausted",
    }
    local bank_closed = scenario.watch.bank_closed {
        follows = bank_empty,
        within = 150,
        label = "bank closed",
    }
    scenario.watch.stat_xp_gain {
        id = PRAYER,
        min = 27,
        since = first_five,
        follows = bank_closed,
        within = 150,
        label = "prayer XP after the first banked burial",
    }
    scenario.seal { logout = false }

    for _ = 1, CARRIED do
        api.interact.heldByName("Bones", "Bury")
        wait_ticks(api, 4)
    end

    -- The nearest-bank route uses the open F2P road and avoids the Al-Kharid toll.
    api.walk.nearestBank()
    while true do
        local x, z, level = api.status.tile()
        if x ~= nil and z ~= nil and level == 0
            and math.max(math.abs(x - 3092), math.abs(z - 3243)) <= 4 then
            break
        end
        api.wait()
    end
    api.bank.openNearest()
    wait_ticks(api, 4)
    api.bank.withdraw("Bones", "Withdraw-All")
    wait_ticks(api, 4)
    api.bank.closeNow()
    wait_ticks(api, 4)

    api.interact.heldByName("Bones", "Bury")
    wait_ticks(api, 4)
    for _ = 1, REMAINING do
        api.interact.heldByName("Bones", "Bury")
        wait_ticks(api, 4)
    end
    while true do
        api.wait()
    end
end

return { tick = tick }
