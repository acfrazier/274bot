# Step-6 banking capability ownership design

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-10. Kind: bounded pre-implementation design/audit of
`docs/compat/briefs/17-banking-capability-design.md` against accepted
operation source. Not runtime evidence, not live acceptance, not
PeriodicBank/DeathRecovery/common-loot/loadout work, not campaign release.

Read once: `AGENTS.md`, `docs/execution.md`, fail-closed-dispatch, plan
step 6 in `docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md`,
`docs/compat/banking-capability-audit.md`, and the named brief. Branch
checked first: `codex/rs2b0t-multirevision` (not `main`). Work was
read-only except this report. No product edits, fixtures, STATE, matrix,
subagents, commit, stash, reset, restore, checkout, merge, remotes, live
sessions, or suite reruns. Concurrent WIP on snapshot reset / nav pack
was left untouched.

## Verdict

**Proceed to the first banking-capability implementation** under the
required constraints below. Design approval is not source acceptance and
not live acceptance.

The first family is implementable on the named pins without a foreign
Banking router, without a universal job framework, and without a
revision/card allowlist:

1. Fresh empty-bank readiness is a **per-container full-inventory packet
   after this open**, owned by the Rust host from a generic client
   publication. Modal visibility, a nonempty list, backpack traffic, and
   all-family session/grant/invalidation cannot identify that packet.
2. Withdraw-X is a **host-owned bounded pending op**: wait for the live
   count dialog, answer once, settle on observed inv/bank change. A queued
   request is not success. Reuse script dispatch + `Interactions`; do not
   extend `PendingBankFetch`.
3. `Banking.open({stand, boothName, boothOp})` and `Bank.openBooth` must
   forward those three arguments to walk-near + named loc interact.
   Honor them when the scene loc exists; refuse missing options
   explicitly. Do not copy `resolveBankOpenRoute` / `BankLocations` /
   `walkOpening`.

Scripts continue to load and start irrespective of revision. Suitability
belongs to the user; host/client operations refuse unsupported
capabilities.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Named host operation source | `b9cacc6e5b3b5101cd2baf6b2a91e9ecf13f84f8` |
| Campaign HEAD at write-up | `b5df9c93723d6b3f94aa37a2f481c698804c4037` |
| Client pin (submodule) | `6cb5a0b17aeef74da6b57b205e916681daee4f76` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief SHA-256 | `a0c2fb58e4beb38d3813797326e91a4da12433abeb8e6eb680b478c8ddd1a20a` |
| Audit SHA-256 | `a954c7bf50d1bcbc3d0d245f291348dad6bd9b7d3a4f48ce2528de4d3601385f` |
| Catalog 100adccc Bank.ts / Banking.ts | `3031b0a6…747` / `4bb9948f…fec` |
| Catalog 8e7d965b Bank.ts / Banking.ts | identical hashes (API files do not differ) |
| Kanban card | `t_36e521e1` run 1110 |

Frozen card call sites only, both archives:

- `100adccc` BankFletcher banks via `Bank.openBooth(stand, boothName, op)`
  then `Bank.openNearest` fallback; Superheater uses
  `Bank.openNearest(name, op)` and `Bank.withdrawX('Staff of fire', 1)`.
- `8e7d965b` BankFletcher banks via `Banking.open({stand, boothName,
  boothOp: 'Use-quickly'})` and adds `mode=auto|cut|string|cut+string`.
  Superheater selects `FIRE_STAVES` (Staff of fire first, then every
  `STAFF_RUNES` entry that supplies a fire rune) and re-checks the bank
  list before declaring none left.

`docs/compat/STATE.md` still treats step 4 live proof and snapshot
review as pending. This review used the brief's `b9cacc6e` operation
source. Uncommitted snapshot/nav files in the worktree were not used as
product.

## Exact contracts (source, not planner bodies)

Identical in both frozen catalogs (`src/bot/api/bank/{Bank,Banking}.ts`).

### Open / ready / empty vs stale

| Fact | Contract |
|---|---|
| `Bank.isOpen()` | `reader.bankComId() !== -1` (withdraw component exists). |
| `Bank.loaded()` | `reader.bankItems().length > 0` (nonempty list). |
| `Bank.snapshotReady()` | current open session + that component is transmitting + a **full** inv packet landed **after this open**. |
| `Bank.ready()` | `isOpen && (snapshotReady \|\| loaded)`. Empty banks can never satisfy `loaded()`. |
| `Bank.snapshotGeneration()` | per-component inv generation, or `-1` when the session/component does not match. |
| `waitSnapshotAfter(g)` | wait until generation `> g` and snapshotReady; default **4000 ms**. |
| `waitReady` | default **4000 ms**. |
| Open completion | `openedReady` waits `waitReady(4000)` then returns `isOpen()` (not `ready()`). |

Host today posts `bank_loaded = bank_component_id != -1 && !bank.is_empty()`
(`crates/host-play/src/lib.rs` around the `SnapshotInput` build). That is
foreign `loaded()`, not `snapshotReady`. Shim `Bank.ready()` is
`isOpen && loaded`, so a fresh empty bank never becomes ready.

Client today bumps one global `gens.inv` for every `UPDATE_INV_FULL` /
`PARTIAL` / `STOP_TRANSMIT` (`vendor/fr-client-rust` `bump_gens` and the
289 apply path). Host `InvIfaceGate` rebuilds **every** item family when
that global pair moves. Session/grant/explicit-invalidation watermarks
from step 4 likewise do not name a component. A backpack full-update,
shop traffic, or retained rows after reopen are therefore
indistinguishable from a new bank snapshot.

Foreign observation (read as contract, not copied into JS): ready is
`transmitting && fullGeneration > openedAt` for the withdraw component
of **this** `IF_OPENMAIN_SIDE` session. `IF_OPENMAIN_SIDE` and the full
inv packet may arrive in either order; `openedAt` is the component's
generation captured at the **previous close**, not "now" at modal
detect. Stop-transmit clears transmitting. Partial bumps generation but
not full-generation, so it cannot make an empty bank ready.

### Withdraw-X

From `Bank.withdrawX` / `withdrawXById`:

| Bound | Value | Meaning |
|---|---|---|
| Count ≤ 0 | success without a send | keep |
| Missing row / no Withdraw-X op / available 0 | false | keep; shim already throws `not impl` when ops lack Withdraw-X |
| Side backpack ready | wait side modal **2000 ms** + 1 tick | first family may keep current "bank open" gate; do not invent a new backpack matcher |
| Fixed 1/5/10 | use that op when present, skip dialog | keep as a fast path **after** snapshotReady |
| Dialog wait | **3000 ms** for `countDialogOpen` | host must wait for the live dialog |
| Answer | one `answerCountDialog(take)` | host `Interactions::answer_count` already refuses `NoCountDialog` |
| Settlement | **4000 ms** until inv count ≥ target or (increased && pack full) | observed change, not the queued answer |
| Close | **3000 ms** | already in shim |
| `Execution.delayUntil` default | **6000 ms** | unrelated; do not reuse as a bank bound |
| Isolate runtime | `RUNTIME_TIMEOUT` **50 ms** / tick, `JOIN_TIMEOUT` **2 s**, `SLOW_TICK` **50 ms** | do not change |
| Host park | `FRAME_MS` **20 ms**, `IDLE_PARK_MS` **600 ms** | do not change |
| Login socket read | **5 s** | do not change |

Shim today queues Withdraw-X, waits **one tick**, queues `answer-count`,
then waits 4000 ms for inv movement
(`crates/script/src/shim/bank.js`). Isolate test
`isolate_bank_withdraw_x_queues_x_op_then_answer_count` currently
encodes that 1-tick answer. Host `answer_count` will refuse if the
dialog is not actually open, so a queued answer is not success and a
late dialog never gets answered.

### Open arguments

`OpenBankOpts` defaults: `boothName = 'Bank booth'`, `boothOp =
'Use-quickly'`. Required for this family: `stand`, `boothName`,
`boothOp`. Not in this family: `obstacles`, `destination`,
`preferNearby`, `nearbyRadius`, NPC/`openNpcAccess`, `openFirst`.

`Bank.openBooth(stand, boothName, op)` walks to `stand` (radius 1) after
a failed click; foreign walk timeout there is **90_000 ms**. Shim
`Traversal.walkTo` / `walkResilient` default is **60_000 ms**;
`openNearestAccess` already uses 60_000. Do not raise walk timeouts in
this family. Do not silently ignore a supplied stand.

Shim today: `Banking.open()` takes no options and calls
`Bank.openNearest()` → `open-booth`. `Bank.openBooth()` ignores
stand/name/op. `OpenStand`'s booth arm hardcodes `Use-quickly` and
ignores name/op. `openNearestAccess` honestly throws `not impl:
unsupported bank access` unless name/op are the default booth pair —
keep that honesty for non-object access; named booth variants must
reach loc dispatch instead of being dropped.

## Ownership split

```
client  generic per-component inv packet facts
        (full / partial / stop-transmit, generation, transmitting)
        + existing slot publication, no world clone

host    GameSnapshot bank session (openedAt / ready / generation)
        + reset_session / reconnect
        + Interactions send
        + one PendingWithdrawX per slot
        + walk-near + named loc open

shim    marshal stand/name/op and withdraw-x args
        await posted ready / inv / op result
        throw not impl for out-of-family options
```

Fail-closed: missing host facts stay `not impl` or false. Do not
recreate `Banking.ts` routing, `BankLocations`, or the JS
`bankInventorySession` object in the shim.

## 1. Per-container publication and host snapshot

### Client seam (generic, not bank-named)

In `apply_update_inv_full`, `apply_update_inv_partial`, and both 274/289
stop-transmit paths, **after** staged slot publication succeeds, record
on that `com_id`:

| Packet | `generation` | `full_generation` | `transmitting` |
|---|---|---|---|
| FULL | +1 | +1 | true |
| PARTIAL | +1 | unchanged | true |
| STOP_TRANSMIT | +1 | unchanged | false |

Expose a borrowed map/view on `Client` (`component → {generation,
full_generation, transmitting}`). Zero extra world copies. Logout /
session start clears the map so reconnect cannot inherit the previous
login's generations. Keep the existing global `gens.inv` bump;
snapshot family rebuild still uses it.

Do not put bank-open policy, withdraw-iop search, or `openedAt` in the
client. Do not add a bot action API.

### Host snapshot / reset

Keep current `bank_component_id` (main modal child whose `iop[0]`
contains `"withdraw"`) and `bank_side` deposit component. Add host-owned
session fields on `GameSnapshot` (skipped in serde, like the gates):

- `bank_opened_at_full: u64` — last **closed** full-generation for this
  withdraw component (0 if never seen).
- `bank_snapshot_ready: bool`
- `bank_snapshot_generation: i32` (`-1` when closed)

On modal/component change:

- Close (`bank_component_id` → `-1`): store current full-generation as
  the next `opened_at`, clear ready, generation `-1`.
- Open or component identity change: `opened_at` = stored previous
  full-generation for that id (0 if absent). Ready becomes true only
  when that component's `transmitting && full_generation > opened_at`.

`reset_session` already wipes views and sets `inv_session_current =
false`. Also drop the bank session and per-component opened_at so
retained iface tables cannot look ready. A later packet for that
component beyond the watermark republishes only that family — same
reset rule as step 4, now with a component id.

Keep `bank_loaded` as nonempty (foreign `loaded()`). Shim `ready()`
becomes `isOpen && (snapshotReady || loaded)`, matching Bank.ts.

Do not treat global `gens.inv`, grant, or all-family invalidation as
bank freshness.

### FlatBuffer compatibility

Append only, preserve existing Snapshot vtable ids (last current field
`shop_stock` = offset 116):

| New field | Offset | Default |
|---|---|---|
| `bank_snapshot_ready` | 118 | false |
| `bank_snapshot_generation` | 120 | -1 |
| `count_dialog_open` | 122 | false |

`count_dialog_open` is already on `GameSnapshot`; posting it is optional
for host-owned Withdraw-X (the pending op reads the host snapshot
directly) but is the smallest observation if a later shim wait needs it.
Omitted fields remain "unchanged" under the existing delta post rules.
Do not reuse `bank_loaded` for snapshotReady.

## 2. Withdraw-X pending op

Do **not** invent a job framework. Do **not** latch this on
`PendingBankFetch` (BankBudget walk/open/deposit/withdraw/wear/close;
Open already pops before a fresh bank). Add one optional
`PendingWithdrawX` on the script slot, pumped from the same observe
path that drains `dispatch_script_interact`.

```
WaitDialog { name, count, lands_as, before, target, deadline = now+3000ms }
  → dialog open? answer_count once → WaitSettle { deadline = now+4000ms }
  → inv count ≥ target or (count > before && pack full) → Done(true)
  → timeout / refuse / bank closed / missing row → Done(false)
```

Rules:

- Shim queues **one** `{op:'withdraw-x', name, count}` (Interact table
  already has `name` + `x`; map `x` as count, same as `answer-count`).
  It does **not** queue `answer-count`. It awaits posted inv change
  **or** a one-shot posted result; it must not succeed on the queued
  withdraw alone.
- Host sends the Withdraw-X inv-button only if `bank_snapshot_ready`
  (or nonempty `loaded` while ready is still climbing) and the live
  row still has that op and count > 0. Stale/absent row → false, no
  send.
- Answer at most once per latch. `NoCountDialog` keeps waiting until
  the 3000 ms dialog deadline; it does not retry-spam.
- Pause: freeze the pending clock and do not send. Resume continues
  remaining budget. Stop, isolate drop, `reset_session`, and
  logout/reconnect abort false and drop the latch.
- Fixed Withdraw-1/5/10, when that op exists, skip the dialog latch
  and only wait the 4000 ms settlement.
- Preserve `answer_count` value < 0 refusal and existing COUNT_DIALOG
  revision mapping from step 4. No new opcode.

Replace isolate test
`isolate_bank_withdraw_x_queues_x_op_then_answer_count` so the isolate
emits a single withdraw-x and does not treat a 1-tick `answer-count` as
the contract. Keep
`isolate_bank_items_do_not_invent_ops_and_withdraw_x_throws`.

## 3. Stand / name / op open

Extend existing `InteractReq::OpenBooth` using fields already on the
Interact table (`x,z,level`, `name`, `action`). No new vtable ids.

| Args | Dispatch |
|---|---|
| none (today's `Banking.open()` / `openBooth()`) | keep `open_nearest_booth()` (nearest Use-quickly, same plane) |
| `stand` only | `walk-near` stand radius 1 (existing 60s traversal bound), then nearest Use-quickly |
| `boothName` / `boothOp` | nearest loc with that name **and** that op; interact that op. Do not fall back to a different loc's Use-quickly |
| both | walk-near stand if not adjacent, then named loc+op |

`OpenStand` booth arm must stop ignoring name/op: if supplied, they
select the loc and operation; if absent, keep Use-quickly. NPC
`stand_op` / `choose` stay as they are (choose still deferred).

Unsupported in this family — throw `not impl: …` with the option name,
do not no-op:

- `obstacles`, `destination`, `preferNearby`, `nearbyRadius`
- `Bank.openNpcAccess` / Gundai dialogue
- `openFirst` (closed chest/door before the booth)
- `Bank.withdrawLoad`, `matchesCommonBankLoot`,
  `depositMatcher.includeCommon`
- PeriodicBank / DeathRecovery execution

`openNearestAccess` keeps the explicit unsupported-access throw for
anything other than default booth name/op **or** becomes the named
loc path when name/op **are** a scene loc. Do not clone
`resolveBankOpenRoute`.

BankFletcher 8e7d965b `mode` and Superheater `FIRE_STAVES` are later
proof inputs for this family's open/withdraw-x, not extra routing.

## Minimal owned files

| File | Change |
|---|---|
| `vendor/fr-client-rust` `client.rs` (+ small Client getter) | per-component inv packet state at publication |
| `crates/api/src/snapshot.rs` | bank session, ready, generation; reset clears them |
| `crates/api/src/interact.rs` | named booth open (name+op); no new opcode |
| `crates/host-play/src/lib.rs` | SnapshotInput fields; OpenBooth args; PendingWithdrawX pump |
| `crates/script/schema/isolate.fbs` + `isolate_fb.rs` | append snapshot fields; `withdraw-x` op on existing Interact ids |
| `crates/script/src/shim/{bank,banking,mod}.js/rs` | marshal args; ready uses snapshotReady; withdraw-x latch; honest not-impl |
| tests: `api/tests/snapshot.rs`, `api/tests/interact.rs`, `script/tests/load_isolate.rs`, focused host-play | empty/stale/wrong-container/late-dialog/one-answer/reset/wrong-access |

No 274bot crates in the client repo beyond the generic inv-state
publication. No catalog allowlist. No JS `invUpdateState` port.

## What remains unsupported (this family)

- Common-loot matching, PeriodicBank, DeathRecovery, loadouts, shop/trade
  production, BankFletcher cut+string live proof, Superheater every
  fire-staff alternative live proof.
- `PendingBankFetch` Open popping before a fresh snapshot — later
  provisioning should wait on `bank_snapshot_ready`; do not fold it
  into this patch.
- NPC banker dialogue, obstacle web-walk, nearby-vs-preset snap radius.
- `withdrawLoad`.
- Live 274/289 bank qualification (depends on step-4 live proof and
  step-5 nav/content where the card walks).

Those are dependencies, not silent success.

## Meaningful tests (source; no live run here)

Must traverse script call → IPC → Rust send/pending → posted result.
Must not snapshot source text.

- Empty FULL (0 slots) on the withdraw component after open: `isOpen`
  true, `loaded` false, `snapshotReady` true, `ready` true. Same on
  274 (`g1` count) and 289 (`g2` count) frames.
- Reopen same component ids with no new FULL: not ready; retained
  nonempty rows are not a fresh snapshot.
- Backpack/equipment FULL must not set bank ready or bump
  `bank_snapshot_generation`.
- STOP_TRANSMIT on the bank component: transmitting false, not ready.
- `reset_session` / logout: ready false, generation -1, pending
  withdraw-x dropped.
- Withdraw-X: delayed `count_dialog_open` then **one** COUNT_DIALOG
  send, then inv publication → true. Answer without dialog → no send.
  Dialog timeout 3000 ms → false, zero answers. Settlement timeout
  4000 ms after answer → false. Pause freezes; Stop/reconnect abort.
- OpenBooth with name+op: loc interact uses that op; a different
  Use-quickly loc is not sent. Wrong/unsupported access → `not impl`
  and no send.
- Existing legal-send / no-clone / 289 mapping tests stay green.
  Do not change FRAME/IDLE/RUNTIME timeouts.

## Live proof that could disprove this (later)

Design approval does not run this. After source review of the
implementation:

1. Isolated local 274 and 289. Wait `ingame && scene_state==2`. No
   public 289.
2. Script-caused `Banking.open({stand, boothName, boothOp})` /
   `Bank.openBooth` → posted `bank_snapshot_ready` including an
   **empty** bank and a **stale reopen**.
3. Script-caused `Bank.withdrawX` with a late count dialog → one
   answer → inv/bank change. A queued withdraw without that change
   fails the cell.
4. Wrong booth name/op refuses. Pause/Stop/reconnect during dialog
   wait does not send a late answer.
5. Both catalog call sites: 100adccc `openBooth`, 8e7d965b
   `Banking.open({…})`. BankFletcher mode and Superheater staff
   alternatives are exercised only as consumers of this family, not as
   new planners.

FAIL + exit 1. Do not extend timeouts to manufacture a pass.

## Distinctions

| Claim | This document |
|---|---|
| Ownership and constraints are implementable | Yes — design approval |
| Empty-bank ready / Withdraw-X / named open already correct | No |
| Source review of the future patch | Later same-card `reviewer` on the implementation card |
| Offline packet/session/pending tests | Implementation gate |
| Local script → IPC → Rust → posted result, both revisions | Later live qualification |
| PeriodicBank / loot / loadout / DeathRecovery | Subsequent families |
| Campaign / 0.1.7 | Not accepted |
