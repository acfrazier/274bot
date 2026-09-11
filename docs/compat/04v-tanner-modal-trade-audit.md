# Tanner Trade and modal publication after count-dialog repair

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 09:33 UTC. Kind: bounded read-only architecture/fidelity
audit for brief 96. Not implementation, LIVE, fixtures, ledger, STATE,
or an enabled-status change. Root owns acceptance and any follow-up hop.

Read once: `AGENTS.md`, brief 96, exact host export
`.superpowers/review-exports/catalog-root-78cc99d0` (not current WIP
source), frozen catalogs `100adccc` / `8e7d965b`, and the named live
plus headed receipts. Branch checked first: `codex/rs2b0t-multirevision`
(not `main`). Work was read-only except this report and
`docs/compat/evidence/tanner-modal-trade-audit/`. Concurrent working-tree
edits on this checkout are not the tested binary. Shared host-play /
nav / request files belong to named-bank then special / teleport / shop /
Make-X / fire. They were not edited. Original 5000ms catalog wait is
preserved. Raw failures stay in
`docs/compat/evidence/catalog-harness/` and
`docs/compat/evidence/catalog-headed/`.

## Verdict

**Trade is the right op and Tanner IF 679 did open. The catalog still
times out because `Npc.interact('Trade')` queues immediately from packed
outdoor arrival `3274,3188`, and the existing 5000ms
`reader.modals().main === 679` wait is charged for indoor approach plus
server `if_openmain`. Soft 274/289 then publish real 679 + tan-all 8686
with 27 cowhides and 2000 coins; the script has already logged
`tanner interface did not open` and started the bank walk, so `ifButton`
never runs.** Hard 274/289 follow the same Trade/timeout/bank pattern
but never meet the witness window (`actual_tanner_widget` null). Native
headed 274/8e7d965b is the same race; its snapshot is the fail terminal
on another route (`modals.main = -1`, no Tanner NPC), not the modal
moment.

Do not dim. Do not extend 5000ms. Do not change TannerBot.ts, IF 679,
opnpc3, or `main_modal_id` posting. Missing host approach-to-range
before that wait is charged stays authorized native work.

| | Choice | Applies? |
|---|---|---|
| (a) | Wrong Trade slot / missing IF_OPEN | No. Content `op3=Trade` / `[opnpc3,tanner]` / `if_openmain(tanner)` / pack `679=tanner` match both revisions. Soft witness is 679. |
| (b) | `main_modal_id` absent from isolate | No. `load.rs` posts it; host `reader.modals().main` reads `snap().main_modal_id`. |
| (c) | Unchanged catalog 5000ms too short as a foreign defect | No as ownership. 5000ms is the contract and stays. Host queues from a door-tile that cannot finish indoor OPNPC + IF_OPEN inside it. |
| (d) | Packet/approach order vs modal wait | **Primary.** Packed `walkResilient(TANNER_STAND 3277,3191, radius 3)` Arrives at `3274,3188` radius 0. Interact queues OP_NPC3 immediately. Wait starts. Client pathing reaches `3276,3193` / `3278,3192` at or after timeout. IF 679 lands after the isolate wait has settled false. |
| (e) | Native snapshot proves no modal | No. Headed JSON is fail-terminal after the next bank walk. |

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Frozen host (tested binary) | `78cc99d07046a781fe8cd5ebf66bb63e663d23ea` |
| Campaign HEAD at write-up | `1d4fb9142caf99dcf7d1c02da119710c35ba4e12` (WIP; not the audit source) |
| Client gitlink | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 96 SHA-256 | `469a052f531e113ba80dfb29e61661ba5006ac3b7ad038a7bdcb2429266a364f` |
| Kanban card | `t_59351ef0` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| TannerBot.ts (both; sha256 equal) | `b8038b17ac72eec3cf7b2e84f8323e4cd898f1975de7d210cfba333e552b8d7f` |
| 274 engine | `4c95f87efe00b068cadbd229d94736626907bd1a` |
| 289 engine | `cc359656b4acd216ca452495874b6beba9a0ac75` |
| 274 nav pack | `05db24743e9f549ced16c1f00b87c30a390d3aaec391815da3f3563130b3bcd4` |
| 289 nav pack | `131db92e32eddcb08148909d477589544320fe7e34a422e8004e97e888032924` |
| Headless binary | `c334aae122661a235af1fd13c8028819a42e8e479f7dd4604fe160f399ee0816` |
| Headed binary | `4400a7c1eacf0a1fe9a57354199f53936321d5034f2683294604db9f97459d31` |

Machine-readable copies: `evidence/tanner-modal-trade-audit/{refs,sequence}.json`.

## 1. What the catalog actually waits for

Frozen TannerBot (both catalogs, identical):

1. `walkTo(TANNER_STAND 3277,3191)` — skip if Chebyshev ≤ 4, else
   `Traversal.walkResilient(..., { radius: 3, timeoutMs: 45_000 })`.
2. `Npcs.query().name('Tanner').nearest()`.
3. `await tanner.interact('Trade')` — `Npc.ts` resolves the label on
   posted ops, `Input.interactNpc` queues `{ op: 'npc', action: 'Trade',
   index }` and **returns true immediately**.
4. `Execution.delayUntil(() => reader.modals().main === 679, 5000)`.
5. On false: log `tanner interface did not open — retrying`, `return
   false` from `tanLeg`, which re-enters `bankLeg`.
6. On true (never reached here): `actions.ifButton(8686)` soft /
   `8690` hard (`tanner:com_93` / `tanner:com_98` in interface.pack).

Host isolate parks that cond on `__rs2b0t_pump` after
`post_snapshot` then `on_game_tick`. Cond is evaluated before timeout.
If 679 were on the posted snap during the wait, the wait would succeed.
`main_modal_id` is present in `load.rs` and `client_adapter.js`.

## 2. What the live cells actually did

All four old-catalog headless cells FAIL step 15 (leather id) with 27
cowhide 1739 and 2000 coins 995 still held. No missing-op exception.
Two Trade attempts each. Nav Arrives at `3274,3188` then immediately
`interact Npc Trade` (index 225 on 274, 229 on 289). No `[nav-walk]`
during the 5s wait (client OPNPC pathing, not host nav). Then WalkNear
bank `3269,3167` radius 3.

| Cell | First indoor tile after timeout | Witness 679 | Witness tick / tile | Fail tile / modal |
|---|---|---|---|---|
| r274 soft | `3278,3192` | yes (8686 in widgets) | 115 / `3276,3193` | `3273,3169` / -1 |
| r289 soft | `3277,3191` | yes (8686 in widgets) | 120 / `3276,3193` | `3272,3166` / -1 |
| r274 hard | `3278,3192` | null | — | `3275,3173` / -1 |
| r289 hard | same Trade/timeout/bank | null | — | `3275,3173` / -1 |

Soft witness also has `bank_generation` 7, coins 2000, cowhide 27,
near TANNER_STAND. That is a real open Tanner interface after the
count-dialog repair, not a queued-if-button success. Fail `latest` is
a later bank-route tile with modal closed. A final tile on the next
bank route does not show that the preceding nav never arrived.

Headed 274 / newer catalog (gpu, 85.288s scenario):

- Trade 32.912s → timeout 38.310s (5.398s). WalkNear 38.943s. First
  host nav after that at 40.070s already `3276,3193`.
- Trade 64.122s → timeout 69.485s (5.363s). WalkNear 70.087s. Next
  nav-follow 71.262s at `3278,3192`.
- Terminal snapshot: `modals.main = -1`, inventory 27 hides + 2000
  coins, tile `3273,3169`, no Tanner NPC row. Panel read
  Trips 0 / Tanned 0. Same as headless: withdrawal real, tan never
  accepted.

Soft 274 second cycle: hitch ticks 101 during the walk in, Arrived +
Trade, five ~1.18s slots, timeout, then `3276,3193`, hitch ticks 122.
Witness tick 115 is that second indoor moment, at or after the wait
settled false. Do not read the script timeout text as "IF never opened".

## 3. Content and packet identity

274 `alkharid.npc` `[tanner]`: `op1=Talk-to`, `op3=Trade`,
`moverestrict=indoors`. 289 the same. Both `tanner.rs2` `[opnpc3,tanner]`
call `@tan_leather_choices` then `if_openmain(tanner)`. Both
`interface.pack` `679=tanner`. `8686=tanner:com_93` (soft ALL),
`8690=tanner:com_98` (hard ALL).

Host `InteractReq::Npc` uses `ActionSpec::Label("Trade")` →
`operation_of` → slot 3 → `NPC_OPS[2] = OP_NPC3`. `check_target` does
not require adjacency for NPCs; `doAction` is accepted as Sent. There
is no live Sent/Refused line for NPC (unlike `shim-count` / `shim-close`).
Soft IF 679 after Trade is enough to reject "wrong op" and "packet
never left". A refused first click is not evidenced.

`ifButton` is mapped (`InteractReq::IfButton` → `ix.press`). It is
not the failing boundary; it is never reached.

## 4. Cause

Tanner is indoor. Packed nav for `walkResilient(3277,3191, radius 3)`
finishes at outdoor `3274,3188` radius 0 (door approach). Catalog
treats that as arrived (Chebyshev 3 from the stand; walkTo also skips
at Chebyshev ≤ 4). Interact then queues OP_NPC3 from that tile and
the 5000ms modal wait starts in the same isolate turn.

Client pathing from that OPNPC does enter the building (indoor tiles
logged at timeout). Server `if_openmain` is 1–2 ticks after adjacency.
That last RTT sits on the wrong side of 5000ms. Isolate pumps see
`main_modal_id == -1` until timeoutAt, settle false, and walk back to
the bank. A later PLAYER_INFO can still carry 679 (soft witness) while
the script is no longer waiting.

Hard cells are the same order without a witness hit: either 679 never
landed while `near(TANNER_STAND, 4)` with hides, or it landed after
they left. Not a 289-only content mismatch.

This is not a reason to lengthen the foreign wait. Queue-immediate
`Npc.interact` stays. Native driver owns dispatch and must finish
OP_NPC3 → posted 679 inside that wait, which means the queued op
cannot be charged for packed-outdoor-to-indoor travel.

## 5. Minimum slice and ownership

Keep: TannerBot.ts, 5000ms, IF 679, Trade label, `main_modal_id`
posting, count-dialog mapping, core nav timeouts, packet opcode tables.

Authorized native slice, exclusive of named-bank → special → teleport
→ shop → Make-X → fire shared files (`crates/host-play/src/lib.rs`,
`crates/nav/src/traveller.rs` are hotspots — do not exclusive them):

1. **Corrective (primary):** host-owned indoor remainder for this
   WalkNear so `interact('Trade')` is queued already Chebyshev ≤ 1 of
   Tanner (or the packed dest is not the door tile). Foreign `walkTo`
   already awaits that walk. Then 5000ms covers server IF_OPEN only.
2. **Not the slice:** `Input.interactNpc` becoming an awaited
   approach; expanding 5000ms; door-open cloned from JS; ifButton /
   widget 8686 work.
3. **Optional diagnostic only, if root wants Sent vs late-open on
   headed:** one existing `BOT_DEBUG` line on `InteractReq::Npc`
   (`Sent`/`Refused`, player tile, npc tile/distance, `main_modal`
   generation) and the same fields when the cond settles false. Do not
   block the corrective on that line. Soft 679 already falsifies
   "never opened".

LIVE acceptance is still actual hide conversion, coin spend, deposit,
restock, closed-bank return, and a further tan. Startup, modal alone,
and queued if-button are not full acceptance.

No dim. No routine review hop.
