Borrowed fingerprint corrected native Linux short screen

Status

The corrected batch 20260907b was stopped after index 2, exactly as predeclared. Index 1 (reference, N=16) independently bound and qualified. Index 2 (candidate, N=16) launched once, failed during qualification, and returned exit 1. Indices 3–8 were not run. No retry, relabel, fixture change, timeout change, server restart, build, or unrelated-service intervention was performed.

Runner and provenance

- Runner: /home/acfrazier/274bot-campaign/borrowed-fingerprint-native-screen-20260907b/run_corrected_cell.py
- Runner SHA-256: 112e64886dfbc5e2008d9e967d74fecdee3e410676ccdee6a55509aceae8bbbf
- Reference binary SHA-256: 019c35779271f150bb83fb7cf6fcb755a4abee8abd1baa2378dcf9e76c932b76
- Candidate binary SHA-256: 10ddc915c7b0f5ef9f3fbb9443c95bc74249303d9a048fcdc4fb4e4fc69af008
- Both cells used client source proof 98714ee076d808e4589bedb3e3c16af1005e98c6bc54a28e1087c0dfcf73f865 and host source snapshot 763700b6c3f7f92e617caa0cf052eaccfab05b56b2c840b068f519de27dd7098.
- Existing server identity remained PID 152004, Linux start ticks 241214967, listener 43594. Server configuration and fixture hashes were unchanged in both cell receipts.
- Host was Ubuntu 24.04.4 x86_64, 2 vCPU, 2014852 kB RAM, no swap. MemAvailable was 851584 kB at reference setup and 829632 kB at candidate setup, both above the 128 MiB managed failure guard.

Reference N=16

The independent-binding record is status bound, binding_ok true, qualified true, missing required provenance null, and exit code 0. The 120-second observation produced 117 native resource samples; all 16 slots were ready and active throughout the observation (min/median/max 16/16/16). Observation qualification had 16/16 Running slots at both boundaries, no slot errors, ingame=true and scene_state=2 at the end.

Native managed metrics:

- Median resident RSS: 622731264 bytes (593.75 MiB).
- Sampled peak resident RSS: 631926784 bytes (602.52 MiB).
- Process CPU: 84.539428 seconds over 119.275152 seconds, 0.7087765 CPU cores per observation wall second.
- Per-slot progress at the end: dispatched counts 362–375, last completed ticks 377–388, and steal gains 2–16.
- Server boundary sample: RSS 665493504 bytes, user CPU 481.44 s, system CPU 23.57 s.
- Managed sampler: 479 samples over 238.998839 s. Median RSS by role was server 654352384, controller 20148224, SSH session 3407872, launcher 20000768, collector 16646144 bytes.
- Sampling overhead was explicitly unmeasured; the run is diagnostic only.

Candidate N=16 failure

Index 2 used the candidate binary and the same predeclared settings. It failed at 17.386014 seconds with the captured failure `live256c5_12: tick 19: Unknown error`. The receipt records `runner:unexpected qualification phase`, `runner:missing observation boundary`, `frontend_failed_or_incomplete`, and `launcher_failed`; launcher exit code was 1. The failure-boundary record contained 16 slots but mixed Idle/Running states and no valid observation boundary. Consequently, candidate RSS, CPU, ready/active/progress, server-series, and per-slot metrics are not reportable as qualified measurements. The candidate had zero observation samples.

Raw evidence preservation

The complete corrected batch directory, controller stdout files, cell receipts, preflight files, failure logs, process-accounting files, and both referenced run directories were copied to:

docs/memory/diagnostics/borrowed-fingerprint-native-screen-20260907b/

SHA-256 verification found zero mismatches for all 59 remote batch files and zero mismatches in both referenced run directories. The remote raw files were copied, not moved, and remain preserved on concord. The failed prior batch 20260907 was not used.

Interpretation

No pair comparison, CPU 5% margin check, RSS-variation check, N=1 screen, latency companion, or final acceptance is possible. The one qualified reference cell is retained as evidence only. The candidate failure must be diagnosed before any further corrected screen; do not average or compare it as a performance result.

Deliverables

- docs/memory/diagnostics/borrowed-fingerprint-corrected-native-screen-report.md
- docs/memory/diagnostics/borrowed-fingerprint-corrected-native-screen-table.json
