# First banking capability family

Current status (2026-09-10 21:43 UTC): candidate 75514ee3/client 56d8027
passed same-card Grok 4.5 review 1134, but root acceptance remains withheld.
The corrections below resolved named-open, rejected-result, noted-ID and zero
quantity findings. Root then found a composed Pause/hold bug: the shim's new
8000 ms wall timer can expire while Rust's pending operation is frozen. The
matching/fill family owns its correction and explicit session-abort results.

Direct fixed-transfer bank-return cells passed on both revisions
(03-world-capabilities.md). The first headed BoneBurier run exposed missing
ordinary bank travel and an undefined Bank.withdraw return value. Its original
first-burial PASS is not full-loop acceptance; see 05a-catalog-live-harness.md.

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

## Corrections and remaining work

Sol 1132/session 20260910_165228_335293 corrected named stand/name/op forwarding,
exact loc selection, rejected Withdraw-X outcomes, noted landsAsId settlement,
and zero quantity no-ops in 75514ee3. Actual Grok 4.5 reviewer run 1134/session
20260910_171431_af8915 approved those paths with composed and focused checks;
its verified metadata is evidence/banking-capabilities/review-run-1134.json.
That review did not catch the outer Pause timer conflict described above.

Sol t_41b2f50e owns that conflict, session aborts, common-loot matching and
withdrawLoad. Task t_90b60f12 follows its review to complete planned Rust-owned
nearest-bank routing and honest Bank.withdraw dispatch results. This behavior
is required by the frozen API; the user confirmed retaining the plan after
root explained its existing travel fallback. The original BoneBurier fixture
continues to start on the mainland, with bank stock prepared before Start.
Root's strengthened observer requires depletion, fresh bank stock, withdrawal,
closed bank and a second burial. Neither queued work nor a first burial is a
passing catalog result.

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
