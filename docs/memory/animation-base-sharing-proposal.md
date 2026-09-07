# One bounded proposal: private animation-base sharing

Propose sharing a decoded animation base among frames from the same unpack call
inside the private process-wide store. Keep the existing public owned types and
`AnimFrame::get` result independence. This is a fixed-working-set candidate under
approved performance-finish-plan section 4A, pending design review.

## Measured owner and purpose

The existing N1/N16 native attribution in
`diagnostics/incremental-owner-attribution-20260907T095008Z/owner-differential.json`
records 27,809,440 bytes in animation-frame stacks at each N (26.521 MiB,
rounded). Within it, the normalized nested `Vec<Option<Vec<u8>>>::clone` path
under `AnimFrame::unpack` owns 21,485,568 bytes across 454,618 live allocation
records at both scales. These are perturbed malloc-history allocation totals,
not RSS or per-bot costs. The [attribution report](incremental-owner-attribution-report.md)
records capture hashes, timing, provenance and parsing limits.

Source `vendor/fr-client-rust/crates/client/src/dash3d/anim_frame.rs` decodes one
`AnimBase` before the frame loop, then stores `Some(base.clone())` in every
frame. `AnimBase` owns the transform type vector and nested label vectors.
Those labels tell model animation which vertices/faces each transform affects;
they are required by simulation/rendering and must not be omitted. Frames from
one archive use identical base contents by construction. The selected clone
stack is therefore a concrete repeated owner; it is not an estimate of the net
replacement saving. The exact surviving base count and allocator overhead must
be measured after implementation.

`Model::animate` and `mask_animate` use `AnimFrame::get`. That API returns an owned
deep clone, allowing callers to mutate it without altering the store or other
calls. `SeqType` reads scalar delay through the existing allocation-free helper.
The public `AnimFrame`, `AnimBase` and `AnimFrameStore` structs have public fields;
changing those field types would break the existing surface even though no
other in-tree use of `AnimFrameStore` was found.

## Proposed private representation and boundaries

Keep all public structs and method signatures unchanged, including
`AnimFrame.base: Option<AnimBase>` and `AnimFrameStore.list: Vec<Option<AnimFrame>>`.
Introduce private store/record types for the actual static storage. Each record
holds its frame-specific delay/transforms and an `Arc<AnimBase>` belonging to
that single unpack invocation. The decoder creates one Arc from the newly
decoded base and clones only that Arc into each published record.

An implementation may use a private record containing an `AnimFrame` whose
base is None plus the Arc, avoiding duplicated transform field declarations.
That incomplete inner value must never escape: `get` materializes the unchanged
fully owned public frame, deep-cloning the shared base and transform vectors
while holding the same store lock. Alternatively explicit private fields are
acceptable if they keep the public contract exact and the decoding diff small.
Do not add a public borrowed/shared getter or change model callers in this task.

Keep the current OnceLock/Mutex boundary, grow-only init, frame-ID growth,
publication order, duplicate-ID replacement, opacity handling, scalar delay
lookup and malformed-input behavior. No new cache keys, global interning,
cross-archive deduplication, decoding shortcuts, validation policy or scheduler
changes. A later unpack creates a separate base even if contents match; replacing
one frame must not alter sibling frames from the prior archive. Old shared bases
drop when their last private record is replaced. Public results remain detached
owned values and do not retain the private Arc.

## Behavior and ownership verification before retention

First add deterministic public behavior fixtures and run them against the
unchanged representation. Cover multi-frame decoding with shared base contents,
inserted origin groups, default scale values, signed transforms and transparency
flags; assert all public fields and relevant model vertex/face results. Mutate
returned base types/nested labels and transforms, then verify fresh reads and
sibling frames are unaffected. Preserve an old returned frame across partial
republication with a different base and delay; verify it and untouched store
frames retain old contents while the replaced ID changes. Check grow-only init,
high IDs, missing/negative ID lookups and public struct construction.

After the representation change, add private ownership proof: frames from one
archive share the same base allocation, separately unpacked archives do not,
partial replacement retains the old allocation and replacing its final frame
releases it. This is an allocation/lifetime assertion, not RSS acceptance.
Keep fixture IDs isolated or serialize store-mutating tests; do not introduce
test races through process-wide state. Retain existing malformed-input behavior
and avoid recovery from a poisoned store as a production change.

Run client unit tests and affected client integration suites separately:
`seq_delay`, `inject`, `iface_model`, relevant model-animation fixtures and `zone`.
Run affected host-play and TUI libraries with memory-profile-no-alloc; serialize
the known shared-vault host-play fixtures. The orchestrator freezes matched
appearance-candidate baseline and new candidate binaries, then conducts the
required functional live qualification and short resource screens. Keep native
allocation confirmation separate from clean CPU/latency measurements. Use one
longer stage only if worth retaining, with the existing investigate-or-park rule.

## Limits

Only the private animation store is in scope; no public API, model transform
algorithm, renderer policy, packet format or host action surface changes.
This targets a fixed owner and cannot close the N16 per-bot budget gap. All
behavior, allocation-removal, clean resource and regression outcomes are pending.
Existing appearance claims remain parked; existing campaign target and final
Grok4.6 requirements remain unchanged. No accepted RSS saving is claimed.
