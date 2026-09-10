# First banking capability implementation

Use configured `sol` defaults, then hand this same card to `reviewer` and stop.
Read applicable AGENTS/execution, fail-closed-dispatch, plan step 6,
`04-banking-design.md` and `banking-capability-audit.md`. Host branch is
`codex/rs2b0t-multirevision`, current accepted source baseline `fede3c0d`;
client branch `codex/bothost-274-289`, baseline `6cb5a0b17aeef74da6b57b205e916681daee4f76`.
Root is independently completing step-4 live proof using an immutable source
export. This task prepares step-6 source; it cannot claim live acceptance.

Implement only the coherent first banking family approved by Grok 4.6 on
`t_36e521e1`: bank packet freshness/readiness (including empty banks), bounded
host-owned Withdraw-X, and named booth/stand arguments. Thin JS marshals and
awaits Rust facts/results. No foreign Banking router or universal job framework.

## Required corrections and evidence boundaries

Pending-operation clarification for final implementation and review: Pause
retains and freezes the Withdraw-X operation; Stop, disconnect/reconnect and
isolate reset abort it. Use monotonic elapsed time for the existing 3000 ms
count-dialog and 4000 ms observed-settlement bounds, not a fixed assumption
about PLAYER_INFO cadence. Retained old bank contents do not authorize a send.

The design is guidance, not infallible source. Its `campaign HEAD` is an older
snapshot. Preserve the corrected session/scene publication in current source.
Resolve these semantic hazards explicitly in implementation and tests:

- Actual per-container FULL/PARTIAL/STOP facts must follow successful packet
  application for 274 and 289. Generic client facts only; bank policy stays in
  Rust host. Preserve global family generations and existing wire/framing.
- A full update before open and open before full must both work. Close/reopen
  of the same component within one client drain must not be lost merely because
  the final modal id equals the previous id. Identify the smallest generic
  packet observation seam needed; do not guess from unrelated generation bumps.
- Retained nonempty rows must not make an old bank snapshot fresh or authorize
  a withdraw. Keep foreign `loaded` as a distinct observation if required for
  compatibility, but never use the design's suggested `loaded` fallback to
  bypass stale-session rejection in Rust. Wrong-container traffic and reset
  invalidations cannot revive old bank rows. No all-world deep copies.
- Withdraw-X waits up to the existing 3000 ms dialog bound, answers once, then
  waits up to the existing 4000 ms settlement bound. Positive observed outcome
  requires inventory progress; an enqueued send is not success. Keep fixed
  1/5/10 fast paths where supported. Refusal, timeout, missing/stale rows,
  bank close, Pause/Resume, Stop, reconnect and late isolate results must have
  bounded, honest behavior. Do not change runtime/login/frame/walk timeouts.
- Preserve the existing `Bank.withdrawX` count<=0 behavior and real item/count
  semantics, including noted output where supported. A posted operation result
  should use the smallest existing-compatible representation; append any new
  schema fields without repurposing existing ids or delta semantics.
- `Banking.open({stand, boothName, boothOp})`, `Bank.openBooth`, and the booth
  arm of `OpenStand` must carry stand/name/op to existing Rust walk-near and
  named loc interaction. No silent option dropping or fallback to the wrong
  access. Retain the default no-argument behavior and explicitly refuse other
  unsupported options. Read only relevant frozen callers in both full-hash
  catalog roots; never modify those inputs.

## Ownership

Allowed: generic client inventory/interface observation seams and focused client
tests; `crates/api` snapshot/interact and affected tests; `crates/host-play/src/lib.rs`
script dispatch/pending-operation/posting paths and focused tests;
`crates/script` schema/FlatBuffer transport/shim and affected tests. Existing
route service plumbing may be reused, but no nav algorithms or bake/profile
changes. Root retains host gitlink integration; commit the client on its named
branch, report its exact commit and leave the gitlink for root.

World worker `t_13a850d5` owns crates/nav, Cargo.lock, host-play/profile.rs,
api/content.rs and host/random.rs. Do not edit/stage those. If its control audit
requires shared snapshot edits, coordinate through root before either writes.
Do not edit root live harness, frontends, external engines/content, remote
hosts, fixture accounts, STATE, active plan or support matrix. No gameplay or
server restart and no temporary 289 operation-gate removal.

You may stage and commit only your scoped files on the named branch. Never
stash/reset/restore/checkout, stage other WIP, merge or touch remotes. Reviewers
inspect named commits/source exports without moving any shared WIP or index.

## Completion

Use meaningful packet and lifecycle regressions, including both packet orders,
fresh empty bank, stale reopen, wrong-container update, stop-transmit, successful
response-15 reset, delayed dialog/one answer, timeout, Pause/Stop/reset and wrong
access. At least one composed test must traverse script call -> IPC -> actual
Rust operation/pending state -> posted result. Existing isolate queue-only tests
must not be presented as that composed proof. Run focused affected crate and
separate client integration tests, format and strict focused Clippy. Keep raw
failure and passing logs under `evidence/banking-capabilities/` and report
`04-capabilities-banking.md` with exact commits and remaining live work.

Common-loot, loadout/provisioning, PeriodicBank/DeathRecovery, shop/trade and
production are subsequent bounded families. Missing capabilities remain
`BLOCKED: missing <op>`; they are not accepted options. Scripts load/start
irrespective of revision: users decide suitability, operations refuse missing
support. Commit scoped implementation, request same-card review from profile
`reviewer`, then stop. Root owns final live proof and campaign acceptance.
