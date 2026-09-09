# Saved-prefix symbol attribution and next bounded discriminator

Status: supplemental source-attribution/design submission for `t_29a858c3`; not an owner acceptance, memory saving, capture completion, or authorization to run the proposed experiment. Original parser/classifier output remains authoritative and unchanged. Same-card independent review is required.

## Evidence identity and reproduction

Frozen host `c0709aba2f8b45e42193225cf8f4e7325b5ca9bf`, client `3456edc8dabf7b25ada78110ffa56327af9f67a4`, ELF SHA256 `a0c6eb0bed428fefad58530b177caae16ba2df6c7153544bb0852e16485591f9`, saved-prefix manifest `7fcd08aed20e44c81dd495f83ae663b167f4392b84c39a79ce927b45165e1595`. Configuration is System allocator, memory-profile-no-alloc, snapshot-dedup OFF, N1 TUI RasterOff. The original capture failed the raw-size guard. The successful a55972a replay is a `validated_prefix_diagnostic`, with `capture_complete=false` and `acceptance=false`.

Inputs are ONLY the exported TSV/receipt files in `diagnostics/root-native-production-a55972a/production-replay-1/result/` and supplemental JSONs in `diagnostics/root-native-symbols-a55972a/`. This task did not open the huge raw/interpreted streams, repeat native replay, access caches/accounts/server, build Cargo targets, or change runtime/parser/classifier source. The separate original-result report belongs to `t_04537ad7`; it and STATE were not edited.

Run from the campaign checkout:

    python3 diagnostics/saved-prefix-symbol-attribution/verify.py

This runs seven selector regression tests, the independent exported-table audit, frozen-source identity reads, and a second analysis pass proving byte-identical output. Real output and hashes are in `diagnostics/saved-prefix-symbol-attribution/verification.json`. The two passes here read SMALL EXPORTED TABLES, not production allocation streams. All scripts and outputs are confined to that directory.

Artifacts:

- `audit.py`: descriptor-to-stack-to-original-family and supplemental-group audit; no import of the canonical classifier.
- `test_audit.py`: plain/angle-wrapped application functions, allocator/dependency generic traps, first-frame ordering, unresolved/no-match and allowlist boundaries.
- `mapping.json`: all 1,591 referenced nonzero function-string IDs, exact raw names and independently demangled names; all 3,431 exported IP definitions and 6,926 trace-parent definitions. These are the exported cutoff-reachable tables, NOT all definitions from the full replay receipt.
- `populations.json`: every original stack row at every cutoff, original family ID, byte/count contribution, selected supplemental group and coverage flags. No top-N truncation.
- `audit.json`: every group, member trace IDs, totals, source coverage, immutable-input SHA256s and explicit trace-3862 absence at later cutoffs.
- `leading-chains.json`: all member IDs/original family IDs for the eight leading groups per cutoff, with the three largest member chains where available. The complete remaining chains are reconstructible from `mapping.json`, not discarded.
- `source-identities.json`: ten frozen source blobs fetched with `git show` from THIS host/client checkout, SHA256s and comparison with current file bytes. All ten match; the tracked relevant host diff against the frozen commit is empty.
- `inspect_local.py`: line-numbered `git show` helper for reviewing the anchors below without substituting current source.

The audit verifies every result TSV against the receipt's own length/hash manifest, then hashes every input before/after analysis. It validates string byte lengths, nonduplicated IDs, IP/parent references and acyclic chains, every descriptor's size × multiplicity, exact per-trace aggregation, exact original-family aggregation and receipt totals. It independently invokes the frozen demangler and compares every raw string/name, every supplemental application-frame entry, every member trace row, and every group/summary total against root's probe. Assertions fail the run on discrepancy. Canonical full-positive-stack multiset equality is the original runner's attestation; this task does NOT independently rerun that oracle or raw-pointer replay. Hash/aggregation equality is not a substitute for that distinction.

The demangler is `/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/llvm-cxxfilt`, Apple LLVM 17.0.0, SHA256 `de567ed2f5c83f19eda3aac489b631936e60b0c42fcb30a2933dfb7bdeaaf3fb`, invoked with `--no-strip-underscore`. Default Darwin underscore stripping is not appropriate to these retained Linux names. All root mappings reproduce exactly with the corrected option. Two symbols remain mangled (IDs 1756/1757, V8 C++ templates); they must not be mislabeled as unresolved Rust v0. Their containing positive stack contributes 16 bytes at last mark/EOF.

Initial execution limitations are retained in `verification.json`: headless approval mode blocked execute_code and inline python inspection before execution. A file-based first audit failed because CSV quote interpretation removed JSON quotes; the successful version explicitly uses QUOTE_NONE for these TSVs. No result is claimed from the failed invocation.

## Accounting and selection limits

All bytes below are contemporaneous tracked requested bytes, never sums of independent stack peaks. Counts are live allocation multiplicities, NOT bots, objects of one type, or publication epochs. Descriptor IDs encode reusable size/trace classes and are not stable object identities.

| Cutoff | Bytes | Live count | Descriptor rows | Stack rows | Original families | Supplemental groups |
|---|---:|---:|---:|---:|---:|---:|
| First strict global peak, preceding mark 940 ms | 160,842,811 | 89,122 | 826 | 194 | 47 | 38 |
| Last actual mark, 589,611 ms | 132,027,090 | 209,263 | 8,863 | 2,055 | 414 | 150 |
| Captured prefix EOF | 132,028,404 | 209,283 | 8,878 | 2,069 | 419 | 150 |

Peak is an allocation event position, not a sample observed exactly at 940 ms. EOF is an unbounded-time event tail after the last mark, not a synthetic 589,612 ms sample. It adds 1,314 bytes and 20 live allocations relative to the last mark. Neither late population is observe-end, script Stop, shutdown, or post-join.

The supplemental selector walks from allocation callee toward callers, including stored inline function groups. It accepts only a leading `nav::`, `client::`, `api::`, `host_play::`, `script::`, or `tui::`, either directly or immediately after the first `<` for an application self type. For example, `<script::load::JsCard as core::clone::Clone>::clone` qualifies. `<alloc::vec::Vec<client::...>>::resize` and `<std::sync::OnceLock<...client::Sound...>>::initialize` do not. A slice/reference/tuple containing an application type is conservatively not accepted. This is the agreed root exploratory allowlist, not an exhaustive application taxonomy: `host::` is deliberately not newly added. A missing match remains a positive unknown bucket, not dropped cost.

Crate names inside generics still provide context for source investigation but never supply the selected application frame. This matters concretely at trace 3834 (allocator iterator contains `host_play::load_template`), 5483 (Once machinery contains `empty_table`), and 7805 (RawVec contains WidgetView). Root's corrected selection is reproducible; a substring classifier would be materially wrong.

Coverage flags scan the WHOLE stack even after finding a useful function:

| Coverage limitation, overlapping populations | Peak bytes / stacks | Last-mark bytes / stacks | EOF bytes / stacks |
|---|---:|---:|---:|
| No recognized application frame | 74,276 / 3 | 163,229 / 70 | 163,229 / 70 |
| At least one unresolved symbol frame | 160,842,811 / 194 | 132,026,994 / 2,054 | 132,028,308 / 2,068 |
| At least one absent source path/line | 160,842,811 / 194 | 132,027,090 / 2,055 | 132,028,404 / 2,069 |
| Excluded dependency/allocator frame mentions an application crate | 2,470,408 / 24 | 46,471,632 / 1,751 | 46,472,946 / 1,765 |

There are zero exported function groups with both source file and nonzero source line. No dangling referenced trace/IP definitions were found, and no positive stack uses absent IP zero. Unresolved addresses and missing function names nevertheless remain common. Coverage rows overlap and must not be added. No readable-name success repairs missing inlining, field callsite, instance, epoch or free-event lifetime coverage. The original families all retain `ownership=unknown`; original classifier useful-prefix/source-anchor requirements are unchanged. Its all-unknown ownership result is not superseded by this supplemental grouping.

## Frozen source attribution: hypotheses with actual chains

All host anchors below refer to the frozen host commit above; client paths are relative to `vendor/fr-client-rust` at the frozen client commit. An anchor is a manually inspected source location, NOT debug information recovered from the captured IP. Chains below run caller → callee; `leading-chains.json` retains complete captured chains in the opposite direction. Names absent due to inlining are not invented as captured frames.

### Navigation: dominant fixed contribution and transient overlap

- `nav::pack::decode`: peak 73,817,616 bytes/902 allocations; last mark and EOF 73,793,888/1,471. Five original families merge into this supplemental group; six positive traces at peak and seven later. This is a multiway decode group, not one vector. Net peak-to-last decrease is 23,728 bytes despite increased count.
- Trace 3865: `Play::new → NavWorld::load_pack → nav::pack::decode → <u8 as SpecFromElem>::from_elem`, 65,142,784 bytes, one live allocation at all three cutoffs.
- Traces 3867 and 3869: same application callers → `RawVecInner::try_allocate_in`, respectively 8,142,848 and 366,576 bytes, each one allocation at all three cutoffs. The first two large decode rows sum to 73,285,632 bytes, but that numerical resemblance to a serialized layout is not field identity.
- Frozen `crates/nav/src/pack.rs:313-351` reads the header, checks dimensions, creates `walk: Vec<u8>` and `blocked: Vec<u64>`. Lines 352-415 also allocate graph edges, per-edge requirements, graph indices and banks before returning `(WorldCollision, TransportGraph, banks)`. Trace 3865 is a strong source lead for byte-element storage; trace 3867 is only a generic allocation boundary. Do NOT label either a verified specific field/capacity based solely on symbols or matching bytes, and do not label trace 3869 a graph field without a direct binding.
- Frozen `crates/nav/src/world.rs:49-56,61-71` owns the input byte Vec locally, passes a borrowed slice to decode and moves decoded parts into NavWorld. `crates/host-play/src/lib.rs:3159-3185` loads ONE NavWorld into an Arc per Play; `3207-3212,3570-3586` expose/clone the same Arc for consumers/slots. This is a process-shared source domain, not a measured per-bot private owner.
- Trace 3862 is DIFFERENT: `Play::new → NavWorld::load_pack → std::fs::read::inner`, peak 73,438,581 bytes, one live allocation and one full-trace allocation call. Its descriptor and stack rows are absent at last mark AND EOF. Thus these tracked transient read bytes have disappeared by the last actual mark. This is not shutdown reclamation proof. The local Vec drop at load_pack return explains a plausible source lifetime; the exact free timestamp and resident-page return were not recovered here.

Source-level sharing already exists and must not be reimplemented. `crates/nav/src/collision.rs:51-80,151-169,435-443` already stores one byte of directional faces plus one blocked bit per tile/level and keeps raw u32 flags in an optional debug sidecar. Current source bytes match the frozen version. The ledger and finish-plan §4 explicitly say to investigate WHY shared storage is large, while preserving prior sharing/representation work. Neither converting u32 flags to packed bits again nor removing N duplicate production packs is a valid new proposal here. Eliminating the input read buffer alone targets cold-start overlap, not the retained decode contribution at last mark.

### Interface tables: shared templates, mixed allocation sites

`IfType::unpack`: 6,491,160 bytes/27,871 live allocations at all cutoffs; two original families and 42 traces. Trace 3763 contributes 5,887,424 bytes/10,984 allocations with `Play::new → IfType::unpack` as the captured application chain. Traces 3769/3770 each contribute 102,912 bytes via `IfType::unpack → Vec<Option<Box<IfType or IfTypeMut>>>::resize → RawVec reserve/finish_grow`.

Frozen client `crates/client/src/config/if_type.rs:332-340,367-407,414-451,565-572` constructs two sparse tables, nested scripts/children/inventory/name storage and both boxed immutable/mutable records. Frozen host `crates/host-play/src/lib.rs:4131-4157` converts the mutable boxed template to Arcs, then `3159-3170,3576-3578` shares both templates. The supplemental `Play::new` group separately includes trace 3834, 1,933,184 bytes/10,984 allocations, under allocator iterator machinery with template-conversion generic context. Do not add this to IfType's group without declaring a NEW source-domain aggregation; the script preserves the disjoint original rows.

Source implies shared template lifetimes plus potential per-client COW overlays after construction. Unpack names alone cannot identify which nested vector or record survives, a mutation's private owner, or current COW occupancy. The identical populations do not prove object equality across snapshots or all future template retention.

### JagFX: empty-table reservation is a concrete source lead

`JagFX::load_shared`: absent from the leading peak groups, 13,932,080 bytes/four allocations at last mark and EOF, one original family/four traces. Trace 5483 accounts for 13,928,000 bytes. Captured chain: `Client::construct → JagFX::load_shared → OnceLock::initialize → futex Once::call → Once::call_once_force`. The full generic names contain `empty_table` and `Arc<Vec<Option<Sound>>>`, but those dependency frames are NOT selected as application frames.

Frozen client `crates/client/src/sound/jagfx.rs:13,21-48` has an inline `Sound` with ten optional Tones and a process `EMPTY` OnceLock containing 1,000 `Option<Sound>` entries and 1,000 delay integers. Lines 56-67 defer scratch; 98-139 route lowmem/missing-file to default and separately cache unpacked tables by cache directory. This chain plus source points specifically toward default empty-table initialization, not proof of 1,000 populated sounds or 13.9 MB per bot. The other requests are 4,000 and two 40-byte allocations in the same family. A future capacity/layout probe could bind those requests to exact fields/Arc overhead. No such native layout/occupancy experiment ran here. Public `JagFX.synth`/Sound shapes and highmem COW behavior make a representation change an API/behavior design, not permission to silently box fields or turn off audio.

### Model and animation data: process stores, not rendered geometry totals

`Model::unpack`: 5,046,594 bytes/4,556 allocations, trace 6042, one family at last mark and EOF. Actual chain is `Client::maininit_with_progress → load_snapshot_once → load_snapshot → Model::unpack`. Frozen client `crates/client/src/dash3d/model.rs:28-68,211-247,323-354` stores process-wide ModelMeta, makes a transient trailer copy, retains a source-byte copy and supports unload. `357-375` separately resolves shared decoded geometry. A retained ModelMeta source-byte domain is plausible; captured no-line `unpack` is not a proof that this entire group is precisely `ModelMeta.src`, or a GPU/model geometry total. Multiple source allocations can collapse into one symbol.

`AnimFrame::unpack`: 3,538,928 bytes/39,724 allocations, one family/five traces at last mark and EOF. Same load_snapshot caller chain. Frozen client `crates/client/src/dash3d/anim_frame.rs:29-59,94-128,136-161,227-245` contains the process PrivateStore, several temporary packet copies, an already-shared AnimBase per unpack and four per-frame transform Vecs. Traces 6062/6063/6064 each contribute 878,684 bytes/9,847 allocations; equal bytes do not establish which transform field each IP represents. The source supports process-table retention possibilities, not a new claim that AnimBase sharing is missing. Public `get` still materializes owned clones, whereas `delay` at 248-255 already avoids that clone for scalar reads.

### Snapshot widget walk: multiple callers and epochs remain unresolved

`api::snapshot::walk_widget_tree`: 2,947,951 bytes/12,840 allocations at last mark/EOF, five original families/51 traces. Traces 7805, 7852 and 7921 each contribute 671,488 bytes/12 allocations via `walk_widget_tree → RawVec<WidgetView>::grow_one → RawVecInner::finish_grow`; their caller suffixes differ:

- 7805: TuiSession start_play closure → GameSnapshot::rebuild → rebuild_family.
- 7852: Host::client_tick → GameSnapshot::rebuild → rebuild_family.
- 7921: Host::client_frame → host::drain_and_rebuild_snapshot → rebuild_family.

Frozen host `crates/api/src/snapshot.rs:1805-1828,3051-3090,3097-3148` clears/reuses a feature-off widget Vec, walks roots using a temporary queue, appends WidgetViews and materializes nested strings/scripts/actions/items. The same walker serves side-tabs as well (`1847` onward). These are source-private snapshot-family possibilities with transient traversal scratch, not shared interface-template storage. Distinct captured caller paths are real; three equal byte totals and twelve allocations per row are NOT three identified shells, twelve epochs, or content-equal duplicates. No publication identity, slot binding, retained-consumer set or lifetime interval was exported. The separate `GameSnapshot::rebuild_family` group (2,199,183 bytes/11,091 at last mark) has nine original families/108 traces, including LocView growth; it is also multiway, not a widgets-only counter.

The other leading config symbols remain separate complete groups in the artifacts. Their caller chains support config-loading context, but this report does not promote them to exact field ownership. Source-inferred domain, captured allocation call chain, concrete callsite, per-instance/epoch identity, actual requested capacity, and RSS are different levels of evidence throughout.

## Selected next smallest diagnostic — nav uniform-tile census, not a new capture

Decision: test whether the already-shared packed collision representation has enough uniform spatial tiles to justify a later representation proposal. The retained decode group is the largest current source lead; making its resident representation smaller can help the fixed one-client budget. Merely streaming the startup file would not resolve that retained contribution. Do NOT change representation, public Vec fields, load behavior, pack format or routing now.

This is one future OFFLINE probe, requiring a separately authorized implementation/release. No frontend, account, game server, profiler stream, native full replay or new allocation capture is needed. The operator/root must supply a read-only, hash-bound local copy of the exact existing navpack; this task did not access or copy it. Its identity is the capture-plan navpack SHA256 `2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30`. Missing input is a failed prerequisite, not permission to use another pack or fetch remotely.

### Fixed probe contract

1. A separately built standalone diagnostic links the exact frozen nav decoder. Read the one artifact once, hash the bytes supplied to `decode`, retain the ordinary decode result, and export actual field len/capacity/element-size for `collision.walk` and `collision.blocked`, dimensions/origin, flags presence and graph/bank counts. Bind these newly observed field capacities to this OFFLINE instance only; do not retroactively call captured trace IDs proven field identities. No custom allocator tracing, accounts, cache decode or event log.
2. Walk each of four levels in fixed 32×32 spatial tiles. A cell's complete value is `(face byte, blocked bit)`, not just walkable/unwalkable. Count uniform tiles by exact pair and nonuniform tiles per level. Boundary tiles compare valid cells only; missing cells use deterministic padding in size estimates and are never exposed as legal map cells. No cross-level substitution, loss of directional bits or fallback to level 0.
3. Emit one bounded JSON with input/source/toolchain identity, lengths/capacities, per-level tile counts, uniform-pair histogram, checked arithmetic and result classification. No per-cell JSON, copied tile dictionary, routes, coordinates dump or huge trace. Additional scan memory is bounded by a 512-entry pair histogram and scalar counters; input/decode storage is accounted separately.
4. Preconditions for this later probe: input ≤128 MiB, available memory ≥768 MiB and free disk ≥1 GiB on the selected executor. One owned process, CPU ≤30 s, wall ≤60 s, RSS ≤384 MiB, child AS ≤512 MiB, output ≤1 MiB; enforce only child limits and use the existing owned-process supervision pattern with failure/reap receipts. Exact executable must have a separately reviewed build/guard proof before release. Overflow, malformed decoder failure, cap breach or mismatch produces only a failed receipt, no partial favorable estimate. No auto-retry or cap increase. These are proposed bounds, not limits proven by this task.

### Falsifiable benefit model

Let `T = 4 * ceil(width/32) * ceil(height/32)`, `D` be nonuniform tile count and `A = walk.capacity + 8 * blocked.capacity` for the newly decoded instance. A minimal hypothetical uniform/dense representation uses an eight-byte descriptor per tile plus a dense payload of 1,024 face bytes and 128 blocked-bit bytes per nonuniform tile. The screening estimate is:

    candidate_element_bytes = 8*T + 1152*D
    potential_element_reduction = A - candidate_element_bytes

This is an explicitly hypothetical storage design, not a measured allocation or RSS result. It excludes headers, allocator rounding, conversion peak, graph/banks, the input read buffer and page retention; list those separately in output rather than hiding them. Public Vec exposure means actual replacement requires a consumer/API audit and its own design. No zero-copy or backward-compatible migration is promised by this formula.

Predeclare a materiality threshold of at least 16 MiB estimated element reduction after directory and boundary padding. This is a proposed screening threshold, not a new campaign target. If the exact census falls below it, park this representation direction immediately: existing uniformity is insufficient for the stated benefit. Do not search tile sizes until one passes. If it exceeds the threshold, the output selects a bounded representation-design card; it does not authorize implementation or acceptance. A diagnostic can therefore disprove the proposed opportunity without any long live capture.

A subsequent implementation, if separately approved, must preserve every cell's full directional/blocked value, outside-grid and invalid-level behavior, optional raw-flag sidecar, all four planes, graph order/indices, transport requirements, banks, immutable sharing, v8 decoding and BadMagic-only legacy fallback, stale-version/truncation/error behavior, plus existing public consumers. Exhaustive logical cell reconstruction and boundary/diagonal routing fixtures must agree with the dense baseline. Conversion peak and random lookup/router CPU may invalidate the apparent saving. Future matched clean tests must obey the finish-plan 5% CPU/2 ms p99 non-regression and target-hardware gates; a standalone storage census cannot pass those gates.

### Remaining bottlenecks and acceptance boundary

The nav census addresses only the largest retained fixed source lead. It cannot explain the N1/N16 per-bot RSS slope, prove snapshot epoch overlap, resolve the JagFX public-layout tradeoff or measure CPU attribution. Those remain separate, not silently declared solved. The sound empty-table lead is worth retaining for the next prioritization decision, but is not a second released experiment here.

Missing late observe-end, script Stop and post-join populations remain missing. Untracked native/V8/mmap/GPU storage and allocator page retention remain outside this requested-byte audit. Even a successful future nav census or storage removal cannot close capture qualification, resource provenance, helper/profiler overhead, lifecycle plateaus, responsiveness/scaling or final budget gates. Approval sought here is diagnostic/design approval only.
