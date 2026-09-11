# Map the actual posted native inventory into ClientAdapter reader.inventory

Use grok46 defaults; bounded implementation. Read AGENTS.md, docs/execution.md,
fail-closed-dispatch and this brief once. Own ONLY
crates/script/src/shim/client_adapter.js, a NEW unique integration test
crates/script/tests/client_adapter_inventory.rs, report
 docs/compat/04y-client-adapter-inventory.md and unique
 docs/compat/evidence/client-adapter-inventory/. No load.rs/api/host/nav/other
shim edits, no foreign sources, no fixtures, LIVE or cache cleanup. Previous
count-dialog/local-walk owners completed review. Preserve those mappings.

Root28d97f38/client9d090ed new isolated headless old-catalog MuleCrafter fails
on both revisions with tick45/46 not impl: reader.inventory. Raw:
docs/compat/evidence/catalog-harness/live/r274-mule-crafter-100adccc-28d97f38.log
and r289 equivalent. Native289/newer same failure observed in panel with actual
Falador East bank and held talisman. Named-bank repair got beyond prior alias
failure; the native inventory reader is now the bounded missing boundary.

Audit frozen adapter ClientAdapter.ts inventory()/InvItemSnapshot/readInvComponent,
actual runtime/producers.ts caller (Mule imports producer through its logic),
and our posted item rows in load.rs and api snapshot ItemView. Implement a thin
mapping onto posted snap().inv with source identity: slot,id,name,count,ops,
comId from native component_id. Do not copy foreign client inspection or producer
policy. Inventory means the real ordinary inventory container; do not substitute
bank, equipment or bank-side rows. Preserve actual slots, item0 if native-valid,
notes, counts, null/absent semantics and available ops without invented actions.
Audit contract for missing component identity; use honest unavailable sentinel
or fail closed as required, never a hardcoded valid component. Fresh result rows
and ops must not let foreign callers corrupt the shared posted snapshot. No
world-sized copying, JS object catalog or new native gameplay behavior.

Focused meaningful tests: sparse slots+duplicate names/distinct IDs, counts and
component identity/ops, actual empty inventory vs bank contents, caller mutation
isolation, snapshot delta retention and explicit replacement/clear, two isolate
separation. Reuse existing test helpers if possible without editing others tests.
Preserve all client_adapter existing methods; do not expand unrelated readers.

Export exact committed host/client source plus only owned overlay for checks;
record source hashes and exclusively named Cargo target. A previously owned
compiler cache may be reused with accurate metadata (not independent empty start).
Disk is constrained; do not create multiple full caches or touch other owners.
Run focused script test and strict affected Clippy, syntax/diff check. Commit
only named owned paths and verify actual commit. Same-card kanban_request_review
reviewer then stop. Root owns full live reproduction and remaining gates; mapping
success alone does not accept Mule or its onPaint/core loops.
