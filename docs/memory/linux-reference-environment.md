# Linux memory-reference environment (preparation)

**Card:** `t_e0ea755d`  
**Scope:** `docker/memory-reference/**` + this document only.  
**Status:** preparation — Docker CLI present; **desktop-linux daemon socket was absent** at prep time. No image pull/build, no Cargo, no live processes, no Docker Desktop start.

This is plan §1 support for a **Linux** validation path (VPS TUI profile: **2 CPU / 4 GiB**, no GPU), not a performance acceptance result.

## Purpose

Provide a reproducible Linux container for:

1. **Build/test** of the 274bot workspace and the vendored client (separate manifests).
2. Later **2c/4GiB TUI** resource-limited runs against the **existing local engine** (no new paid infra, no server reset/start by this card).
3. Optional **desktop window** (Xvfb + Openbox + x11vnc + noVNC) so an operator can see panel/TUI work in a browser on **host loopback only**. Off by default.

## Provenance pins (explicit)

| Item | Pin / source |
|:---|:---|
| Host Rust | `rust-toolchain.toml` → **1.98.0** (+ rustfmt, clippy) |
| Container base | official **`rust:1.98.0-bookworm`** (docker-library; tags for **amd64** and **arm64v8**) |
| Native build deps | Same set as [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml): `pkg-config`, `libasound2-dev`, `libwayland-dev`, `libxkbcommon-dev`, `libx11-dev`, `libxcursor-dev`, `libxi-dev`, `libxrandr-dev`, plus `build-essential`, `cmake`, `libclang-dev`, `libssl-dev` for Linux link |
| CI behavior | GH Actions uses `SKIP_GPU=1`; never `LIVE=1`. This image defaults the same for unit tests |
| Cargo.lock | Bind-mounted from the checkout (host + `vendor/fr-client-rust/Cargo.lock`); not rewritten by the image |
| Client submodule | Path dep `vendor/fr-client-rust`; tested via **second** cargo manifest |
| Desktop extras (optional image) | Debian bookworm packages: `xvfb`, `openbox`, `x11vnc`, `novnc`, `websockify`, `xterm`, `dbus-x11`, `libgl1-mesa-dri` |

Upstream checks at preparation (docs only, not pulled): docker-library `rust` lists `1.98.0-bookworm` / `1.98.0-slim-bookworm` for amd64 and arm64; noVNC access pattern is websockify `--web <novnc root> <listen> <vnc-host:port>` with browser on `/vnc.html`.

## Layout

```
docker/memory-reference/
  Dockerfile                 # base + optional WITH_DESKTOP=1
  compose.yaml               # ref (default) + ref-desktop (profile desktop)
  .dockerignore
  scripts/
    entrypoint.sh            # relay + optional desktop, then exec
    relay-host-engine.sh     # 127.0.0.1:43594/:80 → host engine
    build-test.sh            # fmt/clippy/test/build/live helpers
    host-run.sh              # Mac/host wrapper + static-validate
  desktop/
    start-desktop.sh         # Xvfb/Openbox/x11vnc/noVNC
docs/memory/linux-reference-environment.md   # this file
```

## Architectures and caches

| Variable | Role |
|:---|:---|
| `MEMORY_REF_ARCH=amd64\|arm64` | Selects image tag suffix and **volume names** |
| `MEMORY_REF_PLATFORM=linux/amd64\|linux/arm64` | Explicit `docker buildx` / compose `platform` |

Named volumes (never share macOS `target/` or cross-arch):

- `274bot-memory-ref-cargo-amd64` / `…-arm64` → `/cargo-home`
- `274bot-memory-ref-target-amd64` / `…-arm64` → `/target` (`CARGO_TARGET_DIR`)

On this **arm64 Mac**, the plan’s Linux x64 reference is **`MEMORY_REF_ARCH=amd64`** (QEMU/binfmt emulation under Docker Desktop). Native `linux/arm64` is supported for Apple-silicon-like Linux hosts; do not mix volume names across arches.

## Network: existing local engine (no new server)

Documented local engine (FIRST-START / CONTRIBUTING):

- Game TCP **`127.0.0.1:43594`**
- HTTP **`/crc` + jags on `:80`**
- Pack cache `$ENGINE_DIR/data/pack/client` (default `$HOME/experiments/Server/engine`)
- RSA: stock Java default, or `$ENGINE_DIR/data/config/private.pem`

### Loopback RSA constraint

`host_play::validate_play_host` / TUI startup **reject non-loopback `--host` while `BOT_TARGET=local`**.  
`host.docker.internal` is therefore **not** a legal app `--host` for local.

**Documented host endpoint for the container network:** `host.docker.internal` (Docker Desktop Mac; compose also sets `extra_hosts: host.docker.internal:host-gateway` for Linux engines).

**App still uses `127.0.0.1`.** Entrypoint starts **socat relays**:

- `127.0.0.1:43594` → `${MEMORY_REF_HOST_ENGINE}:43594`
- `127.0.0.1:80` → `${MEMORY_REF_HOST_ENGINE}:80`

Disable with `MEMORY_REF_RELAY=0` only if the engine is truly on container loopback.

This card does **not** start, reset, or reconfigure the engine. Credentials, vault passphrase, nav pack, and caches stay **runtime bind-mounts / env** — never image layers.

| Runtime input | Mount / env |
|:---|:---|
| Operator `~/.274bot` (vault, navpack, ui) | `${HOME}/.274bot` → `/runtime/home/.274bot:ro` (override path if needed) |
| Engine tree | `ENGINE_DIR` → `/runtime/engine:ro` |
| Vault passphrase | `BOT_VAULT_PASS` at `compose run` time only |
| Optional catalog | `RS2B0T` path (must be visible inside container if used) |

## What is in the default image vs desktop image

| | `ref` (default) | `ref-desktop` (`--profile desktop`) |
|:---|:---|:---|
| Rust 1.98.0 + CI native deps + socat + python3 | yes | yes |
| Xvfb / Openbox / x11vnc / noVNC / websockify | no | yes (`WITH_DESKTOP=1`) |
| Compose CPU/mem | 2.0 CPU / 4g (overridable) | same defaults; **helpers extra** |
| Published ports | none | **`127.0.0.1:6080:6080` only** |
| Default `BOT_CPU` | unset | `1` (honest software path for panel) |

### GPU / software honesty

- **Docker Desktop on this Mac does not establish native Linux GPU performance.**
- Unit tests: `SKIP_GPU=1` (same as GH). **Ignored GPU tests are not proof.**
- Panel headed path may use wgpu; without a real Linux GPU/adapter, treat GPU cells as **unavailable** or use **`BOT_CPU=1` (CpuPix3D)** for software-renderer evidence.
- Desktop stack is **software X (Xvfb)** + optional Mesa DRI packages for GL clients — still not a passthrough claim.
- Preserve app CPU fallback and **real PTY** for TUI reference cells (`run_diagnostic.py` without `--headless`). Do not substitute fake headless interaction for TUI budget proof.

### Desktop accounting

When desktop is enabled, sample **Xvfb / openbox / x11vnc / websockify** CPU and RSS **separately** from `tui-play` / `panel-play`. Do not fold helper cost into the 2c/4GiB **application** budget without labeling it.

## Commands (for orch after diagnostic isolation)

All paths from repo root. **Do not run build/pull while native attribution is in flight** on the shared machine unless orch confirms isolation.

### 0) Prep checks (safe now)

```bash
# Daemon presence only — does not start Docker Desktop
./docker/memory-reference/scripts/host-run.sh check-daemon
# Structure / pins / bash -n (no daemon required)
./docker/memory-reference/scripts/host-run.sh static-validate
```

### 1) Start Docker Desktop (operator/orch)

Ensure context `desktop-linux` socket exists:

`~/Library/...` → `~/.docker/run/docker.sock` (or current Desktop path).

Re-run `check-daemon` until exit 0.

### 2) Build images

```bash
# Linux x64 reference (emulated on arm64 Mac)
export MEMORY_REF_ARCH=amd64 MEMORY_REF_PLATFORM=linux/amd64
./docker/memory-reference/scripts/host-run.sh build

# Optional desktop variant
./docker/memory-reference/scripts/host-run.sh build-desktop

# Optional native arm64 Linux image (separate volumes)
export MEMORY_REF_ARCH=arm64 MEMORY_REF_PLATFORM=linux/arm64
./docker/memory-reference/scripts/host-run.sh build
```

Equivalent raw form:

```bash
docker buildx build --load --platform linux/amd64 \
  -f docker/memory-reference/Dockerfile \
  -t 274bot-memory-ref:1.98.0-bookworm-amd64 .
docker buildx build --load --platform linux/amd64 \
  --build-arg WITH_DESKTOP=1 \
  -f docker/memory-reference/Dockerfile \
  -t 274bot-memory-ref:1.98.0-bookworm-desktop-amd64 .
```

### 3) Host build/test inside container (not live)

```bash
export MEMORY_REF_ARCH=amd64 MEMORY_REF_PLATFORM=linux/amd64
export ENGINE_DIR="${ENGINE_DIR:-$HOME/experiments/Server/engine}"

./docker/memory-reference/scripts/host-run.sh run -- \
  /opt/memory-reference/scripts/build-test.sh smoke-paths

./docker/memory-reference/scripts/host-run.sh run -- \
  /opt/memory-reference/scripts/build-test.sh test-unit

./docker/memory-reference/scripts/host-run.sh run -- \
  /opt/memory-reference/scripts/build-test.sh build-release-ref
```

Manifests (mirrors CONTRIBUTING local CI):

| Step | Command (in container) |
|:---|:---|
| fmt | `build-test.sh fmt` → host + `vendor/fr-client-rust` |
| clippy | `build-test.sh clippy` |
| host tests | `cargo test --workspace` with `SKIP_GPU=1` |
| client tests | `cargo test --manifest-path vendor/fr-client-rust/Cargo.toml --workspace` |
| release frontends | `cargo build --release -p panel --bin panel-play -p tui --bin tui-play --features memory-profile-no-alloc` |

**Do not** count `cargo test -p host -- --ignored` GPU tests or any skipped adapter test as Linux GPU proof.

### 4) Live qualification (separate stage; engine already running on host)

Prereq: local engine accepting `43594` and HTTP `:80` on the **Mac/host**. Relay maps container loopback → `host.docker.internal`.

```bash
# Passphrase and any needed paths at runtime only
export BOT_VAULT_PASS=bot   # example; operator secret — not committed
./docker/memory-reference/scripts/host-run.sh run -- \
  /opt/memory-reference/scripts/build-test.sh live-e2e
./docker/memory-reference/scripts/host-run.sh run -- \
  /opt/memory-reference/scripts/build-test.sh live-host-play
```

TUI limited cell (example shape — exact diagnostic flags stay with `docs/memory/run_diagnostic.py`):

```bash
./docker/memory-reference/scripts/host-run.sh run -- bash -lc '
  /opt/memory-reference/scripts/build-test.sh build-release-tui
  # Real PTY path for TUI reference — not --headless
  python3 docs/memory/run_diagnostic.py tui 1 active --sustain --no-diagnostics \
    --binary /target/release/tui-play --warmup 30 --observe 120
'
```

Resource envelope: compose `cpus: 2.0`, `mem_limit: 4g` on `ref`. Override with `MEMORY_REF_CPUS` / `MEMORY_REF_MEM` if needed. Record OS, entitlement, arch (emulated vs native), and whether desktop helpers were running.

### 5) Optional noVNC desktop

```bash
export MEMORY_REF_ARCH=amd64 MEMORY_REF_PLATFORM=linux/amd64
./docker/memory-reference/scripts/host-run.sh shell-desktop
# On host browser (loopback only):
#   http://127.0.0.1:6080/vnc.html
# Inside container: DISPLAY=:99; panel-play with BOT_CPU=1 for software path
```

## Launch paths reused (not invented)

From FIRST-START / CONTRIBUTING / AGENTS:

- `cargo run --release -p panel --bin panel-play`
- `cargo run --release -p tui --bin tui-play` (real terminal; raster off)
- `BOT_CPU=1` → CpuPix3D
- `BOT_TARGET=local` (default); prod is out of scope for this low-end Linux TUI cell
- `scripts/fetch-cache.sh` for pack cache location hints (no assets in git)
- Live: `LIVE=1 cargo test -p e2e -- --ignored`; `LIVE=1 cargo test -p host-play -- --ignored`
- Memory cells: `python3 docs/memory/run_diagnostic.py` + `qualify_control.py`

## Explicit non-goals / non-claims

- No performance acceptance; no budget PASS/FAIL from this prep card.
- No GPU passthrough; no “Linux iGPU panel” proof from Docker Desktop Mac.
- No secrets in image; no copy of operator `~/.274bot` into layers.
- No destructive cleanup, remote push, system setting changes, or `chown` of the host repo.
- No edit of `STATE.md`, application/client source, or `reference_metrics.py` on this card.
- GUI profile must not block build-only usage (default image has no desktop packages).

## Preparation verification (this card)

| Check | Result at prep |
|:---|:---|
| Branch | `codex/memory-diagnostics` |
| Docker CLI | 29.7.2 |
| Daemon socket `~/.docker/run/docker.sock` | **absent** |
| `host-run.sh static-validate` | run after commit scripts executable |
| Image pull/build | **not executed** (brief forbid) |
| Cargo in container | **not executed** |

## Next execution steps for orch (after native attribution isolation)

1. Confirm no concurrent native profiling/builds on the measurement machine (or use this container only for compile isolation).
2. Start Docker Desktop; `./docker/memory-reference/scripts/host-run.sh check-daemon`.
3. `MEMORY_REF_ARCH=amd64` `build` then `build-test.sh test-unit` and `build-release-ref`.
4. With local engine already up, smoke relay (`smoke-paths` + TCP to 127.0.0.1:43594 inside container).
5. Only then schedule 2c/4GiB TUI diagnostic cells; keep desktop profile off for clean budgets.
6. Optional: `build-desktop` + loopback noVNC for operator visibility; attribute helper RSS/CPU separately.
7. Record emulated-amd64 vs future bare-metal Linux as distinct evidence classes.

## Related plan text

[performance-finish-plan.md](performance-finish-plan.md) §1 — VPS TUI 2 CPU / 4 GiB Linux; § “How to test lower-end hardware using this machine” (quota/throttling ≠ older CPU; no paid infra; Mac-only is not target-device validation).
