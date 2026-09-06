# 274bot Linux memory-reference image (preparation artifact).
# Base: official rust 1.98.0 on Debian bookworm (amd64 + arm64v8).
# Does NOT embed secrets, vaults, caches, or operator home state.
#
# Build (from repo root, after Docker daemon is up):
#   docker buildx build --platform linux/amd64 \
#     -f docker/memory-reference/Dockerfile \
#     -t 274bot-memory-ref:1.98.0-bookworm-amd64 .
#   docker buildx build --platform linux/arm64 \
#     -f docker/memory-reference/Dockerfile \
#     -t 274bot-memory-ref:1.98.0-bookworm-arm64 .
#
# Optional desktop extras (Xvfb/Openbox/x11vnc/noVNC) via build-arg:
#   --build-arg WITH_DESKTOP=1
#
# Pin notes (verified against public indexes at prep time, not pulled here):
#   - docker.io/library/rust:1.98.0-bookworm tags exist for amd64 and arm64v8
#   - Native build deps match .github/workflows/ci.yml (ALSA + X11/Wayland headers)
#   - Desktop stack uses Debian bookworm packages when WITH_DESKTOP=1

ARG RUST_IMAGE=rust:1.98.0-bookworm
FROM ${RUST_IMAGE}

ARG WITH_DESKTOP=0
ARG DEBIAN_FRONTEND=noninteractive

# CI-parity native libs for panel (client audio/cpal → alsa-sys) and window/winit.
# Plus toolchain pieces needed to compile the workspace on Linux.
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config \
        build-essential \
        cmake \
        libclang-dev \
        libssl-dev \
        libasound2-dev \
        libwayland-dev \
        libxkbcommon-dev \
        libx11-dev \
        libxcursor-dev \
        libxi-dev \
        libxrandr-dev \
        ca-certificates \
        curl \
        git \
        python3 \
        python3-minimal \
        socat \
        procps \
        iproute2 \
    && if [ "$WITH_DESKTOP" = "1" ]; then \
        apt-get install -y --no-install-recommends \
            xvfb \
            openbox \
            x11vnc \
            novnc \
            websockify \
            xterm \
            dbus-x11 \
            libgl1-mesa-dri \
            mesa-utils \
        ; fi \
    && rm -rf /var/lib/apt/lists/*

# Toolchain is already 1.98.0 from the base image; keep rustfmt/clippy available.
RUN rustup component add rustfmt clippy \
    && rustc --version && cargo --version

# Workspace mount point. Source of truth is the bind-mounted checkout.
WORKDIR /work

# External runtime dirs (bind-mounted at run time; empty placeholders only).
RUN mkdir -p /runtime/home /runtime/engine /runtime/cache /cargo-home /target \
    && chmod 755 /runtime /cargo-home /target

ENV CARGO_HOME=/cargo-home \
    CARGO_TARGET_DIR=/target \
    PATH="/cargo-home/bin:${PATH}" \
    # Local world defaults; engine reached via loopback relay (see scripts).
    BOT_TARGET=local \
    # Default off; set BOT_CPU=1 for CpuPix3D (honest software path).
    # Do not claim native Linux GPU performance from Docker Desktop on macOS.
    RUST_BACKTRACE=1

COPY docker/memory-reference/scripts/ /opt/memory-reference/scripts/
COPY docker/memory-reference/desktop/ /opt/memory-reference/desktop/

RUN chmod +x /opt/memory-reference/scripts/*.sh \
    && chmod +x /opt/memory-reference/desktop/*.sh 2>/dev/null || true

ENTRYPOINT ["/opt/memory-reference/scripts/entrypoint.sh"]
CMD ["bash"]
