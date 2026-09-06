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
   No live panel/TUI performance result was produced. The board has no pending
   cards at this boundary. Next build immutable release binaries and qualify the
   three approved modes live. Precise responsiveness measurements remain separate work.
5. Continue approved single-client fixed-cost attribution and measured N/N+x
   costs. Terminal-runner cleanup performance acceptance remains pending; do not
   launch more blind repeats. Final whole-branch Grok4.6 review remains required.

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

No accepted claim yet for the final 1/16 three-mode budgets, required p99 and
completed GPU frames, modest-hardware validation, final lifecycle matrix, or
128-bot capacity. The 1,000-bot/16GB VPS ambition remains a later milestone.
No game behavior, random-event settings or fixture requirements may be weakened
to obtain a passing memory result. Keep existing compatibility errors/stubs.

## Handoff maintenance

At each completed task/review or changed acceptance decision, update this file
with the commit, task ID, evidence receipt, unresolved gap and next action.
Keep failed artifacts. Do not duplicate the plan or Git policy here. The primary
checkout's gitignored `docs/superpowers/STATE.md` only points here; update that
pointer if the campaign moves. After memory completion, it also identifies the
preserved historical context for resuming the remaining compatibility work.

## Runtime handoff caveat (2026-09-06)

The installed Hermes `agent/kanban_stop.py` stop nudge recognizes complete/block,
but not request-review. Run45 continued after its handoff and made a scoped
follow-up commit. Orch stopped its stale process; reviewer independently approved
the final `316c2a2` code. Documentation alone does not fix this runtime guard.
Until corrected in Hermes, verify the ended implementer process exits at handoff
and stop that exact old process if necessary, without reclaiming the new reviewer.
For run47/task `t_bd7960b8`, orch started a bounded supervisor that checks run
outcome and original PID identity before terminating only an ended implementer.
No Hermes source, auth or provider configuration was changed. Keep this execution
limitation visible; never accept a review while its producer is still editing.
