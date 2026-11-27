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
        "Seed five carried bones and twenty-eight banked bones, then verify a real bank withdrawal.",
        "The sealed subject buries the remaining twenty-seven bones through the ordinary API.",
    },
}

local CARRIED = 5
local BANKED = 28
local REMAINING = 27
local PRAYER = 5

local function wait_ticks(api: any, ticks: number)
    local start = api.status.tick()
    while api.status.tick() < start + ticks do
        api.wait()
    end
end

local function tick(api: any)
    scenario.phase("fixture")
    scenario.step("Stock carried and banked bones", "Give the product five carried bones and a separate bank stack.")
    scenario.seed.give { item = "bones", count = CARRIED, why = "Stock the initial burial run." }
    scenario.seed.give_bank { item = "bones", count = BANKED, why = "Stock the East Ardougne bank." }

    scenario.phase("product")
    scenario.step("Bury the five carried bones", "Prove the product consumes its carried stack and awards prayer XP.")
    local first_five = scenario.mark("first five burials")
    for remaining = CARRIED - 1, 0, -1 do
        local burial = scenario.mark("one carried burial")
        api.interact.heldByName("Bones", "Bury")
        scenario.expect.action_sent { after = burial }
        wait_ticks(api, 4)
        scenario.expect.item_at_most { name = "bones", count = remaining, within = 60 }
    end
    scenario.expect.stat_xp_gain { id = PRAYER, min = 22, since = first_five, within = 120 }

    scenario.phase("fixture")
    scenario.step("Move to the stocked East Ardougne bank", "Teleport as fixture setup to the contracted bank booth.")
    scenario.seed.teleport { x = 2655, z = 3286, level = 0, why = "Stand west of the seeded bank booth." }

    scenario.phase("product")
    scenario.step("Open the bank booth", "Use the named Use-quickly option of booth 2213.")
    local opened = scenario.mark("bank open")
    api.bank.openBooth(2656, 3286, 0, 2213, { name = "Bank booth", action = "Use-quickly" })
    scenario.expect.action_sent { after = opened }
    scenario.expect.bank_open { within = 60 }
    scenario.expect.bank_item { name = "bones", count = BANKED, within = 60 }

    scenario.step("Withdraw the banked bones", "Withdraw the complete seeded stack into the inventory.")
    local withdrawn = scenario.mark("bank withdrawal")
    api.bank.withdraw("Bones", "Withdraw-All")
    scenario.expect.action_sent { after = withdrawn }
    scenario.expect.item { name = "bones", count = BANKED, within = 60 }
    scenario.expect.bank_item_at_most { name = "bones", count = 0, within = 60 }

    scenario.step("Close the bank", "End the bank interaction before returning to the product subject.")
    local closed = scenario.mark("bank closed")
    api.bank.closeNow()
    scenario.expect.action_sent { after = closed }
    scenario.expect.bank_closed { within = 60 }

    scenario.step("Confirm a sixth burial", "One withdrawn bone proves the banked stock is usable.")
    local sixth = scenario.mark("sixth burial")
    api.interact.heldByName("Bones", "Bury")
    scenario.expect.action_sent { after = sixth }
    wait_ticks(api, 4)
    scenario.expect.stat_xp_gain { id = PRAYER, min = 27, since = first_five, within = 120 }
    scenario.expect.item { name = "bones", count = REMAINING, within = 60 }

    scenario.step("Watch the sealed subject", "Register passive checks before the subject buries the rest.")
    local remaining = scenario.mark("remaining burials")
    local buried = scenario.watch.stat_xp_gain {
        id = PRAYER,
        min = 121,
        since = remaining,
        within = 320,
        label = "remaining twenty-seven burials",
    }
    scenario.watch.item_at_most {
        name = "bones",
        count = 0,
        follows = buried,
        within = 60,
        label = "remaining bones gone",
    }
    scenario.seal { logout = false }

    for _ = 1, REMAINING do
        api.interact.heldByName("Bones", "Bury")
        wait_ticks(api, 4)
    end
    while true do
        api.wait()
    end
end

return { tick = tick }
