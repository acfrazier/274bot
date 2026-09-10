# Common main/selected comparison recipe

Read-only source assessment, September 10, 2026. No build, live launch, product
edit, or benchmark was performed for this recipe.

## Recommendation

Use the same small, untracked integration-test driver in both complete host/client
checkouts. Put its source in campaign evidence and copy it temporarily to
`crates/tui/tests/selective_compare.rs` in each checkout. The `tui` package already
has every needed dependency on main: host-play, api, client, scenario, script,
serde_json, vault. This avoids a manifest change or backport of memory Run.

The baseline must be host `54cfcf8` and its pinned client
`4f2048ea10f75b3bb92ff45610b35ba7313b0308`; selected is host `9527cc63` and its
actual pinned selected client. Record both full SHAs, the common test SHA256,
Cargo feature sets and executable SHA256. A selected host with the old client is
an isolated client experiment, not the requested main baseline.

## Why the existing commands alone are insufficient

Main's `crates/host-play/tests/rss_ladder.rs` accepts only N=1,2,4, sets draw off,
waits up to 180 seconds for scene 2, then logs peak RSS after ten seconds idle.
It has no active script or player-update progress qualification. It is a useful
foundation, but its unchanged output cannot answer N32 or active timing.

Main's `tui-play --live script_thiever` already loads the real catalog, applies
scenario settings and starts the script after seeding. However, it has one
runner and one PendingCatalogStart, targeting names[0]. Increasing the seed
profile count produces extra clients, not 32 active Thievers.

The unmodified one-slot correctness smoke is available on both:

```sh
LIVE=1 RS2B0T=/absolute/pinned/catalog BOT_TARGET=local cargo run --release -p tui --bin tui-play -- --live script_thiever --host 127.0.0.1 --port 43594 --cache /absolute/client-cache
```

Keep BUDGET_S unset: the ordinary gold deadline remains 180 seconds. This smoke
is a TUI functional check and terminates on proof; it is not a matched sustained
CPU/RSS result by itself.

## Minimal common driver delta

The proposed integration test combines existing patterns from rss_ladder and
TUI live preparation; it adds no production API or replacement script policy.

1. Gate on LIVE=1, BOT_TARGET=local and a loopback host. Require comparison N to
   be exactly 1 or 32. Reuse mint_live_names/mint_live_entries, target-aware
   temporary vault preparation and PlayOptions from existing live code. Use the
   same cache, lowmem=true, Raster Off/no renderer and local engine on both.
2. Call run_with_io with no initial profiles; the existing callback sets draw
   false and records a tiny per-slot row: observe count, changes to gens.player,
   last player-update Instant, current stat_xp[17], readiness and tile. These
   fields already exist on main. No owned world/snapshot clone per callback,
   no script probe and no new timing journal. Bind the existing
   ScriptStartHandle and per-slot runner map before spawning all N profiles.
3. Require exact username membership, N distinct profiles and N statuses; every
   slot must be ingame && scene_state==2. Keep rss_ladder's 180-second readiness
   bound. Require the original one shared OnDemand worker; retain its TCP check
   where the existing platform sampler is available. A missing TCP result is
   unavailable, not zero.
4. For `idle`, leave scripts unstarted. Begin a fixed 60-second observation only
   after all-ready; require positive distinct PLAYER_INFO generation changes in
   every slot during the observation. Readiness must remain true. Any lost slot
   or stopped player updates fails qualification.
5. For `thiever`, create one unchanged `scenario::get("thiever")` runner per
   slot after initial all-ready, then tick it from that slot's observe hook using
   tick_with_hold. Preserve the original 180-second scenario deadline, 150-tick
   XP watch, seed actions (10 lobsters, thieving/hitpoints 50), dialog handling,
   600 ms engine tick setting, and terminal evidence. Do not call set_deadline
   or substitute the selected sustained scenario on main.
6. Build the real catalog Thiever once with the same JsLibrary/ensure_js and
   resolve_sibling_modules code used by TUI. Pin identical catalog bytes. Merge
   the unchanged scenario settings inject (Guard, banking=None, loot empty).
   Each runner's on_start_script starts its own isolate once through the existing
   ScriptStartHandle::start_load and only then allows the StartScript step to
   tick. Failure to start is a failure, not a skipped wait. Preserve guardian
   hold behavior.
7. Use ScenarioRunner::with_world(..., play.world()) on both sides; both APIs
   already exist on main. This supplies an identical common driver and avoids
   inventing 32 navigation decodes in the baseline. Consequently this comparison
   does NOT measure the selected Run seed-sharing optimization. Set obj_names
   and the runner's single live name with the existing setters on both sides.
8. For reproducible ordinary loadouts, set the same temporary HOME fixture and
   loadouts.json on both; do not call selected-only post_loadouts or patch main's
   loadout shim. Main's selectedLoadout returns null; this real product difference
   may prevent useful active behavior. Capture it honestly. Do not inject food
   or loadout globals into main to manufacture a performance baseline.
9. Require all N original runners Passed before starting the same 60-second
   active observation. Snapshot each slot's thieving XP at that boundary. During
   observation require exact N readiness, script state Running, no script error,
   new player updates and per-slot XP growth over the observation. Retain per-slot
   start/end rows and terminal evidence; process totals can hide idle bots.
   If any condition fails, label the pair correctness-only and exclude its
   active CPU/RSS comparison. Do not extend a timeout or silently reduce N.
10. Emit only phase markers and one compact row per slot at each phase boundary
    (PID, N, ready names, callback/player-update counts, XP, script state/error,
    start/end monotonic times). Stop every script and slot using existing APIs,
    then assert statuses empty. Keep these checks common to both versions.

This is one test-only adapter, not a new retained benchmark subsystem. No selected
routing, food, withdrawal, loadout, snapshot, renderer or memory hooks should be
copied into main. If implementation needs more than the APIs above, stop and
review that dependency before expanding the driver.

## Concrete run matrix after the common test is authored

Build once in each separate checkout; no memory-profile features on either:

```sh
cargo test --release -p tui --test selective_compare --no-run
```

Run the exact produced test executable directly, one fresh process per cell,
with the same external process sampler attached to its printed PID:

```sh
LIVE=1 BOT_TARGET=local COMPARE_N=1 COMPARE_WORKLOAD=idle /absolute/test-executable --ignored --exact common_session --nocapture --test-threads=1
LIVE=1 BOT_TARGET=local COMPARE_N=32 COMPARE_WORKLOAD=idle /absolute/test-executable --ignored --exact common_session --nocapture --test-threads=1
LIVE=1 BOT_TARGET=local COMPARE_N=1 COMPARE_WORKLOAD=thiever /absolute/test-executable --ignored --exact common_session --nocapture --test-threads=1
LIVE=1 BOT_TARGET=local COMPARE_N=32 COMPARE_WORKLOAD=thiever /absolute/test-executable --ignored --exact common_session --nocapture --test-threads=1
```

COMPARE_* and common_session above are the proposed test-only adapter interface,
not currently shipped commands. The driver source does not yet exist. Root owns
baseline checkout creation and final build/run authorization.

Run main then selected sequentially against the same engine revision/config and
fixed cache/nav pack/catalog. Use fresh equivalently seeded accounts, and stop
all previous sessions first. Collect external process CPU-time deltas and current
RSS through the same sampler for both; align exclusively to observation markers,
not process launch. Distinguish current RSS from lifetime peak. Report CPU seconds
per observation second and player-update throughput alongside raw current RSS;
never count compilation/Cargo CPU or equate allocator accounting with RSS.

## What the result can resolve

A qualified active N32 pair provides a bounded practical check of combined
CPU/current-RSS behavior and whether every bot continues receiving player updates
and gaining XP. Callback/player-update counts expose a speedup obtained merely
by doing less work. The idle pair is useful for ordinary client cost, but cannot
substitute for active script qualification.

This common headless driver does not time JS tick bodies, tick dispatch-to-complete
latency or UI responsiveness; main has no common passive API for those metrics.
It therefore cannot close the old mean UI/script timing concern by itself, nor
attribute combined savings solely to scalar animation delay. Preserve that limit
and pair with root's bounded actual TUI/panel responsiveness checks. Main failing
Thiever is a useful correctness finding but leaves active scalar timing unproven.
A valid idle result must retain its idle label; do not call it active evidence.

## Implementation/build update

The common test is now implemented at
`selective-integration-evidence/selective_compare.rs`, with identical temporary
copies under both checkouts' `crates/tui/tests/`. Both release no-run builds passed
without any production or manifest backport. No live run was performed.

Driver SHA256:
`ea742d2b2f65c14ef74b5f684997bdb2fbdb3e39605459dfb467934e7313dfe3`.
Preserved executables and complete Cargo JSON artifacts/provenance are under
`selective-integration-evidence/`; use the copied binaries rather than whichever
artifact is currently in the shared Cargo target directory.

- Baseline executable: `binaries/selective-compare-baseline`; SHA256
  `3dfeaa8c7d79a18b458958772c91e98b9de6f65057ed2cc04988aace018895f2`.
- Selected executable: `binaries/selective-compare-selected`; SHA256
  `9b46ca19d740691808a9a6de709367a3f2123b24dce402ed0e2d8a07b4492a43`.
- Full main source/client SHAs: `compare-baseline-provenance.json`.
- Full selected source/client SHAs: `compare-selected-provenance.json`.

The test requires an absolute COMPARE_RUN_DIR and HOME equal to its `home`
subdirectory, creates the same Food/Lobster loadout fixture there, and requires
explicit COMPARE_CACHE, NAV_PACK and (active only) RS2B0T. These external fixture
paths were not set in the build environment; the orchestrator chooses the same
paths for both launches. It always connects to local 127.0.0.1:43594 and always
samples for 60 seconds after qualification. Player-update rows count observed
changes of the existing player generation; they do not instrument JS execution.

Phase markers are JSON lines prefixed `COMPARE`: spawned, ready,
observation_start, observation_end, optional failure, stopped. A terminal FAIL
exits 1 after stopping the clients. Failed active qualification never falls back
inside the process; an idle control requires a separate explicit launch.
