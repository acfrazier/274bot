# Native Windows panel presentation wait and worker cadence

## Scope and conclusion

This is a read-only causal analysis of the completed Windows native panel run
`20260907T182109Z_panel_n1_active` (candidate build `a3f729d`, host base
`e3188e2`, client `3456edc`). It does not establish a performance pass, a
physical scanout rate, or an RDP cause.

The evidence supports this bounded explanation:

- The panel UI thread is running a 50 Hz `WaitUntil` redraw policy
  (`crates/panel/src/app.rs:58-80`), but each redraw blocks in
  `surface.get_current_texture()` until FIFO makes an image available
  (`crates/panel/src/window.rs:710-729`). With Vulkan/FIFO and
  `desired_maximum_frame_latency = 2`, the observed blocking point is the
  swapchain acquire, not GUI work, command encoding, or `frame.present()`.
- The host worker is a separate per-slot thread. Its normal active loop runs
  `client_tick`, then sleeps the remainder of a fixed 20 ms frame budget
  (`crates/host/src/lib.rs:253-286`). Its observed ~17 Hz is therefore a
  missed/deferred 50 Hz loop, not a deliberate 17 Hz clamp in the Rust host.
- The most plausible coupling is scheduler/resource contention between the
  panel/event-loop thread and the slot thread while the panel waits for a FIFO
  image and the client produces GPU-backed frames. The run proves where the
  panel thread waited; it does not prove whether the underlying wait was
  compositor, driver queue, desktop-session presentation, or another GPU
  scheduling condition.

The minimal next experiment is a matched native-console versus RDP comparison
with the same binary, server, policy, workload, dimensions, present mode,
latency, and diagnostics. Change only the desktop session. A backend comparison
(Vulkan versus the already-supported alternative) is a secondary experiment,
not a substitute for the session comparison.

## Direct evidence

The production-only diagnostic report says its durations are CPU wall-clock
spans around GUI/WGPU calls, not GPU timestamps, compositor time, scanout, or
proof of RDP causality (`windows-panel-stage-attribution-report.md:44-57`).
The native startup record additionally reports:

- NVIDIA GeForce RTX 5060 Laptop GPU, Vulkan, NVIDIA 616.56;
- logical 1120x580, physical 2240x1160, scale 2;
- FIFO, maximum frame latency 2;
- focused/visible true and minimized false at initialization;
- no offscreen target and no frame errors.

The interval records contain 118 completed samples over the qualified observe
window. The characteristic steady values are approximately:

- swapchain acquire: median 30.942 ms;
- GUI preparation: 0.01695 ms;
- UI body: 0.1291 ms;
- command encode/submit: 0.2347 ms;
- `frame.present()`: 0.0751 ms.

The acquire interval is therefore about two orders of magnitude larger than
CPU-side UI preparation and command submission, and it is the only measured
stage with a duration near the ~31 ms redraw period. The sample at
`run.log:20-21` that includes a 19.8883 ms screenshot readback is an explicit
capture event and is not representative of ordinary frames.

The qualification evidence reports 118 samples across 118.9709663 s,
`client_ticks_per_slot_per_s` about 16.9873, CPU about 0.004071 cores, RSS
507109376 bytes, and 11 steals. The client runtime row shows the slot remained
ingame/scene 2, draw true, GPU backend, and full-rate true at both observe
boundaries (`samples.qualification.jsonl:1-2`). The run exited 0 and has no
binding errors (`cell_report.json:75-139`).

The GPU completion profile is present and is correctly interpreted as a CPU
callback-delivery measurement. The implementation registers
`queue.on_submitted_work_done` after `mainredraw` and before mailbox ownership
moves (`crates/host/src/lib.rs:587-617`); the profile explicitly says callback
time is CPU delivery after prior GPU work, delivered on a later existing
submit/poll, not hardware completion or scanout (`crates/host/src/render_profile.rs:13-28`). Thus callback cadence can corroborate queue progress, but cannot be read as
physical display cadence. The supplied evidence qualifies the 118 completed
GPU-callback samples; it does not contain a full frame-duration distribution.

## Causal path

### Panel thread

`runner_config()` selects `RedrawMode::WaitUntil { fps: 50.0 }`. In the event
loop, `about_to_wait` schedules the next redraw deadline and
`RedrawRequested` runs the complete panel render synchronously. The order is:

1. winit/ImGui frame preparation and UI body;
2. `get_current_texture()`;
3. render-pass encoding and queue submission;
4. `frame.present()`;
5. optional existing screenshot map/poll work.

The attribution timing brackets this exact order. The source intentionally
acquires the swapchain image late (`window.rs:710-729`), so an acquire wait
holds the event-loop thread before command encoding. FIFO is a pacing mechanism:
when the allowed surface images are not available, the next acquire waits for
availability. A 30.9 ms CPU span around acquire is evidence of that thread
being blocked there; it is not evidence that the ImGui body or Rust server
worker spent 30.9 ms computing.

The explicit screenshot readback is different: `map_readbacks()` calls
`device.poll(PollType::Wait)` (`window.rs:893-925`) after a requested capture.
That path explains the isolated readback sample, but not the ordinary acquire
median because readback is normally absent and the sample's readback field is
near zero.

### Host worker and client

The host has one OS thread per slot (`crates/host/src/lib.rs:128-170`). During
an active frame it calls `observe`, drains input, executes one `client.mainloop`,
possibly runs `mainredraw`, stores the resulting `FrameOutput`, and sleeps the
remaining portion of the 20 ms budget (`lib.rs:371-406`, `lib.rs:465-505`,
`lib.rs:581-620`). The sleep is after work; an overrun skips the sleep.

The client itself also has a Java-compatible 20 ms loop when driven by its
normal run path (`vendor/fr-client-rust/crates/client/src/client/client.rs:
10459-10518`), but the production host path is the direct `Host::client_frame`
path described above. `TexturesTarget::present` is only a newest-frame handoff
(`vendor/fr-client-rust/crates/client/src/client/present.rs:117-145`); it does
not wait for the panel surface.

Consequently, a slow panel acquire cannot synchronously block a slot at the
Rust mailbox API: the slot stores its frame and continues. It can still contend
indirectly for CPU/GPU scheduling, and a GPU queue callback may be delivered
later because callbacks are delivered by existing queue progress/polling rather
than by a new wait. That indirect coupling is consistent with the observations,
but this run did not capture per-thread scheduler states or hardware queue
timestamps, so it remains a hypothesis rather than a proven root cause.

### Why approximately 17 Hz beside approximately 32 Hz UI

The panel's 32 Hz completed-frame rate is consistent with FIFO acquire pacing
near a display/compositor interval around 31 ms. The worker's approximately
17 Hz rate is consistent with its 20 ms loop being delayed by roughly 40 ms on
average, or by wake/scheduling gaps that produce the measured average. The host
contains no 17 Hz constant or clamp in this path. Its idle park bounds (200 ms,
600 ms, or 1 s) are not active here because the slot is full-rate and busy; the
active branch is the fixed 20 ms cadence (`lib.rs:625-652`, `lib.rs:194-205`).

The current evidence cannot distinguish these two possible contributors:

1. CPU scheduler starvation/preemption of the worker while the panel thread is
   active and the GPU/desktop stack is servicing FIFO presentation;
2. GPU/driver queue pressure causing worker-side GPU work or callback progress
   to delay enough that the 20 ms loop misses wake deadlines.

The attribution timings rule out large UI preparation, UI body, command-submit,
or `present()` CPU spans as the direct cause. They do not rule out work below
those API calls or OS scheduling around them.

## Answers to the requested questions

### Does Rust server/host polling clamp the worker to ~17 Hz?

No direct clamp is present. Host socket polling is used only in the idle branch
and is bounded by readable server/control events or a timeout. In the active
full-rate branch, the slot does not call `poll`; it runs the fixed 20 ms loop.
The server is local in this run (`127.0.0.1`), so network round-trip is not a
credible explanation for the panel acquire wait. The worker cadence miss is a
scheduler/loop-timing observation, not proof of server polling throttling.

### Does GPU readback/presentation couple worker timing?

The ordinary panel path does not perform a readback. The client GPU frame is
handed through the mailbox, the panel binds it, and the panel renders its own
ImGui surface. The only deliberate blocking readback is the existing screenshot
path, and its isolated 19.8883 ms sample is visible in the diagnostic log.
Therefore readback explains that capture outlier, not the persistent acquire
median.

A GPU/desktop presentation coupling remains plausible indirectly: FIFO acquire
waits on the panel thread, while the same process and adapter service the
client's GPU work and completion callbacks. The run does not prove that this
indirect competition is the worker's sole cause. There is no evidence here of a
Rust API lock or mailbox backpressure that would directly stop the worker.

### Is RDP established as the cause?

No. Initial panel focus/visibility/minimized state was recorded before the run,
but it does not identify the desktop session or prove focus throughout the
observe interval. The only supplied `currentRemoteSession=true` probe was taken
after completion while the observer itself was foreground. Both Austen console
1 and BotTest RDP 2 were active. This is insufficient to attribute the FIFO
wait or worker cadence to RDP.

## Ranked hypotheses and discriminating tests

1. Desktop-session/compositor presentation behavior (including RDP) changes
   FIFO image availability. Prediction: with all binaries and settings fixed,
   native console and RDP show materially different acquire distributions and
   completed UI cadence. Test: paired console/RDP runs, one session variable at
   a time.
2. Vulkan/NVIDIA surface path has a presentation pacing behavior independent of
   RDP. Prediction: a matched alternate backend changes acquire wait and UI
   cadence while worker cadence changes correspondingly or remains unchanged.
   Test only after the session pair, preserving policy and fidelity.
3. OS scheduler contention between panel/event-loop and slot worker. Prediction:
   worker start-to-start intervals show long gaps aligned with acquire waits,
   while a pinned/isolated diagnostic run changes the worker tail without
   changing render-stage costs. This requires thread-level scheduling evidence;
   do not infer it from averages.
4. GPU queue work/callback servicing delays the worker. Prediction: callback
   delivery intervals and pending ages show the same gaps as worker intervals,
   and CPU/GPU queue instrumentation identifies a causal ordering. The current
   callback counters only show delivery cadence, not this ordering.

Do not change present mode, maximum latency, redraw policy, renderer fidelity,
window protocol, or viewer geometry during the discriminating run. Keep the
existing diagnostics enabled and add only timestamps needed to align worker
start, panel acquire entry/exit, callback delivery, and panel completion.

## Limitations and next experiment

This report does not claim a physical 32 Hz scanout rate: attribution is CPU API
wall time and GPU callback delivery is CPU-side. It also does not claim that the
118 last-completed-frame stage samples describe the full distribution of frame
durations. No RDP causality, GPU hardware timestamp, compositor wait reason,
thread-priority trace, or aligned per-event cross-thread timeline was captured.

Run exactly one matched pair next:

- native console session, then RDP session (or the reverse, with order recorded);
- same frozen `panel-a3f729d` binary, server fixture, seed, render policy,
  1120x580 logical window, FIFO, latency 2, active workload, warmup/observe,
  and diagnostic flags;
- collect per-thread worker tick starts, panel acquire entry/exit, panel frame
  completion, and GPU callback delivery in one monotonic timestamp domain;
- retain the existing callback semantics and do not add a wait/poll solely to
  make callbacks arrive;
- compare acquire median/tails, completed UI rate, worker start-to-start
  intervals, callback delivery intervals, pending/lost counts, and frame
  qualification separately.

If session identity does not explain the difference, perform the same pair with
only the backend changed. Until then, the safe attribution is: **persistent
FIFO swapchain-acquire waiting is the direct panel-thread bottleneck; the
worker's ~17 Hz is an active-loop scheduling miss whose precise coupling to
presentation is not yet proven.**
