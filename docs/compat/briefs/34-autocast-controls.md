# Autocast through selected-world combat controls

Operator data direction (2026-09-10): server-derived game facts must come from
programmatically generated, revisioned JSON assets consumed through serde,
using the pipeline in brief 44 and runtime integration in brief 43. Extend that
pipeline for new fact tables needed by this capability; do not hand-maintain
large ID/value/control tables in Rust or copy foreign data modules. Reuse the
immutable per-profile data and keep host gameplay policy separate. Audit any
small protocol constants through the existing client definitions.


Use configured grok46 defaults after the parent card completes its same-card
review. Campaign branch codex/rs2b0t-multirevision. Read applicable AGENTS.md,
docs/execution.md, fail-closed-dispatch and the relevant sections of plan step 6
and 04-combat-production-design.md. Verify design claims against both frozen
catalog roots and selected cache identities. Design approval is not code or
LIVE acceptance. Missing host capabilities remain explicit Rust work. Apply the
operator ownership rules in STATE.md: evidenced broken foreign scripts may be
dimmed with a concrete reason; our host/shim bugs stay in scope. Ancillary
feature stubs remain future native projects and do not dim working cards. Do
not restore foreign JavaScript controllers.

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

Operator routing, 2026-09-11 02:56 UTC: this is one of four additional
implementation tasks assigned to Grok 4.6 to use the current allowance. Use
profile grok46 without model/provider overrides; same-card reviewer remains
profile reviewer (Grok 4.5). Dependency gates remain unchanged. All acceptance
checks must use a new empty Cargo target for an exact frozen source export
with owned overlay and recorded hashes, following evidence/build-isolation/README.md.
Do not reuse the shared campaign Cargo target.
