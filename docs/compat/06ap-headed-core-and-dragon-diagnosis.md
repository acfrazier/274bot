# Headed core-proof reuse and GreenDragon stall (df2)

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`,
`reasoning_effort: high`. Date: 2026-09-11 17:32 UTC. Kind: bounded
read-only source/evidence diagnosis for brief 151. Not implementation,
LIVE, product edits, builds, parent gates, dim, clocks, or STATE.
Root owns acceptance. Own only this report and
`docs/compat/evidence/headed-core-and-dragon-diagnosis/`. Root already
queued 152 `t_620a63ab` for the awaited-validation correction; this
audit does not wait on it and does not implement it.

Read once: `AGENTS.md`, `docs/execution.md`, brief 151. Branch checked
first: `codex/rs2b0t-multirevision` (not `main`). Campaign HEAD at
write-up is `a1434ae2015a0af0b24bf4819ec8a919e5de28dc`. LIVE cells were
recorded on frozen host `df2ba846a51e5010fe8dc8aa104222955b3b0cba` /
client `aef3952d1cd7bb3b93d39c497f0f476b68021c59`. Configured profile
defaults verified: `model.default=grok-4.6`, `model.provider=xai-oauth`,
`agent.reasoning_effort=high`. This run used those defaults. Fixture
139 (scenario WIP) and native 144 (projection / `load.rs`) are excluded
from the audited candidate. No LIVE process was started.

## Verdict

**1. Headed panel PASS is not core PASS.** `catalog_watch` is the
production panel. `live_script_tick` only mirrors `ScenarioRunner`.
Full catalog core (combat defeat/loot/further work, production/bank
cycles) lives only in `catalog_boundary_live.rs` `CoreWitness`.
Smallest sound reuse is to extract that existing witness onto a shared
host-play module and attach it to the headed slot's compact observation
stream. Do not duplicate cycle observers in panel. Do not pretty-print
`GameSnapshot` on the slot/observe path.

**2. Both df2 GreenDragon old-catalog cells die after Start with no
Strength XP because host TaskBot does not await `validate()`.** Frozen
`Bot.ts` does `if (await task.validate())`. Host prelude
`crates/script/src/shim/mod.rs:101` does `if (task.validate())`.
GreenDragon wraps every task in `Traced` whose `validate` is `async`
and therefore returns a Promise. A Promise is always truthy, so the
first task (`ContinueDialog`) is selected every loop even when
`ChatDialog.canContinue()` is false. That matches the Normal-level
silence after the start summary: no `equipped`, no `died! recovering`,
no fight/loot. Not a foreign catalog defect. Do not dim. Do not weaken
the 150-tick Strength watch or `CombatCoreCycle` predicates.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `a1434ae2015a0af0b24bf4819ec8a919e5de28dc` |
| LIVE host (df2) | `df2ba846a51e5010fe8dc8aa104222955b3b0cba` |
| LIVE client (receipts) | `aef3952d1cd7bb3b93d39c497f0f476b68021c59` |
| LIVE binary sha256 | `76cc40403cfd16bbbea9126fda15f4c2dcbee5714e6f08ddea9e06c37307564e` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 151 SHA-256 | `0f98ffdfc05ee0f84ca86695fc26a536d3e0e83d611e9d6c40071f93f79ec310` |
| Kanban card | `t_9f72114a` |
| Queued correction | 152 `t_620a63ab` (awaited validate + isolate regression; preserve 144 `load.rs`) |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| 274 engine (receipt) | `4c95f87efe00b068cadbd229d94736626907bd1a` |
| 289 engine (receipt) | `cc359656b4acd216ca452495874b6beba9a0ac75` |
| GreenDragon.ts (both catalogs) | `46b667c46e82ca8f4d4c7461b93f6be8af05b021bd18126e82784d4264bf1fe9` |
| Bot.ts (both catalogs) | `8b95b5f1c72a88cfe317fb9ba183f20594504b54b48a042cdbf6150369c705b8` |
| Host TaskBot prelude | `crates/script/src/shim/mod.rs:91-107` |
| Host Bot.js re-export | `crates/script/src/shim/bot.js` |
| r274 log SHA-256 | `5aca7297f98befd34c2db3c3f8fcabfee4e7c2f74be823bde49816a3b5a56dae` |
| r289 log SHA-256 | `e601dde6590c1d113955c0c672b23f1156cf747149c6dda2e3e1dcf810f842a3` |

Machine copies:
`evidence/headed-core-and-dragon-diagnosis/{refs,cells,headed-reuse,dragon-cause,hashes}.json`.
Log sha256 values match the per-cell `*.json` receipts. Catalogs `cmp`
identical on GreenDragon.ts and Bot.ts.

## 1. Headed core-proof reuse

### Boundaries

| Surface | What it actually proves |
|---|---|
| `crates/panel/examples/catalog_watch.rs` | Production panel + isolated stores. `--live script_<name>` only. |
| `crates/panel/src/app.rs` `LiveScript` / `live_script_tick` | `ScenarioRunner` status/evidence. PASS latches runner Passed. |
| `crates/scenario` `green_dragon` | Preparation, Start, then `stat_xp_gain(2)>=1` within 150 ticks. Weaker than core. 139 owns this file; excluded. |
| `crates/host-play/tests/catalog_boundary_live.rs` `CoreWitness` | Start baseline + post-Start `observe`. Combat requires engagements≥2, defeat≥1, further_work, selected-style XP, loot/shield extras. |

`Observation` (`catalog_boundary_live.rs:666`) and `CoreWitness` are
test-local. Panel `SlotStatus` is not that type. `live_script_tick`
never calls `validate_case_baseline` or `witness.evaluate()`.

Do not assume a headed scenario PASS equals `CombatCoreCycle.qualified`.
The df2 GreenDragon cells show the split: preparation/Start pass the
runner far enough to watch Strength XP; `core=green_dragon core
post-Start delta incomplete` is the real qualifier.

### Ownership serialization (do not land extraction now)

Until their reviews complete: 139 owns `catalog_boundary_live.rs`; 144
owns `host-play/src/lib.rs` and `panel/src/session.rs`. Shared-core
extraction follows both. Gate native 37 (`Game.teleport`) on that
extraction so it does not overlap `host-play` lib. Mule 127 only
touches `paired_catalog_live` / support and can proceed. Option 147
must wait for the extraction so it does not edit `catalog_boundary_live`
mid-move.

### Smallest sound implementation (after 139 and 144 reviews; not done here)

1. Move `Observation`, `CoreWitness`, and the existing cycle types out of
   the test file into a dedicated `host-play` module (not `scenario`, not
   `script` `load.rs`). Keep `qualified()` predicates unchanged. `lib.rs`
   only re-exports after 144 releases it.
2. `catalog_boundary_live` becomes a consumer of that module (after 139
   releases the file).
3. Headed `catalog_watch` / `LiveScript`: for `CoreCase` names only,
   capture the same Start baseline and `observe` each already-published
   slot snapshot via `Observation::from_snapshot`. One publisher. No
   second world walk. Panel session wiring waits on 144.
4. `live_script_tick`: runner Failed still fails. Runner Passed on a
   core case is not terminal until `witness.evaluate()` is Ok. Non-core
   `script_*` names stay runner-only.
5. Shot sink today (`app.rs:4277-4288`) runs on the slot-threaded
   runner and `serde_json::to_string_pretty` of a full `GameSnapshot`
   before enqueue. Both native 274/289 mage timelines show 72–75 ms
   `observe_us` immediately after `[panel] scenario capture requested`
   on first XP (`evidence/first-fire-strike-hitch/timing.json`). Keep
   capture off the gameplay/observe thread: enqueue a label plus a
   handle, serialize/readback elsewhere. Correlation only; no
   model-loading cause. Do not serialize snapshots inside
   `CoreWitness::observe`.

Later fixture authors reuse the shared path: add a cycle + `CoreCase`
arm on the extracted witness (same `observe` / `qualified` /
`evaluate`). Headed `catalog_watch` then inherits that case through
`LiveScript` without a panel-side observer. Do not copy cycle structs
into panel or into option-fixture tests.

Paired full witness eventually needs the same headed route. Seam: each
paired slot can feed the shared `Observation::from_snapshot` +
`CoreWitness`; a pair-level qualifier stays in the paired harness.
Remaining limitation: `catalog_watch` `LiveScript` is one scenario / one
`ScenarioRunner` today. This audit does not add a paired headed
harness.

## 2. df2 GreenDragon cells

Shared Start baseline (after preparation, before script work): field
`[3096,3814,0]`, scene 2, Attack/Strength/Hitpoints 40, Defence 1, worn
`1540` only, Rune scimitar `1333` in pack, 12 lobster, Strength XP
37224, `local_in_combat=true`, `local_animation=422`, dragon targeting
local, `logDetail=Normal`. Start loaded. Shield wear is the df2
once-before-teleport fixture, not GearEquip.

| Cell | Exit | Wall s | Runner | Tile at FAIL |
|---|---|---|---|---|
| r274 old `100adccc` | 1 | 103.607 | FAIL 180 ticks / 100488 ms step 17 `stat_xp_gain(2)>=1` | `[3222,3217,0]` Lumbridge |
| r289 old `100adccc` | 1 | 45.183 | FAIL 183 ticks / 42090 ms step 17 `stat_xp_gain(2)>=1` | `[3219,3219,0]` Lumbridge |

Both chats include `Oh dear you are dead!` plus shield-absorb lines.
Both FAIL inv: scimitar + 2 lobster (wilderness keep-3 shape). 274
baseline HP 37; 289 baseline HP 28. Brief said 289 dies; 274 also died
on this receipt. Core witness incomplete on both.

Normal-level script log after Start: only the GreenDragon summary line.
Absent Normal-level actions that would have logged if they ran:
`equipped …`, `escaping …`, `died! recovering`, `green dragon down`,
`looted …`. Eat has no Normal log (`setStatus` only) so eat cannot be
proved or disproved from silence. Verbose task-switch (`-> Name`) is
also gated; its absence is not evidence.

## 3. Cause: host TaskBot vs frozen await

Frozen `Task.validate(): boolean | Promise<boolean>`. Frozen
`TaskBot.loop` (`Bot.ts:143-147`, both catalogs):

```
for (const task of this.tasks) {
    if (await task.validate()) {
        await task.execute();
        return;
    }
}
```

Host catalog scripts do not run that class. `shim/bot.js` re-exports
prelude `globalThis.TaskBot`. Prelude (`mod.rs:99-106`):

```
async loop() {
    for (const task of this._tasks) {
        if (task.validate()) {
            await task.execute();
            return;
        }
    }
}
```

GreenDragon (byte-identical both catalogs) wraps every added task:

```
async validate(): Promise<boolean> {
    const ok = await this.inner.validate();
    if (ok) this.bot.noteTask(this.name);
    return ok;
}
```

`async validate` always returns a Promise. `if (task.validate())` is
true for any Promise, including `Promise.resolve(false)`. First added
task is `ContinueDialog`. Inner `validate()` is sync
`ChatDialog.canContinue()` (`snap().chat_continue === true`). Baseline
`main_modal=-1`. Host still enters `ContinueDialog.execute`, which
no-ops when `canContinue` is false, then returns from `loop`. Next
`loopDelay` 600 ms, same first task. GearEquip / Eat / DeathRecovery /
Fight never win.

Existing isolate coverage (`isolate_task_bot_loop_runs_first_passing_validate`,
`isolate_task_bot_loop_awaits_execute`) only uses sync `validate`. No
test that `async validate() => false` must not be selected. That is the
152 regression.

This is a host prelude ABI miss against the frozen TaskBot contract, not
a GreenDragon.ts defect and not 139 tele-into-combat. Native unarmed
422 plus auto-retaliate is what the client does while the script is
stuck; it is not proof that Fight ran. DeathRecovery's Normal
`died! recovering` never appears because that task is never selected.

Related host gap, not this stall: prelude TaskBot also omits frozen
`Game.sceneReady()` before the task scan. Start baseline was already
scene 2.

## 4. Classification

| Question | Applies? |
|---|---|
| Pass preparation + Start, no Strength XP | **Yes.** Both cells. |
| 289 dies | **Yes.** 274 also dies on this receipt. |
| Missing Normal logs = missing verbose | **No** for `equipped` / `died! recovering` / fight. **Yes** for `-> Task` and eat. |
| ContinueDialog inner due | **No** at baseline. Promise truthy still selects it. |
| GearEquip / Eat / DeathRecovery / Fight ran | **Not selected.** Scheduler never leaves ContinueDialog. |
| API transport / Equipment.equip | **Not reached.** |
| Foreign catalog regression | **No.** Both catalogs identical; frozen Bot.ts awaits. |
| Dim GreenDragon | **No.** |
| Weaken 150 / CombatCoreCycle | **No.** |
| Edit 139 scenario or 144 `load.rs` | **No.** 152 owns prelude `TaskBot.loop` + isolate test. |
| Panel PASS = core PASS | **No.** |
| First Fire Strike hitch = model load | **Not claimed.** 72–75 ms observe after capture request; shot sink pretty-prints `GameSnapshot` on the runner thread. |

## 5. Bounded next steps

1. **152 already queued:** `if (await task.validate())` in prelude
   TaskBot, matching `Bot.ts:144`. Isolate test: first task `async
   validate() => false`, second sync true, second executes. Do not
   touch 144 `load.rs`. Do not edit frozen catalogs.
2. **Headed core reuse (after 139 and 144 reviews, not 152):** extract
   `CoreWitness` as above; gate native 37 on it; 147 waits; 127 may
   proceed. Headed PASS for core cases requires `evaluate()`. Keep shot
   serialization off the slot/observe thread. Root may later authorize
   a prepared actor; do not change 139 tele/Start order here.
3. Verbose `logDetail` is no longer the primary diagnostic for this
   stall; the Promise/`ContinueDialog` mechanism is source-proven. A
   headed rerun after 152 is root-owned.

Configured and actual model: `grok-4.6` / `xai-oauth`.
