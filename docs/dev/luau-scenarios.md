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
evidence directory. The runner reports `Retiring` while each role's owned
Stop, any requested logout and the receipt write run; `PASS` comes only after
them. A Rust table run also reports `Retiring` while its planned Stop joins,
with or without a world-speed restore. Optional inputs:

| Variable | Meaning |
|---|---|
| `SCENARIO_ID` | A registry id implemented as a File scenario, instead of `SCENARIO_FILE`. |
| `SCENARIO_CASE` | Case name from the header `cases` list. |
| `SCENARIO_EXPECT_ENGINE`, `SCENARIO_EXPECT_CONTENT`, `SCENARIO_EXPECT_CRC` | Refuse the run before login when a head or the served checksum differs. |
| `SCENARIO_SEED_PACK`, `SCENARIO_SEED_PACK_GRANT` | A typed seed pack and, for a world-scope pack, the engine owner's grant. |
| `SCENARIO_EXPECT_OUTCOME` | `FAIL` for a negative lifecycle trace. Every role must still retire. |

Panel and TUI harness builds take the same inputs as flags: `--live-file`,
`--case`, `--evidence`, `--expect`, `--seed-pack` and `--seed-pack-grant`.
`--live script_<id>` selects a registered File scenario the same way; its
source is embedded from `crates/scenario/files/`. A build without
`scenario-luau` refuses such an id before login.
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

The full typed surface (every check, seed, header field and `scenario.await`)
is generated from the Rust registries into [`scenario.d.luau`](scenario.d.luau).
Check a file with `luau-analyze --definitions=docs/dev/scenario.d.luau <file>`.

## Observe a free-running subject

Declare `subject = "sealed-body"` in the header to keep the body running as
the product subject after `scenario.seal { logout = true }`. From then on the
body uses only ordinary `api` calls. Each action it sends opens a subject
root, which closes only from its contract's own fresh terminal record (or a
settled arrival). Watches admitted before the seal observe the subject; they
never close its roots. A watch with `follows = handle` starts, and its `within`
budget runs, only once that earlier check matched. After the seal the body
cannot `scenario.await` a watch: the runner retires each matched observer's
handle. A sealed body without a declared subject cannot act. The run fences
once every watch has matched, no root is open and the header proof holds.

## Images and painted paths

`visual = { profile = "required" }` refuses a headless caller before login.
`visual = { profile = "functional" }` lets a headless twin run: it records
each `shot` and the terminal shot as omitted, and a headed caller writes them.
An image without a `visual` profile still needs a headed caller.
`visual.nav_paint = "product"` turns on the painted path. Each walk root must
own the product walk's script route. A headed caller must also paint at
least one frame while the panel's published paint draws that route. Only a
frame the renderer paints after the publish counts; a publish alone does not.
The receipt records both: `nav_paint.route_owner` and
`nav_paint.painted_frames`. A headless functional
twin has no renderer, so its receipt shows `painted_frames = 0`. Only a GPU
head draws the nav paint: a headed run with `BOT_CPU=1` fails at the first
walk closure.

In a File run the panel sends no game input of its own for the role
accounts (no `getvar tutorial` probe and no click on its reply box). Any
queued cheat or wire command for a role account fails the run, and the
failure names the effect kind.

## What closes an action

Every normal action you send opens one root. The run cannot pass while a root
is open. A root closes only in one of these ways:

| Route | When |
|---|---|
| Terminal check | A check admitted after the action matches the family's own fresh record, for example the lit lamp in the backpack. The check must observe the family's own domain: its item ids in the family's container, its exact message, or its bank open or close. A true check of another domain does not close the root. A subject root uses its contract's own observer instead. |
| No-effect window | The family's source says nothing more can happen. Two ordered `getvar option_run` reads prove that the server processed the action. |
| Never sent | The normal owner sent nothing. |

A walk closes one PLAYER_INFO after the settled arrival, so the next cycle's
map-zone trigger has run. A bank close closes one PLAYER_INFO after the
closed bank, so its queued reorganize has run. A Bury also closes one
PLAYER_INFO after its message. Its prayer random event can spawn a zombie or
shade in the same cycle. If such an npc that was not in view at send appears,
the run fails instead of closing the root.

Action families without a source-backed contract for the selected engine and
content heads are refused. The supported families and their citations are in
`crates/host-play/src/scenario_luau/contracts.rs` (revision-377 light
sources, equip and the dark hole; revision-289 Bury, bank Use-quickly,
withdraw and close) and `plan.rs` (revision-289 walking in audited map
squares, plus the contracted East Ardougne door).

A fixture seed or a relog waits until the open root closes.

## Values and packets

`received`, `silent` and `count` default to `exact = "value"`. Value mode
compares successive samples: a change and its undo between two samples leave
no record. Use `exact = "packet"` when every event matters. A packet check is
refused when no exact provider is armed for that kind.

Inventory matchers select the carried (`inventory`) or `worn` container only;
a bank or component container is refused, because no provider observes it. A
packet inventory `delta` matches only when the journal saw the same slot
earlier in that session. A seed, `server_var` or relog releases the packet
provider, so `var` and packet deltas then need a new observation.

## Limits

- Revision-377 varbits without base and bit definitions in the selected packs
  are unavailable to seeds and reads.
- Headed captures (`shot`) need a headed caller unless the header declares a
  functional visual profile.
- `LiveSoak` and an approved quiescence cap are not available.
