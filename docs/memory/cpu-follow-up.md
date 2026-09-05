# CPU follow-up from the door stability run

The operator requested a CPU sidebar because fleet cost may limit scaling. This is initial profiling evidence, not an accepted CPU baseline or an optimization proposal with promised savings.

During panel32, one renderer/31 sim clients, macOS ps reported270.8% then254.6% process CPU (about2.5–2.7 CPU cores). Five seconds of sample at1ms saved in diagnostics/20260905T191752Z_panel_n32_active/cpu-sample.txt. Captures, memory diagnostics, allocation counting and earlier concurrent Grok review mean this is an instrumented diagnostic.

Strong leads:

- AnimFrame::get holds a process-wide mutex while cloning an AnimFrame, including multiple vectors and its AnimBase. Sample stacks attributed72205 mutex-wait samples to this lookup. Callers include SeqType frame timing and Model animation. This explains why simulation clients can contend on animation data even when31 clients do not render. It does not imply that those clients perform full3D rendering.
- CountingAllocator::alloc/dealloc were prominent active top-of-stack symbols (1132/668 samples). The measurement wrapper updates shared atomic counters on every allocation/free. Separate this overhead from production CPU before judging throughput or redesigning the allocator.
- The memory harness client_frame path accounted for11296 mutex-wait samples. Its diagnostics and seed hooks use shared locks; attribute those with finer scope measurements before changing them.

These counts include sleeping/waiting threads and cannot be read as CPU percentages. Existing client_tick timers measure elapsed wall duration, not thread CPU: a30.29-second interval showed1284 client ticks/s and18.36ms mean duration, whose sum exceeds actual CPU usage because concurrent waits overlap. Script ticks averaged0.163ms and UI frame construction4.04ms in that interval, also wall time and incomplete descriptions of total cost.

Proposed bounded follow-up after landing navigation stability:

1. Keep identical accounts/workload and one-renderer policy. Compare runs with full instrumentation, allocation counting disabled, and diagnostic sidecars disabled independently. Preserve readiness, script progress, and action traces; record process CPU seconds, elapsed seconds, per-slot throughput and memory separately. Do not compare differently configured runs as if only an application optimization changed.
2. Attribute animation lookup calls and lock wait/hold time in simulation versus rendering. Evaluate immutable shared frame/base storage while preserving existing public ownership/mutation semantics, lazy archive publication, invalid-id behavior and multi-client initialization. Do not replace the lock blindly: unpack/init still publish data.
3. Isolate any proposed change in its own client commit, run client integration tests separately, and measure the same fixed cells before considering broader renderer/scheduler work.

No CPU code was changed in this sidebar.

## Operator-approved measurement order

On 2026-09-05 the operator agreed to defer the full 128-bot benchmark until after the identified waste and instrumentation overhead are addressed at 1 and 32 bots. Finish and commit the current stability correction, separate measurement overhead from application cost, then optimize measured problems in small commits. Return to 128 for scaling validation afterward. A short capacity check is optional if a specific threshold question requires it. No 128-slot acceptance is claimed; the original eventual scaling requirement remains outstanding.
