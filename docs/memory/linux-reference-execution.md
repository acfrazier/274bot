# Linux memory-reference execution (build + separate crate tests)

**Card:** `t_e1c104c1`  
**Branch:** `codex/memory-diagnostics`  
**Scope:** `docker/memory-reference/**` + this report. No `STATE.md`, no Rust app edits, no merges/pushes.  
**Host:** macOS arm64 Mac + Docker Desktop 29.7.2 (`desktop-linux`), QEMU/binfmt **emulated** `linux/amd64`.

This is a **build/test execution record**, not performance acceptance, not live memory/CPU cells, not macOS native GPU proof.

## Source mutability (orch coordination)

Bind-mounted checkout was **not frozen** during this run:

| Marker | git HEAD |
|:---|:---|
| Image build start (approx) | `1f086b0` (post prep `d4945ab` + attribution docs) |
| During host compile / early tests | moved through `606c93b` NavWorld, `63054dc` sampler, … |
| Late tests / release finish | `2d55e0514128ddb59669376ee1b15412e1d1342a` (and still moving on concurrent cards) |

Concurrent cards (NavWorld, scheduling/measurement) edited host/lib + related crates while Linux compile ran. **Initial container results are not frozen-source validation.** Orch note: final settled-source crate retest is required later. Record actual changed files via `git log` / board, not this card.

## Resources (two distinct envelopes)

| Envelope | Setting | Evidence |
|:---|:---|:---|
| **Runtime profile (compose default)** | `MEMORY_REF_CPUS=2.0` `MEMORY_REF_MEM=4g` (+ memswap 4g) | Documented in compose; plan §1 VPS TUI target |
| **Compile/test entitlement used here** | `MEMORY_REF_CPUS=4.0` `MEMORY_REF_MEM=6g` | cgroup inside run: `cpu.max=400000 100000`, `memory.max=6442450944` |
| Docker Desktop VM | NCPU=16, MemTotal≈7.7 GiB | `docker info` after Desktop start |

4c/6g is **compile-only** and must not be confused with the 2c/4g runtime measurement profile. Default 2c/4g was not used for the long cargo runs (risk of OOM under qemu); no endless retry loop.

## Images

| Image | Digest / Id | Arch | Notes |
|:---|:---|:---|:---|
| `274bot-memory-ref:1.98.0-bookworm-amd64` | `sha256:51e9ecf025a9ac89e2ae14be24008827771a25d9792370fa2724c4ae5387d637` | **amd64** | Final rebuild after memref drop-user |
| `274bot-memory-ref:1.98.0-bookworm-desktop-amd64` | `sha256:5158583a5beb3587793f0aad3e56b9b00cf688cbb0176c1071192e1bc7502d56` | **amd64** | Built by orch for viewer (pre-final localhost script fix in tree) |

Base: `rust:1.98.0-bookworm` (`rustc`/`cargo` 1.98.0). Volumes: `274bot-memory-ref-cargo-amd64`, `274bot-memory-ref-target-amd64` (`CARGO_TARGET_DIR=/target`).

In-container `uname -m` → **`x86_64`** (emulated). Release binaries:

- `/target/release/panel-play` — ELF x86-64, ~103 MiB  
- `/target/release/tui-play` — ELF x86-64, ~95 MiB  
- Features: `memory-profile-no-alloc` via `build-test.sh build-release-ref`  
- Exit: **0** (`docs/memory/_linux_ref_build_release2.log`)

## Environment fixes landed (this card)

1. **Non-root exec user `memref` (uid 1000)**  
   Root bypassed DAC tests that `chmod 0500` tempdirs (`panel`/`tui`/`vault` credential upsert failures).  
   Entrypoint still starts as **root** to bind relay `:80`, starts socat, chowns cargo/target, then **`setpriv` → memref** for the user command.  
   Override: `MEMORY_REF_DROP_USER=root` (debug only).

2. **`build-test.sh` separation**  
   - `test-host`: `cargo test --workspace --exclude client`  
   - `test-host-memory`: `-p host-play -p panel -p tui --features memory-profile-no-alloc`  
   - `test-client`: separate `vendor/fr-client-rust` manifest  
   - `test-unit` = host + host-memory + client  

   Cargo metadata still lists path-dep `client` among workspace packages even though root `members` omit it; `--exclude client` keeps host suite honest.

3. **Desktop `x11vnc`** (orch inline + included here): use `-localhost` (not `-localhost false`, which crashes as unrecognized). Websockify still targets `127.0.0.1:5900`.

## Commands run (representative)

```bash
# Daemon was absent → started installed Docker Desktop; check-daemon → 0
./docker/memory-reference/scripts/host-run.sh static-validate   # exit 0
export MEMORY_REF_ARCH=amd64 MEMORY_REF_PLATFORM=linux/amd64
./docker/memory-reference/scripts/host-run.sh build             # exit 0
# after Dockerfile/entrypoint fixes: rebuild again → digest 51e9ecf0…

export MEMORY_REF_CPUS=4.0 MEMORY_REF_MEM=6g
./docker/memory-reference/scripts/host-run.sh run -- \
  /opt/memory-reference/scripts/build-test.sh smoke-paths       # exit 0; platform=x86_64
# Subsequent cargo used bind-mounted scripts:
#   /work/docker/memory-reference/scripts/build-test.sh …
```

## Test results (SKIP_GPU=1 unless noted)

### Host workspace (exclude client) — non-root

| Package / suite | Exit | Notes |
|:---|:---:|:---|
| api (+ integration tests) | 0 | |
| e2e unit | 0 | ignored live tests only (expected) |
| host lib | 0 | 165 passed, 1 ignored (GPU path) |
| host-play | 0 | 114+ unit; ignored live |
| nav | 0 | |
| panel | 0 | **371 passed** after memref drop (was DAC fail as root) |
| scenario | 0 | |
| tui | 0 | **87 passed** after memref |
| vault | 0 | **14 passed** after memref |
| script lib | 0 | 38 passed |
| script `catalog_run` | **101** | Needs host `RS2B0T` path (`…/rs2b0t/src/bot/scripts/index.ts`); not mounted. **Env readiness**, not product defect. |
| Full `test-host` one-shot | **101** | Stops at `script` catalog; all prior packages green under non-root |

Evidence logs (unique paths, not overwritten on restart):

- `docs/memory/_linux_ref_test_host.log` — first attempt (tool timeout mid-compile; not OOM)  
- `docs/memory/_linux_ref_test_host2.log` — root run; failed client GPU overlay under SKIP_GPU  
- `docs/memory/_linux_ref_test_host_excl.log` — root; panel DAC fail  
- `docs/memory/_linux_ref_test_dac_nonroot.log` — panel/tui/vault **pass** as memref  
- `docs/memory/_linux_ref_test_host_full.log` — full host excl client; script catalog fail  
- `docs/memory/_linux_ref_test_by_pkg.log` / `_remaining.log` / `_post_script.log`

### Host memory-profile-no-alloc

| Command | Exit |
|:---|:---:|
| `build-test.sh test-host-memory` (host-play + panel + tui features) | **0** |

Log: `docs/memory/_linux_ref_test_host_memory.log` (e.g. panel 377 passed under feature set).

### Client (separate manifest)

| Run | Exit | Result |
|:---|:---:|:---|
| `test-client` / `SKIP_GPU=1` | **101** | `viewport_overlay_moves_and_clears_without_chrome_redraw` panics: `GPU required … "SKIP_GPU=1"` (expect, not ignore) |
| Same test, `SKIP_GPU` unset | **101** | No wgpu adapter under emulated container: `no GPU context` / noop not compiled; falls back message then still fails GpuBackend expect |

**Client GPU overlay regression is unavailable** in this environment. Not ignored to fake pass. **Not** native Linux GPU qualification. Software Mesa in desktop image was not used to claim functional GPU pass (viewer left for operator; no adapter in default ref image either).

Other client lib tests in the failing package run: 68 passed / 1 failed under SKIP_GPU before stop.

Logs: `_linux_ref_test_client.log`, `_linux_ref_test_client_noskipgpu.log`.

### Live / measurement

**Not run** (card forbid): no LIVE e2e/host-play, no server start/reset, no 2c/4g TUI budget acceptance, no RSS/CPU claims.

### Optional desktop / noVNC

Owned/started by orch for operator visibility while compile continued:

- Container **`memory-ref-amd64-view`** (`8851e3b40365`), image desktop `5158583a…`  
- Publish **`127.0.0.1:6080`**, relay OFF (viewer only), 2c/4g  
- Orch CUA verified browser → X display; x11vnc localhost fix applied  
- **Left running** per operator request (this card does not stop it)  
- `fr-vault-chroma` pre-existed; **not** cleaned up  

This implementer did **not** start a second viewer on 6080.

## First-process timeout evidence

Initial `test-host` under tool timeout (~420s) left a partial log (`_linux_ref_test_host.log`) mid-`Compiling` — **not** OOMKilled evidence. No surviving `memory-reference-ref-run-*` container after timeout. Restarts used **new unique log paths**.

## Missing proofs / follow-ups for orch

1. **Settled-source retest** after concurrent host edits stop (mutable bind-mount).  
2. **script catalog_run**: mount or set `RS2B0T` to a real catalog inside the container if that test is required green.  
3. **Client GPU overlay**: needs real GPU adapter or product change (out of this card); SKIP_GPU/CI skip is not proof.  
4. **2c/4g runtime cells** and live engine relay qualification: separate stage.  
5. Rebuild **desktop** image so baked `start-desktop.sh` matches `-localhost` fix (viewer already patched at runtime).  
6. Emulated amd64 ≠ bare-metal Linux timing/CPU class.

## Containers left running (intentional)

- `memory-ref-amd64-view` — operator noVNC  
- `fr-vault-chroma` — pre-existing  

No compile `ref-run` containers left. Images/caches retained.

## Non-claims

- No performance PASS/FAIL, no budget acceptance.  
- No macOS native GPU or emulated-timing acceptance.  
- No application visual proof from this implementer (viewer verified by orch).  
- No secrets printed; compose config secrets not dumped.
