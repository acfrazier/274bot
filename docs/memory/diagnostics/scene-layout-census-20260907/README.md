# Scene layout census experiment

This is a bounded, isolated layout experiment. It depends read-only on the
checked-out client crate and does not change the client or host production path.

Run from this directory:

```text
cargo run --quiet
```

The fixture deliberately exercises two different dimensions of scene state:
scene A has a stacked tile and a `push_down`; scene B has a different overlay
shape/rotation/texture pattern, sparse clusters, and a different `push_down`.
These are executable structural fixtures, not captures of a shipped map. Their
counts must not be generalized to native workload occupancy.

The program prints `size_of` results from the actual client types and counts
occupied slots, linked-square depth, and overlay stamps after the operations.
The replacement arithmetic is an explicitly conservative layout bound: 68-byte
`GroundStamp` arena entries, one u32 tile index, one u32 free-list entry per
stamp, a complete retained simulation-core record per `Square` (including linked
boxes), and six i32 plus one byte per retained `Square` for proposed renderer
side arrays. The executable defines that core with target alignment and includes
coordinates, levels, sprite indices/spans, quick ground, all retained object
handles, linked identity, sprite counts, and model invalidation. Allocator
metadata, Vec capacity slack, and the separately owned pointed-to objects remain
outside this payload bound and must be measured before migration.
