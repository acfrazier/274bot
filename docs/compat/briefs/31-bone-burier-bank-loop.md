# Complete the real BoneBurier bank path

Use configured `sol` defaults after matching/fill t_41b2f50e completes review,
then request same-card `reviewer` and stop. Campaign branch is
codex/rs2b0t-multirevision. Read applicable instructions, docs/execution.md,
fail-closed-dispatch and plan step 6. Root owns scenario/live harness changes.
Never move, restore or stash shared WIP; export exact sources for checks.

The matching/fill implementer incorrectly closed t_41b2f50e without review.
The real prerequisite is now corrective review t_d68ee0fc. Root reclaimed
premature run 1137 before source edits; resume only after that gate completes.
Use new commits for shared-branch corrections, including generated files.

The next headed BankFletcher run exposed a related required deposit defect.
At f2b04198/client 56d8027/catalog 100adccc on Mac 289, it cut the initial
logs, opened the bank, then the slot panicked writing beyond the 5000-byte
packet buffer. The original panel exited 0 after its earlier XP-only PASS;
root classifies the run as failed. See evidence/catalog-headed/
r289-bank-fletcher-100adccc-f2b04198.log and the read first.png capture.
Root source diagnosis: Bank.depositAllMatching queues one named request for
each equal bank-side row, and Rust InteractReq::Deposit loops over every
matching row for each request. Twenty-seven identical rows can multiply into
729 Deposit-All writes in one dispatch batch. Confirm this through the real
script -> Rust boundary; do not paper over it by growing the packet buffer.
Complete bounded, observed depositAllMatching/depositInventory/depositAllExcept
behavior in Rust, with thin JS predicate selection. Preserve actual foreign
side-view readiness (1200 ms), per-item settlement (2000 ms), 32-step guard,
callback/matcher semantics, Pause/hold/Stop, and fresh slot/id identities.
One Deposit-All of an item must not multiply over every duplicate row. Cover
27 duplicate rows, mixed item types/kept tools, empty/loading view, refusal,
partial progress and session cancellation. This is the same ordinary bank
transfer family required by the existing enabled cards.

Root's count-dialog dismissal is at 76d61beb and in corrective review; preserve
it. Root owns the separately observed ScriptRunner.stop propagation fix in
load.rs/slot.rs and headed panic/error reporting; coordinate any narrow shared
file hotspot rather than restoring work. Do not broaden deposit fixes into
new packet storage or an imported JavaScript controller.

The operator observed the headed 289 BoneBurier run at frozen host e7915812,
client 56d8027, catalog 100adccc. It buried five supplied bones, then stayed
at Lumbridge (3220,3220,0), repeatedly logging `could not open a bank`.
The original scenario's first-burial PASS is insufficient and root has
withheld catalog acceptance. Screenshot/log/receipt are under
evidence/catalog-headed/r289-bone-burier-100adccc-e7915812.*.

Root confirmed two source defects:

- Banking.bankNearest delegates to Banking.open -> Bank.openBooth, which
  sees only the current scene's nearest_booth. It never routes to an off-scene
  bank. The real BoneBurier caller requires travel, opening, fresh stock,
  withdrawal and continued burial. Implement nearest-bank selection and route
  execution in Rust using the selected NavWorld bank stands and existing
  Traveller/host routing. Keep the JS name/argument/result mapping thin.
- Bank.withdraw queues a request and returns undefined; the real card tests
  `if (!(await Bank.withdraw(...)))` and therefore never counts its bank trip
  or exhausted stock correctly. Return honest host dispatch acceptance (or
  the existing observed result where contract-compatible), including false
  on refusal; never return unconditional true merely because JS queued it.
  Inspect withdrawById through the same actual dispatch boundary.

Keep nearby and named-stand opening semantics from the reviewed banking family.
Preserve existing bounds: route 120000 ms where this API requires it, bank
readiness/open bounds, and caller withdrawal observation at 4000 ms. The Rust
operation must freeze on Pause/Guardian hold, abort on Stop/session replacement,
resolve rejected requests, and reject stale exact loc/name/op identities. Do
not clone the foreign Banking.ts router/table or reuse BankBudget in a way
that silently deposits all inventory or closes the requested bank session.
Support this ordinary off-scene booth path using both selected worlds; a
missing bank/world/route must return an explicit failure, not a scene-only
success or an unrelated fallback. Retain required option behavior already
implemented; report other missing bank access options concretely.

Own only required api/script/host-play/nav source and composed regressions,
plus 04-capabilities-bank-routing.md and evidence/bank-routing. Root owns
scenario, panel/TUI, catalog_boundary_live.rs, support ledger, STATE and all
LIVE. No client/engine/profile/gate edits or repo hygiene. Use exact frozen
BoneBurier calls from both catalog roots for a composed offline proof: no
scene booth -> Rust bank route -> fresh open -> accepted/refused withdraw
result; also nearby-bank, no route, stale bank and lifecycle cancellation.
Run existing focused banking and navigation regressions appropriate to edits.
Hand the committed candidate and evidence to reviewer on this same card.
Root will run the headed full loop from Lumbridge with stock seeded only
before Start, requiring depletion, bank travel/open/withdraw, close and a
second burial cycle. No first-burial-only acceptance.
