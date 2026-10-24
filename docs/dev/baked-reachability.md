# Baked reachability contract

Request: `BAKED-REACH-1023`, BR0. Status: accepted integration contract.

This contract fixes the combined `274V18` format and the BR1 ownership cut.
It does not enable a partial v18 reader or producer. BR2 changes the version,
reader, encoder, sizing, fixtures, identities, sidecars, and release bake together.
Until that cut, the product continues to read and produce `274V17`.
The nav integrator owns the format and release bake. No consumer track publishes
another v18. Revision content is immutable. Users do not rebuild its connectivity.

Authority: `MAIN-ACCEPTANCE.md`, `design.md`, `design-r2.md`, `design-r3.md`, and
`OPERATOR-DECISIONS.md` in the request evidence. The five carried corrections in
Main's acceptance are part of this contract.

## Wire rules

All integers are little-endian unless the existing v17 body states otherwise.
Every length counts bytes. Every count counts records. Offsets are relative to
the start of the section payload, not to its length prefix or the file.
No implicit padding is permitted. Reserved bytes must be zero.

Keep the existing body through the zone section in its existing order.
Set the whole-world version byte to 18 and `FORMAT_ID` to `274V18` only at BR2.
Append these fields in exactly this order:

1. Multiway keys: `u32 count`, followed by `count` sorted unique `u32` keys.
2. Labels: `u32 byte_length`, followed by the labels payload.
3. Graph: `u32 byte_length`, followed by the graph payload.
4. Placements: `u32 byte_length`, followed by the placements payload.
5. Stands: `u32 byte_length`, followed by the stands payload.

The S3 multiway key is `level << 28 | zx << 14 | zz`. Check `level < 4` and
both coordinate fields below `1 << 14` before packing. Bits 30 and 31 are zero.
Decode the list into `TransportGraph.multiway: Box<[u32]>`. The approved 289
source has 4,697 keys. This is a source expectation, not permission to truncate.
S3's older document calls this field v17. It belongs in this combined v18.

Each reach section starts with a `u32` subtable count and a directory.
A directory entry is `(kind: u32, offset: u32, count: u32, stride: u32)`.
Entries appear in the order below. The directory must contain exactly these
entries. Fixed rows have the stated stride. Variable byte tables have stride 1.
Tables occupy contiguous, non-overlapping ranges after the directory.
Empty tables have zero count and an offset at the current end.
Reject a duplicate, unknown, missing, overlapping, out-of-order, or trailing table.
All products and range ends use checked arithmetic.

### Labels payload

| Kind | Table | Fixed row or byte layout |
|---|---|---|
| 1 | Geometry | One row: origin x/z `i32`, width/height `u32`, planes `u32` = 4, chunk side `u32` = 64. Stride 24. Geometry must match collision. |
| 2 | Chunk directory | Plane-major, then chunk z, then chunk x. Row: palette start/count `u32`, index byte start/length `u32`, bit width `u8`, three zero bytes. Stride 20. |
| 3 | Palette | Sorted unique `u32` labels per chunk. Zero means uncovered. Nonzero label `n` names base component `n - 1`. Stride 4. |
| 4 | Packed indexes | LSB-first indexes in row-major 64 by 64 order. Minimum bit width is `ceil(log2(palette_count))`. One-entry palettes have width and payload length zero. Stride 1. |
| 5 | Coverage | Row: packed tile `u32`, reason `u32`. Stride 8. Sorted unique tile/reason pairs describe exceptional or incomplete coverage, never fabricated negative membership. |
| 6 | Exceptional transitions | Row: packed source/destination `u32`, edge reference `u32`, coverage reason `u32`. Stride 16. Includes legal non-standable landings and their outgoing transitions. |
| 7 | Legacy paint runs | Row: first collision cell `u32`, cell count `u32`. Stride 8. Sorted disjoint maximal runs. Uses existing transport-anchor and teleport-landing seeds. |
| 8 | Scatter runs | Same run encoding, restricted to the existing level-zero `standable && reached && can_step_off` predicate. Stride 8. |

A partial edge chunk still contains 4,096 index positions. Positions outside the
collision rectangle must use label zero. Palette/index bounds and unused high
payload bits are checked. The minimum bit width and payload length are exact.
Coverage reasons are `1 = unknown content state`, `2 = unmatched join`,
`3 = unsupported geometry`, `4 = unsupported trigger`, `5 = exceptional landing`.
Reason 5 is complete only when its transition records cover every legal outgoing
transition. Other reasons prevent a complete negative for the affected query.

Packed game tiles use x in bits 0..13, z in 14..27, and plane in 28..29.
Reject coordinates outside those fields. Bits 30 and 31 must be zero.
A collision cell is plane-major, then z, then x. Check the full four-plane
cell count fits `u32` before encoding runs. Never clamp a coordinate.

### Graph payload

| Kind | Table | Fixed row or byte layout |
|---|---|---|
| 1 | Component map | One condensed component `u32` for each base component. Stride 4. |
| 2 | Base sizes | One tile count `u32` for each base component. Stride 4. |
| 3 | Movement closure offsets | `base_count + 1` `u32` offsets into table 4. Stride 4. |
| 4 | Movement closure runs | `(first_base: u32, count: u32)`. Stride 8. Each closure includes itself. |
| 5 | Transport closure offsets | `condensed_count + 1` `u32` offsets into table 6. Stride 4. |
| 6 | Unconditional closure runs | `(first_condensed: u32, count: u32)`. Stride 8. Each closure includes itself. |
| 7 | Conditional CSR offsets | `condensed_count + 1` `u32` offsets into table 8. Stride 4. |
| 8 | Conditional joins | `(destination_condensed: u32, edge_reference: u32)`. Source is the CSR row. Stride 8. Sorted unique records within each source. |
| 9 | Teleport templates | `(teleport_reference: u32, landing_base: u32, coverage_reason: u32)`. Stride 12. Source predicates remain runtime predicates. |
| 10 | Session templates | `(kind: u32, source_base: u32, landing_base: u32, source_tile: u32, landing_tile: u32, selector: u32, coverage_reason: u32)`. Stride 28. Kind 1 is an essence exit. |

Base components are movement strongly connected components. Movement closure
preserves any directed movement between them. Condensed components additionally
condense unconditional transport arcs. Store only unconditional forward closure,
not undirected union-find connectivity. Runtime traversal reads these closures.
An unconditional edge has no account, membership, source, consumption, return,
quest, exclusion, equipment, skill, or item predicate.

Local edge references index the decoded local `TransportGraph.edges` order.
Teleport references index its decoded teleport order. Both orders follow the
existing encoded edge order within their class. They never name a raw address.
Conditional joins retain every admissible takeoff and actual landing. A union of
several stands must not erase an executable join. Exceptional transitions use the
same local edge namespace. No copied requirement table is added.

The session selector names the existing essence wizard/return-anchor identity.
The bake covers all portal placements and return anchors. Runtime activates only
the bound slot's live essence latch. No account latch or executable closure is
serialized. Missing session coverage prevents a sealed-mine negative.

CSR offsets start at zero, are monotonic, and end at the target table count.
Each run is nonempty and in bounds. Closure runs are sorted, disjoint, maximal,
and include the owning component. Every base id maps to an existing condensed id.
The empty world has zero base/condensed components and one zero CSR offset.
Component sizes count covered tiles only. A component id is never a route cost.

### Placements and stands payloads

Placements has one table, kind 1, stride 24. Each row contains packed origin
`u32`, loc id `i32`, shape `u8`, angle `u8`, coverage `u8`, trigger `u8`,
stand start/count `u32`, and coverage detail `u32`.
Rows sort by `(plane, x, z, loc_id, shape, angle)` and are unique.
Angles are 0..3. Shape must name an engine-supported strategy or explicit
unsupported coverage. `coverage = 0` means complete, 1 means unsupported geometry,
2 means unsupported trigger, and 3 means unknown content state. `trigger = 0`
means op, 1 means ap, and 2 means a content-state template. Coverage detail names
the labels coverage reason, or zero for complete coverage.

Stands has one table, kind 1, stride 8. A row is `(packed_tile: u32, base: u32)`.
Each placement range is in bounds, sorted by packed tile, and duplicate-free.
A complete placement with no legal stands remains present with count zero.
Unsupported geometry or an ap trigger must not masquerade as that empty proof.
Every stored stand component must equal the labels lookup during qualification.
Apply `game_plane`, including `link_below`, before recording placements or stands.
Reuse the shared engine reach primitive for walls, decorations, footprints,
rotation, blocked sides, and exclusive entity rectangles. Never use a radius ring
as an interaction proof. Live entities retain observed identities and geometry.

### Decoder and publication requirements

Both decoding forms and the sizing path implement this complete layout.
Malformed lengths, keys, geometry, palettes, ids, ranges, references, or ordering
produce typed `PackError`. Decode never constructs labels or floods connectivity.
A v17 pack receives the typed version failure after the BR2 cut. There is no
v17-to-v18 runtime migration, partial reach-only pack, or straight-line fallback.
The release bake publishes the combined pack and rebound manifest/sidecars once.
Qualify 274 and 289. Print section sizes, coverage, exceptional joins, and the
delta from the retained 78,365,101-byte projection. That projection is no ceiling.
Resolve all 18 unmatched prototype joins before claiming complete coverage.

## Borrowed atlas views

`NavWorld` owns one private, non-Clone `ReachabilityAtlas`. It retains compressed
labels and immutable tables. `NavLease::reachability()` returns a borrowed
`ReachabilityView`. Binding more bots clones only the existing world Arc.
No runtime dense labels, whole-world floods, or per-idle-bot reach bitset exists.
Admitted jobs own their temporary account traversal scratch.

The closed facade exposes these meanings, not raw atlas or transport storage:

- `BoundComponentKey`: private pack identity and component index.
- `LocPlacementKey`: checked game tile, loc id, shape, and angle, with accessors.
- `component_at(WorldTile) -> Result<BoundComponentKey, ReachabilityError>`.
- `loc_stands(LocPlacementKey) -> Result<LocStandView, ReachabilityError>`.
- Borrowed movement membership and sizes, legacy paint membership, and scatter
  tile runs. Movement display does not acquire account transport closure.

`ReachabilityError` is a thiserror enum: `MissingComponent`, `MissingPlacement`,
`UnsupportedGeometry`, and `IdentityMismatch`. Missing coverage cannot mean zero
stands or impossible geography. Component keys from another pack are rejected.
If exact routing succeeds from missing membership, its witness wins. Otherwise
preserve a coverage-specific `Undecided` result. A failed sidecar identity is not
bypassed by another calculation. A missing atlas displays unknown coverage.

## Account proof and budgets

One planner-internal tri-state edge evaluator supplies `Allowed`, `Denied`, or
`Unknown(MissingRequirement)`. `WorldState::allows` is its strict Allowed wrapper.
Reuse the existing Strict, CarryWorn, UnknownQuest, EntranceExclusions, and
CarryWornUnknownQuest modes. Missing observations remain unknown. Observed false
quest gates remain false in modes that retain them. Inventory-growth proof is
not a bank-fetch permission. Exact forward labels retain consumption and returns.

A proof keeps one disposition per permitted remedy mode. Its possible topology
is the safe union of every permitted mode. A remedy-only goal skips a strict
search the atlas already proves futile, but retains the real remedy diagnosis,
bank access/session, and strict post-session route. It never executes a relaxed
witness. The internal arc-source iterator covers packed edges, permitted any-tile
teleports, and the live essence session in both routing and atlas traversal.

Only the planner mints `SelectionNegative` and `GoalReachProof`. Bind source,
exact goals, nav identity, observation evidence stamp, context generation,
account/bank facts, session latch, policy, and remedy permissions. A complete
negative requires covered source/goals, complete applicable topology, and no
possible goal under any permitted remedy. Unknown facts are not negative proofs.

Atlas-positive and incomplete queries immediately preflight the ordinary full
bounded forward scratch budget. They do not wait for the smaller First cap or a
static reverse flood. Preferred/fallback sets retain independent proof states.
A better undecided tier cannot silently lose to a lower tier. Pack-bound v18
production and diagnosis never start `ReverseClosure`. Synthetic test worlds may
retain the bounded legacy path. Zones, wilderness admission, avoids, cumulative
supplies, and dynamic collision still require exact routing. Budget, deadline,
and storage stops remain `Undecided`, never exhaustive negatives.

## Picker and pass-end results

The host owns captured account facts and the existing admitted route job.
Scripts use the closed semantic facade. Do not expose `WorldState` or raw edges.
The result vocabulary is:

- `Routed { candidate, stand, route, bank_session, input }`. `bank_session` is an
  optional owned existing host `BankSession`, not part of a compact route lease.
  A bank-assisted result executes that session before its post-session route.
- `NoCandidate`: the semantic eligible set is empty.
- `NoReachableTarget { proof }`: a complete identity-bound negative.
- `Blocked { failure }` and `Refused { failure }`: typed final diagnosis, with
  the original walk end/refusal and cold detail retained.
- `NeedsEvidence { requirements: Arc<[QuestGate]> }`.
- `Undecided { reason }`: coverage, missing input, node count, deadline, or
  required/maximum storage bytes.
- `Stale { expected, observed }` and `Cancelled`.

Preserve every candidate-to-stand association. Use one First job across eligible
goals, not one query per target. BestTier selects the best reachable tier, then
route cost. Nearest selects route cost first, then existing stable ties. Do not
truncate a catalog to 1,024 air-ranked goals. Larger sets need admitted storage
or pages that establish the global cost winner. Final WalkGoals remains checked.

Freeze a finite eligible identity set per pass. Mark an identity tried before
its first approach. Each identity receives at most two total attempts, with a
fresh observation before the second. Alternative stands do not reset the budget.
Changing target starts that target's own episode, but never restores a tried
identity. N identities allow at most 2N attempts and max(N-1, 0) target changes.
A whole-set negative receives at most one fresh planning attempt, not 2N repeats.
Admission deferral does not spend attempts. Hold/Pause preserve the pass. Stop,
revocation, UserInput, Cancelled, and NeedsEvidence do not trigger autonomous retry.
Never repeat an unsettled transfer, purchase, or item use to recover movement.

At pass end apply this order:

1. Any remaining undecided candidate yields `ResourceBlock::ReachabilityUndecided`.
   Keep the first reason in stable candidate order and the last typed walk failure.
2. Remaining evidence yields `ResourceBlock::NeedsEvidence` with typed gates.
3. Any whole-set Blocked or Refused diagnosis yields
   `ResourceBlock::Diagnosed { failure }`. Preserve the typed end and detail.
   This is neither no reachable resource nor undecided.
4. Only an all-negative or approach-exhausted set yields
   `ResourceBlock::NoReachableResource { cause, last_walk }`. Its cause separates
   complete negative from finite approach exhaustion and retains candidate counts.

Parked blocks remain Running/Blocked with responsive Pause and Stop. Do not
search every tick. Relevant changed account, world, policy, or external-position
facts can start a new pass. Successful interaction, a completed bank/new trip,
a completed site move, or a new operator task has the documented reset scope.
A site move does not reset the enclosing Q7 tried/shunned site set.
Availability-only reconsideration preserves exhausted identities and unchanged
negative records. A respawn alone does not search a known-negative input again
or restore attempts. Scene generations, partial autonomous progress, ticks, and
stand changes cannot create an unbounded loop.

## BR1 ownership and checker delta

Create `nav-bake` as a workspace producer/tool crate. It owns bake orchestration,
bundle staging and input inventories, manifest assembly, 26 transport derivers
and parser support, collision construction, config/mapsquare parsing, bank stand
generation, pack/sidecar encoding, canlight generation, POI generation, and the
three bins `nav-pack`, `cache-content-id`, and `nav-input-audit`.

Split runtime schemas before moving their producers. Planner keeps manifest
schemas, hashes, verification, and `manifest::NavIdentityRow`. Remove the old
bundle identity export. Keep runtime collision/transport/zone/map types,
decoders, policy identities, `collision::pack_walk`, cart fare calculation, web
action policy, Wilderness rules, router, and facade in planner. Extract `step_ok`
into planner's movement module and share it unchanged. Keep `paint::bake_reach`
and flood helpers in planner through BR1. BR2 migrates their runtime callers to
the atlas, then removes obsolete flood APIs and caches.

The fixed `required-content-274.tsv` and `required-content-289.tsv` tables stay
in planner. Quester coverage and nightly staging also read these shared input
facts. The baker includes and hashes the same tables; runtime callers do not
include producer-owned files or add a baker dependency.

The paired operator-owned architecture patch admits exactly these additions:

| From | To | Kind |
|---|---|---|
| nav-bake | nav-planner, api, client cache decoder | normal |
| host-play | nav-bake | build and dev |
| host-play | nav-planner | existing normal and retained build |
| e2e | nav-bake | normal tool edge |
| panel, frontend-core, tui | nav-bake | dev only, for producer fixtures |

Planner, executor, script, script-js, frontend-core, TUI, panel, and runtime
host-play have no normal edge to the baker. Checker tests reject those edges,
including otherwise identical normal-edge versions of admitted dev/build fixtures.
Update crate
membership, member ownership, moved bins, source inventories, deny-list paths,
and old exceptions together. Scripts cannot import raw atlas storage.

Use generator id `nav-bake-3` and workspace-relative source labels. Hash actual
producers, schemas, encoder, shared reach rules, source inputs, and movement.
Do not hash all router search code merely to include `step_ok`. Generator-only
changes do not alter the revision cache id. BR1 has one planned identity cut,
zero changed routes, and byte-identical bake outputs except identity fields.
No intermediate release rebake is published.

`host-play` retains planner as a build dependency for CacheManifest, hash_file,
and FORMAT_ID. It adds the baker build edge and named producer dev edge.
`BOT_NAV_BUILD=skip` performs no bake/content scan, but Cargo still compiles its
nonoptional build dependencies. This cut claims no source-build compilation
saving from an environment flag. Release binaries load shipped packs without
compiling or baking. Cargo-tree evidence distinguishes normal, build, and dev
paths and must not claim that nonoptional build dependencies disappear.

BR2 and BR3 require pre-FLOOR before consumers. Remedy-only goals and
atlas-positive policy-negative wilderness/zone cases belong in that case set.
BR1 requires zero route-corpus differences, artifact byte comparison with
identity fields identified, full CI parity against the prepared checker, and
normal/build/dev cargo-tree evidence. The operator publishes the paired policy.
