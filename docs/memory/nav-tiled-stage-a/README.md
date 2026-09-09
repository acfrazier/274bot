# Stage A synthetic load/route tooling

This directory is a preparation-only diagnostic. It never opens a production
navpack and has no default real-input path.

Build and run the local synthetic probe:

    cargo run --manifest-path docs/memory/nav-tiled-stage-a/Cargo.toml --release -- all-uniform docs/memory/nav-tiled-stage-a/routes.tsv
    cargo run --manifest-path docs/memory/nav-tiled-stage-a/Cargo.toml --release -- all-dense docs/memory/nav-tiled-stage-a/routes.tsv

The Rust probe generates bounded 32x32x4 input, decodes it through the original
`nav::pack::decode`, performs repeated route calls, and emits one numeric JSON
record. Milestones are explicit: retained_input, decoded_converted,
hot_lookup_route, world_drop, dropped_input, post_drop. Input bytes and route
outputs are consumed but no correctness stream is retained. Current RSS uses
`/proc/self/statm` on Linux or direct `ps` sampling on Darwin; peak RSS is not
`ru_maxrss`. Whole-process CPU uses getrusage.

`supervise.py` provides hash-bound regular-file admission, strict selector
validation (including state presets 4, 5, and 6), owned process-group cleanup,
output/wall/CPU/RSS bounds, EOF-vs-leader-exit handling, and paired aggregate
noise classification. It is intentionally not a real-input launcher.

The manifest fixes nine repetitions and alternating arm order, System allocator,
release/no-incremental settings, synthetic extremes, limits, and proposed gates.
Those gates are thresholds only, not results or acceptance. Real source, binary,
route corpus, hardware and root authorization must be frozen separately.
