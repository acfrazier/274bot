#!/usr/bin/env bash
# In-container (or documented) build/test helpers for the Linux reference.
# Separates host workspace tests from client submodule tests and from LIVE.
# Never treats ignored/unavailable GPU tests as proof.
set -euo pipefail

ROOT="${MEMORY_REF_ROOT:-/work}"
cd "$ROOT"

usage() {
  cat <<'EOF'
usage: build-test.sh <command>

commands:
  fmt                 cargo fmt --check (host + client manifests)
  clippy              cargo clippy -D warnings (host + client; needs native libs)
  test-host           cargo test --workspace --exclude client (274bot crates only)
  test-host-memory    host-play/panel/tui tests with memory-profile-no-alloc
  test-client         cargo test --manifest-path vendor/fr-client-rust/Cargo.toml --workspace
  test-unit           test-host + test-host-memory + test-client with SKIP_GPU=1 (CI parity)
  build-release-tui   release tui-play (system allocator profile for ref cells)
  build-release-panel release panel-play (same features)
  build-release-ref   both release frontends with memory-profile-no-alloc
  live-e2e            LIVE=1 e2e ignored tests (requires engine via relay)
  live-host-play      LIVE=1 host-play ignored tests
  smoke-paths         print resolved paths / versions (no compile)

Environment:
  SKIP_GPU=1          default for unit tests (no adapter claim)
  CARGO_TARGET_DIR    should be /target (volume) inside the container
  BOT_TARGET=local    default
EOF
}

need_work() {
  [[ -f "$ROOT/Cargo.toml" && -f "$ROOT/rust-toolchain.toml" ]] \
    || { echo "build-test: $ROOT is not the 274bot checkout" >&2; exit 1; }
}

cmd_fmt() {
  need_work
  cargo fmt --all -- --check
  cargo fmt --all --manifest-path vendor/fr-client-rust/Cargo.toml -- --check
}

cmd_clippy() {
  need_work
  cargo clippy --workspace --all-targets --no-deps -- -D warnings
  cargo clippy --manifest-path vendor/fr-client-rust/Cargo.toml \
    --workspace --all-targets --no-deps -- -D warnings
}

cmd_test_host() {
  need_work
  # Host workspace unit/integration tests only. Cargo can still surface the path
  # dep `client` in the package graph; exclude it so host vs client stay separate.
  # GPU-ignored tests stay ignored; SKIP_GPU=1 is not GPU proof.
  SKIP_GPU="${SKIP_GPU:-1}" cargo test --workspace --exclude client
}

cmd_test_host_memory() {
  need_work
  # Memory-campaign crates must also pass under the no-alloc profiling feature set
  # used by release reference frontends (not default features alone).
  export SKIP_GPU="${SKIP_GPU:-1}"
  cargo test -p host-play --features memory-profile-no-alloc
  cargo test -p panel --features memory-profile-no-alloc
  cargo test -p tui --features memory-profile-no-alloc
}

cmd_test_client() {
  need_work
  # Client submodule is a separate cargo manifest (vendor workspace).
  # Run here — not under the host workspace — even if path-dep metadata lists client.
  # Do not demote GPU failures to ignored; SKIP_GPU unavailability is recorded as such.
  SKIP_GPU="${SKIP_GPU:-1}" cargo test \
    --manifest-path vendor/fr-client-rust/Cargo.toml --workspace
}

cmd_test_unit() {
  export SKIP_GPU="${SKIP_GPU:-1}"
  cmd_test_host
  cmd_test_host_memory
  cmd_test_client
}

cmd_build_release_tui() {
  need_work
  cargo build --release -p tui --bin tui-play \
    --features memory-profile-no-alloc
}

cmd_build_release_panel() {
  need_work
  cargo build --release -p panel --bin panel-play \
    --features memory-profile-no-alloc
}

cmd_build_release_ref() {
  need_work
  cargo build --release -p panel --bin panel-play -p tui --bin tui-play \
    --features memory-profile-no-alloc
}

cmd_live_e2e() {
  need_work
  echo "build-test: LIVE qualification — requires engine on relay path" >&2
  LIVE=1 cargo test -p e2e -- --ignored --test-threads=1
}

cmd_live_host_play() {
  need_work
  LIVE=1 cargo test -p host-play -- --ignored --test-threads=1
}

cmd_smoke_paths() {
  echo "pwd=$(pwd)"
  echo "rustc=$(rustc --version 2>/dev/null || echo missing)"
  echo "cargo=$(cargo --version 2>/dev/null || echo missing)"
  echo "CARGO_HOME=${CARGO_HOME:-}"
  echo "CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-}"
  echo "HOME=${HOME:-}"
  echo "ENGINE_DIR=${ENGINE_DIR:-}"
  echo "BOT_TARGET=${BOT_TARGET:-}"
  echo "SKIP_GPU=${SKIP_GPU:-}"
  echo "BOT_CPU=${BOT_CPU:-}"
  echo "platform=$(uname -m)"
  echo "toolchain_file="
  cat rust-toolchain.toml 2>/dev/null || true
  if [[ -d vendor/fr-client-rust ]]; then
    echo "client_head=$(git -C vendor/fr-client-rust rev-parse HEAD 2>/dev/null || echo unknown)"
  fi
  # Relay status
  if [[ -f /tmp/memory-ref-relay/game.pid ]]; then
    echo "relay_game_pid=$(cat /tmp/memory-ref-relay/game.pid)"
  else
    echo "relay_game_pid=none"
  fi
}

main() {
  local cmd="${1:-}"
  shift || true
  case "$cmd" in
    fmt) cmd_fmt "$@" ;;
    clippy) cmd_clippy "$@" ;;
    test-host) cmd_test_host "$@" ;;
    test-host-memory) cmd_test_host_memory "$@" ;;
    test-client) cmd_test_client "$@" ;;
    test-unit) cmd_test_unit "$@" ;;
    build-release-tui) cmd_build_release_tui "$@" ;;
    build-release-panel) cmd_build_release_panel "$@" ;;
    build-release-ref) cmd_build_release_ref "$@" ;;
    live-e2e) cmd_live_e2e "$@" ;;
    live-host-play) cmd_live_host_play "$@" ;;
    smoke-paths) cmd_smoke_paths "$@" ;;
    -h|--help|help|"") usage; [[ -n "$cmd" ]] || exit 2 ;;
    *) echo "build-test: unknown command: $cmd" >&2; usage; exit 2 ;;
  esac
}

main "$@"
