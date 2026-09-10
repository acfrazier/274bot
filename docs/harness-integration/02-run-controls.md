# Integrated scenario and fleet run controls

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-10. Kind: bounded pre-implementation design of brief 46
section B against committed source and a bounded historical harness
snapshot. Not runtime evidence, not LIVE, not source acceptance, not
release, not a second scenario engine, not a distributed campaign
framework.

Read once: `AGENTS.md`, `docs/execution.md`, `docs/harness.md`,
`docs/compat/STATE.md`, `docs/compat/05a-catalog-live-harness.md`,
`docs/compat/01-session-profile.md`, plan step 8 in
`docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md`,
and the named brief. Branch checked first: `codex/rs2b0t-multirevision`
(not `main`). Work was read-only except this report. No product edits,
LIVE, fixtures, STATE, matrix, subagents, stash, reset, restore,
checkout, merge, remotes, or other workers' files.

Current shipped code is authoritative. `codex/memory-diagnostics` was
inspected with `git show` / `git ls-tree` only. Old reports are
evidence of earlier capabilities, not requirements to restart that
campaign. The catalog cell inspected at campaign HEAD `c9d01a29` was
then committed as `6c6bb2d5` (full bank-fletcher loop + milestone
shots). This report uses that committed witness, not a dirty tree.

## Verdict

**Proceed to a shared Rust run controller in `host-play`, reused by
panel, TUI, and the headless catalog cell**, under the constraints
below. Design approval is not source acceptance and not live
acceptance.

Do not add a second scenario engine. Do not fold catalog proof into
`memory::Run`. Do not build a general matrix scheduler, remote fleet,
cohort stimulus, or managed-process campaign. One process still owns
one selected run (or one N=2 isolation pair, or one memory-fleet
workload). Sequential selected runs are multiple processes, not one
in-process Cartesian product.

The useful gap is not missing Pause/Stop/login-reconnect APIs. Those
already exist on `Play` and both frontends. The gap is **split proof
and split prepare**: headed `--live script_*` treats
`ScenarioRunner::Passed` as process success, while the independent
catalog cell requires `CoreWitness::qualify()` after exactly one Start
and a post-seed baseline. Readiness (`ingame && scene_state == 2`) and
a normal exit are not success.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `bba5179cc55722ec4a28b3f1658fbd442113e074` |
| Catalog-cell product | `6c6bb2d5c25d51f5b339d6bb5f73d68b3b7296cb` |
| Client submodule HEAD | `56d80272bcbda3eb1e22db096c1c5e21d3497de4` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief SHA-256 | `4a5a7df764e4c25fc6f1eeafeaa1ce316df2422a5f69397a9714ab60ce7192a9` |
| `catalog_boundary_live.rs` | `935e49d824e5d852db7f5b0cf281ec91f83c71a0` |
| `scenario/src/runner.rs` | `b67c18812937c99c898a3c2f52a997dd1910954d` |
| `host-play/src/memory.rs` | `f31954e8297b9da399aa2114b5c248163c227e51` |
| `script/src/isolated_env.rs` | `4801a11c5b63ccb9ac01b4739a11c07d5f8411d6` |
| Historical harness extraction | `codex/memory-diagnostics` `f9975102` (`docs/memory/selective-harness-extraction-report.md`) |
| Kanban card | `t_82a411f0` |

`6c6bb2d5` strengthens BankFletcher `CoreWitness` to a fresh bank
cycle plus further crafting (first product / empty-stock stop cannot
qualify). That is the same `CatalogCore` split this report names; it
does not add a second engine. Capture milestone shots are section A.

## Capability inventory

### Current built-in (keep)

| Capability | Where it lives now | Notes |
|---|---|---|
| Shared seed/step/proof machine | `scenario::ScenarioRunner` | One instance per run. Slot thread ticks; UI reads `status`/`evidence`. No sleeps inside `tick`. |
| Host-driven scenario success | `RunnerStatus::Passed` | Correct for `nav_full` / door / Guardian / e2e. Wrong as the *only* catalog-card success. |
| Headed live prepare | `panel::Session::live_prepare_script`, TUI twin | Minted names, ephemeral vault, optional MultiBox for 2+ seed profiles, `script_settings_inject`, Start stashed until `StartScript`. |
| Headed/TUI live pump | `live_script_tick` / `TuiSession::live_status` | Exit 0 on `Passed`, exit 1 on `Failed`. Terminal-shot drain and `BUDGET_S` soak are headed-only. |
| Headless catalog cell | `host-play` test `catalog_boundary_live` | One ignored LIVE cell. Identity ledger, `merge_bag`, independent `CoreWitness`, `stop_slot`, fail-closed once `LIVE=1`. |
| Script settings merge | `script::settings_store::merge_bag` | Schema defaults, then overrides, then inject. Catalog cell uses schema + scenario inject + `CATALOG_SETTINGS_JSON`; nothing persisted. |
| Interactive settings | `ScriptSettingsStore` on both frontends | Operator `~/.274bot/script-settings.json`. Must not be the proof path. |
| Pause / Resume / Stop | `Play::{script_pause,script_resume,script_stop}` | Panel and TUI buttons already call these. Stopping ≠ slot teardown. |
| Slot teardown | `Play::stop_slot` | Catalog cell uses this. Live `--live` exits the process instead. |
| Login reconnect | `SlotArm.reconnect`, opcode 18 | `seed_on_first_world` skips mainland seed after reconnect. Keep that. |
| Ephemeral accounts | `host_play::mint_live_names` | 12-char `live…_i`. Engine player saves still accumulate under `player/`. |
| Thread-local store pin | `script::IsolatedEnv` | Tests and `panel/examples/catalog_watch.rs`. Production `panel-play`/`tui-play` do **not** enter it. |
| Fleet memory harness | `host_play::memory::{Config,Run,Sample}` | `BOT_MEMORY_N` ∈ {1,16,32,128}. Workloads idle / seeded-idle / active / lifecycle. Qualification JSONL at observe-start/end. `Ok(true)` is teardown finished, not “qualified”. |
| N=50 RAM watch | `live_prepare_stress50` | Not catalog proof. Debug builds warn; not an isolation test. |
| Shared world | `ScenarioRunner::with_world(Play::world())` | Seeds must not decode a second pack. |

`docs/harness.md` already states: a process that exits normally is not
by itself evidence that every bot did useful work.

### Current external (operator automation — do not clone into the app)

| Capability | Where | Keep outside / absorb |
|---|---|---|
| Headed cell wrapper | `.superpowers/catalog-headed/run_cell.py` | Frozen binary identity, timeline, runtime-error scan, receipt JSON. Absorb **proof** into Rust; keep Python as a receipt/launcher until a CLI exists. Hardcoded engine paths stay operator-local. |
| Headless cell wrapper | `.superpowers/catalog-headed/run_headless_cell.py` | Sets `CATALOG_*` and execs the ignored test. Same split. |
| Platform SSH / Windows watch | `.superpowers/platform-preparation/*.py,*.ps1` | Section C. Stay external. |
| Historical sequential matrix | `codex/memory-diagnostics:docs/memory/run_matrix.sh` | Skip-if-already-PASS, TSV summary, N=1 then 32 then 128. Useful *pattern* (one cell, preserve prior receipts). Do not reintroduce as a campaign DAG. |
| Historical qualify CLI | `docs/memory/qualify_control.py` on that ref | Exit 0 only when `qualified`. Lesson for catalog: print `PASS` only after `qualify()`. Do not make the Python file an application dependency. |

### Missing (useful, in-scope)

1. **One proof kernel used by headed, TUI, and headless catalog.** Today
   only the ignored test requires `CoreWitness` after Start. Headed
   BoneBurier historically exited 0 on first burial (`05a`).
2. **First-class isolated stores on production binaries.**
   `catalog_watch` already does `IsolatedEnv::enter`. `panel-play --live`
   can write operator JS/settings/loadout paths.
3. **Per-run qualification record** for catalog/single-actor cells
   (identity, account, settings, baseline, witness, per-slot script
   state, outcome). Memory fleet already writes
   `samples.qualification.jsonl`; catalog does not share that shape.
4. **Controller-level cancel.** Ctrl-C / window close / UI Stop do not
   share one outcome (`Cancelled` vs `Passed`). Catalog cell stops the
   slot after the loop; headed live just exits.
5. **Repeatable selected-run launcher in Rust/CLI** that runs one
   named cell per process, refuses to overwrite a prior receipt, and
   does not treat skip as PASS.
6. **N=2 same-revision isolation proof** (plan step 8). `BOT_MEMORY_N`
   cannot be 2. Stress helpers at N=2/3 are unit fixtures, not that
   proof. Two minted live names in a fleet scenario are visibility, not
   store isolation.
7. **Partial-failure policy for N>1.** Memory `Run::poll` fails the
   whole run on the first seed/script error. Keep that. Do not average.

### Obsolete / do not reintroduce

- Cohort stimulus / managed-process controllers
  (`cohort-stimulus-controller-integration.md` on the historical ref).
- Direct-owner census, scheduling/latency journals, campaign replay,
  failure-capture controller, borrowed fingerprints, tiled navigation.
- In-process 20-cell catalog matrix, remote N=32 catalog, or merging
  `memory::Config` with catalog identity gates.
- `qualify_control.py` as a required runtime of 274bot.
- Debug-only mandatory build tools; per-tick world copies; a second
  parallel scenario engine.
- Treating `LIVE` unset + ignored test `Ok(())` as a catalog pass if
  someone later wraps it without `--ignored` discipline. Controller
  must refuse unless `LIVE=1` (and local target) for live proof modes.

## Proof semantics (one source)

Three *kinds*, one controller enum. Do not invent a fourth runtime.

| Kind | Success predicate | Failure | Not success |
|---|---|---|---|
| `Scenario` | `ScenarioRunner::Passed` plus no `script_last_error` | Runner `Failed`, timeout, script error | Scene 2, soak elapsed, process exit |
| `CatalogCore` | Exactly one catalog Start **and** Start baseline (ingame scene 2, minted player, case prep) **and** `CoreWitness::qualify()` **and** runner not `Failed` | Any of those missing; settings that contradict a core criterion | Runner `Passed` alone; first burial; seed XP; headed PNG; Python `process_exit_code==0` |
| `MemoryFleet` | Existing `Run` rules: all slots ready; seeded-idle/active extra seed/proof/script gates; qualification files written; no script error | First slot failure fails the run | `poll` returning `Ok(true)` (teardown window elapsed) |
| `IsolationPair` | Two slots, distinct minted accounts, distinct isolated stores, both `ingame && scene_state==2`, Stop on slot A does not stop B, no cross-store settings leak | Shared HOME/JS/settings, one account, one isolate dying both | MultiBox rail visible |

Catalog option branches (Alcher custom/ordered/large-batch) stay
**named witnesses on `CatalogCore`**, not a new kind. Headless tests
already reject seeded-only and unordered large-stack. Do not weaken
core criteria with `CATALOG_SETTINGS_JSON`.

Server-admin preparation (givebank, engine `player/` wipe, speed cheat)
stays explicit and local-only. Catalog cell already forces
`engine_speed_ms = None`. Normal user sessions must not inherit those
cheats.

## Ownership and seams

### New type (implementation later)

`crates/host-play/src/run_control.rs` (name may vary). Not a new crate.
Not in `client`. Not in `panel`/`tui` except thin pumps.

```text
RunSpec { profile, isolation, accounts, scenario, catalog, settings, proof, deadline }
RunController { prepare, bind_play, frame, poll, cancel, pause, resume, stop_script, progress }
RunOutcome { Passed { qualify }, Failed { reason, qualify }, Cancelled { reason } }
```

`frame` is the existing per-slot observe hook. `poll` is what
`live_script_tick`, `live_status`, and the catalog test loop all call.
Frontends do not decide PASS.

### Who may do what

| Crate | Owns | Must not |
|---|---|---|
| `scenario` | Seed/step/arm/proof machine, `script_settings_inject`, `StartScript` | Catalog identity ledger, CoreWitness, process exit |
| `host-play` | `RunController`, Play lifecycle, mint, catalog identity+witness extract, memory `Run` as a *mode* | UI, GPU shots (section A), SSH |
| `script` | `IsolatedEnv`, `merge_bag`, `RunState` pause/stop | Proof predicates |
| `panel` / `tui` | Pumps, Pause/Resume/Stop buttons, progress lines, `--live` / `--isolate-stores` flags | A second prepare/start/exit policy |
| `host-play` ignored test / example | Thin LIVE wrapper around `RunController` | Duplicated witness once extracted |
| Python helpers | Frozen-binary receipts, platform SSH, prior-log preservation | Proof, PASS/FAIL |

Extract from committed `catalog_boundary_live.rs` (`6c6bb2d5`) into a
library module the test keeps calling: `CoreCase`, ledger identity,
`Observation`/`CoreWitness` (including the bank-fletcher cycle),
`prepare_catalog_card`, settings merge order, Start-baseline gates.
The test file becomes env parsing + `RunController` + assert. Further
root witness edits land in the same module.

### Application surface

Keep `--live script_<name>` on `panel-play` and `tui-play`. Add:

- `--isolate-stores` (default **on** for `--live` proof and for
  `catalog_watch`; **off** for interactive operator sessions).
- `--settings-json <object>` (same fail-closed merge as
  `CATALOG_SETTINGS_JSON`; unknown keys fail).
- Existing `--profile` / `--revision` remain the immutable bind.

Promote `panel/examples/catalog_watch.rs` to that default live-proof
path, or keep the example as a one-line `IsolatedEnv` wrapper around
`run_panel`. Do not maintain two headed catalog entries.

Interactive Pause/Resume/Stop stay focused-slot buttons calling
`Play`. Live-proof cancel is `RunController::cancel`: `script_stop`
every actor, then `stop_slot`, then IsolatedEnv drop. Outcome is
`Cancelled` (exit 1), never `Passed`.

Progress: keep the existing step lines (`live {name}: running step
i/n`). Add one compact status the rail/TUI can show: phase, actor
count, last qualify error. Do not add a campaign dashboard.

### Shipped CLI vs operator automation

| Belongs in the application | Belongs in a shipped CLI | Stays operator automation |
|---|---|---|
| Proof kernel, isolation, pause/stop, qualify JSON, ephemeral vault/accounts | `--live`, `--isolate-stores`, `--settings-json`, headless example/test, sequential one-cell launcher | SSH, Hyper-V/Windows watch, frozen-binary SHA receipts, engine `player/` wipe, givebank, hardcoded fixture paths |
| N=2 isolation pair | `BOT_MEMORY_N=32` unchanged for preservation | Historical `run_matrix.sh`, `qualify_control.py` |

Sequential selected runs: a small launcher (host-play example or
`tools/` later) that execs **one process per cell**, refuses if the
receipt path exists, and exits 1 on the first failure. No in-process
fan-out. No skip-as-PASS unless the receipt already contains a real
`qualified: true` for that exact identity.

### Memory fleet vs catalog

`memory::{Config,Run,Sample}` stays the N=1/16/32/128 preservation
harness. Do not add `2` to `parse_n` just to steal that path for
isolation. Do not start catalog scripts from `BOT_MEMORY_WORKLOAD`.
The controller may *call* `Run` as `ProofKind::MemoryFleet` later;
first increment does not merge the types.

N=32 qualified Thiever (plan step 8: every slot banks, returns, and
makes progress) is a **later live** on the existing `Run` + sustained
Thiever fixture, after catalog/frontend proofs. It is not a catalog
matrix.

## Staged implementation cards

Suggested later cards for section D. This report does not create them.

1. **Extract catalog proof library** from committed
   `catalog_boundary_live.rs` into `host-play` (identity, settings
   merge, CoreWitness, Start baseline). Thin the ignored test. Move
   existing negative unit tests with the extract. No LIVE.
2. **`RunController` + headed/TUI catalog path.**
   `live_script_tick` / `live_status` call `poll`. `CatalogCore` cells
   cannot exit 0 on runner `Passed` alone. Host-driven `Scenario` cells
   unchanged. No LIVE required to land; LIVE is root's later cell.
3. **`--isolate-stores` on production panel/TUI live-proof.** Default
   on for `--live`; interactive stays on operator HOME. Unit-test that
   JS/settings/loadout paths are under the pin and restored on drop.
4. **Qualification JSON** for single-cell catalog/scenario (identity +
   per-actor state + witness + outcome). Exit 0 only after
   `Passed { qualify }`. Process completion without that file is fail.
5. **Sequential selected-run CLI** (one process per named cell,
   preserve prior receipts, fail-closed). Replaces local use of
   `run_headless_cell.py` / `run_cell.py` for proof; those scripts may
   remain as frozen-binary launchers.
6. **N=2 isolation pair** (plan step 8): two minted slots, two store
   pins or one IsolatedEnv with two accounts and two script slots,
   Stop-A ↛ Stop-B, no settings leak. After cards 1–3.
7. **Cancel/Stop through the controller** (Ctrl-C, window close, UI
   Stop in `--live`). Outcome `Cancelled`. Do not add a new
   `RunState`.
8. **N=32 preservation** remains `memory::Run` + sustained Thiever.
   Not this controller's first increment.

**Suggested stopping point:** cards 1–5. That is the useful
compatibility-release wiring: one proof, isolated stores, honest
qualify, repeatable selected runs. Cards 6–8 belong with frontend
preservation, not with extracting the catalog witness.

Quick wiring (card 2–3) vs substantial shared-controller (card 1 + 4):
do the extract first so headed cannot keep a weaker predicate.

## Risks

- **Headed/headless drift if extract is skipped.** The BoneBurier
  first-burial PASS is the existence proof of this bug.
- **IsolatedEnv is thread-local.** It does not rewrite process `HOME`.
  Slot threads must see the same pin (`ensure_thread` / enter on the
  UI thread before spawn). Catalog_watch already relies on this.
- **In-process multi-cell** would share one `ServerProfile` / Play /
  JS cache. Forbidden for catalog identity cells. One process, one
  selected run.
- **Witness still lives only in the ignored test.** Extract from
  `6c6bb2d5` before headed `--live` can keep a weaker predicate.
- **Memory `parse_n` vs N=2.** Extending allowed N silently turns
  isolation into a benchmark. Keep IsolationPair separate.
- **Cancel vs Passed race.** A terminal shot drain (section A) must
  not convert a cancel into PASS. Hold shots only for already-decided
  Passed/Failed.
- **Operator settings leak.** Proof bags must use empty overrides +
  inject, never `ScriptSettingsStore::save`.
- **Partial fleet.** First actor failure fails the run. No “17/32
  ready” success.
- **Reconnect during seed.** Keep `seed_on_first_world`; a controller
  must not re-tele on opcode-18 reconnect.

## Focused validation (no LIVE in this architecture task)

Offline, proportional:

- Existing CoreWitness negatives move with the extract (first-burial,
  depleted-only, stale-bank, missing transfer, seeded-only Alcher).
- `merge_bag` order tests already exist; add one catalog-cell test
  that explicit keys unknown to schema fail and that a core-contradicting
  explicit setting fails.
- IsolatedEnv tests already prove HOME is not process-mutated; add a
  prepare-path test that live-proof `js`/`script-settings` resolve under
  the pin.
- Controller unit: `poll` on `Scenario::Passed` without witness is
  `Failed` for `CatalogCore` and `Passed` for `Scenario`.
- Controller unit: `cancel` yields `Cancelled`, never `Passed`.
- `parse_n("2")` remains `Err` (N=2 is IsolationPair, not memory N).

Later LIVE (root, not this card): one frozen headed cell and its
headless twin produce the same `qualify` keys; process exit 0 iff
`qualified`. That is functional proof after ingame/scene 2, not
readiness alone.

## What this report does not decide

- Capture/F12/timeline (section A).
- Three-OS packaging, SSH, support bundles (section C).
- Combined sequencing (section D).
- Generated game-data (section E).
- Whether `catalog_watch` stays an example or becomes the binary
  default — implementer may pick either as long as production
  `--live` catalog proof is isolated.
- Exact qualify JSON schema field names — match catalog `identity` /
  `baseline-after-preparation` / `start` / `pass` records already
  printed by the committed cell, plus per-slot script state.

## Stopping point (human)

Integrate the independent catalog witness and isolated stores into the
application so headed, TUI, and headless cannot disagree on success.
Stop before a campaign scheduler, remote fleets, or merging memory
benchmarks with catalog identity. Source approval of this note does
not imply live acceptance.
