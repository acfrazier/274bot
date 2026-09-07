# Renderer residency after native Intel N16

## Decision

Do not authorize a cross-head completion-leased scratch pool as the next RSS fix. The mode discriminator establishes a material renderer-associated cost, but not its owner: median RSS increases **951.546875 MiB**, while the identifiable GPU scratch is only **74.082–75.619 MiB across all sixteen heads before retaining any replacement leases**. Those are different accounting domains, not a decomposition of RSS. A pool cannot credibly promise the mode delta, and this evidence cannot choose a small nonblocking pool size.

Prioritize a bounded, exclusive CPU render-owner census, especially the fully resolved/lighted `RenderWorld` products and render-triggered `World` materialization, with mesh/sort allocation high-water accounting. Keep the already separate panel lazy-upload work separate. There is also a smaller source-supported GPU experiment: eliminate one internal composite per head without inter-head pooling, subject to submission/visual tests. No production change or experiment is authorized or executed by this report.

Two important corrections to the initial architecture C hypothesis:

- `chrome_rgba` is not merely upload scratch: it retains the previous overlay image for exact dirty comparison. Pooling it unmodified corrupts lazy upload behavior.
- Despite the ping-pong comment, `finish` reads no previous internal composite. It clears the chosen target, copies the persistent scene, draws chrome/minimap, and copies to stable presentation. The persistent history is elsewhere. This makes one work target per head a narrower candidate than renderer multiplexing.

## Evidence boundary and actual raw fields

Authorities: AGENTS.md, docs/execution.md, behavior-contract.md (including the inherited 20 ms logical progression), architecture-c-panel-client-cost.md conditional pool section. Host work is on `codex/memory-diagnostics`. All source references below are to the explicitly designated frozen-source checkout `/Users/acfrazier/experiments/274bot/.worktrees/windows-panel-attribution`, host **36825a9**, client **5ee9b6e**, not an alternative instruction/history tree. Client-relative references start under `vendor/fr-client-rust/crates/client/src/`.

The campaign's client **3456edc** lineage is not the measured instrumentation. Direct file comparison finds the same descriptor-byte accounting and resource owners in `gpu.rs`, but frozen 5ee9 adds submit-expression attribution at scene/copy/chrome submits and monotonic bounded submission recording in `profiling.rs`; host native stage/adapter instrumentation is also separate. Do not call these timers GPU execution timestamps or infer a lock cause. The active campaign has unrelated WIP; this audit does not claim source equivalence of entire trees.

Recomputed directly with jq from both `docs/memory/diagnostics/windows-panel-n16-20260907/{n16-focused-one-console-intel,n16-focused-plus-background-console-intel}/raw-run-01/samples.jsonl`, selecting `.phase == "observe"`, not lifetime/final rows. SHA-256 of these exact files:

- focused-one: `8351cb01a75bc3a9f1ecc453efc03973187a2d4e6fda12c084ca0f7df7564899`
- focused-plus-background: `9641722862bdd38e1cd6692112f67bb11fedeab8eef14ff751975b9cce3424e9`

Each has 119 observe rows. Elapsed endpoints are 103.1156419–222.4790937 s and 122.4445722–241.5067232 s respectively. Every observe row has respectively one or sixteen present renderers. Across all observe `renderer_profile` entries there are **no byte/capacity keys**. This field supplies topology, mode, paint and callback observations, not per-head capacities. Therefore the requested per-head capacity census cannot be recovered from that field. The actual available capacities are the top-level process aggregate `gpu_buffer_bytes` and `gpu_texture_bytes`; the following are measured aggregates and source-derived deductions, explicitly not invented per-head rows.

| Raw measure (bytes; min / median / max across observe) | Focused-one | Focused plus background |
|---|---:|---:|
| `resident_bytes` | 837668864 / 988098560 / 1163481088 | 1403437056 / 1985867776 / 2597179392 |
| `gpu_buffer_bytes` | 1141480 / 1141912 / 1141912 | 17485504 / 18664000 / 19097584 |
| `gpu_texture_bytes` | 17243992 / 17243992 / 17243992 | 131725672 / 131725672 / 131725672 |
| `gpu_tracked_bytes` | 18385472 / 18385904 / 18385904 | 149211176 / 150389672 / 150823256 |
| `gpu_peak_tracked_bytes` | 19525808 / 19527176 / 19527176 | 150331136 / 151509632 / 152030696 |

Peak is the cumulative logical allocation high-water seen at each sample, not current capacity or GPU residency. Buffer replacement briefly counts old plus new descriptors; old wgpu resources may survive their logical owner's drop. Sampled sums of pending callback observations are 0 throughout focused-one and 0–1 in background. Approximately one-second, asynchronously published snapshots cannot bound simultaneous sub-frame scratch leases; zero sampled pending does not mean no in-flight GPU work.

The supplied native result retains full-rate focus, approximately 1 fps background (~0.9904–0.9909 measured), and ~48.6 logical ticks/s. Both original independent full host bindings failed because `pre.server` was omitted. Recovery and the N16 diagnostic report are separate review scopes; this report grants neither original binding success nor final resource/latency/visual/lifecycle acceptance. No background screenshots were captured by that controller. Median mode differences are diagnostic association, not matched optimization savings. Allocation counting was false throughout both observe populations.

## Frozen owner definitions and reconciliation

`profiling.rs:1–40` counts descriptor payload: buffers by requested size, textures by mip/layer/block/sample sizes. It explicitly excludes driver padding, pipelines, bind groups, host UI resources, pending submissions and externally retained views after backend drop. `crates/host-play/src/memory.rs:1086` exports the process sum; `:1223–1299` serializes the separate host renderer/callback registry (`crates/host/src/render_profile.rs:76–140`).

`render/backend/gpu.rs:613–900` owns seven per-head textures and four buffers. Fixed descriptors reconcile exactly:

| Per-head owner | Bytes | Lifetime / safe classification |
|---|---:|---|
| Scene RGBA8, 512×334 | 684032 | Persistent. `scene_ready` and scene-state-1 freeze reuse the last 3D image while overlays change. Cannot return to scratch at scene submit completion. |
| Depth32Float, 512×334 | 684032 | Transient GPU scene-pass storage; cleared on each nonempty scene draw, never consumed as history. Lease through scene completion (or conservatively final frame completion). |
| Two internal 765×503 RGBA8 composites | 3078360 | Work storage, not last-frame history. `finish:1397–1595` uses only `write`, clear → scene copy → chrome load/blend → stable copy. Lease spans both submits; cannot release after the first clear/copy. One composite per active frame is sufficient in a proposed design, not a validated implementation. |
| Stable presentation 765×503 RGBA8 | 1539180 | Persistent per head. Returned texture view is retained by mailbox, panel registrations and submitted UI draws. Neither producer return nor producer completion ends that lifetime. Preserve texture identity and existing shared-queue ordering; never lend this texture to another head. |
| Chrome 765×503 RGBA8 | 1539180 | Persistent dirty-driven atlas, sampled even without uploads. Recreating every background paint adds uploads and changes the existing lazy behavior. |
| Minimap 172×156 RGBA8 | 107328 | Persistent live/held image. Uploaded in scene 2, held through scene 1 under the punched chrome hole. |
| Brightness uniform + two static quads | 16 + 96 + 96 = 208 | Uniform is writable per scene submission; retain privately for minimal design. Static quad bytes could be shared, but trivial and not in the scratch estimate. |
| Streaming vertex buffer | variable, minimum 1048576 | Transient GPU input; whole mesh re-uploaded. Capacity grows exactly to needed bytes, never shrinks (`gpu.rs:988–1062`). Source CPU input can die after `write_buffer`; GPU buffer must remain until consumption completes. |

Textures total **7632112 bytes/head (7.278549 MiB)**. Subtracting one/sixteen times that from each raw texture total yields exactly **9611880 shared texture bytes (9.166603 MiB)** in BOTH modes. The shared model/other atlas `Allocation` owners are in `gpu_atlas.rs:160,277,716`; device, queue, pipelines and asset context are already process-shared (`gpu.rs:81–139`). Thus the measured texture delta is exactly fifteen heads' descriptors, **114481680 bytes (109.178238 MiB)**, not evidence for fifteen devices or driver-size multiplication.

Subtracting 208 per head from raw buffer totals gives apparent aggregate streaming capacities:

- Focused-one: **1141272–1141704 bytes**, median 1141704 (1.088814 MiB).
- Background mode: **17482176–19094256 bytes**, median 18660672 (17.796204 MiB), maximum 18.209702 MiB.

These deductions assume no concurrent profiled readback or replacement overlap in the sampled buffer total; `backend/mod.rs:57` also profiles temporary readback buffers. The allocation-site audit finds no other persistent buffer owner in this counter. Treat them as aggregate capacity envelopes, not exact per-head measurements. Even the deliberately generous maximum-per-head bound is only **3365616 bytes (3.209702 MiB)** after reserving the other fifteen heads' 1 MiB minimum; we do not know which head has that maximum or its contemporaneous mesh length. Shared texture reconciliation is exact, whereas per-head vertex distribution, freed/in-flight physical allocations and driver rounding remain unavailable.

Persistent scene + stable + chrome + minimap alone is **3869720 bytes/head**, or **59.047241 MiB at sixteen heads**, before shared assets, buffers, host views or driver padding. Removing these would violate the retained-image contract, not optimize scratch.

## CPU and RenderWorld ownership: not all staging is disposable

| Owner | Required lifetime / opportunity |
|---|---|
| `GpuBackend.chrome_rgba` | **1545216 bytes** = 3072 padded bytes/row × 503. `finish:1460–1480` compares last upload with current scene overlay, including coverage/alpha semantics. Persistent per head in the current algorithm, despite its scratch comment. A smaller exact comparison history would be a separate algorithm and regression scope. |
| `minimap_rgba` | **119808 bytes** = 768 padded bytes/row × 156, versus 107328 texel payload. Fully filled before `write_texture`; source slice reusable after the call returns, unlike wgpu's internal upload allocation. CPU-local scratch candidate with exclusive use; not a GPU-completion lease. |
| `overlay_coverage` | **171008 bytes**. Cleared at `draw_scene_overlays:1075`, consumed through `finish` comparison/fill. Candidate whole-CPU-frame scratch, not releasable between overlay and finish; clear on every lease and preserve early-return paths. Combined minimap/coverage removal from fifteen idle scratch copies is at most 4.160156 MiB CPU payload before concurrent replacement buffers. |
| Renderer CPU PixMaps/minimap/chrome/title | `renderer.rs:48–165`, `draw.rs:1861–1904`. `draw_area`, game/sidebar/chat/map/back areas carry persistent redraw/freeze state; minimap base is build-dependent, and title/flames/random state belongs to the head. The listed in-game pixel planes plus 512×512 minimap nominally total 3945244 bytes/head when allocated; no raw capacity/residency census proves exact live presence. Do not pool the whole renderer or clear history on a 1 fps skip. |
| `Pix3DDraw` | `pix3d.rs:237–409`: ModelScratch's constructor requests **3296712 bytes/head (3.143990 MiB)** of nested vector payload, predominantly the depth-face bucket. Fifteen extra copies are 47.159843 MiB of requested payload, not necessarily touched/resident GPU-mode pages. Pure projection/sort scratch is CPU-pass-lifetime, but picking results, texture LRU/palette/brightness state and simulation-facing texture averages are not. The 20-row texel pool is 1.25 MiB lowmem / 5 MiB highmem, moved between free and active rows, not two independent pools. CPU interface/model icons can need this even on GPU heads; `draw.rs:1896` initializes it. Do not delete it on the assumption GPU means no CPU raster work. |
| `RenderWorld` persistent products | `world.rs:163–189,1396–1508,2061–2075`: tile/linked `TileModels`, sprite `SceneModel` caches and stamps persist across paints. First share-light resolves the entire scene, not just visible tiles. Build-generation changes clear products; model stamps and dynamic entities have independent invalidation. Render-triggered overlay materialization lives in simulation `World`, so counting only the Renderer misses it. These are the leading unmeasured scalable CPU owner family. |
| Model sharing boundary | `dash3d/model_source.rs:20–59`: `Shared(Arc<Model>)` already exists, but lighting mutations use `Arc::make_mut`; final models depend on placement, neighbor normal merging/face removal, animation and contouring. They are not frame scratch or safely shared merely by model ID/map coordinates. Count unique shared pointees once and COW-owned products separately. |
| `RenderWorld` work arrays and mesh | `world.rs:112–162,183–189,2416,5176–5207`: fill queue, active occluder/sprite buffers, share-light maps and local mesh/sort intermediates are candidates for bounded CPU-pass scratch after their consumers finish. Reinitialize stamp domains if sharing `share_map`/`share_map2`; stale marks from another head are not valid. `vis_backing` is persistent precomputed viewport visibility (665856 bool elements), not an empty per-frame array. `SceneMesh` creates three vectors, sorting creates triangle tuples plus new vertices, then concatenation may grow again. GPU capacity counts neither these CPU peaks nor allocator-retained freed pages. |
| Panel-owned GameView | Frozen `crates/panel/src/game_view.rs:50–110` allocates 1539180 CPU bytes and another nominal 1539180-byte upload texture per view even for GPU binding. This is outside client counters. Count actual pane/rail/grid views, not renderer count; aliasing a client texture does not create a second client presentation image. Separate lazy-owner card owns this fix; no double-counted pool saving here. |

Largest unmeasured categories are therefore (a) persistent lighted/transformed render models and materialized scene overlays, (b) CPU mesh/sort/model temporary peaks and allocator arenas retaining their freed storage, (c) CPU pixel/raster owners just enumerated, and (d) driver resource heaps/suballocation alignment, staging from `write_buffer`/`write_texture`, deferred old vertex allocations, command buffers/descriptors, UI/surface images, and mapped shared/system GPU memory. Their ranking by actual native resident bytes is unknown. Do not subtract 125.889 MiB of logical GPU payload from 951.547 MiB RSS and call the remainder measured CPU heap.

## Pool ceiling, overlap and the smaller alternative

The most generous identifiable old GPU scratch bucket is depth + BOTH internal composites + streaming vertices: **74.081848 / 75.205750 / 75.619247 MiB** (min/median/max background sampled envelope). Eliminating every byte is impossible while still drawing. Chrome CPU history and persistent images are excluded; GPU upload staging/driver effects are unknown, not an assumed bonus.

For a one-composite-per-lease design, let K be simultaneous leased bundles, C_j each lease's allocated vertex capacity (including alignment/capacity-bin rounding), and R retired-but-not-completed old buffers. Logical replacement bytes are:

    K * (684032 + 1539180) + sum(C_j) + R

Subtract those from the old scratch envelope; separately account driver allocation padding, staging and all persistent images. Four 4-MiB vertex bundles would leave an illustrative **50.724869 MiB** logical reduction at the old median. Four-MiB bins cover the conservative sampled per-head envelope, not arbitrary future scenes. Sixteen such bundles instead **increase** logical scratch by 22.717773 MiB. Neither K=4 nor the bin size is validated. Transition/rebuild peaks, old-plus-new growth, asynchronous host submissions, and focus rotation may exhaust a small pool even if average background paints are sparse. Current callback telemetry has no lossless acquisition/retirement timeline and cannot supply K.

A one-work-composite-per-head change alone has a clean structural ceiling of **1539180 bytes/head**, **23.486023 MiB for sixteen heads**, with no cross-head lease contention and no vertex-capacity inflation. Source reads support it because each target's old contents are discarded and it is never exported, but source is not synchronization/visual proof. Reuse must remain ordered on the same queue through the stable copy before the next clear; preserve the existing submission structure and presentation identity. This is a better bounded GPU experiment if root wants a GPU-only candidate, not the leading explanation of ~952 MiB RSS.

### Minimum design if a later owner/overlap measurement justifies pooling

- Keep Renderer, RenderWorld products, scene/chrome/minimap and stable presentation per head. Lease only private depth, one composite, and vertex input for a frame; preserve current mainloop/mainredraw ordering and focused full-rate/background 1 fps scheduling.
- Exclusive lease starts before first upload/encoding, covers scene submit and both finish submits, and retires only after the last copy to that head's stable texture completes on the shared queue. Completion notification returns only the lease, never a mutable client reference. A CPU return, telemetry permit release, canceled callback or producer drop is not GPU completion. Loss/device loss quarantines resources until safe teardown, not reuse.
- Keep queue-write/submit ordering explicit: no second writer can queue writes into the same vertex lease before its prior consumer, including writes queued by another thread. Continue to order stable presentation updates and panel sampling on the shared queue; do not change the existing latest-frame semantics into mutable cross-client content. UI views are never pool entries. Resizes/restarts carry device and slot-generation identity; retired resources stay charged until actual completion/last reference.
- A nonblocking try-acquire needs a capacity-miss policy. Waiting/polling on GPU completion in `client_frame`, skipping a scheduled paint, delaying its mainloop or adding unbounded emergency allocations are all unacceptable. A bounded preallocated private fallback preserves the existing path but must be counted and can erase the saving. An off-thread renderer would require a separate state/progression analysis, not a pool implementation detail. Until measured overlap proves a useful capacity with a nonregressing fallback, this is precisely why a small shared pool is not selected.

## One most discriminating next instrument

Propose one opt-in **exclusive headed-owner census with phase high-water counters**, not another unchanged RSS-only run. Root can implement it at renderer attach, post-first-share-light, steady observe boundaries, rebuild, and detach in a separately reviewed diagnostic binary. Emit slot generation/build generation, target `size_of`, len/capacity and uniquely owned payload totals for RenderWorld tile/linked/sprite caches and nested model arrays; separately deduplicate Arc pointees, count materialized World overlays, PixMaps/minimap, Pix3D scratch/active-plus-free texels, GPU-side CPU arrays and actual panel view owners. Add scoped live/high-water counters at mesh/sort/concatenation and vertex replacement sites. Aggregate per-phase on the owning thread and publish bounded scalar rows; no full model walk, allocation backtrace or extra lock on every 20 ms tick. Preserve all existing byte/cadence gate fields and distinguish logical counters from native residency.

This most directly discriminates the large uncounted render CPU family from the already-small enumerated GPU scratch. Compare census growth with separately sampled process RSS/private working set in the same bounded mode diagnostic; do not make them sum by definition. If persistent/temporary CPU payload stays small while RSS grows, stop pursuing model pooling and route the remaining question to native driver/allocator resident-page attribution. If fully resolved model products dominate, use their actual capacities and COW duplication count to choose a representation candidate; do not deduplicate final geometry without an exact lighting/mutation contract. No positive savings claim is possible from today's raw fields alone.

## Mandatory validation before any candidate integration

1. Baseline-first client GPU tests: distinct per-head scenes/depth/brightness; overlapping submissions with deliberately delayed completion; no lease reuse before final copy; old/new buffer growth; canceled/delayed callback and device loss; no zeroing/clear of still-sampled output. Assert persistent IDs, pool/retired byte counts and lifecycle plateau.
2. Scene 2 → scene 1 → rebuilt scene, freeze with moving/removing overlays and main modal/empty mesh, dirty versus unchanged chrome, minimap hole/held image, title transitions; exact baseline pixels plus directly viewed captures. Run CPU fallback and GPU-init-failure paths. Retain existing renderer failures rather than weaken expectations.
3. Focus rotation, rail/grid, draw-off/on, restart same slot while mailbox/UI retain old frames, resize with submits in flight. Verify correct account/frame attribution, registrations and lifetime cleanup. A single completed frame or callback is not scanout proof.
4. Independent client integration suites for draw/render_backend/GPU texture/depth/overlay and world/build/model invalidation, plus affected host/panel/host-play feature tests. Preserve supported script/API outcomes, packet/input/mainloop/mainredraw order, camera/picking/audio and 20 ms game fidelity.
5. Separately authorized native N1/N16 mode evidence with profiles-off resources and owner/latency companions, measured overlap/peaks including rebuild/focus bursts and pool misses, same frozen lineage/configuration, qualified gameplay and unchanged gates. Verify focused full rate, background ~1 fps, logical cadence, CPU/p99 nonregression, final lifecycle/resource requirements. Root owns later proof and final whole-branch Grok review.

Verification here: read-only frozen source trace, direct jq aggregation of all 238 observe rows, source-derived arithmetic via jq, file hashes via shasum, and scoped documentation whitespace/diff checks. No builds, tests, native/live/network actions, production edits, client git mutations, or 289 work. This report is the only authored file; same-card reviewer handoff follows its scoped commit.
