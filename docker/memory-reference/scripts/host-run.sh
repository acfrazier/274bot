#!/usr/bin/env bash
# Host-side wrapper (Mac or Linux) for the memory-reference container.
# Static-safe: does not start Docker Desktop, pull, or build unless asked.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REF_DIR="$(cd "$HERE/.." && pwd)"
REPO_ROOT="$(cd "$REF_DIR/../.." && pwd)"

ARCH_DEFAULT=amd64
# On an arm64 Mac the common *emulated* Linux x64 reference is amd64;
# native Linux arm64 is available via MEMORY_REF_ARCH=arm64.
MEMORY_REF_ARCH="${MEMORY_REF_ARCH:-$ARCH_DEFAULT}"
case "$MEMORY_REF_ARCH" in
  amd64|x86_64)
    MEMORY_REF_ARCH=amd64
    MEMORY_REF_PLATFORM="${MEMORY_REF_PLATFORM:-linux/amd64}"
    ;;
  arm64|aarch64)
    MEMORY_REF_ARCH=arm64
    MEMORY_REF_PLATFORM="${MEMORY_REF_PLATFORM:-linux/arm64}"
    ;;
  *)
    echo "host-run: MEMORY_REF_ARCH must be amd64 or arm64 (got $MEMORY_REF_ARCH)" >&2
    exit 2
    ;;
esac
export MEMORY_REF_ARCH MEMORY_REF_PLATFORM
export MEMORY_REF_IMAGE="${MEMORY_REF_IMAGE:-274bot-memory-ref:1.98.0-bookworm-${MEMORY_REF_ARCH}}"
export MEMORY_REF_DESKTOP_IMAGE="${MEMORY_REF_DESKTOP_IMAGE:-274bot-memory-ref:1.98.0-bookworm-desktop-${MEMORY_REF_ARCH}}"

COMPOSE=(docker compose -f "$REF_DIR/compose.yaml")

usage() {
  cat <<EOF
usage: host-run.sh <command> [args...]

commands:
  check-daemon     exit 0 if Docker daemon responds; else 1 (no start attempt)
  config           docker compose config (needs daemon for full resolve)
  static-validate  local file/structure checks (no daemon)
  build            build image for MEMORY_REF_ARCH/PLATFORM (needs daemon)
  build-desktop    build WITH_DESKTOP=1 image
  shell            interactive shell in ref service (2c/4GiB)
  shell-desktop    interactive shell with noVNC on 127.0.0.1:6080
  run              docker compose run --rm ref -- <cmd...>
  tui-limits       print the 2c/4GiB resource stanza in use

env:
  MEMORY_REF_ARCH=amd64|arm64     default amd64 (Linux x64 reference)
  MEMORY_REF_PLATFORM=linux/...   default derived from arch
  MEMORY_REF_CPUS=2.0 MEMORY_REF_MEM=4g
  ENGINE_DIR  BOT_VAULT_PASS  RS2B0T  BOT_CPU  SKIP_GPU
  MEMORY_REF_HOST_ENGINE=host.docker.internal

repo root: $REPO_ROOT
EOF
}

cmd_check_daemon() {
  if ! command -v docker >/dev/null 2>&1; then
    echo "host-run: docker CLI missing" >&2
    return 1
  fi
  if docker info >/dev/null 2>&1; then
    echo "host-run: docker daemon OK"
    docker version --format 'client={{.Client.Version}} server={{.Server.Version}}' 2>/dev/null || true
    return 0
  fi
  echo "host-run: docker daemon unavailable (socket missing or Desktop stopped)" >&2
  docker context ls 2>/dev/null || true
  return 1
}

cmd_static_validate() {
  local rc=0
  echo "host-run: static-validate (no daemon, no pull, no cargo)"
  for f in \
      "$REF_DIR/Dockerfile" \
      "$REF_DIR/compose.yaml" \
      "$REF_DIR/.dockerignore" \
      "$REF_DIR/scripts/entrypoint.sh" \
      "$REF_DIR/scripts/relay-host-engine.sh" \
      "$REF_DIR/scripts/build-test.sh" \
      "$REF_DIR/scripts/host-run.sh" \
      "$REF_DIR/desktop/start-desktop.sh"
  do
    if [[ -f "$f" ]]; then
      echo "  ok file $f"
    else
      echo "  MISSING $f" >&2
      rc=1
    fi
  done
  for s in entrypoint.sh relay-host-engine.sh build-test.sh host-run.sh; do
    if [[ -x "$REF_DIR/scripts/$s" ]] || [[ -f "$REF_DIR/scripts/$s" ]]; then
      bash -n "$REF_DIR/scripts/$s" && echo "  ok bash -n scripts/$s" || { echo "  FAIL bash -n $s" >&2; rc=1; }
    fi
  done
  bash -n "$REF_DIR/desktop/start-desktop.sh" && echo "  ok bash -n desktop/start-desktop.sh" || rc=1

  # Dockerfile base pin
  if grep -q 'rust:1.98.0-bookworm' "$REF_DIR/Dockerfile"; then
    echo "  ok rust pin 1.98.0-bookworm in Dockerfile"
  else
    echo "  FAIL rust pin" >&2
    rc=1
  fi
  # CI dep parity (subset)
  for dep in libasound2-dev libx11-dev libwayland-dev pkg-config; do
    if grep -q "$dep" "$REF_DIR/Dockerfile"; then
      echo "  ok Dockerfile has $dep"
    else
      echo "  FAIL missing $dep" >&2
      rc=1
    fi
  done
  # compose volume names are arch-split
  if grep -q '274bot-memory-ref-target-amd64' "$REF_DIR/compose.yaml" \
     && grep -q '274bot-memory-ref-target-arm64' "$REF_DIR/compose.yaml"; then
    echo "  ok arch-split target volumes"
  else
    echo "  FAIL arch-split volumes" >&2
    rc=1
  fi
  # noVNC loopback publish
  if grep -q '127.0.0.1:6080:6080' "$REF_DIR/compose.yaml"; then
    echo "  ok noVNC loopback publish"
  else
    echo "  FAIL noVNC publish binding" >&2
    rc=1
  fi
  # toolchain file in repo
  if grep -q '1.98.0' "$REPO_ROOT/rust-toolchain.toml"; then
    echo "  ok repo rust-toolchain.toml 1.98.0"
  else
    echo "  FAIL toolchain" >&2
    rc=1
  fi
  # secrets must not be copied
  if grep -nE 'COPY.*(\.274bot|vault|private\.pem|\.env)' "$REF_DIR/Dockerfile"; then
    echo "  FAIL Dockerfile appears to COPY secrets" >&2
    rc=1
  else
    echo "  ok no secret COPY in Dockerfile"
  fi
  return "$rc"
}

cmd_config() {
  cmd_check_daemon || return 1
  (cd "$REF_DIR" && MEMORY_REF_ARCH="$MEMORY_REF_ARCH" MEMORY_REF_PLATFORM="$MEMORY_REF_PLATFORM" \
    "${COMPOSE[@]}" config)
}

cmd_build() {
  cmd_check_daemon || return 1
  (cd "$REF_DIR" && MEMORY_REF_ARCH="$MEMORY_REF_ARCH" MEMORY_REF_PLATFORM="$MEMORY_REF_PLATFORM" \
    "${COMPOSE[@]}" build ref)
  docker tag "${MEMORY_REF_IMAGE}" "${MEMORY_REF_IMAGE}" 2>/dev/null || true
}

cmd_build_desktop() {
  cmd_check_daemon || return 1
  (cd "$REF_DIR" && MEMORY_REF_ARCH="$MEMORY_REF_ARCH" MEMORY_REF_PLATFORM="$MEMORY_REF_PLATFORM" \
    "${COMPOSE[@]}" --profile desktop build ref-desktop)
}

cmd_shell() {
  cmd_check_daemon || return 1
  (cd "$REF_DIR" && MEMORY_REF_ARCH="$MEMORY_REF_ARCH" MEMORY_REF_PLATFORM="$MEMORY_REF_PLATFORM" \
    "${COMPOSE[@]}" run --rm --name "memory-ref-${MEMORY_REF_ARCH}-shell" ref bash)
}

cmd_shell_desktop() {
  cmd_check_daemon || return 1
  (cd "$REF_DIR" && MEMORY_REF_ARCH="$MEMORY_REF_ARCH" MEMORY_REF_PLATFORM="$MEMORY_REF_PLATFORM" \
    "${COMPOSE[@]}" --profile desktop run --rm --service-ports \
      --name "memory-ref-${MEMORY_REF_ARCH}-desktop" ref-desktop bash)
}

cmd_run() {
  cmd_check_daemon || return 1
  (cd "$REF_DIR" && MEMORY_REF_ARCH="$MEMORY_REF_ARCH" MEMORY_REF_PLATFORM="$MEMORY_REF_PLATFORM" \
    "${COMPOSE[@]}" run --rm ref "$@")
}

cmd_tui_limits() {
  echo "MEMORY_REF_ARCH=$MEMORY_REF_ARCH"
  echo "MEMORY_REF_PLATFORM=$MEMORY_REF_PLATFORM"
  echo "MEMORY_REF_CPUS=${MEMORY_REF_CPUS:-2.0}"
  echo "MEMORY_REF_MEM=${MEMORY_REF_MEM:-4g}"
  echo "image=$MEMORY_REF_IMAGE"
}

main() {
  local cmd="${1:-}"
  shift || true
  case "$cmd" in
    check-daemon) cmd_check_daemon ;;
    config) cmd_config ;;
    static-validate) cmd_static_validate ;;
    build) cmd_build ;;
    build-desktop) cmd_build_desktop ;;
    shell) cmd_shell ;;
    shell-desktop) cmd_shell_desktop ;;
    run) cmd_run "$@" ;;
    tui-limits) cmd_tui_limits ;;
    -h|--help|help|"") usage; [[ -n "$cmd" ]] || exit 2 ;;
    *) echo "host-run: unknown $cmd" >&2; usage; exit 2 ;;
  esac
}

main "$@"
