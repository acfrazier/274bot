# Luau File scenarios

> **Developer reference.** This page is for people who write or run test scenarios as Luau files.
> To use 274bot, start at the [README](../../README.md); to find another page, see the [docs index](../README.md).

A File scenario is one `.luau` file with a `scenario_header` and an ordinary
script body. Rust owns every check, seed, closure, retirement and the verdict.
The script can choose what to check; it cannot change how a check is computed.
File scenarios need a harness build with the `scenario-luau` feature. Product
builds cannot load them.

## Run a scenario file

Headless, through the e2e runner:

```sh
LIVE=1 BOT_REVISION=289 BOT_LIVE_NAME_PREFIX=me01 \
SCENARIO_FILE=tests/scenarios/file_ground_walk.luau SCENARIO_CASE=ground-walk \
SCENARIO_EVIDENCE=/path/to/evidence \
cargo test -p e2e --no-default-features --features scenario-luau \
  --test scenario_file -- --ignored --exact scenario_file --nocapture
```

The test prints `PASS` and writes `scenario-file-receipt.json` to the
evidence directory. Optional inputs:

| Variable | Meaning |
|---|---|
| `SCENARIO_CASE` | Case name from the header `cases` list. |
| `SCENARIO_EXPECT_ENGINE`, `SCENARIO_EXPECT_CONTENT`, `SCENARIO_EXPECT_CRC` | Refuse the run before login when a head or the served checksum differs. |
| `SCENARIO_SEED_PACK`, `SCENARIO_SEED_PACK_GRANT` | A typed seed pack and, for a world-scope pack, the engine owner's grant. |
| `SCENARIO_EXPECT_OUTCOME` | `FAIL` for a negative lifecycle trace. Every role must still retire. |

Panel and TUI harness builds take the same inputs as flags: `--live-file`,
`--case`, `--evidence`, `--expect`, `--seed-pack` and `--seed-pack-grant`.
A revision-377 file uses the `local-377` profile and its default ports.

## Write a scenario

```luau
scenario_header {
    name = "file-ground-walk",
    revision = 289,
    accounts = { main = { mainland = true, preamble = "tutorial" } },
    seeds = "confirmed-only",
}

local function tick(api)
    scenario.seed.teleport { x = 3222, z = 3218, level = 0, why = "Start tile." }
    scenario.phase("product")
    local m = scenario.mark("walk")
    api.walk.to(3221, 3218, 0, { allow_teleports = false })
    scenario.expect.action_sent { after = m }
    scenario.expect.tile { at = { x = 3221, z = 3218, level = 0 } }
    scenario.finish { logout = true }
end

return { tick = tick }
```

1. Seed only your own account, and give each seed a `why`.
2. Call `scenario.phase("product")` before the product actions. Only checks
   admitted in the product phase count as gameplay evidence.
3. Take a `mark` before each action. Check `action_sent` first, then what
   must arrive and what must stay silent.
4. End with `finish`. Add `logout = true` for a real terminal logout.

The full typed surface (every check, seed and field) is generated from the
Rust registry into [`scenario.d.luau`](scenario.d.luau).

## What closes an action

Every normal action you send opens one root. The run cannot pass while a root
is open. A root closes only in one of these ways:

| Route | When |
|---|---|
| Terminal check | A check admitted after the action matches the family's own fresh record, for example the lit lamp in the backpack. |
| No-effect window | The family's source says nothing more can happen. Two ordered `getvar option_run` reads prove that the server processed the action. |
| Never sent | The normal owner sent nothing. |

Action families without a source-backed contract for the selected engine and
content heads are refused. The supported families and their citations are in
`crates/host-play/src/scenario_luau/contracts.rs` (revision 377) and
`plan.rs` (revision-289 courtyard walking).

A fixture seed or a relog waits until the open root closes.

## Values and packets

`received`, `silent` and `count` default to `exact = "value"`. Value mode
compares successive samples: a change and its undo between two samples leave
no record. Use `exact = "packet"` when every event matters. A packet check is
refused when no exact provider is armed for that kind.

## Limits

- Revision-377 varbits without base and bit definitions in the selected packs
  are unavailable to seeds and reads.
- Headed captures (`shot`) need a headed caller.
- `LiveSoak` and an approved quiescence cap are not available.
