# Hostile attacker and duel interface observations

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

Complete Game.attackedByPlayer from the actual local-player target fact, the
Rust-owned Guard/Knight of Ardougne/Paladin/Hero predicate used by enabled
Fight/Flee options, and reader.ifText over posted selected-world widget text.
Verify exact foreign call semantics: name, inCombat, not targeting another
player, bounded distance, and Attack action. Avoid policy beyond those facts.
Keep disabled Fight/Flee branches inert without fabricating true observations.

Audit required duel interface roots/partner/waiting/accept controls against both
selected caches and the enabled Duel Arena Combat Trainer calls. Existing
player Challenge/Fight operations stay host-owned. Add missing host facts or
interaction mapping required by those calls; never count a seeded modal as a
real duel. Leave trade transfer and special attack to their own families.
Prove different local/opponent target identities, stale player removal, name
case/radius/action negatives, and selected-modal text rather than global stale
widget labels. Deliver 04-capabilities-hostile-duel.md and matching evidence.

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
