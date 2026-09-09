# Bounded 32×32 tiled collision storage design

Status: proposed representation and experiment contract, `t_71e88dd7`.
Documentation only; no runtime implementation, new census or measurements are
released here. Read [native census report](nav-uniform-census-native-report.md)
for provenance and its histogram limitation. The 16 MiB element screen passes,
justifying this one design, not claimed RSS savings. No tile-size search,
uniform-pair dictionary, JagFX experiment, client change or bot action API.

## 1. Decision and release boundary

Use one immutable level-major directory of 32×32 spatial tiles: inline full
(face u8, blocked bool) for uniform tiles, otherwise index one flat dense-tile
pool. Keep four independent planes. Do not deduplicate dense tiles, run-length
encode rows or add a lazy expansion cache. A route/query performs bounded direct
lookups, not decompression or allocation. The world remains shared through the
existing `Arc<NavWorld>`; no per-bot collision instance.

There is a real Rust source-API conflict: `WorldCollision.walk: Vec<u8>` and
`blocked: Vec<u64>` are PUBLIC, as are geometry/flags, and external callers can
construct literals, mutate/index buffers and borrow contiguous slices. A tiled
replacement cannot implement those Vec contracts without retaining/materializing
the dense map. `Index` returning a reference, `Deref<[u8]>`, a hidden dense cache,
or two empty compatibility Vecs are NOT acceptable substitutes.

Selected migration proposal: make collision storage private, retain existing
query method signatures and the `WorldCollision`/`NavWorld` types, and provide
explicit owned packed-parts construction, allocation-free logical reads and
iteration, and explicit dense export only for callers who request ownership.
Freeze geometry too: replace writable origin/width/height fields with read-only
`origin()`, `width()` and `height()` accessors. Otherwise changing a public
dimension after construction would invalidate the tile directory's indexing.
Migrate host geometry reads mechanically and include them in the API review;
do not keep writable headers with stale cached tile dimensions. `flags` and
its existing attach/drop behavior remain separate from immutable packed storage.
This is an acknowledged Rust source-breaking change, NOT backwards-compatible
merely because `nav` is `publish=false`. Before implementation release root
must approve that migration and establish whether downstream Rust consumers
outside this checkout exist. If the public Vec surface must remain compatible,
park this candidate and return for a separate API design; do not secretly
retain dense storage or introduce another world type across router signatures.
The design review alone does not waive that decision. No JS API change follows.

Proposed names below are new APIs, not symbols claimed to exist today:

- `WorldCollision::from_packed_parts(origin, width, height, walk, blocked, flags)`:
  consumes vectors and freezes storage; a checked result for constructor shape
  errors, separate from existing wire errors. Keep `pack_walk` and
  `derive_walkable` as existing public dense utility functions.
- `logical_cell_count()`, `packed_pair_at(index) -> Option<(u8,bool)>`,
  `packed_pairs()` in original level/z/x order, and `packed_blocked_words()`
  in original global 64-cell order. Iterators allocate nothing.
- `to_packed_parts()` explicitly returns owned dense buffers, documented O(cells)
  memory/time and never called in load, routing, paint or per-bot paths. A
  caller edits those owned vectors then consumes them in construction again;
  no hidden COW, per-cell mutation of shared storage or expansion-on-read.

Constructor contract to approve: positive checked dimensions with at most four
planes; support complete 1–4 plane buffers for existing synthetic worlds,
remember exact logical cell count and blocked word count, and reject inconsistent
buffer lengths explicitly. Native v8 decode always constructs exactly four.
No claim is made to preserve arbitrary malformed public struct literals or
independent Vec mutation into inconsistent lengths; that is part of the API
break requiring approval. Existing decoder errors/acceptance must remain exact,
not be silently replaced by constructor validation. Test-only construction may
use complete single-plane shapes rather than fabricate upper data. A query
inside geometric bounds but beyond a synthetic short buffer must retain the
baseline behavior (including panic where it currently indexes absent data);
`packed_pair_at` returning None is not permission to turn it into open ground.
The panel's explicit short-buffer guard must continue to skip missing planes.

## 2. Current compiled source/consumer audit

Audit anchor: host `f2195a1ce186ac63b4c5516ea80d8bd5736b9ae0` on the named
campaign checkout. Nav/api source bytes match frozen census host c0709aba;
compiled client inputs match 3456edc8. This is a source audit, not a new build.
Workspace-wide searches for `.walk`, `.blocked` and `WorldCollision {` were
traced to distinguish unrelated interactions/StepGrid fields from collision.

| Current source | Actual constraint / planned handling |
|---|---|
| `crates/nav/src/collision.rs:57-80,96-98,155-224` | Public Vec fields and literal construction; `walkable_word`, `walkable`, `standable` index them. Replace internal reads with pair lookup; preserve raw-sidecar branches and outside/level behavior. |
| `collision.rs:282-389,400-443` | Bake mutates raw u32 flags, then `pack_walk` produces face/bit vectors. Keep stamp/derive semantics; consume packed temporary once when freezing. Bake's raw flags remain for sidecar export, not production decode. |
| `crates/nav/src/pack.rs:256-301` | `encode(&WorldCollision, &TransportGraph, &[BankStand]) -> Vec<u8>` uses Vec len, slice and u64 iteration. Keep signature and wire; stream logical faces then logical blocked words into its existing owned output, never create an intermediate full collision copy. |
| `pack.rs:313-416` | `decode(&[u8])` allocates dense walk and blocked, then edges/indices/banks; returns same tuple. Freeze collision only after successful ordinary parsing, drop dense sources before returning. |
| `crates/nav/src/world.rs:49-71,105-166` | `load_pack` owns file bytes, BadMagic-only legacy fallback; `from_parts` moves ownership; `from_grid` creates four planes with only L0 boolean stamps. Preserve behavior and signatures except explicit constructor migration inside these functions. |
| `crates/nav/src/paint.rs:467-492` | `bake_reach` sizes a separate paint-only bitset from `walk.len()`, then floods through `step_ok`. Use logical cell count, not padded tile count; do not remove reach storage or claim it is collision savings. |
| `crates/panel/src/picker.rs:213-233` | `available_levels` reads both Vecs and has a synthetic short-plane length guard. Retain ground level and exact any-nonzero-pair definition; logical scan or a per-plane summary computed at freeze, with no raw-sidecar substitution. |
| `crates/nav/src/router.rs:165-350,606-650` | All find variants consume `&WorldCollision`; directional `step_ok` goes through `walkable_word`, including destination/corner/orthogonal masks. Do not edit policies/neighbor order to compensate for lookup cost. |
| `crates/host-play/src/lib.rs:3181,3207-3212,2364-2433` | One world loaded per Play, Arc cloned to panel/slots and outstanding request; preserve sharing, off-pump find and token/generation checks. This is not a global cache shared between unrelated Play instances. |

Direct Vec writes outside collision construction found in host crates are test
fixtures, notably `host-play/src/lib.rs:5197-5198` (one footprint bit and one
face-only wall). Migrate these by editing owned packed input BEFORE freeze, not
by weakening standability checks. Vec equality/length/index assertions in nav
world/pack/paint tests must become exhaustive logical equality and explicit
layout assertions, not be deleted. Literal builders also occur in nav
bank_fetch/router/transport/collision/paint/world/pack tests, host-play
scatter/memory/lib tests, tui bin tests, and panel picker/overlay/session tests.
These are source-API consumers requiring compilation coverage, not evidence of
production maps duplicated per bot. Diagnostic census code also intentionally
binds the old Vec API to frozen source: do not rewrite the immutable tool or its
raw output to compile against a future API.

Unrelated `.walk` matches include `StepGrid` (`grid.rs`, legacy pack merge),
Traveller's walk-hop state, script closures, interaction calls and TUI callbacks.
They are not collision storage migrations. Panel session paints and transport
bank/approach paths query collision methods; no direct runtime Vec mutation was
found there. Preserve optional raw flags in `attach_flags`/`drop_flags` and the
panel-held side table (`collision_at_with`); it is not compressed by this work.

## 3. Exact storage and construction

Descriptor: explicit u64, not a Rust enum with unspecified layout. Bit 63 clear
means uniform, low 9 bits encode `face | blocked<<8`, all other bits zero.
Bit 63 set means dense, low 32 bits are pool index, all remaining bits zero.
Use checked index/count conversions and assertions on generated descriptors;
never reinterpret input bytes as descriptors. Maximum accepted wire dimensions
fit this index scheme, but check arithmetic rather than relying on that fact.

Directory order: `level * tile_rows * tile_cols + (local_z/32)*tile_cols +
local_x/32`. Dense local cell order: `(local_z%32)*32 + local_x%32`. Each pool
entry has `[u8;1024]` followed by `[u64;16]`, explicit C layout and 8-byte
alignment, 1,152 element bytes. No per-entry Box, Vec or Arc. Use two exact-sized
boxed slices (directory and dense pool) owned directly by collision. Validate
`size_of`/alignment on the measurement target before relying on these sizes.
No dictionary, hash table or uniform-value array is necessary.

Keep logical count and the original last blocked-word unused high bits as a
scalar. Those high bits are not legal cells but current encode preserves them;
zeroing them would change bytes on non-word-aligned packs. Reconstruct global
blocked words across rows/planes, not concatenate tile-local u64 words. Retain
original geometry and origin.level in the header; lookup plane selection still
uses the queried level 0..3, not subtraction of origin.level.

Uniformity compares all VALID cells' exact nine-bit pairs. Partial right/bottom
tiles use deterministic zero padding in dense payloads, never exposed through
queries or encoding; padding must not disqualify a valid-cell uniform tile.
Absent synthetic planes are not present uniform tiles. All 256 face values are
preserved even when blocked=true; blocked never substitutes all faces set.

Layout accounting on a 64-bit target (proposed, not measured): two fat boxed
slice headers occupy 32 bytes; logical count, tile_cols, tile_rows and last-word
padding bookkeeping can occupy four 8-byte scalars, another 32 bytes. Thus budget
64 bytes for storage metadata before surrounding WorldCollision padding, geometry,
flags Option, NavWorld graph/banks, and the existing Arc counters/allocation.
Assert actual aggregate sizes; do not treat this illustrative header budget as
an ABI guarantee. Directory and pool require two allocations plus existing world
ownership; include allocator usable sizes/page rounding separately. Existing
Vec headers are removed, so do not count both sets as retained costs.

For the observed pack: directory 508,928 bytes, dense pool 3,559,680 bytes,
elements 4,068,608 bytes. Headers/rounding are additional. Graph edge requirements,
teleport vectors/at index and bank strings remain; census counts are not byte
estimates. Optional flags, reach bitmap, route search maps/queues, retained routes,
input/encode buffers and allocator pages all remain in the residual ledger.
Savings are fixed per loaded Play world, not multiplied by 16 bots.

### Construction, peak and reclamation

First candidate deliberately preserves the existing dense decoder/parser. On
successful parse, perform a scalar counting pass over the dense vectors, allocate
exact directory/pool capacity, then a fill pass. Use bounded one-tile stack
scratch at most; never a second whole-world pair map. The count/fill passes are
serialized during Play construction before Arc publication, not on slot startup,
first query or inside a route lock. Finish/freeze exactly once. Drop BOTH dense
input vectors before returning the collision. No retained dense shadow, cached
encode bytes, per-slot directory or lazy duplicate decoder.

The borrowed navpack bytes remain live through decode; `load_pack` frees its
file Vec on return. Peak requested element overlap for this input is at least
73,438,581 input + 73,285,632 old collision + 4,068,608 candidate = 150,792,821
bytes, before graph/banks, headers, allocator rounding and other Play startup
owners. This is an overlap model, not an observed peak. Exact allocation avoids
a grow-and-shrink pool overlap; implementation must report any allocation/copy
introduced by conversion to boxed slices. Do not promise returned pages on Vec
drop or use allocator purge calls to manufacture the resident comparison.

Baking still owns raw u32 flags for sidecar output; quantify its separate peak,
but do not optimize the bake in this hop. Legacy `from_grid` remains a separate
conversion path with its current flags temporary. Encoding still necessarily
allocates the public returned wire Vec; tiled layout does not shrink v8 files.
A direct-from-wire tiled decoder could reduce startup overlap later, but is NOT
this experiment: changing error precedence and parsing while changing storage
would confound the bounded test.

No new process-global cache: Play's existing one-time construction and Arc
clones are enough. Concurrent Play instances remain separate ownership domains.
Outstanding route workers/panel handles may extend world lifetime after Stop;
release only after the last legitimate Arc drops. Do not free collision while
retained handles read it, or advertise post-Stop zero nav memory while Play
intentionally keeps the world. Future cache/hot-reload changes require separate
design, keys, failure policy and ownership evidence.

## 4. Behavior invariants (not opportunities to fix policy)

- Same full `walk_word_from_parts`, `flag`, `walkable_word`, `walkable`,
  `standable` and nearest-walkable results on all legal cells, empty upper
  planes, out-of-grid and invalid levels, both sidecar states. Flag queries and
  walk-word queries return zero outside/invalid; walkable/standable return false.
  Sidecar branch masks are not interchangeable with a simplistic pair==0 test.
- Keep router entry validation and `step_ok` as-is, including any existing
  quirks of invalid-level direct internal calls. Do not infer that preserving
  collision query bounds authorizes extra route validation. Eight-direction
  masks, corner clearance, deterministic neighbor/tie order, cost model,
  missing-requirement behavior, transports/teleports/essence/banking options,
  approach radius and no-path results remain exact.
- Wire stays `274V` version 8, faces in original level/z/x order followed by
  global u64le blocked words, ordinary edges then teleports, requirements and
  banks in the same order. Preserve graph.at index order and teleport exclusion.
  No v7 acceptance, flags-to-walk fallback, new trailing-byte rejection, sorted
  requirements or changed malformed UTF-8/tag errors. `decode` currently leaves
  extra trailing bytes unread; preserve this. Raw sidecar format stays `274F` v1.
- `NavWorld::load_pack` keeps BadMagic-only `274N` fallback and current second
  file read; stale versions, truncation, dimension caps, I/O and BadLength
  messages/error precedence retain their existing paths. Successful parse is
  required before conversion; no partial favorable world on failure.
- `host-play/src/lib.rs:2392-2443,9700-9730`: keep worker identity tokens,
  wrapping request generation, pending request replacement and spawn-failure
  cleanup. Stale result does not publish. NoPath clears requested_route so the
  same destination can retry, but retains previous route/Traveller state;
  success clears Traveller and replaces route/bank session. Do not clear the
  old route early to gain memory or cache failed requests indefinitely.
- Preserve hold/pause/resume, follow terminal clearing, script Stop/restart,
  slot removal/join and old Arc reader lifetime. Collision is not a mutable
  live-scene overlay. Preserve current incomplete features/errors as well as
  successful routes. No action/tick/protocol or renderer behavior changes.
- The panel/TUI `WalkArm` is separate from the script NavBot:
  `host-play/src/lib.rs:392-459` returns Err(NoPath) without touching the old
  arm; only successful route/bank-session outcomes clear Traveller and replace
  it. Preserve both lanes, including no focused slot (return a route without
  creating an arm). Add old-arm retention assertions to this lane as well.

## 5. Required correctness proof before measurement release

Tests must compare independent dense access to tiled access, not two calls
through the same new helper. Retain dense oracle code in test-only fixtures;
do not keep it in production or compare its resident memory in the candidate.

1. Exhaust all 512 pairs as uniform tiles, including every face byte with both
   blocked states. For mixed tiles, deterministic seeded patterns and each
   single differing cell; exercise edges 31/32 and 63/64, dense indices across
   levels, all-uniform and all-dense. No estimate from a favorable fixture alone.
2. Dimensions 1, 31, 32, 33, 63, 64, 65 in combinations with nonzero/negative
   origins and distinct four-plane contents; partial boundary padding,
   non-word-aligned plane joins, unused final blocked bits, empty upper planes
   and existing complete single-plane synthetic fixtures. Exhaustive valid-cell
   pair/read/derived-word comparisons, plus invalid/outside probes. Exercise
   raw sidecar attach/drop and external side table, standable vs face walls.
3. For one separately authorized hash-bound real pack, compare EVERY logical
   cell and blocked word against the frozen dense oracle, not a sample. Compare
   canonical encode bytes old-vs-new, encode/decode roundtrips, complete ordered
   graph fields/requirements/indices/teleports and banks. Input decode→encode
   need not equal input for noncanonical interleaved teleport/trailing streams;
   compare to BASELINE encoding, and preserve baseline accepted trailing bytes.
4. Differential malformed-wire corpus: each header truncation and section
   boundary; versions, zero/oversize/overflow dimensions, short face/bit data,
   short/invalid edges/requirements/banks/sidecars and bad UTF-8/tags. Compare
   exact error variant/payload and fallback decision to dense baseline. Do not
   force raw decode failures through the new public constructor's error type.
5. Exhaust all start/destination pairs in bounded small maps using all find
   options/cost models represented in existing tests; compare complete route
   legs, tiles, edge identity/order, costs and NoPath, not just reachability.
   Separately exhaust local 3×3 directional/corner mask cases with a dense
   oracle and include tile/plane boundaries. Real fixed route corpus: long
   walks, sealed destination, doors, stairs, requirements, bank-fetch,
   teleports-disabled/enabled, essence and radius approach; freeze selectors
   before running either arm. Differential nondeterminism is a finding, not
   permission to weaken exact expected results.
6. Keep host-play tests
   `route_publication_rejects_stale_results_and_preserves_route_on_failure` and
   `failed_radius_search_can_retry_same_destination_with_old_route_retained`;
   add synchronized retained-world reader/drop tests with bounded join and
   weak-Arc checks. Prove one storage identity across N=16 consumers and no
   conversion on reads, focus changes, attach/drop or route replacement.
7. Compile/test affected nav, host-play, panel and tui targets with their normal
   and memory-profile feature combinations; cover all workspace literal users.
   No skipped local-pack test counts as real-pack proof. Required client
   integration suite is separate if a later scope explicitly touches client;
   this design does not authorize such a change. No Cargo runs occurred here.

## 6. Predeclared bounded matched experiment (future authorization only)

Freeze baseline/candidate host and identical client commits, compiler/target,
release flags, lockfiles, System allocator, snapshot-dedup setting, navpack hash,
cache/scene, account roster, route corpus, frontend/audio/lowmem settings and
all workload scripts. Save binary hashes, hardware/OS entitlement and a pairing
manifest BEFORE launch. Do not pair a current highmem vault run with a throwaway
lowmem control. Actual GPU backend and frame cadence must qualify; unavailable
hardware is pending evidence. No profiler/counting allocator in clean CPU/RSS
comparisons; separate layout instrumentation runs cannot substitute for them.

Stage A, after correctness review: a cold-load/drop and frozen-route microbench
with dense and candidate binaries run separately, equal repetitions/order,
explicit input buffer lifetime, construction duration, process CPU, lookup and
route latency, requested capacities/usable bytes and RSS sampling. All-uniform
and all-dense synthetic extremes detect lookup and expansion risks, without a
new tile-size selection. Gate: observed candidate allocation layout matches
count-derived directory/pool and baseline vectors have been dropped; zero
read-time collision allocations; no startup RSS peak increase beyond 8 MiB,
no cold-load median elapsed increase beyond 10%, route CPU increase ≤5% and
route p99 increase ≤2 ms. These are task-specific proposed gates, not results;
record baseline repeat variation first, declare inconclusive if it masks them.
Failure parks/reviews the candidate, not a switch to a different tile size.

Stage B: required matrix N=1 and N=16, both idle and active, in each of:
TUI RasterOff, panel with ONE full-rate GPU renderer, and panel focused full-rate
GPU plus other rendered bots at 1 fps with normal simulation. Compare within
mode, never average modes or scale a shared saving by N. Use three fresh paired
screens per cell with 30s warmup / 120s observe / 60s settle; alternate baseline
and candidate order. Freeze actual order and route/input schedule in manifest.
Measure per-run steady median and peak RSS, whole-process CPU seconds/wall,
per-bot qualified progress/pump cadence, route latency, decoded-update→dispatch
and focused-input→action p99, frontend responsiveness and real frame cadence.
Include nav-enabled work so an idle route-free fixture cannot hide lookup costs.

Resident retention gate for this task: at least 16 MiB lower steady median RSS
in EVERY matched matrix cell, and reduction larger than that cell's measured
same-state repeat variation. CPU must not regress >5%; each latency p99 must
not regress >2 ms. No averaged pass over failing cells, no dropping failures,
no allocator purging. Report N16−N1 finite differences: expected collision
improvement is fixed, not a new per-bot slope claim. Report raw deltas for each
pair plus repeat/noise ranges; if noise overlaps a gate classify inconclusive.
A concrete allocation removal alone can motivate further review but is not this
design's claimed resident win. Preserve stricter campaign absolute budgets as
final gates, not replaced by these differential thresholds.

For a candidate worth retaining, one longer confirmation stage uses three fresh
pairs per required cell, 120s warmup / 600s observation, and explicit observe-end,
script Stop, restart, slot-stop/post-join and Play-last-owner-drop markers.
Hold an old world reader deliberately in a bounded test; distinguish intended
Play retention from leaked owners and allocator page retention. Inspect final
three same-state post-Stop windows against repeat noise; no growing live
collision owner count. Include focus/paint-sidecar toggles and scene transitions
without changing renderer/audio configuration. CPU/RSS costs of sidecar/reach
storage must be visible separately, not attributed to tiled storage.

Stop after paired screens and this one confirmation stage: identify a named
confounder for root decision or park, never rerun until favorable. Incorrect
cell/route/wire/lifecycle behavior rejects the candidate regardless of memory.
Microbench win without matched resident matrix is not acceptance. Final campaign
N32/lifecycle, target-hardware and absolute resource gates remain governed by
`performance-finish-plan.md`; this bounded representation experiment does not
claim to close them or authorize them on this card.

## Operator decision — 2026-09-09: measured cold-load exception

The operator accepted the measured allocation/startup tradeoff: “I think the
allocation tradeoff is worth the extra 344ms here, unless you disagree.” Root
agrees this justifies continued qualification and provisional retention.

This is a specific exception to the original Stage A +10% cold-load gate for
frozen tiled8385babb versus dense29b7aea on the measured274 pack. The completed
six-pair screen measured108.637ms versus452.555ms median (+343.919ms), with
69,217,024 fewer requested collision-element bytes (66.0105MiB). See
[the measured report](nav-stage-a-cold-screen-report.md). The original failure
classification and raw evidence remain accurate under the original gate;
the operator has explicitly accepted this particular startup cost.

The exception permits provisional retention and further testing. It does not
establish deployed resident savings, accept the one-row route metrics as the
full routing gate, permit unbounded future startup regressions, change process
resource caps, or waive CPU/latency, startup-peak, Stage B, lifecycle, target
hardware, absolute campaign budgets or final review. The interrupted59-case
routing run remains incomplete. A separately reviewed resource/phase plan is
required before another full-routing measurement; do not retry the old release
or silently raise its caps. No source reversion is required on the basis of
this accepted cold-load tradeoff alone.

## 7. Risk and next authorization

Extra division/indexing, a data-dependent descriptor branch and another memory
access may hurt random router expansion; diagonals make several queries per
step. Full-grid paints and encode can lose contiguous bandwidth. Two conversion
passes add cold-start CPU, and input+dense+candidate overlap can erase startup
benefit while allocator page retention can erase RSS benefit. Dense input extremes
can expand slightly versus baseline. These are measured failure modes, not
reasons to change path order or retain dense caches.

Root must resolve the explicit public Rust migration, obtain independent
accounting/design approval, then issue a bounded implementation/testing card
with this contract and frozen pairing. Integration review precedes native
measurement release. No implementation or second diagnostic follows automatically
from committing this document. STATE remains root-owned.
