# Special attack controls and observed arming

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

Implement the enabled Special API calls against selected-world weapon and
combat controls. Inspect both frozen Special.ts and actual callers for arm,
energy/cost, supported weapon and timeout behavior; do not recreate its foreign
controller. Derive or verify the selected weapon cost and posted special bar
control from cache/content identity. Missing bar/cost/weapon must fail honestly.

Reuse host button/varp facts and bound pending action only where the call
requires observed completion. Preserve existing combat modes, autocast and
melee/range behavior. Cover already armed, insufficient energy, no control,
weapon change and observed special state with both content identities. Spell
teleport and new combat planners are outside this task. Deliver
04-capabilities-special.md and evidence/special-capabilities/.

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
