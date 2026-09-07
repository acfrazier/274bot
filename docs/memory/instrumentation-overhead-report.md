# Four-cell instrumentation overhead reader

`instrumentation_overhead.py` independently calls `matched_evidence_adapter.bind_side` for exactly four declared receipts in N16 TUI OFF/ON/ON/OFF order. Production callers cannot substitute bound dictionaries or an injectable binding callback. Tests patch the adapter symbol instead.

Every side must bind successfully, qualify its workload and native boundaries, and have independently validated cache and continuous process evidence. Run identities/directories must be distinct and native wall windows nonoverlapping and chronological. Frozen binary, host/server conditions, cache identity, sampler backend/modules, and workload settings (including TUI input probes) must match. Only scheduling/render/responsiveness/fine profile switches change as a group; GPU profiling stays off.

Native configuration uses the real nested schema: qualification_settings, slot_runtime_settings_by_ordinal, and renderer_config_by_ordinal. Actual and requested profile flags must agree with each cell. Only those exact flags are removed for cross-cell comparison; remaining environment flags, host/port/cache and per-ordinal runtime settings compare exactly. ON renderer rows must match each other. OFF renderer instrumentation is explicitly unavailable and cannot establish an actual backend.

| Owner | Source |
| --- | --- |
| Rust host | Independently bound samples.jsonl analysis resource counters: CPU seconds / sampled monotonic span, median/max RSS, separate process high-water mark |
| Game server | Continuous game_server role, with enclosing acquisition interval and endpoint bounds |
| Helpers | Controller (Python run_managed_cell), launcher, collector, gateway, server supervisor and any declared ambient roles |

Controller is not the Rust host. Missing host numbers cannot fall back to controller numbers. The existing resource gate may still say unavailable because overhead and raw metadata provenance are pending; its numeric fields are usable here only behind independent side/native/cache/process qualification. They are not acceptance results. No helper RSS/CPU or allocation bytes are subtracted from host measurements.

The report gives two observations per setting, observed ranges/spreads, and two adjacent ON-minus-OFF deltas (off_a to on_a, on_b to off_b). These are not confidence intervals. Host rate values are sampled-span point estimates. Helper/server rate comparisons use the reported acquisition-interval upper endpoints for display; full endpoint intervals remain in each cell's resource record.

The empirical host CPU screen uses Rust host cores only. It reports within_5pct_empirical_screen only if ONmax/OFFmin <=1.05 and both replicated spreads are <=5%. A tight spread with ONmin/OFFmax >1.05 is a regression. A range crossing the margin, excessive spread, or nonpositive denominator is inconclusive. This screen does not grant final acceptance.

`resource_deltas_available` means the four resource series can be compared. `instrumentation_overhead_measured`, final_acceptance_claim, accepted_rss_saving and pair_eligible remain false: missing OFF histograms leave latency overhead unmeasured, and continuous helper resource accounting does not measure sampler causal perturbation. Those conditions remain explicit even when resource deltas are available. TUI input writes are stimuli, not visible acknowledgments.

Validation: 29 tests cover ordering/overlap/duplicate identity, binary/config/cache/server/module changes, missing helpers or host numbers, fake measured labels, qualification failures, nested native runtime/env mismatches, actual/requested profile disagreement, controller-versus-host ownership, threshold-crossing noise, and rejection of public callback injection. The existing real N1 receipt repeated four times is rejected. The fixtures use the real nested native schema and separate controller/host numbers. No real N16 quartet has run yet.

Review history: Grok4.5 rejected the initial owner/schema/callback design. The revision worker exited without a lifecycle handoff or patch; root completed the corrections inline, retaining the review findings. Only this new reader, its tests and report changed. No Rust host or client code changed in this task.
