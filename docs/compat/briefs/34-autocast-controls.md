# Autocast through selected-world combat controls

Operator data direction (2026-09-10): server-derived game facts must come from
programmatically generated, revisioned JSON assets consumed through serde,
using the pipeline in brief 44 and runtime integration in brief 43. Extend that
pipeline for new fact tables needed by this capability; do not hand-maintain
large ID/value/control tables in Rust or copy foreign data modules. Reuse the
immutable per-profile data and keep host gameplay policy separate. Audit any
small protocol constants through the existing client definitions.


Use configured sol defaults after the parent card completes its same-card
review. Campaign branch codex/rs2b0t-multirevision. Read applicable AGENTS.md,
docs/execution.md, fail-closed-dispatch and the relevant sections of plan step 6
and 04-combat-production-design.md. Verify design claims against both frozen
catalog roots and selected cache identities. Design approval is not code or
LIVE acceptance. Missing capabilities remain explicit failures; do not shrink
the enabled set or restore foreign JavaScript controllers.

Implement Autocast.arm and the enabled mage-style paths using selected-world
combat-tab choose/grid/toggle controls and observed armed state. The old literals
328/353/1829/349/varp 108 are audit inputs, not unchecked universal constants.
Use host-posted metadata and the spell facts from brief 27. Preserve the actual
foreign arm sequence and 3000 ms waits; a missing control must refuse. Keep
melee/range style resolution and the corrected Game.combatStyleResolution
return shape. Do not import Autocast.ts or CombatStyleLogic.ts policies.

Cover initially armed, choose-to-grid, spell selection, toggle, unavailable
spell/control, stale modal, and real posted armed-state transitions on both
selected caches. A queued button alone cannot satisfy the host completion.
Special attack and spellbook teleport remain separate tasks. Deliver
04-capabilities-autocast.md and evidence/autocast-capabilities/.

Allowed: required api/script/host-play observation, wire, Rust services and thin
shim mappings plus focused composed tests and the named report. Reuse client
protocol definitions and existing serializers. Snapshot/schema additions append;
no new packet table or client bot API. No client, frontend, profile, engine or
fixture edits. Preserve shared WIP and use exact source exports for checks.
Root owns LIVE, ledger, scenario, gitlink and repository integration.

Prove the real script call through wire, Rust operation and posted outcome,
including unavailable content, stale identities, refusal, timeout and
Pause/Guardian freeze/Stop/session abort where operations wait. Use meaningful
selected-world fixtures and affected existing tests/Clippy, without broad
repetition. Commit scoped files in new commits; request review on this same
card with profile reviewer and then stop. No agents or complete-before-review.
