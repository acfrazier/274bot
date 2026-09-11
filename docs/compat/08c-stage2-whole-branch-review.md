# Stage-2 whole-branch review

Reviewer: Hermes profile `branchreviewer`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11. Kanban card `t_482a30d9`.
Kind: required independent Grok 4.6 whole-branch review of the second
incremental integration batch. Not a same-card task review, not
publication, not a release, not full catalog compatibility, not a
performance claim, and not the later-batch campaign finish line.

Read once: `AGENTS.md`, `docs/execution.md`, brief
`docs/compat/briefs/146-stage2-whole-branch-review.md`,
`08b-stage2-preflight.md`, `09-shim-ownership-audit.md`,
`09a-native-shim-sequencing.md`, `06am-brimhaven-source-dim.md`,
`06an-combat-evidence-serialization.md`. Historical next-actions were
not used as current directions. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Work was read-only except
this report and `docs/compat/evidence/stage-2-review/`. No product/test
edits, remotes, `LIVE`, compile-cache mutation, merge, or push. Working
fixture139/native144 files were not inspected as part of this review.

## Verdict

**SOURCE REVIEW COMPLETE. APPROVAL WITHHELD.** Root has not written
`docs/compat/evidence/stage-2-preflight-324/validation-ready.json`.
The operator now requires headed LIVE for remaining cells; orch
withholds that file until then.

No material host/API/script/frontend/scenario correctness, 274
preservation, identity, protocol, ownership, or accidental
foreign-JS-policy defect was found in the frozen product range. The
mandatory two-line follow-up `df2ba846a` is a justified scenario-send
correction, not a host workaround and not a concealed npcBox. This is
**not** publication and **not** complete catalog support.

## Inspected refs

| Role | Exact value |
|---|---|
| Branch | `codex/rs2b0t-multirevision` (not `main`) |
| Host base | `76b2016b7dafe9aae7b0c33dd591d8a9907380cb` |
| Code-equivalent tested pin | `324d7b5946534db28305942d9bfba75fdd2e5dc8` |
| Frozen docs candidate (brief) | `9b3dc71c0486490bd6429faab39ee5dbf8c138ad` |
| Current product candidate | `df2ba846a51e5010fe8dc8aa104222955b3b0cba` |
| Client gitlink | `aef3952d1cd7bb3b93d39c497f0f476b68021c59` |
| Ordinary-history commits base..df2 | 265 |
| Reviewer model/provider | `grok-4.6` / `xai-oauth` |
| validation-ready | **absent** at source-review write-up |

Confirmed: `git branch --show-current` = `codex/rs2b0t-multirevision`.
`git ls-tree df2ba846a vendor/fr-client-rust` = client `aef3952d`.
`324d7b594..9b3dc71c0` is documentation/evidence only (crates/vendor
empty). `9b3dc71c0..df2ba846a` is exactly two `StepKind::Repeat` →
`StepKind::Perform` replacements in `crates/scenario/src/lib.rs`.
Vendor client is unchanged. `Cargo.lock` adds `serde_json` on `api`.

`git diff 9b3dc71c0 df2ba846a` (complete):

- `combat_core_scenario` wear step (Dragonfire shield 1540, also used
  by `green_dragon`)
- `auto_fighter_mage_scenario` staff step (Staff of fire 1387)

Runner semantics: `Repeat` re-sends every tick until `wait.arm`;
`Perform` sends once. `Interactions::wear` refuses `StaleTarget` once
the item has left the pack. Exact 324 mage logs:
`r274-auto-fighter-mage-100adccc-stage2324` and the 289 twin fail at
step 9 `driver rejected the send` with `has_item_id(1387)<=0`,
`core={"error":"no Start baseline"}`, pack showing Trout/runes and no
staff. That is repeated wear after a successful first wield, same class
as Green. Original 324 failures are retained. Fresh df2 mage/gear cells
are required in validation-ready; this review does not infer them.

## Scope

In: native capability/transport ownership; generated data provenance
and revision binding; stale/full/delta/lifecycle; banking, loadout,
recovery, combat, UI, query; preservation of native navigation, last-FBO
freeze, and no per-read world copy; native123 after audit09; source-
specific Brimhaven dim.

Out: fixture139 Gnome/Herblore/Ardy/Wildy/Rock/Green/Coal preparation;
native144 `reader.npcBox`; teleport/shop/Make-X/fire/trade/paired Mule;
corrected145 extra combat/utility scenarios; complete support
advertising; Alpha 2; performance.

## Source findings

Native123 / audit09. Pin `d142c0ee0` plus observation projection
`0e7ba119b` restore Rust `begin`/`next`/`done` for
`Bank.openBooth` / named `openNearest` / `openNearestWorld`,
`stealCakes`, and `Autocast.arm`. JS dispatches only posted
`walk-near` / `walk-nearest-bank` / `open-booth` / `walk-to` / `loc` /
`side-tab` / `if-button` and parks with `Execution.delayUntil(..., 0)`
(`timeoutAt` null; Pause/hold skip the pump). Rust bounds:
`WALK_BOUND_MS = 60_000`, steal resolve 2400 ms, autocast 2 s tab /
3 s phase. Introduced 120 s waits are gone from those three APIs.
Cake restock/target predicates live in `api::cake_stall`; JS
`needsCakeRestock` is a rustyscript marshal. `classifySteal` and
`AcquireTask` stay `not impl`. PeriodicBank / DeathRecovery keep the
valid Rust-next / callback split with native 60 s walks.

Generated data. `SelectedGameData::decode` refuses schema ≠ 3 and
revision mismatch. `for_revision` is process-lifetime `OnceLock` per
274/289 blob. `for_profile` refuses cache-id mismatch;
`for_optional_profile` returns `None` rather than mixing facts.
Manifest SHA-256 of `274.json` /
`6ca4c04acd7f12f635b99c044009d24d0af09d1d23afb709e8f0e6ca5a634232` and
`289.json` /
`f7c78bf2ac32b29c618cc5690c555495abd29f101d1e1eba7e382dc4b8f0422b`
match the committed blobs. Engine/content commits and packed input
hashes are recorded. Cache ids match the live 324 mage identity lines
(`4aac9b63…` / `c4d8ab36…`).

Snapshot / lifecycle. `publish_snapshot` still resets on `!ingame` and
on `session_changed` using `session_start_gens`. Families rebuild only
when their gate moves. Bank loaded requires transmitting +
`full_generation != 0` + `full_observation` after the last main-modal
close; session generation advances on modal packet delta or withdraw-
component identity change. Frontends call
`Host::publish_frontend_snapshot` (panel `session.rs`, TUI `bin.rs`)
through the same `Pump`/`publish_snapshot` seam. Panel bind-once
refusal is unchanged (`server profile is already bound; restart to
change revision`).

Query / reachability. `pack_reach_query(None)` posts unavailable /
empty dims. `bebd2fe8b` posts native flood ranks; JS
`Reachability` is a bit/rank projection and returns false when the
view is unavailable. No JS flood.

Banking / loadout / recovery / combat / UI. Named booth facts resolve
from packed booth tiles + walkable stands (`NavWorld::named_bank_facts`)
without copying the packed stand table into published facts. Loadout
start now threads `game_data` and `named_banks` into isolate load
(`memory.rs` fixture path included). DeathRecovery `needs` still throws
AcquireTask unsupported. Special bar/cost identities are generated;
`Special.arm` remains a one-shot JS `ifButton` plus posted
`arm_confirm_ticks` wait with a reset token — not a foreign planner,
not in native123 scope, not treated as a thin mapping. Paint buttons,
tick subscribers, and TUI/panel loadout/params are host-owned surfaces.

Navigation / render / memory. Nav pack decode is one read plus
`from_bytes`; BadVersion still refuses a stale 274V pack. Streamed
1 MiB hashing does not emit early completion. `SharedClientTemplate`
still shares `Arc<NavWorld>` (`world.clone()` is Arc). Client
`gpu.rs` `freeze_last_scene` remains last-FBO freeze while
`scene_state == 1`. Candidate crates have no `npcBox` (native144 is
later). No fake null box.

Brimhaven dim. `catalog_defect_reason` requires register name
`BrimhavenAgility`, card SHA-256
`771daff07bd4b3d6f2826ab1300d4fd66bcbae0f9d7a76e4a2ad07a4d050e859`,
and sibling `BrimhavenAgilityLogic.ts`
`fedf5f8e05fd43642efb0370352e71e5feebf258a784bc503a2e145c33b46d4d`.
`CATALOG_DIM` is the unrelated name set (AIOQuester, ClueSolver,
gatherers, quest-def, MarketMaker) and does not include Brimhaven.
Hash-gate evidence: both frozen catalogs dim the exact pair (44
enabled / 10 dim); changed-card and changed-helper controls are
enabled (45 / 9). This does not conceal a host route defect and does
not inherit across version changes.

## Evidence independently checked (not live acceptance)

Workspace pin `324d7b594` / client `aef3952d` in
`evidence/stage-2-preflight-324/checks.json`: fmt exit 0; all-target
Clippy `-D warnings` exit 0; `cargo test -p script` 517 passed / 0
failed / 3 ignored; `catalog_boundary_live` 49 passed / 1 LIVE ignored.
`08b` b18 full 3066 PASS plus three retained failures and targeted
fixes is not a single full-green-run claim. Combatcff 15 PASS / 11 FAIL
plus held newer-catalog cells, serializer panic repair, missing npcBox,
and Green Repeat-wear remain recorded in `06an`. N1/N32 Thiever Auto
is historical qualified behavior, not this milestone's performance
comparison.

Headless root cells were read from logs, not inferred from process
exit. They are **not** validation-ready and were not used to approve.

324 binary `fe7a3223…` / host `324d7b594`: both 100adccc mage cells
FAIL at step 9 `driver rejected the send` / `no Start baseline` after
Repeat wear (staff 1387 already gone). Four Flax reach cells PASS with
chat `You pick some flax.` and `has_item_id(1779)>=1`
(274/289 × 100adccc/8e7d965b).

df2 binary `76cc4040…` / host `df2ba846a`: four mage cells PASS with
Start, `autocast armed: Fire Strike`, Guard attack, and
`stat_xp_gain(6)>=1` (magic). Two 100adccc Green Dragon cells FAIL
after Start on `stat_xp_gain(2)>=1` within 150 ticks at Lumbridge
tiles, not wear refusal. That remaining Green miss stays fixture139;
df2 only removed the Repeat-wear refusal. Harvest ledger 457 is
orch-owned. Future cells are headed native UI.

## Residuals (non-blocking for source; not silent exclusions)

- `Bank.openNearestAccess` still JS-composes a 60 s adjacent wait then
  `openBooth`. Inherited on published main `76b2016b7`, not the audit
  F1 120 s leak, not in native123 scope.
- `Special.arm` JS one-shot click + tick wait (intentional later split).
- `Banking.bankNearest` still open + optional deposit composition.
- FireGiant `reader.npcBox` absent in this candidate.
- 274/289 Chaos selected-loot and 274 guard-food failures retained.
- 289 unpack `models.bin` still missing in this environment
  (`snapshot load skipped`); not a GPU/performance claim.
- Catalog completeness, teleport/shop/Make-X/fire/trade/mule, and
  fixture139 preparation remain later work.

## Gate for approval

Read `evidence/stage-2-preflight-324/validation-ready.json` only after
root writes it following headed LIVE. Require: exact df2 (or a later
code-identical pin) mapped to mage/gear cells that show Start and
script/host-caused progress, not process exit; Flax reach cells mapped
with original 324 mage failures retained; `ready_for_final_review`
true; `approved_for_publication` false. Native regression remains a
blocker. Incomplete later cards, including Green Strength-XP miss
after Start, are not automatic pass or reject.
