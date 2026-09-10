# Responsive profile preparation for panel startup

Use configured grok46 defaults for a bounded read-only design review. Campaign
branch codex/rs2b0t-multirevision; read AGENTS.md and docs/execution.md once.
The full plan step 8 includes frontend/preservation behavior. Root traced the
operator's reproducible native beachball; see 06b-panel-startup-trace.md and
its raw evidence. This is design/source analysis only, no product edits or LIVE.

The current debug panel blocks its UI thread for about 38 seconds after the
first blank window. ProfileSelection.bind validates the nav pack/flags,
SharedClientTemplate::load validates again, and run_with_template validates a
third time. The three passes hash about 1 GB. The sample actually enters these
paths on the main thread before the slot/network starts. Source is currently
1947741f/client 56d8027 (stop correction; profile startup unchanged). Later
bank work in api/script/host-play is concurrent and unreviewed; do not edit it.

Find the smallest sound correction that lets the native event loop paint and
respond during preparation. Keep one immutable process profile, cache/nav/RSA
identity checking, changed-resource rejection before slots, and actual errors.
Do not remove validation or treat a cached pathname/mtime as checked identity.
Do not turn this into a startup/cache/performance campaign or change login and
script lifecycle bounds. Existing slot/GPU ownership and IsolatedEnv thread-local
store isolation must survive; moving the entire Session onto a worker may be
inappropriate. Cover both deferred live boot and normal vault Unlock paths.

Inspect app.rs Boot/boot_execute and first-present callback, session.rs bind /
start_vault/start_play/live preparation, and host-play template/profile APIs.
Recommend exact ownership: what can be prepared on a worker, how the UI accepts
or rejects a completion after user actions/shutdown, where the final validation
runs, and how an error remains reviewable without spawning a partial session.
Consider a reusable host preparation value only if it preserves the validation
contract and helps keep the UI change bounded. Account for both Windows and Mac.

Write 06c-panel-startup-design.md with proposed allowed files, concrete risks,
existing regressions to extend, and a bounded native proof using actual event
loop responsiveness plus mismatched/changed resource refusal. Raw sample is
functional diagnosis, not a release-performance acceptance or measured saving.
No source edits, other tasks/agents, builds of moving WIP, fixture actions,
operator configuration, commits, remotes, gitlink changes or release actions.
Return a concrete scoped recommendation or evidence-backed objection; root
owns implementation dispatch, live proof and committing the report.
