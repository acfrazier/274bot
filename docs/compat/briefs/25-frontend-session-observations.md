# Frontend session facts and armed navigation

Use configured `sol` defaults in the campaign checkout on
`codex/rs2b0t-multirevision`. This is bounded lifecycle/navigation completion
under plan steps 4 and 8. Read host instructions, docs/execution.md, and this
brief. Root owns live engines, remote platforms, public integration and gates.

Root source audit found the panel per-frame callback in session.rs publishes
`nav_states` via `slot.0.rebuild(c)` only after its local-player early return.
The TUI callback in bin.rs publishes `snap.rebuild(c)` directly. Both bypass
the already-reviewed host::publish_snapshot / Pump boundary that rejects old
facts on logout and real login/reconnect (including response 15). Both retain
external WalkArms and player-family tick latches keyed by username. Verify and
fix the concrete stale observation / queued navigation paths in both frontends.
The selected navigation world itself is already wired: panel picker::set_pack
uses Play::world, and TUI nav_world does too. Do not duplicate that work.

Requirements:
- Both frontend snapshots and panel WorldState gating facts must clear on
  logout and session changes, before local-player/hold early returns, and only
  republish facts decoded after the client's actual session grant watermark.
  Reuse the production host publication semantics; do not invent pointer,
  socket descriptor or equal-counter heuristics, or clone the client world.
- Cancel external armed routes, BankBudget continuation and tick latches on
  logout/reconnect and slot removal/recreation so they cannot emit old work.
  Preserve ordinary scene changes and Guardian holds: those freeze or gate
  existing work under the current policy and are not session replacement.
- Observe slot recreation using an actual ownership/lifecycle boundary, not an
  assumption that per-client session generation is globally unique. Username
  reuse must not revive a previous client's facts/routes.
- Keep incremental publication, command ordering, existing timeouts, focus,
  drawing and last-FBO behavior. UI routing must use the immutable bound world.

Allowed files: crates/panel/src/session.rs, crates/tui/src/bin.rs and relevant
existing frontend tests; minimal shared helper/test in crates/host if useful.
Do not edit crates/api, crates/script, the client or crates/host-play/src/lib.rs:
banking owns those concurrent files. If the smallest correct fix needs the
host-play callback contract, report its exact proposed signature/ownership to
root before editing that file so root can serialize it after banking. Do not
work around ownership by copying the world or introducing per-frame clones.

Use focused behavior tests for logout, response-15-style session replacement,
same-name slot recreation and Guardian hold/scene preservation. Existing real
client packet/session fixtures may support the shared helper; keep tests
proportional and do not add mirrored formatting tests. Compile/check affected
frontends with their required features, and record explicit limits from dirty
shared banking WIP. No LIVE runs, no engine or external user-resource changes.

Deliver docs/compat/06a-frontend-session-observations.md and concise raw check
receipts. Commit only scoped files, hand the same card to profile `reviewer`
with kanban_request_review, then stop. Root integrates exact reviewed commits
and runs native/lifecycle proof. Do not spawn additional agents.
