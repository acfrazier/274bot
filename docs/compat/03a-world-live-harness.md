# Controlled world live harness

This step-5 harness is `crates/host-play/tests/world_boundary_live.rs`. It is
ignored by default and requires `LIVE=1`, `WORLD_REVISION=274|289`,
`WORLD_CASE=nav_full|nav_door|guardian_lamp`, and an explicit
`WORLD_NAV_PACK`; `WORLD_ENGINE_DIR` and `WORLD_NAV_FLAGS` are optional.

The harness resolves and binds an explicit loopback `local-274` or `local-289`
profile, loads the selected cache and `template.world()`, and prepares clients
through `SharedClientTemplate`. Navigation uses the registered
`ScenarioRunner::with_world` scenarios and their existing predicates/deadlines.
Each frame publishes through `host::publish_snapshot` and passes the selected
snapshot through the real `host::Guardian`; no production source, gate, engine,
or original harness is changed.

`nav_full` and `nav_door` use fresh per-run account names. `guardian_lamp`
uses the local `give lamp` fixture, requires an observed inventory lamp, a
Guardian-owned hold (`ours && hold`), and subsequent lamp consumption with the
hold lifted. The harness intentionally makes no live acceptance claim: root
runs and records the 274 and 289 cells against the frozen local engines.

Support checks completed in this worktree:

- `cargo test -p host-play --test world_boundary_live --features memory-profile --no-run`
- `cargo clippy -p host-play --test world_boundary_live --features memory-profile --no-deps -- -D warnings`
- `git diff --check`

No live engine was launched, as required by the brief.
