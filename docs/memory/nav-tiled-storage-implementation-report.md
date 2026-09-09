# Nav tiled collision storage — implementation report

Task: `t_797a8226`  
Branch: `codex/memory-diagnostics`  
Workspace: `.worktrees/t_a1f4796f`  
Baseline (design): `29b7aea779322c8611f83dc193e939ca7d756f75`  
Design contract: `docs/memory/nav-tiled-storage-design.md` §§1–5  

This report documents the runtime implementation and generated correctness tests only. It does **not** claim RSS savings, live acceptance, real-pack proof, or Stage A/B measurement results.

## 1. Exact source migration

`WorldCollision` no longer retains dense `walk: Vec<u8>` / `blocked: Vec<u64>` after construction. Storage is private:

| Component | Layout |
| --- | --- |
| Directory | `Box<[u64]>` — one explicit descriptor per spatial 32×32 tile per present plane strip |
| Dense pool | `Box<[DenseTile]>` — `#[repr(C)]` entry = `[u8; 1024]` faces + `[u64; 16]` blocked words (1152 bytes, ≥8 align) |
| Scalars | `origin`, `width`, `height`, `logical_cells`, `tile_cols`, `tile_rows`, `final_blocked_high_bits` |
| Sidecar | optional `flags: Option<Vec<u32>>` unchanged attach/drop semantics |

Descriptor encoding (design §3):

- bit 63 clear → uniform; low 9 bits = `face | (blocked as u64) << 8`; other bits zero
- bit 63 set → dense; low 32 bits = pool index; other bits zero

Uniformity is decided from **valid cells only**. Partial edge padding does not disqualify a uniform tile. Absent synthetic upper planes are not present as directory entries.

### Construction / freeze

- `WorldCollision::from_packed_parts(origin, width, height, walk, blocked, flags) -> Result<Self, PackedPartsError>`
  - Validates: positive dimensions; walk length multiple of plane size; 1..=4 planes; blocked word count = `logical_cells.div_ceil(64)`.
  - Does **not** validate `flags` length against cell count. Mismatched flags are stored as-is (sidecar attach remains a separate path). `PackedPartsError` is **constructor-only** and never covers flags-length failures.
  - Count pass over dense inputs → allocate exact directory + dense pool → fill → **drop both input `Vec`s** before return.
- `PackedPartsError` variants: `ZeroDimension`, `TooManyPlanes`, `InconsistentLength { … }`.
- Pack wire errors remain `PackError` (`BadMagic`, `BadVersion`, `Truncated`, `BadLength(String)`, `Io`). Decode never rewrites wire failures through `PackedPartsError`.
- `pack::decode`: parse faces, blocked words, edges (incl. requirements), banks fully; only on success call `from_packed_parts` (`.expect` after already-validated lengths). Freeze does not run on any error path.

### Public accessors (geometry / queries)

- Geometry: `origin()`, `width()`, `height()`
- Queries: `walkable_word` / `walkable` / `standable` / `nearest_walkable` / reach helpers (signatures preserved)
- Encode support: `packed_pairs()`, `packed_pair_at()`, `packed_blocked_words()`, `logical_cell_count()`, layout helpers `directory_len()` / `dense_pool_len()`
- Flags: still optional `Vec<u32>` field; attach/drop behavior preserved

### Workspace call-site migration

Consumers of former public dense fields or struct literals moved to accessors / `from_packed_parts`:

- `crates/nav`: `collision`, `pack`, `paint`, `world`, `router`, `transport`, `bank_fetch`, `bin/nav-pack`
- `crates/host-play`: `lib`, `memory`, `scatter`
- `crates/panel`: `picker`, `overlay`, `session`
- `crates/tui`: `bin`, `map`
- `crates/scenario`: `runner`

No production path retains dense collision vectors after freeze. Unrelated `StepGrid` / Traveller walk symbols were not rewritten. Immutable census tooling under `docs/memory/nav-uniform-census*` was not modified.

Wire preservation: v8 canonical encode order (faces → blocked words → edges/teleports → requirements → banks), accepted trailing bytes, existing decode error variants/payloads, flags sidecar `274F` v1.

## 2. Sizes, ownership, temporary peaks

### Element sizes (compile-time / layout accounting)

| Item | Bytes |
| --- | --- |
| `DENSE_TILE_BYTES` (`size_of::<DenseTile>()`) | 1152 (`1024 + 16*8`) |
| Directory entry | 8 (`u64`) |
| Design census residual (observed pack, design §3) | directory 508,928 + dense pool 3,559,680 = **4,068,608** element bytes |

Ownership: one existing `Arc<NavWorld>` owner; collision directory/pool live inside that world. No dense dedup/hash/run-length/cache; no per-bot collision maps; no tile-size tuning.

### Temporary peak model (design §3 construction — not measured RSS)

First candidate preserves the existing dense decoder path. On successful parse:

1. Borrowed/owned navpack input bytes remain live through decode.
2. Temporary dense `walk` + `blocked` Vecs are filled.
3. Count pass (no second whole-world map) → exact directory/pool allocate → fill.
4. Both dense input Vecs drop inside `from_packed_parts` before the collision is returned / published under `Arc`.

Illustrative **requested element overlap** for the observed ~73 MiB pack (design model, not observed peak):

```
input navpack     73,438,581
+ dense collision 73,285,632
+ candidate pool  4,068,608
≈ 150,792,821 bytes
```

before graph/banks, headers, allocator rounding, and other Play startup owners. Headers/page rounding are additional. Exact-capacity boxed slices avoid a grow-and-shrink pool; no second whole-world pair map; at most bounded one-tile stack scratch during fill.

Bake still owns raw `u32` flags for sidecar export (separate peak; not optimized here). Encode still allocates the returned wire `Vec`. A direct-from-wire tiled decoder is **not** this hop.

This task did **not** run the real pack, remote/native measurement, or live apps. Peak numbers above are the design overlap model only.

## 3. Meaningful tests — bounded generated evidence (not full §5 close)

Test-only module: `crates/nav/src/collision_tiled_oracle.rs`  
Independent `DenseOracle` owns dense buffers and never routes through tiled storage. No production dense oracle retained.

This card ships **bounded generated** proofs useful for implementation review. It does **not** claim every design §5 gate is closed. Open items are listed in §4.

| Test | What it proves (bounded) |
| --- | --- |
| `all_512_uniform_pairs_round_trip_through_tiled_storage` | All 512 face×blocked pairs freeze uniform; oracle match |
| `single_differing_cell_forces_dense_and_matches_neighbors` | One differing cell → dense; neighbors match |
| `mixed_tile_and_edge_dimensions_match_dense_oracle` | Edge sizes / multi-tile layouts cell-by-cell |
| `four_plane_distinct_contents_and_padding` | Four-plane + short synthetic planes |
| `encode_bytes_match_dense_baseline_and_roundtrip` | Empty-graph encode bytes = hand dense baseline; decode RT. **Does not** prove ordered transport/bank wire old-vs-new equivalence for non-empty graphs |
| `raw_sidecar_attach_drop_matches_oracle_and_preserves_walk` | Flags attach/drop vs oracle |
| `constructor_rejects_malformed_shapes_without_replacing_wire_errors` | `PackedPartsError` constructor-only; wire stays `PackError` |
| `malformed_wire_corpus_matches_pack_error_variants` | Manual PackError variant+payload corpus on current decoder; freeze never on failure. **Not** a frozen dense-baseline binary differential harness (§5.4 still OPEN) |
| `bounded_route_and_corner_masks_match_dense_oracle` | Independent `dense_step_ok` + walk-only `CostModel::running` `dense_find_walk` full `Route` eq on 5×5. **Not** all represented find options/cost models (§5.5 still OPEN for that breadth) |
| `layout_accounting_sizes_are_exact_for_known_grids` | Directory/dense counts + `DENSE_TILE_BYTES` |

### Wire corpus cases (`malformed_wire_corpus_matches_pack_error_variants`)

All failures assert `PackError` (never `PackedPartsError`). Exact string payloads matched against the **current** decoder (same code path as production), not a separately frozen baseline binary:

| Case | Expected |
| --- | --- |
| Bad magic / 274N as pack | `BadMagic` |
| Truncated header | `Truncated` |
| Versions 0–7,9,255 | `BadVersion(v)` exact |
| Zero / oversize / overflow dimensions | `BadLength(…)` |
| Short faces / blocked / edge count body | `Truncated` |
| Unknown transport kind 99 | `BadLength("unknown transport kind 99")` |
| Unknown door dir 5 | `BadLength("unknown door dir 5")` |
| Requirement section truncation (skill_req count=1, no pairs) | `Truncated` |
| Bad UTF-8 quest req | `BadLength("quest req is not UTF-8")` |
| Bank section truncation (count=1, no body) | `Truncated` |
| Bad UTF-8 bank name | `BadLength("bank stand name is not UTF-8")` |
| Unknown bank access tag 2 | `BadLength("unknown bank access tag 2")` |
| Valid empty graph + trailing garbage | accepted (trailing unread) |
| Flags sidecar bad magic/version / partial u32 | `PackError` paths |

### Retention (host-play)

- `shared_world_storage_identity_across_sixteen_consumers`
- `retained_reader_weak_arc_drops_after_joins`
- `walk_arm_no_path_retains_old_route`

Existing route generation / token / retry / old-route tests preserved (optioned finds exercise tiled storage, but are not a dense-baseline differential).

### Verification run (this card)

| Suite | Result |
| --- | --- |
| `cargo test -p nav --lib` | **264 passed** |
| `cargo test -p host-play --lib` | **118 passed** |
| `cargo test -p panel --lib` | **376 passed** |
| `cargo test -p tui --lib` | **92 passed** |
| `cargo check … --features memory-profile` (host-play/panel/tui) | ok |

No real 73 MiB pack execution on this card. No native measurement release.

## 4. Remaining gates (explicitly OPEN — root-owned)

Do **not** treat this card as closing full design §5 or authorizing Stage A/B.

| Gate | Status |
| --- | --- |
| **§5.3** hash-bound real-pack exhaustive cell/word/encode/graph oracle | OPEN — not run; root commissions after review |
| **§5.4** frozen dense **baseline** differential wire/error/fallback harness | OPEN — current corpus is manual expectations on the live decoder, not old-vs-new baseline binary compare |
| **§5.5** all represented find options/cost models + full corner breadth vs dense oracle | OPEN — bounded walk-only 5×5 + existing router tests only |
| Ordered non-empty transport/bank wire old-vs-new equivalence | OPEN — empty-graph encode baseline + pack roundtrips are insufficient proof |
| **Stage A** cold-load/microbench CPU/RSS/latency gates (design §6) | OPEN — not authorized/executed |
| **Stage B** N=1/16 matched matrix | OPEN — not authorized/executed |

Root will commission a frozen-baseline differential harness with generated qualification before any separately authorized real-pack proof. Review may approve this implementation as **bounded generated evidence** while leaving the gates above OPEN.

Out of scope on this card: census tool edits, JagFX/client/render/API-policy/benchmark-threshold changes, remotes/server/accounts/cache/input mutations, live apps, savings claims, native measurement.

## 5. Intentional limits / honesty notes

- `from_packed_parts` does **not** reject mismatched `flags` lengths; do not treat `PackedPartsError` as covering flags-length failures.
- Oracle route proof is walk-only `CostModel::running` on a bounded 5×5; existing router tests still cover optioned finds through tiled storage.
- Empty-graph encode baseline in oracle encode test; graph order still covered by pack roundtrips elsewhere.
- Absent upper planes on short synthetic fixtures are not uniform directory tiles; OOR plane indices retain baseline panic-on-index behavior where applicable.
- No production performance claims vs dense indexing.
- STATE remains root-owned; not edited by this implementer commit.
