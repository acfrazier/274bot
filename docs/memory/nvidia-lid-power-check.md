# NVIDIA lid-state power check

Read-only native check, 2026-09-08 00:17–00:20 UTC. Operator confirmed open,
closed, then reopened lid. No bot/frontend/compiler workload ran, and no GPU
or power configuration changed during these samples. Current AC policy keeps
lid/sleep/hibernate/display timeout at0. Raw outputs and hashes are in
`diagnostics/nvidia-lid-power-20260908/manifest.json`.

| State | NVIDIA CSV graphics MHz | Reported P-state | Power W | Temperature C |
|---|---:|---|---:|---:|
| open |2085|P0|13.52|55|
| closed |345|P0|13.81|57|
| reopened |33877 (implausible)|P0|13.54|54|

All three detailed queries reported45W current/requested/default ceiling,
SW Power Cap active, hardware and software thermal slowdown inactive. Memory
clock12001MHz and Intel2560x1600/240Hz WMI values remained reported; NVIDIA
had no active display output. WMI mode values do not prove physical panel
scanout while the lid is closed.

The reopened sample reports graphics33877MHz and video21746MHz while maximum
clocks are3090MHz. Preserve this anomaly; do not interpret it as a real boost or
use these snapshots as validated clock evidence. Both CSV and detailed output
use the same NVIDIA telemetry backend and are not independent corroboration.
Even without that anomaly, isolated idle snapshots would not prove loaded
throttling. The invariant45W limit does not rule out other firmware/driver policy.

Earlier reviewed workload evidence remains: NVIDIA49.01ticks/s open,
28.16closed,48.89reopened, with closed acquire/scene-submit p99 about430/423ms;
Intel stayed near49ticks/s. See windows-adapter-clamshell-report.md. This check
neither establishes a hardware power cause nor excludes it. No power-limit,
clock, driver, or adapter-policy change is justified by these readings.

Next discriminator, if pursued: predeclare the same loaded N1 open/closed/open
workload and align validated power/event telemetry with existing acquire/submit
and callback timestamps. Keep it separate from clean memory comparisons;
telemetry observer overhead is unmeasured. A display/presentation boundary is
still a candidate explanation. No further loaded run was performed here.

NVIDIA documents the SW power-cap event and clock/P-state semantics at
https://docs.nvidia.com/deploy/nvidia-smi/index.html . That defines the field,
not the cause of this machine's observed closed-lid stalls.
