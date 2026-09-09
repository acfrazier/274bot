# Reuse validated coordinates in tiled collision reads

Status: design proposal only. No source change, new candidate admission, native
run or change to the current Stage A releases. This is a possible refinement of
the existing tiled representation, not a second representation or router.

## Evidence and question

At frozen tiled8385babb, `crates/nav/src/collision.rs:474–549` validates the
plane and local x/z, forms `idx = plane * width * height + lz * width + lx`,
and calls `pair_at_index_or_panic`. The latter calls `packed_pair_at` and
`pair_at_index` (lines330–336,385–417), which divide the index by plane size
and width to recover plane, lz and lx before addressing the tile directory.
All three coordinate APIs repeat this path when they use packed storage.
Root inspected these exact unchanged source spans in the campaign checkout.

That is demonstrably redundant source arithmetic; it is NOT proof of executed
divisions, their cost, or a measured improvement. Inlining/optimization may
remove some work. Inspect optimized code or measure before making that claim.
The existing row40 cold screen's route timings are diagnostic only; neither
its difference nor the failed59-row process establishes a full routing regression.

## Narrow proposed change

Extract the existing directory/dense-pool read into a private inline helper that
accepts the already validated `(plane, lx, lz)`. Keep the descriptor, local-cell
bit arithmetic and checked slice accesses identical. The linear-index API still
decomposes its index, then uses this same helper; it retains its public contract.

For `walkable_word`, and the packed branches of `walkable` and `standable`, keep
all existing level/origin/bounds expressions and the same flat-index expression.
Check that index against `logical_cells` before using the coordinate helper.
On a missing synthetic plane, preserve the current `pair_at_index_or_panic`
failure (including the same message) rather than treating it as a uniform tile
or open ground. Do not remove the logical-length check on the strength of the
usual full four-plane pack. Keep overflow behavior of the existing expressions;
do not introduce saturating arithmetic or reinterpret malformed coordinates.

Keep raw `flags` branches and their existing buffer indexing/failure order.
`walkable_word` must still ignore raw flags; walkability and standability must
continue to use their distinct masks. Do not change plane selection to subtract
origin.level, short-plane handling, directional faces, graph/search policies,
transport/bank semantics, budget/timeouts or ordering. No unsafe indexing,
lookup cache, extra retained owner, directory layout, constructor/decoder/encode,
iteration, allocation, feature/default, client or public API change.

Only `crates/nav/src/collision.rs` and narrowly scoped nav tests/report would
be owned by a later implementation. Keep current8385 source/admissions and all
failed/cold evidence immutable. The refined source would have a new commit and
new build/provenance; never relabel the currently frozen executable.

## Falsifiable verification

Before any implementation release, independently review this design. A later
bounded implementation must compare all three coordinate APIs against the
frozen8385 behavior and the unchanged logical packed-pair oracle. Cover all256
face values and both blocked values; uniform/dense tiles; 31/32/33 edges and
non-square partial tiles; negative/nonzero origins; 1–4 planes; unknown levels;
missing synthetic planes; absent/present/short raw flags. Preserve wire/blocked
padding equivalence and existing nav route/pack/paint regressions. Include exact
panic cases where the old API panics, not just successful cells.

Use a bounded generated clean release-build read/route comparison to test the
CPU hypothesis with identical checksums and workload. Inspect optimized code
when useful; do not call an assembly change an accepted performance win. All
samples and failed checks remain evidence. No real pack access is released by
this proposal or a generated result. Layout/allocation equality should establish
that this refinement adds no collision storage or read allocation.

The existing full-byte differential tooling and all59 exact selectors remain
the correctness/qualification basis for any later native candidate. Before a
new real measurement root must freeze the refined commit and update/review tool
source bindings as needed, build and admit fresh binaries, and qualify them.
Current Stage A tooling still binds29b/8385; do not edit that worker's source or
manifest in parallel to make a new candidate fit. Previous results are reused
only for unchanged scopes, with explicit limits.

## Decision boundary

Current routing tooling and its bounded F1 feasibility path continue unchanged.
Review the F1 evidence before deciding whether to finish qualification of8385 or
release this refinement first. A design approval does not make that choice or
launch another route schedule. If the redundant arithmetic is already eliminated
or generated measurement shows no useful benefit, park this refinement rather
than expanding into search optimization.

Any later acceptance still needs the approved CPU/latency/peak/noise gates,
measured cold behavior consistent with the narrowly accepted startup tradeoff,
and Stage B/resident/lifecycle/absolute/final-review work. Requested collision
storage removal remains shared per world, not per bot. No savings claim follows
from this source inspection.
