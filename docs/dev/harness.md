# Reusable live harness

> **Developer reference.** This page is for people who change 274bot.
> To use 274bot, start at the [README](../../README.md); to find another page, see the [docs index](../README.md).

The harness runs real revision clients against your local engine for the
bound **server profile**. It keeps scenario preparation, proof predicates
and failure exits in Rust so preservation projects can reuse the same
behavior through `host-play`, the TUI or the panel. No engine assets or
external catalog scripts are included in this repository.

0.2.7 Beta 2 is the current release. Revision 289 is the production revision: the harness cells
are written, run and claimed for it. Revision 274 cells run best-effort and
are neither tested nor claimed.

For ordered multi-case native runs with ledgers, receipts and process-tree
ownership, prefer the tracked suite entrypoint
[`e2e-suite`](e2e-suite.md). A green child or zero exit is an observation
record — not automatic script qualification. Captures that require human
readback stay `pending_visual_review`; raw failures stay preserved.

## Live entry points

Live cells need a local engine and cache, run one at a time
(`--test-threads=1`), and are all behind `LIVE=1` with `--ignored`:

```sh
# Native script cells (Gatherer, combat, Quester families, bank, WalkGuard).
LIVE=1 cargo test -p host-play -- --ignored
# Navigation and door cells against engine-backed endpoints.
LIVE=1 cargo test -p e2e -- --ignored
```

Cells behind Cargo features do not build without them. `quester_dev_paths_live`,
`quester_families_live`, `quester_path_live`, `quester_sheep_live` and
`cook_bank_loop_live` need `--features "live-harness test-support"`;
`bank_choice_live`, `quester_queue_live`, `quester_squire_live` and
`quester_vampire_live` need `live-harness`; `script_api_live` needs
`test-support`; `debug_panel_live` needs `debug-catalog`;
`catalog_boundary_live` and `world_boundary_live` need `memory-profile`. The
`--lib` cells (`walk_guard_live_tests`, `quester_journal_live_tests` and the
other `*_live*` modules) run under `cargo test`. Each cell file's header lists
its own environment.

The `quester_skip_active_cook_parks_then_sheep_completes` cell in
`quester_queue_live` also needs `--features "live-harness test-support live-probe"`.
`test-support` on `panel` or `tui` enables the Skip quest button or `X` key. Each
feature forwards `frontend-core/test-support`, which enables the shared
`SkipQuest` command. Build both control-enabled binaries with:

```sh
cargo build --locked -p panel --bin panel-play --features test-support
cargo build --locked -p tui --bin tui-play --features test-support
```

Release-profile builds that omit `test-support` omit the command, button and
key.


The expected-combat Quester cell seeds Tourist Trap (`desertrescue`) at stage
3. The loc-dialogue cell seeds Priest in Peril (`priestperil`) at stage 1.
These permanent server variables are not transmitted to the client. After
relog, each cell sends `getvar` before Start. Its oracle requires the exact
fresh system reply. The fixture receipt records that reply, its message
sequence, and the client-cache value as a diagnostic. An absent, stale,
incorrect, or player-authored reply cannot prove the seed.
The combat fixture also carries full waterskins. The real desert heat timer
can otherwise damage the player and interrupt dialogue before combat starts.

The loc-dialogue result uses the content's final dialogue line after its
stage assignment, not the untransmitted varp. Neither cell sends cheats after
Start. Family Paths use the released Cook Path's empty text-rule list and
journal-colour fallback. Every colour selects the fixture's one authored
stage. The fixture builder receives an explicit Cook stage-zero seed because
the empty text-rule list supplies no seed hint.
The held-item result requires the retained Completed lifecycle receipt.
Native completion removes the live status, so the result does not depend on
observing a temporary Complete status.
The ground-use-on fixture seeds `cogquest`, which backs the content's
`get_cog_progress` procedure. It requires a fresh `getvar cogquest` reply
before Start. A procedure name is not a variable name for `setvar`.
Its empty-bucket result prevents the sequence from repeating the cooling
action after that action consumes the water.
Before Start, it also waits for the real ground spawn within its content
respawn interval plus the query budget. It never creates a ground item.

The staged Imp hand-in cell requires a retained `Completed` lifecycle receipt
and the actual green Imp quest row. Its receipt stores both in `m1_completion`.
The slot removes the native run during completion, so later status polling
cannot require a transient `imp:2` status. The cell still requires both wizard
talks, valid dialogue packets, four seeded beads, and removal of those beads.
The dialogue packet check accepts only `CLOSE_MODAL` for `CloseModal`, and
only when the action snapshot shows the selected quest completion scroll as the
open main modal and the close follows the final hand-in talk. Unknown
operations and extra packets fail.

The Sheep bank-loop cell proves withdrawal from the loaded bank, matching
bank generation, empty stock, and exact received items. It accepts `Ready`
as well as `Withdrawing` because the provisioner can finish before observation.
The cell still requires spinning, twenty balls of wool, hand-in, green progress,
and the retained `Completed` lifecycle receipt.

The Quester Path smoke runs one content quest id from the embedded release
index (or from `QUESTER_PATH_DIR`) under a fixed 45-minute deadline:

```sh
env HOME="$(mktemp -d)" LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=qh BOT_NAV_BUILD=skip \
  WORLD_GAME_PORT=44594 WORLD_HTTP_PORT=1080 WORLD_NAV_PACK=<274bot.navpack> \
  WORLD_ENGINE_DIR=<engine-dir> RS2B0T=<catalog-root> \
  BOT_CACHE_DIR=<writable-copy-of-client-cache> LIVE_EVIDENCE_DIR=<evidence-root> \
  QUESTER_PATH=cook QUESTER_SEEDS=<evidence-root>/seeds/cook.json \
  cargo test -p host-play --test quester_path_live --features "live-harness test-support" \
    live_quester_path_smoke -- --ignored --nocapture --test-threads=1
```

Headed runs use the panel (or TUI) scenario with `--live
script_quester_path` and optional `HEADED_RECORD=1` video, as below. The
headless cell needs `QUESTER_SEEDS`; the headed scenario accepts seeds and
otherwise starts from a clean account at the Path's authored anchor.

### Fast quest runs (Engine Q only)

`QUESTER_TICK_MS=300` opts the generic `quester_path` cell and the panel or
TUI `--live script_quester_path` entry into a 300 ms world tick. All three
entry points use one shared `ScenarioRunner` lifecycle: a scenario that
explicitly requests a tick sends `::speed <ms>`, waits for an exact
system-chat confirmation newer than that send, then restores `::speed 600`
during terminal teardown. A scenario that requests no tick — the default —
sends nothing. Every send records the current chat mark first, so a matching
confirmation already in the ring neither suppresses nor satisfies it.
`::speed` is world-wide, so use `QUESTER_TICK_MS`
only on isolated Engine Q, never on shared Engine A or the builder. The runner
logs the target endpoint once when the command fires. Use `local-289q` only in
the disposable run HOME's `~/.274bot/servers.json`, with game/asset ports
`44694`/`1180` and the Q engine directory as `login_key.engine_dir`; leave
`members` unset so the Q `world.json` supplies world membership. Do not add it
to product defaults.

`QUESTER_SUSTAIN_RUN=1` enables the existing per-account `Sustain` behavior.
When tick speed is set, sustain defaults on unless explicitly disabled with
`QUESTER_SUSTAIN_RUN=0`. It sends the content debugproc cheat `~energy` at each
navigation leg and whenever run energy is at or below 25. Engine Q is configured
`maxConnected: 1`, so a successful cell is its only connected account; terminal
PASS/FAIL cleanup restores `::speed 600`. A process interrupted before terminal
cleanup can leave Q at the fast rate, so keep Q quest-only.

```sh
HOME=/path/to/throwaway-home BOT_SERVER_PROFILE=local-289q \
QUESTER_LIVE_KEEP_HOME=1 LIVE=1 BOT_CPU=1 BOT_NAV_BUILD=skip \
BOT_LIVE_NAME_PREFIX=qh WORLD_GAME_PORT=44694 WORLD_HTTP_PORT=1180 \
WORLD_ENGINE_DIR=/path/to/lostcity-289-q/engine \
WORLD_NAV_PACK=/path/to/274bot.navpack RS2B0T=/path/to/rs2b0t \
BOT_CACHE_DIR=/path/to/owned-client-cache LIVE_EVIDENCE_DIR=/path/to/evidence \
QUESTER_PATH=priestperil QUESTER_TICK_MS=300 QUESTER_SUSTAIN_RUN=1 \
QUESTER_SEEDS=/path/to/seeds/priestperil.json \
cargo test -p host-play --features "live-harness test-support" \
  --test quester_path_live live_quester_path_smoke -- --ignored --nocapture --test-threads=1
```

Interaction timing uses wall-clock limits: the `InteractPlan` defaults are a
20,000 ms inactivity window and an 8,000 ms settle deadline (authored
`settle_ms` overrides them); repeated action rounds are bounded by
`ACTION_ROUND_MS=8,000`, door waits by `DOOR_WAIT_MS=5,000`, and the host's
`EXIT_SAVE_TIMEOUT` is 5 s. A 300 ms world supplies more game ticks within
these bounds, so fast runs are throughput/progress checks, not release timing
qualification. Release qualification must still run at 600 ms. The same Q-only
guard and cleanup apply to the headed `--live script_quester_path` entry.

## Evidence paths and throwaway HOME

| Variable | What it holds |
| --- | --- |
| `LIVE_EVIDENCE_DIR` | Receipts, CPU-rendered PNGs and captures for `host-play` live cells. Always outside the throwaway HOME. |
| `BOT_EVIDENCE_DIR` | Receipt root for the `script_api_live` cells; the test's `HOME` must be a disposable directory under it. |
| `274BOT_SMOKE_DIR` | Capture root the `e2e-suite` runner points at its run's `shots/` directory. |

Live cells run under a throwaway `HOME` (`env HOME="$(mktemp -d)"`) with an
ephemeral vault and minted `live<token>_<i>` accounts, so a run never
touches `~/.274bot` or the operator's vault. The source client cache is
copied into the cell's temp root before use; the run never writes the
source cache.
Windows uses `USERPROFILE` only when `HOME` is unavailable; an explicitly
blank `HOME` remains explicit.
Scenario PASS/FAIL receipts report a top-level `ticks` count in observed game
ticks (one per `PLAYER_INFO`; about 600 ms each on 289 while run steps execute).
It excludes seed waits. Client cycles, frontend polls and dirty snapshots do
not count; `elapsed_ms` records wall time separately.

## Live-cell rules

- **Real content.** In the Quester Path and Gatherer cells, shared world
  entities stay real: no spawned NPCs, locs or chance-drop waits. Only the
  tested account is seeded, before Start.
- **Seeded incidentals are observations, never requirements.** A seeded gem
  disposal or casket deposit is verified when observed; a naturally dropped
  one is recorded as an observation (`not_exercised` when absent) and never
  required to pass.
- **Nothing cheats after Start.** In the Quester Path and Gatherer cells, all
  fixture seeds, kit, stats and teleports happen before Start; after Start the
  cell only observes completion, stops and restarts.
- **Fixed deadlines.** Every cell runs under a fixed bound: 45 minutes for
  the Quester Path smoke (`QUESTER_PATH_DEADLINE_S` overrides the headed
  scenario's deadline; zero and non-integer values are rejected), per-scenario
  deadlines with a short terminal grace elsewhere. A cell that cannot finish
  inside its bound fails instead of burning the shared engine.
- **Random-event Guardian cells are the one exception** (operator decision
  2026-10-14): after the activity's first XP they summon one random event
  through the engine's own random-event machinery. A random event always
  appears beside the player, so how it was triggered must not matter to the
  host. See "Random-event Guardian cells" below.

## Engine test patches

Some cells need a test-only cheat that the stock 289 engine or content lacks.
The patches, what each adds and how to apply them are in
[`tools/engine-patches/289/README.md`](../../tools/engine-patches/289/README.md).
A cell that needs a patch the engine lacks ends with outcome `UNAVAILABLE`
in its evidence JSON, and its message names the patch file. It is never a
plain FAIL, and it still exits non-zero.

`0001-engine-givebank-cheat.patch` (`::givebank`, engine) is needed by these
scenario cells, besides the `host-play` live tests that seed a bank:

<!-- engine-patch-cells:0001 -->
`aio_teleport`, `aio_teleport_falador`, `aio_teleport_no_staff`, `alcher`,
`alcher_custom`, `alcher_custom_alias`, `alcher_custom_name`,
`alcher_defaults`, `alcher_dwarven_mine`, `alcher_fire_battlestaff`,
`alcher_large_batch`, `alcher_low`, `alcher_ordered`, `alcher_swarm_drain`,
`auto_fighter_bank`, `auto_fighter_food_override`, `auto_fighter_targets`,
`bank_fletcher`, `bank_fletcher_cut_string`, `bank_fletcher_headless`,
`bank_fletcher_shafts`, `bank_fletcher_string`, `bone_burier`,
`bone_burier_v2_js`, `bone_burier_v2_ts`, `brimhaven_moss_inspect_v1`,
`chaos_druid_bank`, `cook_bot`, `cook_bot_lobster`, `fire_giant_bank`,
`fire_giant_bank_prepared`, `fire_giant_camelot_prepared`, `flax_aio_spin`,
`flax_spinner`, `flour_collector`, `gem_cutter`, `gem_cutter_named`,
`green_dragon_bank`, `green_dragon_bank_default_prepared`,
`green_dragon_bank_prepared`, `green_dragon_tele`, `herb_cleaner`,
`herb_cleaner_empty_bank`, `herb_cleaner_named`, `hill_giant_bank`,
`hill_giant_bank_prepared`, `hill_giant_loot_deposit`, `jive_chests`,
`jive_crafting`, `jive_demons`, `jive_dragons`, `jive_dragons_black_range`,
`jive_dragons_mixed_food_restock`, `jive_dragons_potion_restock`,
`jive_enchanter`, `jive_kbd`, `jive_kq_four`,
`moss_giant_bank`, `moss_giant_bank_start`, `moss_giant_dart`, `mule_crafter`,
`potion_maker`, `potion_maker_named`, `quester_queue`, `quester_skip`,
`ranging_guild_bank`, `ranging_guild_full`,
`rune_crafter`, `rune_crafter_earth`, `smelter_bot`,
`smelter_bot_steel`, `superheater`, `superheater_fire_battlestaff`,
`superheater_mithril`, `superheater_silver_low_natures`, `superheater_steel`,
`tanner_bot`, `tanner_bot_hard`, `thiever_diagonal_door_trap`, `vial_filler`,
`vial_filler_east`
<!-- /engine-patch-cells -->

`0002-content-skill-macro-event-cheat.patch` (`::skill_macro_event`,
content) is needed by:

<!-- engine-patch-cells:0002 -->
`guardian_skill_big_fish`, `guardian_skill_ent`, `guardian_skill_gas`,
`guardian_skill_lost_axe`, `guardian_skill_lost_axe_same_kind_head`,
`guardian_skill_lost_pickaxe`, `guardian_skill_river_troll`,
`guardian_skill_rock_golem`, `guardian_skill_shade`,
`guardian_skill_tree_spirit`, `guardian_skill_watchman`,
`guardian_skill_whirlpool`, `guardian_skill_zombie`
<!-- /engine-patch-cells -->

The scenario crate's `every_engine_patch_user_is_documented` test keeps
these lists in step with what the cells send.

## Random-event Guardian cells

`guardian_event_*` (the eight general events, `::macro_event 1`–`8`) and
`guardian_skill_*` (the twelve skill events, `::skill_macro_event 1`–`12`)
check that the host Guardian handles every random event while a low-level
account (combat level 3, 10 Hitpoints) does real work: the Gatherer fishing at
Catherby (general events and the fishing events), mining at Varrock East,
chopping west of Lumbridge (oaks at Draynor for the ent), and the catalog BoneBurier and Thiever cards at
Lumbridge for the prayer and thieving events. Each cell seeds only the
account's own skill and required items, waits for the activity's first XP,
then sends one trigger after the content's busy reply. It never adds an NPC
itself.

The `guardian_skill_lost_axe_same_kind_head` variant also seeds a Rune axe
head and drops it with an inventory action before triggering the event. Its
Bronze axe oracle checks that the Guardian recovers the event's own tier.

The oracle (`crates/scenario/src/random_episode.rs`) judges the Guardian's
documented behaviour for the event's class from the host hold and the
snapshot:

- **Evade** (Swarm, River troll, Rock Golem, Tree spirit, Shade, Zombie,
  Watchman): the hold rises within five ticks of the attacker targeting the
  bot; the bot gets past the attacker's content `maxrange + 1` from its start
  tile; it never moves back toward the start while the attacker still targets
  it; the hold is not released before the escape; and the attacker doesn't
  hit the bot again while still targeting it after the hold is released.
- **Dialogue** (Genie, Drunken Dwarf, Mysterious Old Man): the Guardian
  engages the NPC, the NPC leaves, and the reward arrives. For the Genie, the
  lamp is also rubbed (lamp auto, Strength).
- **Strange plant**: the fruit is picked. **Strange box**: the box is opened
  and consumed. **Mime and Maze**: the bot is held while trapped and comes
  back.
- **Lost pickaxe and axe, big fish**: the lost tool is back in the pack or
  hand.
- **Whirlpool, gas rock** have no documented Guardian act: the bot must
  survive them and keep its tool.
- **Ent**: the bot keeps its axe. The Guardian cancels the forced chop and
  holds the Gatherer until the Ent reverts, despawns, or reaches 60 held ticks.

Every cell also fails on a death or on the content's failed-event teleport.
It passes only when the hold is released and the activity earns fresh XP
after the event. The terminal evidence carries a `random_event` receipt
(ticks, tiles, hold edges, attacker start and range, escape distance).

The host identifies attackers by the macro-event NPC types in the content,
not their display names. A visible former attacker does not replace a newly
detected event. The host normally waits for the event attacker to despawn
before returning. Its flee route clears both `maxrange + 1` and
`wanderrange + 3` from the earliest NPC sighting
(`Npc.ts:661-714`, `macro_events.rs2:166-173`).

If the NPC leaves client view, the bot still walks toward its flee target.
Once the bot stops outside chase range, 22 consecutive ticks without the
attacker (about 13 seconds) end the hold, even if the route stopped short.
The content checks for hostile removal every 20 ticks. The extra two ticks
allow NPC processing before player movement and delayed plant deletion
(`antimacro.npc:130`, `World.ts:340-408`, `macro_event_triffid.rs2:51-55`).
A reappearing attacker resets that countdown.

If the bot is boxed in while still being attacked inside chase range, it
stops trying after `flee_distance + 2` fresh ticks without a new greatest
distance from the attacker's first position. This is 21 ticks for a Swarm
and 9 for a Tree spirit or River troll: about 13 seconds or less.
An unblocked outward route moves one or two tiles per tick
(`PathingEntity.ts:135-152`). This budget allows each nearer escape ring
one try, plus two ticks to observe movement. Accepted walks and repeated
observations of the same game tick do not count as distance progress.

Only while the attacker is out of sight or has stopped attacking can the
script remain paused for up to 1,002 game ticks (about 10 minutes).
This total attempt limit uses the normal hostile spawn's
`npc_add(..., 1000)` lifetime, plus two ticks to observe removal
(`macro_events.rs2:36`, `macro_event_dryad.rs2:10`).
Neither limit proves the attacker is gone. An angry Strange plant extends
its lifetime when it changes type (`macro_event_triffid.rs2:35`), and NPC
delays can suspend its lifetime (`Npc.ts:120-148`).
At either limit the Guardian sends no further walk, including no return
walk. The script resumes and can eat or fight. The same actor cannot
immediately pause the script again, until removal or type change.

Ent ownership prefers the local player's visible NPC target, set by the
content's `p_opnpc(1)` (`macro_event_ent.rs2:18-20`, `PlayerOps.ts:404-415`,
`World.ts:710-717`, `PathingEntity.ts:514-531`). A known other target disables
the proximity fallback; an Ent visibly targeted by another player is also
excluded. Before the target update reaches the client, ownership is not
observable. The remaining five-tile fallback can mistake a nearby player's
Ent for this bot's event. It is a bounded guess, not proof of ownership.
The radius covers a size-three tree and two running steps
(`antimacro.npc:1285,1325`, `PathingEntity.ts:144-148`). Every hold ends after
60 game ticks, from the Ent's `npc_add(..., 60)` in `macro_event_ent.rs2:18`.
An expired visible Ent cannot immediately start another hold.

A Mime return message can block the mapzone-exit script that restores the
inventory tab. The Gatherer continues that message before it requires the
inventory tab to become available.

```sh
ENGINE_DIR=<engine-dir> BOT_LIVE_NAME_PREFIX=ge BOT_CPU=1 BOT_NAV_BUILD=skip \
  isohome target/release/tui-play --profile local-289 \
  --nav-pack <274bot-289-v17.navpack> --nav-flags <274bot-289-v17.navflags> \
  --catalog <rs2b0t> --live script_guardian_skill_river_troll
```

## Running a scenario

Start the local engine for the profile under test and provide its client
cache. Application builds already bake and stage nav for the selected
**build-time** revision (`BOT_NAV_REVISION`, default **289**) next to the
binary, so ordinary WalkTo needs no manual pack step. Use `nav-pack` only for
deliberate custom-input bakes. `RS2B0T` (or `--catalog DIR`) names the
external catalog checkout for scenarios that start a catalog script:

```sh
export ENGINE_DIR=/absolute/path/to/engine
export RS2B0T=/absolute/path/to/catalog
# optional custom bake (not required for default bundled nav):
# export NAV_PACK=/absolute/output/274bot.navpack
# export NAV_FLAGS=/absolute/output/274bot.navflags
# cargo run --locked --release -p nav-bake --bin nav-pack

cargo run --locked --release -p tui --bin tui-play -- --profile local-289 --live script_thiever
cargo run --locked --release -p panel --bin panel-play -- --profile local-289 --live script_thiever

# Run an embedded Path (repeat with any id from the bundled Path index).
QUESTER_PATH=cook cargo run --locked --release -p panel --bin panel-play -- --profile local-289 --live script_quester_path

# Run a Path supplied by a folder of Path documents.
QUESTER_PATH=cook QUESTER_PATH_DIR=/absolute/path/to/paths \
  cargo run --locked --release -p panel --bin panel-play -- --profile local-289 --live script_quester_path
```

`script_quester_path` requires `QUESTER_PATH` (for example `cook`) and refuses
to choose a default quest. It reads embedded Paths by default. Set
`QUESTER_PATH_DIR` to an absolute folder to use the existing folder registry;
the requested Path must be served from that folder. `QUESTER_SEEDS` optionally
names a JSON file in the `quester_path_live` seed format (`stage`, `stand`,
`loadout`, `extra_items`, `seed_vars`, `before_relog`, `expect`, and `mode`).
`expect` and `mode` control the headless cell's settling; the headed scenario
always watches native quest completion.
When supplied, the seed file's `stand` selects the fixture's starting tile and
may differ from the Path's authored not-started anchor (for example, it may
start at a bank). Without a seed file, the fixture starts from a clean account
and teleports to the authored anchor after relog.
`QUESTER_PATH_DEADLINE_S` overrides the 45-minute deadline in seconds;
zero and non-integer values are rejected. A terminal window shot is captured
when the scenario completes.

Live runs mint fresh accounts named `live<token>_<i>` (at most 12
characters). Set `BOT_LIVE_NAME_PREFIX` to 1–4 characters (default `live`)
to replace `live`, for example `BOT_LIVE_NAME_PREFIX=tm`, so concurrent runs
can be told apart in engine logs and player saves. The engine rejects a name
it cannot register.

Match `--profile` and the engine ports to the revision (274: `:43594`/`:80`;
289: `:44594`/`:1080`).

The nav pack is format v17 (magic `274V`); `decode` rejects v16 and older as
`BadVersion`. A new executable does not rewrite an existing override pack, so
rebake override packs with `nav-pack` after updating.

The `nav_door` integration test uses hard-coded Direct local-274 options and
the `ENGINE_DIR/data/pack/client` cache path. `BOT_SERVER_PROFILE` does not
select that test's endpoint. It also accepts `BOT_NAV_DOOR_REVERSE_LOGIN=1`;
its closer then stays active while the driven player must reach the exact
destination:

```sh
BOT_NAV_REVISION=274 ENGINE_DIR=/absolute/path/to/274-engine LIVE=1 cargo test --locked --release -p e2e --test nav_door -- --ignored --test-threads=1
```

Direct integration-test helpers that use engine cache data require
`ENGINE_DIR`; frontend harnesses use the configured engine/cache path from
the resolved profile.

### Proof baselines

Catalog `stat_xp_gain` proofs capture all later cumulative skill baselines
immediately before `StartScript`, excluding preparation XP. A gain remains
observable even if another skill or an item is watched first.
`fresh_stat_xp_gain` is deliberately different: it arms only when its own
step begins, so an earlier trip cannot qualify a later bank-return phase.
Scenarios without `StartScript` retain first-watch cumulative baselines.

Compiled-card scenarios wait for Start to reach Running and fail immediately
with the recorded rejection or preparation error. Stashed cards survive setup
settlement: a scenario Stop revokes the script walk and bank selection, verifies
the slot is Idle, and resets the card for a new Start. Quester's Cook fresh,
resume, restart and login fixtures use this same compiled Start outcome gate.

### Quester journal fixtures

The headless Quester journal fixtures run with
`LIVE=1 cargo test -p host-play --lib live_quester_journal -- --ignored --nocapture --test-threads=1`.
Set an inline disposable `HOME`, `BOT_ENGINE_DIR` to the local R289 engine,
and `BOT_NAV_PACK` to a matching navigation pack; remove the disposable
directory after the command finishes. `BOT_GAME_PORT` and `BOT_HTTP_PORT`
override these fixtures' default local endpoints. The fixtures cover a
synthetic Rune Mysteries stage advance, parked ReadJournal retry, Stop/Start
and Pause/Resume with a retained journal page, and a DebugPanel `getcoord`
command sent while the journal is open. The overlap proof requires the host
chat-reply candidate and the journal's own fresh evidence and close sequence;
temporal debug replies are never command acknowledgements or journal progress.

The journal and fresh Cook fixtures mint their accounts like every other live
run, so simultaneous processes do not share an account and
`BOT_LIVE_NAME_PREFIX` tags an owner's fixtures. Keep the strict journal click
accounting: Stop/Pause recovery adopts the retained page without a new click.

### Headed checks

The live map has two headed panel cells on merlot against Engine A at
`127.0.0.1:44594` and `127.0.0.1:1080`. `panel.headed-nav-script` sets
`BOT_CPU=1` and covers CPU client paths. `panel.headed-render` removes
`BOT_CPU` to select the GPU renderer and covers GPU client and imgui paths.
Changes to the client login path also select both relog cells.

Run each cell in merlot's interactive display session with a throwaway HOME.
The CPU cell is not a substitute for the GPU cell. `--verify-cells` checks the
proxy test selector; it does not run either headed command or validate its
`--live` argument or host.

### Headed recording

Any panel session can record video of the panel window: an overnight
`--live script_<name>` run for next-morning review, or an interactive
session a headed check drives by hand. Recording reuses the terminal
shot's whole-window readback (no second screenshot path): the render pass
stages one extra capture per interval and a background `ffmpeg` process
encodes H.264:

```sh
QUESTER_PATH=cook HEADED_RECORD=1 \
  cargo run --locked --release -p panel --bin panel-play -- --profile local-289 --live script_quester_path
# or, interactively (no --live), under a throwaway HOME:
ISOHOME_INTERACTIVE=1 HEADED_RECORD=1 \
  .superpowers/bin/isohome ./target/release/panel-play --profile local-289 --port 1
```

It is off by default and costs nothing when off. `HEADED_RECORD=1`
(`true`/`yes`/`on` also work) enables it; `HEADED_RECORD_FPS` sets the
capture cadence (default 2, clamped to 0.1–10). Encoding runs off the
UI/slot threads through a bounded queue: a slow encoder drops a frame
(counted) instead of stalling the run. At the default cadence the
1280 px-wide `veryfast`/`crf 23` encode stays well under 500 MB per hour.
The file lands next to the terminal shot in the session's shot dir as
`record.mp4`, with a `record.txt` sidecar mapping each video timestamp to
its scenario step and runner game tick (`t=<s> frame=<n> step=<i>/<total>
tick=<ticks> name="<step>"`), so the video lines up with the step trace.
An interactive session has no harness shot dir of its own, so the
recorder creates one when it arms (`~/.274bot/smoke/<runId>/` in the
session's HOME) and names the scenario `interactive`. The recorder start
message in the host log names the shot dir. The mp4 is finalized on PASS,
FAIL and window close, so it is always playable. Resizing the window
mid-run drops the mismatched frames (counted) because the encoder pipe
has fixed geometry. `ffmpeg` is resolved from `HEADED_RECORD_FFMPEG`,
then `/opt/homebrew/bin/ffmpeg`, then `PATH`; when it is missing the run
logs one warning and continues without recording.

Next to `record.txt`, the recorder writes `settings.txt`: one line per
saved-setting state change, appended and flushed as the change happens,
so a headed check (for example the T5.3-S3 settings check) can verify
save states by script instead of reading frames:

```text
t=3.120 setting="capture" state=pending error=""
t=8.204 setting="capture" state=failed error="preferences is busy"
t=9.120 setting="capture" state=pending error=""
t=12.010 setting="capture" state=cleared error=""
t=12.180 setting="capture" state=saved error=""
```

`t` is the seconds since the recording armed. `setting` is a stable
identity from the setting owner. Draw preferences use their preference
key, such as `capture` or `sidecar_50`. Navigation preferences use a
separate `nav.*` key for each member. Other core rows keep their `what`
name, such as `danger routing`, `session file setting`, `map bake choice`,
`Quest Paths settings`, `background bots`, or `profile order`. Panel-owned
members use their dotted path.

The states are `pending` (an accepted write is in flight), `failed` (the
write failed, with the reason in `error`), `cleared` (the owner removed a
retained failure), and `saved` (the write became durable). A pending or
saved event does not imply that the owner cleared a failure. Repeated
reports of the same state write nothing.

If a bounded event queue overflows, the recorder writes this marker:

```text
# incomplete: one or more setting transitions were dropped from a bounded event queue
```

A checker must reject a recording that contains this marker. Verify a
check with, for example, `grep 'setting="capture"' settings.txt`.

### Catalog scenario notes

A scenario that waits for a card's clean stop (`wait_script_stop`) must
also watch the card's own work. Its post-Start watch and terminal proof must
be an outcome its pre-Start seed cannot already satisfy, or the run passes
on its Start snapshot and the 45-second stop grace becomes the only check.
The catalog test `clean_stop_scenarios_are_not_satisfied_by_their_own_seed`
builds each such scenario's seeded state (mainland landing, then its fixture
prerequisites) and fails any proof that already holds there. Cards that
only observe prove themselves with the receipt row they paint
(`script_receipt(prefix)`); cards that walk use `arrived_ring` around the
tile they start on.

The `ardy_cakes_fight` fixture waits, within its ordinary bounded scenario
step, for a nearby unengaged Guard with native scene line of sight before
starting the catalog script. It prepares Attack, Strength and Hitpoints 70
with the same adamant scimitar so back-to-back Guard fights do not consume
the entire stall watch. A ready Guard can still wander away: the Strength
watch tolerates one catch-less stall session, observes its bank visit,
closure and return, then requires renewed Guard readiness at the stall.
A second unqualified bank visit fails rather than admitting a third try.
The scenario has a 360-second wall deadline and a 900-game-tick combat
watch; the Thieving XP and exact Cake watches retain their original bounds.
All three still require real post-Start evidence; no catch is forced.
Script kill messages alone are not XP evidence:
in a crowded single-combat camp, a selected NPC can disappear after
another player kills it while the observing slot receives no XP.

### Core-gated qualification

A scenario PASS alone proves the scenario's own predicates. The shared core
witnesses in `host-play` (`catalog_core`, `paired_core`) prove the full
post-Start cycle. Both front ends run them through one gate,
`host_play::live_gate`:

```sh
cargo run --locked --release -p tui --bin tui-play -- --profile local-289 --live script_thiever --catalog-core
cargo run --locked --release -p tui --bin tui-play -- --profile local-289 --live script_flax_runner --pair-core
# environment form for harnesses that pass only environment:
BOT_LIVE_CORE=catalog cargo run --locked --release -p tui --bin tui-play -- --profile local-289 --live script_thiever
```

These are the panel's `catalog_watch` / `pair_watch` modes. The witness is
armed before either slot publishes, and its Start baseline is frozen
immediately before the actual isolate Start. While the witness is Pending,
a scenario PASS is held and the 45-second clean-stop grace has not started.
A failed witness, or one still unqualified at the scenario deadline
(`BUDGET_S` when set), fails the run. The compact witness receipt is printed
as `CATALOG_CORE: script_<name> {…}` / `PAIRED_CORE: script_<name> {…}`
next to the terminal line. The paired proofs (`nature_crafter_air`,
`mule_crafter_air`, `flax_runner`, `duel_arena`) refuse to run without the
pair gate.

The flax pair witness distinguishes an offer being staged from a completed
delivery. Offered flax has already left the runner's pack before the confirm
screen, so that inventory change alone is not a delivery receipt. A transient
offer-to-confirm modal gap can cross a player-update tick; continuation requires
fresh offer provenance, the same named counterpart and session, and matching
independent delivery observations from both actors. The content's explicit
stop-transmit handoff may advance each captured offer-container generation
once; an intervening content mutation or missing source is not that handoff.
Unrelated inventory updates cannot refresh an offer's provenance. Empty
counteroffer packets may precede modal-open and stay unchanged; the contributing
container must have fresh contents. A stale positive counteroffer is refused.
Replacement offers, refunds, stale observations and one-sided completion cannot qualify. The
bank/deposit/return and second-delivery predicates remain ordered after the
first proven delivery.

The frozen FlaxRunner omits the trade helper's pre-offer `baseline` callback.
Its confirm-time inventory baseline can therefore be zero both before and
after a successful delivery, producing the script log “trade closed without
delivering flax.” The pair receipt explains this callback limitation and
reports whether independent bilateral observations prove delivery; it does
not rewrite the script log, change the callback contract, or waive a real
non-delivery failure.

The TUI's flax-pair live receipt also prints `LIVE_TIMING`, separating
`setup_elapsed_ms` from `script_elapsed_ms` (null until shared Start is
dispatched). Flax Start waits for the spinner companion's observed relog,
fresh scene and inventory tab, and completed fixture preparation—not merely
an old ready snapshot. Setup has its own 120-second bound, clamped by the
original core deadline, and expires as a named setup failure. Login response
5 retains the host's normal backoff while the engine finishes logout; it
does not bypass the readiness barrier. The original 180-second flax core
ceiling is still armed once at live setup and is never reset at Start:
late readiness can therefore still fail with incomplete script work.

## Path-backed Quester qualification

New quest fixtures use `scenario::quester::quester_stage(QuesterStage { ... })`.
The older numeric `scenario::quester_stage` remains available to the shipped
S2, combat and WalkGuard cells; it does not apply the new qualification profile.

Add a `QuestFixtureProfile { quest, profile }` row to `FIXTURE_PROFILES` in
`crates/scenario/src/quester.rs`. `Base40` and `Base60` set Attack, Strength,
Defence, Hitpoints, Magic and Ranged to the named floor, and Prayer to 43.
The builder adds the Path's declared skill requirements, at their exact levels
for noncombat skills and without reducing a combat floor. The harder quests
(Dragon Slayer, Temple of Ikov, Underground Pass, Legends Quest, Elemental
Workshop, Horror from the Deep) already have Base60 rows. A profile is a
fixture plan, not an eligibility requirement or evidence of a successful
live run.

`QuesterStage` takes a scenario name, the selected quest-tab display,
the decoded `PathDocument`, its selected `QuestIdentityRow` and
`SelectedGameData`, a stage key, an optional loadout, extra carried items,
and the starting tile. Each stage needs a `progress.rules` varp hint;
missing or contradictory hints fail rather than becoming stage zero.

Paired quests use `quester_role_stage(request, gang, stage_varp, seed_vars)`.
It selects an explicitly authored Phoenix or Black Arm role, seeds that role's
real content variable, and keeps prerequisite variables in pre-Start setup.
`run_pair(PairCell { roles, mode })` prepares both accounts before either Start,
adds reciprocal account settings, and starts each account's own Quester. Role 0
is Phoenix; role 1 is Black Arm. Restart cells explicitly Stop and restart both
accounts; an ordinary paired step never starts or stops the other account.
Set `BOT_LIVE_NAME_PREFIX` for role 0 and `BOT_LIVE_PARTNER_NAME_PREFIX` for
role 1. Both names use the same invocation token; the token budget uses the
longer prefix so each account stays within the engine's 12-character limit.
Reserved-phase death cells use `PairMode::Death`: inject during the named
owned combat, observe cancellation, then explicitly Stop both accounts and
restart them without reseeding. Ordinary-role fights outside a reserved phase
use `PairMode::DeathIndependent`: the affected account recovers in its original
run, both finish with one Start and no Stop, and the peer must remain alive.
Miniquests use `miniquest_stage(MiniquestStage { ... })`, with explicit content
variable seeds and a proof predicate instead of an invented quest-tab identity.
Their terminal proof also requires the Path's owned progress reader to publish
completion; a closed card or successful click alone is not completion.

`FixtureLoadout::Path(name)` seeds that Path's actual kit.
`FixtureLoadout::Standard(StandardKit::{Melee, Magic, Ranged})` supplies the
standard starting kits. Gear never raises the profile to make it wearable:
an item refusal fails the fixture and is a real qualification limit.
Product Path loadout headers require selected config aliases. Fixture lookup
also accepts case-insensitive display names; this does not relax product header
validation. The builder emits only resolved config aliases in `give` commands
and merges carried kit/extras by item ID.

The returned `QuesterFixture` has independently owned `scenario`,
`start_settings`, `seed_commands` and a cloneable `seed`. Move `scenario`
into `ScenarioRunner`, and retain `seed` and `start_settings` in the live cell.
Use the returned settings for the compiled Start; they select only this quest.
The scenario clears carried and worn items, sets the profile and quest stage,
seeds and equips the chosen kit, then relogs and teleports to the authored
starting tile. Teleporting after relog prevents the mainland login hop from
overwriting the fixture origin. Its final pre-Start observation gate requires
the exact posted base/effective stats, every carried/worn seed and the exact
starting tile. It reads actual skill slots, including Agility, not the energy
proof. At Start, record `seed.observe_start(snapshot)` with the live receipt;
that receipt includes the observed tile. Only actual observations may be used
to write the Path's `tested_stats`.

All builder cheats occur before Start. After Start it only observes completion.
Sequence, Stop/restart and death assertions belong to the live cell. Additional
bank stock or auxiliary quest flags may be inserted into the pre-Start setup,
never into the gameplay proof. Shared world entities must remain real content.

## Gatherer live cells

Gatherer cells live in `crates/host-play/tests/gatherer_live.rs`. Each uses an
isolated `HOME` plus absolute `GATHERER_ENGINE_DIR`, `GATHERER_NAV_PACK` and
`GATHERER_CATALOG_ROOT`, and the `BOT_LIVE_NAME_PREFIX` namespace.

### Target switching

`gatherer_live::gatherer_mine_tier_power` supports a single selected ore through
`GATHERER_MINE_RESOURCES` and an explicit `GATHERER_MINE_TILE=x,z,level`.
Set `GATHERER_MINE_TILE` to the stand under test and `GATHERER_MINE_RESOURCES`
to one ore (`tin` or `copper`). The cell has no built-in tile.
The fixture requires real yield, two full disposal cycles, and renewed gathering.
It seeds one uncut sapphire before Start and requires observed disposal of that
gem. Naturally mined gems are reported as observations (`not_exercised` if none);
an incidental random drop is never required to pass. The fixed deadline remains.

The Gatherer runner reselects on the same observation that completes a depleted
or missing target, or a resource-approach walk. It still validates supplies and
equipment before the next action. An already usable loc stand permits one gather
click; targets requiring movement retain native navigation and its approach and
danger-zone rules. Bank, recovery, disposal, refusal and failed-walk settlement
remain fenced to a later tick. For fixed rocks and trees, terminal unavailability
is confirmed on the next observation so a delayed yield packet is accounted
before the runner blocks. An empty fishing actor view or expired fishing wait
renews observation while fewer than 800 ticks have elapsed since the last yield
or XP. Before the first yield, Start initializes this window on the session clock.
Walking does not renew this bound. At expiry, unavailable fishing blocks
with `no fishing spot observed for 800 ticks`. This exceeds the longest movement
timer (280 + 249 = 529 ticks, `skill_fishing/scripts/fishing_movement.rs2:8-12`)
plus the 60-tick whirlpool (`macro events/scripts/fishing/macro_event_whirlpool.rs2:7`).
Idle surveys and observation walks report `surveying` / `Waiting`, not gathering.

### Fishing bank returns

The ignored `gatherer_live::gatherer_fish_harpoon_bank` cell tests a fixed
Start location at Catherby with an inventory harpoon, a seeded `casket` (item
ID 405), a spare `net` (303), and a `beer` (1917). Its first bank receipt
requires all three incidental items to be deposited; the net and beer must
also be included in the native deposit candidates.
`gatherer_live::gatherer_fish_harpoon_bank_auto` cell exercises the operator's
Auto return settings: Fishing level 76, canonical method ID
`fishing.rarefish.op3`, location `Auto`, radius 40, Bank disposition, and the
Catherby bank. It defaults to start at tile `2840,3436,0`. Initial level/tool
fixtures precede the progression baseline; the Auto case has no seeded
inventory items, proof XP gains, or randomized fishing-location state.

Both cells require at least two positive fish deposits followed by a fresh
fishing yield after each bank return. Only the fixed-location cell requires the
seeded casket, spare net and beer in its first deposit receipt; the Auto case
has no incidental-item gate. Each trip receipt derives expected item and
product counts from the actual pre-deposit inventory, excluding protected
tools/supplies; it verifies deposited IDs leave the pack and their bank counts
increase while the bank is loaded. A naturally dropped casket is recorded only
as an observation, never required. The existing preparation and wedge deadlines
still bound both runs.

After banking, Gatherer returns to the selected method's content-derived
observation stand using area arrival with a one-tile radius. The work-area radius
remains the gathering bound; a bank inside that bound does not complete Return.
Area arrival settles only at a currently loaded, standable tile near the
observation stand, then Gatherer observes resources again.

The command below runs the matching ignored cells (the filter intentionally is not
`--exact`):

```sh
LIVE=1 BOT_CPU=1 BOT_NAV_BUILD=skip BOT_LIVE_NAME_PREFIX=gf \
cargo test -p host-play --test gatherer_live gatherer_fish_harpoon_bank \
  -- --ignored --nocapture --test-threads=1
```

Set a disposable `HOME` with copied `BOT_CACHE_DIR`/`CLIENT_UNPACK_DIR` inputs,
absolute `GATHERER_ENGINE_DIR`, `GATHERER_NAV_PACK`, and `GATHERER_CATALOG_ROOT`,
and matching explicit `WORLD_ENGINE_DIR`/`WORLD_NAV_PACK`. The defaults use the
builder engine at `45594`/`2080`; `GATHERER_GAME_PORT`/`GATHERER_HTTP_PORT` can
select the local engine at `44594`/`1080`. `GATHERER_FISH_HARPOON_TILE` optionally
overrides the default `2840,3436,0`. `LIVE_EVIDENCE_DIR` saves a real client
capture and a receipt containing the tick-stamped bank status/event sequence.

Fishing tools are carried, not wielded. In the Auto case, radius 40 bounds the
gathering area only; it is not a bank-return shortcut. After banking, the return
must reach a resource observation stand using navigation `Area` arrival radius
1 before another catch can demonstrate return progress. NPC view is smaller
than the loaded map, so returning inside the gathering area alone does not
prove that moving fishing spots are visible. For NPC-backed fishing, the
selector derives observation stands from the content movement envelope and
accumulates empty coverage for at most 100 gameplay ticks. It caps reapproaches
at eight per method/placement; exhaustion follows Avoided/Exhausted selection
instead of falsely declaring the spot absent. Positive gather progress renews
that budget. Empty coverage or expiry of an unavailable-resource wait also
renews it, but only within the 800-tick no-yield bound above. A live actor clears
empty evidence but does not itself renew the approach budget. A cooldown or hazard
in only the visible part of a movement envelope does not suppress observation of
the rest. Rock and tree placement observation still uses the loaded map rectangle.

### Catherby spot movement

The ignored `gatherer_live::gatherer_fish_catherby_spot_moves` cell uses the native
Gatherer with `fishing.saltfish.op1`, Site `fishing.catherby`, radius 32, and
Bank/Nearest. It gives the account a net, sets its stats, and teleports it near
the real shore before Start; it never spawns a fishing spot or a bank.
`GATHERER_FISH_LEVEL` overrides the default Fishing level 76.

The cell requires the same NPC instance to change tile at least twice, followed
by fresh fishing XP after each move, with no Gatherer failure. Its normal deadline
is 180 seconds for preparation plus 900 seconds for the movement witness.
`GATHERER_FISH_SPOT_SOAK=1` instead observes the entire fixed 1,980-second window
(180 seconds plus the existing 1,800-second power-cell allowance). Naturally
occurring random events can be recorded but are not required.

Receipts contain `fish_spot_moves` (instance index, old/new tiles, native
observation tick, XP baseline, resumed-fishing tick) and `fish_status_sequence`
(status changes with player tile and visible saltfish/hazard NPCs). Snapshot
ticks and native observation ticks are separate clocks. Use the same isolated
HOME, engine, navigation and evidence settings as the fishing bank-return cells:

```sh
LIVE=1 BOT_CPU=1 GATHERER_GAME_PORT=44594 GATHERER_HTTP_PORT=1080 \
  cargo test -p host-play --features live-probe --test gatherer_live \
  gatherer_fish_catherby_spot_moves -- --ignored --exact --nocapture
```


### Bank trip soak

`gatherer_live::gatherer_fish_bank_soak` runs one real net-fishing bot at the
Catherby site (the 0.2.0.2 user configuration: `fishing.saltfish.op1`, Site
location, Bank disposition, nearest bank) through back-to-back real bank trips
for a fixed `GATHERER_SOAK_SECS` (default 1800 s). The pack starts with 27 raw
shrimp and is refilled (`::give`, the account's own inventory) after every
return so each trip banks at once. A run that blocks is recorded (status,
failure and the last client-side bank facts: modal ids, the withdraw table's
inventory packet generations, run energy and auto-run sends) and restarted
with the same settings, as the user does. Every trip goes through the same
per-trip conservation receipt as the other bank cells: each deposit candidate
that leaves the pack (the catch and any incidental item, such as a random
event's Strange fruit) must reappear as a bank increase, the Gatherer's
`deposited` tally must equal the candidates it held, and the protected net
must stay; a verified receipt per trip is in `bank_trip.bank_trip_receipts`.
A recorded block abandons its trip's receipt (the block is that trip's
failure). The receipt's `soak` object carries trips, blocks by failure code
and the facts windows; the cell passes with at least `GATHERER_SOAK_MIN_TRIPS`
(default 10) trips and no `bank-deposit-failed` block. Run several copies
with distinct `BOT_LIVE_NAME_PREFIX` values for a rate; HOTFIX-0.2.1
reproduced both shipped deposit blocks with ten bots in thirty minutes.

`GATHERER_SOAK_TWO_PRODUCTS=1` is a measurement mode, not qualification: the
pack is seeded with raw shrimp and raw anchovies so every trip deposits in
two clicks, about doubling the transfer window a run-energy threshold
crossing can land in. The host's `run_on` latch clears only at energy 0
(`host/src/lib.rs` `apply_auto_run`), so the crossing cannot be forced from
the account's own state; the mode raises the rate at which a recorded
crossing falls inside a transfer. Each host press is logged with the modal
roots it saw (`auto-run press energy= main= chat= side=`), so a receipt
shows a press under the bank followed by the stop-transmit/close order and
the kernel's `bank-reopen` / `bank-reopen-reconcile` lines, or the 0.2.1
host's deferral of the press until the bank closes.

## Walk Guard live cells and receipt replay

The ignored W1 cells (`--lib`, module `walk_guard_live_tests`) exercise the
`death-plateau-throwers` crossing against the local 289 engine. The gate
retains the ranged-projectile queue rule `floor((32 + 5d) / 30)` without
relaxing it; at distance 1 the expected delay is one tick. The first thrower
is staged on a standable tile three to five tiles ahead of the route start:
the fixture teleports there, spawns the NPC, then teleports back to the route
start before scripted movement begins. Unlike those cells, this fixture stages
throwers with engine teleports and `npcadd` after Start and before scripted
movement. That staging is part of the cell, not a product cheat. This avoids
attributing a distance-zero launch at the start to the moving crossing.

The live and replay gate requires exactly two accepted guard clicks, both
component 5622. The enable click must show Missiles off before the click and
occur from the first launch through arrival; the off-click must show Missiles
on before the click and occur from arrival through the observed off tick.
Every per-tick crossing observation from `protect_tick` through
`arrival_tick`, inclusive, must show Missiles on. The receipt must also have
an observation for every tick from the first launch through the off tick and
three following ticks, with all prayers remaining off during cleanup. Any
extra click, non-Missiles click, missing crossing observation, Missiles lapse,
or prayer return fails the same gate used by offline replay.

Arrival uses the host's network tile (`Snapshot.tile`, derived from `route[0]`),
not the rendered actor's interpolated pixel tile. `last_tile` and
`final_snapshot.tile` use the rendered local-player tile consistently;
`final_snapshot.network_tile` retains the separate host coordinate.
If crossing completion and prayer cleanup are observed on the same game tick,
the pre-cleanup off-click snapshot supplies the genuine Missiles-on crossing
observation only when its network tile confirms arrival at the destination;
the post-click frame is a separate cleanup observation at that tick. Receipts
label those phases explicitly; they do not invent a game tick or waive the
inclusive arrival requirement.

| Cell | What it proves |
| --- | --- |
| `live_walk_guard_w1_protected_crossing` | The full gate above over one crossing. |
| `live_walk_guard_w1_stop_mid_crossing` | The real `ScriptStartHandle::stop` API after Missiles is observed on during the crossing and before arrival: every prayer turns off within four ticks of Stop and stays off for three more observed ticks. |
| `live_walk_guard_eat_crossing` | A Prayer 37, HP 20/40 account with four lobsters crosses and eats: the guard's Eat click is followed within four ticks by an HP rise and one lobster consumed. Name prefix `ge`, endpoints `44594`/`1080`, and an isolated `HOME` you set yourself. |
| `live_walk_guard_eat_no_food_control` | The same seed with no lobsters crosses without an Eat. |

The two W1 cells connect through the forwarded builder ports `45594`/`2080`, set
their own `BOT_LIVE_NAME_PREFIX` (`wg`) and create a throwaway `HOME` under
`$LIVE_EVIDENCE_DIR/homes/`:

```sh
LIVE=1 BOT_CPU=1 BOT_NAV_BUILD=skip LIVE_EVIDENCE_DIR=<evidence-root> \
  BOT_ENGINE_DIR=<engine-dir> BOT_NAV_PACK=<274bot.navpack> \
  BOT_COMBAT_CACHE_SNAPSHOT=<retained-cache-snapshot> \
  cargo test -p host-play --lib walk_guard_live_tests::live_walk_guard_w1_protected_crossing \
    -- --exact --ignored --nocapture --test-threads=1
```

Replace the cell name to run the others. Every run saves a real final scene
PNG with a matching JSON/Core record in a per-run capture folder under
`$LIVE_EVIDENCE_DIR`; inspect that image alongside the receipt before
accepting the live proof. The final receipt's `tile` is normalized to
`local_player.tile`. An explicit navigation pack must have its
`.navpack.json`, `.navreach`, `.navflags`, `.navcanlight` and `.navpois` files
beside it.

The ignored offline replay `replay_walk_guard_w1_retained_receipts` applies
the same gate to three retained receipts (one bad, two passing) read from
`$LIVE_EVIDENCE_DIR`, skips with a message when the directory is unset or a
receipt is absent, and writes `$LIVE_EVIDENCE_DIR/W1-lifecycle-replay.json`.
Legacy receipts without per-tick rows use their guard-click pre-click prayer
varps, recorded protect/off observations, and holds between clicks; the
replay output marks that their crossing frames and three post-off ticks were
not directly recorded and does not fabricate per-tick rows:

```sh
LIVE_EVIDENCE_DIR=<evidence-root> \
cargo test -p host-play --lib walk_guard_live_tests::replay_walk_guard_w1_retained_receipts \
  -- --exact --ignored --nocapture
```

## Survival fruit cells

Run these ignored `host-play` library tests with `live-harness,test-support`
against Engine A at 600 ms:

- `survival_eat_live_tests::live_survival_strange_fruit_after_bite`
- `survival_eat_live_tests::live_survival_strange_fruit_through_api_combat_fight`

Use an isolated HOME, a unique `BOT_LIVE_NAME_PREFIX` of one to four characters,
and the accepted revision-289 cache and nav pack. Set `LIVE=1`, `BOT_CPU=1`,
`BOT_NAV_BUILD=skip`, and an absolute `LIVE_EVIDENCE_DIR`.
Both cells seed only their account. The combat cell attacks real Cows.
Each cell enables poison cure after the first health bite. It requires exactly
one Strange fruit click at the first content-allowed tick, T+3, and an observed
cure. The combat cell also requires decoded wire actions from the same fight.

The harness reads captured actions on every sample, including repeated ticks.
A late policy change produces a classified fixture failure. The receipt is
written before the final assertion. Receipts include `commit` and
`worktree_dirty`, so an uncommitted mutation is not reported as clean source.

## JS API v2 checks

### Declarations and sample

The `host_js` integration test renders `crates/script/host-js/index.d.ts` from
the Rust API tables. Regenerate it with:

```sh
cargo test -p script --test host_js regen_host_js -- --ignored
```

The pinned TypeScript 5.8.3 consumer probes check the generated declarations,
including the `gather_quest_v2.ts` sample. Its probe checks with and without
`--strict`, emits JavaScript, and requires the checked-in
`gather_quest_v2.js` to match that emit:

```sh
cargo test -p script --test host_js -- --ignored tsc
```

### Live cells

The ignored `script_api_live` tests load the checked-in `gather_quest_v2.js`
through the real Load path and exercise it against a local R289 server. The
engine and game/cache endpoints must be reachable on `127.0.0.1:45594` and
`127.0.0.1:2080`. Provide a matching `WORLD_ENGINE_DIR` and `WORLD_NAV_PACK`,
and set `BOT_CACHE_DIR` to a copied client cache.

Run from the repository root with `LIVE=1`. The `HOME` used by the test must be
a disposable directory under `BOT_EVIDENCE_DIR`; the test writes receipts
there. `BOT_LIVE_NAME_PREFIX` may be set to distinguish the minted test
account from other live runs.

```sh
LIVE=1 cargo test -p host-play --features test-support --test script_api_live -- --ignored --test-threads=1
```

`script_api_gather_and_quest` records the running/gathering phases, confirmed
yields and emptied drop slots, the Driver packet budget, the stopped session
envelope, and Cook's colour-only progress row. Its receipt is
`script-api-gather-and-quest-receipt.json` in `BOT_EVIDENCE_DIR`.

`script_api_progress_journal` uses the released Romeo & Juliet journal rules.
It seeds stage 30, reads progress, advances the fixture to stage 40 and reads
again. It requires known stage/rule keys, `inProgress`, `complete: 'false'`,
hidden journal paint while the quiet-paint lease is held, and public evidence
matching the real closed-journal observation after the acquired observation.
Its receipt, `script-api-progress-journal-receipt.json`, contains both reads,
their host evidence stamps and the journal open/close packet trace.

## Fleet memory harness

Both frontends expose the same opt-in `host_play::memory::{Config, Run, Sample}`
harness. Build with `memory-profile` for allocation counters, or
`memory-profile-no-alloc` to use the system allocator directly. The latter leaves
allocator fields null while retaining real CPU, RSS, V8 and tracked GPU samples.
Unset `BOT_MEMORY_N` for normal interactive use.

```sh
cargo build --locked --release -p tui -p panel --features memory-profile-no-alloc

LIVE=1 \
BOT_MEMORY_N=1 BOT_MEMORY_WORKLOAD=active BOT_MEMORY_SUSTAIN=1 \
BOT_MEMORY_WARMUP_S=30 BOT_MEMORY_OBSERVE_S=240 \
BOT_MEMORY_OUTPUT=/absolute/new-run/samples.jsonl \
target/release/tui-play --profile local-289
```

Use a real terminal for `tui-play`. Substitute `panel-play` for a native window.
Create a fresh output directory first. The harness creates isolated accounts and
an ephemeral encrypted vault; sustained Thiever preparation supplies its own food
loadout and bank stock. It does not read the operator's saved loadouts. Seed
runners share the Play navigation `Arc`; if Play has no pack, they do not silently
load another copy.

| Setting | Meaning |
|---|---|
| `BOT_MEMORY_N` | Exactly 1, 10, 16, 32, 50 or 128; accepting a count is not a capacity guarantee. |
| `BOT_MEMORY_WORKLOAD` | `idle`, `seeded-idle`, `active` or `lifecycle`; default `idle`. |
| `BOT_MEMORY_SCENARIO` | Scenario for `active`/`lifecycle`; default `thiever`. `gatherer` uses the Draynor tree fixture; its multi-slot cells start the native Gatherer. Catalog cells include `moss_giant_bank_start` and `duel_arena`. |
| `BOT_MEMORY_WARMUP_S` / `BOT_MEMORY_OBSERVE_S` | Positive seconds, defaults 120 / 600. |
| `BOT_MEMORY_TEARDOWN_S` | Positive teardown seconds after scripts Stop; default 60. |
| `BOT_MEMORY_SUSTAIN=1` | Active/lifecycle Thiever only: four food initially, stock in the bank, target 22 food and restocking at three. Other scenarios reject this setting. |
| `BOT_MEMORY_DIAGNOSTICS=1` | Optional bounded frames, logs, requests and per-script progress sidecar. |
| `BOT_MEMORY_RENDER_POLICY` | Panel `rotating-all`, `fixed-one`, `focused-one`, `focused-plus-background`, `stress50` or `stress50-full`. `stress50` draws one of 50 slots; `stress50-full` draws all 50 at full rate. |
| `BOT_MEMORY_SINGLE_RENDERER=1` | Legacy fixed slot-zero rendering. |
| `BOT_CPU=1` | CPU fallback for the panel's client renderer. |

`idle` only logs clients in. `seeded-idle` performs the ordinary Thiever seed
without starting the catalog script. `active` starts the scenario's native or
catalog script. `lifecycle` additionally alternates Stop and restart every 60
observation seconds. After observation, scripts Stop and the harness remains
open for `BOT_MEMORY_TEARDOWN_S`.

Ingame readiness requires every slot to remain `ingame && scene_state == 2`
for two seconds. A ready pulse before the mainland-hop reload does not latch.
Samples report `all_bots_ingame_scene2_s` separately from
`qualification_complete_s`. Active workloads also require seed or proof
completion and running scripts.

The phase gate applies to idle and Thiever workloads. Idle requires every slot
to remain in scene 2 without a started script. Its tick baseline starts with the
existing two-second readiness hold. Seeded-idle and Thiever require the initial
Lobster pack and a closed bank. Seeded-idle and sustained Thiever start with
four Lobsters. Ordinary Thiever starts with ten.

Each bot must observe the configured warmup divided by 600 ms, rounded down, in
`PLAYER_INFO` ticks. For example, 10 seconds requires 16 ticks. Five seconds
requires 8 ticks. The configured wall-clock warmup still controls the window
start after every bot reaches the tick floor.

An idle script start fails with
`phase_missed:idle_script_started:<name>:tick=N`. A Thiever bank opening, food
count at or below three, or food count above its initial value fails with
`phase_missed:thiever_first_bank:<name>:tick=N`. Another initial-food change
fails with `phase_missed:thiever_initial_food_changed:<name>:tick=N`.
A gated run that does not reach its phase fails with
`phase_not_reached:qualification_timeout`. The observe-start check reports
`phase_not_reached:observe_start`.

`moss_giant_bank_start`, `duel_arena` and `gatherer` report `phase not gated`.
Their `phase_gated` field is false. MossGiant records its Lobster count without
a gate. A workload with no food records `null`.

Multi-slot `moss_giant_bank_start` replaces only its contended post-return
fresh-XP checkpoint with fail-closed local-player named-NPC engagement. The
scenario's final Strength-XP-since-Start proof remains mandatory.

Multi-slot `gatherer` uses a fleet qualification for the contended Draynor
normal trees. Every bot must reach a safe legal tree stand and send a real
`Chop down` request within its unchanged post-Start tick budget. No sampled
position may enter a denied zone. A death, native failure or final `Blocked`
phase fails the cell. The fleet must gain Woodcutting XP. A bot with no XP must
also show a genuine resource-exhaustion wait. Generic `Waiting`, idle and paused
states do not supply that evidence. The qualifier collects its evidence even
when the optional diagnostic sidecar is disabled.

During an N10 planned lifecycle restart, safety checks continue through the
harness's Stop and Start. Only that Stop arms the restart allowance. The
admitted Start allows `Starting` for at most three observed `PLAYER_INFO`
ticks, then requires `Running`. Every slot reached `Running` 0.43-0.93 seconds
after Start in the N10 measurements. This measured qualification bound is
not a scheduler guarantee. Gatherer lifecycle cells with any other N fail
at preparation with `restart_bound_unmeasured`. Active cells retain their
existing fleet-size support.

The warm start path has four worker dispatches per slot: Load, Configure,
Construct and Seal (`script/src/preparation.rs` and `slot/compiled.rs`).
All slots queue in the same harness tick on one shared FIFO `work-admission`
worker. Each dispatch has a soft 10 ms wall-clock slice
(`script/src/work_admission.rs::WORK_SLICE`), not one game tick.
A yielded job returns to the queue's back. Full admission defers submission
until `poll_compiled` retries it. The next lifecycle observe adopts completed
work. These facts do not establish a three-tick bound for an unmeasured N.

An unplanned `Starting`, including one between Stop and Start, fails the cell.
A restart overrun or an unfinished restart at observation end also fails.
The harness does not start another lifecycle cycle at observation end.
A planned restart never resets the original 300-tick proof window.
If observation ends inside a planned Stop, the `observe-end` row records
`verdict: "pass"` only after all other qualification checks succeed.
It also records `lifecycle_state: "planned_stop"` and each slot's actual
`run_state`. During the Stop interval, the ongoing verdict remains `pending`.
The test `observe_end_inside_planned_stop_records_pass_and_actual_slot_states`
covers this boundary and rejects missing evidence and an unplanned Start.

The diagnostic record
`gatherer_fleet_qualification.bots[].first_missing_position` retains the first
missing sample's tick, reason, run state, ingame flag, scene state, raw tile
and planned-restart state. Its reason is `coordinates_missing`,
`geometry_missing`, `geometry_error`, `zone_data_missing` or
`zone_filter_missing`. The test
`first_missing_position_records_first_context_without_changing_verdict`
checks the retained context and unchanged missing-evidence verdict.
This record is available without `BOT_MEMORY_DIAGNOSTICS=1`.

The single-bot `gatherer` cell still requires fresh Woodcutting XP. Other
Gatherer live cells retain their individual gates. They do not use this fleet's
shared-tree fixture.

`duel_arena` is the pair-script cell: every slot runs the frozen Duel Arena
Combat Trainer, so N=10 is five duels and N=50 is twenty-five, and an odd N is
refused at start. The fixture seeds a bronze scimitar (the existing
`duel_arena` pair seed) and stages mint-order pairs so that at Start the only
free challenge-area candidate is the partner. Unstarted slots wait at the Al
Kharid bank hold, outside the lobby. A finished pair is parked back at the hold
before the next pair is admitted, so an already-fought slot is not a lobby
candidate on the frame the completed-duel watch advances and the parking tele
has not yet been sent (`ScenarioRunner` only `advance_step()`s that tick;
`play_slots` then runs `slot_frame`).

Admission has one owner with the login FIFO's shape: one mutex, slot owner
tokens, a FIFO of Start requests, a grant, and a slot-worker retirement guard.
Every slot thread's per-frame position report, every admission and Start
decision, every breach and every fence transition happen under that one lock,
so no decision reads a half-updated fleet. The benchmark poll Starts a slot only
on a granted permit, and a grant needs every slot outside the pair to have
reported a clear position (at least 20 steps from `DUEL_ZONE`) with an epoch
newer than the request. A clear reading from before the request, left by a
stalled or delayed slot thread, cannot admit a Start; the request waits for that
thread's next report. A slot worker that exits or panics while the fence is up
fails the run instead of stalling the grant.

The frozen card walks a parked slot back toward the arena. The fixture
re-teleports it once it is 8 tiles from the hold, but that tele is neither
acknowledged nor sent while the scene settles, so it is not the guarantee. The
zone-clearance fence is: every in-scene client frame of a waiting or parked
slot reports its tile, not gated by scene settling, and the first such tile
closer than 20 steps to the challenge area records `duel fleet clearance
breach: slot … was seen at …` under the admission lock. After it no permit is
granted and the next benchmark poll fails the run with it. The fence stays up
after every pair has entered a pen: it comes down only once the last pair has
parked back at the hold and every slot has reported a clear position since, so a
late report from near the arena is a breach rather than a release. Parked slots
are then released to the lobby so observation is a fighting fleet.

A step moves at most one tile on each axis and a running player takes two steps
per game tick, so a slot last seen outside the clearance is at least 10 game
ticks (6 s) from becoming a candidate, and its thread reports it on the way. The
hold is 59 steps from the challenge area, which leaves room for the correction's
own excursion (22 tiles from the hold at most in the live N=50 runs). The fence
bounds distance to the arena, not to the hold: parked slots have landed at the
Lumbridge spawn, 106 steps out, and are simply re-teleported. What remains is
that a grant proves every other slot was clear at a moment after the request,
not at the instant the started card evaluates `challengeTargets()`: a
fought slot moved straight into `DUEL_ZONE` after a grant and before its next
report would be a candidate first and a breach (the run fails) second. The
fixture has no such mover: its corrective teles target the hold, observed
respawns land 106+ steps out, and the card walks at least 10 ticks through the
fence. The qualification records carry a `duel_gate` object with the
clearance, the closest approach and the farthest hold distance seen for a
fenced slot, the fence state, the permits granted, any pending Start and the
slots it still awaits a clear report from, and the fleet failure if any. Each
slot qualifies only after its own snapshots show a fight pen visit followed by a
return to the lobby, the transition the card counts as a finished duel. A duel
only moves its two fighters (no stakes, HP restored, no spawned or dropped world
state), so a cell leaves nothing for the next one.

Seed/proof or script failures fail the run and return a nonzero exit, and so does
a fleet that has not qualified after 30 minutes (`blocked: ready=… seeded=…
proved=… wanted=… ever_ready=…`). A `duel_arena` fleet stages one pair at a time,
so that bound is 60 minutes. Every such failure first appends one
`"phase":"failed"` record to `samples.qualification.jsonl` with the error and,
for every slot, whether it ever reached `ingame && scene_state == 2` (and when),
its last observed session state (startup phase and how long it has been in it,
login-queue place, error, worker terminal, latch, welcome hold), its scenario
step by name, and its script state and error. `qualified_now` says whether the
slot satisfies the run's own qualification definition at that instant (for an
active workload: in game, proof passed and script running), so a ready slot
whose script has stopped is not reported qualified; `reached_ingame_scene2` is
history. The observation-boundary records carry the same per-slot fields.
Inspect them before treating a run as a
benchmark: a process that exits normally is not by itself evidence that every
bot did useful work for the whole interval.

## Reading results

`samples.jsonl` separates current resident bytes from process-lifetime peak RSS.
It also records cumulative process CPU, optional allocator counts, V8 sample
coverage/age, snapshot capacity and tracked GPU buffer/texture bytes. Host
mainloop/observe/raster fields are cumulative **wall-time** counters, including
waits, preemption and contention; they are not CPU time. Retired slot counters
remain in the per-username process totals across relogs, while
`host_profile_slots` counts only active slots.

Sample JSON serialization is buffered to avoid a file write for every JSON
token, including on shared VM volumes. Each complete row is explicitly flushed
before the sample poll returns, so the frontend's immediate process exit does
not leave completed rows in a userspace buffer. This does not change the sample
fields or exclude sample-writing work from the whole-panel frame timer.

Panel samples include a cumulative one-millisecond frame histogram whose final
bucket means “at least 250 ms”, cumulative `ui_frame_count` and
`ui_frame_total_ns`, and a swap-reset `ui_frame_max_ns` for that sample
interval. A receipt must label overflow percentiles as censored and report
overflow count, observation-window mean and the maximum of the interval maxima
rather than presenting 250 ms as an exact slow-frame duration. Requested
rendering metadata does not prove the observed backend or cadence.

Every sample row also carries run metadata, each field a JSON `null` when it does
not apply (a headless `tui-play` has no adapter or panel frames):

- `adapter`: the wgpu adapter the panel selected (`name`, `backend`,
  `device_type`, `driver`, `driver_info`, `vendor`, `device`, `pci_bus_id`), with
  `adapter_selections` counting how many times the panel selected one (more than
  one means the GPU stack was rebuilt). It is recorded once, in the run metadata,
  without `BOT_DEBUG` and without a log line.
- `game_data`: the profile's generated-facts trust decision with its reason:
  `attached`, or withheld because the selection is `unsupported-server`, the
  cache does not match the generated asset (`cache-identity`), or a pinned
  generator source failed verification (`source-rejected`, with how many of the
  inputs failed and the first failures by root, pinned path and kind). Facts are
  withheld exactly as before; only the cause is recorded, and the same cause is
  logged once at warn level when a profile is bound for play.
- Startup timeline, in seconds from the process entry the frontend `main`
  recorded: `startup_window_created_s`, `startup_gpu_ready_s`,
  `startup_first_render_s`, `startup_first_frame_presented_s` (the first
  presented panel frame), `startup_first_client_frame_s`, and
  `run_started_process_s` (when this harness's own timer began), all placed
  against launch by `process_epoch_unix_ms`. `render_gap_*` times the space
  between panel render callbacks, which the frame histogram does not: the longest
  gap since the process began (`render_gap_max_ns`, ended at
  `render_gap_max_at_s`), the longest in this sample interval
  (`render_gap_interval_max_ns`, swap-reset), and how many gaps reached five
  seconds (`render_gaps_ge_5s`), the point at which an OS “Not Responding” state
  can begin. A large gap shows the UI thread was elsewhere; it does not by itself
  prove the OS marked the window unresponsive.
- `ready_ever`: how many slots have ever been `ingame && scene_state == 2`,
  against `ready`, how many are now. Active workloads also carry `seeded`
  (scenario runners that passed, or sit at the Start step waiting for the script
  to load) and `proved` (runners that passed their final proof), so a fleet that
  stalls shows when `proved` stopped rising; both are `null` for idle workloads.

`samples.qualification.jsonl` records observation start, observation end and
failed-fleet rows. Each row has `gameplay_phase_rule`, `phase_gated` and
`required_warmup_game_ticks`. Each slot has a `gameplay_phase` object with
`gated`, `rule`, `phase`, `observed_game_tick`, `warmup_start_game_tick`,
`warmup_game_ticks`, `food`, `bank_open`, `tile` and `first_phase_miss`.
The MossGiant `observe-start` row records its Lobster count in `food`. A
workload without food records `null`. `moss_giant_bank_start`, `duel_arena`
and `gatherer` report `phase_gated: false`.

`memory.rs` is compiled into each measured binary. A commit without this code
does not apply the phase gate. A gated record contains the phase fields above.
An older FLOOR baseline lacks them. The candidate is the only gated side when
it contains this code and the baseline does not. If one record has the phase
fields and the other does not, the binary with fields used this code. Compare
results only when both commits contain the change.

With optional diagnostics, `samples.diagnostics.jsonl` adds bounded per-slot
evidence.

The diagnostics sidecar's `recent_navigation` is always null: detailed
navigation history, direct-owner census, scheduling/latency journals, managed
process controllers and campaign replay/provisioning tools are not part of
this harness.

Compare matched workloads, platforms and intervals. Allocation avoidance is not
necessarily an RSS reduction, and current RSS must not be confused with peak RSS
or allocator counts. Negative or non-monotonic RSS slopes can be sampler noise,
especially with macOS `resident_size`; they are measurements, not savings
claims. Allocation-focused changes do not establish an additive total saving,
universal responsiveness improvement, low-end budget, or 128-client capacity
guarantee.

## Known validation limits

The revision-specific stun recovery infers an onset from spot animation 245 and
waits eleven distinct player-update ticks, at most once per follow run. It is not
a universal stunned flag. Seeing an ordinary thieving stun does not prove the
navigation recovery path ran.

Runner completion is not script qualification. Prefer capability language over
copying rapidly changing pass/fail inventory counts into product docs.
