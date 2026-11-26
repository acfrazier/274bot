scenario_header {
    name = "light-sources-r377",
    revision = 377,
    deadline_s = 600,
    idle_ticks = 200,
    cases = { "exterior-lifecycle" },
    accounts = { main = { mainland = true } },
    seeds = "confirmed-only",
    proof = { is = "ingame_scene_2" },
    limits = {
        "Own exterior position and items only; no manufacture or cave admission is proved.",
        "The selected canon content has no Firemaking level guard on light sources, so no silent level-guard root is exercised.",
    },
}

local FM, INV, WORN = "firemaking", "inventory", "worn"
local HOLE = { x = 3169, z = 3172, level = 0 }

local function count(item, where, n)
    scenario.expect.item_count { item = item, where = where, eq = n }
end

-- One action: mark, send through the normal api, require the native send,
-- name what must arrive, then require no XP and check the state after.
local function action(label, why, send, want, verify)
    scenario.step(label, why)
    local m = scenario.mark(label)
    send()
    scenario.expect.action_sent { after = m }
    want.after = m
    scenario.expect.received(want)
    scenario.expect.silent { after = m, ticks = 3, stats = { FM } }
    scenario.expect.xp_since { skill = FM, since = m, eq = 0 }
    verify(m)
end

local function tick(api)
    scenario.step("own-exterior-position", "Labelled exterior fixture; it does not prove a cave route.")
    scenario.seed.teleport { x = 3168, z = 3172, level = 0, why = "own exterior position" }
    for _, item in { "tinderbox", "oil_lamp_unlit", "cave_goblin_mining_helmet_unlit" } do
        scenario.seed.give { item = item, count = 1, why = "own item fixture" }
    end

    scenario.phase("product")
    action("oil-lamp-lit", "Real tinderbox use lights the filled lamp without experience",
        function() api.inventory.useOn("tinderbox", "oil_lamp_unlit") end,
        { inventories = { INV }, messages = { { type = 0, text = "You light the oil lamp." } } },
        function()
            count("oil_lamp_unlit", INV, 0); count("oil_lamp_lit", INV, 1); count("tinderbox", INV, 1)
        end)

    action("oil-lamp-extinguish-no-trigger", "Canon content has no Extinguish script: the engine emits its NoTrigger diagnostic only",
        function() api.inventory.option("oil_lamp_lit", 2) end,
        { messages = { { type = 0, text = "No trigger for [opheld2,oil_lamp_lit]" } } },
        function(m) scenario.expect.container_unchanged { where = INV, since = m } end)

    action("helmet-worn", "Option 2 wears the unlit helmet",
        function() api.inventory.option("cave_goblin_mining_helmet_unlit", 2) end,
        { inventories = { INV, WORN } },
        function()
            count("cave_goblin_mining_helmet_unlit", INV, 0); count("cave_goblin_mining_helmet_unlit", WORN, 1)
        end)

    action("helmet-removed", "Worn-items Remove restores the helmet to the backpack",
        function() api.equipment.remove("cave_goblin_mining_helmet_unlit") end,
        { inventories = { INV, WORN } },
        function()
            count("cave_goblin_mining_helmet_unlit", INV, 1); count("cave_goblin_mining_helmet_unlit", WORN, 0)
        end)

    scenario.expect.loc { at = HOLE, name = "Dark hole", first_option = "Climb-down" }
    action("natural-dark-hole-closed", "The natural exterior Climb-down has no content handler",
        function() api.loc.interact(HOLE, "Climb-down") end,
        { messages = { { type = 0, text = "No trigger for [oploc1,goblin_cave_entrance]" } } },
        function(m)
            scenario.expect.tile { unchanged_since = m }
            scenario.expect.container_unchanged { where = INV, since = m }
        end)

    scenario.step("clean-logout", "The case ends through the real logout button.")
    scenario.finish { logout = true }
end

return { tick = tick }
