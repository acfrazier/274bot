# Native nearest-bank access ownership and composition

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 07:20 UTC. Kind: bounded read-only architecture/fidelity
audit for brief 79. Not implementation, LIVE, fixtures, ledger, or an
enabled-status change. Root owns acceptance and any follow-up repair.

Read once: `AGENTS.md`, `docs/execution.md`, fail-closed-dispatch, current
STATE imported-script / ancillary-stub rules, brief 79, `05l` and its raw
evidence. Branch checked first: `codex/rs2b0t-multirevision` (not `main`).
Work was read-only except this report and
`docs/compat/evidence/native-bank-access-boundary/`. Concurrent working-tree
edits on this checkout are not the tested binary. Shared request/dispatch
source is owned by `t_c35281b3`, then `t_af045121`; it was not edited.

## Verdict

**East is not an imported VialFiller defect. It is an incomplete
`Bank.openNearest` mapping: the bridge chose low-level geometric
`OpenBooth` instead of the existing packed-nav approach plus named booth
open.** Incomplete host mapping remains **BLOCKED**, not a foreign dim.
Do not dim `vial_filler_east` or default `vial_filler` from this audit.
Do not change `open_booth_at` / `open_named_booth_at`.

Classification against brief 79:

| | Choice | Applies? |
|---|---|---|
| (a) | Caller bug | No as ownership. Radius-3 arrival at `(3010,3352)` is real geometry and is retained from `05l`, but frozen `Bank.openNearest` is specified to recover after that arrival. |
| (b) | Intentional low-level host contract | Yes, and it must be preserved. `open_named_booth_at` refuses Use-quickly until Chebyshev ≤ 1, then geometric `Walk` toward the signum-adjacent tile. That is the loc-op contract (`open_nearest_booth_walks_when_not_adjacent`). |
| (c) | Missing high-level capability | Yes as a **shim composition of existing host verbs**, not a new `Interactions` method and not a cloned JS router. |
| (d) | Bridge chose the wrong existing host operation | **Primary.** `Bank.openNearest` is `openBooth(undefined, name, op)` → named `open-booth` only. Approach already exists as `WalkNear` radius 1 (`ScriptWalkArm`) and is already used by `openBooth(stand)`, `openNearestAccess`, and PeriodicBank Access. |

West PASS is a plaza-geometry control: outdoor Chebyshev-2 is already
adjacent enough for (b) to fire Use-quickly. It does not prove East
responsibilities belong to the foreign card.

Correction to `05l`: retain the mechanical trace and cell table as
historical evidence. Withdraw the imported-script / dim-East verdict.
The `05l` root qualification already records that withdrawal; this
report is the ownership replacement.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `f79a4aa2cdd83977ae9b826d965a82d3d3f6424c` |
| Frozen B1 host (East cells) | `b1cff8a7fdf6d18a5f7a3c7c23c6fa8a020e173b` |
| Frozen 2f host (West control) | `2f1bd9cf75bfcff0900eb9e029deb98433f5a6d3` |
| Client gitlink | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 79 SHA-256 | `7b2acfb08531d5dc8f285d831ea899a2d8414e700bedda3d35e999d50aab1224` |
| 05l SHA-256 | `47b9da00468158b39d7ff4803b425fb68f000ca704057afca1cae9e6d9aad787` |
| Kanban card | `t_b3223dcc` |
| Prior East audit | `t_715971a2` / brief 76 |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| Bank.ts (both catalogs) | sha256 `3031b0a602209bdf81c33acdffe47aed8666fc4324d9af3c5796a7ce9928f747` (`cmp` exit 0) |
| VialFiller.ts (both catalogs) | sha256 `58f95dca5a65bfc6048c1da6a3eda8a8153b90979c0a30c237e76ed437c8bd4c` |
| `crates/script/src/shim/bank.js` | sha256 `31d0380098d9c9d1b759447ce832461c3dd9b52069ceb9b6bd96d5181cfb1aeb` (unchanged vs B1) |
| `crates/api/src/interact.rs` | sha256 `7e2e23fabdcf29d82181c14e0b3f9399e21f0f16ae2c3126b0749ca9141deb06` (unchanged vs B1) |

`bank.js` and `interact.rs` hashes match `05l` extra-hashes. The named
open mapping did not move between frozen B1 and this HEAD. `host-play`
has later loc-identity work; OpenBooth / WalkNearestBank dispatch was
read at HEAD and still routes named `OpenBooth` to `open_named_booth_at`
and `WalkNearestBank` to packed booth loc + `route_with_radius(..., 1)`.

## 1. What foreign `Bank.openNearest` actually is

Frozen `Bank.ts` 470–510 (both catalogs, identical):

1. Up to 6 attempts while `!Bank.isOpen()`.
2. `Locs.query().name(boothName).where(actions.length > 0).nearest()`.
3. First `booth.interact(chosen)` with op fallback
   (`exact op` / `/^use|^bank/i` / `acts[0]`).
4. Wait **8000 ms** for `Bank.isOpen()` or chat-continue; object-bank
   dialog is a separate continue helper.
5. If `booth.distance() > 1`: `bankStand(booth.tile())` then
   `Traversal.walkTo(stand, { radius: 1, timeoutMs: 15000 })`, else
   `walkTo(booth.tile(), { radius: 1, timeoutMs: 15000 })`.
6. Re-query an adjacent named loc (`distance() <= 1`) and interact;
   wait **4000 ms**.
7. `openedReady`: `isOpen` then `waitReady(4000)`.

`bankStand` (531–548) is four cardinal neighbours of the booth loc,
filtered by `Reachability.canReach`, then nearest Chebyshev to the
player. It is reachable-counter selection, not geometric signum.

`Bank.openBooth(stand, …)` (342–378) is a different helper: after a
failed click it walks the **supplied stand** radius 1 with **90_000 ms**,
then adjacent interact. Do not clone either retry body, dialog continue,
or action fallback. Do not raise walk timeouts to 90 s / 120 s.

VialFiller `bankLeg` (frozen): `walkTo(this.bankStand)` with Chebyshev
early-out ≤ 4 else `walkResilient(..., { radius: 3, attempts: 2,
timeoutMs: 60_000 })`, then `Bank.openNearest('Bank booth',
'Use-quickly')`. East stand is `(3013,3355,0)`. The caller does not pass
that stand into `openNearest`. Ordinary 45-card callers of
`Bank.openNearest(name, op)` (Alcher, Superheater, Tanner, FlaxPicker,
BankFletcher fallback, RuneCrafter fallback, Firemaker, LeatherCrafter,
and the combat/agility bank returns) depend on this helper, not on
VialFiller policy.

## 2. What the bridge does now

`Bank.openNearest` is `return Bank.openBooth(undefined, boothName, op)`
(`bank.js` 347–348). With no stand there is **no approach walk**. Named
filter + distance sort posts `op: 'open-booth'` with `x,z,level,id,name,
action`. Wait is `waitSnapshotAfter(generation, 5000)`.

Host `InteractReq::OpenBooth` → `open_named_booth_at` (`host-play`
1585–1597, `interact.rs` 947–1002). Identity must still match; name/op
mismatch is `StaleTarget`, never a different loc or `Use-quickly`
fallback. If Chebyshev > 1 it **returns `self.walk(dest)`** to the
signum-adjacent tile — `WireCommand::Walk`, not packed nav, not a
reachable counter, not a door leg.

`05l` East return: arrive `(3010,3352)`, OpenBooth `(3011,3354)` id 2213
Use-quickly eleven times, inferred dest `(3010,3353)`, player never
leaves the stall tile, no Close, no generation bump. That is (b) doing
what it is specified to do from a tile its geometric neighbour cannot
accept. It is not proof that Use-quickly from range should be added.

## 3. Existing native bank-access paths

These already exist. None of them is the current `openNearest` mapping.

| Path | What it does | Why it is not a drop-in by itself |
|---|---|---|
| `open_named_booth_at` / `open_booth_at` | Identity-preserving loc op; Chebyshev > 1 → geometric Walk | Intentional low-level contract. Preserve. |
| `open_nearest_booth` | Nearest same-plane `Use-quickly`, then `open_booth_at` | Drops selected name/action/id. Still geometric approach. |
| `openBooth(stand, name, op)` | `walk-near` stand radius 1, wait 120 s, then named open-booth | Requires a stand the `openNearest` caller did not pass. 120 s exceeds the family bound. |
| `openNearestAccess` | Default name/op only; `walk-near` **nearest_booth loc** radius 1, wait **60_000**, then **unnamed** `openBooth()` | Closest existing composition. Throws `not impl` for non-default access. Unnamed click drops posted name/action. |
| `openNearestWorld` / `WalkNearestBank` | Packed `BankStand` loc tile (content `bankbooth` placements, not counters), `route_with_radius(..., 1)` | Nearest packed booth globally. Does not retain the script-selected loc identity. |
| PeriodicBank Access → Open | If dest set: `walk-near` dest radius **1**, 60_000 ms; else `walk-nearest-bank`; then `open-booth` with id and optional name/action; wait generation | Rust-owned sequencer for periodic banking. The **ops** are the ones `openNearest` should emit. Do not hijack its token/session. |
| Posted `Reachability.canReach` | Scene flood bits, including coordinate tiles | Enough to clone `bankStand` in JS. That would be a foreign router. Do not. |

Packed `BankStand.tile` is the **booth loc tile**, `BankAccess::Booth { op: 2 }`
(`derive_banks`, `pack.rs` 120–137, 711–736). `WalkNearestBank` therefore
walks to within 1 of a loc tile via `ScriptWalkArm`, which is packed nav
and already owns door/transport legs. Geometric `Walk` does not.

`ScriptWalkArm` refuses when a `bank_fetch` session or conflicting route
is armed; Pause does not drain the isolate interact queue; guardian hold
drops dispatch and leaves the parked wait frozen; Stop/reset_session
abort. A shim `WalkNear` + named `OpenBooth` inherits that lifecycle.
Do not add a new open-bank session, hold token, or Pause clock.

Door/route ownership stays on packed `Traveller` / `ScriptWalkArm`. Do
not add a foreign door router, `openFirst`, obstacles, or coordinate
exceptions for `(3010,3352)` / `(3011,3354)`.

## 4. Smallest authorized composition

No new public `Interactions` method. No JS `bankStand` / retry / dialog
/ action-fallback port. Name-map `Bank.openNearest` onto verbs that
already exist:

1. Keep today's named loc selection (same plane, name **and** action,
   nearest by posted distance, require integer `x,z,level,id`).
2. If Chebyshev to that loc ≤ 1: queue named `open-booth` with the
   posted `x,z,level,id,name,action`. Wait `waitSnapshotAfter(generation,
   5000)` as today. Completion is `Bank.snapshotReady() && generation >
   baseline` while still open.
3. If Chebyshev > 1: queue `walk-near` to **that selected loc tile**,
   `radius: 1`, `allow_teleports: false`. Wait adjacent Chebyshev ≤ 1
   with the existing family bound **60_000 ms** (`openNearestAccess` /
   PeriodicBank `WALK_BOUND_MS`). On timeout return false. Do not walk
   a packed nearest-bank stand (identity loss). Do not walk the
   VialFiller script stand (that is the caller's `walkTo`).
4. Then queue the **same** posted identity as named `open-booth`. Host
   `open_named_booth_at` still refuses `StaleTarget` if the loc/name/op
   vanished after the walk. Wait generation as in (2).
5. Already-open: keep `waitReady(5000)`.

That is PeriodicBank Access→Open without its token, without
`walk-nearest-bank`, and with name/action required for the named
`openNearest` family.

Do **not** put this walk on unnamed `Banking.open()` /
`openBooth()` with no name/op. Isolate
`isolate_banking_open_queues_open_booth_without_walk` encodes that
click. Unnamed nearest Use-quickly stays on (b).

Do **not** implement `openFirst`, NPC/`openNpcAccess`, obstacles, or
chest-door policy. Non-default `openNearestAccess` stays the honest
unsupported throw. A named `openNearest('Bank chest', 'Use')` is the
named loc path in (1)–(4), not `not impl`, once that name/op exists on
a scene loc.

Original operation/order/timeouts to preserve:

| Bound | Keep |
|---|---|
| Low-level OpenBooth | Chebyshev ≤ 1 then loc op; else one geometric Walk. No timeout change. |
| Named click wait | `waitSnapshotAfter` **5000** |
| Approach wait | **60_000** already used by `openNearestAccess` / PeriodicBank. Do not raise to 90_000 / 120_000. Do not clone foreign 15_000. |
| `waitReady` when already open | **5000** |
| Foreign 6 attempts / 8000 / 4000 / dialog continue | Do not clone. VialFiller already retries `bankLeg`. |
| Stop / reset_session / logout | Abort false; drop latch. No new token. |
| Pause | Do not drain queued interact; do not send. Resume continues remaining wait. |
| Hold | Drop dispatch; parked wait stays frozen. |

Selected-bank identity: the posted loc `id/name/action/tile` from step 1
rides through the walk. Do not retarget to another booth at dispatch.
Revision/profile identity is unchanged (no client/bot-action API).

## 5. Recommended implementation brief

Bounded shim-only hop. Assignee: `luna` or `implementer`. Own
`crates/script/src/shim/bank.js` and the isolate tests that pin
`openNearest` queue shape. Do not edit `crates/api/src/interact.rs`,
`host-play` OpenBooth geometric walk, packed nav, PeriodicBank tokens,
fixtures, ledger, or catalog scripts. Shared isolate request schema is
already `walk-near` + `open-booth`; no new vtable id.

Meaningful negative tests (behavior, not source snapshots):

- Named `openNearest` from Chebyshev 2 queues `WalkNear` radius 1 to the
  selected loc, then named `OpenBooth` with the same id/name/action.
  It must not be a lone `OpenBooth` (today's mapping).
- Named `openNearest` from Chebyshev 1 queues only named `OpenBooth`.
- After the walk snapshot, missing/mismatched loc returns false and
  does not invent another booth.
- `open_named_booth_at` Chebyshev > 1 still geometric `Walk`
  (`open_nearest_booth_walks_when_not_adjacent` stays green).
- Unnamed `Banking.open()` still queues `OpenBooth` without `WalkNear`.
- Non-default `openNearestAccess` still throws `not impl`.
- Do not assert a 6-attempt loop, dialog continue, or `bankStand`.

Root LIVE falsifier, after that hop is reviewed and built isolated:

- Host `2f1bd9cf` / client `9d090ed`, old-catalog `vial_filler_east`,
  both revisions. Witness: fill 28 × 227, return from the fountain,
  named open produces Close / `fresh_bank_item_id(227)>=1`. Must leave
  the `(3010,3352)` stall without editing `interact.rs`.
- Control: default `vial_filler` still PASS on the same binary.
- If East still FAILs with WalkNear-then-named-open logged and
  unchanged OpenBooth, remaining cause is packed-nav/door reach to
  that loc, not an imported card. Reclassify; still no dim from `05l`.
- An East PASS that required loc-op-from-range, East coordinate
  exceptions, or a cloned `Bank.ts` body falsifies this brief.

## Limits

- No LIVE, no product edits, no ledger/enabled writes.
- Geometric dest `(3010,3353)` remains inferred as in `05l`; the stall
  tile, absent Close, and absent generation bump are the observed facts.
- Packed Falador East door legs were not re-simulated here; the
  composition uses the existing nav arm rather than claiming a measured
  East PASS.
- Newer-catalog East was not in the B1 batch; scripts/`Bank.ts` are
  byte-identical across catalogs.
- This card does not implement loc-identity repair `t_c35281b3`.
- `04-banking-design` named-without-stand row documented click-only
  mapping. Brief 79 supersedes that row for ordinary `openNearest`.
  Unnamed nearest Use-quickly is unchanged.
