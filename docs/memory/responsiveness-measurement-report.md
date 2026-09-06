# Responsiveness measurement (decode→script + input→UI endpoint)

Opt-in host profiler `responsiveness_profile` records **bounded** latencies for
two independent paths. Default **off**. Independent of scheduling, render, and
GPU-completion profiles. Not a performance-acceptance claim.

## Enablement

- Default **off**. No queue allocation, stamps, or JSON rows unless enabled.
- Env: `BOT_RESPONSIVENESS_PROFILE=1` (`Run::prepare` and host slot loop).
- Launcher: `--responsiveness-profile` (panel or tui). Scrubbed unless set.
- Input surface is set by the frontend at start:
  - panel → `InputSurface::Panel`
  - PTY TUI → `InputSurface::Tui`
  - TUI without controlling terminal → `InputSurface::TuiHeadless` (visible-ack
    **unavailable**)

## Metrics

### 1. Decode → script dispatch

| Field | Meaning |
|:---|:---|
| Start | Host `PLAYER_INFO` generation edge after `after_drain` (decoded update that can drive a script tick) |
| End | `script_observe` path that actually enters `on_game_tick` / hold isolate tick |
| Bridge | Host stamps into a process-wide ring; host-play drains after observe |
| Cancel | Tick edge with no script dispatch (coverage incomplete) |
| Lost | Local pending still open when `Local` drops |
| Drop | Pending ring full (`MAX_DECODE_PENDING` per slot) |
| Latency | Start Instant → dispatch Instant; fixed `LATENCY_BOUNDS_MS` buckets |
| p99 | Upper bound of the bucket that contains the 99th percentile sample |
| Coverage | `decode_coverage_complete`: pending empty and every edge dispatched, canceled, or lost; no drops |

**Not measured:** script body duration, V8 isolate work beyond entry, packet
decode wall time before the gen edge.

### 2. Input → visible acknowledgement

| Field | Meaning |
|:---|:---|
| Start | Actionable focused input capture: panel click/key edge or TUI key/mouse Down |
| Bind (panel) | Host, after `mailbox.store`, binds outstanding starts to that mailbox generation when ≥1 actionable event drained into the shell |
| End (panel) | `note_panel_present` when a new mailbox generation is presented to the focused Game Image / wall tile texture (`require_gen != 0` and `require_gen <= presented_gen`) |
| End (TUI) | `note_tui_draw_flush` after a successful `terminal.draw` for the focused slot |
| Cancel | Explicit cancel (e.g. focus loss path) |
| Drop | Input pending ring full |
| Lost | Pending still open when slot ends / surface reset |
| Headless TUI | `visible_ack.available = false`; `missing_capability` names PTY draw path |

**Not measured / unavailable:**

- Display scanout / compositor present (panel and TUI)
- GPU queue completion or HW timestamps (see `gpu-completion-report.md`)
- Non-focused slots’ inputs
- Pure mouse-move streams without button/key edges
- Equating queue admission, ImGui construction, or a paint *call* with visible completion

`visible_ack` JSON always states endpoint, availability, missing capability, and
semantics string so a missing endpoint is never a silent pass.

## Bounds

- Decode pending: **8** per slot generation
- Input pending: **64** process-wide
- Latency histogram: fixed `LATENCY_BOUNDS_MS` (same style as render intervals)
- Disabled path: `Local::new` → `None`; note_* no-ops; JSON `null`

## JSON (`responsiveness_profile`)

Top-level sample field (array of slots while on, else `null`). Per slot:

- identity: `slot_id`, `generation`, `updated_ms`, `sample_age_ms`, `ended`, `input_surface`
- decode: edge/dispatch/canceled/lost/dropped/pending, latency n/ns/buckets, `decode_p99_upper_bound_ms`, `decode_coverage_complete`, `decode_means`
- input: start/complete/canceled/lost/dropped/pending, latency n/ns/buckets, `input_p99_upper_bound_ms`, `input_coverage_complete`
- `latency_bound_ms`, nested `visible_ack` (`available`, `endpoint`, `missing_capability`, `means`)

Existing `renderer_profile` / `scheduling_profile` / `gpu_completion` meanings are unchanged.

## Files

- `crates/host/src/responsiveness_profile.rs` — module, registry, tests
- `crates/host/src/lib.rs` — SlotLoop Local, PLAYER_INFO decode stamp, mailbox bind, actionable drain
- `crates/host/src/slot_io.rs` — `drain_with_actionable_flag`
- `crates/host-play/src/lib.rs` — bridge drain after `script_observe`
- `crates/host-play/src/memory.rs` — enable + JSON emit
- `crates/panel/src/{session,app}.rs` — capture start + present endpoint + surface
- `crates/tui/src/bin.rs` + `Cargo.toml` — key/draw endpoints + host dep
- `docs/memory/run_diagnostic.py` / `test_run_diagnostic.py` — flag + scrub

## Tests run (this task)

- `cargo test -p host --lib` → 152 passed, 1 ignored (existing GPU smoke)
- `cargo test -p host --lib responsiveness_profile` → 11 passed (disabled, pair/order, delay, cancel, overflow, bridge, panel present, TUI draw, headless unavailable, p99 buckets)
- `cargo test -p host-play --lib` → 114 passed
- `cargo test -p host-play --lib --features memory-profile` → (re-run at commit)
- `cargo check -p panel` / `cargo check -p tui` → ok
- `python3 docs/memory/test_run_diagnostic.py` → 9 passed

No client crate changes. No live panel/TUI acceptance run in this task.

## Matched enabled/disabled overhead recipe (for downstream live task)

Do **not** treat unit tests as overhead evidence. For the live card, run a
**matched pair** on the same machine/binary/commit:

1. Build once: release panel-play (or tui-play) at the frozen commit.
2. Disabled cell: same frontend/N/workload/warmup/observe; **omit**
   `--responsiveness-profile` (and leave `BOT_RESPONSIVENESS_PROFILE` scrubbed).
3. Enabled cell: identical args plus `--responsiveness-profile` only.
4. Keep scheduling/render/GPU profiles **off** unless the live card explicitly
   combines them; if combined, enable the same extras on both cells.
5. Record: wall time, process CPU, RSS samples, and whether
   `responsiveness_profile` is null vs populated; do not claim savings from unit
   tests or from mismatched flags.
6. Fail closed if coverage is incomplete (`decode_coverage_complete` /
   `input_coverage_complete` false) or if headless TUI reports visible-ack
   unavailable when the card required a visible endpoint.

## Remaining gaps (explicit)

- No live overhead or p99 acceptance numbers yet.
- Panel present is texture/upload present, **not** display scanout.
- TUI ack is post-`terminal.draw` flush, **not** terminal emulator paint.
- Input metric is focused-slot only; Multibox non-focused capture is out of scope.
- Decode stamp is PLAYER_INFO gen edge after drain, not full packet decode start.
- Code review is not performance acceptance; downstream live + attribution cards remain.

## Review note

Code review of this commit is **not** campaign performance acceptance.