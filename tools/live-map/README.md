# Live path map

This tool lists the live cells that cover changed product paths. A live path changes bot actions in the game or changes what the operator sees. See `AGENTS.md:142` for the full rule.

Run the tool from the repository root:

```sh
python3 -B tools/live-map/live_map.py --diff BASE..HEAD
```

The tool reads paths with `git diff --name-only` and expands changes to the
`vendor/fr-client-rust` gitlink into paths changed between its old and new
commits. If either commit is missing locally, the tool prints a fetch command
and exits with an error. It lists covering cells and their exact commands.
Run each listed cell three times on the changed code. The tool also lists
uncovered live paths and unclassified paths. Exit code 0 means no gaps. Exit
code 1 means a live path has no cell or a path is unclassified. Exit code 2
means the input or Git diff is invalid.
Do not use a broad live glob to hide an uncovered path.

Run the self-tests with temporary Git repositories. The real-submodule test
stores its temporary repository under the evidence folder. Set the required
`LIVE_MAP_TEST_TMPDIR` variable to a directory under that folder. Do not use
`/tmp`.

```sh
LIVE_MAP_TEST_TMPDIR=/path/to/274bot-evidence/request-id/fix-r4 \
  python3 -B tools/live-map/live_map.py --self-test
```

## Validate test selectors

Before using a map, list the tests selected by every unique cell command:

```sh
ISOHOME=/Users/acfrazier/experiments/274bot/.superpowers/bin/isohome \
  python3 -B tools/live-map/live_map.py --verify-cells
```

The verifier extracts each test selector and runs `cargo test ... -- --list`.
It fails if the selector exits with an error or lists no tests. For a
`cargo run` cell, the third field is only a proxy selector. The verifier does
not validate the `cargo run` command, the `--live` name, the input values, or
the assigned host. It does not run the live cell.

## Cell hosts

The map format has no host field. This table assigns each live cell to a host.
All cells not listed below normally run on merlot. Engine A uses game port
`44594` and HTTP port `1080` on that host. Do not start or stop Engine A.

| Cell | Host and endpoint |
|---|---|
| `host.npc-hop-engine-b` | Run on chardonnay. The command sets `WORLD_CACHE_DIR` and `LIVE_EVIDENCE_DIR`. Engine B is reached through the loopback tunnel at game port `45594` and HTTP port `2080`. |
| `host.sherlock-search-live` | Run on merlot or chardonnay when that host has Engine A at `127.0.0.1:44594` and `127.0.0.1:1080`. The cell uses loopback ports and has no host-specific code. Use chardonnay when merlot is measured or reserved. It needs `LIVE=1`, `BOT_CPU=1`, `BOT_LIVE_NAME_PREFIX`, `WORLD_NAV_PACK`, `WORLD_ENGINE_DIR`, `BOT_CACHE_DIR`, `RS2B0T`, `WORLD_GAME_PORT`, `WORLD_HTTP_PORT`, and `LIVE_EVIDENCE_DIR`. The command also uses `ISOHOME`. Leave `QUESTER_TICK_MS` unset. |
| `host.quester-path-engine-q`, `scenario.quester-engine-q` | Run on the Engine Q host. This setup uses merlot at `127.0.0.1:44694` and `127.0.0.1:1180`. Set `QUESTER_TICK_MS=300` only for these cells. |
| `nav.essence-follow` | The orchestrator must start a 274 engine on the cell host at `127.0.0.1:43594`. The orchestrator supplies `ENGINE_274_DIR` and `NAV_PACK_274`. Do not run this cell until the orchestrator provides both inputs and starts the engine. |
| `nav.essence-289-engine-a` | Run on merlot or chardonnay when that host has Engine A at `127.0.0.1:44594` and `127.0.0.1:1080`. Use chardonnay when merlot is measured or reserved. |
| `panel.headed-render` | Run on merlot with its interactive GPU and display. The command removes `BOT_CPU` so the client uses the GPU renderer. |


Client submodule paths have separate coverage. The gitlink and the client
workspace and package manifests select the GPU headed cell, including for a
test-only client bump. `client/client.rs`,
`client/actor_289.rs`, `render/draw.rs`, and `render/backend/cpu.rs` select
both headed cells. `client/client.rs` also selects both relog cells because it
contains `Client::login` (`vendor/fr-client-rust/crates/client/src/client/client.rs:3445`).
Changes under `client/src/io/**` select both relog cells. The CPU headed cell
sets `BOT_CPU=1`; the GPU cell removes it.

The map lists test-only paths in `not_live.globs`. A path in
`not_live.exceptions` stays live when it also matches a live glob. The
ScriptWalkArm admission test is the only current exception.
Sherlock requires the inputs listed at `crates/host-play/tests/common/quester_live.rs:28-32,332-359`.
Its profile reads `WORLD_GAME_PORT` and `WORLD_HTTP_PORT` at
`crates/host-play/tests/common/quester_live.rs:358-359`.
The NPC-hop test requires `WORLD_NAV_PACK` and reads its optional `WORLD_NAV_FLAGS`,
`WORLD_ENGINE_DIR`, and `WORLD_CACHE_DIR` at `crates/host-play/tests/npc_hop_live.rs:87-115`.
It checks `LIVE=1` and writes receipts when `LIVE_EVIDENCE_DIR` is set at
`crates/host-play/tests/npc_hop_live.rs:53-58,308-323`.

## ScriptWalkArm call paths

The `script_nav.rs` and `script_walk.rs` row selects five live cells: four scenario cells and the compat-v1 crossing cell.
A separate row maps `survivable_admission_live.rs` to both admission tests.

| Cell | Call path |
|---|---|
| `host.api-gather-wire-shadow` | The live source starts Gatherer at `crates/host-play/src/api_gather_live_tests.rs:13-29,490-539`.<br>The cell requests `disposition: 'Power'` (`api_gather_live_tests.rs:25`). Its walk is the resource approach at `crates/script/src/gatherer/runner.rs:1321-1327`.<br>`crates/host-play/src/script_observe.rs:1546-1572` creates `ScriptWalkArm` and calls `queue_native_route` at `crates/host-play/src/script_nav.rs:4567-4573`. |
| `host.gatherer-bank-cost-fallback` | The cell runs the live test at `crates/host-play/tests/gatherer_live.rs:7945-7949`.<br>`gatherer::runner::trip_walk` submits `Walk` at `crates/script/src/gatherer/runner.rs:445-472`.<br>`crates/host-play/src/script_observe.rs:1546-1572` creates `ScriptWalkArm` and calls `queue_native_route` at `crates/host-play/src/script_nav.rs:4567-4573`. |
| `host.grinch-cross-region-return` | The live selector is at `crates/host-play/tests/grinch_live.rs:119-123`.<br>Return recovery submits `Walk` at `crates/script/src/grinch.rs:885-903`.<br>`crates/host-play/src/script_observe.rs:1546-1572` creates `ScriptWalkArm` and calls `queue_native_route` at `crates/host-play/src/script_nav.rs:4567-4573`. |
| `host.sherlock-search-live` | The live cell starts compiled Sherlock at `crates/host-play/tests/clue_walk_arrive_live.rs:193-212`.<br>The walk-near step is at `crates/script/src/clue/isolate.rs:1640-1653`.<br>`crates/host-play/src/script_interact.rs:529-585` creates `ScriptWalkArm` and calls `queue_route_in_snapshot_avoiding` at `crates/host-play/src/script_nav.rs:4932-4957`. |
| `host.survivable-admission` | The live test checks refusal and opt-in at `crates/host-play/tests/survivable_admission_live.rs:283-286`. |
| `host.survivable-compat-v1-crossing` | The live test loads a real compat-v1 class whose `Traversal.walkTo` call is at `crates/host-play/tests/survivable_admission_live.rs:837-870,943-950`. It asserts the live admission snapshot and crossing assessment at `:963-983`; the selector is `real_compat_v1_crossing_and_own_sustain_at_45_and_14_hp` (`:868-870`). |

The four scenario cells advance routes through the live slot loop at
`crates/host-play/src/play_slots.rs:1909-1929` and
`crates/host-play/src/script_walk.rs:867-885`.

## Essence router call paths

The essence files and the shared `navigation.rs` file select the 289
`nav.essence-289-engine-a` cell and the separate 274 `nav.essence-follow`
cell. The 289 cell runs `script_nav_essence` on revision 289 and Engine A. The
registered `nav_essence` scenario enters the mine through Aubury, latches an
essence session, and follows out through the exit portal
(`crates/scenario/src/catalog.rs:81`, `crates/scenario/src/scenarios/navigation.rs:616-714`).
It passed once on chardonnay Engine A. The receipt is
`LIVE-MAP-1023/fix-r4/nav-essence-engine-a.log`.

The e2e `nav.essence-follow` cell still needs a local 274 engine and a rebaked
274 navigation pack (`crates/e2e/tests/nav_essence.rs:233-255,271-298`). It
does not replace or gate the 289 cell.

| Cell | Router call path |
|---|---|
| `nav.essence-289-engine-a` | `tui-play --live script_nav_essence` runs the registered scenario. It follows Aubury's wizard entry into the mine, sets an `EssenceSession`, then routes back through the exit portal (`crates/scenario/src/catalog.rs:81`, `crates/scenario/src/scenarios/navigation.rs:616-714`). |
| `nav.essence-follow` | The e2e test runs the same scenario in `nav_essence_follow` and needs a local 274 engine (`crates/e2e/tests/nav_essence.rs:233-255,271-298`). |

## Input values

These are the 19 names reviewed in round 2. Three inputs are fixed in the map.
The caller must not replace the gather tile, Path, or seeds file.

| Name | Value or owner |
|---|---|
| `ISOHOME` | Use the absolute wrapper path on the host. Merlot: `/Users/acfrazier/experiments/274bot/.superpowers/bin/isohome`. Chardonnay: `/home/acfrazier/experiments/274bot/.superpowers/bin/isohome`. |
| `BOT_LIVE_NAME_PREFIX` | Choose a unique prefix of 1 to 4 characters for each run. The Sherlock receipt in this round uses `l2r`. See `STANDING-RULINGS.md:33` and `crates/host-play/src/play_bootstrap.rs:67-75`. |
| `LIVE_EVIDENCE_DIR` | Use a fresh directory outside the throwaway HOME. Merlot: `/Volumes/dev-scratch/274bot-evidence/LIVE-MAP-1023/fix-r2/live/<cell>`. Chardonnay: `/srv/dev/scratch/274bot-evidence/LIVE-MAP-1023/fix-r2/live/<cell>`. |
| `ENGINE_A_DIR` | Merlot: `/Users/acfrazier/experiments/lostcity-289/engine`. Chardonnay: `/home/acfrazier/experiments/lostcity-289/engine`. Keep the directory read-only. |
| `ENGINE_B_DIR` | For the chardonnay tunnel cell, use the local R289 client tree at `/home/acfrazier/experiments/lostcity-289/engine`. This optional client path is not the Engine B service. The service is the tunnel at `45594/2080`. |
| `ENGINE_Q_DIR` | Use the Engine Q tree on its host. Merlot: `/Users/acfrazier/experiments/lostcity-289-q/engine`. Chardonnay: `/home/acfrazier/experiments/lostcity-289-q/engine`. |
| `ENGINE_274_DIR` | The orchestrator supplies the 274 engine directory when it starts the 274 service. This value is not set in this map round. |
| `NAV_PACK_289` | Use the CI reference v17 pack. Merlot: `/Volumes/dev-scratch/274bot-evidence/NAV-GATED-ENTRANCES/reference-v17/274bot-289-v17.navpack`. Chardonnay: `/srv/dev/scratch/274bot-evidence/NAV-GATED-ENTRANCES/reference-v17/274bot-289-v17.navpack`. See `STANDING-RULINGS.md:16`. |
| `NAV_FLAGS_289` | Use the matching `274bot-289-v17.navflags` in the same reference-v17 directory as `NAV_PACK_289`. |
| `NAV_SNAPSHOT_ROOT_289` | Use the read-only decoded snapshot that `isohome` exposes as `BOT_NAV_SNAPSHOT_ROOT`. The default paths are `/Users/acfrazier/.274bot/unpack-289` on merlot and `/home/acfrazier/.274bot/unpack-289` on chardonnay. See `AGENTS.md:149`. |
| `BOT_CACHE_DIR` | Make a fresh copy of the selected engine's `data/pack/client` at `<LIVE_EVIDENCE_DIR>/cache-289`. The Quester helper copies this source into its isolated run cache. |
| `CATALOG_ROOT` | Use the local `rs2b0t` checkout root. Merlot: `/Users/acfrazier/experiments/rs2b0t`. Chardonnay: `/home/acfrazier/experiments/rs2b0t`. |
| `CLIENT_UNPACK_DIR` | Use a fresh writable directory at `<LIVE_EVIDENCE_DIR>/client-unpack`. Do not use the operator's real HOME. |
| `PANEL_LIVE_HOME` | Use a fresh throwaway HOME. Set it below `ISOHOME_ROOT`: `/Volumes/dev-scratch/isohome/panel-<prefix>` on merlot or `/srv/dev/scratch/isohome/panel-<prefix>` on chardonnay. |
| `GATHERER_WC_TILE` | The map fixes `3197,3209,0`, backed by the R289 normal-tree placement `maps/m49_50.jm2:5342` (`crates/api/data/game-data/289/gathering.json:17173-17190,38398`). It is outside LocationAuto's `AUTO_NEXT_TILES` (`crates/host-play/tests/gatherer_live.rs:107`) and stump fixture (`:197-243,5791-5812`). The caller does not choose it. |
| `QUESTER_PATH` | The map fixes `cook`, which selects the committed Cook Path. The map owner selected this path. The caller does not choose it. |
| `QUESTER_SEEDS` | The map points to `${PWD}/crates/host-play/tests/fixtures/quester-path/cook.json`. This file copies the `COOK_SEEDS` value at `quester_path_live.rs:186-195`. Run the command from the repository root. Do not use a caller-written seeds file. |
| `NAV_PACK_274` | Use `274bot-274-v17.navpack` in the same reference-v17 directory as `NAV_PACK_289`. The orchestrator supplies this path to `nav.essence-follow` when it starts the 274 service. |
| `BOT_UNPACK_DIR` | Use a fresh writable unpack directory at `<LIVE_EVIDENCE_DIR>/client-unpack`. This can be the same path as `CLIENT_UNPACK_DIR`. |

Run commands from the repository root. Keep writable cache and unpack paths in
fresh run directories. `AGENTS.md:149` requires a throwaway HOME for product
runs and `isohome` exposes the read-only navigation snapshot.

## Add a cell

1. Write and run the live cell against real game content. Do not seed the target resource or entity.
2. Add one `[[cover]]` row for the narrowest product paths the cell exercises.
3. Add each cell as `name|command` in that row's `cells` array. Keep the command exact.
4. Add test-only files to `[not_live]`, not `[live]`.
5. Run the tool on the change. Confirm that it lists the new cell and no changed live path remains uncovered.

An explicit exception in `not_live.exceptions` keeps a path live only when it
also matches a live glob. Add the cover row for that path.

The scenario registry and nightly cell lists are separate from changed-path matching. Inspect them with:

```sh
python3 -B tools/nightly/run_nightly.py --list
```
