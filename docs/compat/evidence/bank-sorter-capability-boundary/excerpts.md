# Source excerpts used by 04l

Frozen BankSorter.ts (both catalogs) defaults and core call:

- sortBank default true, reportQuestJunk default true, dropQuestJunk default false
- findQuestJunk before sortBank when report || drop
- sortBank({ log, categoryOverrides: questCategories(found) })
- BankSortResult.moves painted; stop with result.reason

Shim today:

- bank_sort.js: throw notImpl('bankSort.sortBank'); unused 8130/8131/304
- bank_sort_rules.js: CATEGORY_ORDER=[]; categoryOf/isUnmatched throw
- bank_quest_junk.js: QUEST_JUNK=[]; findQuestJunk throw

Named packet already in client:

- 274 ClientProt::INV_BUTTOND id 93 length 7
- 289 ClientProt::INV_BUTTOND id 253 length 7
- handle_obj_drag / hud.rs: p2(com) p2(from) p2(to) p1(mode)
- MiniMenuAction has INV_BUTTON1..5 only, not INV_BUTTOND
- api::Send has no INV_BUTTOND constructor
- PendingBankOpKind is Deposit | Withdraw only

Host snapshot already has ItemView.slot and def.base_value. Bank.items()
in bank.js omits slot. Isolate varp post is 108 plus 31 nonzero; do not
use it as %bankinsert.
