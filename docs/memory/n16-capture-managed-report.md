# Managed N16 capture diagnostic

The single run `20260907T041253Z_tui_n16_active` completed successfully, including
teardown. Both the frontend metadata and managed launcher report exit 0. This is
functional diagnostic evidence, not a matched performance result or a fix for the
earlier intermittent bank-route failure.

## Build and run

- Native source matches Grok-reviewed `864ae3e`; build checkout was `ee09dd7`
  (later Python changes do not alter the native source). Client: `451759f2`.
- System-allocator release binary SHA-256:
  `efc1679e58a109a017c4783a196cc9a046fb01c1012ec4f25fb0c071ad8187bd`.
- Build receipt: `diagnostics/n16-capture-build-20260907T040110Z/build-receipt.json`.
  Source hashes were stable across build; old frozen binaries were unchanged.
- TUI, 16 active sustained Thievers, 30-second warmup, 300-second observation,
  existing 60-second teardown; 900-second outer deadline. One attempt, no retry.
- Failure capture, navigation capture, responsiveness and fine histograms were
  enabled. Periodic diagnostics were enabled; allocation counting was off.
- Workers were parked before launch; the frontend has exited. The deadline was
  not exceeded. Managed wall time was 473.423 seconds, including startup/seeding.

## Completed evidence

The harness's observation boundaries span **300.029 seconds**. The 294 ordinary
observation samples span **299.389 seconds**; these are different timestamp cuts.
All 16 clients were ingame with scene state 2 at observation start. The existing
workload qualifier returned `qualified=true`, no errors; every slot gained steals
(14–40). All 16 were observed with `bank_open=true`, and each final paint reported
one bank trip. One slot had an observed guardian hold; this alone does not prove
the same interruption as the earlier failure.

Three TUI data-only files were written for watched slot `live1cfa0_0`:

| Checkpoint | Native snapshot tick |
|---|---:|
| scene-ready | 14 |
| bank-arrival | 210 |
| return-route-start | 217 |

Each contains the native checkpoint snapshot. There was no nav-failure or
failure-boundary record. Thus this run exercises enable/drain and milestone
capture, but does **not** provide live proof of the error-path history output.

The final teardown sample reports active scripts 0, live V8 isolates 0, and
in-flight snapshot bytes/capacity 0. This short teardown is not a one-hour
lifecycle qualification or an RSS plateau result.

## Responsiveness and limits

The approved analyzer from `864ae3e` was loaded separately, so the unfinished
artifact-reader changes were not used for this diagnostic assessment. Its source
hash and qualifier hash are recorded in `analyzer-provenance.json`.

- Decode-to-script: all 16 slots available; conservative fine p99 upper bounds
  range **48–60 ms**, meeting the existing 100 ms diagnostic gate.
- Selected-window counters total 7,952 edges and 7,952 dispatches, with zero
  canceled, lost, dropped, pending-delta or unmatched-canceled events. The analyzer
  checked native clock brackets and histogram/counter conservation.
- Input: unavailable, no input samples. Scheduling and GPU-completion profiles
  were disabled. No paired 2 ms claim is made.
- Resources: unavailable for acceptance because provenance and overhead evidence
  remain incomplete. This instrumented run is not a clean CPU/RSS comparison.

The earlier managed N16 failure remains valid evidence. No route retry, guardian
policy, fixture, timeout or shader behavior was changed to obtain this pass.

## Artifacts

Under `diagnostics/n16-capture-managed-20260907T041253Z/`:
`launch.json`, `completion.json`, `qualification.json`,
`metrics-approved-analyzer.json`, `analyzer-provenance.json`, and
`functional-summary.json` (post-run raw/capture hashes and per-slot bank evidence).
Raw files and captures remain in `diagnostics/20260907T041253Z_tui_n16_active/`.

The artifact reader, measured overhead, matched/repeated resource results,
absolute-budget matrix, long lifecycle runs and whole-branch Grok-4.6 review
remain open.
