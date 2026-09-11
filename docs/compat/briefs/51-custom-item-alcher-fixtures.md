# Selected-data custom Alcher live fixtures

Implement this bounded harness extension with configured `grok46` defaults.
Read AGENTS.md, docs/execution.md, the selected-data source/review reports
04-item-metadata.md and 04a-item-metadata-review.md, plus this brief. Check the
campaign branch. Start after bank-option card t_ac56c0b5 has received review,
so its scenario and harness files are stable. Root owns all LIVE execution.

The older alcher_custom cells only select Rune chainbody, which was present in
the old handwritten single-row ITEM_DB. They do not qualify another generated
custom item. Add two named cases using the production frozen Alcher card:
alcher_custom_alias and alcher_custom_name. Select the actual Custom sentinel,
with customItem=adamant_scimitar or customItem=Adamant scimitar respectively.
Both cases apply to both frozen catalogs and revisions. Do not change existing
cases or their historical evidence and do not edit the catalogs or runtime API.

Verified generated rows in both revisions: adamant_scimitar is id 1331, display
Adamant scimitar, cost 2560, certificate_link 1332; cert_adamant_scimitar is id
1332, template 799, link 1331, stackable. Check these selected facts and actual
catalog high-alch behavior; expected coins per cast are 1536. Use the existing
Alcher seed/start flow with a fresh account and one banked target, its required
rune and fire staff setup, and batch size 1. Acknowledge seed stock before Start
using the accepted fresh bank observation. No seeded outcome, no engine edits.

The independent witness must establish the selected custom target was withdrawn
as the expected noted ID, then consumed with a Nature rune into the correct coin
increase and Magic XP after Start. Require fresh scene2 baseline and no seeded
coins/target outcome. An unrelated chainbody cast, a first-item acquisition,
seed-only state or queued send must not pass. Reuse the accepted exact-ID
observation helpers from bank options where appropriate and preserve noted-item
semantics. Keep original deadlines and existing cases unchanged.

Own only crates/scenario/src/{lib,proof}.rs if needed,
crates/host-play/tests/catalog_boundary_live.rs, and the concise report
05c-custom-item-alcher-fixtures.md. Reuse shared scenario wiring so native
catalog_watch can select both names. Keep root ledger files untouched. Prefer
bounded source reads; query specific generated rows instead of printing JSON.

Run meaningful focused witness/seed checks and affected formatting/Clippy.
Commit only your paths with git commit --only, never unstage other contributors.
Hand this SAME card to profile reviewer with exact commits/checks using
kanban_request_review, then STOP. Do not self-complete. Root will run the eight
alias/name × catalog × revision cells after actual Grok 4.5 review.
