# Memory campaign — current execution state

Updated 2026-09-06. Branch `codex/memory-diagnostics`; active checkout
`/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`.

Approved plan: [performance-finish-plan.md](performance-finish-plan.md), initially
approved at `530b83e`. Workflow: [../execution.md](../execution.md).
Use this file for current actions. Dated reports are evidence, not instructions
to repeat their old next steps. Hermes board `274bot` supplies live task status;
verify it on resume because a worker may finish after this snapshot.

## Current work and next actions

1. Execution correction is complete: profile-based dispatch without task model
   pins, automatic same-card review, one shared Hermes skill, and portable resume
   pointers. Commit `63863c1` passed Grok4.5 walkthrough review `t_5b4f2547`;
   [receipt](grok-4.5-execution-workflow-review.txt). Final receipt update also
   removes the obsolete Flash role label; no Git permission changed.
2. Qualification gate `b904577` is implemented. Original task `t_88c0a26d`
   completed, but its first review retained a Composer override and is not the
   required Grok4.5 review. The pin was removed. Corrective card `t_61419501`
   completed with Grok4.5 approval; [receipt](grok-4.5-qualification-gate-review.txt).
   Sixteen tests passed, including real corrected-run exit0 and failed-run exit1.
3. Task `t_9cd0e197` adds N=16 and explicit opt-in panel reference policies. It is
   implemented at `9f687af` and approved by Grok4.5 in run44; see
   [receipt](grok-4.5-reference-mode-review.txt). The implementer initially used
   the model string as reviewer profile; orch reassigned to `reviewer` and
   verified the real review completed. Use profile names in handoffs. Its scope does not prove actual GPU backend,
   completed-frame cadence or responsiveness, and includes no live run.
4. Renderer observations `15c4fc4` + hardening `316c2a2` passed Grok4.5 review
   `t_2dfa7342` (actual per-slot residency/backend and host paint cadence);
   [receipt](grok-4.5-renderer-observation-review.txt). Task `t_bd7960b8` completed with Grok4.5 run50 approval at `be8c5fc`
   after round-1 changes; [receipt](grok-4.5-gpu-completion-review.json).
   Independent checks passed: host 141 plus explicitly executed real GPU smoke,
   host-play 143, launcher 8. Bounded queue-completion observations are opt-in
   and distinguish CPU callback delivery from hardware timestamps/scanout.
   No live panel/TUI performance result was produced. Responsiveness
   `t_79e1e21f` approved at `57cbfc7` (code `cc07d09`, report `0e19ea1`); unit
   evidence only — no live overhead/p99 acceptance; report
   [responsiveness-measurement-report.md](responsiveness-measurement-report.md).
   Task `t_dac0878e` freezes immutable system-allocator reference builds at host
   `57cbfc7` / client `451759f2` (clean source); report
   [reference-build-report.md](reference-build-report.md) +
   [reference-build-manifest.json](reference-build-manifest.json); binaries
   `docs/memory/diagnostics/reference-build-20260906T215255Z/` (gitignored)
   panel sha256 `ed403b4f1bbce4c6…`, tui `91103790581692e0…`; features verified
   `memory-profile-no-alloc` via cargo fingerprints; full host/host-play/panel/tui
   tests + explicit real GPU smoke passed. **No live cells in freeze task.**
5. Task `t_672f4ac3` live short reference screen against those frozen binaries:
   report [low-end-reference-screen.md](low-end-reference-screen.md) +
   [low-end-reference-screen-table.json](low-end-reference-screen-table.json);
   batch `docs/memory/diagnostics/low-end-reference-screen-20260906T220129Z/`
   (gitignored). All ten cells exit0 + workload-qualified (six active 1/16
   three-mode + three profiles-off overhead pairs + nav-captures pilot). Clean
   observation windows (before host contamination 22:21:17Z): TUI 1/16 and panel
   focused-one 1/16. **Every clean RSS budget target missed** (TUI n1 median
   ~381 MiB vs ≤256; TUI n16 ~1001 vs ≤512; panel fo n1 ~613 vs ≤384; panel fo
   n16 ~1270 vs ≤768). Actual GPU policy observed (TUI 0 renderers; fo exactly 1
   GPU; fb-n16 16 GPU with ~1 fps background). GPU completion coverage 1.0 on
   profiled panel cells (CPU delivery, not scanout). Profiles-on CPU is
   diagnostic only; all overhead pairs + focused-background resource cells
   contaminated by concurrent Hermes runtime-fix traffic — no overhead
   acceptance, no blind rerun. Visual pilot PNGs inspected (scene-ready,
   bank-area, return-from-bank). Unresolved: scheduling p99, responsiveness
   live p99, last-FBO/CPU-fallback/TUI-resize functional cells, server resource
   series, isolated re-screen of contaminated cells. **No performance pass.**
   Grok review approved `f2ffaa0`.
6. Task `t_6ffdc227` fixed/incremental attribution from qualified portions only:
   report [low-end-owner-attribution.md](low-end-owner-attribution.md). Clean
   finite diffs (profiles-on provisional): TUI Δ/bot med RSS **41.38 MiB**,
   linear fixed ~339 MiB; panel focused-one Δ/bot **43.79 MiB**, linear fixed
   ~569 MiB; panel−TUI N1 ~232 MiB. Contaminated fb/overhead excluded from
   matched claims; prerequisite for those = quiet-host re-screen. Separate N=1
   lite stack-logging attribution on frozen binaries
   (`20260906T225751Z_panel_n1_active`, `20260906T230141Z_tui_n1_active`): both
   workload-qualified diagnostic-only. vmmap: MALLOC_LARGE resident **137.6M**
   both frontends; Memory Tag 255 **32.4G virtual / ~8–11M resident** (not RSS);
   panel IOAccelerator graphics **420.4M** resident (≫ gpu_tracked ~17.5 MiB).
   JsRuntime 256/128/64 MiB stack groups are mmap reservations — **not** ranked
   as resident targets (small V8-tagged sum ~10 MiB). Ranked lead: **two**
   `NavWorld::load_pack` decodes (`Play::new` + `Run::prepare`) totaling
   **140.988 MiB** alloc, RSS-linked via MALLOC_LARGE. ONE next proposal: share
   single NavWorld Arc across Play and Run::prepare; validate with native
   owner count + clean paired TUI RSS (expect tens of MiB if second decode
   resident — not full 62 a priori). Standalone client-play skipped (defaults
   unmatched). No optimization implemented this card. **No performance pass.**
   Grok4.5 run64 approved attribution commit `1f086b0`. Shared-NavWorld
   implementation `606c93b` on `t_513c266a` passed Grok4.5 run69 review:
   host-play 146, panel 377, TUI 87 and scenario missing-world checks passed.
   Native and clean paired validation are still pending.
   Next after review: validate shared-NavWorld on an isolated follow-up card;
   residual fixed cost then GPU mapping / client construct. Final matrix/capacity
   later. Whole-branch Grok4.6 remains required at campaign finish.
7. Linux environment preparation `t_e0ea755d` completed at `d4945ab` with
   review approval: [environment](linux-reference-environment.md). No image
   build/runtime proof in that preparation. Execution card `t_e1c104c1` may
   start Docker Desktop, build/test Linux amd64 and validate optional localhost
   noVNC while no clean measurements run. Emulation is portability evidence,
   not native modest-hardware or Linux GPU performance acceptance.
8. Offline metrics card `t_8bade183` failed before writing deliverables after
   provider timeouts at roughly 78k context. Preserve that failed attempt.
   Recovery `t_135ceb9f` completed at `9be021c` with Grok4.5 run81 approval
   after false-pass corrections (42 tests). Missing coverage/freshness/visible
   endpoints fail closed; a fast GPU callback is not a frame-rate pass.
   Luna GPU interval adapter `t_a0915201` is in progress; scheduling adapter
   `t_a1861aa2` waits for its reviewed handoff. No performance pass.
   Do not start clean paired runs until Linux builds/tests and all other
   native profiling or test activity have stopped. Then collect enabled
   scheduling/responsiveness, matched overhead controls and separate server
   resource evidence; the original flags-off cells cannot prove missing p99.
9. Operator authorized Luna for bounded parallel work. Hermes Codex auth was
   present only in the orch profile; moved that distinct Hermes grant to the
   shared root store with protected backups, preserving other providers and
   the Codex app login. New `luna` profile defaults to `gpt-5.6-luna` through
   `openai-codex`; an actual default-profile request passed (session
   `20260906_192234_c235b6`). Server-resource sampler `t_f9ec3670` and
   hardening `t_014df28c` completed at `50d2ba8`, with Grok4.5 approval.
   Thirteen tests passed on macOS and independently in Linux amd64, including
   real process sampling. Overhead remains unmeasured; macOS start-identity
   resolution and pressure limitations are explicit.
10. Per-slot scheduling `t_c0e6e746` completed at `6345fbc`, Grok4.5 run84
    approval: host 165 plus one ignored,
    host-play 146 and 14 cadence tests. True tick-start endpoints, conservative
    quantile bounds and flush lag are exposed. Scene separation remains
    unavailable. No live p99 or overhead acceptance.
11. Orch created comparison control branch `codex/shared-nav-control` at
    `d373ea0`, checkout `.worktrees/shared-nav-control`: `6345fbc` with only
    Nav-sharing `606c93b` reverted. Client `451759f2` was fetched from the active
    local client because that commit was unavailable remotely. No push.
    Build card `t_9abeb5e2` freezes matching control/candidate binaries before
    isolated measurements. Both sides retain the new timing instrumentation.
12. User-requested VNC desktop is running as `memory-ref-amd64-view` on host
    `http://127.0.0.1:6080/vnc.html?autoconnect=1&resize=scale` (Linux amd64,
    2 CPU/4 GiB, relay disabled for this viewer). CUA visibly verified the
    connected Vivaldi window with a live Linux build-log terminal and an
    interactive `memref` x64 shell. Keep it alive for the user. Orch corrected
    invalid `x11vnc -localhost false` to `-localhost`; running container was
    repaired in place and the Linux worker will include the script correction.
    Account desktop helpers separately; this is functional desktop proof.

## Evidence frontier

- [Runner forward pair](runner-clean-pair.md): one qualified pair and a concrete
  native vector-allocation removal; not full performance acceptance.
- [Reverse pair](runner-reverse-pair.md): corrected run qualified, previous run
  failed for zero progress and below-scale samples. The comparison is rejected.
- [Bounded follow-up](runner-qualification-followup.md): all 32 progressed,
  8–31 steals each, no observed out-of-area scene-2 client during observation.
  Prior endpoints exactly match local Mime/Maze source coordinates; this is
  strong source inference, not a captured failed-run event timeline. Readiness
  dips and the operator's earlier bank-booth bouncing remain unattributed.
  Diagnostic overlap excludes its timing/RSS from acceptance. Grok4.5 approved.
- [Qualification gate](qualification-gate-report.md): `active` and `seeded-idle`
  only. Frontend completion is not qualification; gate success is not budget
  acceptance. Preserve unsupported idle/lifecycle proof gaps explicitly.
- [Ownership](targeted-allocation-owners.md) and
  [consumer audit](snapshot-consumer-audit.md): attribution and consumer contracts
  for next measured work; do not count allocation totals as resident savings.
- [Low-end reference screen](low-end-reference-screen.md): short 1/16 three-mode
  screen; workload qualified; clean RSS budgets all missed; contamination
  limits overhead and focused-background resource claims.
- [Low-end owner attribution](low-end-owner-attribution.md): clean 1/16 fixed
  vs Δ/bot; N=1 native owners; dual nav decode + panel IOAccel; one shared-NavWorld
  proposal; V8 large stacks not RSS.

No accepted claim yet for the final 1/16 three-mode budgets, required p99 and
completed GPU frames as presentation gates, modest-hardware validation, final
lifecycle matrix, or 128-bot capacity. The 1,000-bot/16GB VPS ambition remains a
later milestone. No game behavior, random-event settings or fixture requirements
may be weakened to obtain a passing memory result. Keep existing compatibility
errors/stubs.

## Handoff maintenance

At each completed task/review or changed acceptance decision, update this file
with the commit, task ID, evidence receipt, unresolved gap and next action.
Keep failed artifacts. Do not duplicate the plan or Git policy here. The primary
checkout's gitignored `docs/superpowers/STATE.md` only points here; update that
pointer if the campaign moves. After memory completion, it also identifies the
preserved historical context for resuming the remaining compatibility work.

## Runtime handoff correction (2026-09-06)

The old Hermes guard nudged implementers after request-review (observed on run45;
run47 received a bounded exact-process supervisor). That was a runtime defect.
Local Hermes branch `codex/kanban-handoff-stop` now contains fix `32e33b6b`, plus
instruction-only follow-up `e33b4235`. Grok4.5 review `t_82629455` run58 approved
`32e33b6b`; [receipt](grok-4.5-hermes-handoff-review.json). Validation: 124 tests
passed with one Windows-only skip; reviewer independently ran 81 passing tests.
The original baseline failed all five new cases. Actual run58 exited with
`reason=kanban_run_ended`; its PID was confirmed gone without the workaround.

Retire the workaround for fresh workers importing the local patch. Workers
already running before the patch retain their imports: allow them to drain and
verify handoff exit, using the prior exact-process workaround only if needed.
Already-started background jobs must finish/stop before shared-workspace handoff.
No gateway restart, auth/provider configuration change or remote push was made.
The patch is local, not a claim that an upstream Hermes release contains it.

The preserved compatibility plan/spec in the primary checkout now defer to the
canonical execution protocol and use the at-pen fixture consistently. Missing
capabilities remain diagnostic outcomes rather than functional passes; authorized
bounded diagnosis may continue after a failed proof. Historical state is evidence.

Live task `t_672f4ac3` acknowledged concurrent Hermes test activity beginning
2026-09-06T22:21:17Z. Its affected observations must remain diagnostic for resource/
latency acceptance; preserve gameplay qualification separately. No blind reruns.

## Matched shared-nav screen stopped on script failure (2026-09-07 UTC)

Current Rust sources remain `6345fbc`, client `451759f2`; later commits are
measurement tooling, Docker environment and reports. The reviewed immutable
control/candidate binaries are recorded in [build manifest](shared-nav-build-manifest.json).
Control is `d373ea0` (only shared-nav change reverted). Dedicated target directories
avoided the preserved failed shared-target build attempt.

Grok-4.5 approved per-slot scheduling tooling `fd8f907` and resource-tool hardening
`bd5dc1e`. Resource numbers remain diagnostic: strict eligibility cannot be
unlocked by an `overhead: measured` label; actual attributable overhead evidence
support remains missing. Negative values, idle-budget misuse and non-increasing
timestamps fail closed. Decode coverage still conservatively rejects lifetime
cancellations; the qualified first control has no new cancellations in observe.

The first quiet short screen (`t_54b3b727`) stopped after three attempted cells:

- Control N1: exit 0, workload qualified; median RSS 367.125 MiB, CPU 0.0378257 cores.
- Shared-nav N1: exit 0, workload qualified; median RSS 296.015625 MiB, CPU
  0.0409057 cores. First-pair RSS difference is -71.109375 MiB, but CPU is
  about +8.14%, outside the 5% margin. Both scheduling windows meet the target
  with p99 bounds 25–26 ms. This is not accepted CPU/RSS savings, repeated-run
  variation, overhead qualification or an absolute-budget pass.
- Shared-nav N16: FAIL + exit 1 before observation completed. Exact failure:
  `livefd010_5: script requested stop on tick 289; isolate stopping`.
  All 16 were initially ready/active, but only about 110.56 seconds of observation
  completed. No observe-end qualification exists. Do not claim N16 qualified.
- Remaining control N16 and four overhead cells were not run. No retry until the
  functional failure is understood. Raw artifacts are preserved under
  `diagnostics/shared-nav-clean-screen-20260907T004029Z/` and its linked runs.

Forensics card `t_872d244a` is read-only plus a scoped report. Existing clean
artifacts retain the generic Stop line but not the script's stored stopReason or
failure-time runtime snapshot. Do not assume a bank, navigation or food cause.
A bounded failure-capture diagnostic may follow the evidence review; do not
change loadouts, script timeouts or behavior to obtain a passing benchmark.
Offline responsiveness-window task `t_3a521e21` follows the screen review; it may
only qualify contained spans supported by the actual timestamps and counter
accounting. Input has no events, and coarse decode bins cannot automatically
prove the 2 ms paired regression margin.

Linux environment follow-up `33ac6b5` / report correction `5c8f16d` passed
Grok-4.5 review. Full host and memory-feature suites passed as non-root with the
catalog mounted read-only. Mesa lavapipe makes the overlay test pass, but the
full client suite fails `gpu_textured_shade_scales_texel_brightness` at
`gpu_texture.rs:485`: shade 16 expected red near 223, got 255. Cause undiagnosed;
no test weakening or native Linux GPU acceptance. Exact commands and image
identities are in [Linux follow-up](linux-reference-followup.md).

Operator noVNC viewer remains running on localhost6080 (old desktop image,
runtime x11vnc fix applied); rebuilt desktop image is available separately.
Existing Chroma service was not changed. These are ambient helpers, not free
resources to omit from later accounting. No target-hardware, final matrix,
lifecycle, 128-capacity, remaining-candidate or whole-branch acceptance yet.
