# GPU main-modal restoration report

**Task:** `t_1d185f15`  
**Checkout:** host `codex/memory-diagnostics`; client submodule `codex/windows-native-parking` (dirty local only — root owns commit/gitlink).  
**Scope:** client GPU chrome finish/overlay compare + unit regression; this report. No git mutations, no Windows/live reruns.

## Baseline (pre-patch)

| Check | Result |
|:---|:---|
| Host branch | `codex/memory-diagnostics` (not main) |
| Client branch | `codex/windows-native-parking` (not main) |
| `gpu_main_modal_rect_is_opaque_over_the_scene` | **FAIL** black `0` vs `0x336699` |
| Provenance | Parent `cc81f6e` PASS; first bad `f2504be` lazy chrome |

## Failed branch (instrumented before production fix)

One-frame `finish` probe on the synthetic main-modal path (`BOT_MODAL_DIAG`, removed after):

| Frame | `scene_ready` | `pending` | `overlay_changed` | `cov_nz` | `draw_area` center |
|:---|:---|:---|:---|:---|:---|
| 1 (no modal) | **false** | true | false | 0 | 0 |
| 2 (main modal) | **false** | false | **false** | **171008** | **0x336699** |

Mechanism:

1. Empty mesh → `render_scene` sets `scene_ready = false` (vertices empty).
2. First chrome upload with `scene_ready == false` seals the scene window as **opaque alpha 255** (`fill_draw_area_rgba` else-branch), not coverage-keyed hole.
3. Second frame: CPU main-modal + full coverage + SEA in `draw_area` are correct; HUD atlas dirty does **not** include `main_modal_id`.
4. `overlay_changed` was gated `scene_ready && chrome_uploaded && scene_overlay_changed(...)`. With `scene_ready` false the compare never ran, so no re-upload → black sealed chrome stayed.

Unit `viewport_overlay_moves_and_clears_without_chrome_redraw` still passed because it forces `scene_ready = true` and mutates coverage manually.

This is the same hole class as provenance/`windows-native-additional-failures.md`, refined: the miss is not missing coverage marks on the full path; it is **lazy compare/alpha semantics mismatched to sealed empty-mesh chrome**.

## Fix (production)

File: `vendor/fr-client-rust/crates/client/src/render/backend/gpu.rs`

1. **`scene_overlay_changed`** now takes `scene_ready` and uses the **same alpha rule as `fill_draw_area_rgba`**: coverage while ready, else sealed opaque `255`. RGB under zero coverage remains immaterial; sealed RGB changes invalidate.
2. **`finish`**: `overlay_changed = chrome_uploaded && scene_overlay_changed(..., scene_ready)` — no longer short-circuits the entire compare on `scene_ready` alone.
3. **No** unconditional every-frame atlas rebuild; no `main_modal_id` force-dirty. Lazy path restored for sealed and hole cases.
4. **Unchanged:** `scene_state==1` freeze / minimap hold punch path; CPU backend; shade shader.

Unit regression: `sealed_scene_window_uploads_modal_rgb_without_chrome_redraw` (empty `scene_ready`, modal RGB without HUD dirty).

## Verification (this machine, release)

```text
cargo test --release -p client --test iface_model -- --nocapture
→ 9 passed (includes gpu_main_modal_rect_is_opaque_over_the_scene,
   gpu_ship_journey_paints_the_title_over_the_scene,
   gpu_ship_journey_stays_over_a_frozen_scene)

cargo test --release -p client --lib render::backend::gpu::tests -- --nocapture
→ 6 passed (incl. sealed + viewport overlay + freeze helpers)

cargo test --release -p client --test gpu_texture \
  gpu_textured_shade_scales_texel_brightness -- --exact --nocapture
→ 1 passed (Mac; shade failure left untouched — no shader redesign)
```

Exact asserts unchanged (no tolerances).

## Unresolved / out of scope

| Item | Status |
|:---|:---|
| Windows native re-run of modal / ship_journey | **Root owns** (not authorized here) |
| Client commit + host gitlink | **Root owns** (no git mutations) |
| `crc_unreachable_sets_error_loading_in_bounded_time` Windows timing | Unchanged (separate card class) |
| Shade-16 Windows/Linux backend variance | Unchanged; Mac exact still passes |
| Live game / process actions | Not run |

## Diff footprint

```text
crates/client/src/render/backend/gpu.rs | ~75 insertions / ~11 deletions
docs/memory/modal-restoration-report.md | this file (host tree)
```
