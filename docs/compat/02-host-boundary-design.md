# Step 4 host action and snapshot boundary design

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-10. Kind: bounded pre-implementation design/audit of
`docs/compat/briefs/11-host-boundary-design.md` against current source.
Not runtime evidence, not live acceptance, not step-5 world/guardian
qualification, not campaign release.

Read once: `AGENTS.md`, `docs/execution.md`, plan architecture and step 4
in `docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md`,
and the named brief. Branch checked first: `codex/rs2b0t-multirevision`
(not `main`). Work was read-only except this report. No product edits,
subagents, commit, merge, remotes, live sessions, or suite reruns.

## Verdict

Orchestrator correction after source inspection: the blanket statements below
that old numeric ids must be absent from the R289 legal table are incorrect.
R289 OPPLAYER2 legitimately has id 51 and EVENT_MOUSE_CLICK has id 224.
Validation must follow named packet identity and its revision-selected length,
not a numeric blacklist. Current ClientRevision has only R274/R289; exhaustive
matching is appropriate without introducing a hypothetical variant. These
corrections are part of implementation briefs 12/13. Actual reviewer execution
was verified as session `20260910_132555_633342`, `grok-4.6` / `xai-oauth`,
completed on card `t_d6761a15`, run 1096.

**Proceed to step-4 implementation** under the required constraints
below. Design approval is not source acceptance and not live acceptance.

The proposed ownership shape is right and implementable on the named
pins:

- Protocol names and revision mapping stay in the client.
- Host `api::Driver` must expose the bound session revision (missing
  today). Typed host sends, legal-send rows, cardinal door steps and
  local cheats must select those named client rows.
- `press` / `walk` / `op_loc` / widget `doAction` already take the
  mapped client path. Direct `Send`, `WalkStep` and `cheat` do not.
  Opcode mapping alone does not prove payloads or host return
  semantics.
- Snapshot publication already borrows the live `Client` by generation
  and must stay that way. Logout/reconnect reset and leftover queued
  work need explicit host tests; do not invent a tick-end opcode.
- Keep the production 289 bot-operation gate. The smallest honest
  local proof is `SharedClientTemplate::prepare_client` (already
  documented for protocol qualification) plus the real `Driver` /
  `GameSnapshot` path, on isolated local engines only.

Do not lift `try_spawn_slot`, scripts, catalog, `queue_wire`, Play
cheat, panel/TUI start, or 289 nav/guardian. Do not send step-4 proofs
at public `w1.rs2b2t.com:443`.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Named host product base (source inspected) | `db9b741a8cf2dfc80d667f9c0f66d1dd49223d20` |
| Campaign HEAD at write-up | `db9b741a` (same; untracked briefs 10/11 only) |
| Accepted client pin | `2be1697060e4d2b8b709ad4d5e54d12513b38333` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief SHA-256 | `5d780bca12df415b864b790bbacdbe2a529fbdc7e2553005da25a72983fef264` |
| Concurrent card | `t_96dc78da` (public profile help/tests/docs only; not action/snapshot) |

`docs/compat/STATE.md` still names older host product `41d896b3` and
campaign base `b2bd5023`. This review used the brief's `db9b741a` /
`2be16970` pair.

## What the proposal gets right

- Client owns framing, ISAAC, named `ClientProt` / `ClientProt289`,
  `map_client_prot`, `Client::client_opcode`, world state and render.
  Host owns interactions, legal sends, navigation policy, guardians and
  compatibility. No second host opcode table. No bot action API in the
  client. No foreign JS runtime. No 377 import or `!is_274()` policy.
- Public 274 ids collide with 289. Silent 274 bytes on a 289 socket are
  not a near-miss:

  | Host writer | 274 id | 289 mapped id | 289 collision if 274 id is sent |
  |---|---|---|---|
  | `Send::if_button` | 9 | 86 | 274 `MOVE_MINIMAPCLICK` is also 86 |
  | `Send::close_modal` | 51 | 93 | 289 `OPPLAYER2` is 51 |
  | `Send::count_dialog` | 102 | 180 | — |
  | `cheat` `CLIENT_CHEAT` | 224 | 34 | 289 `EVENT_MOUSE_CLICK` is 224 |
  | `WalkStep` `MOVE_GAMECLICK` | 207 | 234 | — |

- Kernel `WireCommand::Close` already goes through
  `MiniMenuAction::CLOSE_BUTTON` → `Client::close_modal` →
  `client_opcode`. `WireCommand::Count` and `DoorStep` do not.
  `nav::traveller` walks through `Interactions::walk` (`tryMove`);
  door crossings use `pending_door_step` → `WalkStep`.
- `LEGAL_SEND` is a 274-only coverage table, not a runtime allowlist.
  Its current test hardcodes `IF_BUTTON` id 9. Revision-selected rows
  must not keep that 274 id on R289, and must not treat mapping output
  as the only oracle.
- Snapshot families are generation-gated copies of views, not a world
  clone (`walk_in_place_does_not_clone_for_previous_readers`). Host
  `after_drain` rebuilds dirty families; `should_emit_tick` is the
  `PLAYER_INFO` gen bump. Script FlatBuffers post that same view.
- 289 inventory decode is already revision-correct in the client:
  full count is `g2`, partial slot is `gsmart`, item id/count is
  `g2` + `g1`/`g4`. Host snapshot publishes decoded `link_obj_*`.
- `Client::logout` takes the stream, sets `ingame = false`, resets
  map/modals/ground, and `bump_all_gens()`. Scene rebuild then copies
  `ingame` / `attached`. Actor/inv leftover arrays are not obviously
  cleared; that is a step-4 observation test, not a new opcode.
- Step 3 already left
  `SharedClientTemplate::prepare_client` callable without starting a
  host loop or bot policy. `run_with_template` with an empty profile
  list also skips `require_bot_operation`. That is the proof seam.
- Local engines: 274 `43594/80`, 289 `44594/1080` in
  `docs/compat/fixture-inputs.md`. 289 nav pack is not qualified.
  Public 289 is operator-confirmed at `w1.rs2b2t.com:443`; it is not a
  step-4 proof target.

## Required constraints

1. **`Driver::revision()` is the host bind.** Default `R274` for
   legacy recorders/stubs so existing tests keep 274 bytes. `impl
   Driver for Client` returns `Client::revision()`. Match `274|289`
   only; refuse anything else, including 377, before menu / ISAAC /
   route / `out.pos` mutation. Never `if !is_274()`.

2. **Map every remaining raw host writer through the client.**
   `Send::write`, `WalkStep::write` and `cheat` currently
   `p1_enc(ClientProt::*.id)`. They must
   `p1_enc(map_client_prot(driver.revision(), named).id)` and keep the
   existing payload builders (IF_BUTTON `p2`, CLOSE empty, COUNT `p4`,
   cheat `p1(len+1)+pjstr`, WalkStep `p1(5)+p1(0)+p2 abs x+z`). Do not
   re-encode `doAction` / `tryMove` in the host. Do not add a host
   opcode table.

3. **Refuse, do not panic, on the host side.** `map_client_prot` panics
   on unmapped R289 constants. Host writers must refuse before
   `p1_enc` so `out.pos`, ISAAC, menu slot and route stay unchanged.
   `do_action` staying `true` even when `doAction` writes nothing is
   existing 274 return semantics; preserve it. Tests must assert bytes
   / `out.pos`, not only the bool.

4. **Legal rows are revision-selected named client rows.** Derive
   `LEGAL_SEND` for a revision by mapping each named 274 constant
   through `map_client_prot` and taking that row's id+length. R289
   must not contain 274 ids 9/51/102/207/224. Valid 289 additions that
   already have a named map (`REPORT_ABUSE` → `SEND_SNAPSHOT` 94/10)
   appear as that mapped row. Unmapped names fail closed. Coverage
   tests may not hardcode 274 ids as universal.

5. **Independent byte oracles, not the mapper under test.** Expected
   289 bytes come from pinned primary write sequences:
   `vendor/fr-client-rust/docs/revision-289/source-contract.md`,
   `protocol-289.json`, and client
   `tests/fixtures/revision_289/outbound_lengths.rs` /
   `revision_289_{outbound,stage3}.rs`. Host tests drive the actual
   `Driver`/`Send`/`WalkStep`/`cheat` path and compare. A test that
   only asserts `map_client_prot(R289, IF_BUTTON).id == 86` is not
   payload proof. R274 walk/close/count/cheat/button bytes stay
   identical to today's fixtures.

6. **Snapshot stays a generation-gated view over the live client.**
   No deep world copy. No invented tick-end packet. Tick publication
   remains `should_emit_tick(player_info_this_drain)`. Feed 289
   inventory frames with `g2` counts and `gsmart` slots into the
   production decoder, then assert snapshot views; do not decode 274
   widths on R289. Bank/iface/actor/scene rebuilds follow the existing
   dirty-family path.

7. **Logout / reconnect / queued work are host observation
   contracts.** After logout, snapshot `ingame` and `attached` must be
   false. Live NPC/player/inv/bank views used by scripts and
   Interactions must not keep the previous session as current. If the
   client leaves leftover arrays, host rebuild when `!client.ingame`
   publishes empty live views rather than those leftovers. Drop or
   refuse queued `WireCmd` / Play cheats / script interacts on logout
   or `!ingame`. Preserve 274 incomplete stubs that are not this
   reset (trade hardcoded ifaces, body-plane-vs-`minusedlevel`,
   timeouts, routing safeguards). Do not finish step-5/7
   capabilities.

8. **Narrow proof seam only.** Keep
   `HOST_BOUNDARY_NOT_QUALIFIED` on `try_spawn_slot`, script
   start/load, Play cheat, `queue_wire`, non-empty
   `run_with_template` / `run_with_profile`, TUI `run`, panel
   `boot_execute` / `start_vault`. Do not disable guardian policy.
   Do not bind or fall back to 274 nav for 289. The proof constructs
   via `host::prepare_client_with_profile` /
   `SharedClientTemplate::prepare_client`, drives `api::Driver` on
   that `Client`, and rebuilds `GameSnapshot`. Isolated local engines
   only. A queued send is not action proof.

## Base-to-current risk inventory

| Risk | Files / call sites | If unfixed |
|---|---|---|
| Direct 274 `Send` on 289 | `crates/api/src/prot.rs` `Send::write`; `interact.rs` `close_modal` / `answer_count`; kernel `WireCommand::Count` | Count dialog writes 274 id 102; free `close_modal()` writes 51 = 289 `OPPLAYER2` |
| Direct 274 door walk | `prot.rs` `WalkStep`; `interact.rs` `WireCommand::DoorStep`; `nav/src/traveller.rs` `pending_door_step` | Cardinal door step emits 274 `MOVE_GAMECLICK` 207 |
| Direct 274 cheat | `interact.rs` `cheat` / `mainland_hop` / `seed_at`; `host-play/src/lib.rs` slot cheat drain ~907 | Local `::` command emits 224 = 289 `EVENT_MOUSE_CLICK` |
| `LEGAL_SEND` 274-only | `prot.rs` table; `api/tests/interact.rs` `legal_send_covers_every_client_prot` | 274 id 9 treated as legal on 289; 289 `SEND_SNAPSHOT` invisible |
| `Driver` has no revision | `interact.rs` trait; stubs in api/nav/host/host-play/script tests | Writers cannot select named rows without ambient `bot_target()` |
| `do_action` always true | `interact.rs` `impl Driver for Client` | `Sent` with zero `out.pos` when NPC missing or IF_BUTTON vetoed; preserve bool, test bytes |
| Mapped path is not the whole host | `press`/`walk`/`op_loc`/`set_run`/`logout`/`CLOSE_BUTTON` use `client_opcode`; Count/DoorStep/cheat do not | Assuming `doAction` mapping finishes step 4 leaves colliding writers |
| 289 inv widths | `client.rs` `apply_update_inv_full` / `_partial`; `snapshot.rs` `rebuild_inv` / `rebuild_inventory` / bank | 274 `g1` count/slot on 289 corrupts containers |
| Logout leftover views | `Client::logout` + `bump_all_gens`; `snapshot.rs` `rebuild_scene` vs `rebuild_npcs` / inv; host `after_drain` | `ingame=false` with previous NPCs/items still queryable |
| Queued work after logout | `host-play` `wires`, cheat queue ~902, `dispatch_wires`, script interact under hold | Cheat loop writes even when not ingame |
| Content mistaken for protocol | `RUN_ORB_IFACE = 153`; bank widget names; stun 245 | 289 run/bank/stun IDs are step 5; guessing them here is a false pass |
| Slot/guardian used as 289 proof | `Play::try_spawn_slot`, `Host::run_client` guardian tick, nav pack | Exposes ordinary 289 slots or requires unqualified 289 nav |
| Public 289 as proof | `w1.rs2b2t.com:443` | Violates isolated-engine rule; cheats are local-only |

`doAction` / `tryMove` payload shapes for NPC (`p2` index), loc
(`interact_with_loc`), IF_BUTTON (`p2` child), MOVE (`p1` size + run
+ abs x/z + deltas) are already revision-selected inside the client
and covered by client `revision_289_outbound.rs`. Host named-op tests
must go through that driver, then check bytes against the pinned
oracles.

## Bounded implementation tasks

Group into three coherent cards. Do not split helpers. Do not start
step 5/7.

1. **Host outbound boundary.** Add `Driver::revision()`. Route
   `Send` / `WalkStep` / `cheat` through `map_client_prot`. Make
   legal-send revision-selected. Refuse unmapped revisions before
   mutation. Keep R274 bytes identical. Independent byte fixtures for
   button, close, count, cheat, door step, plus named
   NPC/loc/ground/item/player/widget ops through the real `Client`
   driver. Files: `crates/api/src/{prot,interact}.rs`, Driver stubs,
   `crates/api/tests/interact.rs`. Client seams only if a named map
   row is wrong; do not add a bot action API.

2. **Snapshot publication and reset.** Tests (and host-only empty-view
   gate if leftover client arrays remain) for 289 `g2`/`gsmart`
   inventory, bank/iface updates, actor identities, scene/region
   generations, `PLAYER_INFO` tick, reconnect/logout reset, and
   rejection of old queued wires/cheats/script interacts. Preserve
   generation ownership and 274 incomplete stubs. Files:
   `crates/api/src/snapshot.rs`, `crates/host/src/{lib,slot}.rs`,
   host-play wire/cheat queues, existing snapshot tests.

3. **Narrow local proof harness.** No production-gate lift. Construct
   with `prepare_client_with_profile`. Drive login → `ingame &&
   scene_state==2` → walk → one snapshot-visible NPC and loc → logout
   on isolated local 274 and 289 engines. Root owns engine
   preparation. `LIVE=1` ignored tests; FAIL + exit 1. 274 may keep
   using existing slot live tests as extra evidence; 289 must not
   spawn Play slots or load nav.

## Meaningful tests

- R274: current close/count/cheat/WalkStep/IF_BUTTON bytes, ISAAC
  `p1_enc`, door-step abs x/z, refused send leaves `out.pos`/menu/route
  unchanged.
- R289: same builders, mapped ids 86/93/180/34/234, lengths from
  pinned outbound fixtures, not from the mapper. 274 ids 9/51/102/207/224
  absent from the 289 legal table.
- Named ops through `Interactions` + real `Client` at R289: opcode and
  ordered payload match `revision_289_outbound.rs` oracles. Return
  bool is not sufficient.
- Inventory: production 289 full/partial frames → snapshot slot/id/count;
  a 274-width frame must not be treated as 289 success.
- `PLAYER_INFO` gen bump increments snapshot tick once; extra 20 ms
  frames without player gen do not.
- Logout: `ingame`/`attached` false; live actor/inv/bank views empty;
  queued cheat/wire not flushed onto the socket.
- R274 snapshot no-clone / generation tests remain green.

Do not add tests that only stringify `LEGAL_SEND` or `map_client_prot`.

## Local proof plan

Design approval does not run this. Implementation task 3 does, after
root prepares engines from `docs/compat/fixture-inputs.md`.

1. Offline writer/snapshot tests (task 1–2) on `db9b741a` + later
   step-4 commits. No network.
2. Isolated local 274 (`43594`) and 289 (`44594`) engines. Recheck
   fixture identity at the live run. Do not use public 289.
3. Bound profile → `SharedClientTemplate::load` →
   `prepare_client`. Same constructor as production slots.
4. Login, wait `ingame && scene_state == 2`.
5. Walk via `Driver::try_move` / `Interactions::walk` to a tile
   observed in the snapshot (not a 274 nav-pack destination on 289).
   Proof is displacement in a later snapshot, not `SendResult::Sent`.
6. NPC and loc: pick identities from the live snapshot after scene 2.
   Do not hardcode 274 content ids. Proof is a following world/inv/chat
   change, not a queued opcode.
7. Logout via the existing CC_LOGOUT `doAction` path. Observe reset
   in (5).
8. Production 289 slot/script/cheat/wire/frontend paths still return
   `host-boundary-not-qualified`. Guardian policy unchanged.

Missing, explicit, not in this step: 289 nav pack, 289 `givebank`,
289 run-orb/bank widget content ids, guardian/nav solvers, catalog
cards, mixed fleets, Linux/Windows, public live smoke.

## Distinctions

| Claim | This document |
|---|---|
| Ownership and constraints are implementable | Yes — design approval |
| Direct writers and snapshot reset are already correct | No |
| Source review of the future patch | Later same-card `reviewer` on the implementation cards |
| Offline byte/state tests | Implementation gate for tasks 1–2 |
| Local login → scene2 → walk → NPC/loc → logout | Implementation gate for task 3; live acceptance |
| 289 bot slots/scripts/nav/guardians | Still refused; step 5+ |
| Campaign / 0.1.7 | Not accepted |
