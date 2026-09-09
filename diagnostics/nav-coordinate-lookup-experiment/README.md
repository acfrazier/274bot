# Generated coordinate lookup experiment

Diagnostic only. This directory contains the bounded macOS aarch64 release-build comparison for the private coordinate lookup refinement. It uses no native host, SSH, live client, real pack, Stage A worker, manifest edit, or admission change.

The ignored nav unit benchmark constructs deterministic generated collision worlds, warms each workload, and records seven read and route wall-time samples per process. Four clean-built/reused processes per arm are retained in `samples.jsonl`; the six reused-binary processes ran in before/after/after/before/before/after order after one clean-build process per arm. Every process uses the same inputs, probe/route order, checksums, and layout fields. `analyze.py` validates those invariants and writes `summary.json`.

The `before` clean build was taken after the behavior-preserving helper extraction but before any coordinate API called that helper. The checkout's production `collision.rs` had first been verified byte-identical to frozen `8385babb`; the helper extraction left the linear decomposition and coordinate API call path intact. The `after` clean build redirects only validated coordinate calls through the helper. Therefore timings isolate the call-path refinement, while source fidelity is established separately by the frozen-source diff and differential tests.

Commands:

    NAV_COORD_ARM=before CARGO_TARGET_DIR=/tmp/274bot-navcoord-before-t8cdb3301 cargo test -p nav --release generated_coordinate_lookup_benchmark -- --ignored --nocapture --test-threads=1
    NAV_COORD_ARM=after CARGO_TARGET_DIR=/tmp/274bot-navcoord-after-t8cdb3301 cargo test -p nav --release generated_coordinate_lookup_benchmark -- --ignored --nocapture --test-threads=1
    python3 diagnostics/nav-coordinate-lookup-experiment/analyze.py

`assembly-before-pair-at-index.txt` and the four `assembly-after-*` files are exact `objdump -C --disassemble-symbols=...` outputs from the release test binaries. The baseline packed coordinate path calls `pair_at_index`, whose body contains two `udiv` instructions. The after coordinate methods contain no `udiv`; `assembly-after-pair-at-index.txt` retains two `udiv` instructions because the public linear-index API still decomposes and shares the same coordinate helper as designed. Assembly evidence falsifies the possibility that LLVM had already removed all redundant coordinate decomposition in this exact benchmark build; it does not by itself establish an accepted performance win.
