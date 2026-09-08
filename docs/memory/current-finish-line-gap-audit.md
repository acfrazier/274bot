# Current finish-line gap audit

Date: 2026-09-08 08:13 UTC. Branch `codex/memory-diagnostics`.
This is a bounded evidence synthesis, not a new acceptance or target change. The
approved budgets are from `performance-finish-plan.md` §1, §3, §4, §5 and §6
(approved at `530b83e`); current boundaries are `STATE.md` lines 12–27.

Status vocabulary: `observed` means a real number exists but is not acceptance;
`diagnostic meet` means the contained reader bound is compatible with a target;
`pending` means the gate is not covered or its evidence is invalid/incomplete;
`parked` means the candidate is not retained as a saving.

| Approved requirement | Target | Current result / delta | Evidence scope and status |
|---|---:|---|---|
| TUI N1 steady median RSS | <=256 MiB | 167.14 MiB observed; -88.86 MiB | Linux native N1B, `native-platform-resource-screen.md` (2026-09-07); short diagnostic, not final acceptance. |
| TUI N1 startup/transition peak RSS | <=384 MiB | Not reported as a matched final peak | N1B reports median only; pending three-fresh final cells. |
| TUI N16 steady median RSS | <=512 MiB | 610.68 MiB; +98.68 MiB | Linux N16A, same report; one ~119 s diagnostic cell, target miss. |
| TUI N16 startup/transition peak RSS | <=768 MiB | No accepted peak result | N16A short diagnostic does not replace final peak protocol; pending. |
| Panel N1 steady median RSS, one renderer | <=384 MiB | 507,463,680 B = 484.0 MiB; +100.0 MiB | Windows focused-one N1; binding retains missing `terminal_size`, managed resource unavailable; not acceptance. |
| Panel N1 peak RSS | <=512 MiB | Not reliable/currently qualified | Existing N1 screen has no accepted startup/transition peak; pending. |
| Panel N16 steady median RSS, focused one | <=768 MiB | 942.324 MiB; +174.324 MiB | Frozen Intel/Vulkan short diagnostic (`windows-panel-n16-diagnostic-report.md`, archive `bb2de5d6`); reader binding remains failed, diagnostic only. |
| Panel N16 peak RSS, focused one | <=1 GiB | 1,113.719 MiB field max; +89.719 MiB | Same diagnostic; sampled RSS max 1,109.582 MiB. Not accepted peak evidence. |
| Panel N16 steady median RSS, focused + background | <=768 MiB | 1,893.871 MiB; +1,125.871 MiB | Same report, archive `7fe09f3f`; all 16 GPU backends, 15 skip-paint; reader/provenance failure preserved. |
| Panel N16 peak RSS, focused + background | <=1 GiB | 2,482.988 MiB field max; +1,458.988 MiB | Same short diagnostic; not a final repeated qualification. |
| Additional idle bot, finite difference (TUI/panel modes) | <=8 MiB | TUI matched N16 screen gives no idle finite-difference card; panel unavailable | `shared-nav-resource-live-report.md` is active N16 only; no matched N1/N16 idle pair in current evidence. |
| Additional active bot, finite difference (TUI/panel modes) | <=16 MiB | Linux N1→N16 descriptive increment 29.570 MiB/slot; +13.570 MiB | `native-platform-resource-screen.md`; unlike-N arithmetic, not a fitted or accepted finite difference. Panel finite differences absent. |
| TUI CPU at N16 | <=0.5 core | 0.729583 core; +0.229583 | Linux N16A diagnostic; target miss, one short cell. |
| Panel CPU at N16, one renderer | <=1 core | 0.572045 core; -0.427955 | Focused-one diagnostic; observed below budget but reader/provenance and final repeated cells pending. |
| Panel CPU at N16, focused + background | <=1 core | 0.630054 core; -0.369946 | Same diagnostic; observed below budget, not acceptance. |
| Simulation cadence | >=40 iterations/slot/s; p99 start interval <=40 ms | Linux N1 49.6703, N16 48.3294; per-slot p99 20–21 ms diagnostic meet | `native-platform-latency-audit.md` gives N1 per-slot 48.6602/49.6715 and 20–21 ms; N16 panel gives 48.192–48.624. No final three-fresh all-mode proof. |
| Decode update → script dispatch | p99 <=100 ms, complete coverage | Linux selected span 24 ms but `decode_coverage_complete=false`; Windows unavailable (`boundary_pending_incomplete`) | Native latency audit (2026-09-07); 600 s N16 companion has candidate 14/16 and reference 12/16 decode available, remaining boundary-pending. Pending. |
| Focused input → visible UI acknowledgement | p99 <=100 ms, complete coverage | N1 Windows/Linux contained bound 2–5 ms, coverage complete for 117/118 selected samples; N16 600 s candidate 13/16, reference 13/16 available | Native latency audit is diagnostic only; TUI producer correction is source-fixed but requires live remeasure. `tui-input-origin-ack-report.md` records 92 tests passed. |
| Focused GPU cadence | >=40 completed frames/s; p99 frame interval <=40 ms | Callback/paint ~48.8/s in N16 focused-one; no hardware presentation proof or accepted p99 | Panel N16 report measures callback/host-paint, not scanout. Older N1 panel screen observed 16.6086 fps, -23.3914 fps, with 50–100 ms callback p99; conditions/provenance unresolved. |
| Background renderer cadence | configured 1 fps, simulation preserved | 15 background slots ~0.985–0.991 paint/s; simulation ~48.2–48.6/s | Panel N16 report; short diagnostic, no final repeated one-hour/lifecycle proof. |
| Calibration / provenance | frozen source, binary, cache/nav/settings/server/geometry/renderer policy; target-device limits disclosed | Partial: hashes, Intel/Vulkan, N=16, policy and archives recorded; reader binding/provenance gaps remain | Focused panel reader `901c2b0` fails on omitted `pre.server`; N1 Windows terminal size missing; Linux is 2 CPU/1,967 MiB diagnostic, not approved target hardware. |
| Behavior: focus/watch, overlays, minimap, last-FBO freeze, CPU fallback | all preserved, with G1/G2/G3/G4 proof | Qualitative focus/none↔GPU/CPU round trips and scene/minimap restoration approved; full freeze/cadence gates absent | Visual proof `38693c6` approved by Grok 4.5 (`t_da614000`, 2026-09-08); STATE explicitly limits it to qualitative proof. G1/G4 during-freeze remain pending. |
| Lifecycle / Stop / owner plateau | all slots progress; Stop clears isolates; repeated stable post-Stop plateau | No final lifecycle acceptance | Short receipts show clean exit/qualification, but not repeated start/Stop, one-hour plateau, or owner-growth limit. |
| Final 1/16, three fresh runs per mode, idle + active | 120 s warmup + 600 s observation | 0/18 complete as accepted final matrix | Existing short and one longer N16 diagnostics are screening/confirmation evidence, not the required three-fresh-per-mode matrix. |
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
* Focused reader `t_02027ed2` remains isolated and pending review after rejected
  epoch validation. It is not integrated and supplies no accepted GPU/RSS.

## Nearest bounded next work

The nearest actual blocker is not another memory screen: it is closing the
reviewed instrumentation/provenance path needed for trustworthy acceptance.
First, complete review of `t_bfaf729` and run one corrected native synchronized
burst to establish (or falsify) G1/G4 during-freeze and input timing. In
parallel only through the existing review boundary, finish `t_02027ed2`'s
attach/detach/backend epoch validation; do not integrate it before approval.
Then remeasure the corrected TUI input-origin producer and complete decode
boundary coverage in a matched latency companion. These steps advance the
measurable rendering/behavior, input <=100 ms, and decode <=100 ms requirements;
they do not waive the still-missing absolute RSS/CPU, calibration, final
1/16, 32/one-hour, or gated-128 requirements. After those bounded fixes, the
root owner should choose one measured ownership optimization or calibration /
workload correction for the large N16 RSS gaps, rather than indefinite reruns
or another focused-tile stage.
