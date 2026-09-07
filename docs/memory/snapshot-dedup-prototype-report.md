# Snapshot rebuild-edge exact dedup prototype (A1 discriminator)

Implements the bounded A1 experiment from `architecture-a-host-observations.md`
(smallest discriminating experiment) and `architecture-synthesis.md` lane A.
**No production owner wiring.** Public snapshot types and rebuild gates unchanged.
No live/network/RSS claims. No client git mutations.

## Checkout

| Item | Value |
|------|-------|
| Branch | `codex/memory-diagnostics` |
| Workspace | `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f` |
| Target dir | `CARGO_TARGET_DIR=…/target-snapshot-dedup-proto` (dedicated; does not contaminate native hosts) |
| Design anchors | A1 + CX1–CX6 (`snapshot-family-sharing-report.md`); behavior-contract game fidelity |

## Mechanism (prototype only)

| Piece | Role |
|-------|------|
| `GameSnapshot` rebuild edge | Real `rebuild_family` for Widgets / SideTabs / Loc; existing `InvIfaceGate` and scene+`loc_model_stamp` gates |
| `DedupCursor` ×3 | Owner-local gates/history via internal `GameSnapshot`; published bodies held as `Arc<Vec<…>>` |
| `SlotFamilyRegistry` | Per-slot, max 3 weak slots per family; overwrite on replace; clear on unregister/drop |
| Equality | Full content compare after a completed rebuild only. Widgets/SideTabs use derived `PartialEq`; Loc uses explicit total-field `loc_eq` (no derived PartialEq on `LocView`) |
| Quiet path | Gate miss → no walk, no equality, no registry lookup, no allocation |
| Not used | Gen-only share stamp; read-path deep compare; hash-as-equality; production host/panel/host-play wiring |

Minimal api support (no production behavior change): `proto_take_*` / `proto_put_*` /
`proto_capacity_*` on `GameSnapshot` to extract owned family bodies after the real
rebuild without altering gates or public getters.

## Files

| Path | Action |
|------|--------|
| `crates/api/src/snapshot_dedup_proto.rs` | Prototype registry + cursors + counters + payload accounting |
| `crates/api/src/snapshot.rs` | Doc-hidden take/put/capacity hooks only |
| `crates/api/src/lib.rs` | Export `snapshot_dedup_proto` |
| `crates/api/tests/snapshot_dedup_proto.rs` | Deterministic fixture tests (CX1–CX6 + share/quiet/lifecycle) |
| `docs/memory/snapshot-dedup-prototype-report.md` | This report |

## Proof matrix

All under `cargo test -p api --test snapshot_dedup_proto` with dedicated target dir.
**12 passed / 0 failed.**

| Case | Test | Result |
|------|------|--------|
| Equal populated bodies share (`ptr_eq` across 3 cursors) | `equal_populated_bodies_share` | PASS |
| Quiet read zero equality/walk work | `quiet_read_zero_work` | PASS |
| CX1 tab click, unequal gate history, no share | `cx1_tab_click_unequal_gate_history_no_share` | PASS |
| CX2 scroll without gen | `cx2_scroll_without_gen_no_share` | PASS |
| CX3 local inv drag without inv gen | `cx3_local_inv_drag_without_gen_no_share` | PASS |
| CX4 text rewrite without gen | `cx4_text_without_gen_no_share` | PASS |
| CX5 tutorial root without iface gen | `cx5_tutorial_root_without_gen_no_share` | PASS |
| CX6 loc distance after player move | `cx6_loc_distance_player_move_no_share` | PASS |
| Older publication immutable after replace | `older_publication_immutability` | PASS |
| Drop/restart identity; weak cleanup | `drop_restart_identity` | PASS |
| Prototype body == independent `GameSnapshot` rebuild | `prototype_matches_independent_game_snapshot_builders` | PASS |
| Walk / equality counters recorded | `equality_and_walk_counts_recorded` | PASS |

CX scenarios use **unequal owner gate history**: O₁ rebuilds and publishes; local
mutation without gen bump; O₂ still behind gates walks independently and must
**not** install O₁’s Arc when content differs. Quiet O₁ already at the gate
keeps stale content (existing single-owner behavior preserved).

## Allocation accounting (fixture bytes, not RSS)

Measured on the equal three-owner populated fixture after three family rebuilds
each (probe during development; same path as `equal_populated_bodies_share`):

| Metric | Bytes |
|--------|------:|
| Old per-owner private payload (unique × holders) | 23 718 |
| Unique live body payload | 7 906 |
| Scratch peak (candidate body) | 3 800 |
| Registry metadata | 72 |

Interpretation: with k=3 equal bodies, unique ≈ old/3. Net live reduction on this
fixture is `old − unique − registry ≈ 15 740` bytes of **estimated nested
payload**, before counting Arc headers and ignoring allocator freelists.
**Not RSS. Not N16. Not a fleet hit-rate.** Synthetic fixture bodies are small;
the mechanism-level claim is only that equal completed rebuilds release
duplicate owned bodies safely.

Counters on that equal run: walks=9 (3 families × 3 owners), equality
comparisons=6, hits=6, publishes=3 (one publish per family from the first owner).

## What this does **not** claim

- No production migration of host / host-play / panel cursors.
- No live allocation or RSS screen; no 10 MiB N16 go/no-go (needs separate
  authorized capture after integration).
- No CPU/p99 comparison of equality cost under load.
- No A2 normalization (Loc distance still baked into body; CX6 correctly
  refuses share when distances differ).
- No change to 20 ms logical client progression, script surface, or client fence.

## Verdict for root

**A1 mechanism discriminator: pass on safety + body-removal feasibility** under
real builders and CX1–CX6 counterexamples. Equal bodies share; quiet frames do
zero extra work; unequal content under equal gens does not share; older Arcs
remain stable; drop/restart clears weak identity.

Next root-owned steps (not this card): decide production wiring behind review;
optional matched native owner capture for overlap frequency; 10 MiB threshold
from architecture-a before expanding the lane; whole-branch Grok4.6 still required
later. A2 borrowed read surface remains a separate authorization.

## Command

```bash
export CARGO_TARGET_DIR=/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f/target-snapshot-dedup-proto
cargo test -p api --test snapshot_dedup_proto
# 12 passed; 0 failed
```
