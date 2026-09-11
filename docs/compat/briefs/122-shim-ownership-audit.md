# Audit the thin-JavaScript shim ownership contract

The operator explicitly asked how well the contract of not reimplementing the
JavaScript API in JavaScript held. Use grok46 defaults. Read AGENTS, execution,
fail-closed-dispatch skill and the plan's ownership boundary. Audit committed
campaign b2bd5023489ab2e0b6ba690f228217e8984ac91b..64d73f337 only; ignore later
concurrent WIP. Own only09-shim-ownership-audit.md and evidence/shim-ownership-audit/.
No product/source edits, build, LIVE, Git remotes, merge or push.

Review all changed crates/script/src/shim/*.js (32 JS files, about1739 added /
211 deleted across JS at this snapshot), trace native counterparts as needed.
Distinguish data/shape/filter projections, invoking user callbacks and dispatching
Rust-owned phase commands from newly implemented game-policy/controllers in JS.
Root spot-check found cake_stall.js:stealCakes now chooses walk/steal/wait/stocked/
aborted decisions in JS; bank.js openBooth/openNearest/openNearestWorld also
contain travel/open sequencing and deadlines. PeriodicBank and DeathRecovery
have Rust next-step controllers plus JS execution/callback loops: assess actual
ownership rather than treating every loop as automatically a violation.

For each actionable finding, identify whether inherited on main or introduced
here; exact current/source methods, native capability already available or
missing, minimum bounded Rust ownership correction and meaningful regressions.
No foreign JS body cloning, whole-router/runtime port, speculative redesign or
new feature policy. Preserve current observable behavior and refusal semantics.
An ancillary stub remains a stub until a separate native feature exists.
Report candidly whether the strict contract held; do not label executable JS
policy a thin mapping merely because its packets eventually go through Rust.

Return a concise sorted finding list and safe boundaries for incremental merge.
Do not choose Git staging/merge mechanics: root owns repo hygiene. Root already
created separate stage1 for the client/profile/protocol/world foundation before
these later helpers. Commit report/evidence only and complete; no routine second
review card is needed for this read-only audit.
