# Periodic banking with observed deposit and return

Use configured `sol` defaults after the preceding shared capability review.
Campaign branch: codex/rs2b0t-multirevision. Read applicable instructions,
docs/execution.md, fail-closed-dispatch, plan step 6 and the PeriodicBank section
of 04-provisioning-recovery-design.md. Match actual enabled frozen callers in
both catalogs; do not recreate foreign Banking.bankNearest or its router.

Implement non-Off PeriodicBank through a small Rust-owned service, using the
accepted fresh bank, named access, matching, deposit and navigation operations.
The shim preserves the existing class/options ABI and marshals settings and
script callbacks. Off construction and validate=false remain harmless. Rust
parses Off/Loot count/Time/Either, evaluates the actual threshold and loot count,
suppresses banking in combat, and preserves 180000 ms failure backoff. Verify
the frozen timing/reset contract rather than assuming shim token `loot` equals
the source token `items`.

Sequence the required destination/nearest access, fresh bank readiness, observed
deposit, script-owned afterDeposit, close and actual return within radius 6.
Honor commonJunk and the caller's own deposit/keep predicate. Callback execution
belongs to the script and must be acknowledged before the Rust service moves
on. A queued open, deposit or walk is not completion. Preserve the existing
60000 ms host walk bound, 4000 ms bank waits and all current command ordering;
do not adopt the foreign 120000 ms router timeout. Missing access stays explicit.

Pause/Guardian hold freezes clocks and sends; Stop, isolate drop and actual
session replacement abort pending work and prevent callbacks/results from an
old operation affecting a new one. Use a generation/token where needed rather
than elapsed game ticks as wall-clock time. Keep PendingBankFetch ownership
separate and retain its accepted bank-return semantics.

Allowed: script shim/runtime/service/wire and focused tests, minimal API and
host-play plumbing required for the same family. No loadout redesign, client,
nav router rewrite, frontend, engine, operator settings, LIVE or gate change.
No main, remotes or gitlink. Do not implement DeathRecovery in this task.

Composed proofs must exercise Off with zero sends; loot/time/either trigger;
combat suppression; exact supplied destination; common junk true/false;
observed deposit followed by afterDeposit and observed return; refusal/backoff;
Pause/Stop/session replacement without late sends. Include the actual enabled
ChickenKiller or Ardy/RockCrab call shape. Reuse existing tests proportionately.

Deliver 04-capabilities-periodic-bank.md and raw focused checks in
evidence/periodic-bank-capabilities/. Commit scoped files, request same-card
profile `reviewer` review, then stop. No agents or live qualification claim.
