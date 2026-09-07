# Linux memory-reference follow-up (catalog mount + desktop rebuild + Mesa software GPU)

**Card:** `t_39990f75`  
**Branch:** `codex/memory-diagnostics`  
**Scope:** `docker/memory-reference/**` + this report only. No Rust app/test weakening, no `STATE.md`, no metric-tool edits, no merges/pushes.  
**Host:** macOS arm64 + Docker Desktop 29.7.2; QEMU/binfmt **emulated** `linux/amd64`.

Follow-up to `docs/memory/linux-reference-execution.md` / commit `8b60b2b` (card `t_e1c104c1`).

This is an **environment readiness** record: catalog integration under non-root, desktop image bake, and one bounded Mesa software-rendering client attempt. **Not** performance acceptance, live memory/CPU cells, or native Linux GPU proof.

## Settled sources

| Marker | Value |
|:---|:---|
| Reviewed Rust baseline | `6345fbc` (`fix(memory): true per-slot interval endpoints and lag labels`) |
| Crates/client commits since `6345fbc` during this card | **0** (concurrent HEAD moves are Python/docs metrics only) |
| Client submodule | `451759f2a7df9c57895657d5b8d506172860cee1` |
| Catalog checkout `RS2B0T` host path | `/Users/acfrazier/experiments/rs2b0t` @ `100adccc037d9f6898080e1cad58fcfc43364775` |
| Pre marker | `docs/memory/_linux_ref_followup_pre.txt` |
| Image used for host/memory suites | `274bot-memory-ref:1.98.0-bookworm-amd64` @ `sha256:51e9ecf025a9ac89e2ae14be24008827771a25d9792370fa2724c4ae5387d637` (unchanged from prior card) |

Bind-mounted checkout may still receive concurrent **docs/Python** commits; crate tree relative to `6345fbc` stayed empty for this run.

## Environment changes

### 1) Catalog mount (host-run)

- Auto-detect host catalog at `~/experiments/rs2b0t` (or `MEMORY_REF_RS2B0T_HOST` / `$RS2B0T`).
- Bind-mount **read-only at the matching container path** via Bash array `"${catalog_args[@]}"` (no unquoted `$(...)`).
- **Default does not export container `$RS2B0T`.** Several script unit tests call `rs2b0t_root_at` without IsolatedEnv and prefer process `$RS2B0T` over a scratch path file; exporting the env broke `js_library_registers_rs2b0t_cards_without_isolates`.
- Catalog integration uses the operator `~/.274bot/rs2b0t-path` (RO mount → `/runtime/home/.274bot/rs2b0t-path`) pointing at the same host path, which is now visible inside the container.
- `MEMORY_REF_EXPORT_RS2B0T=1` optionally adds `-e RS2B0T=…` for live/panel cells.
- `compose_run_env` unsets host `RS2B0T` before `docker compose` unless export is requested (compose.yaml forwards `RS2B0T`).

### 2) Desktop image rebuild

| Image | Id / digest | Arch | Notes |
|:---|:---|:---|:---|
| `274bot-memory-ref:1.98.0-bookworm-desktop-amd64` | `sha256:ad9c194dcc918eb188f9b6bc9b0a343e435c5f0919cc87a33faea7a48332f67d` | **amd64** | Rebuild on this card |
| Prior desktop (viewer still on this) | `sha256:5158583a5beb3587793f0aad3e56b9b00cf688cbb0176c1071192e1bc7502d56` | amd64 | **Left running** as `memory-ref-amd64-view` |

Baked `start-desktop.sh` confirms `x11vnc … -localhost` (not `-localhost false`).  
Desktop packages added for software GPU attempt: `mesa-vulkan-drivers`, `vulkan-tools`, `libegl1`, `libgl1`, `libglx-mesa0` (plus existing `libgl1-mesa-dri` / `mesa-utils`).

### 3) Mesa software GPU (bounded, ephemeral)

Client `GpuBackend` uses `wgpu::Instance::new_without_display_handle()` + `request_adapter` (Vulkan preferred via `WGPU_BACKEND=vulkan`). No app patches.

Ephemeral `ref-desktop` container **without** host `6080` publish; viewer on `127.0.0.1:6080` untouched.

| Attempt | Result |
|:---|:---|
| Overlay unit only (`viewport_overlay_…`) | **PASS** under lavapipe (`WGPU_BACKEND=vulkan`, `VK_ICD_FILENAMES=…/lvp_icd.x86_64.json`, `LIBGL_ALWAYS_SOFTWARE=1`, `DISPLAY=:99`, non-root memref) |
| Full client workspace `SKIP_GPU=0` + same Mesa/lavapipe stack | **FAIL** — not a full-suite pass. After client lib green (incl. overlay **ok**, 69 passed), `gpu_texture` test `gpu_textured_shade_scales_texel_brightness` panicked at `vendor/fr-client-rust/crates/client/tests/gpu_texture.rs:485`: **shade 16 must scale the red texel to ~223, got 255**. Backend: wgpu Vulkan → Mesa **lavapipe** software ICD. **Cause undiagnosed** (not labeled precision noise). No tolerance/app change; failure preserved. |
| Full client with default `SKIP_GPU=1` (base ref image) | **FAIL** honest: overlay requires GPU, reports `SKIP_GPU=1` (68 pass / 1 fail) |

**Software Mesa is an optional functional attempt only.** Overlay can pass under lavapipe; full client-manifest under the same stack did **not** pass. This is **not** native Linux GPU performance or acceptance evidence. No further Mesa retries on this card.

Note: `build-test.sh test-client` uses `SKIP_GPU="${SKIP_GPU:-1}"` — unset becomes 1; use explicit `SKIP_GPU=0` for GPU attempts.

## Commands / exits (representative)

```bash
export MEMORY_REF_ARCH=amd64 MEMORY_REF_PLATFORM=linux/amd64
export MEMORY_REF_CPUS=4.0 MEMORY_REF_MEM=6g   # compile envelope; not 2c/4g runtime claim

./docker/memory-reference/scripts/host-run.sh build-desktop
# EXIT 0 → desktop sha256:ad9c194d…

./docker/memory-reference/scripts/host-run.sh run -- \
  /work/docker/memory-reference/scripts/build-test.sh smoke-paths
# EXIT 0; uid=1000 memref; RS2B0T unset; mount verified separately

./docker/memory-reference/scripts/host-run.sh run -- \
  /work/docker/memory-reference/scripts/build-test.sh test-host
# EXIT 0 — includes script catalog_run / rs2b0t_registry

./docker/memory-reference/scripts/host-run.sh run -- \
  /work/docker/memory-reference/scripts/build-test.sh test-host-memory
# EXIT 0

./docker/memory-reference/scripts/host-run.sh run -- \
  /work/docker/memory-reference/scripts/build-test.sh test-client
# EXIT 101 — SKIP_GPU=1 overlay fail (expected)

# Ephemeral desktop Mesa (no --service-ports / no host 6080):
docker compose -f docker/memory-reference/compose.yaml --profile desktop run --rm --no-deps \
  -e MEMORY_REF_DESKTOP=1 -e MEMORY_REF_RELAY=0 -e SKIP_GPU=0 \
  -e LIBGL_ALWAYS_SOFTWARE=1 -e WGPU_BACKEND=vulkan \
  -e VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json \
  --name memory-ref-amd64-mesa-… ref-desktop …
```

Non-root: logs show `memory-ref: dropping to user memref (uid=1000)`.

## Test summary

| Suite | Exit | Log |
|:---|:---:|:---|
| smoke-paths + mount check | 0 | `_linux_ref_followup_smoke2.log`, `_linux_ref_followup_mountcheck.log` |
| `test-host` (workspace exclude client) | **0** | `_linux_ref_followup_test_host2.log` (`catalog_cards_except_dim_set_remap … ok`) |
| `test-host-memory` | **0** | `_linux_ref_followup_test_host_memory.log` |
| `test-client` SKIP_GPU=1 | **101** | `_linux_ref_followup_test_client_skipgpu.log` |
| Mesa overlay only | **0** | `_linux_ref_followup_test_client_mesa_vulkan.log` |
| Mesa full client SKIP_GPU=0 | **101** | `_linux_ref_followup_test_client_mesa_full2.log` (texture shade) |
| desktop rebuild | **0** | `_linux_ref_followup_build_desktop.log` |

Earlier false start with `$RS2B0T` exported: `_linux_ref_followup_test_host.log` EXIT 101 (`js_library_registers…` env pollution) — fixed by mount-without-export; log retained.

## Left running (intentional)

- `memory-ref-amd64-view` — operator noVNC on **old** desktop digest `5158583a…` (not restarted onto new image)
- `fr-vault-chroma` — pre-existing; not touched
- No ephemeral mesa containers left; no compile `ref-run` left

## Non-claims / still pending

- No 2c/4g TUI budget or RSS/CPU acceptance  
- No live gameplay / engine start-reset  
- No native Linux GPU or bare-metal timing  
- Emulated amd64 ≠ reference hardware  
- Overhead and reference-hardware proof remain pending for orch  
- Mesa full-client receipt is **failed** (`gpu_texture.rs:485` shade16 ~223 vs 255 on lavapipe); undiagnosed; not ignored and not a suite pass  

## Files changed (this card)

- `docker/memory-reference/scripts/host-run.sh` — catalog mount array, export opt-in  
- `docker/memory-reference/scripts/build-test.sh` — smoke-paths RS2B0T/uid fields  
- `docker/memory-reference/compose.yaml` — catalog mount comment  
- `docker/memory-reference/Dockerfile` — desktop Mesa/Vulkan packages  
- `docs/memory/linux-reference-followup.md` — this report  
- Unique logs under `docs/memory/_linux_ref_followup_*.log` (untracked evidence; not required in git)
