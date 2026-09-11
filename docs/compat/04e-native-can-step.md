# Native adjacent-step mapping

Task `t_1a213450` on `codex/rs2b0t-multirevision`. Source commit
`5612b2652968ebcfa6577bf4439f183de1fd5e21`. Client pin is git archive
`56d80272bcbda3eb1e22db096c1c5e21d3497de4` (tarball sha256
`ffe863519e87cc44e56823bab93ab275f42408a9f0ccbf8fcbc070cf7c881a0f`),
not `git rev-parse` inside the gitless export. Not LIVE, fixtures, or
`DirectNavigator.walkTo`.

## Bridge

Rust still owns collision. `pack_reach_query` already posts walkable /
reachable / reachable_adj from one borrowed `SceneView` plus the flood
computed for entity rows. Next to that, it packs one `u8` per in-scene
tile (`lx * height + lz`): bit `i` is `can_step_local` along `DIRS[i]`
(W E N S NW NE SW SE). No second BFS and no extra scene retain.

JavaScript (`reachability.js` `canStep`) only checks finite integer
coordinates, same level, and Chebyshev 1, then selects that posted bit.
Zero-distance, off-scene `from`, missing masks, other plane, and
unavailable views are false — the same outcomes as `SceneQuery::can_step`.
Walkable/reachable bits are not used; a `W_W` dest can stay walkable
while the east step is illegal.

Keyframe always includes `reach` with `step`. A delta omits the table
only when origin, dims, availability, bitsets and step bytes match.
Available → unavailable posts empty dims and empty `step`. Old buffers
without `step` decode as empty; JS fail-closes.

## Ownership and size

- Observe/encode thread: borrow `GameSnapshot::scene()`, one flood plus
  walkable pack plus step masks, drop after encode. No extra scene retain
  and no extra flood.
- FlatBuffer owns the copy in the post bytes.
- Isolate thread decodes into `__rs2b0t_host.snapshot.reach` before the
  tick. JS reads are synchronous and local (no `host.interact` row).
- Per available 104×104 view: 4056 bitset bytes + 10816 step bytes =
  14872 view bytes, plus origin fields. Bounded design fact, not a
  measured performance win.

## Proof (isolated empty target)

Export `.superpowers/task-exports/t_1a213450-5612b265` with owned overlay
and `CARGO_TARGET_DIR=.../target-t_1a213450` (`isolated_build=true`).
Raw logs: `docs/compat/evidence/native-can-step/`. A first script-lib
invocation timed out mid-compile (300s); the saved log is the successful
retry. `load_isolate` waited on the package cache lock; that does not
invalidate the empty compiled target.

- Native packing matches `SceneQuery::can_step` on open/wall/diagonal
  corner/zero-distance/off-scene/other-plane cases. Walkable dest with
  `W_W` is not a legal step. 5×5 exhaustive dirs match. Per-tile bytes
  at indices 31/32 are not packed across u32 words.
- Encode/decode round-trips `step`. Unchanged deltas omit `reach`.
  Clearing available posts empty `step`.
- Isolate: open orthogonal/diagonal true; wall/corner/zero/far/off-scene/
  other-plane/non-integer false; missing level defaults to 0; word-edge
  tiles 31/32; empty `step` fail-closes; omitted delta keeps last;
  unavailable clears; sync read with empty interact queue.

`cargo clippy --no-deps -D warnings` on api (all-targets), script
(all-targets) and host-play (`--lib`) passed. host-play `--all-targets`
was not used: concurrent catalog LIVE tests in the shared worktree
depend on uncommitted scenario edits this card does not own.

## Out of scope

Gnome radius-8 LIVE, foreign FlaxPicker defect decisions,
`DirectNavigator.walkTo`, `probeable`, fixtures, client, Tile distance,
PeriodicBank dispatch, DeathRecovery, and player-plane decoding remain
separate. A green unit test is not live flax success.
