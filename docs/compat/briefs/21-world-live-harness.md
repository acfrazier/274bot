# Step 5 controlled navigation and guardian harness

Root integration addition: commit `6c70aed67c97b5cada70478d5eb158d8881b7921`
changes the two panel/TUI live-runner constructors to share Play::world()
instead of loading the ambient default pack. Root is running the focused
existing frontend preparation tests on an isolated committed export. Include
this tiny navigation ownership fix in your same-card reviewer scope; do not
edit those frontend files. Evidence will be under
`evidence/world-capabilities/frontend-binding/`.

Use configured `luna` defaults, then same-card `reviewer` and stop. Root will
run live cells. The accepted host boundary is host `4f43800a` / client
`6cb5a0b1`; world source `80720784` is independently reviewed. Read
03-world-capabilities.md and 02-host-boundary.md. Branch must be
codex/rs2b0t-multirevision. Current banking WIP is owned by Sol t_e52e0a03;
do not edit, stage, stash, reset or restore any of its files or the gitlink.

Create only crates/host-play/tests/world_boundary_live.rs (and one small
new test-support module if useful), using the existing revision_boundary_live
fixture/pump pattern. No product source changes, no original-harness edits,
no gate removal, no engine changes, no live run. The ignored test requires
LIVE=1, explicit WORLD_REVISION=274|289 and WORLD_CASE=nav_full|nav_door|guardian_lamp,
plus an explicit WORLD_NAV_PACK. Optional WORLD_ENGINE_DIR selects the engine.
Bind a loopback-only local ProfileOptions with explicit matching nav/flags and
cache identity. Require a loaded template.world(), emit revision/cache/nav hash
and baseline observations. Reuse a fresh disposable account and existing local
mainland seed, then observed logout/relogin before the proof baseline. Use
host::publish_snapshot with Pump, not manual clears or guessed generation facts.

Nav full: use ScenarioRunner::with_world and the existing nav_full scenario
with the shared selected world. Its existing destination and arrival predicate,
360-second scenario deadline / 400-second process bound remain. Set only the
scenario's engine_speed_ms to None for this campaign so the harness never
changes the shared engine tick speed; record that fixture setting. This is a
new controlled cell and makes no timing comparison. Observe real displacement
and actual cross-mapsquare arrival after preparation. Emit the runner evidence.

Nav door: reuse the existing production Traveller/router and source-derived
Catherby outside/inside fixture from the existing nav_door scenario. A bounded
single-account traversal is sufficient here: prepare at the outside stand,
ensure the selected live door is closed before the baseline (close and observe
if needed), then require route-caused door opening and arrival inside. Do not
teleport to the destination during proof. Do not use the two-bot slamming
companion in this new step-5 cell; that existing stress scenario remains separate.
Keep the existing 180-second outer bound and send/settlement timeouts. Emit
before/after door identity, exact arrival and runner/Traveller evidence.

Guardian lamp: prepare a real lamp via the existing local give command after
mainland relog, observe it in inventory, and capture inventory/strength XP
before enabling Guardian. Drive the actual host::random::Guardian with the
selected snapshot, actual Client driver and default host claim. Observe a
hold, real lamp skill interface, consumption and increased selected-skill XP,
then lifted hold and a post-resolution host walk to prove resumed action. Use
bounded waits (existing 30-second action bound is sufficient); a seed effect,
queued button or initial ready state is not success. Preserve disabled-option
semantics. Run relevant existing guardian hold/claim/resume/reset tests as
source evidence; no new solver campaign. A bank-return live cell will follow
root's acceptance of the independent banking capability and is outside your file.

Compile with the needed existing host-play memory-profile feature if scenario
is required; do not edit Cargo feature structure. Focused format/diff, compile
and strict Clippy are sufficient support-code checks; preserve failure logs.
Report docs/compat/03a-world-live-harness.md and logs under
 docs/compat/evidence/world-capabilities/harness/. No live acceptance claim.
Commit only your new harness/support/report/evidence files. Report exact commit,
then kanban_request_review with reviewer and stop. Review source only, without
moving shared WIP or index. No additional workers, remotes or engine processes.
