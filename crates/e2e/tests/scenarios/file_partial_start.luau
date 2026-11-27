-- T29 negative trace: role "broken" fails its Load while role "ready" is
-- either Ready or not yet started (login order is not controlled). The fleet
-- must fail, revoke or cancel "ready", and join every admitted execution.
-- The expected outcome of this file is FAIL with every role resolved.
scenario_header {
    name = "file-partial-start",
    revision = 289,
    cases = { "load-failure" },
    accounts = {
        ready = { mainland = true },
        broken = { mainland = true },
    },
    deadline_s = 180,
    proof = { is = "ingame_scene_2" },
    limits = { "Negative lifecycle trace; no gameplay claim." },
}

if scenario.role == "broken" then
    error("deliberate Load failure for the partial-Start trace")
end

local function tick(api)
    scenario.step("Wait for the fleet", "A Ready role keeps an open check while its peer fails.")
    scenario.expect.signal { name = "never-sent", within = 2000 }
    scenario.finish { logout = true }
end

return { tick = tick }
