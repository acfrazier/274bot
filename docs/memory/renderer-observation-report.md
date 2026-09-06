# Renderer residency and paint-cadence observation

Opt-in host instrumentation to distinguish **requested** focus/draw policy from
**actual** live `Renderer` residency, backend kind, and host paint cadence.

## Enablement

- Default **off**. Memory JSONL emits `renderer_profile: null` while disabled.
- Launcher: `--render-profile` → `BOT_RENDER_PROFILE=1` (scrubbed unless set).
- `Run::prepare` calls `host::render_profile::enable()` before clients start.
- Independent of verbose diagnostics, allocation counting, and scheduling profile.

## What is measured

Per live slot (register once; local counters; publish ~1s or on state change):

| Field | Meaning |
|:---|:---|
| `slot_id` / `generation` | FNV username id + lifecycle generation |
| `renderer_present` | `Option<Renderer>` is `Some` after the paint decision |
| `backend_kind` | `null` / `"cpu"` / `"cpu_fallback"` / `"gpu"` |
| `prefer_cpu` | Latched prefer-cpu request while attached |
| `ingame` / `scene_state` | Client scene flags at the sample |
| `draw` / `full_rate` | Actual latches (not only requested policy) |
| `client_loop_n` | Host `client_frame` completions |
| `paint_n` / `skip_n` | Host `mainredraw` completions vs skips |
| `stable_paint_*` | Paints/intervals while `ingame && scene_state==2` |
| `transition_paint_*` | Paints/intervals during startup/rebuild |
| `attach_n` / `detach_n` / `backend_change_n` | Residency transitions |
| `sample_age_ms` / `ended` | Publish freshness and slot end |

**Critical distinction:** `paint_n` counts host-completed `Renderer::mainredraw`
calls. It is **not** GPU completed work, presented frames, or panel UI present.
Do not treat bucket edges as exact percentiles. Interval bounds
`[10,20,25,40,50,100,250,500,1000,2000]` ms + overflow support a conservative
upper bound around the 40 ms gate and ~1 s background watch cadence.

Bucket membership compares full `Duration` values (not floored millisecond
integers), so `40ms + 1ns` is outside the ≤40ms bucket.

Interval linkage requires matching cadence mode (`stable` scene, `draw`,
`full_rate`, backend). Mode changes clear the previous paint anchor even when
the transition frame only skip-paints, so mixed 1 fps / full-rate gaps are not
one steady sample.

## Design bounds

- Fixed-size counters/histograms only; no per-tick heap, world copy, or logging.
- No global registry lock on the hot path (local accumulate; lock only on flush).
- Sampling does not hold client/renderer ownership or force paint work.
- Parked/low-cadence paths still record when a tick runs (watch 1 fps included).
- Ended slots publish once then prune on `read()`; unread ended rows are also
  hard-capped (`MAX_ENDED_UNREAD=64`) on register/end so restart storms stay
  bounded without a sampler.
- Production paint policy unchanged: last-FBO `scene_state==1` freeze, skip-paint
  cadence, existing counters/snapshot APIs.

## Files

- `crates/host/src/render_profile.rs` — registry, local counters, tests
- `crates/host/src/lib.rs` — `client_frame` hook + `run_client` registration
- `crates/host-play/src/memory.rs` — enable + JSONL `renderer_profile`
- `docs/memory/run_diagnostic.py` — flag, env scrub, metadata provenance
- `docs/memory/test_run_diagnostic.py` — flag regression

## Verification

- `cargo test -p host --lib` — 129 passed (module + attach/detach client_frame)
- `cargo test -p host-play --lib --features memory-profile` — 143 passed
- `python3 docs/memory/test_run_diagnostic.py` — 7 passed

No live run, release build, or GPU completion claims in this task. Another
orchestrator path may inspect GPU completion seams separately.
