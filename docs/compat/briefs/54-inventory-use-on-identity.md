# Preserve selected inventory identity through useOn

Implement with configured sol defaults. Read AGENTS.md, docs/execution.md,
fail-closed-dispatch and this bounded correction to the authorized catalog
compatibility plan. Verify codex/rs2b0t-multirevision. Root owns LIVE/ledger.
No changes to frozen catalogs or engine/client protocol are needed.

Concrete failure at reviewed host d4f1809e/client56d8027: bank_fletcher_string
274 headless and 289 native reach Start and make one strung bow, then spam
Nothing interesting happens until step14 fails (two-pair XP not reached).
Inventory at failure: id849 finished bow in the first slot, id60 unstrung bow
still present, id1777 string still present. Both IDs have display Willow
shortbow in selected data. Longbows likewise share Willow longbow without (u).
Both frozen BankFletcher.ts files correctly select source and target by ID in
instantInput0/instantInput1 -> packItemById -> lastItemById. Do not dim the card:
the operator suggested that only if bad source behavior caused the failure;
root verified the card selects correctly and our bridge loses identity.

Root trace:
- shim/inventory.js held(row) has id and slot, but useOn queues source name
  and target_name only. It discards the actual selected source/target handles.
- InteractReq::UseOn in shim/mod.rs, host_js.rs shape and isolate.fbs/handwritten
  isolate_fb.rs transport preserve only those names and unrelated scene fields.
- host-play/src/lib.rs dispatch finds the first source name and resolve_op_target
  finds the first target name. That can redirect id60 onto id849.

Preserve optional source inventory id+slot and target inventory id+slot through
this existing thin queue, JSON shape, FlatBuffer round-trip and Rust dispatcher.
Prefer explicit optional fields appended to the command schema over overloading
bank fields or scene coordinates. Keep existing field indices/old messages and
normal scene-target behavior. A supplied selected identity must match the
current published inventory and dispatch checks; a removed/replaced slot must
refuse rather than silently retarget a same-name item. Legacy name-only callers
may retain their existing behavior when no identity was supplied. Do not turn
queued true into a new completion protocol or alter ticks/timeouts/ordering.
No deep snapshot copies, entire table clones, or per-tick allocation campaign.

Own crates/script/src/shim/inventory.js, relevant UseOn enum/shape/IPC code in
shim/mod.rs, host_js.rs, isolate_fb.rs, schema/isolate.fbs, and the narrowly
related host-play/src/lib.rs dispatch/resolver. Own focused existing test files
needed to prove this seam plus docs/compat/04b-inventory-use-on-identity.md.
No scenario/proof/catalog_boundary_live edits: t_3317197c owns those. No loadout,
spell/shop/banking policy, new item metadata, UI, nav or client edits. Keep
UseWidgetOn/held interact semantics unchanged unless a required compiler use
needs a mechanical adaptation; do not expand into adjacent feature completion.

Meaningful checks: a shim-produced command actually preserves exact source and
target slot/id over FlatBuffer; composed host dispatch with same-name id849
before id60 uses the selected id60 slot; same-name source selection is preserved;
stale/changed selected slot sends no incorrect packet; legacy name-only and
non-inventory useOn remain compatible. Exercise the existing API stale item
checks rather than bypassing them. Run affected script and host tests with
required features, formatting and strict focused Clippy. Export exact committed
sources for checks rather than touching other workers' WIP. Commit only owned
paths; never unstage peers. Request SAME-card reviewer review with exact source
and checks, then STOP. Root repeats the failed cells and the remaining string /
cut+string matrix after actual Grok4.5 approval. Report no LIVE from this task.
