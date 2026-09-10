# Loadout slots, quantities and required provisioning

Operator data direction (2026-09-10): server-derived game facts must come from
programmatically generated, revisioned JSON assets consumed through serde,
using the pipeline in brief 44 and runtime integration in brief 43. Extend that
pipeline for new fact tables needed by this capability; do not hand-maintain
large ID/value/control tables in Rust or copy foreign data modules. Reuse the
immutable per-profile data and keep host gameplay policy separate. Audit any
small protocol constants through the existing client definitions.


Use configured `sol` defaults after parent matching/fill review completes.
Campaign checkout / branch: codex/rs2b0t-multirevision. Read applicable host
instructions, docs/execution.md, fail-closed-dispatch, the plan step 6 and
04-provisioning-recovery-design.md. This is the next coherent family after
first banking and common-loot/withdrawLoad; do not redesign those operations.

Complete the actual in-scope frozen callers' loadout behavior. Current host
Loadout has worn: Vec<String> and carry: Vec<String>; selected_compat_loadout
loses equipment slots and quantities. Both native editors flatten those arrays
to comma-separated text. Required foreign shape has named worn slots and
carry entries {item, qty}. Match real enabled callers in both frozen catalog
roots; dim/quest-only helpers do not become general new scope merely because
their names occur in the design report.

Preserve old saved entries without silently dropping any item when reading,
editing, renaming or saving them. Use a small backward-compatible representation
and migration. Do not guess legacy worn slots from array order; retain unassigned
legacy gear and use the documented accessor fallback when slot identity is
unknown, or verified selected-cache equipment metadata when already available.
Explicit new slots and quantities must survive both editors and isolate posting.
Use temporary test files only; never read or rewrite the operator's actual
~/.274bot/loadouts.json or preferences for this task.

Implement Rust-owned gear/supply/weapon accessors required by enabled callers,
with thin JS name/shape mappings. Preserve selected-name matching, first/null
fallback and existing food behavior. Weapon means righthand, not first worn
entry; supplies retain the configured positive quantities. Complete required
withdraw/wear provisioning by composing the accepted fresh-bank, withdraw,
close and equipment operations with observed outcomes. Verify the exact caller
contract before adding an operation; do not create a foreign loadout planner,
AcquireTask, quest engine or LoadoutPanel clone. JavaScript script callbacks
remain script-owned; host policy stays in Rust.

Preserve existing timeouts, command order, Pause/Guardian freeze, Stop/reconnect
abort, stale-bank refusals and noted-item semantics. A queued wear or withdrawal
is not a successful provision. Unsupported inputs/operations remain explicit
errors; no script revision or catalog allowlist.

Allowed: crates/script loadouts_store, required shim/runtime/transport and tests;
minimal host-play dispatch/service integration if required by the actual callers;
native loadout editing in crates/panel/src/app.rs and crates/tui/src/loadouts.rs,
plus directly affected construction/tests. Coordinate first if another active
worker owns an affected file. Frontend session-observation task owns panel
session.rs, TUI bin.rs and shared host lifecycle helpers; do not touch that work.
No client, nav algorithms, external engines, live runs, main/remotes or gitlink.

Prove legacy/new-format round trips including edit/save, explicit righthand,
quantity retention, fallback, actual script-to-Rust accessor result and composed
fresh-bank provision with observed inventory/equipment result. Include stale,
missing, refused and Pause/Stop cases where new behavior introduces risk. Reuse
existing tests; avoid broad repeats after relevant suites/Clippy pass.

Deliver 04-capabilities-loadouts.md and concise raw checks in
evidence/loadout-capabilities/. Record exact scope, migration, tests and live
limits. Commit only scoped source/report/evidence, request the same card's
profile `reviewer` review via kanban_request_review, then stop. No more agents.
