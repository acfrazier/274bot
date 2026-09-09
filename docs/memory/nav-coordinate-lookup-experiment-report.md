# Generated coordinate lookup refinement report

Status: bounded generated diagnostic and behavior-preserving implementation only. This report does not admit a native candidate, alter Stage A bindings, authorize a real pack or F2 run, establish a 5% regression/acceptance result, or claim all-59, lifecycle, resident-memory, or campaign savings.

## Frozen source and implementation

Before editing, `git diff --exit-code 8385babb23fd15b876506d4a3f6154984a6b2df1 -- crates/nav/src/collision.rs` exited 0. Production collision therefore exactly matched the frozen tiled source selected by the reviewed design.

The implementation extracts only the descriptor/dense-pool access into private inline `pair_at_coords(plane, lx, lz)`. `pair_at_index` keeps its original logical index decomposition and then calls that helper. `walkable_word`, plus the packed branches of `walkable` and `standable`, retain their original plane, origin subtraction, bounds checks, flat-index expression, masks, and checked indexing. They first compare that unchanged index with `logical_cells`, preserve the exact `collision index {index} past logical length {logical_cells}` panic payload, and then use their already validated coordinates. Raw flags branches retain their direct `flags[idx]` access and failure ordering. There is no unsafe indexing, cache, retained owner, storage/layout, allocator, route/search, constructor/decoder/encode, public API, feature, client, or manifest change.

## Correctness verification

The tiled dense-oracle suite now explicitly covers the refinement boundary in addition to its existing regressions:

- all 256 face bytes with both blocked states in uniform storage, and all 512 combinations in a forced-dense tile;
- 1/31/32/33/63/64/65 dimensions, non-square partial tiles, and boundary coordinates;
- negative and nonzero origins, an intentionally nonzero `origin.level`, and one through four present planes with absolute level selection;
- unknown levels and out-of-grid coordinates;
- absent raw flags, present raw flags with distinct walk/stand masks, and a short raw flags vector whose three direct-index panics retain the exact standard bounds payload while `walkable_word` ignores it;
- missing synthetic planes: all three packed coordinate APIs retain the exact `collision index 4 past logical length 4` payload, while bounds/unknown-level rejection still precedes the packed read;
- public linear `packed_pair_at` and private coordinate helper equality;
- blocked high-bit padding, exact wire bytes, malformed wire variants, encode/decode, dense route equality, directional/corner masks, pack and paint regressions through the complete nav suite.

`cargo test -p nav collision::tiled_oracle:: -- --skip generated_coordinate_lookup_benchmark --nocapture` passed 15/15 focused tests before and after redirecting coordinate reads. Final `cargo test -p nav` passed 269 library tests and 3 `nav-pack` tests with the one diagnostic benchmark intentionally ignored; doc tests passed with no tests.

Storage equality is explicit: both release arms report `size_of::<WorldCollision>() = 120`, `size_of::<DenseTile>() = 1152`, read-world directory/dense counts 252/248, and route-world counts 9/0. The before `pair_at_index` and after coordinate-method disassemblies contain no allocator calls; the source change adds no allocation expression or field. This establishes no new collision storage or direct read allocation. It does not make a claim about the router's existing search workspaces.

## Bounded generated experiment

Platform/compiler: macOS 15.7.9 arm64, rustc 1.98.0 (`88d9e12ae`, LLVM 22.1.8), cargo 1.98.0 (`797e8a9bc`), default workspace release profile. Each arm used a distinct initially absent `CARGO_TARGET_DIR`, so both were clean release builds. Test execution was single-threaded. Timing is in-process `std::time::Instant` wall elapsed; caches and allocator state were not flushed or purged, and no CPU affinity was set.

The deterministic dense read world is 257×193×4. Each sample executes 24 passes over the same 65,536 precomputed coordinate probes in the same order and calls all three coordinate APIs. The deterministic route world is a uniform-open 96×96 single plane with eight fixed routes in the same order. Each process warms reads and two routes before seven read/route samples. Four process records per arm are retained; after the clean-build records, reused binaries ran before/after/after/before/before/after. No bound was changed after seeing results.

All eight process records completed. Read checksum `1852188150496694907`, route checksum `17711828836823812446`, warm checksums, input dimensions/order, and layout fields are identical across every arm and process.

| Diagnostic | Before flat median | After flat median | Change | Before range | After range |
| --- | ---: | ---: | ---: | ---: | ---: |
| 24×65,536 three-API reads | 14.561896 ms | 10.072709 ms | -30.828% | 14.307708–21.191291 ms | 9.944041–15.098334 ms |
| eight generated routes | 23.685313 ms | 22.989812 ms | -2.936% | 23.039000–29.584416 ms | 22.420750–26.591917 ms |

Per-process read medians were before `[16.851667, 14.549833, 14.461333, 14.511458]` ms and after `[10.693250, 10.033542, 10.029625, 10.144708]` ms. Per-process route medians were before `[27.642750, 23.655500, 23.622208, 23.414500]` ms and after `[24.286250, 22.972416, 22.871792, 22.946292]` ms. Raw nanosecond samples and machine-recomputed statistics are in `diagnostics/nav-coordinate-lookup-experiment/samples.jsonl` and `summary.json`.

The clean `before` build was made after the helper extraction needed for test-first development but before any coordinate API called it. That intermediate leaves the measured coordinate path unchanged: coordinate methods still call linear `pair_at_index`. Before editing, that production source was verified identical to 8385. The final arm changes only the coordinate call path. Exact binary identities are in `environment.txt`.

Optimized arm64 evidence supports, but does not replace, the timing result. Baseline `pair_at_index` contains two `udiv` instructions, and the generated read loop calls it for packed coordinate reads. Final `walkable_word`, `walkable`, and `standable` contain zero `udiv`; final `pair_at_index` retains two for the public linear-index contract. Thus LLVM had not already eliminated the redundant coordinate decomposition in this exact generated build. None of these instructions or timings is a native/real-pack acceptance result.

## Decision

The generated experiment shows a repeatable, useful direct-read reduction and a smaller route-time reduction on this bounded synthetic workload, with identical behavior checksums and storage. Recommend retaining this narrow refinement for source/methodology review. Do not infer a real-pack, all-59, 5% gate, latency, RSS, or final acceptance result, and do not launch native measurement automatically from this card. Root must separately decide any candidate freeze, new bindings/admissions, qualification, and later real work after review.

## Evidence

- `diagnostics/nav-coordinate-lookup-experiment/samples.jsonl`: all 56 read and 56 route samples.
- `diagnostics/nav-coordinate-lookup-experiment/summary.json`: invariant checks and recomputed medians/ranges/change.
- `diagnostics/nav-coordinate-lookup-experiment/assembly-*.txt`: exact optimized disassemblies and division evidence.
- `diagnostics/nav-coordinate-lookup-experiment/environment.txt`: compiler, configuration, source anchor, target and binary hashes.
- `diagnostics/nav-coordinate-lookup-experiment/failures.log`: blocked setup/inspection attempts and bounded alternatives; none launched a benchmark or changed the fixture.
- `diagnostics/nav-coordinate-lookup-experiment/analyze.py`: standard-library verifier/reducer.
