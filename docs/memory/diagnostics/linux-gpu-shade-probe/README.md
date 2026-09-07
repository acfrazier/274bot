# linux-gpu-shade-probe (scratch diagnostic)

Standalone crate **outside** the host workspace (`[workspace]` empty table).
Does not modify client production or tests.

## Build / run (memory-ref container)

```bash
export PATH=/usr/local/cargo/bin:$PATH
export CARGO_TARGET_DIR=/target CARGO_HOME=/cargo-home HOME=/runtime/home
export SKIP_GPU=0 BOT_CPU=0 DISPLAY=:99 XDG_RUNTIME_DIR=/tmp
# WGPU_BACKEND may be set; on memory-ref-amd64-view actual adapter is Gl/llvmpipe
export WGPU_BACKEND=vulkan
export PROBE_OUT=/work/docs/memory/diagnostics/linux-gpu-shade-out
cd /work/docs/memory/diagnostics/linux-gpu-shade-probe
cargo build
/target/debug/linux-gpu-shade-probe
```

Optional: `PROBE_SHADES=16,17,32` to limit cases.

See `docs/memory/linux-gpu-shade-forensics.md` for analysis.
