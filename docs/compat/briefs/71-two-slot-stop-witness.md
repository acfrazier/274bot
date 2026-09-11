# Correct the post-Stop witness and retain raw two-slot observations

Use grok46 profile defaults on codex/rs2b0t-multirevision. Read
06a-two-slot-isolation-proof.md, brief65 and docs/execution.md. No agents/LIVE.
This is a corrective follow-up to completed card t_4eb51061 (source587a69fd,
actual Grok4.5 review1230), not a duplicate routine review.

Root exact isolated LIVE on both revisions returned PASS in 47.618/49.804s;
raw receipts/logs are evidence/two-slot-isolation/r{274,289}-587a69fd/.
Source audit identifies an unsound post-Stop assertion: Phase::AfterStop and
IsolationWitness::qualify compare B against b.pause_end, captured before A
resumes and before A stops. B can progress during A's resume and the next
AfterStop loop accepts that already-completed progress. Thus those raw PASS
runs do not establish B progressed after A stopped. Preserve all receipts;
label only this part unqualified, not a product lifecycle regression.

Correct only the standalone fixture/support witness. Capture both actual
observations at the Stop boundary and require a strictly later B observation
with new XP/coins plus appropriate consumed own item/rune observations, using
that boundary rather than pause_end. Retain own/peer identity, no foreign
fodder, Start/readiness, pause drain/stability, resumed work, and actual states.
No synthetic tick movement, widened timeouts, product lifecycle or control
changes. If actual in-flight behavior needs interpretation, report to root.

Current terminal JSON discards intermediate witnesses; serialize the complete
IsolationWitness/SlotRecord observations (baseline, first, drain, pause_end,
resumed/further, Stop boundary, after_stop, identities/settings/states) so root
can independently recompute the ordered proof from retained raw data. Output
these on qualification failure too when available. Logging does not prove a
predicate; enforce it. Update focused existing tests with a regression that
has B progress after pause but none after Stop, and a positive with actual
post-Stop deltas. Include missing/stale stop-boundary refusal.

Own ONLY crates/host-play/tests/two_slot_isolation_live.rs and its unique
support/two_slot_isolation.rs plus 06a report/evidence/two-slot-isolation.
No shared scenario/catalog, host runtime, API/client, engine or foreign edits.
Freeze exact host/client plus owned overlay; NEW EMPTY target. Existing
witness tests, focused strict Clippy and formatting; no broad churn. Commit
only owned source/report/new check artifacts (do not stage root LIVE/build
files), inspect scope, request SAME-card reviewer, then STOP. Root reruns both
revisions only after actual Grok4.5 approval.
