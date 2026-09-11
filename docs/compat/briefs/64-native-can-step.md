# Map native adjacent-step queries

Use grok46 defaults on codex/rs2b0t-multirevision. Read applicable instructions,
docs/execution.md, fail-closed-dispatch, STATE.md and the reviewed coordinate
query design/implementation from brief 59. Do not spawn other agents. Start
after t_246e320b review; DeathRecovery is gated on this card to serialize shared
snapshot/runtime files. DirectNavigator owns only its separate walk mapping.

Root's isolated 6401d3a4 FlaxPicker cells on both revisions collect no flax:
the old coordinate canReach only resolves NPC/ground rows, which brief 59 fixes.
Inspection also exposes the next required native operation: the unchanged
FlaxPicker calls Reachability.canStep(from,to) while checking its full-pack
escape pocket. Do not run the failed cells again or alter that foreign script.

Map canStep to existing Rust SceneView::can_step/can_step_local semantics.
Native query.rs already checks cardinal/diagonal collision and corner rules.
Preserve those rules; no JavaScript collision algorithm, copied foreign search,
host timeout/policy change or per-read scene/world copy. Arbitrary unrelated
walkable/reachable bits do not prove that a wall-separated step is legal.

Extend the compact derived query view with native adjacent-step results, using
one bounded byte mask per in-scene tile (8 neighbor directions) if that is the
smallest faithful encoding available. Reuse the same scene/collision view used
for row/coordinate publication; do not add a flood or retain another world.
JS only validates finite integer coordinates, same level and adjacent delta,
then selects a posted native bit. Audit zero-distance, off-scene and unavailable
semantics against the existing native method instead of inventing them. Missing
optional data from old buffers and scene/session clears must fail closed.

Allowed scope: api query-view packing/accessors and focused tests; existing
script snapshot schema, hand-written encode/decode/fingerprint, snapshot apply
and shim/reachability.js; minimal host-play publication only if necessary;
related meaningful fixture literals; 04e-native-can-step.md and
evidence/native-can-step/. Preserve concurrent DirectNavigator and loadout-test
work. No shared catalog/scenario fixture, native UI, engine, client, foreign
catalog, LIVE, ledger or remote edits.

Check native open/wall/diagonal corner/off-scene/level cases, parity of posted
mask with SceneView::can_step, finite/bounds/word-edge validation, full/delta
transport and unavailable/reset clearing. Demonstrate no queued command on
synchronous reads and no extra flood. Record maximum view bytes and ownership/
copy cost as design facts, not a measured win. Use an exact committed export
plus owned overlay in a NEW EMPTY Cargo target, affected tests and strict
Clippy. Commit only scoped paths, request SAME-card reviewer, then STOP. Root
owns FlaxPicker gameplay and any separate foreign-script defect decision.
