# Headless catalog proof through production Play

Use configured `sol` defaults, then same-card `reviewer` and stop. This is a
bounded harness task for plan step 7 after step-5 harness source review; root
owns the actual world/bank live checks and temporary 289 gate removal. Branch
codex/rs2b0t-multirevision. Read instructions, fail-closed-dispatch, the support
matrix, 02-host-boundary.md and relevant existing scenario/frontend runner
code. Do not modify client, host runtime, engine, frontend or world-harness WIP.

Create only crates/host-play/tests/catalog_boundary_live.rs and optional new
small test-support code. It must run actual frozen catalog scripts through
SharedClientTemplate + run_with_template + ScriptStartHandle/Play and the real
ScenarioRunner; the harness never supplies the script's gameplay actions.
Compile with existing memory-profile feature (scenario dependency), no Cargo
architecture changes. Root will run the cells; no live launch by this worker.

Require LIVE=1 and explicit CATALOG_REVISION=274|289, CATALOG_ROOT,
CATALOG_COMMIT, CATALOG_SCENARIO, CATALOG_NAV_PACK; optional CATALOG_ENGINE_DIR
and explicit settings JSON for later branch proofs. Begin with the existing
bone_burier, chicken_killer, thiever, alcher, bank_fletcher scenarios. Resolve
exact names from the registry. The two catalog commit hashes and per-card
source paths/hashes are in support-matrix.json; use them to verify proof input
identity, not as a product loading allowlist. Missing required source, unknown
case, malformed settings, no nav world or unavailable host operation fails the
campaign cell. No successful optional-source skip with LIVE=1.

Use a loopback-only bound profile and selected shared world, ephemeral local
accounts/settings/JS cache, and real JsLibrary register_rs2b0t -> ensure_js ->
resolve_sibling_modules. Schema defaults, scenario injections and explicit
settings merge in that order without writing operator preferences. Record
catalog/card source hash, compiled JS/sibling hashes, revision/cache/nav identity,
settings and account. Do not silently substitute a shim-only script.

Reuse frontend StartScript sequencing: stage the real source first; fire once
when the scenario reaches StartScript after its actual seed/relog waits. Build
the Play with zero accounts, install the start handle, then spawn fresh accounts
to avoid a callback/handle race. Keep existing scenario deadlines and per-action
bounds. Disable only scenario engine_speed_ms changes for these controlled
cells and record this; never change server timing. Drive the runner with the
actual guardian hold and selected template.world(), not the default pack.

At the actual StartScript point capture a fresh production-published snapshot
baseline via host::publish_snapshot/Pump. Require ingame, scene2 and fresh local
player. In addition to the existing runner predicate, retain independent
post-Start observations sufficient for the five core loops: BoneBurier inventory
bones consumed + prayer XP; ChickenKiller combat XP plus configured core loot/
bury outcome; Thiever theft/XP and inventory progress; Alcher magic XP plus
fodder/rune consumption and coins; BankFletcher fletching XP plus logs consumed
and product created. Inspect exact scenario settings and source names, do not
guess IDs. A seeded XP/item/chat state, initial full bank or queued send cannot
satisfy a delta. Do not weaken a scenario's predicate to get a pass. The five
core cells do not claim banking/restock/all options or all 45 cards accepted.

Print structured phase/baseline/outcome observations and FAIL + exit 1 for
missing gates, script error, refused Start, failed runner or bounded timeout.
Use actual errors instead of logging and silently waiting. Stop all owned Play
slots on completion/failure. No vault credentials or private keys in logs.

Run focused format/diff, compile, strict Clippy and a meaningful offline test
for proof-baseline/identity failure logic if needed. Retain failures. Commit
only new harness/support, docs/compat/05a-catalog-live-harness.md and evidence
under docs/compat/evidence/catalog-harness/. Report exact commits, then request
same-card reviewer review and stop. Reviewers inspect named source exports
without moving WIP or index. No extra agents, remotes or product gate changes.
