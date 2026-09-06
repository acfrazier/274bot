# Runner comparison: functional follow-up

The previous-build half of the reverse comparison remains unqualified. This
follow-up supplies a likely workload confounder and one successful functional
run; it does not repair that pair or establish a memory/CPU saving.

## Follow-up result

Run `diagnostics/20260906T183535Z_panel_n32_active` used the same saved
shared-stop binary as the rejected previous-build cell, 32 sustained Thiever
scripts, one renderer, 30 seconds warmup, 180 seconds observation and 60 seconds
teardown. Verbose diagnostic sidecars and navigation captures were enabled;
allocation counting, stack logging and scheduling profiling were disabled.
Grok document review and an attempted implementer startup overlapped this run.
All resource/timing values are excluded from performance acceptance.

The process exited 0 and `cpu_screen.summarize(run, False, True)` qualified it
with no errors. The sampled observation covered 178.628 seconds. All 32 bots
made progress: 8–31 additional successful steals. No observed ingame, scene-2
client left the broad workload bounds x=2600–2700, z=3200–3400. One hold sample
occurred for slot 23 at the start of observation, inside the workload area;
that slot subsequently made progress. This is not a claim that no random event
occurred anywhere in the run.

The bank-arrival capture shows slot 0 inside the bank at (2655,3286), a full
22-food inventory and the script closing the bank. Its navigation history records
arrival and a return route. The earlier reported bank-booth bouncing was not
reproduced or explained by this bounded follow-up.

## Why the earlier endpoints matter

Read-only inspection of the local server content gives exact coordinate matches:

- Slot `live14f0f_11` ended at (2008,4762). `macro_event_mime.rs2` teleports the
  player to `0_31_74_24_28` = (2008,4764), then `mime_step_right_up` moves z by -2.
- Slot `live14f0f_1` ended at (2891,4597). `macro_events.enum` contains exactly
  `0_45_71_11_53` in `macro_maze_teleports`; `start_macro_maze` selects from that
  enum and teleports the player there.

This strongly suggests server random encounters as the source of those
out-of-area endpoints. It is source-coordinate inference, not a captured event
trace from the failed run. The three aggregate readiness/activity dips are still
unattributed. The successful follow-up did not reproduce those endpoints.
Local source paths and SHA-256 hashes are retained in the companion JSON;
no server settings, content, guardian behavior or random-event policy changed.

## Disposition

Keep the earlier forward pair as a single qualified comparison and the reverse
pair as rejected evidence. Terminal runner cleanup still has a concrete native
allocation removal, but broader performance acceptance remains pending. Do not
repeat indefinitely seeking a favorable stochastic sample. First make functional
qualification return failure to automation, then continue the approved reference
work on N=16, rendering-mode proof and missing responsiveness measurements.

Evidence: [reverse pair](runner-reverse-pair.md),
[follow-up details](runner-qualification-followup.json),
[approved plan](performance-finish-plan.md).
