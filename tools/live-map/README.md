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

## Clean logout

A gameplay pass does not excuse a failed shutdown. Live terminal runs and the
Quester path helper stop scripts and wait for Stop acknowledgement before
requesting logout. If an account stays connected, another logout request waits
for 16 more observed game ticks. R289 content sets this combat lockout in
`content/scripts/login_logout/logout.rs2:26-30`. The wait does not count render
frames or wall-clock seconds. A fixed 30-second deadline bounds the whole
shutdown, including Stop. A forced worker close is not a clean logout.


## Cell hosts

The map format has no host field. This table assigns each live cell to a host.
All cells not listed below normally run on merlot. Engine A uses game port
`44594` and HTTP port `1080` on that host. Do not start or stop Engine A.

| Cell | Host and endpoint |
|---|---|
| `host.npc-hop-engine-b` | Run on chardonnay. The command sets `WORLD_CACHE_DIR` and `LIVE_EVIDENCE_DIR`. Engine B is reached through the loopback tunnel at game port `45594` and HTTP port `2080`. |
| `host.sherlock-search-live` | Run on merlot or chardonnay when that host has Engine A at `127.0.0.1:44594` and `127.0.0.1:1080`. The cell uses loopback ports and has no host-specific code. Use chardonnay when merlot is measured or reserved. It needs `LIVE=1`, `BOT_CPU=1`, `BOT_LIVE_NAME_PREFIX`, `WORLD_NAV_PACK`, `WORLD_ENGINE_DIR`, `BOT_CACHE_DIR`, `RS2B0T`, `WORLD_GAME_PORT`, `WORLD_HTTP_PORT`, and `LIVE_EVIDENCE_DIR`. The command also uses `ISOHOME`. Leave `QUESTER_TICK_MS` unset. |
| `host.quester-path-engine-q`, `host.quester-sheep-spin-engine-q`, `host.quester-prince-smelt-wig-spin-engine-q`, `host.quester-qualified-priest-engine-q`, `host.quester-jonny-stand-engine-q`, `host.quester-mordred-continuation-engine-q`, `host.quester-digsite-thieve-engine-q`, `scenario.quester-engine-q` | Run on the Engine Q host at game port `44694` and HTTP port `1180`. Engine Q uses fixed 300 ms ticks. Leave `QUESTER_TICK_MS` unset. Do not send `::speed`. |
| `host.quester-bank-teller-engine-q`, `host.quester-bank-player-open-engine-q`, `host.quester-bank-chest-engine-q` | Use the Engine Q endpoint and tick rules in the row above. |
| `script.release-path-refusal` | Run on a build-enabled lab host with an isolated HOME and `LIVE_EVIDENCE_DIR`. It builds production release libraries but opens no game session. |
| `host.walk-no-route-live` | Run on aubaine against the 600 ms Engine L at `127.0.0.1:47594` and `127.0.0.1:3080`, using its assigned lab scope and the per-cell I/O admission gate. The release command maps the read-only Engine L tree from `BOT_ENGINE_DIR` and uses the `NAV_PACK_289`, `NAV_FLAGS_289`, `BOT_CACHE_DIR`, and `CATALOG_ROOT` inputs for the local R289 client profile. It writes the test receipts under `LIVE_EVIDENCE_DIR`. |
| `nav.essence-follow` | The orchestrator must start a 274 engine on the cell host at `127.0.0.1:43594`. The orchestrator supplies `ENGINE_274_DIR` and `NAV_PACK_274`. Do not run this cell until the orchestrator provides both inputs and starts the engine. |
| `nav.essence-289-engine-a` | Run on merlot or chardonnay when that host has Engine A at `127.0.0.1:44594` and `127.0.0.1:1080`. Use chardonnay when merlot is measured or reserved. |
| `host.nav-zanaris-entrana` | Run on aubaine with Engine L at `127.0.0.1:47594` and `127.0.0.1:3080`. Set `ENGINE_A_DIR` to the Engine L directory. Also set `BOT_CACHE_DIR`, `NAV_PACK_289`, `LIVE_EVIDENCE_DIR`, `CATALOG_ROOT`, and `QUESTER_PATH_DIR`. Use the composed head's pack and its merged Zanaris Path source. This cell proves entry to the Entrana dungeon, not completion of Lost City. |
| `host.sherlock-entrana-walk-arrive`, `host.sherlock-entrana-content-categories` | Run on aubaine through Engine L's relay at `127.0.0.1:44594` and `127.0.0.1:1080`. Set the standard Engine A, pack, cache, catalog, and evidence variables. |
| `panel.headed-render` | Run on merlot with its interactive GPU and display. The command removes `BOT_CPU` so the client uses the GPU renderer. |

**Focused Quester content**

These cells use `live_quester_path_smoke` with committed seeds under
`crates/host-play/tests/fixtures/quester-path/`. Success requires an observed
transition beyond the seeded stage, or actual quest completion.

| Cell | Seed and required result |
|---|---|
| `host.quester-qualified-priest-engine-q` | `priest-start.json`: Father Aereck advances the qualified Path from `priest:0` to `priest:1`. |
| `host.quester-jonny-stand-engine-q` | `blackarm-jonny.json`: one explicit pre-Start `phoenixgang=8` seed selects the journal-only `p:8` stage (`quest_blackarmgang.constant:11`; journal:33-46). The Phoenix role walks from the cabbage field, fights real Jonny, and hands in his report to reach `p:9`. The report and joined state are not seeded. |
| `host.quester-mordred-continuation-engine-q` | `arthur-mordred.json`: Base40 fights real Mordred and continues Morgan's dialogue to reach `arthur:4`. Neither success state is seeded. |
| `host.quester-digsite-thieve-engine-q` | `itexam-thieve.json`: Thieving25 approaches the workman from the recorded route start and advances from the first exam to the second. Only the unrelated orange and purple samples are seeded. |

The three drafted Paths require the command's explicit test-support feature.
These focused receipts do not qualify a complete draft guide for release.

The bank ownership cells use `quester_bank_ownership_live` and the same explicit
test-support features. The Shilo teller case completes only the account's bank
access prerequisite before Start. It requires an automatic Quester close after
the withdrawal. The Gundai case requires a player-open bank to remain open in
the same generation. The Duel Arena case requires the naturally closed lid to
open before the actual bank opens. It refuses an already-open shared chest
without resetting world content.

**Draft release refusal**

The exact draft files in the `script.release-path-refusal` row do not ship as
runnable guides. This cell proves refusal only. It does not execute their quest
bodies or qualify them for release. Run its paired qualified-Priest live cell as
well.

The cell first builds `script` with `cargo build --release --locked --lib`.
It selects the exact rlib and one rmeta from that invocation's Cargo artifact
record. The rmeta's parent supplies the dependency search directory: Cargo can
publish the rlib in `target/release` and its rmeta in `target/release/deps`.
Missing or ambiguous metadata is refused, with no directory search or fallback.
The test-artifact, debug-assertion, `test-hooks` and ambiguous-rlib checks remain.
The external Rust consumer calls the public production queue entry point for
each mapped draft, with both an explicit pick and an explicit priority. It
requires the typed `UnqualifiedPath` reason and its complete display text, even
when saved settings request draft inclusion. It also checks that Priest remains
selectable. This uses the external-library proof pattern in
`tools/architecture/nav_clippy.py`; no probe crate or product API is added.

The second part runs the real shared picker model against every bundled draft.
It requires a non-selectable saved row with the full refusal reason in both the
quest and priority pickers. These labels are the panel/TUI presentation source.
The production-library part is separate because the script and frontend test
targets deliberately enable test hooks through their self dev-dependencies.
Running a release-mode unit test alone would not prove release admission.

Each draft's own run stays with the quest campaign. Before changing its status
to qualified, that campaign must qualify it. The same diff must remove the exact
file from refusal coverage and add its real quest cell. The production probe
fails if a qualified row remains in the refusal map. A Priest pass does not
substitute for that quest's live qualification.

`python3 -B tools/live-map/release_refusal.py --self-test` checks exact-path and
artifact selection. `--verify-cells` lists only this cell's picker-test selector;
it does not run or validate the external production-library proof.



Client submodule paths have separate coverage. The gitlink and the client
workspace and package manifests select the GPU headed cell, including for a
test-only client bump. `client/client.rs`,
`client/actor_289.rs`, `render/draw.rs`, and `render/backend/cpu.rs` select
both headed cells. `client/client.rs` also selects both relog cells because it
contains `Client::login` (`vendor/fr-client-rust/crates/client/src/client/client.rs:3445`).
Changes under `client/src/io/**` select both relog cells. The CPU headed cell
sets `BOT_CPU=1`; the GPU cell removes it.

The map lists test-only paths in `not_live.globs`. A path in
`not_live.exceptions` stays live when it also matches a live glob.
The current exceptions are the ScriptWalkArm admission test and the Engine L
no-route live test.
Sherlock requires the inputs listed at `crates/host-play/tests/common/quester_live.rs:28-32,332-359`.
Its profile reads `WORLD_GAME_PORT` and `WORLD_HTTP_PORT` at
`crates/host-play/tests/common/quester_live.rs:358-359`.
The NPC-hop test requires `WORLD_NAV_PACK` and reads its optional `WORLD_NAV_FLAGS`,
`WORLD_ENGINE_DIR`, and `WORLD_CACHE_DIR` at `crates/host-play/tests/npc_hop_live.rs:87-115`.
It checks `LIVE=1` and writes receipts when `LIVE_EVIDENCE_DIR` is set at
`crates/host-play/tests/npc_hop_live.rs:53-58,308-323`.

## ScriptWalkArm call paths

Six existing live cells select `script_nav.rs` and `script_walk.rs`: the five
cells in the row below plus `panel.state-gap-stop-live` from the shared command
row. The added `host.walk-no-route-live` cell also covers `script_nav.rs` and
`crates/script/src/native/walk.rs`.
A separate row maps `survivable_admission_live.rs` to both admission tests.

| Cell | Call path |
|---|---|
| `host.api-gather-wire-shadow` | The live source starts Gatherer at `crates/host-play/src/api_gather_live_tests.rs:13-29,490-539`.<br>The cell requests `disposition: 'Power'` (`api_gather_live_tests.rs:25`). Its walk is the resource approach at `crates/script/src/gatherer/runner.rs:1321-1327`.<br>`crates/host-play/src/script_observe.rs:1546-1572` creates `ScriptWalkArm` and calls `queue_native_route` at `crates/host-play/src/script_nav.rs:4567-4573`. |
| `host.gatherer-bank-cost-fallback` | The cell runs the live test at `crates/host-play/tests/gatherer_live.rs:7945-7949`.<br>`gatherer::runner::trip_walk` submits `Walk` at `crates/script/src/gatherer/runner.rs:445-472`.<br>`crates/host-play/src/script_observe.rs:1546-1572` creates `ScriptWalkArm` and calls `queue_native_route` at `crates/host-play/src/script_nav.rs:4567-4573`. |
| `host.grinch-cross-region-return` | The live selector is at `crates/host-play/tests/grinch_live.rs:119-123`.<br>Return recovery submits `Walk` at `crates/script/src/grinch.rs:885-903`.<br>`crates/host-play/src/script_observe.rs:1546-1572` creates `ScriptWalkArm` and calls `queue_native_route` at `crates/host-play/src/script_nav.rs:4567-4573`. |
| `host.sherlock-search-live` | The live cell starts compiled Sherlock at `crates/host-play/tests/clue_walk_arrive_live.rs:193-212`.<br>The walk-near step is at `crates/script-js/src/compat/clue/isolate.rs:1640-1653`.<br>`crates/host-play/src/script_interact.rs:529-585` creates `ScriptWalkArm` and calls `queue_route_in_snapshot_avoiding` at `crates/host-play/src/script_nav.rs:4932-4957`. |
| `host.walk-no-route-live` | The ignored Engine L test starts a fresh account without a Dusty key and begins a native `Walk` request to the radius-5 Taverley gate cell (`crates/host-play/tests/walk_no_route_live.rs:289-322,406-421`). It records the origin, request/terminal game ticks, typed result and Quester arrival conversion, then asserts `WalkEnd::NoPath` retains the diagnosed gate or explicit diagnostic-budget reason and becomes `Blocked(detail)` (`:437-497`). |
| `host.survivable-admission` | The live test checks refusal and opt-in at `crates/host-play/tests/survivable_admission_live.rs:283-286`. |
| `host.survivable-compat-v1-crossing` | The live test loads a real compat-v1 class whose `Traversal.walkTo` call is at `crates/host-play/tests/survivable_admission_live.rs:837-870,943-950`. It asserts the live admission snapshot and crossing assessment at `:963-983`; the selector is `real_compat_v1_crossing_and_own_sustain_at_45_and_14_hp` (`:868-870`). |

The four scenario cells advance routes through the live slot loop at
`crates/host-play/src/play_slots.rs:1909-1929` and
`crates/host-play/src/script_walk.rs:867-885`.

## Death recovery

`host.gatherer-death-return` runs the `gatherer_death_return` test in
`crates/host-play/tests/gatherer_live.rs`. It starts the compiled Gatherer on
an authored oak at `3017,3170,0`, then sends the engine's `~death` debugproc
(`content/scripts/_test/scripts/cheats/cheat_other.rs2:25-26`, a real
`damage_self(999)`). Each death must recover through respawn, supply, worn
tool, return and fresh product and XP; the cell kills twice with a haul
between and pauses over the first death. The fixture seeds only the account's
own bronze axe in the bank and its inventory fillers, and never injects a
product or XP. Inputs: `LIVE=1`, `BOT_LIVE_NAME_PREFIX`, `GATHERER_NAV_PACK`,
`GATHERER_NAV_FLAGS`, `GATHERER_ENGINE_DIR`, `GATHERER_CATALOG_ROOT`,
`BOT_CACHE_DIR`, `CLIENT_UNPACK_DIR`, and `LIVE_EVIDENCE_DIR`;
`GATHERER_WC_BANK_TILE` pins the fixture tile. Run it on Engine L (aubaine or
its 44594/1080 relay).

## State-gap and refusal paths

`panel.state-gap-stop-live` runs
`manual_click_live_tests::live_stop_retires_script_walks_and_keeps_the_manual_walk`.
It starts three real script walks and one manual WalkTo. Fleet Stop must retire
the script walks without cancelling the manual walk. It then changes focus,
marks and filters and checks that the retired labels do not return.

The call path starts at `crates/panel/src/manual_click_live_tests.rs:1873-1932`.
The test calls `Session::plan_fleet_action` and `Session::confirm_fleet_plan`,
then waits for the shared result at `:1933-1959`. The shared fleet plan submits
Stop to `Play::script_stop` in `crates/host-play/src/play_scripts.rs:1024`.
That owner retires the published script destination through
`retire_script_run_status` at `play_scripts.rs:114`. The slot loop and script
walk code must not publish an old destination afterwards. The test reads the
shared Fleet projection and checks the manual walk separately at
`manual_click_live_tests.rs:1962-2038`. `BotProjection::stop_eligibility`
(`crates/frontend-core/src/bot_projection.rs:117`) uses the same row facts.

The command and refusal rows use controlled-state headless drivers. A game
connection cannot create an empty focus, advance the UI error clock exactly,
or prove that an unavailable control submits no command. These tests are the
UI-rule alternative to game cells, not a claim of interactive desktop proof.

| Changed paths | Driver proof |
|---|---|
| `frontend-core/src/error_expiry.rs`, `frontend-core/src/lib.rs`, `frontend-core/src/session/commands.rs` | `frontend-core.driver-command-headless` submits through `HeadlessDriver`. Its Stop and refusal regressions are at `crates/frontend-core/src/driver_tests.rs:4182-4556`. `frontend-core.error-expiry-headless` checks timeout, replacement and resolution through the exported error tracker. |
| `frontend-core/src/resources.rs` | `frontend-core.resources-headless` checks that the resource count names in-game clients, not running scripts. |
| `panel/src/debug_panel.rs`, `panel/src/session_catalog.rs` | `panel.session-command-headless` checks debug capability and catalogue command handling through the panel session. `panel.debug-targets-headless` checks target identity changes and the Debug window's saved command state. |
| `panel/src/loadouts.rs` | `panel.loadout-command-headless` runs the form's command-seam actions and the empty-selection refusals (`crates/panel/src/loadouts.rs:962-1005`). |
| `tui/src/commands.rs`, `tui/src/input.rs`, `tui/src/overlay.rs`, `tui/src/palette.rs` | `tui.command-refusal-headless` runs the real key, palette and context-menu dispatch. The gate regression is at `crates/tui/src/bin_tests.rs:7300-7414`. It also captures the message line before and after timeout and resolution at 120x40 and 80x24 (`bin_tests.rs:7076-7214`). Set `LIVE_EVIDENCE_DIR` to retain text and JSON captures. |

These drivers do not prove the visual appearance of the disabled Debug and
Loadouts controls. A visible panel change still needs the operator's headed
check before push. The state-gap cell tests its real game behavior separately.

## Login notice

`frontend-core.login-notice-refused` runs
`a_refused_login_notice_outlives_log_out_and_renews_on_real_retries` in
`crates/frontend-core/tests/login_notice_refused.rs`. Real slot workers log in
to the refused endpoint `127.0.0.1:1` through `HeadlessDriver`; only client
asset startup is bypassed. After the first refusal the test logs the bot out.
The notice must stay in every poll while the bot is logged out and clear at
15 seconds. A second Log in lets the host retry after its backoff, and each
refusal must renew the notice so that it never lapses. The cell takes about
40 seconds of wall clock and needs no game engine.

The headless cells drive controlled host rows through the real poll and the
real front-end widgets. They check the parts a refused endpoint cannot
produce on demand: a queue wait, a successful login, the exact 15-second
boundary on an injected clock, and the drawn panel and terminal text.

| Changed paths | Proof |
|---|---|
| `frontend-core/src/login_notice.rs`, `frontend-core/src/session.rs` | `frontend-core.login-notice-refused` (above). `frontend-core.login-notice-headless` runs the notice clock tests (`login_notice_tests.rs`) and the poll-driven `views::tests::login_notice_*`: Error, Connecting, Queued and Logged out before the timeout keep the notice; a same-text failure renews it; a login clears it. |
| `tui/src/status.rs` | `tui.login-notice-headless` runs the status pane test and the 120x40 and 80x24 capture test `bin::tests::a_login_notice_stands_through_retry_states_and_expires_at_both_sizes`. Set `TUI_CAPTURE_DIR` to retain text and JSON captures. |
| (panel top line) | `panel.login-notice-headless` draws the panel banner and status rows through the sequence above. The `panel/src/app.rs` change also selects the panel cells of its own cover entries. |

## Log out of a retrying login

`frontend-core.logout-retry-refused` runs
`log_out_stops_a_login_that_keeps_retrying_for_the_single_command_and_the_fleet_plan`
in `crates/frontend-core/tests/logout_retry_refused.rs`. A real slot worker
logs in through `HeadlessDriver` to a local listener that accepts each
connection and closes it, so the login fails and is retried with the host's
backoff, and every attempt is counted on the wire. After two attempts the
cell checks that Log out is eligible in every state the retry loop passes
through, then logs the bot out, first with the single command that the panel
button and the TUI key submit, then with the Fleet Log out plan. A bot that
never logged in is skipped. Accepts are counted from the moment Log out is
sent, with no settling period. The requirement is that no new login attempt
starts after Log out, so at most one accept is allowed (an attempt that had
already passed the host's pre-connect check may still open one socket inside
the client connect, `vendor/fr-client-rust/crates/client/src/io/client_stream.rs:760-768,810-811`,
which the client closes at once) and none after it for 20 seconds, longer than
the next backoff step. The cell takes about 65 seconds of wall clock and needs
no game engine.

| Changed paths | Proof |
|---|---|
| `frontend-core/src/bot_projection.rs` (`LogoutEligibility`), `frontend-core/src/slot_view.rs`, `panel/src/app.rs`, `tui/src/app.rs` | `frontend-core.logout-retry-refused` (above). `frontend-core.logout-retry-headless` runs the projection table, the Fleet and Log out marked plans, the poll test and the repeat-guard tests (guard trip, operator Log out then Log in, a worker that ends, a reaped worker). `panel.logout-retry-headless` runs the button gate, the drawn side-panel Log out button, the focused-slot projection and the Fleet window's Log out all. `tui.logout-retry-headless` runs the `u` key, the dim and lit `[Log out u]` button and Fleet Log out at 120x40 and 80x24 (set `LIVE_EVIDENCE_DIR` to retain captures). |

## Saved settings

A saved setting (draw switches, walk and danger permissions, the map-bake
choice, the Quest Paths source, the session log file, notice
acknowledgements) takes effect only from the latest durable preference
snapshot, and a pending or failed save shows at its setting. The cells hold
the disk, fail the write, and check the applied models and the setting's
state through the real seams after `submit` has returned.

| Changed paths | Proof |
|---|---|
| `frontend-core/src/{command,log,log_file,map_bake,prefs,quester_paths,setting}.rs`, `frontend-core/src/session/prefs.rs`, `host-play/src/local_state.rs`, `host-play/src/local_state/frontend.rs`, `panel/src/{fleet_toolbar,log_pane,picker,ui_state,wall}.rs`, `tui/src/{lib,settings,shell}.rs` | `frontend-core.driver-command-headless` runs the held-then-failed table over every saved setting, the A-durable/B-failed case with a second process between, the session-file sink and the failure expiry through `HeadlessDriver`. `host-play.local-state-headless` runs the preference snapshot version test with the rest of the service. `panel.settings-feedback-headless` runs the same sequence through the panel session, the render-all dialog's drawn failure and the cold start. `tui.settings-feedback-headless` runs the popup row at 120x40 and 80x24, its expiry and the cold start. |

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

## Navigation generator inputs

The `not_live` entries for `nav-bake/src/bake.rs`, `transport.rs` and
`transport/static_routes.rs` identify generator code, not a live runtime
path. They do not exempt a change to the generated navigation from proof.
This includes the Entrana restricted-item list in `static_routes.rs`.

After a generator change, bake the navigation pack from the composed
generator head, compare the route corpus with the accepted reference, and
run the affected mapped cells with that pack and its matching flags and
sidecars. Classify every changed route as an improvement or justify it from
content. For the Entrana list, run `host.nav-zanaris-entrana`,
`host.sherlock-entrana-walk-arrive` and
`host.sherlock-entrana-content-categories`.

For a batch, do this after the final generator-source merge. An interim
source-only check is not the combined-pack corpus or live proof.

## Input values

The map has 23 named inputs, including the 19 reviewed in round 2. Five input
values are fixed in the map.
The caller must not replace the gather tile, Path, seeds file, or Engine L ports.

| Name | Value or owner |
|---|---|
| `ISOHOME` | Use the absolute wrapper path on the host. Merlot: `/Users/acfrazier/experiments/274bot/.superpowers/bin/isohome`. Chardonnay: `/home/acfrazier/experiments/274bot/.superpowers/bin/isohome`. |
| `BOT_LIVE_NAME_PREFIX` | Choose a unique prefix of 1 to 4 characters for each run. The Sherlock receipt in this round uses `l2r`. See `STANDING-RULINGS.md:33` and `crates/host-play/src/play_bootstrap.rs:67-75`. |
| `LIVE_EVIDENCE_DIR` | Use a fresh per-cell directory outside the throwaway HOME under the current task's evidence root. The no-route test creates its own timestamped subdirectory. |
| `ENGINE_A_DIR` | Merlot: `/Users/acfrazier/experiments/lostcity-289/engine`. Chardonnay: `/home/acfrazier/experiments/lostcity-289/engine`. Aubaine: `/Volumes/dev-scratch/engines/lostcity-289-l/engine` for Engine L relay cells. Keep the directory read-only. |
| `BOT_ENGINE_DIR` | Set to the read-only local Engine L R289 client tree on chardonnay; the Engine L runner accepts it as an override (`COMBAT-PREP-ESCAPE/r3/live.sh:5`). |
| `ENGINE_B_DIR` | For the chardonnay tunnel cell, use the local R289 client tree at `/home/acfrazier/experiments/lostcity-289/engine`. This optional client path is not the Engine B service. The service is the tunnel at `45594/2080`. |
| `ENGINE_Q_DIR` | Use the Engine Q tree on its host. Merlot: `/Users/acfrazier/experiments/lostcity-289-q/engine`. Chardonnay: `/home/acfrazier/experiments/lostcity-289-q/engine`. |
| `ENGINE_274_DIR` | The orchestrator supplies the 274 engine directory when it starts the 274 service. This value is not set in this map round. |
| `NAV_PACK_289` | Use the CI reference v17 pack for standard cells. The Zanaris and Sherlock Entrana cells use the task's head pack. Merlot: `/Volumes/dev-scratch/274bot-evidence/NAV-GATED-ENTRANCES/reference-v17/274bot-289-v17.navpack`. Chardonnay: `/srv/dev/scratch/274bot-evidence/NAV-GATED-ENTRANCES/reference-v17/274bot-289-v17.navpack`. See `STANDING-RULINGS.md:16`. |
| `NAV_FLAGS_289` | Use the matching `274bot-289-v17.navflags` in the same reference-v17 directory as `NAV_PACK_289`. |
| `WORLD_GAME_PORT` | The map fixes the no-route cell to Engine L's game port, `47594`, on chardonnay. |
| `WORLD_HTTP_PORT` | The map fixes the no-route cell to Engine L's HTTP port, `3080`, on chardonnay. |
| `NAV_SNAPSHOT_ROOT_289` | Use the read-only decoded snapshot that `isohome` exposes as `BOT_NAV_SNAPSHOT_ROOT`. The default paths are `/Users/acfrazier/.274bot/unpack-289` on merlot and `/home/acfrazier/.274bot/unpack-289` on chardonnay. See `AGENTS.md:149`. |
| `BOT_CACHE_DIR` | Make a fresh copy of the selected engine's `data/pack/client` at `<LIVE_EVIDENCE_DIR>/cache-289`. The Quester helper copies this source into its isolated run cache. |
| `CATALOG_ROOT` | Use the local `rs2b0t` checkout root. Merlot: `/Users/acfrazier/experiments/rs2b0t`. Chardonnay: `/home/acfrazier/experiments/rs2b0t`. Aubaine: `/Users/acfrazier/experiments/rs2b0t-fix-gnome-chop`, the catalog used by the reviewed Entrana cells. |
| `CLIENT_UNPACK_DIR` | Use a fresh writable directory at `<LIVE_EVIDENCE_DIR>/client-unpack`. Do not use the operator's real HOME. |
| `PANEL_LIVE_HOME` | Use a fresh throwaway HOME. Set it below `ISOHOME_ROOT`: `/Volumes/dev-scratch/isohome/panel-<prefix>` on merlot or `/srv/dev/scratch/isohome/panel-<prefix>` on chardonnay. |
| `GATHERER_WC_TILE` | The map fixes `3197,3209,0`, backed by the R289 normal-tree placement `maps/m49_50.jm2:5342` (`crates/api/data/game-data/289/gathering.json:17173-17190,38398`). It is outside LocationAuto's `AUTO_NEXT_TILES` (`crates/host-play/tests/gatherer_live.rs:107`) and stump fixture (`:197-243,5791-5812`). The caller does not choose it. |
| `QUESTER_PATH` | The map fixes `cook`, `sheep`, and `prince` for the Cook, sheep-spinning, and Prince-production cells. The caller does not choose it. |
| `QUESTER_SEEDS` | The map points to the committed Cook, sheep-spin, and Prince-production seed files in `crates/host-play/tests/fixtures/quester-path/`. Run the command from the repository root. The caller does not supply seed data. |
| `QUESTER_PATH_DIR` | Set to `${PWD}/crates/script/paths/289` from the composed repository root. The Zanaris Path source must be merged into the tested head. Do not substitute an unmerged external Path checkout. |
| `NAV_PACK_274` | Use `274bot-274-v17.navpack` in the same reference-v17 directory as `NAV_PACK_289`. The orchestrator supplies this path to `nav.essence-follow` when it starts the 274 service. |
| `BOT_UNPACK_DIR` | Use a fresh writable unpack directory at `<LIVE_EVIDENCE_DIR>/client-unpack`. This can be the same path as `CLIENT_UNPACK_DIR`. |

Run commands from the repository root. Keep writable cache and unpack paths in
fresh run directories. `AGENTS.md:149` requires a throwaway HOME for product
runs and `isohome` exposes the read-only navigation snapshot.

## Script surface boundary

The exact core paths beside `script-js/src/runtime/**`, `script-js/src/materialize/**` and `script-js/src/library/**` share the external-script
start, snapshot, tick and Stop boundary. They do not add a second loader.

| Cell | Call path |
|---|---|
| `host.script-runtime-load` | `script_api_live::runtime_cell` starts through `Play::script_start_load`, `SlotScript::start_surface` and `JsSurfaceFactory::spawn` in `script-js/src/surface.rs`. It observes a real tick, then Stop and physical retirement. |
| `host.api-gather-wire-shadow` | The v2 Gatherer request enters `SlotScript::start_api_gather`. The real reconnect and wire-shadow checks cross the shared native fact and seat rows. |
| `scenario.auto-fighter-mage-live` | The registered `auto_fighter_mage` scenario starts the catalog AutoFighter with Fire Strike. Its `Autocast.arm` call enters the Rust `Autocast` family and its observation accessor. |
| `host.script-progress-journal` | `journal_cell` reads Romeo & Juliet twice. The second read uses `script_api_live_read_again`; the progress requests enter `SlotScript::start_api_progress`. The cell compares the closed-journal stamps and verifies that the quiet paint lease ends. |

Classify compiler-only or test-only changed hunks in the task report when
they have no game behavior to exercise. Keep their compile or test proof.
Do not hide a whole production file in `not_live` for such a change.

Exact core rows use named headless commands for workspace registration,
selected-cache facts and controlled host scheduling states. Those commands
are not game witnesses. In particular, revision-274 quest facts do not prove
revision-274 gameplay.

`host.quest-watch-seed-engine-q` proves only the unstarted seed, bank, relog
and final logout. It does not start or qualify the campaign quest. Its speed
helpers have controlled tests; Engine Q speed changes remain forbidden.

The navigation positive cells must use a pack with the matching nonempty
quest-family identity. A stale or unbound pack cannot substitute for those
positive runs.

## Background work health

`script.work-admission-headless` controls a native step and the monotonic clock.
It proves that a slow returned step permits later work, that shutdown reports
an unreturned worker, and that panic and start failure keep their conditions.
It also consumes a completed terminal before an uncontained cleanup panic,
then checks that the worker exits and releases its retained resource custody.
These states cannot be created by game content. `panel.headed-nav-script`
complements this proof with real profile and cache preparation before the live
script starts.

## Add a cell

1. Write and run the live cell against real game content. Do not seed the target resource or entity.
2. Add one `[[cover]]` row for the narrowest product paths the cell exercises.
3. Add each cell as `name|command` in that row's `cells` array. Keep the command exact.
4. Add test-only files to `[not_live]`, not `[live]`.
5. Run the tool on the change. Confirm that it lists the new cell and no changed live path remains uncovered.

Use the Rust module names from `cargo test ... -- --list` for exact selectors.
A `#[path = "compat/dialog.rs"] mod dialog` declaration names `dialog::...`,
not `compat::dialog::...`. The source directory is not a module prefix.
A zero-test selector is not proof, even when Cargo exits successfully.

An explicit exception in `not_live.exceptions` keeps a path live only when it
also matches a live glob. Add the cover row for that path.

The scenario registry and nightly cell lists are separate from changed-path matching. Inspect them with:

```sh
python3 -B tools/nightly/run_nightly.py --list
```

## Controlled legacy adapter limits

`compat.dialog-combat-close-adapter` and `compat.make-x-clock-adapter` are
**adapter proof, not a game witness**. No catalog script reaches these branches
in a fixed-deadline cell. The first test checks the legacy dialogue result for
combat with a visible NPC or player targeting this account, unrelated targets,
and self-damage without an attacker. The second checks the legacy Make-X
count-open, count-close and menu-close deadlines. Native Mordred dialogue and
native Sheep spinning do not exercise these adapters.

Keep both limits in the W6 report. W5 also adds its direct makeOne game cell;
that cell does not prove the Make-X count/menu waits. The retained **3 s / 3 s /
5 s wall-clock waits** in `compat/production.rs` are an open item for the compat
track, in the same class as its queued `WaitChoiceAck` and Main-doc 3000 ms
waits. That track must count these waits in observed ticks. This mapping does
not fix or retire the timing debt.

## Run cells

The runner uses the command from `map.toml`. It does not duplicate per-cell commands.
Use `live_map.py run` so one tool lists and runs cells. `run_cells.py` holds the
run logic and imports `live_map.py`. This split keeps listing stable while the runner
evolves. No `map.toml` schema change was needed. Every current cell is one shell
string with `name|command`. Port choice stays in the runner by rewrite, not in the map.

```sh
python3 -B tools/live-map/live_map.py run --repo <lab clone> \
  --cells frontend-core.driver-command-headless,host-play.local-state-headless \
  --prefix lcr --evidence /path/to/evidence --passes 1

python3 -B tools/live-map/live_map.py run --repo <lab clone> \
  --diff BASE..HEAD --prefix lcr --evidence /path/to/evidence --passes 3 \
  --engine-game-port 44594 --engine-http-port 1080 \
  --quest-game-port 44694 --quest-http-port 1180

python3 -B tools/live-map/live_map.py run --repo <lab clone> \
  --cells host.script-runtime-load,panel.state-gap-stop-live,nav.route-search-live \
  --prefix lcr --evidence /path/to/evidence --passes 1 \
  --engine-game-port 47594 --engine-http-port 3080 \
  --catalog-root /home/acfrazier/experiments/rs2b0t \
  --unpack-source /home/acfrazier/.274bot/unpack-289
```

The runner refuses to start when the clone has uncommitted tracked changes. It runs
cells one at a time and never stops on a failure. Populate `<cell>/cache-289` with
the copied cache seed. For each pass, the runner copies that seed without symlinks
to `<cell>/passN/evidence/cache-289`. This copy is `BOT_CACHE_DIR`, inside that
pass's `LIVE_EVIDENCE_DIR`; writes in one pass do not change the seed or another
pass's cache. The runner also creates per-cell unpack dirs and a kept panel home.
It sets `BOT_LIVE_NAME_PREFIX`, `BOT_NAV_BUILD=skip`,
`WORLD_GAME_PORT`/`WORLD_HTTP_PORT` from the selected engine ports. Explicit caller
engine and nav paths take precedence over discovered reference-v17 defaults.
It sets `ISOHOME_DIR` to `<LIVE_EVIDENCE_DIR>/home`
for each cell and pass, so `isohome` keeps `HOME` under `BOT_EVIDENCE_DIR`
(`crates/host-play/tests/script_api_live.rs:503-507`). Panel cells override this
with the inline `ISOHOME_DIR=${PANEL_LIVE_HOME:?}` in the map command. Quest cells use the quest ports and set
`QUESTER_SPEED_RESTORE=0`. The runner leaves `QUESTER_TICK_MS` unset and refuses
any cell whose command sets it. Engine Q stays at 300 ms; never send `::speed`.
It rewrites inline `_GAME_PORT`, `_HTTP_PORT`, `--port`, and `--http-port` values
to the selected ports.
`ISOHOME` comes from the host wrapper where the map command names it. Each cell runs
by `bash -lc` from the repo root.

Paired quest cells, including `host.quester-jonny-stand-engine-q`, also require
`BOT_LIVE_PARTNER_NAME_PREFIX`. Export a distinct partner prefix in the same
remote shell that invokes the runner (for example, partner `b2q` with
`--prefix b29`). The runner inherits that value; a Merlot-only environment
assignment is not forwarded by the lab wrapper. It still sets the primary
prefix from `--prefix`.

Export the selected combined nav pack and matching flags for batch proofs.
Do not substitute a discovered reference pack when its quest-family binding
does not match the selected data.

Use `--catalog-root <path>` for the local `rs2b0t` checkout root. The flag is
required when any selected cell uses `${CATALOG_ROOT}`. The runner exports it as
`CATALOG_ROOT` and refuses to start without it. Use `--unpack-source <path>` for
the revision decoded unpack directory. The flag is required when any selected
cell uses `${PANEL_LIVE_HOME}`. For each such cell and pass the runner copies the
source tree (never a symlink) to `panel-home/.274bot/unpack-289`, as the panel
live test expects (`crates/panel/src/manual_click_live_tests.rs:77-90`). The copy
runs once per panel home and the receipt records it as `unpack_source`
(`<source>#<hash>`).
The merged `live-port-env` repair (`8b9e84e2691d`) makes its covered live tests
read the selected ports through `live_ports`. The runner does not rewrite
ports embedded in Rust code. The five Engine B sites left outside that repair
retain their 45594/2080 contract; changing them needs a separate brief.
Non-quest lab cells use Engine L. On aubaine the 44594/1080 defaults reach Engine L
through the relay. On chardonnay pass `--engine-game-port 47594 --engine-http-port 3080`
(the forwarded Engine L) and never use 44594/1080 there. Quest cells use Engine Q.

Each cell and pass writes `passN/cell.log` and `passN/receipt.txt` with `cell`, `pass`,
repo `head`, `command`, `start_utc`, `end_utc`, `exit_code`, `log`, `evidence_dir`,
`isohome_dir`, plus `catalog_root` when `--catalog-root` is set and `unpack_source`
when the cell stages a panel unpack. The runner prints a
summary table and writes `summary.txt`. The table lists `cell`, `pass`, `exit`, `seconds`,
`start_utc`, `end_utc`, and `log`, followed by one `command`, `start_utc`, and `end_utc`
line per cell and pass. A non-zero exit means at least one cell failed.

### Support proof boundaries

`api.hostlog-write-headless` passes a failing writer to the logging sink. It
checks that the write does not panic. This is process logging proof, not a
game witness or a full API-suite pass.

`panel.profiles-close-headless` drives the panel facade: Cancel without a
form is refused and leaves the list open; window close with no form or a
clean form succeeds. Main's headed Profiles check is still required.

The existing `host.survivable-admission` cell covers the selected-port helper
because its real profile reads both `live_ports` functions. Run it through
the shared runner with the selected Engine L ports. `bank_npc_live.rs` is
test-harness-only: `host-play/src/lib.rs:634-635` gates it to tests or both
`live-harness` and `test-support`. Its endpoint assertion/receipt change is
not shipped game behavior.

For Fleet order captures, set `TUI_CAPTURE_DIR` while running the TUI test
`bin::tests::alt_keys_reorder_the_saved_load_order_and_the_drawn_table`.
It checks the moved table at 80x24 and 120x40 and writes text plus coloured-cell
JSON under `<TUI_CAPTURE_DIR>/fleet-order`. The proof records the saved and
drawn order after the real Alt+Down key path. This is controlled UI proof,
not a game witness or a per-bot memory measurement.

### BATCH-P proof scope

The memory-profile cells are controlled contract tests, not live capacity
measurements. The scatter test prepares ten slots and checks both active and
idle workloads with `stress50` and `stress50-full`; it does not measure fifty
running clients. The Chicken bank boundary and the twelve
`chicken_bank_oracle_` cases are controlled proof. The separate
`scenario.chicken-killer-bank-live` command must prove real combat, fresh
loot and deposit, return, and further work.

`scenario.boat-fare-retry-live` runs the same three real cases in order:
`boat_fare_v1_ts`, `boat_fare_far_start_v1_ts`, and
`boat_fare_reconnect_v1_ts`. Each uses the caller's explicit engine, pack and
navflags, and its own `<LIVE_EVIDENCE_DIR>/<case>/home`. The first failing
case stops the command. Export `ENGINE_A_DIR` for the selected Engine L root
(the name is historical), and the matching `NAV_PACK_289` and
`NAV_FLAGS_289`. Record the engine identity before the live command.

### Queued tooling item: nightly supplied-pack reuse

Status: **queued**; no nightly-runner change is part of this batch.

`run_nightly.py` drops the inherited nav environment
(`tools/nightly/run_nightly.py:1893-1898`, `tui_cmd:1992-1997`), so nightly
cells rebake instead of using a given pack. `NIGHTLY_NAV_FIXTURE` supplies
CI lanes, not those live CLI invocations. Until the nightly tool can use the
selected pack, the fare map row calls the existing CLI directly. This is a
validation-tool limit, not a change to the product's route behavior.

### Map and snapshot coverage boundaries

`frontend-core.map-walk-headless` runs the full frontend library with the
selected pack, navflags and engine paths, with `LIVE` unset. Its real-seam
test is `group_walk::tests::frontend_owned_map_staging_arms_the_real_profile_bound_play`:
`OperatorSession::walk_map_context` creates the frontend context;
`WalkMap::confirm_walk_plan` stages its selection; then
`OperatorSession::confirm_marked_walk` calls `Play::map_walk_group`.
The test uses a real profile-bound `Play` and verifies its actual walk arms.
Only the game client is absent. The same suite checks typed incomplete-search
details and retention of a failed walk's destination. This is controlled
contract proof, not a real budget-exhausted live walk or an arrival witness.

The snapshot, optional FlatBuffer field and native observation paths share
the existing wire-shadow, runtime and dialog-close-adapter cells. Those
declarations do not turn the adapter into a live combat-close witness.
Actual changed dialogue cells and final three-pass coverage remain required.

`crates/nav-planner/examples/route_parity.rs` is an offline real-pack
diagnostic example, not a product runtime. Its exact `not_live` classification
does not waive the release corpus, changed-route comparison or final combined-pack
qualification.

