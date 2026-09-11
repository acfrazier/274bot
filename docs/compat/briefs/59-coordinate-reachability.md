# Connect native coordinate reachability to the isolate

Use configured grok46 defaults. Work on codex/rs2b0t-multirevision. Read
applicable instructions, docs/execution.md, fail-closed-dispatch and the approved
bounded design 04d-coordinate-reachability-design.md at cc57adca. Apply current
operator ownership policy from STATE.md. Do not spawn other agents.

Implement the design's two operations: Reachability.walkable and coordinate
canReach. Rust already owns collision and one flood per snapshot publication.
Reuse that flood and post a compact derived query view with scene origin,
level/dimensions/availability and JS-safe u32 or byte bitsets. JavaScript is
only coordinate/argument shaping and a constant-time lookup of native results;
no collision algorithm, BFS, foreign Reachability/controller or scene copy.
Keep entity row reachability semantics unchanged. Existing maxSteps behavior
stays unchanged; do not invent a new per-call search budget or policy.

Preserve delta/keyframe and session reset behavior. A transition to unavailable
must explicitly clear any retained view; omitted delta means unchanged only.
Keep old buffers without the new optional table readable and fail closed.
Validate finite integer coordinates and bounds before indexing. Missing level
keeps existing default. Use the same scene/flood for row and coordinate facts.
Do not treat player-body plane 0 as a newly verified plane observation.

Allowed scope: minimal api query-view packing/accessors and focused tests;
host-play with_script_snapshot_input publication; script SnapshotInput,
isolate schema/hand-written FlatBuffer encoding and decoding/fingerprint,
load.rs snapshot apply, shim/reachability.js, related meaningful existing tests;
04d-coordinate-reachability-implementation.md and evidence/coordinate-reachability/.
Preserve one flood per encode and avoid another world retain or copy. Report
actual per-view bytes and allocation/copy ownership as a bounded design fact,
not a measured performance win. No navigation engine policy, client, settings
Tile, DirectNavigator, fixtures, LIVE, native frontend, ledger or remote edits.

Concurrent workers: PeriodicBank t_51452e47 owns its service/dispatch registration;
Tile t_701d17f4 owns tile.js, settings and a separate load.rs Rust distance
callback. Your load.rs ownership is only snapshot-apply for the new query view.
Inspect and preserve their edits, commit only your files/hunks, and use exact
committed export plus your owned overlay for checks. Never copy/restore over
shared files. New empty Cargo target, no shared-target reuse; record source and
client hashes, overlay and target, following build-isolation/README.md.

Meaningful proof: native open/blocked/off-scene/other-plane and unavailable
walkability; reachable empty floor versus isolated pocket and a wall; adjacent
selection; row parity; full encode/decode plus synchronous isolate reads with
no queued command; stale scene/session clear; omitted unchanged delta and
changed origin/player/bit contents repost. Cover word boundaries so u64-to-JS
precision cannot hide bit errors. Run affected existing checks/strict Clippy.

Stop this card at these two mapped native query operations and their reviewed
proof. The observed Gnome radius-8 failures are preserved; DirectNavigator.walkTo
and foreign course re-sync remain separate. A green unit test is not LIVE
success and does not authorize dimming a card. Commit only scoped source/report/
evidence, request SAME-card profile reviewer with evidence, then STOP.
