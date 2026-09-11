# Native BankSorter core vs ancillary quest reporting

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 07:28 UTC. Kind: bounded read-only architecture/fidelity
audit for brief 80, plan sections 6–7, and the current imported-script /
ancillary-stub policy. Not implementation, LIVE, fixtures, ledger, dim,
or an enabled-status change. Root owns acceptance and any follow-up card.

Read once: `AGENTS.md`, `docs/execution.md`, fail-closed-dispatch, current
STATE imported-script / ancillary-stub rules, brief 80, frozen catalogs,
and named host/client files at HEAD. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Work was read-only except this
report and `docs/compat/evidence/bank-sorter-capability-boundary/`.
Concurrent working-tree edits on this checkout are not the tested binary.
No product sorting occurred.

## Verdict

**BankSorter core is a missing host mapping, not a foreign-script defect
and not a dim.** `sortBank` is an explicit enabled-card request; the shim
throws `not impl: bankSort.sortBank` before any reorder packet. Named
`ClientProt::INV_BUTTOND` already exists on both revisions. There is no
typed `Send`, no `Interactions` method, no `MiniMenuAction`, and no
pending continuation that settles on bank **order**.

Do not import `bankSortRules` / `bankSortPlan` / `bankSortRank` /
`QUEST_JUNK`. Do not treat those files as inherited product policy.
The smallest independently owned capability is a **host-owned swap
reorder** of the open, loaded bank, using already-posted item identity
and cost, with observed server order change.

Ancillary `reportQuestJunk` / `dropQuestJunk` stay honest stubs. An empty
junk list is not a successful scan. Zero moves is not a successful sort.
Do not enable drop. Do not dim the card because the optional finder is
absent.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `6750713b80ecfb8ad5036f3ffb0564459f67a856` |
| Client gitlink | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 80 SHA-256 | `91f8ca7fee59d94ea5aa0811a9909bf6923ec8d2454e1e9fad34733dfe59daf2` |
| Kanban card | `t_395bc0f4` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| BankSorter.ts (both; `cmp` 0) | sha256 `ec6b320c5d6201999aa63baf227a0207c1309a0cd3158e1968d60ee5a8b5c2ea` |
| bankSort.ts (both; `cmp` 0) | sha256 `450b0fe9a6f13085d059295c4369eca9538b73826e3a2b4683516c27b2b3a235` |
| bankQuestJunk.ts (both; `cmp` 0) | sha256 `f60a19007bc12e4476aece55dfdc5260a7702c31481d9681980cfd94c4205022` |
| `crates/script/src/shim/bank_sort.js` | sha256 `e1bee7c0e30e74d51c66d0b267e2c98b6108c7db22dcff915d1b9a7d160cb728` |
| `crates/api/src/interact.rs` | sha256 `7e2e23fabdcf29d82181c14e0b3f9399e21f0f16ae2c3126b0749ca9141deb06` |
| `crates/api/src/prot.rs` | sha256 `647ef35b4bb26138b325de7bc3da4072fc4cb9b60f448437d1324468d969fedb` |

All seven BankSorter / bankSort / junk sources are byte-identical across
the two frozen catalogs (`cmp` exit 0). There is no catalog split.

## 1. What the enabled card actually does

Frozen `BankSorter.ts` is a one-shot:

1. `Game.openSideTab(2)` then `Quests.all()` (quest tab before bank).
2. `Banking.open()` (no stand). Stop if that fails.
3. If `reportQuestJunk || dropQuestJunk`: `findQuestJunk(reader.bankItems(), …)`
   and log one line. Default report is **true**.
4. If `dropQuestJunk`: withdraw-all + `Drop` per droppable row. Default drop
   is **false**. This audit must not enable it.
5. If `sortBank` (default **true**): `await sortBank({ log, categoryOverrides })`.
6. `Bank.close()` then `ScriptRunner.stop` with `result.reason`.

`BankSortResult` is `{ sorted, moves, mode, unmatched, reason }`. The
card's defining witness is **posted bank order changed and `moves >= 1`**,
then stop. A pre-sorted zero-move bank cannot prove the core. Turning
`sortBank` off, or turning report off, is not proof of the default
report.

Foreign `sortBank` (`bankSort.ts`) is a JS planner, not a host verb:

- Snapshot slots + `objCatalog().cost`.
- `planBankSort` (LIS insert vs swap, `categoryOf`, `rankWithin`).
- `ifButton(8130|8131)` and wait for **varp 304** (`%bankinsert`).
- Up to four `dragInvSlot(bankComId, from, to, mode)` per round.
- Comment in source: the server reads the varp, not the packet mode byte.

Those component ids are 274 `interface.pack` literals. They are not a
revision-safe host contract. The shim already copies `ARRANGE_SWAP_COM =
8130` / `ARRANGE_INSERT_COM = 8131` / `BANK_INSERT_VARP = 304` and then
throws. Unused constants are not an implementation.

## 2. What already exists in Rust

| Fact / op | Where | Enough for core? |
|---|---|---|
| Bank open / loaded / generation / session | snapshot + `Bank.ready` / `snapshotReady` | Yes. Fresh list after this open. |
| Occupied bank rows with `slot`, `id`, `name`, `count`, `def.base_value` | `ItemView`; host-play posts `slot`; isolate JS can see `slot` | Yes for identity and native rank. `Bank.items()` currently **drops** `slot`. |
| Dense-open assumption | `read_inv_component` skips empty slots but keeps `slot` | Foreign planner requires `slot === i` for `0..n-1`. Server compact-on-open is the usual case; gaps must refuse, not invent fills. |
| `Banking.open` / `Bank.close` / side tab 2 / `Quests.all` | already mapped | Yes for the one-shot wrapper. Not the missing reorder. |
| `withdrawById` + held `Drop` | already mapped | Drop path ops exist. Finder does not. Do not arm drop. |
| `ItemDefView` / generated `GameItem.cost` | obj cache + `game_data` | Enough for a host order key. Not a category table. |
| `IfButton` / `press` / `set_toggle` (Note/Item) | `interact.rs` | Arrange Swap/Insert **may** reuse toggle discovery. `press` refuses `client_code > 0`. |
| `PendingBankOp` Deposit/Withdraw | `slot.rs` | **Wrong settlement.** Count change, not order. Do not hijack. |
| Withdraw-X pending | freeze on Pause/hold, abort on Stop/session | Pattern to copy, not the object to reuse. |
| Isolate varps | varp 108 plus up to 31 **nonzero** others | **Not** a reliable `%bankinsert` view. Missing 304 means 0 on the host snapshot API, but the isolate filter can omit it. Read `GameSnapshot::varp(304)` on the host thread. |
| `LEGAL_SEND` `INV_BUTTOND` | 274 id 93 length 7; 289 id 253 length 7 | Named packet. No typed builder. |
| MiniMenu `INV_BUTTON1..5` | withdraw/deposit | Different packet family. Do not send a menu op as a drag. |

Client drag encoding (hud test + `handle_obj_drag`):

```
p1_enc(INV_BUTTOND) p2(com) p2(from) p2(to) p1(mode)
```

`mode == 1` only when `bank_arrange_mode == 1` and the inv component
`client_code == CC_BANKMODE` (206). Varp clientcode 9 writes
`bank_arrange_mode`. Typed `Send` today only knows IF_BUTTON (p2),
CLOSE_MODAL, and RESUME_P_COUNTDIALOG (p4). Count-dialog already writes
through `Driver::out()`. Drag must take that ISAAC path, not a raw
opcode and not `doAction`.

## 3. What is missing (exact public ops)

1. **`Send` + `Interactions::bank_drag(com, from, to, mode)`** for named
   `INV_BUTTOND`. Preconditions: attached, ingame, scene ready, bank
   open+loaded, `com == bank_component_id`, occupied `from`, in-range
   `to`, `from != to`. Mode must match observed varp 304 (0 swap, 1
   insert). A queued request is not success.
2. **Observe / force swap mode** on the host snapshot: `varp(304) == 0`
   before swap drags. If it is 1, press the open bank's Swap control
   (discover from the main-modal tree / varp-304 binding / labels; do
   not hardcode 8130/8131) and wait for varp 304. If the control cannot
   be found, refuse. Do not send mode 0 against insert varp.
3. **Host-owned `sort-bank` continuation** posted as one isolate request.
   Settle on **id-per-slot order**, not item counts. Freeze Pause/hold
   (no isolate wall clock). Abort Stop, session generation change, bank
   close, or deadline. Restore swap mode if this request changed it.
4. **Posted result** matching `BankSortResult` (`sorted`, `moves`,
   `mode`, `unmatched`, `reason`) plus a result seq. Boolean
   `bank_op_result` is the wrong shape.

`categoryOf` / `isUnmatched` / `QUEST_JUNK` stay `notImpl`. They are
foreign policy, not missing packet facts.

## 4. Native sort policy (not inherited)

New host choice, not RS2B0T categories:

**Target order of occupied dense slots: `def.base_value` descending,
then `def.id` ascending.** Cost and id are already on `ItemView.def`
from the selected cache. No name regex, no metal ranks, no quest
overrides, no LIS, no world copy.

Why this and not `categoryOf`: the foreign rule table is a large
imported planner. Plan section 6 forbids reimplementing foreign banking
planners in JS; the stub policy forbids absorbing that project. Cost+id
is enough to move a mixed unsorted bank (coins 995, logs 1511, law 563,
bones 526) and to observe a different posted order.

`categoryOverrides` from `questCategories(found)` is ignored. Ignoring
is not quest-filing success. `unmatched` is always `[]` because this
capability does not file categories.

Swap-only. Insert mode + LIS is broader (section 7).

If the bank is already in that order, return honest `{ sorted: true,
moves: 0, reason: 'sorted' }`. That is a **fixture** failure for core
proof, not a capability pass.

Gaps in slot indices, bank not open, or snapshot not ready: no packet,
honest reason (`bank not open` / `snapshot not ready` / `bank not dense`).
Do not compact locally and pretend the server did.

## 5. Ancillary quest reporting / drop

`findQuestJunk` throws `not impl: bankQuestJunk.findQuestJunk`.
`QUEST_JUNK` in the shim is `[]`. The frozen table is two rows (`Rats
tail` 300 / `Witch's Potion`, `Stake` 1549 / `Vampire Slayer`). Small
does not make it in-scope: brief 80 forbids importing the quest catalog,
and STATE says features that need foreign world/policy stay stubs until
a separate native project.

Caller result shape is `QuestJunkFinding[]`. Returning `[]` is a
successful scan of none. That is a lie while the finder is unimplemented,
including when the seeded bank happens to lack those two ids.

`dropQuestJunk` default is false. Drop uses existing withdraw + Drop.
Without an honest finder there is nothing truthful to drop. This audit
does not enable drop and the implementation card must not.

Default `reportQuestJunk=true` means the **default Start path calls the
stub before `sortBank`**. That must not dim the card. It also must not
be papered over:

- Do not return `[]` from `findQuestJunk`.
- Do not treat `reportQuestJunk=false` as proof of the default report.
- Do not invent a new Start-gate framework in the sort card (no such
  general settings-refusal seam was found beyond unloadable/dim and
  unknown-setting preflight). Keep the honest `notImpl` throw.
- Core LIVE for this capability uses explicit `sortBank=true`,
  `reportQuestJunk=false`, `dropQuestJunk=false`. Label it sort-core.
  Default report stays unproven until a later native junk project.

`Quests.all` after side-tab 2 is a first-family mapping, not the junk
catalog.

## 6. One bounded implementation task

**Title:** Host-owned bank swap reorder for `sortBank` (swap-only).

**If in scope:** yes, plan section 6 missing host capability. Not a
foreign dim.

**Owned files (suggested):**

- `crates/api/src/prot.rs` — typed `INV_BUTTOND` write (`p2 p2 p2 p1`)
  on `write_for_revision`.
- `crates/api/src/interact.rs` — `bank_drag`; optional swap-mode press
  via existing button dispatch / `set_toggle` style, not `press` if
  `client_code > 0` blocks.
- `crates/api/src/snapshot.rs` — only if Swap/Insert controls need a
  `bank_note_controls`-style discovery. Do not hardcode 8130/8131.
- `crates/api/tests/interact.rs` — 274 and 289 encodings; refuse paths.
- `crates/script/src/shim/mod.rs` — `InteractReq::SortBank`.
- `crates/script/src/shim/bank_sort.js` — thin queue + `delayUntil(..., 0)`
  wait on result seq. Keep exporting the unused constants or delete them
  in that card; do not use them as ids.
- `crates/script/src/slot.rs` — new pending sort continuation (not
  `PendingBankOpKind::{Deposit,Withdraw}`).
- `crates/script/src/isolate_fb.rs` + snapshot apply — result fields.
- `crates/host-play/src/lib.rs` — dispatch, Pause/hold freeze, Stop abort,
  settle on posted order.

Do not edit frozen catalogs, `bank_sort_rules.js`, `bank_quest_junk.js`,
ledgers, STATE, or remaining-production.md in that card.

**Operation / result contract:**

```
sortBank(opts) -> BankSortResult
  bank closed -> {sorted:false, moves:0, mode:null, unmatched:[], reason:'bank not open'}
  not loaded -> reason 'snapshot not ready'
  not dense 0..n-1 -> refuse, no packet
  already native-ordered -> {sorted:true, moves:0, mode:null, unmatched:[], reason:'sorted'}
  success -> {sorted:true, moves>=1, mode:'swap', unmatched:[], reason:'sorted'}
  closed/aborted/timeout -> sorted false, honest reason, no fabricated moves
```

`opts.log` may be ignored. `opts.categoryOverrides` ignored. No mutation
without this request.

**Meaningful negatives (not source snapshots):**

- Closed / not loaded / Pause freeze / Stop abort / session generation
  change: no further `INV_BUTTOND`.
- `from == to` or empty `from`: no packet.
- Insert varp stuck and Swap control missing: refuse, no drag.
- 274 payload id 93 length 7; 289 id 253 length 7; bytes
  `com, from, to, mode=0`.
- `findQuestJunk` still throws; never `[]`.
- Deposit/withdraw pending still settles on counts, not order.

**Controlled local LIVE (implementation card, not this audit):**

Private unsorted bank at Varrock East (3253,3420,0), both revisions, one
frozen catalog is enough (sources identical). Seed e.g. coins 995, logs
1511, law rune 563, bones 526 in an order that is **not** cost-desc/id-asc.
Empty pack. Preserve original inventory/history. Settings:
`sortBank=true`, `reportQuestJunk=false`, `dropQuestJunk=false`. Witness:
`sortBank` returns `moves >= 1`, posted bank id order changed, script
stops. Already-sorted or seed-only contents fail the fixture. Root
qualifies default-report separately.

## 7. Broader than this core (do not absorb)

| Topic | Why it exceeds authorized core |
|---|---|
| Foreign `categoryOf` / `rankWithin` / LIS insert planner | Imported sorting policy. Core witness is any observed reorder under a stated host order, not RS2B0T tabs. |
| Insert mode as a required path | Same `INV_BUTTOND` byte, but needs confirmed varp 304 == 1, local bubble, and a planner that prefers insert. Swap-only proves order change. |
| Hardcoded 8130/8131 | Cache layout, not a protocol fact. 289 may differ. Discover from the open modal. |
| Quest-junk table + drop | Separate native project under the stub policy. Empty list is a lie. Drop is destructive and stays opt-in off. |
| JS `dragInvSlot` optimistic local swap | Client-owned echo. Host should send the named packet and wait for the bank list, not clone `IfType.swapSlots` in the isolate. |
| New Start-gate framework for default report=true | No existing general settings-refusal seam for this. Honest `notImpl` plus an explicit sort-core cell is the bound. |

## Stopping rule

This design is done. Implementation is a later bounded card with
same-card `reviewer`. Stop that card when `sortBank` no longer throws,
swap `INV_BUTTOND` is revision-correct, Pause/Stop/session negatives
hold, and one unsorted local bank shows `moves >= 1` with a changed
posted order.

Do not, in that card: clone foreign rules, return empty junk, enable
drop, dim BankSorter, invent a packet, sort before the script asks, or
call default `reportQuestJunk=true` proven because report was off.

## Operator disposition, 2026-09-11 17:49 UTC

The operator explicitly chose to keep BankSorter unavailable pending a separate native sorter. The proposed value-then-ID policy is not authorized for implementation in this campaign. Record `BLOCKED: missing native bank sorting`, preserve the original catalog inventory and current honest sortBank refusal, and do not classify this as a foreign-script defect or a successful zero-move sort. This closes the campaign policy question without a sorting implementation or LIVE acceptance claim.
