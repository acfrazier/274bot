# Windows GPU textured-shade diagnosis

**Task:** `t_63f24b30`  
**Scope:** read-only source diagnosis + native receipt correlation. No patches, no builds/tests in this card, no tolerance/skip/waiver, no savings claim.  
**Workspace at diagnosis write:** host `codex/memory-diagnostics` @ `61b7b7d4823f91900e4601893e5acfa474e2fa96`, client submodule `codex/windows-native-parking` @ `4b3530049a2596f5081efb2f1345d35cbf2053e8`.  
**Native receipt freeze (root):** host `c8b9334` / client `35f1c13`, serial `cargo test --release -p client` on Windows 11 IoT Enterprise LTSC x64 build 26100 (Ultra 9 275HX + Intel iGPU + RTX 5060 Laptop; Rust 1.98.0 / MSVC 17.14 / NASM 3.02).

## Native / cross-OS observations

### Windows receipt (root — not re-run here)

| Item | Value |
|:---|:---|
| Failure | `gpu_texture::gpu_textured_shade_scales_texel_brightness` @ `tests/gpu_texture.rs:485` |
| Message | shade **16** expected **~223**, got **255** |
| Suite slice | `gpu_texture` **9 passed / 1 failed**; earlier GPU backend/mesh tests passed; later integration bins not reached |
| Adapter | **unobserved** — do not assume DX12, Vulkan, Intel, or NVIDIA |
| Other | native panel release build OK; host 179 / host-play 119 OK; memory-profile fixture issues separate |
| Ordering check | root re-running the **exact Windows test EXE** from the failed log **standalone** (rules out suite ordering); result not in this card yet |

### Mac comparison (root mid-run note — same unchanged shader)

```text
cargo test --release -p client --test gpu_texture \
  gpu_textured_shade_scales_texel_brightness -- --exact --nocapture --test-threads=1
→ 1 passed (0.14s)
```

Same client shade shader/test path, **release**, exact filter: **PASS on Mac**. Cross-OS lock/adapter dumps not yet compared.

### Linux (prior forensics)

Same panic text/line on Mesa software stacks (`docs/memory/linux-gpu-shade-forensics.md`). Linux probe proved **bimodal** red hist at shade 16 (majority 223, minority 255), not all-255.

### Classification (memory-campaign constraint)

| Framing | Status |
|:---|:---|
| **Preexisting backend-dependent numerical behavior** of smooth `f32` shade + truncating `u32` | **Best fit** to Mac pass + Windows/Linux fail on **unchanged** shader source |
| **Portability regression** introduced by a Windows-only code path in this shade shader | **Not supported** by source: one `SCENE_SHADER` string; no `cfg(windows)` in the FS shade factor |
| Authorization for global shader redesign / round / flat-interpolate | **Not assumed** — diagnostic only; preserve existing behavior and known errors until a scoped fix card is authorized |

Mac pass is strong evidence the test and mesh path are capable of succeeding under at least one native Metal/wgpu stack; failure is not “the factor path is missing from the binary.”

## Source path under test (precise lines)

### 1. Test fixture and metric

File: `vendor/fr-client-rust/crates/client/tests/gpu_texture.rs`

- Cases table `445–454`: shade 16 → expected red `0xdf` (223 = round of `255 * 0.875`).
- Loop `456–489`: per shade, fresh `textured_pix()` + wall model with `face_colour_{a,b,c} = shade`, `render_type=2` textured, both faces `TEXTURE_RED` (id 7).
- Solid red atlas source `114–120`, `144–151`: palette `[0, 0xff0000]` — full bright texels, no CPU block bake on the GPU path.
- Metric `476–487`: **`max_red`** among pixels with `r > g+40 && r > b+40`, assert `|max_red - expected| <= 8`.
- **Implication:** a **minority** of full-bright red pixels fails the case even if most pixels are correct (Linux probe: 64544 @ 223 + 8874 @ 255).

### 2. Mesh packing (CPU)

File: `vendor/fr-client-rust/crates/client/src/render/world.rs`

- `GpuVertex::pack` `5129–5130`: `abhsl = alpha<<24 | bias<<16 | (shade as u32 & 0xffff)`.
- Textured emit `5904–5938`: corner shades from `face_colour_a/b/c` (type 2 uses per-corner; test sets all equal).
- For shade 16, low 16 bits are exactly `16` on every textured vertex (Linux CPU mesh hist confirmed unimodal `{16:6}`; not re-measured on Windows).

### 3. Scene shader (VS/FS)

File: `vendor/fr-client-rust/crates/client/src/render/backend/gpu.rs` `SCENE_SHADER` `318–463`

| Step | Lines | Behavior |
|:---|:---|:---|
| VS unpack | `409–411`, `425` | `hsl = abhsl & 0xffff`; **`out.hsl = f32(hsl)`** |
| Interpolation | `338–346` | `hsl: f32` is **default smooth** (perspective-correct). Only `tex_id` is `@interpolate(flat)` |
| FS textured | `435–459` | sample atlas; **`let s = u32(in.hsl) & 0x7fu`**; `block = (s >> 4) & 3`; factors `1.0, 0.875, 0.75, 0.625`; bit6 halves |
| Conversion | `455` | WGSL **`u32(f32)` truncates toward zero** (not round-to-nearest) |

Block map for the test cases:

| shade | `s>>4` block | factor | expected red |
|----:|---:|---:|----:|
| 0 | 0 | 1.0 | 255 |
| **16** | **1** | **0.875** | **223** |
| 32 | 2 | 0.75 | 191 |
| 48 | 3 | 0.625 | 159 |
| 64 | 0 + half | 0.5 | 128 |
| … | … | … | … |

If any fragment sees `s=15` instead of `16`, block stays **0** → factor **1.0** → red **255** — exactly the failure mode.

### 4. Intended CPU fixed-point textured shade

File: `vendor/fr-client-rust/crates/client/src/graphics/pix3d.rs`

- `get_texels` `612–653`: four **pre-baked** brightness blocks (high-mem 16384-texel strides): full, `rgb-(rgb>>3)` (~7/8), `rgb-(rgb>>2)` (~3/4), `rgb-(rgb>>2)-(rgb>>3)` (~5/8), with `0xf8f8ff` mask.
- `texture_raster` `3316+`, `3487–3488` / low-mem `3353–3354`: after fixed-point expand, **bits derived from the 0..127 shade** select block via `cur_u` OR and `shade_shift = shade_a >> 23` for the ≥64 right-shift half.
- CPU is **integer fixed-point** along the span; it does **not** convert an interpolated float shade with truncating `u32`.

GPU atlas (`gpu_atlas.rs` `1037–1090`) uploads **only the full-bright** 128×128 layer once per id; brightness is entirely the FS `factor`. That design matches RuneLite-style sampling + shader shade, and matches the test’s solid-red expectation when `factor` is correct.

### 5. Upload / queue / readback (contamination surface)

File: `gpu.rs` `render_scene_for_test` `967–981`, `render_scene` `988–1060`; `backend/mod.rs` `read_back` `48–114`

- Each call: `ensure_model_textures` (skip if id already uploaded) → `write_buffer` mesh → **color LoadOp::Clear BLACK** + depth clear 1.0 → draw → `queue.submit` → copy texture→buffer → `map_async` + `poll` until mapped.
- Scene format **Rgba8Unorm** (explicitly not sRGB) `623–624`.
- Atlas red id 7 is process-global once-upload; shade loop does not re-upload texels (correct for solid red).
- Silent map failure returns **empty** vec → `max_red=0`, not 255. Does **not** explain this panic.
- Stale **prior shade-0 full-bright** pixels would require a failed clear/draw or reading the wrong resource. Code always clears the same `scene_view` before draw. **Not ruled out on Windows without a histogram**, but weaker than the boundary/truncation hypothesis given Linux bimodal evidence on the same shader.

## Hypothesis ranking (honest confidence)

### H1 — Truncating `u32(interpolated float shade)` at block boundaries (primary; backend-sensitive)

**Claim:** Even with equal corner shades, smooth perspective-correct `f32` shade can land slightly below the integer (e.g. `15.999…`) at some fragments; `u32` truncates to the previous bucket; `max_red` sees 255. Magnitude of the under-shoot is **backend/GPU dependent**, so Mac can pass while Windows/Linux software fail on identical source.

**Support:**

- Exact FS math at `gpu.rs:455–458` and VS `out.hsl = f32(hsl)` without flat interpolation.
- Test uses **boundary** shade 16 first failure in the table (after shade 0 which is also a boundary but previous block does not exist / same factor).
- Linux software GL probe (`docs/memory/linux-gpu-shade-forensics.md`, `diagnostics/linux-gpu-shade-out/probe-report.txt`): shade 16 **bimodal** 223×64544 + 255×8874; interiors 17/33/49/65 **unimodal correct**; mesh shades exact. Pattern repeats at 32/48/64/80/96/112 with previous-block contamination.
- Identical panic string on native Windows receipt.
- **Mac release exact test PASSED** on the same unchanged shader (root) — rules out “always wrong on all GPUs” and favors **preexisting numerical sensitivity**, not a Mac-vs-Windows source fork.

**Confidence on mechanism (cross-platform source):** **high** that this code path *can* produce got-255-at-16.  
**Confidence that Windows native pixels are the same bimodal mix:** **medium** — **not measured** on the dual-GPU Windows box; adapter unknown. Symptom match is necessary but not sufficient.  
**Portability-regression vs preexisting:** **lean preexisting backend numerical behavior** until a Windows-only divergent shade path is shown (none found in source).

### H2 — Stale buffer / readback / fixture contamination

**Claim:** Scene or atlas retains shade-0 (255) or another full-bright pass.

**Against:** Clear BLACK each render; readback waits on map; empty readback → 0 not 255; atlas is full-bright by design and factor should darken; Linux fresh-process probe still failed with majority-correct minority-255.

**Confidence as primary Windows cause:** **low**, pending hist. Keep as secondary check in the probe below.

### H3 — Factor path never runs / wrong bit field / always block 0

**Against (Linux):** majority of shade-16 pixels at 223; interiors clean; shader symbols present.  
**Windows:** if **all** red-dom pixels were 255, H3 would rise; root only reported `max_red=255`, which is compatible with H1 minority OR H3 total. **Need histogram.**

**Confidence:** **low** as sole explanation without all-255 hist.

### H4 — Dual-GPU / backend-specific (DX12 vs Vulkan, Intel vs NVIDIA)

**Open.** Adapter model and `wgpu` backend were not logged on the native receipt. Could change FP interpolation noise or rasterization, but the shader source is shared. Do not assume which adapter wgpu picked.

### H5 — Memory-campaign / host coupling

**Not demonstrated.** Failure is in client workspace GPU integration test with frozen client sources; host panel/host tests passed on the same machine.

## What this card does **not** claim

- That Windows histograms match Linux.
- That DX12 or Vulkan is involved.
- That a fix is chosen (round vs flat interpolate vs CPU-style discrete shade — any of those is a **behavior change** needing separate authorization and corner-varying-shade regressions).
- That loosening `max_red` / ±8 is acceptable (it is not proposed).

## Smallest decisive probes for root (run on the actual Windows machine)

Do **not** change production shaders/tests for these. Prefer a scratch binary or `RUST_LOG`/eprintln probe under `docs/memory/diagnostics/` (pattern: existing `linux-gpu-shade-probe`). Freeze the same host/client commits as the failing receipt if possible.

### Probe A — Adapter identity + red-channel histogram (highest value)

Reuse the scene construction from `gpu_textured_shade_scales_texel_brightness` (or the existing Linux probe crate pointed at this client). For shades `{0,15,16,17,32,33,64,65,80,112}`:

1. Print `wgpu::Instance::enumerate_adapters` name/backend/driver_info for **every** adapter and which one `GpuBackend::try_new` selected (force none first; optionally repeat with `WGPU_BACKEND=dx12` / `vulkan` / `gl` if available).
2. After `render_scene_for_test`, build histogram of red channel among red-dominant pixels (same predicate as the test).
3. Also dump mesh textured-shade hist (`abhsl & 0xffff`) before GPU.

**Decision table:**

| Result | Interpretation |
|:---|:---|
| Shade 16: majority ~223 + minority 255; 17 clean; mesh `{16:n}` | **H1 confirmed on Windows** (same class as Linux). Fix card can target shade quantization/interpolation with evidence. |
| Shade 16: **all** ~255; 17 also wrong or all 255 | H3 / factor-path or packing bug on that backend — different fix path. |
| Shade 16 all ~223 but test still fails | Metric/predicate or different binary than source — check build identity. |
| Nonzero garbage / previous-case bins without boundary pattern | Revisit H2 (clear/readback/queue). |

Bounded sketch (illustrative; root freezes/runs):

```rust
// After scene = backend.render_scene_for_test(...):
let mut hist = [0u32; 256];
let mut red_dom = 0u32;
for &rgb in &scene {
    let r = ((rgb >> 16) & 0xff) as usize;
    let g = ((rgb >> 8) & 0xff) as i32;
    let b = (rgb & 0xff) as i32;
    if (r as i32) > g + 40 && (r as i32) > b + 40 {
        hist[r] += 1;
        red_dom += 1;
    }
}
// print shade, expected, max_red, top bins, red_dom
```

Adapter enum sketch:

```rust
let instance = wgpu::Instance::default();
for a in instance.enumerate_adapters(wgpu::Backends::all()) {
    let i = a.get_info();
    eprintln!("adapter name={:?} backend={:?} driver={:?} info={:?}",
        i.name, i.backend, i.driver, i.driver_info);
}
```

### Probe B — Isolate shade-16 only + fresh process

```text
cargo test --release -p client --test gpu_texture \
  gpu_textured_shade_scales_texel_brightness -- --exact --nocapture
```

Single-test process rules out cross-test atlas pollution from earlier `gpu_texture` cases. If it still fails at 16 with the same message, suite ordering is not required for the bug.

Optional one-line mesh assert before render (scratch only): all textured `abhsl & 0xffff == 16`.

### Explicit non-probes (out of scope / low yield first)

- Full client suite again before A/B.
- Epsilon fudge on the assert.
- Mac comparison is root-owned in parallel; this card does not claim Mac results.

## Cross-reference

- Linux measured forensics (Gl/llvmpipe, bimodal proof): `docs/memory/linux-gpu-shade-forensics.md`
- Linux probe receipts: `docs/memory/diagnostics/linux-gpu-shade-out/`
- Earlier undiagnosed suite note: `docs/memory/linux-reference-followup.md`

## Source content hashes (this tree at write)

| File | sha256 |
|:---|:---|
| `.../render/backend/gpu.rs` | `f12964e1ffeeace8195af2cfa656e052da809b2eace1b5894bae6d5d1425e613` |
| `.../tests/gpu_texture.rs` | `053eaef148d5a15e9bfe5da2d55393717707aef675e30a26aa40a22351513e9f` |
| `.../render/world.rs` | `9e2cc21a2c42c9f9956b7bd7e0a7790f0138a975b66413b41e8ca64a432e54e9` |

(Matches the Linux forensics card’s hashes for these three files — same shade shader/test/mesh packing content.)

## Bottom line

The Windows native failure is **the same assertion and first boundary shade** as the known Linux textured-shade failure, while **Mac release exact test passes** on the **same unchanged** shader. Source analysis pins a **plausible, high-confidence mechanism**: smooth `f32` shade + **truncating** `u32(in.hsl)` at block edges (`gpu.rs:425,455–458`), detected by **`max_red`** (`gpu_texture.rs:476–487`). CPU textured shading uses discrete fixed-point block selection (`pix3d.rs` `get_texels` / `texture_raster`), not float truncation.

**Classification under memory-campaign rules:** treat as **preexisting backend-dependent numerical behavior** of the shared shader (exposed on some Windows/Linux stacks, not on this Mac run), **not** as proof of a Windows-only portability code fork and **not** as authorization for a global shader redesign. Preserve the failing assert as-is.

**Root native follow-up, 2026-09-07:** exact standalone EXE reproduction also fails at shade16. Probe A has now run; source, lockfile, adapter enumeration and histograms are preserved under `diagnostics/windows-native-20260907a/`. Its matching-request adapter is NVIDIA RTX5060 Laptop / Vulkan driver616.56. Shade16 contains 41435 red-dominant pixels at223 and31983 at255; shade17 is uniformly223. Other boundary/interior pairs show the same adjacent-band pattern. This supports H1 on the native probe path; direct fragment floating-point values were not instrumented. The separate adapter request does not identify the adapter inside the live panel. See `windows-native-first-proof.md` for current evidence and limits. No total factor-path failure or stale framebuffer claim is warranted; no production shader or assertion was changed.
