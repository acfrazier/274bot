# Low-end 1/16 three-mode reference screen

Task `t_672f4ac3`. Short screening only (30s warmup / 120s observation / 60s
teardown). **Not** the final 3×600s matrix, target-hardware validation,
lifecycle acceptance, or 128 capacity. No behavior/renderer/scheduling
optimization in this task.

Machine-readable twin:
[low-end-reference-screen-table.json](low-end-reference-screen-table.json).
Raw batch (gitignored):
`docs/memory/diagnostics/low-end-reference-screen-20260906T220129Z/`.

## Provenance

| Item | Value |
|:---|:---|
| Branch | `codex/memory-diagnostics` |
| Host HEAD at runs | `721efd7` (docs freeze commit; source freeze `57cbfc7`) |
| Client | `451759f2` |
| Panel binary sha256 | `ed403b4f1bbce4c6ff86d18417bdf2ab267064f25f5ff4b7f64a3c9daa9b6abb` |
| TUI binary sha256 | `91103790581692e079ff544aa813e9651c689e221f6584df66afcfdfa5789017` |
| Build dir | `docs/memory/diagnostics/reference-build-20260906T215255Z/` |
| Allocator | System (`memory-profile-no-alloc`) |
| Fixture | sustained level-50 Thiever (`--sustain`), loadouts/random events unchanged |
| Clean flags | `--no-diagnostics`, no stack logging, no counting allocator |
| Local server | node `src/app.ts` (Server engine) listening `*:43594` (PID 4719) |
| OS | macOS 15.7.9 arm64 (16-core / 128 GiB Mac) |

Immutable parent binaries were reused; **no rebuild between cells**. Cells ran
sequentially.

## Host contamination (orch correction)

Concurrent Hermes runtime-fix verification began on this Mac at
**2026-09-06T22:21:17Z** (isolated venv + repeated small Python suites through
at least 22:29:40Z; further independent review/testing may have continued).

Classification rule: any cell whose **observation window** ends at/after
22:21:17Z is **DIAGNOSTIC/CONTAMINATED** for CPU / latency / RSS acceptance.
Workload qualification (exit 0, ready/active scale, per-slot steal progress) is
retained. **No blind reruns.**

| Cell | obs window (UTC) | Resource class |
|:---|:---|:---|
| `tui_n1_active` | 22:02:40–22:04:40 | **CLEAN** (provisional profiles-on) |
| `tui_n16_active` | 22:06:51–22:08:50 | **CLEAN** (provisional profiles-on) |
| `panel_focused_one_n1_active` | 22:11:41–22:13:41 | **CLEAN** (provisional profiles-on) |
| `panel_focused_one_n16_active` | 22:15:47–22:17:47 | **CLEAN** (provisional profiles-on) |
| `panel_focused_background_n1_active` | ends after 22:21:17 | **CONTAMINATED** |
| `panel_focused_background_n16_active` | after | **CONTAMINATED** |
| all three profiles-off overhead pairs | after | **CONTAMINATED** |
| nav-captures visual pilot | after | **CONTAMINATED** (functional-only anyway) |

Because every overhead pair is contaminated, **instrumentation overhead is not
assessed**. All profiles-on CPU/RSS figures remain **enabled-only timing
diagnostics** until a later isolated overhead screen. Do not claim CPU/RSS
savings or final budget acceptance from profiler-on data.

Cells needing a later isolated clean screen for resource claims:

1. panel focused-background N=1 and N=16 (profiles on)
2. matched profiles-off overhead pairs for TUI, focused-one, focused-background
   (and optionally responsiveness-profile pair)

## Workload qualification

Every cell: process **exit 0**, `qualify_control.py` (system allocator, no
sidecar) **qualified=true**, all requested slots ready/active during observe,
positive steal gains on every slot. Frontend exit 0 alone was not treated as
qualification.

| Cell | exit | qualified | steals min–max | ticks/slot/s |
|:---|---:|:---:|---:|---:|
| tui_n1_active | 0 | yes | 3 | 42.81 |
| tui_n16_active | 0 | yes | 3–17 | 42.53 |
| panel_focused_one_n1_active | 0 | yes | 8 | 43.69 |
| panel_focused_one_n16_active | 0 | yes | 4–16 | 42.06 |
| panel_focused_background_n1_active | 0 | yes | 7 | 42.39 |
| panel_focused_background_n16_active | 0 | yes | 5–15 | 42.08 |
| tui_n16 profiles_off | 0 | yes | 5–18 | 42.10 |
| panel_fo_n16 profiles_off | 0 | yes | 3–17 | 42.07 |
| panel_fb_n16 profiles_off | 0 | yes | 5–19 | 42.01 |
| nav_captures pilot | 0 | yes | 9 | 40.81 |

Simulation scheduling target (≥40 client iterations/slot/s) holds on all cells
as a **tick-rate observation**; scheduling **interval p99** was **not** collected
(`--scheduling-profile` off on these cells) → **unresolved**.

## Resource table vs working budgets

RSS = process resident bytes median over observe; peak = whole-run
`peak_resident_bytes` max. CPU cores = Δ(user+system CPU seconds) / observe
wall seconds. Values with profiles on are **diagnostic** (see contamination).

MiB = bytes / 1024².

### Clean observation windows only

| Target | Result | vs budget | Qualification | Missing proof |
|:---|---:|:---|:---|:---|
| TUI N=1 median RSS ≤256 MiB | **380.7 MiB** | **MISS** (~1.49×) | workload OK | overhead-corrected RSS; modest hardware |
| TUI N=1 peak RSS ≤384 MiB | **389.0 MiB** | **MISS** (slight) | workload OK | same |
| TUI N=16 median RSS ≤512 MiB | **1001.4 MiB** | **MISS** (~1.96×) | workload OK | same |
| TUI N=16 peak RSS ≤768 MiB | **1021.3 MiB** | **MISS** | workload OK | same |
| TUI N=16 CPU ≤0.5 core | **0.347** (profiles on) | under budget but **diagnostic only** | workload OK | clean profiles-off + overhead pair |
| Panel focused-one N=1 median RSS ≤384 MiB | **612.9 MiB** | **MISS** (~1.60×) | workload OK | overhead-corrected; iGPU HW |
| Panel focused-one N=1 peak ≤512 MiB | **622.6 MiB** | **MISS** | workload OK | same |
| Panel focused-one N=16 median ≤768 MiB | **1269.7 MiB** | **MISS** (~1.65×) | workload OK | same |
| Panel focused-one N=16 peak ≤1 GiB | **1287.4 MiB** | **MISS** | workload OK | same |
| Panel N=16 CPU ≤1 core | **0.449** (profiles on) | under budget but **diagnostic only** | workload OK | clean overhead pair |

Current vs peak: current resident median is reported separately from
`peak_resident_bytes` (ru_maxrss-style peak) in the machine-readable table.

### Contaminated cells (values preserved, not clean acceptance)

| Cell | median RSS MiB | peak MiB | CPU cores | notes |
|:---|---:|---:|---:|:---|
| panel_fb_n1 | 629.6 | 635.3 | 0.225 | functional GPU policy OK; RSS contaminated |
| panel_fb_n16 | 2548.1 | 2560.3 | 0.484 | 16 GPU residents; RSS contaminated |
| tui_n16 profiles_off | 1026.4 | 1030.7 | 0.260 | overhead pair unusable for acceptance |
| panel_fo_n16 profiles_off | 1188.2 | 1229.0 | 0.432 | overhead pair unusable |
| panel_fb_n16 profiles_off | 2559.5 | 2585.1 | 0.524 | overhead pair unusable |
| nav pilot | 649.0 | 731.4 | 0.284 | exclude from clean resources |

No CPU/RSS saving claim is made from profile on/off differences.

## Renderer / GPU policy (actual, not flags alone)

Requested flags do not prove policy. `--render-profile` / `--gpu-completion-profile`
observations:

| Cell | present | backends | full_rate | stable paint ~fps (slot0) | stable p99 upper | GPU reg/comp | coverage |
|:---|---:|:---|---:|---:|---:|:---|---:|
| tui_n1 | **0** | none | 0 | 0 paints | n/a | n/a | n/a |
| tui_n16 | **0** | none | 0 | 0 paints | n/a | n/a | n/a |
| panel_fo_n1 | **1** | gpu×1 | 1 | **43.7** | ≤40 ms | 6979/6979 | **1.0** |
| panel_fo_n16 | **1** | gpu×1 | 1 | **43.1** | ≤25 ms | 9338/9338 | **1.0** |
| panel_fb_n1 | **1** | gpu×1 | 1 | **42.5** | ≤40 ms | 7146/7146 | **1.0** |
| panel_fb_n16 | **16** | gpu×16 | 1 | slot0 **43.1**; bg slots **~0.98–0.99** | slot0 ≤25 ms; bg ≤2000 ms bucket | 15025/15025 | **1.0** |

Findings:

- **TUI:** zero resident renderers and zero host paints — matches `render_policy=none`.
- **Focused-one:** exactly one GPU backend; remaining slots `renderer_present=false`,
  no heads — matches policy.
- **Focused-background N=16:** N GPU backends; only slot0 `full_rate`; background
  stable paint ≈ **1 fps** (requested skip-paint policy observed).
- **No CPU fallback** (`backend_kind` never `cpu`/`cpu_fallback`) on these GPU cells.
- GPU completion counters are **CPU delivery** timestamps (queue callback), **not**
  hardware GPU timestamps or display scanout. Coverage 1.0 with zero drops/lost
  on profiled panel cells supports completed-work counting for this delivery path
  only.
- Host paint cadence for focused full-rate slots is above the 40 completed
  frames/s working target on these samples; p99 upper-bound buckets are ≤40 ms
  (or tighter). This is **not** final presentation-gate acceptance.

## Responsiveness

`--responsiveness-profile` was **not** enabled on any cell in this batch.
decode→script and input→UI p99 remain **unresolved** (unit instrumentation
exists from prior task `t_79e1e21f`; no live overhead/p99 acceptance here).

## Functional / visual checks (outside clean resources)

Nav-captures pilot run:
`docs/memory/diagnostics/20260906T224245Z_panel_n1_active/captures/…`

Three PNGs were **opened and inspected** with an image-capable tool (not
filename inference):

1. **scene-ready** — real ingame 3D interior, minimap, inventory (lobsters),
   chat/overlays, status `ingame scene 2`, thieving script active. Healthy
   render; no blank/login/error.
2. **bank-arrival** — real 3D bank-area interior near counters/ropes; inventory
   full of lobsters; script log “closing the bank” / pathing to bank booth.
   **OSRS bank grid UI was not open in this frame** (capture timing caught
   close/walk, not an open bank window). Still a real scene, not a failure blank.
3. **return-route-start** — header “returning from the bank”, bank trips=1,
   food restocked to 22 lobsters, script resuming pickpocket. Real 3D scene;
   no freeze/blank/error.

Separate functional gaps (explicit unresolved, not passes):

| Check | Status |
|:---|:---|
| Panel focus/watch + overlays | Partially evidenced via pilot UI + running bot chrome |
| Last-FBO freeze while `scene_state==1` | **Not separately exercised** this batch |
| CPU-renderer fallback visual | **Not exercised** (GPU path only; no fallback observed) |
| TUI real PTY path | **Yes** — `terminal=true`, 120×40 PTY, substantial `run.log` drain |
| TUI interaction / resize / error visibility | **Not separately exercised** beyond harness PTY drain |
| Server resource / pressure evidence | **Unresolved** as a measured series; server process was up on :43594 for the batch, but no sampled server RSS/CPU/pressure log was collected |

## Overhead screening

Matched profiles on/off pairs were run for three frontend paths (TUI n16,
panel focused-one n16, panel focused-background n16). **All three profiles-off
cells are host-contaminated**, so overhead is **not accepted**. Directional
(contaminated) CPU deltas are recorded in the JSON table only as diagnostic
noise, not savings.

## Mac-only limitations

- Development Mac (16-core, 128 GiB), not 2c/4GiB VPS or 4c/8GiB iGPU reference.
- No Linux cgroup CPU/RAM quota run.
- Ambient desktop load; concurrent agent test traffic after 22:21:17Z.
- GPU completion ≠ scanout; no modest-hardware claim.

## What next attribution may use

**Allowed now (qualified portions):**

- Workload qualification proof for all ten cells (progress + scale).
- Clean-window **provisional** resource magnitudes for TUI 1/16 and panel
  focused-one 1/16 (profiles-on, overhead unknown) as **misses vs budgets**.
- Actual renderer residency/backend/cadence and GPU completion coverage for
  profiled cells (policy proof), with fb-n16 cadence noted even though RSS is
  contaminated.
- Visual pilot inspection notes and PNG paths.

**Not allowed as clean acceptance:**

- Any contaminated cell’s CPU/RSS/latency.
- Overhead percentages.
- Responsiveness p99, scheduling p99.
- Final budget pass, lifecycle, 128, target hardware.

## Artifact index

Batch:
`docs/memory/diagnostics/low-end-reference-screen-20260906T220129Z/`
(`results.json`, per-cell symlinks, `reference-screen-table.json`).

Per-cell run dirs (also under `docs/memory/diagnostics/`, gitignored) include
`metadata.json`, `samples.jsonl`, `samples.qualification.jsonl`,
`control-qualification.json`, `run.log`, `reference-extract.json`, and hashes in
the machine-readable table’s `artifact_sha256` fields.

## Remaining gap

1. Isolated re-screen of contaminated cells + overhead pairs on a quiet host.
2. Optional `--scheduling-profile` / `--responsiveness-profile` with separate
   overhead pairs.
3. Explicit last-FBO freeze, CPU-fallback, and TUI resize/error functional cells.
4. Separate server resource sampling during client cells.
5. Fixed/incremental cost attribution (`t_6ffdc227`) using only qualified
   portions above — **budgets are missed on every clean RSS target**.
6. Later: final 3×600s matrix, modest hardware, lifecycle, capacity.

**No performance pass is claimed.** Reviewer may approve this honest
failed/incomplete resource picture; failed RSS targets must not be marked
passed.
