# Step 4C: controlled host boundary live harness

## Live fixture correction, 2026-09-10

The original frozen harness passed 274 but failed 289 after login and observed
walk because visible Hans was absent. Read `02-host-boundary.md` and retained
`evidence/host-boundary/live/r289-fede3c0d.{log,json}`. Root has changed only the
harness to prepare a temporary nearby Hans via the existing local npcadd command,
observe a new nearby identity, then establish the action baseline. Exact NPC
dialogue, loc-change and logout assertions and action deadlines remain.
Review the committed harness correction against `fede3c0d`, including the source
of npcadd (non-production/staff guard, 500-cycle expiry) and primary patrol data.
No live launch or worker source edits. Inspect named commits/source exports;
never stash/reset/restore/checkout or alter shared WIP/index. This is corrective
source review before a justified live reproduction, not a claim of acceptance.

## Original task brief

Root implements this bounded task inline on `codex/rs2b0t-multirevision`.
Follow plan step 4 and the approved/corrected design report. Product scope:
`crates/host-play/tests/revision_boundary_live.rs` only. The constructor,
`api::Driver` and GameSnapshot are production paths; the pump directly calls
Client::mainloop and GameSnapshot::rebuild. It starts no Play slot or guardian,
loads no navigation into use and does not lift the 289 bot-operation gate.

The ignored test requires LIVE=1 plus an explicit BOUNDARY_REVISION=274|289,
one revision per process. An optional BOUNDARY_ENGINE_DIR selects the matching
isolated engine. Binding requires loopback local endpoints. It creates a new
disposable account, loads the bound shared cache, and performs:

1. login and actual ingame/attached/scene2 readiness;
2. local tutorial/mainland seeding, observed seed acknowledgement, clean logout
   and relog before the action baseline;
3. one-tile host walk with exact observed displacement;
4. Talk-to on snapshot-resolved Hans, requiring a new nonempty chat dialogue;
5. Open on a nearby snapshot-resolved closed Door, requiring arrival near it,
   removal/replacement of that closed identity, and a nearby Door offering Close;
6. logout through the cache-discovered client-code-205 control, requiring
   ingame/attached false and empty live actors/inventory/bank views.

The targeted scene locations are fixture choices; NPC/loc ids, operations and
logout component ids are obtained from the selected cache/live snapshot. JSON
phase receipts include profile/cache/account identity, observation baselines and
actual deltas. Failures emit FAIL and exit 1. Cleanup teardown on a failure never
satisfies the logout observation. Existing scene/action deadlines are bounded;
failed predicates must be diagnosed, not weakened to accept a queued send.

Initial source checks: focused compile and strict harness Clippy passed; logs
are in `docs/compat/evidence/host-boundary/harness/`. These are compile evidence,
not a live pass. Snapshot/reset and outbound cards are still being implemented;
the reset predicates deliberately depend on step-4B. Root will run local proof
only after source acceptance of all step-4 components. Platform fixture setup
is independent and does not count as host action proof.

Review this same card with profile `reviewer`, actual Grok 4.5. Read the named
harness source commit and its logic, not concurrent workers' uncommitted edits
as though they were accepted. Return any false-pass or preservation concerns.
No source edits, live sessions, remotes, gitlink changes or extra agent dispatch.
