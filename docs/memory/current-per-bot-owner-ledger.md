# Current per-bot owner ledger — O1 Phase A

Status: offline source/archive ledger only. This document does not claim an RSS or CPU saving, does not authorize an optimization or live rerun, and does not close any campaign gate.

## Scope and frozen provenance

The ledger is anchored to the approved diagnostic pair and its source preflight:

- Host source: `c0709aba2f8b45e42193225cf8f4e7325b5ca9bf`.
- Client submodule: `3456edc8dabf7b25ada78110ffa56327af9f67a4`.
- Measured binary: `a0c6eb0bed428fefad58530b177caae16ba2df6c7153544bb0852e16485591f9`.
- Allocator/features: System allocator, `memory-profile-no-alloc`; snapshot-dedup is OFF; allocation counting, scheduling/responsiveness/render profiles are OFF; TUI `RasterMode::Off`.
- Source-delta preflight: `diagnostics/current-per-bot-owner-preflight/current-source-delta.json`, 351 Rust/Cargo files checked. The only current-vs-frozen file is `crates/tui/src/bin.rs`, and the receipt identifies it as reviewed `cfg(test)` fixture code. The frozen runtime source relationship must be rechecked before any layout/build action.
- Evidence: `docs/memory/current-tui-n1-n16-attribution-report.md`, `docs/memory/current-tui-n1-n16-attribution-evidence.json`, and approved independent review `docs/memory/current-tui-n1-n16-attribution-review.md`.
- Archives: N1 archive has 101 verified payload files; N16 has 93. N16 was not rerun. The two publications are distinct epochs even when content is equal; no feature-off dedup is inferred.

The root-owned native Linux compile-only layout artifact is incorporated from `diagnostics/current-per-bot-owner-layout/owner-layout-1827/`. Its receipt says `no_program_executed=true`, `executed_client=false`, all 1113 frozen source files were verified on the builder, and the target was `x86_64-unknown-linux-gnu` with rustc 1.98.0/LLVM 22.1.8. Exact Cargo-selected rlibs were supplied to rustc. These are native type-header sizes, not heap occupancy or RSS measurements.

## Lifecycle and accounting boundary

The profile-off harness reaches observe end, calls `Play::script_stop` for each name, and samples during the existing teardown interval while clients and their snapshots remain alive. `crates/host-play/src/memory.rs:1014-1078` preserves the observe-end row before the stop path; `crates/host-play/src/lib.rs:3373-3379` delegates script stop; `crates/script/src/slot.rs:174-185` joins a Load isolate, runs compiled teardown, drops the instance, and reaches Idle. There is no sampled post-slot-join RSS value in this pair. The N1/N16 retained RSS values (171008000 and 433799168 bytes) are after script Stop observations, not post-join measurements and not proof of allocator page release.

Domains remain separate:

- V8 used/total are logical heap gauges.
- Rust allocator payload/capacity, if enabled, is logical requested storage and excludes V8/native/GPU allocators; the measured binary has counting disabled.
- RSS/smaps are resident mappings/pages.
- `client_tick_total_ns`, script spans, and UI timers are elapsed work spans that may overlap and are not per-owner process CPU.
- Server/helper series are outside client ownership and must not be subtracted as overhead.

## Owner ledger

| Owner / fields | Source and consumers | Sharing boundary | Lifetime in this pair | Fixed reservation formula or known storage | Dynamic facts missing / discriminator |
|---|---|---|---|---|---|
| Shared cache tables (`Client.cache`, `Play.cache`) | `vendor/fr-client-rust/crates/client/src/client/client.rs:300-304`; host construction in `crates/host-play/src/lib.rs:3051-3060` | One immutable `Arc<Cache>` per Play, referenced by each Client; not N independent cache copies | Play/client process lifetime; remains after script Stop while clients live | Guaranteed fixed share is one cache object per Play, but its allocation graph is dynamic from decoded jag/config tables | Actual decoded table capacities, allocator bins and resident pages are not in the archive. Capture shared-vs-private allocation families.
| Shared interface template (`Client.ifaces`, `Play.ifaces`) | client source `client.rs:305-315`; host stores `Arc<Vec<Option<Box<IfType>>>>` | Immutable `ifaces` shared; `ifaces_mut` is an Arc template with per-slot COW overlays | Process lifetime for template; overlay entries live per Client and may COW on writes | `Vec` reservation is `size_of::<Option<Box<IfType>>> * capacity`; template capacity and nested `IfType` allocations are dynamic | Per-slot COW count/capacity and nested widget/model references are unmeasured. Do not infer from serialized snapshot lengths.
| Client mutable tables and world | `client.rs:339-379`; `core/world.rs:39-104` | One full Client and one World per slot; no lean baton (`host-play/src/lib.rs:3046-3047`) | Client slot thread lifetime, including after script Stop until slot join | Native headers: `Client=28048 B`, `World=248 B`, `Square=184 B`, `Sprite=84 B`, `ClientPlayer=496 B`, `ClientNpc=360 B`, `CollisionMap=40 B`. `World.squares`: `max_level * max_tile_x * max_tile_z * size_of::<Option<Box<Square>>>` reservation for outer grids; `dynamic_sprites`: `5000 * size_of::<Option<usize>>`; `occluders`: `4 * 500 * size_of::<Option<Occlude>>`; `occlusion_cycle`: `max_level * (max_tile_x+1) * (max_tile_z+1) * 4` bytes. These are reservation formulas, not occupied payload. `Client` also reserves fixed arrays such as four collision maps and heightmaps, with nested Vec capacities dynamic. | Actual dimensions/capacities, occupied Square/Sprite/Occlude counts, entity boxes, ground objects, loc changes, packet queues, audio queues, and allocator retention are missing. This is the strongest logical per-bot candidate but cannot be ranked against RSS without live allocation-family capture.
| Client entity and packet state | `client.rs:341-358`, `client.rs:348-358` | Per Client; sparse `Vec<Option<Box<ClientPlayer>>>`, `Vec<Option<Box<ClientNpc>>>`, appearance packets boxed per occupied slot | Client lifetime; packet/appearance vectors can retain capacity after updates | Pointer-slot reservation is `2048 * size_of::<Option<Box<ClientPlayer>>>` and `16384 * size_of::<Option<Box<ClientNpc>>>` only if those are the configured vector lengths; source comments establish capacities, but this ledger does not substitute runtime lengths. | Occupied entity counts, box payload sizes, appearance packet capacities and packet queue retention are not in current raw counters.
| `GameSnapshot` navigation/host shell | `crates/api/src/snapshot.rs:532-687`; rebuilt by `host-play/src/lib.rs:2925-2935` | One `GameSnapshot` per slot for the host/script path; owns family Vecs and nested views. Feature-off fields (`loc`, `widgets`, `side_tabs`) are private Vecs, not Arcs | Per slot; rebuilt on generation/tick edges and remains through after-script-Stop until slot join | Native headers: `GameSnapshot=1384 B`, `WidgetView=344 B`, `SideTabView=40 B`, `LocView=144 B`, `ItemView=88 B`, `NpcView=184 B`, `PlayerView=160 B`, `SceneView=48 B`, `ChatLineView=56 B`, `StatView=48 B`, `GroundItemView=96 B`. Each Vec reserves `capacity * size_of::<T>` plus nested allocations. Family rebuild gates avoid deep-copying unchanged families but changed families allocate/retain their own capacity. | Runtime capacities and nested string/entity/view allocations per family are unknown. Need a capture that reports allocation families at observe and after-script-Stop, or exact equality/retention evidence across the two epochs.
| Snapshot family payloads | `snapshot.rs:539-686`: NPC/player/stats/inv/scene/world/camera/loc/ground-item/inventory/equipment/bank/trade/shop/widgets/side-tabs/chat/make/quest/modals/menu/varps | Per-slot shell; source-present `snapshot-dedup` Arcs are compiled out in measured configuration | Snapshot lifetime and generation-gated rebuild lifetime; not necessarily dropped at script Stop | Structural formula for a family is `Vec<T>::capacity * size_of::<T>` plus each element's heap-backed fields; no serialized byte length is a capacity bound. | Current archive contains logical gauges and encoded payloads, not vector capacities or nested allocator ownership. Missing discriminator: actual live capacities/allocations versus retained pages.
| `nav_snapshot` alias / publication | `host-play/src/lib.rs:3869` creates a `GameSnapshot`; `observe_rebuild_snapshot` rebuilds on tick edge | Per-slot navigation snapshot shell; publication is a second consumer/epoch, not permission to count one fixture twice | Before current frame drain / tick-edge rebuild; remains with slot | Same `GameSnapshot` formula as above. A representative serialized blob is not storage capacity. | Need prove whether host and script publication shells coexist and their peak overlap in the actual runtime. Equal content alone does not prove equal allocation or retention.
| Script isolate and fingerprint | `crates/script/src/slot.rs:26-52`; `crates/script/src/isolate_fb.rs:1386-1448` | One compiled script XOR one Load isolate per slot; `SnapshotFingerprint` is an owned per-slot copy of many fields; `IsolateBuf` is one reusable FlatBuffer builder per slot | Script instance starts on Start; Stop joins/drops isolate or compiled instance, but the Client/snapshots survive | Native headers: `SlotScript=1272 B`, `SnapshotFingerprint=752 B`, `Packet=2112 B`; `Option<SnapshotFingerprint>` is also 752 B and `Option<Packet>` 2112 B on this ABI. Each Vec is `capacity * size_of<T>` plus cloned String/Vec payloads. Isolate/V8 bytes are outside Rust allocator accounting. `IsolateBuf` capacity is dynamic and reusable. | Actual V8 heap, isolate native allocations, fingerprint capacities, builder capacity and post-Stop allocator/page behavior are missing. Existing V8 zeros after Stop do not establish RSS ownership.
| Encoded snapshot buffers | `host-play/src/lib.rs:1437-1464`; `isolate_fb.rs` encoder | Per-slot IPC buffer, with deltas compared against retained fingerprint; no JSON path | Reused while script is running; Stop clears script-side owners but after-stop snapshot/client population remains | `Vec<u8>::capacity` is dynamic; payload length is not capacity. Exact fixed reservation is only `capacity * 1` after observing capacity. | Need distinguish peak transient encoding from retained builder capacity and fingerprint payload. Capture at keyframe, delta, and after Stop.
| NavBot, wires, status publication and host script map | `host-play/src/lib.rs:3071-3088`, `NavBot` consumers around nav pump | Per-slot maps keyed by username; shared host `NavWorld` is `Arc` | Nav state and queued commands live until consumed/removed; status rows live in shared process state | HashMap/VecDeque/Arc formulas depend on entry count and capacity; no fixed upper bound from current source sufficient for RSS | Current archives do not expose queue capacities, route lengths, status string allocations or `Arc` strong counts. Need bounded live occupancy capture.
| TUI/harness infrastructure | `crates/tui`, `host-play/src/memory.rs` publisher and collectors | Process-level, not per-bot; helpers/server separately bound | Process/harness lifetime | No valid per-bot formula; helper overhead explicitly unmeasured in evidence | Keep process/harness overhead out of per-bot owner ranking. Raw resource provenance remains missing.

## Exact source-backed formulas and limits

For a `Vec<T>`, guaranteed element-storage reservation is `capacity * size_of::<T>()`; nested heap fields must be added separately and are not bounded by `len` or serialized length. For a `Vec<Option<Box<T>>>`, the vector reservation is pointer-sized slots; each occupied `Box<T>` allocation is separate. The native probe reports `Option<Box<Square>>=8 B`, `Option<ClientPlayer>=496 B`, and `Option<ClientNpc>=360 B`; the boxed payload sizes are `Square=184 B`, `ClientPlayer=496 B`, and `ClientNpc=360 B`. For the current `World` source, the fixed-capacity terms are:

```
world_grid_slots = max_level * max_tile_x * max_tile_z
squares_slot_bytes = world_grid_slots * size_of::<Option<Box<Square>>>()
dynamic_sprite_slot_bytes = 5000 * size_of::<Option<usize>>()
occluder_slot_bytes = (4 * 500) * size_of::<Option<Occlude>>()
occlusion_cycle_bytes = max_level * (max_tile_x + 1) * (max_tile_z + 1) * size_of::<i32>()
```

With the native ABI, `Option<Box<Square>>` is 8 B, so `squares_slot_bytes = world_grid_slots * 8`; `World`'s own header is 248 B. The probe reports `Option<Sprite>=88 B`, but `dynamic_sprites` stores `Option<usize>` and its exact target size must be taken from the compiler rather than inferred from a different type. These formulas are exact for the source's reservations once runtime dimensions and target ABI are supplied. They do not include outer Vec headers, allocator rounding, nested `Square`/`Occlude`/`Sprite` payloads, `World` itself, or RSS page retention. A structural worst case is not an observed workload occupancy.

For per-bot fixed-versus-shared comparison, if `S` is a genuinely process-shared immutable allocation and `P_i` is a slot-private allocation, the source-backed logical accounting is `S + sum(P_i)`; it is not valid to derive `S` by fitting a line through N1/N16. The observed finite difference is only `(RSS16 - RSS1)/15 = 22.963802 MiB` and CPU `(CPU16 - CPU1)/15 = 0.03418354 cores`; neither is an owner size or causal attribution.

## Archive gauges, with domains preserved

| Gauge | N1 | N16 | Interpretation |
|---|---:|---:|---|
| Steady median RSS | 184887296 B / 176.322 MiB | 546076672 B / 520.779 MiB | Resident process observation; no owner attribution |
| Native lifetime peak RSS | 185495552 B / 176.902 MiB | 627003392 B / 597.957 MiB | Resident peak, not logical payload |
| Process CPU | 0.048992949 | 0.561746044 cores | Process CPU delta over observe bracket; not elapsed per-owner spans |
| Median V8 used / total | 6097876 / 9437184 | 94923480 / 118226944 | Logical V8 gauge; do not add to RSS |
| After-script-Stop resident | 171008000 B | 433799168 B | Clients/snapshots still alive; not post-join |
| After-script-Stop active / V8 isolates / V8 bytes / inflight | 0 / 0 / 0 / 0 | 0 / 0 / 0 / 0 | Cleanup state only; does not prove all live owners dropped |

Raw resource provenance and helper overhead remain open. The ledger intentionally does not rank owners by these gauges.

## Missing discriminator and next bounded capture

Phase A cannot order the candidates because dynamic occupancy/capacity and shared/private peak overlap are absent. The selected discriminator is **actual live allocation families and capacities**, with snapshot equality/retention as a secondary field: capture the client process's live Rust allocation families, V8/native totals separately, and the two `GameSnapshot` publication epochs at observe and after-script-Stop. Do not count one serialized fixture twice; establish coexistence and identity in the runtime.

A later root-owned card must predeclare one exact platform, N, frozen binary/features, account/cache/server identity, warmup/observe/teardown, capture point/window, helper/headroom bounds, cleanup and one-attempt failure rule. A result can select a candidate only if it shows a material per-bot private family or duplicated payload that survives the actual lifecycle boundary. If allocation families remain unbound, keep ownership and RSS causal attribution unresolved. No new live cell or optimization is authorized by this ledger.

## Verification boundary

This document is source/archive analysis only. It intentionally contains no native heap-occupancy claim, no post-join column, no RSS arithmetic across V8/payload domains, no elapsed-span-as-CPU claim, and no acceptance decision. The compact machine-readable companion is `docs/memory/current-per-bot-owner-ledger.json`.
