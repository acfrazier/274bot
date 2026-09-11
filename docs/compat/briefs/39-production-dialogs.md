# Observed Make-X and smithing production panels

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

Complete ChatDialog.makeX using the real count-dialog state, one Answer-Count,
and observed close/menu transitions. Preserve the existing 3000 ms count-open,
3000 ms count-close and 5000 ms make-menu waits from the verified contract;
remove the current one-tick race. Reuse the reviewed count-dialog dismissal and
host lifecycle machinery without expanding PendingBankFetch into a universal
action controller. Preserve existing fixed make and item-on-item ordering.

Implement the enabled SmithingBot main-modal isMainMakePanel/mainMakeProducts/
makeFromPanelMax facts and operation from posted selected-world rows. The anvil
main modal is distinct from chat make-products. Choose the actual largest
posted Make-N operation; missing controls refuse. BankFletcher cut/string/
cut+string remain script policy; preserve both frozen catalog mode sets.
Prove real count prompt/reopen/reject behavior and XP/input/product deltas,
including multiple required products and stale/wrong-modal negatives.
Deliver 04-capabilities-production.md and evidence/production-capabilities/.

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


Current ownership clarification (2026-09-11): audit133 and correction134 forbid
bulk snapshot roundtrips for native polling. Reuse the compact Rust-owned
observation seam introduced by134 or borrow existing host observations; do not
send complete loc/NPC/inventory/bank/widget arrays V8-to-JSON-to-Rust on each
poll, retain a world clone, or move candidate selection back into JavaScript.
JS sends caller arguments/callback projections and dispatches returned verbs.
Preserve delta/reset/session/hold behavior and native deadlines. This applies
to the new capability's implementation and same-card review. Exact frozen
source with one exclusive reusable Cargo cache is permitted by current
execution policy; no shared-target or empty-started claim if reused.
