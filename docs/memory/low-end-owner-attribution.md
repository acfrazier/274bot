# Low-end fixed/incremental cost attribution

Task `t_6ffdc227`. Plan §§3.3 and 4.A/4.C. Evidence/design only — **no
optimization implementation**, no runner-cleanup rewrite, no budget weakening,
no long final/capacity runs.

Parent reference screen: [low-end-reference-screen.md](low-end-reference-screen.md)
(`f2ffaa0`, batch `low-end-reference-screen-20260906T220129Z`). Frozen binaries
from `t_dac0878e` (panel `ed403b4f…`, tui `91103790…`).

**No performance pass is claimed.** Failed clean RSS budgets remain failed.

## 1. Qualification gate

Contamination cut (orch): observation end ≥ **2026-09-06T22:21:17Z** → resource
cells are diagnostic only. Workload qualification may still be retained.

| Cell | Resource class | Used for matched performance claims? |
|:---|:---|:---|
| `tui_n1_active` | CLEAN | yes (provisional profiles-on) |
| `tui_n16_active` | CLEAN | yes |
| `panel_focused_one_n1_active` | CLEAN | yes |
| `panel_focused_one_n16_active` | CLEAN | yes |
| `panel_focused_background_*` | CONTAMINATED | **no** CPU/RSS/latency |
| all profiles-off overhead pairs | CONTAMINATED | **no** overhead |
| nav-captures pilot | CONTAMINATED | functional only |

Usable clean cells for fixed/incremental **resource** finite differences: the
four rows above. Focused-background incremental GPU residency and overhead
percentages remain **blocked** pending an isolated quiet-host re-screen (one
concrete prerequisite: re-run fb 1/16 + three profiles-off pairs with no
concurrent agent/test load).

All ten cells remain workload-qualified (exit 0, scale, steals). That is not
budget acceptance.

## 2. Clean 1/16 finite differences (profiles-on, provisional)

Source: `docs/memory/low-end-reference-screen-table.json` observation medians.
Profiles-on; overhead unknown — treat CPU/RSS as **enabled-only diagnostics**,
not final savings. Linear model uses N=1 and N=16 only (two points).

Definitions:

- Δ/bot = (metric₁₆ − metric₁) / 15
- linear fixed intercept ≈ metric₁ − Δ/bot  
  (cost attributed to process/shared base if growth were constant per bot)

### Memory (RSS MiB)

| Mode | N=1 med | N=1 peak | N=16 med | N=16 peak | Δ/bot med | Δ/bot peak | linear fixed med |
|:---|---:|---:|---:|---:|---:|---:|---:|
| TUI (`render_policy=none`) | 380.703 | 389.000 | 1001.406 | 1021.344 | **41.380** | 42.156 | **339.323** |
| Panel focused-one | 612.875 | 622.641 | 1269.656 | 1287.375 | **43.785** | 44.316 | **569.090** |

Budget gaps (median): TUI n1 **+124.7** vs ≤256; TUI n16 **+489.4** vs ≤512;
panel fo n1 **+228.9** vs ≤384; panel fo n16 **+501.7** vs ≤768. All **MISS**.

Panel−TUI at N=1 (frontend/GPU path delta, same frozen host/client):  
**+232.2 MiB** median RSS. Not a pure GPU-buffer size (see §3 GPU).

### CPU (cores over observe; profiles-on → diagnostic only)

| Mode | N=1 | N=16 | Δ/bot |
|:---|---:|---:|---:|
| TUI | 0.0517 | 0.3461 | 0.0196 |
| Panel focused-one | 0.4285 | 0.4489 | **0.0014** |

TUI CPU scales with bots; focused-one CPU is almost flat N1→N16 on this sample
(one full-rate GPU slot dominates). Contaminated fb-n16 CPU must not be used.

Tick rate ~42–44 client iterations/slot/s on all clean cells (scheduling
**interval p99 unresolved** — profile off).

### Renderer / GPU residency (clean cells + policy-only notes)

| Cell | present | backend | full_rate | median `gpu_tracked_bytes` | host paint |
|:---|---:|:---|---:|---:|:---|
| tui n1/n16 | 0 | none | 0 | 0 | 0 paints |
| panel fo n1 | 1 | gpu×1 | 1 | 18 301 952 (~17.5 MiB) | ~43.7 fps |
| panel fo n16 | 1 | gpu×1 | 1 | 18 422 408 (~17.6 MiB) | ~43.1 fps |

`gpu_tracked_bytes` barely moves N1→N16 under focused-one (one resident
renderer). It does **not** explain the +232 MiB panel−TUI RSS gap. Contaminated
fb-n16 showed present=16 and ~150 MiB tracked — **not** a clean claim.

GPU completion coverage 1.0 on profiled panel cells is CPU-delivery semantics,
not scanout. No CPU-fallback observed.

### Scheduling / latency

- Scheduling interval p99: **unresolved** (`--scheduling-profile` off).
- Responsiveness decode→script / input→UI p99: **unresolved**.
- Clean cells establish tick-rate floor only.

### Scene / load / noise limits

- Fixture: sustained level-50 Thiever; Mac 16-core/128 GiB; ambient desktop.
- Single short 30/120 sample per cell; no reverse-order pair on this matrix.
- Profiles-on without accepted overhead pair.
- N=1 and N=16 only — nonlinearity between untested.
- Focused-background and overhead excluded by contamination.

## 3. Bounded single-client native attribution (diagnostic, separate)

**Separate** from clean performance cells. Stack-logging-lite perturbs allocator
layout and CPU; **do not** mix these RSS/CPU figures into §2 acceptance.

| Run | Frontend | Binary sha256 | Workload | Capture |
|:---|:---|:---|:---|:---|
| `20260906T225751Z_panel_n1_active` | panel focused-one | `ed403b4f…` | active sustain 30/120 | vmmap + `malloc_history -allBySize` in observe |
| `20260906T230141Z_tui_n1_active` | tui PTY 120×40 | `91103790…` | active sustain 30/120 | same |

Both exit 0; `qualify_control.py --no-write` **qualified=true** (workload only;
`diagnostic_only`). Artifacts gitignored under `docs/memory/diagnostics/`.

Standalone client-play: **not run**. Matching cache/scene/audio/memory/renderer
defaults to host-play was not established in this card; unlike defaults would
contaminate the fixed-cost split (plan §3.3).

### 3.1 vmmap residency (ground truth for “what is resident”)

| | Panel n1 lite | TUI n1 lite |
|:---|---:|---:|
| Physical footprint | 902.0M | 326.3M |
| Footprint peak | 928.7M | 328.3M |
| TOTAL dirty | 885.7M | 325.1M |
| MallocStackLoggingLiteZone **ALLOCATED** | **333.6M** | **226.7M** |
| Zone resident | 446.6M | 265.0M |
| MALLOC_LARGE resident | **137.6M** | **137.6M** |
| IOAccelerator (graphics) resident | **420.4M** | 0 |
| IOSurface resident | 37.4M | 0 |
| Memory Tag 255 virtual | **32.4G** | **32.4G** |
| Memory Tag 255 resident | 11.0M | 7.8M |

Notes:

- **~33 GiB** Memory Tag 255 (+ other reserved VA) is **not** RSS. Excluded from
  ranked resident targets. Do not subtract stack totals from RSS.
- Lite-zone **ALLOCATED** (333.6M / 226.7M) is the better malloc payload ceiling
  than summing `malloc_history` group sizes.
- Panel footprint ≫ clean panel RSS (~613 MiB) because instrumentation + GPU
  driver residency (IOAccelerator) inflate the diagnostic process; clean
  `gpu_tracked_bytes` (~17.5 MiB) still under-counts driver-side graphics.
- TUI footprint 326M is **below** clean TUI RSS 381M — different metric families
  and instrumentation; use for owner ranking inside the capture, not as a new
  baseline.

### 3.2 V8 / JsRuntime — reservations vs live payload

`malloc_history` exclusive “script isolate / JsRuntime” sums ~458 MiB on both
frontends and includes **256 + 128 + 64 MiB** power-of-two groups. Decomposition:

| Class | Panel | TUI |
|:---|---:|---:|
| Large ≥16 MiB V8-tagged groups | 256+128+64 (+ panel also had 32 GiB mmap filtered) | same 256+128+64 |
| Those large groups `mmap` | yes | yes |
| Isolate3new / create_isolate on 64 MiB | yes | yes |
| Small &lt;16 MiB V8-tagged sum | **~9.9 MiB** | similar order |

**Do not rank 256/128/64 MiB V8 groups as resident-memory savings targets.**
They are address-space / heap-reservation style entries. Live V8 payload in this
capture is closer to the small-group sum plus whatever fraction of reservations
is actually dirty — bounded by zone ALLOCATED and footprint, not by the stack
sum. Sample gauges still report v8_used near 0 on many clean samples; treat V8
heap policy as an open investigation, not a measured RSS lever from this card.

### 3.3 Ranked measured contributions (allocator bytes ≠ RSS)

Priority exclusive first-match on filtered groups (groups ≥512 MiB and selected
system UI catalog noise dropped). **Same ranking structure on panel and TUI
where both apply.**

| Rank | Owner (stack filter) | Panel alloc MiB | TUI alloc MiB | Sharing / lifetime | Consumers | Resident evidence | Replacement cost |
|---:|:---|---:|---:|:---|:---|:---|:---|
| 1 | **NavWorld::load_pack** (decode) | **140.988** | **140.988** | **Two full decodes** at N=1: `Play::new` and `Run::prepare` each ~62.141 + ~7.781 MiB siblings | nav routing, seed bounds, host Play world | **MALLOC_LARGE resident 137.6M on both frontends** — strongest RSS-linked fixed owner in this card | Share one Arc of NavWorld across Play and Run::prepare (and seeds); preserve bounds/routes |
| 2 | Panel GPU driver mapping (vmmap, not malloc stack) | **IOAccel 420.4M + IOSurface 37.4M** | 0 | Per focused GPU client; not shared across processes | wgpu/Metal present path | Dominant panel−TUI footprint gap; **≠** `gpu_tracked_bytes` | Renderer residency / texture policy — needs design; out of “small stack fix” |
| 3 | Client unpack / anim snapshot (`spawn_slot_thread` → `load_snapshot` / AnimFrame) | ~34.5 | ~34.5 | Per client construct | dash3d models/frames | Inside malloc zone | Cache/share decoded assets carefully |
| 4 | `Client::construct` (incl. JagFX `load_shared`) | ~20.9 | ~20.9 | Some OnceLock shared | client object | Inside zone | Field-level audit still needed |
| 5 | RenderWorld prepare_scene / share_light path | ~71.4 | ~0 | Per drawing client | GPU scene prepare | Panel malloc; may overlap textures | Only with behavior-preserving render design |
| 6 | wgpu device/buffer stacks | ~70.3 | ~0 | GPU backend | renderer | Partial; driver has more | Same |
| 7 | Host snapshot / `client_frame` rebuild paths | ~16.6 | ~8.3 | Per slot publication | guardian, scripts, panel nav | Grows with N (see prior N=32 owners) | Existing snapshot-consumer candidates |
| 8 | WidgetView/LocView (N=1) | small | ~4.2 | Per snapshot owner | panel nav, scripts, host | Prior N=32: ~46 MiB × multiple owners | Borrowed FP / family narrowing after contracts |
| — | JsRuntime large mmap groups | ~448 tagged | ~448 tagged | Per isolate | scripts | **Not 1:1 RSS** (§3.2) | Do not implement from this sum |
| — | ScenarioRunner pack path | 0 matching large | 0 | Prior runner-share fix held | harness seeds | No third 62 MiB runner pack here | Keep; don’t rewrite |

Prior N=32 targeted owners ([targeted-allocation-owners.md](targeted-allocation-owners.md)):
four snapshot Widget/Loc owners ~179 MiB total; Client::construct ~220 MiB path
sum at N=32; NavWorld ~141 MiB; share_light ~66 MiB. N=1 here reproduces
**NavWorld 141** and shows the **Play vs Run::prepare duplicate** explicitly.
Harness ScenarioRunner pack duplication is **not** reappearing as a third 62 MiB
load in these N=1 captures.

### 3.4 Fixed vs incremental synthesis

| Bucket | Clean RSS signal | Native owner signal |
|:---|:---|:---|
| **Fixed (shared / process)** | TUI intercept ~339 MiB; panel fo ~569 MiB | Dual nav decode ~141 MiB alloc / ~138 MiB MALLOC_LARGE; client construct ~21 MiB; TUI base frontend; panel +GPU driver hundreds of MiB mapped |
| **Incremental / bot** | TUI ~41.4 MiB/bot; panel fo ~43.8 MiB/bot | Per-slot client+script+snapshots; fingerprints/IsolateBuf traffic (prior active captures); not fully decomposed at N=1 alone |
| **Mode delta** | panel fo − TUI ~232 MiB at N=1 | IOAccelerator/IOSurface + render prepare/wgpu malloc; not explained by gpu_tracked ~17.5 MiB |

To hit TUI n1 ≤256 needs ~125 MiB below clean 381 — **one** 62 MiB nav share
cannot close alone even if fully resident. Panel fo n1 ≤384 needs ~229 MiB;
nav share helps fixed base but **GPU mapped residency** dominates the mode gap.

## 4. ONE next bounded behavior-preserving proposal

### Proposal: single shared `NavWorld` for `Play::new` and `Run::prepare`

**Problem.** At N=1 active, both panel and TUI lite captures show **two**
independent `NavWorld::load_pack` decodes (~62.141 MiB primary + ~7.781 MiB
sibling each path): one under `Play::new`, one under `host_play::memory::Run::prepare`.
Combined filter **140.988 MiB**. vmmap **MALLOC_LARGE resident = 137.6M** on
both frontends — consistent with both decodes living in process RSS. Scenario
runner pack fan-out was previously fixed; this remaining pair is **Play vs
memory Run prepare**, still present on the frozen reference binaries.

**Change (subsequent implementation card only).** Pass the existing Play-owned
NavWorld Arc into `Run::prepare` / seed setup so the pack is decoded **once**.
Do not drop navigation; do not change route/bounds semantics; do not rewrite
terminal runner cleanup further.

**Validation**

1. Unit/integration: seed prepare and live Play routing still see identical
   bounds and pack identity (pointer/Arc equality or content hash).
2. Diagnostic lite N=1 TUI (and panel): `malloc_history` shows **one** ~62 MiB
   load_pack primary group, not two; MALLOC_LARGE resident drops toward ~half
   of 137.6M (order-of-magnitude check, not exact).
3. **Clean** paired short screen (no stack logging): TUI n1 and n16 median RSS
   vs frozen reference; expect fixed-base reduction on the order of **tens of
   MiB** if the second decode was fully resident — **do not** claim full 62 MiB
   RSS a priori. CPU non-regression ≤5% diagnostic margin when overhead-known.
4. Workload qualification must remain exit0 + steals/scale on active sustain.

**Expected metric.** Primary: clean TUI n1 median RSS reduction vs
380.7 MiB reference (and n16 fixed component). Secondary: native owner count
for load_pack. Success is measured reduction + behavior preserve — not budget
pass by itself.

**Why this one first.** Largest **RSS-corroborated**, fixed, behavior-local
duplicate with clear owners; smaller than opening GPU texture policy or V8
heap limits; does not depend on contaminated cells.

## 5. Unresolved decisions (explicit)

| Topic | Status |
|:---|:---|
| Responsiveness live p99 + overhead | unresolved |
| Scheduling interval p99 | unresolved |
| Focused-background clean RSS/CPU + 16×GPU incremental | blocked on quiet re-screen |
| Instrumentation overhead % | blocked (pairs contaminated) |
| Modest / target hardware | Mac-only; no claim |
| Lifecycle hour soak / leak proof | not this card |
| Last-FBO freeze, CPU-fallback visual, TUI resize | not exercised here |
| Server resource series | unresolved |
| Standalone client-play matched defaults | not established → skipped |
| V8 heap resident policy | large stack groups ≠ RSS; needs separate gauge design |
| IOAccelerator 420 MiB vs tracked 17 MiB | panel mode gap; design decision later |
| Existing candidates (borrowed FP, ≤2 buffer pool, snapshot family contracts, heightmap CoW) | still open; not selected as *this* next step |
| Terminal runner cleanup acceptance | retain evidence; don’t rewrite here |
| Final 3×600 matrix / 128 capacity | later |

## 6. Artifact index

| Artifact | Role |
|:---|:---|
| [low-end-reference-screen-table.json](low-end-reference-screen-table.json) | Clean cell numbers |
| `diagnostics/20260906T225751Z_panel_n1_active/` | Panel n1 lite vmmap + malloc_history + receipts |
| `diagnostics/20260906T230141Z_tui_n1_active/` | TUI n1 lite same |
| `*/attribution-ranked.json`, `*/attribution-decompose.json` | Parsed owner ranks (gitignored tree) |
| [targeted-allocation-owners.md](targeted-allocation-owners.md) | Prior N=32 owners |
| [snapshot-consumer-audit.md](snapshot-consumer-audit.md) | Snapshot consumer contracts |

## 7. Remaining gap

1. Implement/validate the shared-NavWorld proposal on a follow-up card.
2. Quiet-host re-screen of contaminated fb + overhead pairs.
3. Optional scheduling/responsiveness profile pairs with overhead.
4. After nav share: re-attribute residual fixed cost (client construct, panel
   GPU mapping) before choosing the next proposal.
5. Final matrix, modest hardware, lifecycle, capacity — unchanged later work.

**No performance acceptance. No failed target marked passed.**
