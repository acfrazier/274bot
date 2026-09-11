# Inventory production catalog fixtures

Implement with profile grok46 defaults after t_1a24382e source review. Read
AGENTS.md, docs/execution.md, the active completion plan steps 6-8, this brief,
and sections 6, 8 and 16 of fixtures/remaining-production.md. Verify the named
campaign branch. This is the next bounded harness extension in the authorized
45-card campaign. Root owns LIVE and the support ledger.

Own only crates/scenario/src/{lib,proof}.rs if needed,
crates/host-play/tests/catalog_boundary_live.rs, and concise report
05d-inventory-production-fixtures.md. Add shared registry scenarios and strict
independent witnesses for DartFletcher, HerbCleaner and GemCutter using the
existing production Start/observation machinery. Do not add a second runner,
modify runtime operations, either catalog, selected engine, or UI. Preserve
accepted cases, exact-ID helpers and timeouts. Missing operation is an explicit
BLOCKED finding with source evidence, not a successful fixture.

Add six cases, each applicable to both frozen catalogs and revisions:
- dart_fletcher: default Bronze; dart_fletcher_iron: Iron tier.
- herb_cleaner: default-all with eligible unidentified guam stock;
  herb_cleaner_named: explicit Guam leaf filter with an additional eligible
  higher herb in bank, proving it remains untouched across a bank cycle.
- gem_cutter: default-all with uncut sapphire stock; gem_cutter_named: explicit
  sapphire selection plus another eligible uncut gem in bank that stays there.

Confirm actual setting labels, item aliases/IDs, skill requirements, quest or
server conditions against frozen scripts and selected generated/server data.
The fixture architecture is a design lead, not authority to guess item IDs or
waive a server prerequisite. Query individual generated rows; do not dump full
JSON. If a name above differs from the actual option surface, preserve the
intent and report the exact setting. Seed levels legitimately through existing
local commands before Start. Keep all seed acknowledgements and baseline prior
to Start. No seeded output and no fixture mutation after Start.

Darts have no bank cycle. Require exact output ID, both input IDs consumed,
Fletching XP after Start, and further product progress across multiple observed
actions; seeded output, one queued useOn, wrong-tier output, or XP alone fail.
Herb Cleaner and Gem Cutter must produce exact outputs with the corresponding
skill XP, deposit those script-created outputs in a fresh loaded bank, restock
input from the pre-Start bank stock, close bank, then make another product with
further XP. Chisel preservation for gems and exact unidentified-herb IDs matter;
display names alone are insufficient. Select bounded stock/counts that trigger
the natural production bank cycle within existing scenario windows, rather than
seeding an already-completed result or reducing the product behavior. Preserve
before/after inventory and bank count proof, including untouched filtered stock.

Tests should exercise these witnesses with meaningful rejection cases: seed-only,
wrong item ID, missing ingredient use, XP-only, absent post-deposit restock,
stale/closed bank observations, missing next production, and filter violations.
Reuse accepted observation helpers. Run focused scenario/harness tests and
formatting/strict affected checks from exact source if concurrent work requires
it. Keep reports concise and do not claim LIVE. Commit only owned paths with
commit --only. Request review on THIS card with profile reviewer and exact
commits/checks, then STOP; do not self-complete. Root runs twenty-four bounded
catalog/revision cells after actual Grok 4.5 review.
