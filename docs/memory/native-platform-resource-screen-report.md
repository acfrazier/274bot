# Native platform resource screen

Date: 2026-09-07

This report is an offline read of the corrected Concord Linux N1B/N16A
receipts and the native Windows focused-one panel N1 receipt. It records
resource and cadence observations only. It does not accept a memory saving,
CPU saving, target-budget result, renderer result, or final platform
qualification.

## Evidence set and provenance

Linux evidence roots:

- `docs/memory/diagnostics/concord-native-20260907b/native-linux-n1-b/`
- `docs/memory/diagnostics/concord-native-20260907b/native-linux-n16-a/`
- `docs/memory/diagnostics/concord-native-20260907b/20260907T172143Z_tui_n1_active/`
- `docs/memory/diagnostics/concord-native-20260907b/20260907T172732Z_tui_n16_active/`

Windows evidence root:

- `docs/memory/diagnostics/windows-native-panel-b4b686f/`

The Linux N1B and N16A runs use the same frozen system-allocator binary
`f00e7fb18e28c013fc173e78d956bd4db4b819fd28a3a39b8784de71a7ed2546`, host
checkout `b4b686f`, client `3456edc`, and diagnostic flags with allocation
counting off. Concord is native Ubuntu 24.04.4 x86_64 with two logical CPUs,
1,967 MiB RAM, and no swap. The co-located server is PID 152004 and is kept as
a separately sampled role; its resource series is not combined with frontend
values.

The Linux on-host `bind_side` results for both cells report `status: bound`,
`qualified: true`, no missing match keys, and native plus managed resource
inputs available. The receipts still mean artifact-bound diagnostic evidence,
not final acceptance. The N16A predeclared 128 MiB `MemAvailable` guard with a
0.5 s poll and controller thread did not trigger.

The Windows panel is the frozen native release build from host `b4b686f`,
client `3456edc`, with binary SHA-256
`8ccdc44b5c2ebe4b3e0b7030b4087d4e5a1d55842a7dcc8e466f410e54b8d512`. The
focused-one N1 workload qualified and completed, but its top binding retained
`terminal_size: null` as a missing match key. Its managed-resource result is
unavailable because the extended native cache path did not match the ordinary
fingerprint. That failure is preserved; the separate corrective reader is not
folded into this result.

## Linux corrected scale screen

The qualification readers report the following frontend-only diagnostic
observations:

| Cell | N | Median RSS | CPU cores | Client ticks/slot/s | Steal gains |
| --- | ---: | ---: | ---: | ---: | ---: |
| native-linux-n1-b | 1 | 175,255,552 B (167.14 MiB) | 0.06651113 | 49.6703 | 10 |
| native-linux-n16-a | 16 | 640,348,160 B (610.68 MiB) | 0.72958343 | 48.3294 | 16 slots, 1–17 |

N1B qualified with a 119.575-second observation window; N16A qualified with a
119.273-second observation window. Both are one short diagnostic observations
with profiles enabled, and both exited cleanly. The N16A slot gains sum to 163
across the 16 slots. The N16A reader records `qualified: true`, no errors, and
`diagnostic_only: true`.

The requested N+15 arithmetic is:

- median RSS increase: 465,092,608 B = 443.546875 MiB;
- median RSS increment: 29.5697917 MiB per additional slot;
- CPU-core increase: 0.66307230 cores, or 0.04420482 per additional slot;
- client tick-rate change: -1.3409 ticks/slot/s overall, or -0.0894 per
  additional slot.

These are descriptive differences between two unlike N values, not a fitted
scaling law and not matched savings. The N16A reader's 512 MiB median-RSS
budget is a diagnostic miss (610.68 MiB), while its 768 MiB peak-RSS budget is
not a substitute for the median result. The N16A CPU observation also exceeds
its 0.5-core diagnostic budget. No target is accepted from either outcome.

The native host's low available RAM and the separately running PID 152004
server matter to interpretation. The server was sampled separately, and the
explicit process-role collector is not a complete descendant-tree total.
Linux current RSS is `/proc` RSS for named roles; child CPU semantics are
limited to cumulative `RUSAGE_CHILDREN` for reaped children. These values must
not be described as whole-host capacity or as application-only savings.

## Windows native panel screen

The focused-one panel N1 qualification reader reports:

- observation: 119.155 s;
- frontend median RSS: 507,463,680 B;
- recorded CPU: 0.00537638 cores, derived from 0.640625 process CPU seconds;
- client ticks: 16.6086 per slot/s;
- qualified: true, 14 steal gains, clean exit;
- terminal: false, `terminal_size: null`;
- native GPU callback completion: 1,979 registered and 1,979 completed,
  lossless, with no pending end count;
- callback-completion p99 bucket: 50–100 ms;
- observed callback cadence: 16.6086 fps against the 40 fps target, a miss.

The GPU counters are callback-delivery observations, not hardware scanout or
physical-display presentation. The receipt explicitly says that the callback
is CPU delivery after a prior submit and does not establish GPU hardware or
scanout cadence. The low reported CPU value is retained as recorded process
counter output; it is not combined with the Linux server or treated as a
trusted cross-platform comparison without validating the recorded counters
against source semantics.

The raw capture set was read directly from the repository. The scene-ready
capture shows the courtyard at tile 2661,3305 with four food items; the
bank-arrival capture shows tile 2655,3286 with 22 food and `modal-1`; the
return capture shows tile 2649,3284 with one script bank trip. The captures
show the game, minimap, inventory, and script/status surfaces. No capture
shows an open bank modal, a freeze, CPU fallback, or a physical-display
condition. These observations are root evidence, not a claim of model image
interpretation.

The Windows run does not establish desktop foreground, RDP state, adapter
identity, or physical scanout. Those conditions remain unproven. The managed
resource unavailability caused by the extended cache path versus ordinary
fingerprint must remain visible in later reporting.

## Interpretation and next diagnostic

The Linux corrected binding and qualification evidence is sufficient for a
bounded diagnostic scale screen. It is not sufficient to infer a per-slot
memory target or accepted saving. The N16 result is above the diagnostic
median-RSS and CPU budgets, but the budgets are not converted into a final
capacity claim.

The next diagnostic should target the Windows focused cadence miss with one
bounded investigation of cadence semantics and host presentation conditions:
retain the same focused-one workload and instrument the relationship between
mainredraw, callback delivery, and the actual window/adapter presentation
endpoint, while recording foreground/RDP/adapter identity explicitly. Do not
blindly expand the N matrix or repeat the same run before resolving whether
the 16.6 fps observation is callback cadence, a scheduling limitation, or a
presentation-path condition.

## Source receipts

Linux: `native-linux-n1-b/independent-binding.json`,
`native-linux-n1-b/cells/native-linux-n1-b/cell_report.json`,
`native-linux-n16-a/independent-binding.json`,
`native-linux-n16-a/cells/native-linux-n16-a/cell_report.json`, and both
run `metadata.json`/qualification samples.

Windows: `managed-panel-b4b686f-a/independent-binding.json`,
`managed-panel-b4b686f-a/cells/native-panel-n1-a/cell_report.json`,
`managed-panel-b4b686f-a/20260907T173738Z_panel_n1_active/metadata.json`,
and the three navigation captures under that run.
