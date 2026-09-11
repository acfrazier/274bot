# Native observed-tick subscribers for Duel Arena

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 10:09 UTC. Kind: bounded read-only design for brief 104.
Not implementation, compilation, LIVE, fixtures, ledger, STATE, cache
cleanup, or a foreign-runtime/opcode copy. Root owns acceptance, any LIVE
snapshot, serialized implementation insertion, and commit of this evidence.
No routine reviewer card.

Read once: `AGENTS.md`, `docs/execution.md`, brief 104. Branch checked
first: `codex/rs2b0t-multirevision` (not `main`). Paired Duel FAIL evidence
is Root7c `7c2eef6eb8db6b01933c333aa64cf5f68169d8d2` + client gitlink
`9d090ed04957e4efc254f073cda97bc5510ca72b`. Campaign HEAD at write-up is
`9cc6adb1fac3adef01529f70778552e8e3a3c5da` (ClientAdapter inventory). Named
tick files are byte-identical 7c2eef6e → HEAD. Concurrent cake / special /
teleport / Solshop / MakeX / fire / reader101 / UI94 / paired103 workers
must not be touched. Work was read-only except this report and
`docs/compat/evidence/native-tick-subscriber-boundary/`.

## Verdict

**Duel bothrev FAIL is the missing `BotHost.addTickListener` member, not a
missing game-tick publisher.** Native already posts PLAYER_INFO-edge
snapshots then `IsolateCmd::Tick`. Map the foreign Set-of-callbacks onto
that existing JS tick entry after snapshot, before `onStart` / `loop` /
parked `settle`. Fire on normal and parked ticks. Do not fire on pause,
hold, generation skip, or stale-tick drain. Do not attach packets, invent a
tick-end opcode, poll frames, or synthesize skipped ticks.

ImportedDuel `observeFightState` is required for the ordinary combat loop
(`fightSignal.phase === 'ready'` gates Fight; pen enter/leave counts
duels). It is not an ancillary whale to stub.

| | Hypothesis | Status |
|---|---|---|
| (1) | Native has no observed-tick edge; need foreign attach / packet listener / tick-end opcode | **Falsified.** `script_observe` posts snapshot then `SlotScript::on_game_tick` on PLAYER_INFO. Isolate serializes Snapshot before Tick. |
| (2) | `BotHost.tickCount` already is the subscriber | **Falsified.** Getter reads `snap().tick`. Duel calls `addTickListener`. Proxy throws `not impl: BotHost.addTickListener`. |
| (3) | Reuse `this.on` / `inst._subs` (chat.message seam) | **Rejected.** That bus is instance IPC for snapshot `chat_text` deltas. Tick listeners are host-level PLAYER_INFO post-process. Duel calls `BotHost.addTickListener`, not `this.on('tick')`. |
| (4) | Fire only from `__rs_tick` (non-parked) | **Rejected.** FightOpponent parks on `Execution.delayUntilTicks`. Overhead 3/2/1/FIGHT! and pen leave must still be observed. |
| (5) | Fire during hold/pause to resemble foreign packet listeners | **Rejected.** Native owns tick timing. Pause skips Tick. Hold is paint-only. Do not weaken either. |
| (6) | Thin JS Set + fire from existing `__rs_tick` / `__rs2b0t_pump` | **Accepted mapping.** Callback glue in JS. Scheduling/transport stays Rust. |

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Paired Duel FAIL host | `7c2eef6eb8db6b01933c333aa64cf5f68169d8d2` |
| Campaign HEAD at write-up | `9cc6adb1fac3adef01529f70778552e8e3a3c5da` |
| Tick files 7c → HEAD | identical (`bot_host.js`, `execution.js`, `load.rs`, `slot.rs`, `shim/mod.rs`, `_kernel.js`, `script_runner.js`) |
| Client gitlink | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 104 SHA-256 | `05a52f43e31d6887c5e0bc5ee69326341fc8d67b01d1463c6d20a8a43ff5870e` |
| Kanban card | `t_a4fbd8aa` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| BotHost.ts (both catalogs; sha256 equal) | `1669d8ad8c4a92197e9c233bdc427f7185fa10ee13b521220c243bc2086d1fed` |
| DuelArena.ts (both; sha256 equal) | `5656dabb30a47aac590fa1afadba19e689dd792d70da8dc4851e18d62e52d090` |
| DuelArenaLogic.ts (both; sha256 equal) | `325ce631a7a3f246ab0bc51e9b09945aaa018d7c8971b334994384f62cd1d8f2` |
| Native `bot_host.js` | `9069b0a6a6743dc44cd72300f453e16adc21c7423c41a545fa20226b6b815336` |
| Native `execution.js` | `6395e4b43cdd8cccf079a82a3eef02b8228ee0e042f55dfef197c40b70a0ee05` |
| Native `_kernel.js` | `84ca00c764def0ac2810f1fcd723c3ad6ef411eb4543d6668fe9b0b166cd5408` |
| Native `load.rs` | `3f6b8f87c8797c8cf56411cb53ed929dc9531ab9d804319938ca0d656095fdb6` |
| Native `slot.rs` | `1593826ffe821e7a3cd0d4706b7778bbfcf15db8bc8560563d81222700d0d2c4` |
| Native `shim/mod.rs` | `72db2f3563f06d6c4621439cf929df9427c3e27482de8eaf84941f44aa8afb15` |
| r274 Duel log | `c55de8ab2f2bebe3c2209009612dfe52a3e0a7844ae8fc1a1d69ecf469a2a43f` |
| r289 Duel log | `ff9dc6941632bf64df5de5386a147d2aa9a067f188386360d9c02786168c04ec` |

Machine-readable copies: `evidence/native-tick-subscriber-boundary/{refs,hypotheses}.json`.

## 1. Actual FAIL

Root7c old-catalog paired cells, both revisions:

- 274 `liveu35qcr_0` tick 19: `not impl: BotHost.addTickListener` then
  `Duel Arena trainer starting — melee only…` then
  `FAIL: paired_catalog_duel_live: script error on liveu35qcr_0`.
- 289 `livekww5ii_1` tick 17: same throw; peer slot had not finished
  `onStart`. Same FAIL.

Mechanism, existing, not a new opcode:

1. `DuelArena.onStart` logs, then
   `this.stopFightObserver = BotHost.addTickListener(() => this.observeFightState())`.
2. Shim `proxy('BotHost', { tickCount })` throws `not impl: BotHost.addTickListener`.
3. COMPAT_RUNNER `.__rs2b0t_tick_async.catch` stores `host.lastError`.
4. `tick_loop` logs `tick {n}: {e}`.
5. `SlotScript::drain_logs` treats any line starting `tick ` as `last_error`.
6. Paired harness FAILs on `script_last_error`.

The start log is *before* the throw, so both lines appear. Catalog sources
are frozen and byte-identical across 100adc / 8e7d.

## 2. Foreign contract (do not port the runtime)

Frozen `runtime/BotHost.ts` (both catalogs):

- `tickListeners = new Set<FrameListener>()`.
- `addTickListener(cb)` adds to the Set; returns `() => tickListeners.delete(cb)`.
- `handlePacket` increments `tickCount` and `fire(tickListeners)` only on
  `ServerProt.PLAYER_INFO` after client post-process.
- `fire` try/catches per listener (`console.error('[rs2b0t] listener error', err)`),
  continues the rest.
- `addFrameListener` / `addDrawListener` / `attach` / `setPacketListener` /
  `tickMeanMs` / loc-invalidate / producers are **other** flavors.

Do not implement frame/draw listeners, attach, or packet routing.

Duel usage (`DuelArena.ts` 109–136, 358–381, 314–321, 533–576):

- Register in `onStart` after `Execution.delayUntil(() => Game.sceneReady(), 0)`.
- Immediate `observeFightState()` after subscribe (first observation does
  not wait for the next PLAYER_INFO).
- `onStop` unsubscribes. Native COMPAT_RUNNER does **not** call `onStop`
  today; isolate drop still drops the Set. Do not add `onStop` as part of
  this mapping.
- `observeFightState` tracks pen enter/leave (`duels++` on leave) and
  `observeFightSignal` over posted `reader.selfChat()` (exact 3/2/1/FIGHT!).
- `canAttemptFight` requires `fightSignal.phase === 'ready'`.
- `FightOpponent.execute` parks on `Execution.delayUntilTicks(...)`.

Other frozen callers (`RandomEventGuardian`, `WorldMapPicker`) are not
native catalog cards. Do not build them.

## 3. Actual native tick edge

Owner of timing remains Rust.

```
PLAYER_INFO observe
  script_observe (host-play): if tick_edge && Running
    post_snapshot (hold bit included)
    SlotScript::on_game_tick          // skipped if !want_run (Pause)
      LoadIsolate::on_game_tick       // skipped if isolate stopped
        IsolateCmd::Tick { tick, generation }
tick_loop (load.rs), after Snapshot materialize:
  Pause or generation mismatch → continue (no JS tick)
  host_hold → set tick, onPaint only (no __rs_tick, no pump)
  parked   → __rs2b0t_pump(n)  // settle then paint; no re-entry to loop()
  else     → __rs_tick(n)      // COMPAT_RUNNER: onStart once, chat.message,
                               // kick loop, paint, await loopP
  slow tick → drain queued Tick cmds (skip stale); do not synthesize them
```

`BotHost.tickCount` already reads `snap().tick` (`isolate_bot_host_tick_count_reads_posted_tick`).
Snapshot-before-Tick is already the PLAYER_INFO post-process equivalent.
`__rs2b0t_pump` is already the parked wait pump. No new isolate command.

Existing callback seam closest in shape: COMPAT_RUNNER `inst._subs['chat.message']`
per-cb try/catch. Wrong bus. Do not route tick listeners through it.

## 4. Minimum mapping

Callback glue only. Gameplay scheduling/transport stays Rust.

**`crates/script/src/shim/bot_host.js`** (primary):

- Keep `tickCount` getter.
- Module-local `Set` of callbacks.
- `addTickListener(cb)` → `Set.add`; return unsubscribe `Set.delete`.
  Same function added twice is once (foreign Set).
- `globalThis.__rs2b0t_fire_tick_listeners()` iterates the Set with
  per-cb try/catch. **Never** `host.lastError`. **Never** log a line
  starting `tick ` or containing `script requested stop` (those become
  `last_error` and FAIL paired cells). Optional `host.log` /
  `console.error('[rs2b0t] listener error', err)` is fine.

**`crates/script/src/shim/execution.js`** (`__rs2b0t_pump`):

- After `state.tick = n`, before `park.settle`. So overhead/pen updates
  are visible to a cond that settles on this tick.

**`crates/script/src/load.rs` COMPAT_RUNNER** (`__rs2b0t_tick_async`):

- After `host.tick = n`, **before** `onStart`. Empty Set on the start
  tick; Duel then registers and calls `observeFightState()` itself.
  Firing after `onStart` on that same tick would double-observe one
  snapshot (`observeFightSignal` treats a new overhead as an event).
- One function lookup. Empty Set is a no-op. Do not reorder chat.message,
  loop kick, paint, or `await loopP`.

Do **not** fire from hold's paint-only path, Snapshot-only posts, Pause,
generation skip, or stale-tick drain. Do **not** wrap `__rs_tick` with
`Object.defineProperty`. Do **not** change scripts that never subscribe
beyond that empty call.

`load.rs` is a known hotspot (death-recovery / periodic-bank / autocast /
COMPAT_RUNNER). Touch only the one fire site in `COMPAT_RUNNER`. Do not
edit `client_adapter.js` (reader101), registry/UI (UI94), or paired
harness (paired103).

### Ordering (must hold)

| Edge | Listeners | loop / pump | paint |
|---|---|---|---|
| Normal posted tick | fire after snapshot+tick, before onStart/loop | existing | existing |
| Parked posted tick | fire after tick, before settle | settle only; no second `loop()` | existing |
| Hold | no | frozen | yes |
| Pause / !want_run | no | no | no isolate tick |
| Stale skip / generation | no | no | no |
| Stop / join | Set dies with isolate | Stop cmd / `stopRequested` | — |
| Reload / second spawn | empty Set | fresh isolate | — |
| `reset_session_work` | Set survives (script state survives) | skipped until next Snapshot+Tick | — |

Slot isolation: one V8 isolate per slot; module `Set` is not process-global.

## 5. Unsupported flavors (honest)

Leave `not impl` (proxy): `addFrameListener`, `addDrawListener`, `attach`,
`onFrame`, `onDraw`, `tickMeanMs`, packet listener, loc snapshot
invalidation, producer pump. No `requestAnimationFrame`, no wall timer, no
synthetic ticks for skipped numbers. Native tick-shaped cards
(`NATIVE_MAIN` `__rs_tick`) do not import BotHost; do not add a fire there.

Native still does not call `onStop` / `onPause` / `onResume`. Unsubscribe
on Stop is isolate teardown, not a new runner hook. Completing those
hooks is out of scope.

## 6. Owned files and tests (implementation, not this card)

Owned:

1. `crates/script/src/shim/bot_host.js`
2. `crates/script/src/shim/execution.js` (pump fire only)
3. `crates/script/src/load.rs` (`COMPAT_RUNNER` fire only)

New focused file `crates/script/tests/tick_subscriber.rs` (do not pile
`load_isolate.rs`). Copy the local `base_snapshot` / `post_snapshot_input`
pattern from `hostile_duel.rs`. Meaningful behavior, not source snapshots:

1. `onStart` `addTickListener` does not set `last_error` / `tick N: not impl`.
2. Zero fires during the start tick; one fire per later posted tick; snapshot
   already applied (`BotHost.tickCount` / `Game.tick()` match posted tick).
3. Parked `delayUntil`: listener count increases while `__rs_loops` stays 1.
4. `pause()` then ticks: listener count frozen; `resume()` continues.
5. Posted `hold=true`: listener count frozen; paint still forwards.
6. Unsubscribe: no further fires.
7. Same function added twice fires once.
8. Throwing listener: other listeners run; `drain_logs` has no `tick ` last_error.
9. Two isolates do not share the Set.
10. `reset_session_work` skips in-flight generation; next Snapshot+Tick fires once.

Preserve (do not rewrite): `isolate_pause_ignores_ticks_and_resume_continues`,
`isolate_hold_freezes_loop_and_parked_conds`,
`isolate_execution_delay_until_parks_loop_until_cond`,
`isolate_parked_wait_still_paints_each_tick`,
`isolate_bot_host_tick_count_reads_posted_tick`,
`isolate_js_cannot_clear_host_hold`.

No compiler / LIVE on this card. Source review of a later patch is not
Duel LIVE acceptance.

## 7. Falsifiers

A later implementation is wrong if any of these hold:

- Pause or hold delivers listener callbacks.
- Parked ticks do not deliver them.
- Start tick double-fires after Duel's own `observeFightState()`.
- Listener throw becomes `last_error` (paired FAIL).
- New `IsolateCmd`, packet attach, frame timer, or tick-end opcode.
- `addFrameListener` / `addDrawListener` implemented.
- Loop/paint/chat.message order changes for scripts with an empty Set
  beyond one no-op call.
- `client_adapter.js`, registry/UI, or paired-test sources edited here.

Root inserts implementation after this design. Not LIVE acceptance.
