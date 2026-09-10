# Integrate useful external harness capabilities into the application

The operator explicitly requests Grok architecture tasks to bring useful
capabilities that became external during the memory campaign into 274bot. The
operator also confirms release binaries for macOS, Windows and Linux, and has
Grok capacity available until the next 07:40 America/New_York reset. Produce
useful independent design work; there is no token-spending or runtime target.

Read applicable AGENTS.md and docs/execution.md in the current campaign
checkout. Verify branch `codex/rs2b0t-multirevision`. These are architecture
tasks, not implementation or release authorization. Use profile grok46 defaults.
Each task owns only its named report under `docs/harness-integration/`, commits
that report, then completes with its design findings. No source edits, LIVE,
benchmarks, remote mutations, pushes, new worktrees or other workers' files.

## Shared scope and evidence

Inspect current source and `docs/harness.md`, `docs/compat/STATE.md`, and the
relevant compatibility reports. The active `.superpowers/catalog-headed/` and
`.superpowers/platform-preparation/` Python/PowerShell helpers show recent
external orchestration. Read helper source, not their many frozen source exports
or all raw logs. Respect current worker ownership and distinguish WIP from
committed code; record source commits.

For the earlier external harness, the local git ref `codex/memory-diagnostics`
exists. Inspect a bounded set of relevant files using `git show REF:path` and
`git ls-tree`, without checkout or reading alternative instructions. Start with
`docs/memory/architecture-synthesis.md` and selected capture/controller reports
only as evidence of earlier capabilities. Current shipped code is authoritative.
Old reports are evidence snapshots, not requirements to restart that campaign.
Do not mechanically reintroduce excluded experiments. Each candidate needs a
current user benefit, ownership decision and maintenance cost.

Preserve the immutable per-process revision/profile, shared world ownership,
functional proof after ingame/scene 2, supported options, explicit failures,
bounded diagnostics and ordinary application behavior. Avoid per-tick world
copies, debug-only mandatory build tools, or a second parallel scenario engine.
Reuse scenario/host-play/panel/TUI mechanisms. Keep server-admin preparation
explicit and isolated; normal user sessions must not inherit test privileges.

Every report must include a capability inventory (current built-in, external,
missing, obsolete), concrete ownership/API/UI seams, staged implementation
cards, risks and a focused validation method. Identify what belongs in the
application, a shipped CLI, build-time tooling or external operator automation.
Use plain terms and bounded increments, with a suggested stopping point.

## A — Capture and evidence

Output `docs/harness-integration/01-capture-evidence.md`.
Inspect `crates/scenario/{src/shot.rs,src/runner.rs}`, panel ShotState/GPU
readback/pump_shots/F12, host-play memory diagnostics and external run receipts.
The whole-window GPU capture already exists and writes PNG plus snapshot JSON;
F12 currently uses an empty snapshot. Some catalog scenarios forgot to arm
terminal shots; root is correcting that separately. Avoid proposing another
capture backend. Design useful milestone/failure/on-demand capture, bounded
timeline and diagnostic collection, exact source/profile provenance, evidence
export and a readable panel/TUI result surface. Explain rendering/thread
ownership and completion/error propagation; screenshots must be tied honestly
to the observed state and the code that actually ran.

## B — Scenario and fleet controls

Output `docs/harness-integration/02-run-controls.md`.
Inspect ScenarioRunner, current catalog_boundary_live harness, panel/TUI live
pumps and memory Config/Run/Sample. Inventory external script settings, batch
matrix execution, seeded accounts, isolated stores, pause/resume/Stop/reconnect,
per-actor qualification, and N=2/N=32 handling. Design a shared Rust run
controller exposed through a useful application surface and CLI, with one
source of proof semantics. Cover cancellation, partial failures, account/store
isolation, progress display and repeatable selected runs. Readiness/normal exit
alone must not become success. Avoid a general distributed campaign framework.

## C — Platforms and release diagnostics

Output `docs/harness-integration/03-platform-delivery.md`.
Inspect current startup/profile/resource preparation and its active reviewed
design, tools/game-data, packaging/build metadata and external platform helpers.
The operator observed startup stalls on both Mac and Windows; the Windows
584d05bc diagnostic predates the active startup fix. Design what a three-OS
binary should expose for resource/profile validation, responsive preparation,
support bundles, useful build/cache identity, clean runtime paths and repeatable
smoke checks. Decide which SSH/build/server-provisioning responsibilities should
remain outside the app. Preserve security and credentials boundaries without
inventing a remote control product. Include a concrete release-artifact checklist
and identify architecture/format choices still requiring an operator decision.

## D — Combined plan (after A, B and C)

Output `docs/harness-integration/04-integration-plan.md`.
Read the three completed reports, verify critical current seams, resolve
overlap/conflicts and produce one ordered implementation plan with crate/file
ownership, dependencies, user-visible outcomes and proportional acceptance.
Separate quick wiring fixes from substantial shared-controller changes. Name
what is useful for this compatibility release and what belongs in a following
increment; source approval does not imply live acceptance. No implementation.

## E — Generated game-data architecture

Output `docs/harness-integration/05-generated-game-data.md`.
Independently review the current pipeline and briefs 43/45 plus the upcoming
loadout, spell, shop and production briefs. The operator wants script-required
game facts generated from server data and consumed through serde, rather than
handwritten tables. Design a practical revisioned schema, provenance/dirty-input
gates, deterministic generation, immutable runtime sharing, static JS module
publication, unknown/conditional fact handling and coverage reporting. Include
an extensible acquisition-facts layer (sources, inputs/tools, requirements,
locations, outputs) that can later serve native Quester. Do not implement a
Quester, universal planner or server interpreter. Check actual scripts/tables
for two simple and one conditional example, with clear extraction limits.
