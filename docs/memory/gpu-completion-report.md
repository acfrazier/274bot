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
| Latency histogram | Host `mainredraw` **start** Instant → callback delivery (upper bound on that interval only; not pre-paint host work, not HW GPU ts, not scanout/panel present) |
| Completion interval | Observed callback **delivery cadence** within one **mode epoch** only |
| Lost / canceled | Registered callback dropped without a successful complete (`lost_n`) |
| Not measured | Hardware GPU timestamps, display scanout, panel present |

`paint_n` remains host `mainredraw` counts. CPU `PixMap` frames increment
`cpu_frame_n` only and **never** count as GPU completions. Empty / unfired
callbacks are not completions; dropping them increments `lost_n`.

GPU completion **registration** may be treated complete only when:

- every Texture frame registered (`registered_n == gpu_frame_n`)
- `dropped_n == 0` and `lost_n == 0`

**Throughput / coverage** additionally requires `completion_coverage_complete`:
pending cleared and `completed_n == registered_n`.

Any drop / lost / unsupported evidence → treat throughput as **invalid or unavailable**.

## Hot-path / concurrency (reviewer fixes)

- Routine register/complete traffic updates the **local** snap only. Global
  `REGISTRY` publish stays on the parent ~1s / state-change / end path (no
  dirty/flush every GPU frame).
- Completion/lost/latency counters drain as one coherent `GpuDeltaBatch` under
  a single lock (no multi-swap discard of partial histograms).
- `pending_submit_ms` is cleared **only** by the owning `GpuPermit` Drop — not
  by `on_complete` — so a reused cell cannot lose a newer stamp.
- Cadence **mode epoch** bumps on mode change and Local drop so A→B→A late
  callbacks cannot invent stable intervals across eras.
- Real wgpu smoke is `#[ignore]` and **fail-closed** (`expect` adapter/device);
  it submits bounded buffer work, not an empty submit-only check. Soft-skip is
  not used.

## Bounds

- Per slot generation: max **8** outstanding callbacks
- Process-wide (all slots + restarts): max **256** outstanding
- RAII permit releases when the callback runs **or** the closure is dropped
- Callback captures only `Weak` telemetry + permit — never client/renderer/frame/device/queue owners
- Generation + `live` + mode epoch: late callbacks after slot end / mode change cannot contaminate restart or pair across eras
- Fixed pending timestamp slots (no unbounded pending age storage)
- Disabled path: `Local.gpu == None`; `observe_painted_output` no-ops registration

## JSON (`renderer_profile[].gpu_completion`)

Emitted on each slot row when render profile is on (zeros / `enabled: false`
when GPU completion off). Fields include `gpu_frame_n`, `cpu_frame_n`,
`registered_n`, `completed_n`, `dropped_n`, `lost_n`, `pending_n`,
`oldest_pending_age_ms`, stable/transition completion counts, latency and
interval histograms (`interval_bound_ms` shared with paint),
`registration_complete`, `completion_coverage_complete`, and explicit
semantics strings (`latency_means`, `interval_means`, `timestamp_semantics`).

While render profile is off, `renderer_profile` remains JSON `null`.

## Files

- `crates/host/src/render_profile.rs` — GPU shared state, permits, tests
- `crates/host/src/lib.rs` — `mainredraw` start Instant + register before `mailbox.store`
- `crates/host/Cargo.toml` — `wgpu` + `pollster` **dev-dependencies** only
- `crates/host-play/src/memory.rs` — enable + JSONL
- `docs/memory/run_diagnostic.py` / `test_run_diagnostic.py` — flag + gates
- `docs/memory/gpu-completion-report.md` — this report

No client submodule changes. No mailbox/present ownership changes. No production
blocking poll for completion.

## Verification

```text
cargo test -p host --lib render_profile::
  → 20 passed; 1 ignored (real GPU adapter proof)

cargo test -p host --lib
  → (full host lib)

cargo test -p host-play --lib --features memory-profile
  → (host-play memory)

python3 docs/memory/test_run_diagnostic.py
  → (launcher gates)
```

Real queue proof (manual):

```text
cargo test -p host --lib gpu_real_queue -- --ignored --nocapture
```

Fails hard if adapter/device unavailable. Test-only poll/wait; production never
adds a completion poll.

## Overhead caveat

When enabled, each Texture paint allocates one `'static` callback closure and
touches atomics on delivery. Slot/process caps drop excess registrations rather
than growing queues. Disabled path is free. This task does **not** claim
renderer performance acceptance from unit tests; live panel runs are
orchestrator-owned.
