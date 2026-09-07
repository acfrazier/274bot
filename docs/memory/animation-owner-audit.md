# Animation owner audit

This is a source reconciliation for the current client fork at submodule
`vendor/fr-client-rust` (`451759f2`). It does not revise the historical
allocation measurements or propose an ownership change.

## Current persistent ownership

`AnimFrameStore` is process-wide: `STORE` is a `OnceLock<Mutex<...>>`, and its
`list` and `opaque` tables are initialized once and shared by all clients
(`vendor/fr-client-rust/crates/client/src/dash3d/anim_frame.rs:21-37`).
`AnimFrame::init` only grows those vectors and preserves already-unpacked
entries (`anim_frame.rs:40-54`). Therefore a later client's `maininit` does
not retain a client-owned copy of the decoded animation table.

Snapshot injection is likewise process-wide. `load_snapshot_once` guards its
`Loaded` marker with `SNAPSHOT_INJECT: Mutex<Option<Loaded>>`; only the first
successful caller publishes the marker, and later callers return it without
reading the snapshot (`vendor/fr-client-rust/crates/client/src/unpack/mod.rs:297-316`).
`load_snapshot` feeds every animation record to `AnimFrame::unpack`
(`unpack/mod.rs:271-294`). `Client::maininit` calls `AnimFrame::init`, then
`load_snapshot_once`, for each client (`vendor/fr-client-rust/crates/client/src/client/client.rs:1655-1687`).
Thus the normal snapshot path is one process-fixed decoded animation store,
not ~34.5 MiB of retained animation tables per client.

There are two bounded lifecycle qualifications. `load_snapshot_once` performs
the potentially expensive `load_snapshot` before taking the publishing lock
(`unpack/mod.rs:300-313`), so simultaneous first callers can transiently
unpack duplicate data before one result is discarded. Also, if snapshot load
fails, `maininit` falls back to the live cache (`client.rs:1689-1697`) and the
normal animation prefetch/unpack path remains enabled (`client.rs:1700-1723`).
Those are exceptional/transient or fallback paths, not evidence that the
successful snapshot store is per-client.

## What unpack retains

`AnimFrame` owns its delay, transform-index/vector fields, and an optional
`AnimBase` (`anim_frame.rs:10-19`). `AnimBase` owns a type vector plus an outer
labels vector and one label byte vector per group (`vendor/fr-client-rust/crates/client/src/dash3d/anim_base.rs:14-19,21-42`).
During each archive entry, `AnimFrame::unpack` decodes one base, then clones it
into every frame (`anim_frame.rs:85-87,103-115`); transform arrays are copied
into per-frame vectors before publication (`anim_frame.rs:186-192`). The
published store therefore has persistent per-frame decoded transform vectors
and repeated owned `AnimBase` contents. This is duplication inside the single
process-wide table, not duplication once for every client.

`unpack` publishes by assigning `s.list[id] = Some(frame)` while holding the
store lock (`anim_frame.rs:95-112,176-192`). Re-unpacking an existing ID
replaces the entry; it does not append a second retained version. Old frame
allocations can nevertheless be transiently live until their owning clones
are released.

## Read-time cloning and callers

`AnimFrame::get` locks the process-wide store and returns `f.clone()`
(`anim_frame.rs:196-201`). Because `AnimFrame` derives `Clone`, this clones the
frame vectors and the `AnimBase`; it is a transient owned read result unless a
caller retains it. The current non-test callers are the model animation paths:
`Model::animate` gets one owned frame (`vendor/fr-client-rust/crates/client/src/dash3d/model.rs:1209-1218`),
and `Model::mask_animate` gets primary and secondary owned frames
(`model.rs:1244-1263`). These are per-animation-operation clones, not
per-client persistent table ownership. The public API intentionally returns an
independent owned frame, so existing callers can retain or mutate their copy
without exposing the store lock; the current regression contract preserves
that behavior.

`SeqType` no longer needs a full clone for scalar delay lookup: the crate-
internal `AnimFrame::delay` reads only `frame.delay` under the same store lock
(`anim_frame.rs:203-211`). The public `get` clone remains for rendering and
compatibility. The scalar-delay report records that this path removed seven
allocations per fallback lookup in its fixture, but does not establish a
retained-memory saving (`docs/memory/scalar-delay-experiment.md:1-5,31-35`).

## Reconciliation of the historical ~34.5 MiB figure

`docs/memory/low-end-owner-attribution.md:174-190` reports the filtered
`spawn_slot_thread -> load_snapshot / AnimFrame` stack as approximately
34.5 MiB in both panel and TUI and labels it “Per client construct.” Read
literally as an allocation-stack attribution, it identifies bytes charged to
that construction/unpack ancestry in the captured process. It does not prove
that 34.5 MiB remains resident per client, nor that each client independently
retains a decoded animation table under the current `OnceLock` snapshot path.

The current source supports this narrower interpretation:

- successful snapshot decoding is process-fixed (`load_snapshot_once` above);
- the decoded store itself can contain substantial internal duplication,
  chiefly one cloned `AnimBase` plus transform vectors per published frame;
- each `AnimFrame::get` can add transient owned frame/base/vector clones at
  read time; retention depends on the caller and operation lifetime;
- concurrent first-load races, archive replacement, and the documented live
  fallback can add transient or exceptional allocation ancestry.

Consequently, the historical number can be used as an attribution lead for
animation decoding and clone traffic, but not multiplied by bot/client count
or presented as an immediately recoverable resident amount. The report's
separate fixed/incremental table should remain unchanged.

## Bounded structural estimate (not a resident-savings claim)

Without archive contents or a resident measurement, only a symbolic structural
accounting is defensible. For one decoded frame with `F = frame.size` populated
transforms, the four transform payloads contribute `4 * F * size_of(i32)`;
`anim_frame.rs:186-190` copies exactly the populated `[..current]` ranges into
those vectors. For a base with `S` groups and total label bytes `L`, the payload
terms are `S * size_of(u8)` for `type` and `L * size_of(u8)` for label bytes.

Separate from those payloads, the Rust layout includes the relevant `Vec`/`Option`
headers: four `Option<Vec<i32>>` fields on `AnimFrame`, one `Option<Vec<u8>>`
for `AnimBase::type`, one `Option<Vec<Option<Vec<u8>>>>` for `labels`, and one
inner `Vec<u8>` header per label slot (with at most `S` nonempty label
allocations). These headers are not additional payload bytes, and `S` inner
headers are already contained in the outer labels allocation; do not count them
twice. Clone behavior and allocator capacities are implementation details, so
this accounting does not assert that source capacities are retained.

This is a byte-layout accounting aid only. Archive distributions, allocator
rounding, overlap with the historical stack filter, clone lifetime, and
dirty/resident status are unknown. It must not be converted into resident
savings or a new budget result without a matched native measurement.

## Compatibility boundary

Preserve grow-only initialization, replacement publication, and independent
owned results from public `AnimFrame::get`; tests explicitly exercise later
initialization/publication and owned-frame independence (`vendor/fr-client-rust/crates/client/tests/inject.rs:194-202`;
`tests/seq_delay.rs:51-55`). The existing scalar delay helper is already the
bounded optimization for callers needing only `delay`; this audit does not
reopen public clone semantics, redesign the store, or claim that the global
snapshot/table optimization is absent.
