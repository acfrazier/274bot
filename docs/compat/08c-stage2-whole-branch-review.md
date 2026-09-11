# Stage-2 whole-branch review

Reviewer: Hermes profile `branchreviewer`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11. Kanban card `t_482a30d9` (second run after validation-ready).
Kind: required independent Grok 4.6 whole-branch review of the second
incremental integration batch. Not a same-card task review, not
publication, not a release, not full catalog compatibility, not a
performance claim, and not the later-batch campaign finish line.

Read once: `AGENTS.md`, `docs/execution.md`, brief
`docs/compat/briefs/146-stage2-whole-branch-review.md`,
`08b-stage2-preflight.md`, `09-shim-ownership-audit.md`,
`09a-native-shim-sequencing.md`, `06am-brimhaven-source-dim.md`,
`06an-combat-evidence-serialization.md`,
`06ap-headed-core-and-dragon-diagnosis.md`. Historical next-actions were
not used as current directions. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Work was read-only except
this report and `docs/compat/evidence/stage-2-review/`. No product/test
edits, remotes, `LIVE`, compile-cache mutation, merge, or push. Working
fixture139/native144/152/153 files and campaign HEAD after `df2` were
not inspected as part of the product review.

## Verdict

**APPROVE bounded incremental integration of frozen host `df2ba846a` /
client `aef3952d` versus published main `76b2016b7`.** Independently
verified native mage/Flax cores, headed mage watches, retained failures,
and the unchanged-on-main TaskBot disposition in
`evidence/stage-2-preflight-324/validation-ready.json`
(`ready_for_final_review` true).

This is **not** GreenDragon core acceptance, **not** whole-campaign
acceptance, **not** a release or tag, **not** approval of campaign HEAD
(`bbe1742cd` and 144/139/152/153), and **not** a performance claim.

No material host/API/script/frontend/scenario correctness, 274
preservation, identity, protocol, ownership, or accidental
foreign-JS-policy defect was found in the frozen product range. The
mandatory two-line follow-up `df2ba846a` is a justified scenario-send
correction, not a host workaround and not a concealed npcBox.

## Inspected refs

| Role | Exact value |
|---|---|
| Branch | `codex/rs2b0t-multirevision` (not `main`) |
| Host base | `76b2016b7dafe9aae7b0c33dd591d8a9907380cb` |
| Code-equivalent tested pin | `324d7b5946534db28305942d9bfba75fdd2e5dc8` |
| Frozen docs candidate (brief) | `9b3dc71c0486490bd6429faab39ee5dbf8c138ad` |
| Product candidate | `df2ba846a51e5010fe8dc8aa104222955b3b0cba` |
| Client gitlink | `aef3952d1cd7bb3b93d39c497f0f476b68021c59` |
| Ordinary-history commits base..df2 | 265 |
| Campaign HEAD at this write-up (excluded) | `bbe1742cd8c31e33edb4b95d6dd3babf9610d705` |
| Reviewer model/provider | `grok-4.6` / `xai-oauth` |
| validation-ready | present; `ready_for_final_review` true; `whole_campaign_acceptance` false; `green_dragon_core_acceptance` false; `release_or_tag_authorized` false |

Confirmed: `git branch --show-current` = `codex/rs2b0t-multirevision`.
`git ls-tree df2ba846a vendor/fr-client-rust` = client `aef3952d`.
Same gitlink at base `76b2016b7`. `324d7b594..9b3dc71c0` is
documentation/evidence only (crates/vendor empty).
`9b3dc71c0..df2ba846a` is exactly two `StepKind::Repeat` →
`StepKind::Perform` replacements in `crates/scenario/src/lib.rs`.
Vendor client is unchanged. `Cargo.lock` adds `serde_json` on `api`.

`git diff 9b3dc71c0 df2ba846a` (complete):

- `combat_core_scenario` wear step (Dragonfire shield 1540, also used
  by `green_dragon`)
- `auto_fighter_mage_scenario` staff step (Staff of fire 1387)

Runner semantics: `Repeat` re-sends every tick until `wait.arm`;
`Perform` sends once. `Interactions::wear` refuses `StaleTarget` once
the item has left the pack.

Product after `df2` (`a1434ae20` npcBox, `e05ecafb8` isolate gate,
`bbe1742cd` resource starts, plus uncommitted 152 shim) is **out of
scope**. Integrate/publish only the frozen pair.

## Scope

In: native capability/transport ownership; generated data provenance
and revision binding; stale/full/delta/lifecycle; banking, loadout,
recovery, combat, UI, query; preservation of native navigation, last-FBO
freeze, and no per-read world copy; native123 after audit09; source-
specific Brimhaven dim; bounded live mapping of mage/Flax/headed/Green
on the frozen pair.

Out: fixture139 Gnome/Herblore/Ardy/Wildy/Rock/Green/Coal preparation;
native144 `reader.npcBox`; teleport/shop/Make-X/fire/trade/paired Mule;
corrected145 extra combat/utility scenarios; TaskBot await-validate 152;
headed CoreWitness 153; complete support advertising; Alpha 2;
performance.

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

## Independent live mapping (not process-exit)

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

df2 export `source-geardf2.json`: 5111 files; `crates/scenario/src/lib.rs`
SHA-256 `0040a6ad1e3856be3b8425d11581e813a73eef9797fc2af253f09a4b628bc276`
matches `git cat-file` of `df2ba846a`. 324 export remains 5030 files
with scenario SHA `f515da8a…`. Extra df2-export files are evidence, not
a second product delta.

Headless logs (exact `catalog_boundary_live`, not inferred from exit):

324 binary `fe7a3223…` / host `324d7b594`: both 100adccc mage cells
FAIL at step 9 `driver rejected the send` / `has_item_id(1387)<=0`
after Repeat wear (staff already gone). Four Flax reach cells PASS with
chat `You pick some flax.` and `has_item_id(1779)>=1`
(274/289 × 100adccc/8e7d965b).

df2 binary `76cc4040…` / host `df2ba846a` / client `aef3952d`: four mage
cells PASS with `autocast armed: Fire Strike` and runner
`stat_xp_gain(6)>=1` at `[2661,3306,0]`. Log SHA of r274 100adccc mage
`53edc1bf…` matches the receipt. Two 100adccc Green Dragon cells FAIL
after Start on `stat_xp_gain(2)>=1` within 150 ticks at Lumbridge
`[3222,3217,0]` / `[3219,3219,0]`, not wear refusal. Log SHA of r274
Green `5aca7297…` matches the receipt and 06ap. Newer-catalog Green
cells were held after the older-catalog FAIL; not promoted to PASS.
Original 324 mage failures are retained.

Headed panel watches (scenario-only; **not** extra CoreWitness passes):
both native 274/289 `catalog_watch` processes exit 0 on host `df2ba846a`
/ client `aef3952d` / catalog `100adccc`. Independent screenshot read:

- 274: title `274bot`, `alpha 1 - df2ba84`, `state ingame scene 2`,
  tile `2661 3306`, weapon `Staff of fire`, `Attack with Fire Strike`,
  AutoFighter attacking Guard, log `script: autocast armed: Fire Strike`.
- 289: title `289bot`, same host rail, `Staff of fire`, Fire Strike,
  AutoFighter attacking Guard, `ingame scene 2`.

72–75 ms observe hitch after capture request remains diagnostic only.

## Unchanged-base TaskBot disposition

Prelude `TaskBot.loop` lines 91–107 at `76b2016b7` and `df2ba846a` are
byte-identical. SHA-256 of that block:
`df612f35c83636078313e4f49fd814f8b93c7e687c91aa22623c43c58d78ebb4`
(matches validation-ready). The rest of `shim/mod.rs` differs for
unrelated stage-2 content (`tileFromPosted`, Traversal module order,
generated `content_json`) — not this stall.

Mechanism (06ap, independently re-read): frozen `Bot.ts` awaits
`task.validate()`; host prelude does `if (task.validate())`. GreenDragon
`Traced.validate` is async, so a Promise is always truthy and
`ContinueDialog` wins every loop. That is a host ABI miss **already on
published main**, not a df2 regression, not a catalog defect, and not
in this candidate's 152 correction. Bounded milestone may integrate
without claiming Green support and without folding 152 into the pin.

## Residuals (non-blocking for this milestone; not silent exclusions)

- Host TaskBot does not await `validate()`; 152 owns the fix; excluded.
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
- BankSorter remains `UNAVAILABLE_BY_OPERATOR_DECISION` pending a
  separate native sorter; four rows, no value/ID substitute, not a
  foreign-defect classification. Documentation-only; no df2 runtime
  change.

## Gate result

`validation-ready.json` maps exact df2 mage/gear cells with Start and
script/host-caused magic XP; Flax 324 cores with original 324 mage
Repeat-wear failures retained; `ready_for_final_review` true;
publication/release flags false. Native regression versus `76b2016b7`
was not found. Green Strength-XP miss after Start is an honest
pre-existing host bug, not an automatic reject of the verified native
batch.
