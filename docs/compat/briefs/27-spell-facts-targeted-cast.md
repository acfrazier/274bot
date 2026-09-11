# Selected-world spell facts and targeted inventory casting

Operator data direction (2026-09-10): server-derived game facts must come from
programmatically generated, revisioned JSON assets consumed through serde,
using the pipeline in brief 44 and runtime integration in brief 43. Extend that
pipeline for new fact tables needed by this capability; do not hand-maintain
large ID/value/control tables in Rust or copy foreign data modules. Reuse the
immutable per-profile data and keep host gameplay policy separate. Audit any
small protocol constants through the existing client definitions.


Use configured `grok46` defaults after the loadout task's source review. Work on
the campaign branch codex/rs2b0t-multirevision. Read applicable instructions,
docs/execution.md, fail-closed-dispatch, plan step 6 and the spell accounting /
targeted cast sections of 04-combat-production-design.md. The design is
guidance: verify its source/cache claims against the two selected inputs.

Complete the shared Rust spell and staff facts used by the enabled frozen
cards. Preserve the real SPELL_DB export shape and add STAFF_RUNES, including
all fire providers used by the later Superheater source. Implement the required
castsAvailable, runeWithdrawList and spellButtonCom contracts in Rust with thin
JavaScript argument/result mapping. Use selected-cache metadata where it
exists, or a small Rust-owned verified content table tied to the world identity.
Do not copy the foreign generator or its JavaScript computation. Account for
multiple rune-providing staves, unknown spell handling and the no-runes-needed
case according to actual callers. Do not silently cut the later catalog's
fire-staff alternatives or change its import surface to hide a missing export.

Make Game.castOnItem resolve and dispatch the actual named inventory-target
spell through posted selected-world magic controls. Check the real return
contract: the script may own its subsequent XP/item wait. Do not introduce a
second wait or claim successful casting merely because a packet was queued.
If the existing contract requires a host completion result, return it only from
observed outcome with the existing timeout. Keep spellbook opening, stale
target rejection, command order, Pause/Guardian freeze and Stop/session abort.
No new protocol table, bot API inside the client, or silent 274 ID fallback.

Scope is the enabling facts and inventory-target cast used by Alcher and
Superheater and rune supply planning for the other enabled cards. Autocast,
special attack, spellbook teleport, hostile facts, shop, make menus and fire
completion remain subsequent coherent tasks. Excluded quest castOnLoc and
castOnNpc are not added merely because they appear in a declaration file.

Allowed: API content/interaction/snapshot helpers, script shim/runtime/wire and
focused tests, minimal host-play integration if needed. Prefer existing facts
over schema additions; any schema fields append. Do not edit client, nav
algorithms, frontends, external engines, fixtures, operator configuration or
concurrent tasks. No LIVE, operation-gate change, main, remotes or gitlink.

Prove actual script imports and helpers through Rust callbacks, including the
later Superheater STAFF_RUNES import. Cover staff substitution, remaining rune
costs, unknown inputs and selected-world IDs. For casting, traverse script call
to IPC to the actual Rust operation and posted state/result; include missing
spell control, stale inventory and a positive observed Magic XP/item outcome.
Do not substitute a serializer-only test for the composed path. Run relevant
existing suites/Clippy once after fixes; retain the first banking regressions.

Deliver 04-capabilities-spells.md with exact facts/provenance, supported callers,
raw checks under evidence/spell-capabilities/ and clear live limits. Commit only
scoped files, request same-card profile `reviewer` review, then stop. No agents.

Operator dispatch update (2026-09-11 01:00 UTC): this is a Grok 4.6
implementation task, using profile `grok46` defaults (grok-4.6 / xai-oauth, high
reasoning). This supersedes the original card body mentioning Sol. Keep the
same branch, scope, dependencies, required checks and same-card `reviewer`
handoff. Do not self-complete implementation or bypass the review gate.
