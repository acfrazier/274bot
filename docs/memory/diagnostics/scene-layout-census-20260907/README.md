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
The replacement arithmetic is an explicitly conservative sketch: 68-byte
`GroundStamp` arena entries, one u32 tile stamp index, one u32 free-list entry
per stamp, and six i32 plus one byte per occupied tile for proposed renderer
side arrays. Allocator metadata, Vec capacity slack, alignment, and retained
simulation fields are excluded and must be measured separately before any
migration.
