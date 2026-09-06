# GPU queue-completion measurement

Opt-in extension of host `render_profile` that records **bounded**
`wgpu::Queue::on_submitted_work_done` deliveries for Texture frames after
`mainredraw`, before the frame enters the mailbox.

## Enablement

- Default **off**. No GPU callback allocation or registration unless enabled.
- Launcher: `--gpu-completion-profile` requires **panel** and `--render-profile`.
- Env: `BOT_GPU_COMPLETION_PROFILE=1` only honored when `BOT_RENDER_PROFILE=1`
  (`Run::prepare` nests the enable). Both env keys are scrubbed unless set.
- Independent of scheduling profile, allocation counting, and live benchmarks.

## Semantics (read carefully)

| Concept | Meaning |
|:---|:---|
| Callback timestamp | **CPU delivery** after prior queue submission completes |
| Delivery driver | Later existing `submit` / `poll` (production adds **no** wait poll) |
| Latency histogram | Register/submit-sample → callback delivery (conservative end-to-end **upper bound**) |
| Completion interval | Observed callback **delivery cadence** (same cadence mode only) |
| Not measured | Hardware GPU timestamps, display scanout, panel present |

`paint_n` remains host `mainredraw` counts. CPU `PixMap` frames increment
`cpu_frame_n` only and **never** count as GPU completions. Empty / unfired
callbacks are not completions.

GPU completion **throughput** may be qualified only when:

- `registration_complete` (every Texture frame registered; `dropped_n == 0`)
- counters monotonic
- recent samples present
- stable scene/mode (`ingame && scene_state == 2`, matching cadence mode)

Any drop / lost / unsupported evidence → treat throughput as **invalid or unavailable**.

## Bounds

- Per slot generation: max **8** outstanding callbacks
- Process-wide (all slots + restarts): max **256** outstanding
- RAII permit releases when the callback runs **or** the closure is dropped
- Callback captures only `Weak` telemetry + permit — never client/renderer/frame/device/queue owners
- Generation + `live` flag: late callbacks after slot end cannot credit a restarted slot
- Fixed pending timestamp slots (no unbounded pending age storage)
- Disabled path: `Local.gpu == None`; `observe_painted_output` no-ops registration

## JSON (`renderer_profile[].gpu_completion`)

Emitted on each slot row when render profile is on (zeros / `enabled: false`
when GPU completion off). Fields include `gpu_frame_n`, `cpu_frame_n`,
`registered_n`, `completed_n`, `dropped_n`, `pending_n`,
`oldest_pending_age_ms`, stable/transition completion counts, latency and
interval histograms (`interval_bound_ms` shared with paint),
`registration_complete`, and explicit semantics strings.

While render profile is off, `renderer_profile` remains JSON `null`.

## Files

- `crates/host/src/render_profile.rs` — GPU shared state, permits, tests
- `crates/host/src/lib.rs` — register before `mailbox.store`
- `crates/host/Cargo.toml` — `wgpu` + `pollster` **dev-dependencies** only
- `crates/host-play/src/memory.rs` — enable + JSONL
- `docs/memory/run_diagnostic.py` / `test_run_diagnostic.py` — flag + gates
- `docs/memory/gpu-completion-report.md` — this report

No client submodule changes. No mailbox/present ownership changes. No production
blocking poll for completion.

## Verification

```text
cargo test -p host --lib
  → 137 passed (includes real queue smoke when adapter present; soft-skips unavailable)

cargo test -p host-play --lib --features memory-profile
  → 143 passed

python3 docs/memory/test_run_diagnostic.py
  → 8 passed
```

Real queue smoke: `queue.on_submitted_work_done` + empty submit + device poll
in the **test only**. Soft-skips with an honest message if no adapter /
`request_device` fails.

## Overhead caveat

When enabled, each Texture paint allocates one `'static` callback closure and
touches atomics on delivery. Slot/process caps drop excess registrations rather
than growing queues. Disabled path is free. This task does **not** claim
renderer performance acceptance from unit tests; live panel runs are
orchestrator-owned.
