# Controlled world live harness

The step-5 harness is `crates/host-play/tests/world_boundary_live.rs`. It is
ignored by default and requires `LIVE=1`, `WORLD_REVISION=274|289`,
`WORLD_CASE=nav_full|nav_door|guardian_lamp`, and an explicit
`WORLD_NAV_PACK`; `WORLD_ENGINE_DIR` and `WORLD_NAV_FLAGS` are optional.

The harness resolves an explicit loopback local profile, loads the selected
cache and `template.world()`, and mints a unique per-run account. Every cell
performs the same preparation before its proof baseline: login, local
mainland `seed_at(3220,3212,0)`, observed logout, and observed relogin.

`nav_full` injects the selected world into `ScenarioRunner::with_world`,
clears only the scenario fixture's `engine_speed_ms` to `None`, records that
setting, and retains the scenario's 360-second deadline with a 400-second
process bound. `nav_door` reuses the production Catherby outside/inside
Traveller route and door identity, but removes the two-bot closer companion;
it prepares at the outside stand, closes and observes the door before the
baseline, then requires observed route-caused opening and inside arrival. The
180-second outer bound and runner scene/send settlement behavior are retained.

`guardian_lamp` gives a real lamp after mainland relog, records inventory and
strength XP, drives the actual `host::random::Guardian` with default host
claim, and requires an observed hold, lamp skill interface (`2808`), lamp
consumption, strength XP gain, lifted hold, and a post-resolution host walk.
Existing guardian hold/claim/resume/reset behavior is covered by the source
unit-test evidence recorded in the harness evidence log. No live acceptance
claim is made here: root runs and records the 274 and 289 cells.

Support checks and their exact outcomes are recorded in
`evidence/world-capabilities/harness/support-checks.log`. Live engines were
not launched by this task.
