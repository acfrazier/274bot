# First banking capability family

Current status (2026-09-10 20:53 UTC): source correction in progress. Candidate
host `8a60eef77e9c4e74c3ee81332e495fb9331c046a` / client
`56d80272bcbda3eb1e22db096c1c5e21d3497de4` is not yet accepted. Same-card
Grok 4.5 review requested changes, and root found additional concrete outcome
and supported-option gaps. The direct fixed-transfer bank-return cells subsequently passed on both
revisions (03-world-capabilities.md); named access and Withdraw-X live
qualification are not claimed.

## Implemented candidate

The client adds generic per-container FULL/PARTIAL/STOP observations and ordered
main-modal open/close facts on successful packet application for both revisions.
Host snapshot bank readiness requires the current bank component to have a full
update after the last close and still be transmitting. The bank session token
advances on observed open/close transitions even when a single drain ends on the
same component. Tests cover both full/open packet orders, empty banks, stale
reopen, unrelated containers, stop-transmit and session clearing.

The host owns a pending Withdraw-X operation with a 3000 ms dialog phase and
4000 ms observed-inventory settlement phase. It emits one count answer after the
actual dialog opens. Pause and Guardian hold freeze monotonic deadlines; Stop
and session replacement invalidate work. A result token travels back through the
existing FlatBuffer snapshot. A composed regression exercises isolate request,
Rust dispatch, delayed count dialog, one answer and posted inventory outcome.
Exact same-plane nearest-booth identity now reaches Rust dispatch and is checked
again before action.

## Remaining corrections

- Named `Banking.open({stand, boothName, boothOp})`, `Bank.openBooth` and OpenStand
  behavior remains incomplete. The candidate still drops supplied options.
  Preserve defaults, walk near the requested stand, and dispatch the requested
  loc/name/action without fallback to another access.
- A rejected Withdraw-X is silently discarded in several host dispatch arms,
  while the shim waits with no deadline. Every rejected request must post a
  failure result, including same-generation stale/missing target/action cases.
- `withdrawXById` still drops `landsAsId`. Enabled Alcher passes the noted output
  ID; settlement must observe that actual delivered identity.
- Count <= 0 must retain the required success/no-op behavior. Invalid positive
  quantities and unsupported operations must still fail honestly.

These are original brief-18 requirements, not new product scope. Corrective Sol
run 1132/session `20260910_165228_335293` uses the verified configured model
`gpt-5.6-sol` / `openai-codex`. It must obtain another same-card Grok 4.5 review.

## Checks and evidence

Sol implementation run 1113/session `20260910_153013_6f733c` used actual
`gpt-5.6-sol` / `openai-codex`. Root captured its terminal results in
`evidence/banking-capabilities/implementation-checks.json`: 83 distinct commands,
including intermediate failures, final repeated checks and original truncation.
Those commands ran during development; timestamps distinguish older failures
from final candidate checks. The final suite outputs record:

- host-play library: 118 passed, plus strict library Clippy.
- selected script suites: library 42, gold stubs 11, load-isolate 147, host-JS 2
  passed with its one ignored regeneration helper; strict all-target Clippy.
- API: 174 passed across the relevant targets; strict all-target Clippy.
- client focused gens/login/revision-289 targets: 16/10/56 passed, strict
  client library/tests Clippy and formatting.
- full client crate suite: 620 passed, one ignored test; workspace check passed.

The broader script command exposed the existing missing STAFF_RUNES casting
capability in catalog_start. That remains required later campaign work; it is
not a banking regression or concurrent catalog product edit. The host-play
integration command without memory-profile lacked the scenario dependency;
world/catalog harness builds require that existing feature. World source is
committed, not untracked WIP.

Actual Grok 4.5 review run 1131/session `20260910_164827_a8604b` independently
reran focused freshness, packet, pending Pause/Stop and composed-result tests,
then requested named-open correction. Receipt: `review-8a60eef7.json`. Root's
additional rejected/noted/zero-request findings are recorded in that receipt's
comments and in brief 18; passing earlier tests does not resolve those gaps.

## Integration boundary

The implementation staged the client gitlink despite its brief reserving that
operation to root. Root verified that the host gitlink exactly matches clean
client 56d8027 on the named client branch, with no unrelated client delta. The
commit is retained for review; this workflow deviation does not grant source
acceptance or authorize future worker gitlink changes. No remote was pushed.

Common loot/withdrawLoad, provisioning/recovery, casting and later families
remain separately scoped. Root owns live bank-return and catalog qualification.
