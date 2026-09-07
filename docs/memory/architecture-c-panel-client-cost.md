# Architecture C: panel and incremental client ownership

## Recommendation and decision

Recommend preserving the existing one-process/shared-device architecture and completing its ownership split, not replacing it with a renderer service or a shared mutable game world:

1. Make panel presentation storage variant-owned: a GPU view holds only the client's existing texture registration; CPU upload storage exists only while actually needed. This is the smallest source-proven removal, useful especially in focused-plus-background mode, but not a solution to the Windows fixed-cost gap.
2. For the larger client investigation, choose a compact, instance-owned scene store: move render-only tile fields into the renderer and replace individual immutable terrain-stamp boxes with a per-build packed store addressed by stable indices. Keep simulation tiles, entity arenas, collision and all progression private. This targets the measured world/build family without assuming that clients with the same map coordinates have identical worlds. Gate implementation of the broad tile split on the bounded layout/occupancy discriminator below.
3. Do not commit to inter-client scene sharing or transient GPU-resource pooling yet. Existing native Windows N16 focused-one versus focused-plus-background evidence must determine whether additional renderer residency is a material deployment blocker. If that difference dominates, a separate completion-leased transient-buffer design is more credible than multiplexing entire renderers, but is a genuine architecture choice for root, not a concealed detail of item 1.

The present evidence does NOT establish sufficient avoidable owners to promise either the Linux active increment target or Windows N1 budget. In particular, reducing GPU allocations cannot repair a TUI gap. A useful design result is a bounded removal and an explicit insufficiency finding, rather than attributing hundreds of MiB to unspecified 'modern overhead'. No change to targets is proposed.

Independent initial recommendation: no other new architecture fan-out report was read. This is design only; no build, native probe, network call, live workload, source/client edit or 289 work was performed.

## Evidence and source boundary

Source inspected in the designated checkout: host HEAD `37ca48e952bb345c59e7df006bc79f813efe1880`, client gitlink/checkout `3456edc8dabf7b25ada78110ffa56327af9f67a4`. Host branch was `codex/memory-diagnostics`. Existing unrelated `crates/script/src/load.rs` modifications and untracked evidence were not changed. Paths below beginning `client/`, `core/`, `dash3d/`, or `render/` are relative to `vendor/fr-client-rust/crates/client/src/`.

This source is NOT the Windows diagnostic branch at host `36825a9` / client `5ee9b6e`. Windows adapter selectors and acquire/submit instrumentation in the supplied report must not be described as already integrated into this checkout. The report's lock-adjacent timing is an association, not proof of dynamic lock causality.

Read authorities: behavior-contract.md including game fidelity; performance-finish-plan.md deployment budgets and sections 4A/4B/4C; native-platform-resource-screen-report.md; windows-adapter-clamshell-report.md; targeted-allocation-owners.md; incremental-owner-attribution-report.md; active STATE pointer and docs/execution.md. Historical instructions in owner reports are not current scope restrictions or new launch permissions.

### Gaps, not accepted performance results

- Native Linux same-binary diagnostic N1/N16 RSS: 175,255,552 / 640,348,160 bytes. The N16 median gap to 512 MiB is 98.68359375 MiB; finite difference is 29.5697917 MiB/additional active slot against 16 MiB. Meeting that increment requires 13.5697917 MiB less per additional slot, or 203.546875 MiB across the N+15 difference. Meeting total N16 RSS alone is therefore substantially weaker than meeting the increment. The profiled 0.72958343 cores also misses 0.5; this task has no clean CPU saving estimate.
- The five later Windows N1 diagnostic medians span 527,222,784–663,781,376 bytes: 118.798828–249.03125 MiB over the 384 MiB target. Resources-gate provenance/overhead remains unavailable. These are real reported current-RSS samples, not accepted savings or peaks.
- Intel's higher observed N1 RSS did not make it a worse cadence choice: closed Intel retained approximately 49 client ticks/callbacks per second while closed NVIDIA fell to approximately 28, with scheduling p99 250–500 ms. No universal adapter or lid policy follows. A cheaper-looking residency sample cannot excuse game-loop stalls.
- No native N16 panel result is established by these supplied reports. Do not extrapolate a Windows N16 absolute number from Linux slopes or old Mac panel data.

Arithmetic above and payload calculations below were evaluated with `bc`, not inferred from allocator/RSS subtraction.

## Owner accounting: what exists already

| Owner and consumers | Sharing/lifetime in current source | Evidence and remaining opportunity |
|---|---|---|
| Decoded config and interface templates | `Client.cache`, `ifaces`, `ifaces_mut`, client/client.rs:300–315; `host::prepare_client`, crates/host/src/lib.rs:116–125. Immutable tables shared; mutable interface slots COW independently. | Historical N1/N16 template 8.565 MiB, other shared unpack 5.290 MiB stayed fixed. Do not multiply by 16. Do not merge live interface mutations. |
| Navigation, animation, sound | Historical owner capture has one navigation family, animation/sound flat with N; STATE records later animation-base sharing. | 70.494 MiB nav, 26.521 animation, 13.301 sound were historical allocated families, not resident additions per client. The old animation-base duplication is not available to remove again. Audio queues/control remain account-local; no mute or altered music order proposed. |
| Appearance/entity tables | client/client.rs:339–358 already boxes players, NPCs and occupied appearance packets. | The historical 4.109375 MiB/client empty-appearance reduction has shipped. A new sparse-entity-table recommendation would double count it. |
| Mutable simulation scene | `World`, core/world.rs:41–104; boxed `Square`, dash3d/square.rs:11–55; construction at core/world.rs:107; `set_ground`:256. | Historical exclusive world/build family 8.298/66.432 MiB N1/N16, difference 58.134 MiB. This is the best large remaining non-snapshot per-client lead, but includes necessary state and is not an exact current field census. |
| Renderer and scene products | Optional `SlotLoop.renderer`; host client_frame:419–501. `Renderer` owns `RenderWorld`, framebuffer, per-view Pix3D, media Arc and minimap/chrome; render/renderer.rs:48–130. | TUI already constructs no renderer. Draw-off drops the head and dematerializes overlays; 1 fps skip-paint deliberately retains the head. Removing heads on every skipped paint would be a regression, not a free saving. |
| Shared raw geometry and final scene models | dash3d/store.rs:1–35 shares bounded raw decode LRUs; transformed products and render/world.rs:163–189 tile/sprite caches remain separate. | `run_share_light`:1402 and `apply_share_light`:1446 resolve/merge/modify scene-local normals and faces. Same model ID is not sufficient to share a final lit model. Old 66.297 MiB N32 light-path attribution is not 66 MiB fixed overhead or a current N16 saving. |
| Shared GPU context/assets | render/backend/gpu.rs:81–95,112–139; injected from crates/panel/src/app.rs:4078. One device/queue, assets, shaders and pipelines shared per process. | 'Share the GPU device/atlas' is already implemented. `OnceLock` intentionally retains context/assets until process exit; Stop need not make this zero. No bounded RSS amount is established for changing that lifetime. |
| Per-renderer GPU/CPU composites | gpu.rs:528–599: scene/depth, two composite targets plus stable presentation texture, vertex buffer, chrome/minimap textures and CPU scratch. | These are actual per-head owners. Vertex capacity starts at 1 MiB (:675) and grows. FBO/texture bytes are not equal to driver committed/resident bytes or CPU RSS. Scene/minimap freeze state must remain per head. |
| Host frame transport | crates/host/src/slot_io.rs:106–159, `FrameBuf` stores one latest `FrameOutput`, `take` transfers it. | Already bounded, no GPU CPU-readback ring. GPU view aliases client texture; mailbox replacement does not allocate another complete GPU frame. No 'zero-copy transport' windfall remains here. |
| Panel game/rail/grid views | crates/panel/src/game_view.rs:50–113 allocates RGBA scratch AND owned upload texture even for GPU-only use; `bind`:140 leaves both allocated. app.rs:1085/3590 builds pane/tile views. | Concrete redundant ownership: GPU direct binding (:116–150) needs neither CPU upload scratch nor owned upload texture. `dispose` already removes off-mode tile views; do not claim that existing detach saving again. Pane and rail can each retain a view: count actual view instances, not bots. |
| Window/surface/driver | crates/panel/src/window.rs:225–332: shared device, physical-size surface, latency hint 2, optional offscreen only when copy-source unsupported. | Surface allocations, driver slabs, shader caches and mappings are distinct from tracked client GPU allocations. Historical Mac IOAccelerator 420.4M versus ~17.5 MiB tracked GPU is only an evidence snapshot in STATE, not Windows attribution or a reclaimable pool. |

Do not add the table's categories into RSS. The old targeted allocation filters overlap; the later exclusive family report is still instrumented allocation evidence. Foreign platform residency, CPU heap commitment, live allocation payload, virtual reservation and GPU resource capacity require separate columns.

## Concrete design A: presentation storage follows the frame variant

Keep `FrameOutput`, `TextureHandle`, `FrameBuf::store/take`, GPU device injection, rendering cadence and compatibility APIs unchanged. Change only the private `GameView` representation and its panel callers:

- Replace unconditional `texture/view/rgba` with an optional CPU-upload owner. A GPU binding retains the client's texture handle and ImGui registration, not a dormant second applet texture.
- Keep a shared initialized placeholder registration for a view without its first frame. Preserve current initial/empty pixels, dimensions and input hit rectangle; do not display an uninitialized texture. Account for the placeholder once.
- On first `PixMap` frame, allocate the upload owner, convert/upload as today, then register it. On GPU transition, unregister old CPU registration before relinquishing that owner; retain valid resources through already-submitted UI work according to wgpu lifetime rules. Never explicitly destroy a still-sampled client texture.
- `unbind_client` must preserve its current visible transition behavior, including any retained CPU frame that is still observable before the next CPU frame. Conservative migration: eliminate never-used CPU owners in GPU-only views first; retain a previously used CPU owner when its old pixels are required. Do not silently replace a historically retained CPU frame with black to satisfy a claimed lower bound.
- `dispose`, restart, focus/rail/grid changes release the corresponding registration exactly once. No new texture registration on each steady GPU frame. Leave the one-consumer-per-mailbox policy intact.

Exact seams: game_view.rs `GameView`, `Bound`, `init`, `present`, `bind`, `unbind_client`, `upload`, `dispose`; app.rs `game_pane` and `cell_body`. This change needs no client API or engine edit.

### Minimum credible improvement and ceiling

A GPU-only initialized GameView currently requests 765 × 503 × 4 = 1,539,180 CPU bytes (1.46787643 MiB) for `rgba`, and one same-sized nominal RGBA8 GPU texture. For each such view after its first GPU bind, design A can remove that CPU allocation and that texture resource, minus the amortized shared placeholder. Sixteen qualifying views mean 23.48602294 MiB CPU allocation payload and separately 23.48602294 MiB nominal GPU payload removed. These are two accounting domains, NOT a 46.97 MiB RSS prediction. Views that need preserved old CPU contents do not count in this lower bound. Actual pane/rail overlap determines V; use `V * 1,539,180`, not a guessed N-to-V ratio.

This is both the credible structural estimate and upper payload bound for this narrow change. Guaranteed measured RSS improvement is zero until a clean native pair establishes residency/reclamation; allocator demand-zero behavior and driver allocation granularity can change the realized result. Metal-padding comments in source are not a portable multiplier. Even a successful one-view removal is nowhere near the 119–249 MiB Windows fixed gap.

Do not remove the client's ping-pong/stable presentation targets as part of this patch. That is a different synchronization design with a different oracle; the CPU upload optimization does not make the client's targets redundant.

## Concrete design B: compact instance scene, without sharing the game

The next meaningful client-side design boundary is `World` versus `RenderWorld`, not host snapshots. Current `Square` carries per-render fill flags/stamps, inline `quick_ground`, boxed `ground`, boxed `overlay_stamp`, placement and mutable occupants together. A headless client still allocates the tile structure and terrain stamps even though no render fill executes. `set_ground` ensures lower-level squares and allocates a stamp; `push_down` can retain a linked former square; overlay attach/detach traverses linked squares too.

Proposed private layout, in explicit stages:

1. Per-build terrain arena of densely stored `GroundStamp` values and compact optional indices from tiles. Keep all 17 i32 fields exactly; this is lossless storage, no quantization or recomputation from current heightmaps. Source ground.rs:345–368 explains why stored heights differ after `push_down`. Preserve optional presence and linked-square identity. Updating an existing tile overwrites its stamp slot rather than appending; tile deletion returns its slot to a bounded free list, and moving a linked tile moves its identity without aliasing another tile. Arena lifetime is the world's build lifetime; reset invalidates all indices together. No global cache or locks in packet/mainloop paths.
2. Split renderer-only tile materialization and render flags into renderer-owned side arrays keyed by stable tile/linked-tile identity. Leave coordinates, placement, occupant IDs/spans, model-change stamps and picking handshake where simulation consumers require them. `World.click/ground_x/ground_z` cannot simply move: a draw writes the pick consumed by `game_loop` (core/world.rs:74–82). Do not relocate an untraced field based only on its name.
3. Use the existing attach boundary to materialize overlays and light models; preserve `build_generation`, model stamps, linked-square ordering, `overlay_pending`, `share_light_pending` and minimap invalidation. Dematerialization at core/world.rs:1004 is the current lifecycle oracle. When a head detaches, renderer-owned side arrays drop; the compact recipe remains ready to rebuild at the same next paint.

Keep maintained `Client::mainloop`, renderer `mainredraw`, `World` operations and host APIs. Changes stay within client representation and render consumers; add no bot API to client. This is a bounded storage refactor, but broader than changing a field to Arc: private accessors and all direct tile accesses must migrate together. Public `Square` fields and existing client integration fixtures require a reviewed compatibility decision before removal, not an unannounced external source API break.

### How large could this be?

The historical world/build difference is only 58.134 MiB across N+15, approximately 3.8756 MiB/additional client. That entire difference is an intentionally impossible-to-achieve elimination ceiling for THAT historical family, not a current RSS upper bound; required simulation data cannot all disappear. N16's whole historical family is 66.432 MiB, also not fully avoidable. A source-grounded stamp example is 104 × 104 × 17 × 4 = 0.70141601 MiB payload for one fully populated level, BEFORE indices, boxes and allocator overhead. Removing boxes does not remove stamp payload. Occupancy and real target layouts are missing; no positive MiB reduction for the broad split is yet guaranteed.

For sizing the discriminator, record `old_live_tile_bytes + old_stamp_allocated_bytes + render_only_bytes_in_sim - replacement_arena_capacity - replacement_indices - remaining_tiles - per_head_side_arrays`, separately by mode and phase. Require actual target `size_of` and counts before approving implementation. Initial minimum is zero measured RSS, with a goal of eliminating demonstrable per-tile overhead rather than promising the entire family. Reject expansion if the replacement allocation is not smaller or merely moves equal bytes between owners in the mode needing improvement.

The constructor's old 84.742 MiB N+15 family difference also included the already-fixed appearance storage. Subtracting only the structural 15 × 4.109375 MiB gives an illustrative remaining allocation envelope of 23.101375 MiB, NOT current live or resident savings. Even these overgenerous historical envelopes do not establish the 203.546875 MiB incremental reduction required on native Linux. Thus architecture C alone has no supported full-budget closure argument; coordinate with root's independent host/script work without double counting it.

## Sharing alternatives and reasons not to choose them now

- Whole immutable scene-base sharing with COW dynamic overlays: plausible future scope, but not the first implementation. Map identity alone omits build plane/lowmem, colour/lighting state, dynamic loc/var effects, placement, linked squares and differing packet histories. A safe key must cover exact constructed content with collision-checked equality, and mutations need independent overlays. Computing hashes on every tick would add work to the existing CPU miss. Stamp-arena layout first gives a measurable owner; do not build a global scene cache without a duplicate-content census at rebuild boundaries.
- Share final lit models by model ID: incorrect. Neighbor-normal merge and face removal depend on placements and scene neighbors; animation and contour transforms are mutable. Raw decode is already shared. An immutable post-light product keyed by ALL transforms/neighborhood inputs is a separate, high-maintenance design and has no demonstrated current duplicate-byte total.
- One renderer time-sliced across all background clients: reject for this step. `Renderer` contains per-client chrome, minimap, title, random/flame, picking/camera-related and freeze state, not just a device. Rebuilding on every 1 fps paint can repeat share-light and attach work. Holding each client's required state while pooling only temporary submission storage is not actually eliminating those renderers.
- Completion-leased transient GPU pool: conditional future option if native background delta dominates. Preserve persistent scene/minimap/stable presentation per head; lease only validated scratch such as streaming vertex buffers, depth or composite work targets until GPU completion, never until CPU submit return. Bound leases, count retained last frames and peak transition capacity, and prove no resource is overwritten while sampled. Queue contention/lease waits must remain outside an altered game progression path. Current evidence does not size a safe pool or prove CPU RSS savings.
- Reduce background cadence/resolution, eliminate last-frame retention, mute audio, lower logical ticks, defer game work to paint: rejected by required behavior/modes. Mainloop and mainredraw ordering at host lib.rs:398–501 remains unchanged, including camera/minimap work at that seam.
- Different adapter/backend, surface buffering or process split: root decision after native ownership evidence, not an optimization hidden in measurement. Another process must have explicit helper accounting; moving driver residency out of host RSS is not savings. Vulkan Intel/NVIDIA diagnostic association does not prove backend causality or that selecting the lower-RSS adapter preserves clamshell fidelity.
- Flush all shared caches on Stop: context lifetime is not a leak; arbitrary eviction can create restart latency and rebuild peaks. No measured avoidable current fixed-cache owner supports it.

## Smallest discriminating experiment and migration

No experiment was executed here. These are bounded proposals for separately authorized work, not permission to consume native hosts during the current campaign runs.

A. First implement only the never-used CPU upload owner removal in GameView. Use an instrumented fake `FrameGpu` unit fixture plus the existing real-GPU game_view tests to compare resource creations/releases after GPU-first bind. Pass condition: zero applet CPU upload owners/textures on GPU-only views, identical stable registrations and output, unchanged CPU fallback/transition behavior. This source-level allocation proof is cheaper and more discriminating than another N1 RSS run dominated by drivers. If it cannot preserve visible transitions, retain the affected owner and narrow the estimate, rather than change the oracle.

B. Before broad client layout changes, one bounded existing-fixture census: load one scene unheaded, attach, then detach without changing simulation; count occupied and linked tiles, stamps, render-only fields, arena capacities and current target sizes. Repeat a different scene/build plane in the same fixture and compare the proposed compact layout on paper or a pure layout test. No arbitrary repeated native profiling. Stop at this decision: positive material structural saving with same operations -> scoped implementation; too small/negative -> park broad split and report remaining unowned gap. Never use the archival allocation family as the census itself.

C. Root's pending native Intel N16 focused-one and focused-plus-background diagnostics are the platform discriminator, using matched binary, console/lid/adapter, geometry, cache/memory/audio settings and fixture. Collect actual renderer counts (1 versus 16), actual 1 fps background paint and focused full rate, current RSS/private/shared, tracked GPU buffer/texture capacities, view-owner counts, startup/transition peaks, process CPU and per-slot scheduling. Record server/helpers separately. A failure before observe-end is not a zero-cost cell or a valid subtraction.

Interpretation of that N16 result:

- Focused-one N16 has a near-constant panel surcharge relative to a matched native Windows TUI scale curve, with similar N+15 slope: prioritize fixed window/device/driver residency; client or snapshot changes cannot erase the fixed surcharge. Linux TUI is not the matching control.
- Focused-plus-background minus focused-one is large and tracks per-head GPU/CPU resource counts: apply A, then consider the narrowly leased transient pool, with budget accounting for the persistent per-head floor. If the delta is small while N1 remains expensive, do not rewrite renderer multiplexing to chase the wrong term.
- One-renderer slope itself is high after owner counts confirm only one actual renderer: compact instance scene and host/script storage remain relevant; attributing that slope to background GPUs would be false.
- A nonlinear CPU/cadence miss without corresponding byte growth points to scheduling/submission contention, not a memory-owner redesign. Any new N16 failure needs attribution before another performance cell.

N16 can route ownership work; two short unpaired mode cells cannot prove a savings law or final performance acceptance. After candidate source review, use the approved short alternating comparison and one longer confirmation, separate profiles-off resources from latency/owner diagnostics, stop or investigate a named confounder when inconclusive. No infinite profiling loop.

## Behavior risks and explicit oracles

| Risk | Required oracle before integration |
|---|---|
| Render storage change alters game progression | Preserve input drain -> latch -> mainloop -> chosen mainredraw -> post-drain/guardian order at host client_frame; compare deterministic packet/input/action and movement/animation/camera/audio sequences. No variable-step simulation, catch-up batching or changed 20 ms target. |
| Missing or stale CPU/GPU transition pixels | game_view steady texture identity and registration tests; CPU-first, GPU-first, both switch directions, no-new-frame transition and forced GPU-init failure. Inspect actual captures, not filenames. |
| Cross-client frame or use-after-retirement | Focus rotation, rail/grid, restart same slot name, detach with pending mailbox/frame, resize with in-flight submit; assert registration/owner cleanup and correct slot/frame attribution. |
| Terrain/linked tile corruption | Client tests world, client_build, rebuild, collision, zone, walk, do_action, gens and entity_move; same before/after terrain/placement/collision/action outputs, push_down linked ordering, map reset and independent client mutations. |
| Last-FBO/minimap lost at scene_state 1 | Capture scene-ready -> loading/freeze -> rebuilt scene; previous scene and minimap remain beneath expected loading overlay, then correctly refresh. Run draw, render_backend, GPU texture/depth/overlay and CPU fallback fixtures separately. |
| Shifted render-model invalidation | Same build/model stamps, LOC_ANIM, neighbor lighting, attach-after-headless and repeated detach/attach. No raw-model sharing assumed to prove transformed model equivalence. |
| Script compatibility affected indirectly | Existing host/host-play/script tests with required memory features plus live guarded active qualification, retained observation immutability, bank/wait/cancel/restart, slow/malformed scripts. No compatibility errors or supported API outcomes changed. |
| Resource savings hide cost elsewhere | Allocation payload/owner counts and CPU/GPU residency separate; actual N1/N16 modes, 5% CPU and 2 ms p99 non-regression margins, absolute targets, input/decoded latency, lifecycle plateau and final whole-branch Grok 4.6 remain mandatory. |

Host Cargo tests do not execute client integration tests. Existing renderer failures must be retained and compared with the exact baseline; do not weaken them into passing visual proof.

## Handoff

Scope choices for root: accept A's small safe panel removal; authorize or park B after its bounded census; decide separately whether conditional transient pooling, immutable scene-base sharing or platform/backend policy is justified by native N16. No budget relaxation, product-mode removal or engine rewrite is bundled into this recommendation. Internal enum naming, arena index bookkeeping and resource counter wiring are implementation details once those choices are made.

Verification for this report: read-only source tracing and supplied-report reconciliation; arithmetic evaluated with `bc`; scoped diff/whitespace validation before commit. No tests/builds/live performance runs were permitted or claimed. Report-only commit and same-card Grok 4.5 review requested; root synthesis/integration and final Grok 4.6 remain separate.
