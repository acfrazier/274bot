# Coordinate reachability implementation

Task `t_246e320b` on `codex/rs2b0t-multirevision`. Source commit
`1f8737cfa746d98a9a31a74d3b2326abaabe80e9`. Client gitlink
`56d80272bcbda3eb1e22db096c1c5e21d3497de4`. Design `cc57adca` as
corrected for actual posted `minusedlevel`. Not LIVE, fixtures, or
`DirectNavigator.walkTo`.

## Bridge

Rust still owns collision and one `SceneQuery::flood_reach` per snapshot
encode in `with_script_snapshot_input`. Next to that flood, the host packs
walkable bits from the same borrowed `SceneView` (`SQ_BLOCKED == 0`,
`lx * height + lz`) and converts the flood's u64 words to JS-safe u32
words. The isolate snapshot carries:

```
reach: { available, base_x, base_z, level, width, height,
         walkable[], reachable[], reachable_adj[] }
```

JavaScript (`reachability.js`) only shapes coordinates and does an O(1)
bit lookup. Missing / non-integer `x` or `z` is false. Missing `level`
defaults to 0. Entity npc/ground rows still win for `canReach` when a
row exists. `maxSteps` is unused, as it already was for entity rows.

Keyframe always includes `reach`. A delta omits it only when origin,
dims, availability and bit bytes match the last post. Available →
unavailable posts `available: false` with empty dims so the isolate
cannot keep a previous course flood. Old buffers without the table
remain readable; a keyframe missing `reach` fail-closes to unavailable.

Posted `level` is the flood/scene plane already bound by observe `here`
(`GameSnapshot::tile` / `player_here_tile` use `client.minusedlevel`).
This hop does not add a player-plane field. A destination on another
plane is false.

## Ownership and size

- Observe/encode thread: borrow `GameSnapshot::scene()`, compute one
  flood plus the walkable pack, drop both after encode. No extra scene
  retain and no second BFS.
- FlatBuffer owns the copy in the post bytes.
- Isolate thread decodes into `__rs2b0t_host.snapshot.reach` before the
  tick. JS reads are synchronous and local (no `host.interact` row).
- Per available 104×104 view: 3 × ceil(10816/32) u32 = 338×3 words =
  4056 bitset bytes, plus origin fields. Bounded design fact, not a
  measured performance win.

## Proof (isolated empty target)

Export `.superpowers/task-exports/t_246e320b-1f8737cf` with
`CARGO_TARGET_DIR=.../target-t_246e320b` (`isolated_build=true`).
Raw logs: `docs/compat/evidence/coordinate-reachability/`.

- Native packing matches `SceneQuery::walkable` and `ReachFlood::at`,
  including off-scene / other-plane / blocked tiles and u32 word
  boundaries 31/32/53/63/64.
- Encode/decode round-trips those words. Unchanged deltas omit `reach`.
  Clearing available posts an explicit unavailable table.
- Isolate: open/blocked/off-scene/other-plane walkability; reachable
  empty floor versus isolated pocket and a wall; `adjacentOk`; missing
  level default; non-integer reject; `maxSteps` ignored; far tile not
  Chebyshev-true; existing entity row exact/adj/tile/missing parity;
  sync read with empty interact queue; omitted delta keeps bits;
  unavailable post clears them.

`cargo clippy --no-deps -D warnings` on api (all-targets), script
(all-targets) and host-play (`--lib`) passed. host-play `--all-targets`
was not used: concurrent catalog LIVE tests in the shared worktree
depend on uncommitted scenario edits this card does not own.

## Out of scope

Gnome radius-8 LIVE, foreign course re-sync, `DirectNavigator.walkTo`,
`canStep` / `probeable`, fixtures, client, Tile distance, PeriodicBank
dispatch, and player-plane decoding remain separate. A green unit test
is not live success and does not dim GnomeCourse.
