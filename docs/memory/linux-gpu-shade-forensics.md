# Linux GPU textured shade forensics

**Task:** `t_f4cb042f`  
**Branch:** `codex/memory-diagnostics` @ `23a110326e1383f28806363f01aed97a90902c20`  
**Date (UTC):** 2026-09-07  
**Scope:** diagnostic only — no production/test/shader behavior changes.

## Verdict (observed vs inferred)

| Class | Finding |
|:---|:---|
| **Observed** | Existing compiled test `gpu_textured_shade_scales_texel_brightness` fails isolated with **exit 101**: `shade 16 must scale the red texel to ~223, got 255`. |
| **Observed** | Same failure mode on a **fresh** scratch probe built against current client sources. |
| **Observed** | Adapter on container `memory-ref-amd64-view` is **not** Vulkan/lavapipe: wgpu selects **`Gl` / `llvmpipe (LLVM 15.0.6, 128 bits)` / Mesa 22.3.6** even when `WGPU_BACKEND=vulkan` is set. No Vulkan ICD directory in this image. |
| **Observed** | Mesh packing is exact: every textured vertex carries the requested shade (hist size 1, key = input). |
| **Observed** | At shade **16**, red-dominant pixels are **bimodal**: **64544 @ 223** (correct block-1) and **8874 @ 255** (previous block-0). Interior shade **17** is **unimodal 73418 @ 223**. |
| **Observed** | Boundaries 32/48/64/80/96/112 contain both expected and previous-block brightness. The previous block is the majority at 80 and 112. Tested interiors 17/33/49/65 are clean. |
| **Inferred (strong, not direct FS proof)** | Perspective-correct smooth interpolation of the packed shade `f32`, then `u32(in.hsl)` **truncation**, may drop fragments just below integer block boundaries (e.g. 15.999→15→block0). Direct fragment `in.hsl` values were **not** instrumented. |
| **Rejected / not supported** | “Factor path never takes effect” — **contradicted** by the large correct majority at shade 16 and clean interiors. |
| **Not demonstrated** | Any causal link to host memory-campaign changes or shared GPU memory. Failure is client mesh+shader+test-metric behavior under this software GL stack. |

## Environment and provenance

### Container / adapter (this card)

- Container name: `memory-ref-amd64-view` (left running; not stopped/reconfigured).
- Packages: Mesa 22.3.6 GL stack (`libgl1-mesa-dri`, etc.). **No** `mesa-vulkan-drivers` / **no** `/usr/share/vulkan/icd.d` on this image.
- Probe adapter enumeration (`docs/memory/diagnostics/linux-gpu-shade-out/probe-report.txt`):

```
enumerate_count=1
adapter[0] name="llvmpipe (LLVM 15.0.6, 128 bits)" … backend=Gl driver_info="4.5 (Core Profile) Mesa 22.3.6"
selected_like_client … backend=Gl … (same)
```

- `WGPU_BACKEND=vulkan` was set for runs; **actual** backend is still **Gl/llvmpipe**. Do **not** label this card’s evidence as Vulkan or lavapipe.
- Earlier root/full-suite notes that named lavapipe (`docs/memory/linux-reference-followup.md`, `_linux_ref_followup_test_client_mesa_full2.log`) were on a **different desktop image** with Vulkan ICDs. That identity was **not** re-verified here. Symptom text matches (`shade 16 … got 255`) but **backend identity of that older log is not the same as this probe**.

### Source / binary hashes

| Artifact | sha256 / id |
|:---|:---|
| Host HEAD | `23a110326e1383f28806363f01aed97a90902c20` |
| Client submodule HEAD | `451759f2a7df9c57895657d5b8d506172860cee1` |
| `gpu.rs` (scene shader) | `f12964e1ffeeace8195af2cfa656e052da809b2eace1b5894bae6d5d1425e613` |
| `gpu_texture.rs` (test) | `053eaef148d5a15e9bfe5da2d55393717707aef675e30a26aa40a22351513e9f` |
| `world.rs` (mesh pack) | `9e2cc21a2c42c9f9956b7bd7e0a7790f0138a975b66413b41e8ca64a432e54e9` |
| Existing test bin `/target/debug/deps/gpu_texture-3b22fb5e3f1b99ef` | `58fd55ffa98d64b02848091588fde5d404395d23d874a04181edeabeec44d0bc` (mtime 2026-09-06 23:52:15Z) |
| Scratch probe `/target/debug/linux-gpu-shade-probe` | `c9123c06f605fd6680775bc18940448894524cdbfb7ee6821e8581cbbe3bc0ea` (built this card from current sources) |

- Strings in the **existing** test binary include `block_factor` and `0.875` — the brightness-block shader path is present in that binary (not a pre-fix “always full brightness” build).
- Existing binary `.d` paths say `crates/client/tests/gpu_texture.rs` (client-workspace layout at compile time). Probe was built from `/work/vendor/fr-client-rust/crates/client` path dep. **Source content hashes above are current tree**; byte-identical compile inputs for the older test bin vs current tree are **not** proven beyond matching failure + shader symbols.

### Commands (bounded)

Isolated existing test (explicit `SKIP_GPU=0`):

```bash
docker exec --user memref \
  -e SKIP_GPU=0 -e BOT_CPU=0 -e DISPLAY=:99 -e XDG_RUNTIME_DIR=/tmp \
  -e WGPU_BACKEND=vulkan -e HOME=/runtime/home -e XDG_CACHE_HOME=/tmp/xdg-cache \
  memory-ref-amd64-view \
  /target/debug/deps/gpu_texture-3b22fb5e3f1b99ef \
  --exact gpu_textured_shade_scales_texel_brightness --nocapture
# → panic shade 16 … got 255; process exit 101
```

Scratch probe (outside production; empty `[workspace]` crate under diagnostics):

```bash
# inside container PATH includes /usr/local/cargo/bin
cd /work/docs/memory/diagnostics/linux-gpu-shade-probe
CARGO_TARGET_DIR=/target CARGO_HOME=/cargo-home cargo build
SKIP_GPU=0 BOT_CPU=0 DISPLAY=:99 XDG_RUNTIME_DIR=/tmp WGPU_BACKEND=vulkan \
  PROBE_OUT=/work/docs/memory/diagnostics/linux-gpu-shade-out \
  /target/debug/linux-gpu-shade-probe
```

Receipts: `docs/memory/diagnostics/linux-gpu-shade-out/` (`probe-report.txt`, `run.log`, `build.log`, per-shade hist files).

## Code path under test (read-only)

1. **Mesh** (`world.rs`): textured faces pack raw `face_colour_{a,b,c}` into `GpuVertex.abhsl` low 16 bits. Test model `face_render_type=2` uses per-corner a/b/c; test sets all equal.
2. **VS** (`gpu.rs` SCENE_SHADER): `out.hsl = f32(hsl)` with **default (perspective-correct) interpolation** — not `@interpolate(flat)`.
3. **FS**: `let s = u32(in.hsl) & 0x7fu; let block = (s >> 4u) & 3u;` then factors `1.0, 0.875, 0.75, 0.625` and optional half for bit6. **`u32(f32)` truncates toward zero.**
4. **Test metric** (`gpu_texture.rs:476–487`): `max_red` among red-dominant pixels, tolerance ±8. At shade 16, the 8,874 full-bright pixels fail the case despite 64,544 correctly shaded pixels. Other boundaries can have a majority in the wrong block.

## Probe results (summary)

Shader-mirror expectations (CPU): shade 16 → block 1 → factor 0.875 → red ~223.

| shade | expected | max_red | pass (≤8) | red-dom hist (top) | mesh tex shades |
|----:|----:|----:|:---:|:---|:---|
| 0 | 255 | 255 | yes | 255×73418 | {0:6} |
| 15 | 255 | 255 | yes | 255×73418 | {15:6} |
| **16** | **223** | **255** | **no** | **223×64544, 255×8874** | **{16:6}** |
| 17 | 223 | 223 | yes | 223×73418 | {17:6} |
| 31 | 223 | 223 | yes | 223×73418 | {31:6} |
| **32** | **191** | **223** | **no** | **191×64544, 223×8874** | **{32:6}** |
| 33 | 191 | 191 | yes | 191×73418 | {33:6} |
| **48** | **159** | **191** | **no** | **159×51059, 191×22359** | **{48:6}** |
| 49 | 159 | 159 | yes | 159×73418 | {49:6} |
| **64** | **128** | **159** | **no** | **128×64544, 159×8874** | **{64:6}** |
| 65 | 128 | 128 | yes | 128×73418 | {65:6} |
| 80 | 112 | 128 | no | 128×47716, 112×25702 | {80:6} |
| 96 | 96 | 112 | no | 96×51059, 112×22359 | {96:6} |
| 112 | 80 | 96 | no | 96×40083, 80×33335 | {112:6} |

Pattern: **block boundaries and half-bit boundaries** show mixed populations at the expected and previous discrete brightness; **tested +1 interiors (17/33/49/65)** are clean single bins. Interiors 81/97/113 were not sampled. Mesh never shows mixed shades.

## Hypothesis ranking

1. **Primary (best fit to evidence):** Smooth perspective-correct interpolation of constant-but-not-flat shade `f32` + **truncating** `u32(in.hsl)` may yield fragments with `s` one below the integer shade at block edges → previous `block` factor. The wrong block is a minority at shade 16 and a majority at shades 80/112; **`max_red` detects the incorrect brightness in either case**. Direct `in.hsl` still unobserved.
2. **Test interpretation:** The existing maximum-pixel assertion detects incorrect brightness as intended. Replacing it with a mode or percentile could hide errors and is not a proposed correction.
3. **Not primary:** Missing block_factor in binary — disproved (symbols + correct majority pixels).
4. **Not primary:** Mesh packing wrong shade — disproved (exact hist).
5. **Not demonstrated:** Memory-campaign / shared buffer corruption.
6. **Open:** Whether lavapipe Vulkan (other image) has the same bimodal hist; not re-run here (no ICD on viewer image; no full suite).

## Smallest next correction / repro (proposed only — **not implemented**)

No production correction is selected or proven safe by these measurements. A later diagnostic could inspect fragment shade values directly in an isolated shader probe before considering changes.

Rounding before conversion would move quantization thresholds by half a shade unit; flat interpolation could change faces whose corner shades differ. Either would require authorization for a behavior change, varied-corner textured-shading regressions, and comparison with CPU shading. The existing assertion and tolerance should remain intact.

The original raw probe's `INFERENCE_HINT` text overstates a factor-path failure; its numeric histograms support the narrower boundary hypothesis above. Raw output has been retained unchanged.

**Repro for a fix card:** re-run `linux-gpu-shade-probe` and require shade 16 hist unimodal at 223 (or max_red within ±8) under the same Gl/llvmpipe identity; then isolated `gpu_texture` exact test with `SKIP_GPU=0`.

## Explicit non-actions (this card)

- No client submodule edits, no shader/test patches, no tolerance changes.
- No full client/host suites, no live/server.
- Did not stop or reconfigure `memory-ref-amd64-view` or `fr-vault-chroma`.
- No claim that `WGPU_BACKEND=vulkan` selected Vulkan on this image.

## Artifacts

- Report: this file.
- Probe crate: `docs/memory/diagnostics/linux-gpu-shade-probe/` (scratch only).
- Receipts: `docs/memory/diagnostics/linux-gpu-shade-out/` (`probe-report.txt`, `run.log`, `build.log`, `shade-*-red-dom-hist.txt`).
