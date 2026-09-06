# Immutable low-end reference builds

Task `t_dac0878e`. Freeze only — no live performance acceptance and no source
optimization. Parent responsiveness work is already approved at `57cbfc7`.

## Provenance gate

| Item | Value |
|:---|:---|
| Branch | `codex/memory-diagnostics` |
| Host HEAD | `57cbfc7fae896cbfcef3cc4d7ac98e026960011d` |
| Host dirty (source) | clean (`git diff HEAD` empty) |
| Host sources sha256 | `41eb8d95546af5e416e1990460020cd6a0e96d707bd91dc963d3dd855c451569` |
| Client submodule | `451759f2a7df9c57895657d5b8d506172860cee1` (`codex/memory-sprite-reuse`) |
| Client dirty | clean |
| Client sources sha256 | `27246deba654abac980395b0bbf9ad077d8d3feb66992b4d659512e0630e4a25` |
| rustc | `rustc 1.98.0 (88d9e12ae 2026-08-18)` host `aarch64-apple-darwin` |
| cargo | `cargo 1.98.0 (797e8a9bc 2026-08-05)` |
| OS | macOS 15.7.9 arm64 |
| Nav pack | `~/.274bot/274bot.navpack` sha256 `2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30` |
| Nav flags | `~/.274bot/274bot.navflags` sha256 `92d5dea05c886ac8720be6b47e47cbc68355a8ff42676c0886f5b7ea8343a4cb` |
| Catalog `js-scripts.json` | sha256 `3bee852f888a9c383b23d299979a3e898c4b620b73b5c8a6f69e87584399d4f6` |
| rs2b0t | `/Users/acfrazier/experiments/rs2b0t` @ `100adccc037d9f6898080e1cad58fcfc43364775` (clean) |
| Server intent | `BOT_TARGET=local` (no credentials recorded) |

## Feature / allocator verification

Build (joint release):

```bash
cargo build --release -p panel --bin panel-play -p tui --bin tui-play \
  --features memory-profile-no-alloc
```

Cargo release fingerprints for the newest `panel-play`, `tui-play`, and
`host-play` units all list:

`["memory-profile", "memory-profile-no-alloc"]`

Code path: `host_play::memory` sets `BenchmarkAllocator = std::alloc::System`
under `cfg(feature = "memory-profile-no-alloc")`. Sample JSON reports
`allocation_counting: false` and nulls rust allocation fields. `nm` shows no
host_play `CountingAllocator` symbols on the saved panel binary.

## Immutable binaries (gitignored diagnostics tree)

Directory: `docs/memory/diagnostics/reference-build-20260906T215255Z/`

| Binary | SHA-256 | Size |
|:---|:---|---:|
| `panel-play-system-20260906T215255Z` | `ed403b4f1bbce4c6ff86d18417bdf2ab267064f25f5ff4b7f64a3c9daa9b6abb` | 84970080 |
| `tui-play-system-20260906T215255Z` | `91103790581692e079ff544aa813e9651c689e221f6584df66afcfdfa5789017` | 81263600 |

`SHA256SUMS` sits beside the binaries. Historical control directories
(`cpu-overhead-builds/`, `runner-boundary-build/`, `shared-stop-build/`,
`memory-resume-build/`, …) were **not** overwritten.

Machine-readable twin: [reference-build-manifest.json](reference-build-manifest.json).

## Tests run (qualification before freeze)

| Command | Result |
|:---|:---|
| `cargo test -p host --lib` | 154 passed, 1 ignored |
| `cargo test -p host --lib gpu_real_queue -- --ignored --nocapture` | **1 passed** (fail-closed real GPU smoke) |
| `cargo test -p host-play --lib --features memory-profile` | 143 passed |
| `cargo test -p host-play --lib --features memory-profile-no-alloc` | 143 passed |
| `cargo test -p panel --features memory-profile` | 377 passed |
| `cargo test -p tui --features memory-profile` | 87 passed |
| `python3 docs/memory/test_run_diagnostic.py -v` | 9 passed |

## Frontend geometry / settings (no secrets)

- Panel applet never below **765×503**; focused full-rate target **50 fps**;
  background skip-paint **1 fps**; last-FBO freeze while `scene_state==1`.
- TUI raster **Off**; reference diagnostic PTY size **120×40**; real PTY path
  required for TUI cells (do not use `--headless` for the reference table).
- Client loop target **20 ms**. Fixture: sustained level-50 Thiever
  (`--sustain` / `BOT_MEMORY_SUSTAIN=1`).
- Clean cells: system allocator, `--no-diagnostics`, no stack logging, no
  nav-captures (except the separate visual pilot).
- Approved modes only: TUI; panel `--focused-one`; panel `--focused-background`.

## Six active reference cells (executable)

Timing defaults for this reference stage: **30s warmup / 120s observe / 60s
teardown**. Run sequentially with no concurrent builds, tests, or profiling.
Qualify each cell with `qualify_control.py` after exit; frontend exit 0 alone
is not qualification.

Set:

```bash
PANEL=docs/memory/diagnostics/reference-build-20260906T215255Z/panel-play-system-20260906T215255Z
TUI=docs/memory/diagnostics/reference-build-20260906T215255Z/tui-play-system-20260906T215255Z
```

1. TUI N=1 active (real PTY):

```bash
python3 docs/memory/run_diagnostic.py tui 1 active --sustain --no-diagnostics \
  --render-profile --binary "$TUI" --warmup 30 --observe 120
```

2. TUI N=16 active (real PTY):

```bash
python3 docs/memory/run_diagnostic.py tui 16 active --sustain --no-diagnostics \
  --render-profile --binary "$TUI" --warmup 30 --observe 120
```

3. Panel focused-one N=1 active:

```bash
python3 docs/memory/run_diagnostic.py panel 1 active --sustain --no-diagnostics \
  --focused-one --render-profile --gpu-completion-profile \
  --binary "$PANEL" --warmup 30 --observe 120
```

4. Panel focused-one N=16 active:

```bash
python3 docs/memory/run_diagnostic.py panel 16 active --sustain --no-diagnostics \
  --focused-one --render-profile --gpu-completion-profile \
  --binary "$PANEL" --warmup 30 --observe 120
```

5. Panel focused-background N=1 active:

```bash
python3 docs/memory/run_diagnostic.py panel 1 active --sustain --no-diagnostics \
  --focused-background --render-profile --gpu-completion-profile \
  --binary "$PANEL" --warmup 30 --observe 120
```

6. Panel focused-background N=16 active:

```bash
python3 docs/memory/run_diagnostic.py panel 16 active --sustain --no-diagnostics \
  --focused-background --render-profile --gpu-completion-profile \
  --binary "$PANEL" --warmup 30 --observe 120
```

## Separate overhead comparison (same binary/config)

Match cell 6 policy and binary; only drop observation profiles. Screens combined
`--render-profile` / `--gpu-completion-profile` overhead. One noisy pair — not
an accepted optimization saving.

```bash
python3 docs/memory/run_diagnostic.py panel 16 active --sustain --no-diagnostics \
  --focused-background --binary "$PANEL" --warmup 30 --observe 120
```

Optional later responsiveness overhead pair: same binary + cell 6 flags with and
without `--responsiveness-profile` (not required to freeze the builds).

## Functional / visual pilot (not a clean cell)

Exclude resource values from clean acceptance:

```bash
python3 docs/memory/run_diagnostic.py panel 1 active --sustain --no-diagnostics \
  --focused-one --render-profile --gpu-completion-profile --nav-captures \
  --binary "$PANEL" --warmup 30 --observe 120
```

## Which observations need profiling

| Observation | Required flags | Notes |
|:---|:---|:---|
| Per-slot residency / backend / host paint cadence | `--render-profile` | TUI must show zero resident renderers |
| GPU completion coverage / delivery latency | `--render-profile --gpu-completion-profile` (panel) | CPU delivery timestamps — not HW GPU ts or scanout |
| decode→script and input→UI p99 | `--responsiveness-profile` | Measure overhead separately on same binary |
| Clean CPU/RSS/latency acceptance | profiles off or overhead-corrected | No counting allocator; no diagnostic sidecar |

Downstream overhead controls **must** reuse these exact binary paths and the
same frontend / N / workload / render policy; only toggle profile flags. Do not
rebuild between a pair.

## Out of scope for this task

- Live cell execution and budget acceptance
- Source optimization
- Final 1/16 three-mode matrix, modest-hardware validation, lifecycle, capacity

## Remaining gap

Next card (`t_672f4ac3`) runs the six live 1/16 active reference cells plus the
overhead/functional checks against these frozen binaries, then qualifies results.
This freeze does **not** claim any performance pass.
