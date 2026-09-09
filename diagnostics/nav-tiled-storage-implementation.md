# Nav tiled collision storage — implementation report

Task: `t_797a8226`  
Branch: `codex/memory-diagnostics`  
Workspace: `.worktrees/t_a1f4796f`

## What changed

`WorldCollision` no longer retains dense `walk: Vec<u8>` / `blocked: Vec<u64>` after construction. Storage is a 32×32 tile directory plus a dense pool of full 32×32 entries (1024 face bytes + 16 blocked `u64`s). Uniform tiles store a single face/blocked pair in the directory descriptor; dense tiles store a pool index.

### Construction / freeze

- `WorldCollision::from_packed_parts(origin, width, height, walk, blocked, flags)` validates lengths, builds the directory, allocates dense entries only for non-uniform tiles, then drops the dense input vectors.
- `PackedPartsError` covers empty / inconsistent / too-many-plane / flags-length failures. Pack wire errors stay `PackError` and are not rewritten through this type.
- Pack decode builds temporary dense buffers, freezes via `from_packed_parts` only after a successful full parse, then drops temps.
- Bake / world-build / test fixtures construct through the same constructor.

### Accessors

Public geometry: `origin()`, `width()`, `height()`.  
Queries: existing `walkable_word` / `walkable` / `standable` / `nearest_walkable` / reach helpers unchanged in signature; they resolve via directory + dense pool.  
Encode support: `packed_pairs()`, `packed_pair_at()`, `packed_blocked_words()`, `logical_cell_count()`.  
Flags sidecar: still optional `Vec<u32>`; attach/drop behavior preserved.

### Call-site migration

Workspace consumers that touched private dense fields or struct literals were moved to accessors / `from_packed_parts`:

- `crates/nav`: `collision`, `pack`, `paint`, `world`, `router`, `transport`, `bank_fetch`, `bin/nav-pack`
- `crates/host-play`: `lib`, `memory`, `scatter`
- `crates/panel`: `picker`, `overlay`, `session`
- `crates/tui`: `bin`, `map`
- `crates/scenario`: `runner`

No production path retains dense collision vectors after freeze. `StepGrid` / map-square `walk` fields are unrelated types and were left alone.

## Tests added

### Dense oracle (test-only) — `crates/nav/src/collision_tiled_oracle.rs`

Independent `DenseOracle` keeps owned dense buffers and implements dense `walkable_word` / standable logic without going through tiled storage.

| Test | What it proves |
| --- | --- |
| `all_512_uniform_pairs_round_trip_through_tiled_storage` | Every face×blocked pair freezes as a uniform tile (no dense pool) and matches the oracle |
| `single_differing_cell_forces_dense_and_matches_neighbors` | One differing cell forces dense; neighbors still match |
| `mixed_tile_and_edge_dimensions_match_dense_oracle` | Edge sizes and multi-tile layouts match oracle cell-by-cell |
| `four_plane_distinct_contents_and_padding` | Four-plane content + short synthetic planes |
| `encode_bytes_match_dense_baseline_and_roundtrip` | Encode bytes equal a hand-built dense baseline; decode round-trip |
| `raw_sidecar_attach_drop_matches_oracle_and_preserves_walk` | Flags attach/drop vs oracle |
| `constructor_rejects_malformed_shapes_without_replacing_wire_errors` | `PackedPartsError` only for constructor shape failures |
| `malformed_wire_corpus_matches_pack_error_variants` | Expanded wire corpus: magic/version/dimensions/truncation/kind/flags — all `PackError`, freeze never runs on failure |
| `bounded_route_and_corner_masks_match_dense_oracle` | **Independent** `dense_step_ok` + walk-only `dense_find_walk` vs tiled `step_ok` / `find`; full `Route` equality and NoPath agreement on a 5×5 map |
| `layout_accounting_sizes_are_exact_for_known_grids` | Directory/dense counts and `DENSE_TILE_BYTES` |

Route/corner proof does **not** compare two tiled freezes. Dense path reads only oracle buffers.

### Retention (host-play)

- `shared_world_storage_identity_across_sixteen_consumers` — sixteen `Arc` clones share one allocation (`Arc::ptr_eq` + strong count)
- `retained_reader_weak_arc_drops_after_joins` — weak upgrade fails after last strong drop
- `walk_arm_no_path_retains_old_route` — `arm_walk_on` NoPath leaves the previous walk route intact

## Verification run

| Suite | Result |
| --- | --- |
| `cargo test -p nav --lib` | **264 passed** |
| `cargo test -p host-play --lib` | **118 passed** (includes 3 retention tests) |
| `cargo test -p panel --lib` | **376 passed** |
| `cargo test -p tui --lib` | **92 passed** |

No live pack execution in this task.

## Design notes / intentional limits

- Absent upper planes on short synthetic fixtures are not present as uniform tiles; out-of-range plane indices panic the same way dense `Vec` indexing did.
- Uniform-tile padding (cells outside the logical grid inside a 32×32 block) does not disqualify uniformity; only valid cells are compared.
- Dense pool entries are always full 32×32; partial edge tiles that are non-uniform still use a full dense entry with zero padding outside the world.

## Out of scope / not claimed

- Production performance of directory lookup vs dense index (not measured here).
- Real whole-world pack bake timing.
- JS API compatibility work (blocked until memory campaign closes).
