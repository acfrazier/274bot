# Scene layout and occupancy census

Date: 2026-09-07

Decision

Park broad client scene-store migration for now. The bounded executable
fixture confirms that `Square` is already 184 bytes on this target and that the
lossless 17-i32 `GroundStamp` is 68 bytes. In the two deliberately sparse,
structural fixtures, replacing boxed tile/stamp payloads with the proposed
arena/index/free-list plus conservative renderer side arrays leaves a positive
payload difference of 1,534 and 2,297 bytes respectively. That is a useful
structural discriminator, but it is not a material deployment-sized benefit,
not an RSS result, and not evidence about real map occupancy. Do not broaden the
migration without a real existing scene fixture census and allocation-owner
proof.

Scope and source trace

The experiment imports `client::core::World`, `client::dash3d::Square`, and
`client::dash3d::GroundStamp` from the checked-out client submodule. The target
fields were traced before classification:

- `Square::squares` is an `Option<Box<Square>>` grid in `core/world.rs:41-57`.
- `Square::ground` and `Square::overlay_stamp` are separate boxed optional
  fields (`dash3d/square.rs:20-33`); the stamp is the retained simulation-side
  `set_ground` record.
- `GroundStamp` has exactly 17 public `i32` fields
  (`dash3d/ground.rs:350-368`), including four stored heights and eight colour
  values. It must retain heights because `push_down` can move a tile to a level
  whose heightmap row differs (`ground.rs:342-348`).
- `draw_front`, `draw_back`, `draw_sprites`, the side counters, `draw_level`,
  and `fill_stamp` are render/fill bookkeeping (`square.rs:38-50`).
  `model_stamp` is a mutation/invalidation boundary and was not moved into the
  proposed side-array sketch. Picking (`World::ground_x/ground_z`) remains
  simulation/render handshake state (`core/world.rs:74-82`).
- `linked_square` is retained and counted separately. It cannot be flattened
  away: `push_down` transfers former squares and preserves their identity
  (`core/world.rs:179-215`).

Method

`src/main.rs` builds two worlds with the actual public `World::new` and
`World::set_ground` operations. Scene A uses 4 levels and an 8x8 tile plane,
with five set-ground calls including a stacked `(x=1,z=1)` tile followed by
`push_down(1,1)`. Scene B uses the same plane, seven calls with a different
shape/rotation/texture pattern and sparse clusters, followed by
`push_down(3,3)`. The program walks all public `World::square` results and
recursively counts linked squares and `overlay_stamp` values. This is a pure
local structural fixture, not a live/headless attach-detach workload; no
native host, network, renderer, or client source mutation was used.

Measured actual type layout (cargo run)

```text
pointer=8 usize=8 square=184 ground_stamp=68 option_stamp=8 level_heightmaps=24
```

The optional boxed overlay field itself is one pointer-sized 8-byte slot. The
program reports `Square`'s complete target layout as 184 bytes and the exact
stamp payload as 68 bytes. These are Rust layout sizes, not allocator or RSS
measurements.

Fixture results and arithmetic (bytes)

| fixture | occupied | linked | stamps | grid slots | old tile payload | old stamp payload | arena | indices | free list | side arrays | replacement | old - replacement |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| scene A | 10 | 11 | 4 | 256 | 1,840 | 272 | 272 | 40 | 16 | 250 | 578 | 1,534 |
| scene B | 15 | 16 | 7 | 256 | 2,760 | 476 | 476 | 60 | 28 | 375 | 939 | 2,297 |

`old tile payload = occupied * size_of::<Square>()`; `old stamp payload =
stamps * 68`. The proposed replacement is `stamps * 68 + occupied * 4 +
stamps * 4 + occupied * (1 + 6 * 4)`. The one-byte term packs the three
boolean draw flags; the six i32 terms conservatively cover draw level, four
side counters, and fill stamp. This intentionally does not claim that every
render-only field can safely move: direct accesses, picking, linked ordering,
model invalidation, stored heights, and overlay lifecycle still require a full
migration audit.

Capacity and lifetime notes

The fixture's `World::new(4, 8, 8)` creates 256 tile-grid slots; each nested
vector is populated to its requested length/capacity. That grid is a fixed
`Option<Box<Square>>` pointer table and is not included in the old live-tile
payload table above. The experiment does not guess allocator headers, Box
allocation rounding, Vec spare capacity, or the memory for `Ground`, linked
boxes, entities, collision, occlusion, and heightmaps. A replacement arena
must update an existing stamp slot, return deleted slots to a bounded free
list, and reset all indices at build-generation boundaries; appending-only
accounting would be invalid.

Attach/detach and fidelity implications

The source lifecycle is explicit: unheaded builds retain stamps and
`materialize_overlay` rebuilds meshes on attach; `dematerialize_overlay` drops
render meshes while retaining stamps. A future census must execute those
existing boundaries and verify identical picking, overlay invalidation,
minimap state, linked-square order, `push_down` heights, and scene freeze
behavior. This report does not claim those dynamic or visual oracles passed.
The inherited 20 ms logical client progression, mainloop ordering, and
maintained client fence are untouched.

Verification

Command:

```text
cd docs/memory/diagnostics/scene-layout-census-20260907 && cargo run --quiet
```

Result: exit 0; output is reproduced above. The isolated Cargo target was
created under the named diagnostics directory; build artifacts remain local
and are not part of the report deliverable. No production client files were
changed.

Recommendation

The measured structural deltas are positive but immaterial compared with the
campaign's measured multi-megabyte/per-client gaps, and they come from sparse
synthetic fixtures. Park broad scene representation migration. If revisited,
first add a read-only census against an existing client scene fixture with
real build-plane variants and headless attach/detach, then measure allocator
owners separately. Do not infer an RSS saving or authorize cross-client scene
sharing from this experiment.
