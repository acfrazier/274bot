# N=16 and explicit low-end panel reference policies

## Scope

Harness-only support for the approved low-end reference cells (performance-finish-plan §1 and §3):

- Scale **N=16** accepted alongside 1 / 32 / 128.
- Two **explicit opt-in** panel policies, plus preserved historical `--single-renderer`.
- No live measurement, no GPU completion instrumentation, no client/CPU-renderer/scheduler changes.

## CLI / env

| Operator request | Env | `render_policy` metadata | `single_renderer` metadata |
|:---|:---|:---|:---|
| (panel default) | (none) | `rotating-all` | false |
| `--single-renderer` | `BOT_MEMORY_SINGLE_RENDERER=1` | `fixed-one` | **true** (historical) |
| `--focused-one` | `BOT_MEMORY_RENDER_POLICY=focused-one` | `focused-one` | false |
| `--focused-background` | `BOT_MEMORY_RENDER_POLICY=focused-plus-background` | `focused-plus-background` | false |
| TUI | (none) | `none` | false |

Metadata also sets `render_policy_requested=true` so consumers treat the field as **requested mode**, not observed cadence/GPU proof.

Conflicts rejected before process start:

- more than one of `--single-renderer` / `--focused-one` / `--focused-background`
- any of those flags (or the corresponding env at Rust prepare) on **tui**
- `BOT_MEMORY_SINGLE_RENDERER` together with `BOT_MEMORY_RENDER_POLICY`
- `--nav-captures` unless panel **and** (`--single-renderer` or `--focused-one`)

Launcher scrubs `BOT_MEMORY_SINGLE_RENDERER` and `BOT_MEMORY_RENDER_POLICY` before applying the chosen flags.

## Draw / focus semantics (panel)

Applied each `memory_focus` via existing `draw_for_slot` / `full_rate_for`:

| Policy | Focus | Draw | Full-rate |
|:---|:---|:---|:---|
| `rotating-all` | rotates every 30s | historical all-render bits only | unchanged prefs |
| `fixed-one` (legacy) | pin 0 | only selected; rail `renderer_by=false` | historical (does not force cadence knobs) |
| `focused-one` | pin 0 | only selected; sim-only others | forces `focused_50=true`, `sidecar_50=false`, `live_full_rate=false`, game pane, wall membership |
| `focused-plus-background` | pin 0 | all members draw | focused full-rate; others **not** full-rate → existing 1 fps skip-paint |

New policies start from adverse prefs in tests (pane closed, wall closed, renderer off, sidecar/live full-rate on, focused_50 off) and still produce the table above.

## Validation run (this task)

| Command | Result |
|:---|:---|
| `python3 docs/memory/test_run_diagnostic.py -v` | 6 passed |
| `cargo test -p host-play --features memory-profile memory:: -- --test-threads=1` | 27 passed |
| `cargo test -p panel --features memory-profile memory_ -- --test-threads=1` | 4 passed |

Live mode/cadence proof is **out of scope**; orchestrator builds immutable release binaries in a later task.

## Limitations

- Requested policy ≠ measured frames/s or GPU residency.
- Legacy `fixed-one` deliberately does not force cadence knobs (old summaries stay accurate).
- Production panel defaults and persisted operator UI are unchanged outside the memory benchmark path (`persist_ui=false` on prepare_memory).
