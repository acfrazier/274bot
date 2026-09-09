# Frozen Stage A tooling

Generated qualification only on this card. No real pack, native Linux run,
performance acceptance, frontend, game or account execution is authorized here.
The rejected live-path Cargo probe is removed; its committed state remains in
`aab094f6`, and interrupted edits are in `prior-attempt-main.rs.txt` / `.diff`.

## Reproduce (from campaign checkout)

Use a NEW owned run directory; never rebuild an admitted binary. Each command
returns nonzero on failure and leaves evidence. Requires the frozen host/client
Git objects, Rust toolchain and offline Cargo dependency cache.

    python3 docs/memory/nav-tiled-stage-a/stage_a.py guards --run docs/memory/nav-tiled-stage-a/guards-NEW
    python3 docs/memory/nav-tiled-stage-a/stage_a.py prepare --run docs/memory/nav-tiled-stage-a/run-NEW
    python3 docs/memory/nav-tiled-stage-a/stage_a.py build --run docs/memory/nav-tiled-stage-a/run-NEW --variant clean
    python3 docs/memory/nav-tiled-stage-a/stage_a.py build --run docs/memory/nav-tiled-stage-a/run-NEW --variant counting
    python3 docs/memory/nav-tiled-stage-a/stage_a.py generated --run docs/memory/nav-tiled-stage-a/run-NEW --variant clean
    python3 docs/memory/nav-tiled-stage-a/stage_a.py generated --run docs/memory/nav-tiled-stage-a/run-NEW --variant counting
    python3 docs/memory/nav-tiled-stage-a/test_stage_a.py --run docs/memory/nav-tiled-stage-a/run-NEW

This exercises all-uniform/all-dense 64x64x4 packs, a tiny gated transport/bank
pack, malformed inputs, source/lock/tool/binary mutation and the complete release
protocol with GENERATED stand-in bytes only. It does not run the enormous
differential corpus. `run-04` is the final local receipt set; the verified archive
and inventory are under `evidence/`. Earlier `run-03` is retained locally, not the
final tool version. `archive_evidence.py --run run-04 --guards guards-04` inspects
the retained results without executing any probe; `--archive` creates a NEW
archive and verifies every member hash.

## Frozen source and diagnostics

`frozen_support.py`, `frozen_probe.rs`, `frozen_generate.py` are byte-for-byte
copies from `ccff4bbe22d63cdb8b11612aaf97bbbadcb256dc` at the corresponding
`docs/memory/nav-tiled-differential/{harness.py,probe.rs,generate.py}` paths.
Their SHA256s are pinned in `stage_a.py`; do not edit or invoke their old CLIs.
The copied helper supplies source materialization and bounded process groups.
Stage A compiles original dense `29b7aea...` and tiled `8385babb...` nav/api with
the identical client `3456edc8...`, independently, never the moving checkout.

Production bytes stay exact prefixes. Additions are explicitly admitted:
standalone workspace membership, nav binary/feature/libc manifest suffix,
router access-module suffix from the immutable helper, collision layout-module
suffix, new diagnostic siblings and exact original host routing source spans.
The helper's correctness binary is materialized but never built/run. Stage A
does not alter original decode, conversion, query or route functions. Locks are
copied from each original arm; first offline resolution may prune unrelated
workspace packages, then resolved locks are hashed and subsequent builds locked.
Clean/counting targets are separate for both arms. Toolchain binary hashes,
source inventories, original Git blob identities, tools, lock and executable
hashes are admitted; mutation fails before launch and is checked after exit.
Ancestor/home Cargo configuration is refused rather than inherited silently.

Clean global allocator is explicitly `System`. Counting global allocator wraps
System only under a separately built feature. Counting output is diagnostic,
NEVER clean CPU/RSS evidence. `narrow_allocations` is meaningful ONLY when
`diagnostic=true`; clean counters are inactive, not evidence of no allocations.
The narrow warmed check covers only collision walk-word/standable/walkable reads,
outside observer formatting, routes and workspace allocations. No zero-allocation
claim is made for route calls. Layout reads actual boxed slice lengths, capacities,
sizes, alignment and allocator usable sizes; no dense clone/export is retained.
The ten layout field names are fixed in `proposed-manifest.json`.

## Selectors and accounting

JSON must be an array of ten-integer arrays; TSV is ten integers per row. Both
have the same bounds, with no unknown-field adaptation. JSON is normalized once
to an owned TSV while preserving source and normalized hashes. Original state
presets 0..6 are copied exactly and independently asserted in both binaries.

Every row executes THREE separately declared lanes matching ccff4bb:
model-only original API; teleport-enabled model+facts original API; exact host
`ScriptRouteRequest.calculate` with options, radius, facts and fixed bank
`995x10,1712x1`. The host API uses its original running cost model; it does not
accept model=1. Thus model=1 affects the first two lanes, not the host lane.
Do not describe this as a single all-options/model route API. Essence bit uses
the original wizard 553 session; bank stands and radius approach are preserved.

Input is read in the child (generation happens in Python before launch), retained
through completed original decode/conversion, then dropped while world is live.
No encode output exists. Hot lookups, one full route warmup and eight timed
three-lane sweeps follow. Numeric route results are consumed and immediately
dropped; request/Arc owners are released before world-drop and weak-drop checks.
Current RSS is Mach resident_size or Linux statm with actual page size; ru_maxrss
is ONLY a separately named peak. CPU is self user+system; the Rust probe has no
subprocesses. Phase dwell is outside measured operation durations. Direct phase
samples and process peak survive in output; supervisor group peak survives
leader exit and cleanup. Polling is not a hard instantaneous RSS bound.

## Future root-released real protocol (disabled by default)

`stage_a.py real --run RUN --authorization AUTH --authorization-sha256 SHA`
requires root-supplied JSON and its external hash. Before any pack stat/read it
checks release mode, native Linux guard receipt, hardware/platform, order,
correctness review release, all four admissions, both generated qualification
hashes, unchanged limits, tools and proposal manifest hash. Required keys and
validation are in `check_release`; generated integration constructs a complete
stand-in example in `run-04/standin-release/authorization.json`. This example is
NOT real authorization. Root adds `guard_path`/`guard_sha256` from native guards,
sets mode=real and binds its finalized input/corpus/hardware/order. No default
pack path exists. Pack and selectors are copied to fresh owned outputs and
hash-checked; no admitted binary is rebuilt. Failed launches/results are retained.

Proposal: six fresh paired processes per arm, fixed AB/BA order, 8 MiB peak,
10% cold-load, 5% route CPU and 2 ms route p99 increase gates, baseline-repeat
noise can make them inconclusive. Fresh ownership is NOT cold OS filesystem
cache; no cache flush or allocator purge is performed. Root must finalize and
freeze the actual measurement workload before any real run. No Stage B or
resident saving follows from this tool qualification.
