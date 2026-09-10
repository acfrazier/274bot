# 274bot execution workflow

Repository authority and Git rules live in [AGENTS.md](../AGENTS.md).
Current campaign actions live in the state named by the active task or the
primary checkout's local `docs/superpowers/STATE.md` pointer, when present.
The memory campaign retains its `docs/memory/STATE.md` and evidence tools in its
own checkout; they are not prerequisites for a fresh public checkout.
Historical reports retain their original results; their next-step paragraphs do
not supersede that state. Update state at task/review boundaries, including
failed proofs and explicit acceptance decisions. Update the primary checkout's
local pointer when the campaign checkout moves.

## Proportionate verification

Operator clarification, 2026-09-10: this is a hobby project. Keep code quality
high and preserve the optimization work, while matching verification effort to
the change's risk. Small mechanical, documentation and support-tool changes do
not automatically require new tests, a separate review card or a standalone
report. Inspect the diff and use a focused syntax check or relevant existing
test when useful. Add tests for meaningful behavior and failure modes, not tests
that merely repeat the implementation. Build new proof tooling only when it
answers a concrete unresolved question.

Protocol, lifecycle, navigation, rendering ownership and memory-sensitive
changes warrant appropriate regression tests and independent review. Group
related implementation into coherent tasks and review milestones; do not create
a new review hop for each helper, formatting fix or mechanical correction.
The implementation/review handoff below applies to tasks that warrant that
independent review. Existing task boilerplate does not override this guidance.
Required final Grok whole-branch review for substantial campaigns and honest
functional/performance evidence remain in force.

## Roles and dispatch

Use the existing Hermes `274bot` board and configured profiles: `implementer`,
`reviewer`, and `branchreviewer`. Verify their model/provider defaults against
AGENTS before first dispatch or after a routing failure. The orchestrator can be
Codex using the CLI or a Hermes `orch` session; it is not required to impersonate
another model/profile. Do not change working provider credentials to satisfy a
stale document. Inspect only non-secret settings.

Operator-authorized additional profiles (2026-09-09): `sol` uses
`gpt-5.6-sol` / `openai-codex` with high reasoning; `grok46` uses `grok-4.6` /
`xai-oauth` with high reasoning. Configured defaults and completed routing probes
were verified (`20260909_114546_d7beb0`, `20260909_114547_6b4443`). Use `luna`
for small mechanical tasks, `sol` for demanding implementation across files or
lifecycle/protocol boundaries, and `grok46` for architecture/fidelity review or
escalation after two substantive rejections. Apply this guidance at a task
boundary; preserve active valid workers and measurements. These additional
profiles do not replace same-card `reviewer` handoff, independent milestone
reviews, the final `branchreviewer` pass, or the evidence requirements below.

Create scoped cards without `--model` or `--provider`. Those overrides persist
through review reassignment and can silently replace the reviewer's model.
For an existing pin, use `hermes kanban --board 274bot set-model TASK none`
before review dispatch. A recovery override needs an explicit removal step.
If a review already ran under the wrong model, retain the receipt, classify it
as insufficient, and obtain the required model's independent review.

Choose `--workspace worktree` for a new isolated task, or `--workspace dir:PATH`
for the orchestrator-designated existing campaign checkout/branch. Avoid a second
worktree for review. Briefs identify exact scope, plan section, verification,
report location, and the branch to verify. Do not copy Git policy into plans.
For checks needing committed dependencies, export the exact host and client
source into a separate directory. Never temporarily restore, stash, or copy over
concurrent working files to make a build pass; their owners may be editing them.
Use `fail-closed-dispatch` for foreign-API compatibility work; it is not a
mandatory dependency of a memory measurement or documentation task.

## One task, implementation then review

The configured flow has `kanban.review_dispatch=true`. The implementer commits
only its scoped files and calls `kanban_request_review` with reviewer `reviewer`,
a summary, and test evidence. It then stops; it does not call complete after the
handoff. Hermes dispatches the same card in the review lane, using that profile.
Do not also create a routine child reviewer card.

Immediately inspect the handoff: the assignee must be the profile name
`reviewer`, not the model name `grok-4.5`. A model string in the profile field can
leave the card waiting without a worker. Correct that assignment and verify a
review run actually starts before reporting it as running.

The reviewer verifies the named commit and evidence, then completes on approval
or requests changes back to the implementer. Check the actual model, commit and
verdict before accepting the result; an assignee label alone is insufficient.
A separate corrective review card is appropriate for an already-closed wrong-model
review. A separate whole-branch card assigned to `branchreviewer` remains required
at the campaign finish line; task reviews do not replace it.

Dependencies release when a card completes. Put work that must await acceptance
behind its review-completed card. Independent preparation can proceed, but no
live acceptance run or integration may rely on an invalid review. Keep clean
measurements free of concurrent builds, agent tests, other benchmarks and native
profiling. Diagnostic overlap must be recorded and excluded from performance claims.

## Review cadence and reviewer trial

Operator approved this trial on 2026-09-07. Keep existing profile defaults,
same-card per-task Grok 4.5 review, and required final whole-branch Grok 4.6.

- Before substantial architecture implementation, review the ownership model,
  behavior invariants, expected benefit, and a bounded experiment that could
  disprove the proposal. A design approval does not establish a measured win.
- At a completed integration milestone, review the combined changes before
  expensive native comparison work. Trigger this at changes crossing observation
  publication, script transport, client/rendering ownership, or lifecycle
  boundaries, or when evidence invalidates a shared assumption. Use coherent
  capabilities and risk, rather than a fixed number of commits.
- After measurements, independently recompute important results from the raw
  artifacts and verify that fixture coverage and accounting support the claims.
  Source review alone does not approve an evidence conclusion.
- At campaign completion, review the whole branch and the requirement-to-evidence
  mapping. Earlier milestones narrow the unresolved questions; they do not
  replace the required final Grok 4.6 pass.

For the first integration checkpoint, trial a fresh Astra session using the
existing `orch` profile as an independent reviewer alongside a Grok 4.6 session
using `branchreviewer`. These are separate milestone reviews, not duplicate
routine reviews of every task. Neither reviewer implements its own fixes.
Root freezes an exact base/head pair and evidence manifest after prerequisite
task reviews, gives both reviewers the same contract, diff, relevant surrounding
code, test evidence and known unresolved issues, and withholds the other trial
reviewer's new findings until each independent pass is submitted. Do not evaluate
a moving HEAD or imply that uncommitted worker changes were reviewed.

Root reconciles the findings against source, raw evidence or reproductions and
records severity, confirmed unique findings, overlap, rejected findings and why,
later-discovered misses, actual model/provider, reviewed commits and elapsed time.
Record blocked tool calls, retries and other execution restrictions as timing
confounders; wall time alone is not model review speed. Prefer established
read-only tools and approved verification commands, without weakening approval
policy to make a reviewer comparison look cleaner.
Retain legitimate disagreements for resolution; do not decide by majority vote
or raw finding count. One trial supplies local evidence, not a general model
ranking. Change standing reviewer assignments only after the operator decides.
Fix material findings, obtain the appropriate follow-up review, then release
dependent native work. Independent preparation can continue in the meantime.

## Monitoring from this session

CLI creation is not a promise of a notification to Codex. An external Codex
orchestrator polls `hermes kanban --board 274bot show TASK` or `runs TASK`, with
bounded waits and user updates. The Hermes gateway may dispatch ready cards
automatically; do not assume a card waits for a manual `dispatch` call. When the
active work continues, keep monitoring through review rather than leaving a
worker unattended after implementation.

Inside a Hermes TUI, use its real session key with `notify-subscribe` when needed:
`hermes kanban --board 274bot notify-subscribe TASK --platform tui --chat-id "$HERMES_SESSION_KEY" --notifier-profile orch`.
Do not invent a TUI session key for an external Codex session. Verify subscription
and delivery if relying on wakeups; otherwise poll. Always specify the board.

Kanban comments are next-spawn context, not reliable live steering. To redirect
a running worker, reclaim its run, inspect preserved changes, then update the
brief/comment and resume with the appropriate profile. Do not retry an unchanged
provider/model or credential failure. Preserve the failure and fix its cause.

## Build-cache housekeeping

Fresh evidence directories do not require fresh compiler caches. For functional
regressions, reuse an explicitly named task cache when the test contract permits;
keep raw logs, source hashes, and result receipts in fresh output directories.
Do not change build isolation required by a matched performance protocol.

At task completion, identify disposable compiler intermediates separately from
source snapshots, raw measurements, and frozen release binaries. Root owns
cleanup after verifying that no active worker or open file uses the selected
paths. Retain the evidence needed to reproduce and review the result; cache
removal is not a reason to delete a failed run. Check free space before another
large build and report cache locations in the handoff so stale multi-gigabyte
copies do not accumulate unnoticed.

## Proof, diagnosis and acceptance

A failed live cell is a failed cell: preserve artifacts and report it. It is not
permission to weaken the fixture, change gameplay, or average failure into a
passing comparison. Authorized bounded diagnosis may continue; do not interpret
cell failure as automatically cancelling the whole campaign. Follow the approved
plan's investigate-or-park limit instead of rerunning until lucky.

Keep these stages explicit: process completion; workload qualification; measured
candidate comparison; provisional retention; final target/lifecycle acceptance;
whole-branch review. A code review does not grant performance acceptance.
Above-budget intermediate improvements can be provisionally retained when their
evidence and behavior support that decision; they do not satisfy the final budget.
Missing host operations can be an honest compatibility-task result only when the
brief allows it. They are not a passing memory workload or a completed capability.

When working in the memory campaign checkout with its retained qualification
tool, use the explicit expected mode for completed `active` or `seeded-idle` controls:
`python3 docs/memory/qualify_control.py RUN --no-write` for system allocator and
no sidecar; add `--counting` and/or `--diagnostics` only when intended by the cell.
Check its exit status and JSON `qualified`, not the frontend's completion message.
`idle` and `lifecycle` need their own qualification and are not covered by that CLI.
Other campaigns use their named harness and qualification contract; this
historical campaign tool is not a dependency of ordinary builds or script work.
This check is workload qualification only; CPU/RSS/latency/rendering budgets and
provenance still require separate evaluation. Actual GPU backend and completed
frame cadence must be measured before claiming a GPU mode passes. Read screenshots
with a capable tool; do not infer visual proof from capture filenames.
