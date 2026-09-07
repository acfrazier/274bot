# Native platform latency evidence audit

Date: 2026-09-07

Scope: bounded offline audit of the first workload-qualified native diagnostic-only
TUI N1 runs. This report reads the preserved raw diagnostics and the existing
`reference_metrics.py` adapter. It does not alter source, rerun a cell, or grant
performance acceptance.

## Evidence corpus and interpretation

| Platform | Raw diagnostic | Transport | Observe rows | Observe span |
|---|---|---|---:|---:|
| Windows | `docs/memory/diagnostics/windows-native-conpty-b4b686f/20260907T164915Z_tui_n1_active/` | ConPTY | 119 | 119.355650 s (elapsed 45.2515602–164.6072099) |
| Linux | `docs/memory/diagnostics/concord-native-20260907a/20260907T165530Z_tui_n1_active/` | Unix PTY | 120 | 119.575415 s (elapsed 53.561516086–173.136931492) |

Both metadata files request scheduling and responsiveness/fine profiles. The
Windows metadata has `render_profile: false`; Linux has `render_profile: true`,
but both have `render_policy: none`, `gpu_completion_profile: false`, and no
GPU completion evidence. Both input-probe metadata entries explicitly say
`terminal write only; use native input/draw counters for latency`.

The existing independent reader was run with `--inspect` on each raw directory.
Its result is diagnostic adapter output only (`final_acceptance_claim: false`).
The reader uses native process-monotonic capture/read brackets and counter/histogram
conservation; it does not use JSONL row-time deltas as latency endpoints.

## Native input / acknowledgement coverage

The raw producer rows do contain actual TUI input-origin and completion counters,
not merely terminal writes:

| Platform | Probe writes | Native start delta | Native complete delta | Cancel/lost/drop/pending at final row | Coarse p99 upper | Fine p99 upper |
|---|---:|---:|---:|---|---:|---:|
| Windows | 119 | 119 | 119 | 0 / 0 / 0 / 0 | 5 ms | 2 ms |
| Linux | 120 | 120 | 120 | 0 / 0 / 0 / 0 | 5 ms | 2 ms |

The reader's contained observe spans exclude one leading boundary sample on each
run and return available input results with 117 samples (Windows) and 118 samples
(Linux), respectively. Each selected span has start=complete, zero canceled/lost/
dropped delta, zero pending at both boundaries, conserved coarse/fine histograms,
`input_coverage_complete: true`, `visible_ack.available: true`, and
`visible_ack.endpoint: terminal_draw_flush_after_state_mutating_key`.
The adapter reports a diagnostic target verdict of `meet` against its 100 ms
input target on both runs, but this is not a campaign performance result.

The probe write records remain stimuli, not acknowledgements. The one-write
Windows/Linux difference between writes and selected/native observe-boundary
counts is not evidence of loss: writes are outside the first/last observe envelope
and native start/complete counters are the authoritative accounting. The TUI
endpoint is a successful `terminal.draw` flush after input handling, not terminal
emulator paint, OS compositor scanout, or a physical display acknowledgement.

## Decode coverage and latency

The raw rows contain decode starts/dispatches and latency histograms, but the
selected native spans are not coverage-complete:

| Platform | Decode edge | Dispatch | Canceled | Unmatched canceled | Lost | Dropped | Pending | Coarse p99 upper | Fine p99 upper |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Windows | 269 | 254 | 15 | 1 | 0 | 0 | 1 | 25 ms | 23 ms |
| Linux | 283 | 273 | 11 | 1 | 0 | 0 | 0 | 25 ms | 24 ms |

The offline reader therefore reports Windows decode unavailable for
`boundary_pending_incomplete`; Linux has conserved bounded histograms and
available diagnostic p99 arithmetic, but `decode_coverage_complete` remains false
because the raw cumulative row records cancellation(s). These rows cannot support
an accepted decode-latency or end-to-end input/decode claim. Decode means
`PLAYER_INFO_after_drain_to_on_game_tick_entry`, not packet-decode wall time.

## Per-slot scheduling evidence

The per-slot scheduling adapter finds one expected slot/generation on each N1
run and uses counter deltas plus native endpoint stamps:

| Platform | Interval samples | Cycle delta | Measured duration | Observed iterations/s | p99 bound | Sample age (final) | Overflow bucket | Coverage |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| Windows | 5,800 | 5,800 | 119,194 ms | 48.6602 | 20–21 ms | 831 ms | 0 | complete |
| Linux | 5,950 | 5,950 | 119,787 ms | 49.6715 | 20–21 ms | 62 ms | 0 | complete |

The per-slot rows have `ended: false`, `ended_lost_n: 0`, `mode_break_n: 0`,
`anchor_miss_n: 0` in the selected spans, and `scene_transition_separation:
unavailable`. The raw final snapshots also show one park and two anchor misses
across the lifetime, but those do not enter the selected observe deltas. The
reader's per-slot target verdict is `meet` against its 40 ms interval target and
40 iterations/s floor. This means the contained diagnostic interval bounds are
compatible with that target; it is not a completed-frame, GPU, scanout, or final
responsiveness acceptance result.

The process-wide legacy scheduling groups also report roughly 5,800/5,950 samples
and a 20–21 ms diagnostic bound, but their exact observation coverage is
`inconclusive_process_wide_batch_lag`. They must not be used as a per-slot proof;
the per-slot result above is the relevant reader output. `updated_ms` and
`sample_age_ms` describe registry publication freshness, not interval endpoints.

## Staleness, pending, overflow, and target feasibility

- Responsiveness publication freshness is acceptable at the final rows (10 ms
  Windows, 20 ms Linux); the selected input spans contain native process-monotonic
  clocks and no mixed clock-domain rows. The scheduling final rows are 831 ms and
  62 ms old, below the serializer's 1,000 ms flush-lag bound. These are freshness
  bounds, not latency samples.
- Input has no pending, canceled, lost, or dropped events in either selected span;
  its coarse and fine histogram overflow counts are zero. The raw final input
  counters are likewise zero for pending/canceled/lost/dropped.
- Decode has no drops or lost events, but cancellation and/or pending state makes
  full decode coverage unavailable. Its displayed p99 bounds are descriptive only.
- Scheduling interval histograms have zero overflow in both final per-slot rows.
  The per-slot interval coverage identity is complete for the selected deltas,
  while scene-transition separation remains unavailable.
- Absolute-target feasibility is therefore split: scheduling has a contained
  per-slot diagnostic bound of 20–21 ms and is feasible against the 40 ms target;
  focused TUI input has a contained diagnostic bound of 0–5 ms coarse / 1–2 ms
  fine and is feasible against the 100 ms reader target; decode cannot establish
  complete coverage; GPU/renderer feasibility is unavailable. None of these
  constitutes acceptance.

## What this evidence does and does not establish

Established separately from terminal-write-only probe metadata:

1. Both raw diagnostics contain native TUI producer-origin/start and draw-flush
   completion rows for the exercised focused slot.
2. Native input start/complete accounting is conserved and loss-free over the
   selected contained spans.
3. Native per-slot scheduling intervals are present, bounded, fresh enough for the
   adapter, and non-overflowing for the selected N1 spans.
4. Decode rows and p99 histograms exist, but cancellation/pending coverage prevents
   treating them as complete latency evidence.

Not established:

- a terminal emulator, compositor, display, GPU, or scanout acknowledgement;
- input coverage for non-focused slots or a full N16 rotation;
- packet-decode-to-script end-to-end latency;
- a matched Windows/Linux comparison (the runs have distinct platform/runtime
  environments, side metadata omissions, and no matched performance design);
- accepted CPU/RSS/latency/resource budgets, renderer completion, or final target
  capacity.

The top-level native functional report's missing server `port_listen` and
configuration metadata remains a separate binding omission. It is not repaired by
these raw producer rows. The two runs remain diagnostic-only, with no performance
acceptance.

## Sources and commands

- `docs/memory/native-platform-functional-report.md`
- `docs/memory/responsiveness-measurement-report.md`
- `docs/memory/per-slot-scheduling-report.md`
- `docs/memory/reference_metrics.py`
- The two raw directories listed above (`metadata.json`, `samples.jsonl`,
  `input-probes.jsonl`)
- `python3 docs/memory/reference_metrics.py <run-dir> --inspect` (exit 0 for both;
  both reported `final_acceptance_claim: false`)
