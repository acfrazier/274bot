# Current finish-line gap audit

Updated: 2026-09-08 15:19 UTC. Branch `codex/memory-diagnostics`.
This is a bounded evidence synthesis, not acceptance or a target change. Approved
budgets remain performance-finish-plan.md. STATE.md is the execution authority.

## Current boundary

The1430 fixed-window Windows N16 focused-one pair is complete. Root results in
windows-cohort-pair-1430-report.md/evidence.json are under independent review
(t_4af4c884, actual Grok4.5 run762). Both roles qualify16/16 and close16000 decode
events without losses; aggregate fine p99 is24–25ms each. Candidate slot0 closes
120 input events without losses, fine p9941–42ms. Baseline input is unavailable:
root missed the predeclared60–90s trigger window, spawned no input and did not
repeat. No matched input or overall latency acceptance follows.

Current profiled frontend medians are662515712/687902720 bytes (reference/
candidate), lifetime peaks1062477824/986865664 bytes and CPU0.525871/0.526170
cores. They are diagnostics, not clean matched savings or final budgets:
stimulation is asymmetric, baseline UI inspection overlaps observation, overhead
is unmeasured and the generic match keys differ. Native receipt/build/cache/
runtime/process accounting binds independently; raw resource gate stays
unavailable. See the pair report for exact endpoints, source versions and hashes.

The earlier synchronized0830 visual proof is reviewed qualitative held-scene/
minimap/chat and2→1→2 recovery evidence (0c1e438+b1dd88f, t_9b57233d). It does not
close fullG1/G4, splash/modal/cadence or physical input timing requirements.

The low-end Linux TUI figures below still use old b4/f00e7fb source lineage.
Current source has changed materially since those cells. Task t_c2879587 is a
bounded design/provenance audit for one current-source native TUI N16 calibration
screen, followed by same-card review. No new build or live run is released yet.
This addresses stale source lineage; it does not reopen parked comparisons or
make their source performance-qualified. Final36-cell matrix,32/lifecycle,
gated128 and whole-campaign Grok4.6 requirements remain unresolved.

## Historical requirement inventory — 2026-09-08 08:22 UTC

The inventory below is retained as an evidence snapshot. The current boundary
above supersedes its Windows cohort and synchronized-controller next actions;
older native hardware numbers are not current source measurements.

Status vocabulary: `observed` means a real number exists but is not acceptance;
`diagnostic meet` means the contained reader bound is compatible with a target;
`pending` means the gate is not covered or its evidence is invalid/incomplete;
`parked` means the candidate is not retained as a saving.

| Approved requirement | Target | Current result / delta | Evidence scope and status |
|---|---:|---|---|
| TUI N1 steady median RSS | <=256 MiB | 167.14 MiB observed; -88.86 MiB | Linux native N1B, `native-platform-resource-screen-report.md` (2026-09-07), frozen `f00e7fb…`/host `b4b686f`; short diagnostic, not final acceptance. |
| TUI N1 startup/transition peak RSS | <=384 MiB | Not reported as a matched final peak | N1B reports median only; pending three-fresh final cells. |
| TUI N16 steady median RSS | <=512 MiB | 610.68 MiB; +98.68 MiB | Linux N16A, same report and frozen binary; one ~119 s diagnostic cell, target miss. |
| TUI N16 startup/transition peak RSS | <=768 MiB | No accepted peak result | N16A short diagnostic does not replace final peak protocol; pending. |
| Panel N1 steady median RSS, one renderer | <=384 MiB | 507,463,680 B = 484.0 MiB; +100.0 MiB | Windows focused-one N1; binding retains missing `terminal_size`, managed resource unavailable; not acceptance. |
| Panel N1 peak RSS | <=512 MiB | Not reliable/currently qualified | Existing N1 screen has no accepted startup/transition peak; pending. |
| Panel N16 steady median RSS, focused one | <=768 MiB | 569.260 MiB whole; 568.713 MiB common | Current longer focused-one pair, `37aca9a`, archive `f651fbda…`; `binding_ok=true`, 16/16 qualified, exit 0. Descriptive diagnostic only; parked RSS claim (+0.592% common, +8.02% final-300). Early frozen short `bb2de5d6` 942.324 MiB remains historical, not current primary. |
| Panel N16 peak RSS, focused one | <=1 GiB | No accepted matched peak result | `37aca9a` long report separates RSS max/peak fields but does not close final peak protocol; pending. |
| Panel N16 steady median RSS, focused + background | <=768 MiB | No current matched long result; early 1,893.871 MiB | Early frozen short `7fe09f3f…` had all 16 GPU backends/15 skip-paint; retain as older diagnostic, not current primary or acceptance. |
| Panel N16 peak RSS, focused + background | <=1 GiB | No accepted matched peak result | Early short field max 2,482.988 MiB is historical diagnostic only; final repeated qualification pending. |
| Additional idle bot, finite difference (TUI/panel modes) | <=8 MiB | TUI matched N16 screen gives no idle finite-difference card; panel unavailable | `shared-nav-resource-live-report.md` is active N16 only; no matched N1/N16 idle pair in current evidence. |
| Additional active bot, finite difference (TUI/panel modes) | <=16 MiB | Linux N1→N16 descriptive increment 29.570 MiB/slot; +13.570 MiB | `native-platform-resource-screen-report.md`; unlike-N arithmetic, not a fitted or accepted finite difference. Panel finite differences absent. |
| TUI CPU at N16 | <=0.5 core | 0.729583 core; +0.229583 | Linux N16A diagnostic; target miss, one short cell. |
| Panel CPU at N16, one renderer | <=1 core | 0.572045 core; -0.427955 | Focused-one diagnostic; observed below budget but reader/provenance and final repeated cells pending. |
| Panel CPU at N16, focused + background | <=1 core | 0.630054 core; -0.369946 | Same diagnostic; observed below budget, not acceptance. |
| Simulation cadence | >=40 iterations/slot/s; p99 start interval <=40 ms | Linux N1 49.6703, N16 48.3294; per-slot p99 20–21 ms diagnostic meet | `native-platform-latency-audit.md` gives N1 per-slot 48.6602/49.6715 and 20–21 ms; N16 panel gives 48.192–48.624. No final three-fresh all-mode proof. |
| Decode update → script dispatch | p99 <=100 ms, complete coverage | Linux selected span 24 ms but `decode_coverage_complete=false`; Windows unavailable (`boundary_pending_incomplete`) | Native latency audit (2026-09-07); 600 s N16 companion has candidate 14/16 and reference 12/16 decode available, remaining boundary-pending. Pending. |
| Focused input → visible UI acknowledgement | p99 <=100 ms, complete coverage | N1 Windows/Linux contained bound 2–5 ms, coverage complete for 117/118 selected samples; N16 600 s candidate 13/16, reference 13/16 available | Native latency audit is diagnostic only; TUI producer correction is source-fixed but requires live remeasure. `tui-input-origin-ack-report.md` records 92 tests passed. |
| Focused GPU cadence | >=40 completed frames/s; p99 frame interval <=40 ms | Callback/paint ~48.8/s in N16 focused-one; no hardware presentation proof or accepted p99 | Panel N16 report measures callback/host-paint, not scanout. Older N1 panel screen observed 16.6086 fps, -23.3914 fps, with 50–100 ms callback p99; conditions/provenance unresolved. |
| Background renderer cadence | configured 1 fps, simulation preserved | 15 background slots ~0.985–0.991 paint/s; simulation ~48.2–48.6/s | Panel N16 report; short diagnostic, no final repeated one-hour/lifecycle proof. |
| Calibration / provenance | frozen source, binary, cache/nav/settings/server/geometry/renderer policy; target-device limits disclosed | Partial: hashes, Intel/Vulkan, N=16, policy and archives recorded; some reader/provenance gates remain | Short panel lineage: approved background `e415ade` and focused `0501fed`; approved longer confirmation `37aca9a`; N1 Windows terminal size/managed-resource match missing. Linux is actual modest 2-CPU/1,967-MiB hardware (less RAM than the 4-GiB TUI reference), capacity-limited but not an invalid platform class. |
| Behavior: focus/watch, overlays, minimap, last-FBO freeze, CPU fallback | all preserved, with G1/G2/G3/G4 proof | Qualitative focus/none↔GPU/CPU round trips and scene/minimap restoration approved; full freeze/cadence gates absent | Visual proof `38693c6` approved by Grok 4.5 (`t_da614000`, 2026-09-08); STATE explicitly limits it to qualitative proof. G1/G4 during-freeze remain pending. |
| Lifecycle / Stop / owner plateau | all slots progress; Stop clears isolates; repeated stable post-Stop plateau | No final lifecycle acceptance | Short receipts show clean exit/qualification, but not repeated start/Stop, one-hour plateau, or owner-growth limit. |
| Final 1/16, three fresh runs per mode, idle + active | 120 s warmup + 600 s observation | 0/36 complete as accepted final matrix | 2 N × 2 workloads × 3 modes × 3 fresh repeats = 36 cells. Existing short and one longer N16 diagnostics are screening/confirmation evidence, not this matrix. |
| Repeated 32 + one-hour lifecycle per mode | qualified scaling and last three post-Stop windows | Missing | No current evidence supports completion; do not substitute N16 or short cells. |
| Gated 128 capacity check | only after low-end targets pass; full repeated qualification | Blocked by unmet low-end targets and absent qualification | Plan §6; no 128 acceptance. |

## Candidate and evidence decisions

* Terminal/shared-navigation candidate: the current TUI N16 quartet is real
  short-screen evidence: 944.500–963.844 MiB reference versus 893.500–907.734
  MiB candidate, minimum observed separation 36.766 MiB, CPU worst-role ratio
  +2.6804%. It remains provisional diagnostic evidence, not an accepted saving;
  the absolute 512 MiB target, longer confirmation, latency companion, target
  hardware and lifecycle gates remain open (`shared-nav-resource-live-report.md`).
* Focused tile candidate: park the RSS claim from approved `37aca9a` (reviewed
  2026-09-08). Common RSS rose 0.592%; final-300 RSS rose 8.02%; no sustained
  focused benefit was shown. Do not run another focused clean stage absent
  contradictory evidence.
* Visual candidate: qualitative proof `38693c6` is accepted only for the stated
  focus/scene/minimap restoration. It does not pass G1/G2/G3/G4, frame cadence,
  input, or lifecycle gates.
* Native G1 burst: the 0800 attempt is a failed synchronization, not freeze
  evidence (burst ended 08:04:08 UTC; trigger started ~08:04:12 UTC). Corrected
  controller `t_bfaf7292` is source-only/in progress; it must be reviewed and
  then run once with fresh-ready/first-frame synchronization and timestamped
  input. Do not infer G1 from capture completion or later screenshots.
* Focused GPU reader source `8100b8f` is approved by the actual Grok 4.5 review
  (`20260908_041158_98ce2b`) and integrated source-only through `0f25930`,
  `d22d733`, `b49fcae`, `ec2d716` (the isolated `t_02027ed2` setup is excluded).
  Root primary verification passed 175/175; all six untouched short archives
  recomputed against the original primary reader, with focused-two changing to
  available/meet and four background cells remaining available/meet. This is
  reader integration evidence only: no RSS, scanout, calibration, latency, or
  final acceptance follows.

## Nearest bounded next work

Complete independent review of the1430 archived cohort results and resolve any
material reporting findings. Do not rerun the pair for missing baseline input.
Complete t_c2879587 current-source native TUI calibration design/provenance audit
and its reviewer handoff. Root will release only concrete verified prerequisites
and the bounded declared screen after review; use its actual result to choose
further ownership work. No current result waives absolute or incremental memory,
CPU, latency, calibration, lifecycle, final matrix or whole-branch requirements.
