# Snapshot frame equivalence report

Branch: `codex/snapshot-frame-equivalence`  
Worktree: `/Users/acfrazier/experiments/274bot/.worktrees/snapshot-frame-equivalence`  
Dedicated target: `CARGO_TARGET_DIR=.../target-frame-eq`  
Scope: offline Client fixtures through real Host / host-play / panel publication paths.  
No client/GPU/network edits. Optional `snapshot-dedup` remains default off.

## Production paths exercised

| Path | Symbol | Call site |
| --- | --- | --- |
| Host after_drain body | `host::drain_and_rebuild_snapshot` | `SlotLoop::after_drain` → pump.drain + `rebuild_dirty` (widgets/side tabs/loc via dirty.iface/inv/scene) |
| host-play nav/script | `host_play::observe_rebuild_snapshot` | tick-edge observe → `GameSnapshot::rebuild`; off-edge keeps last blob |
| Panel nav_states | `panel::session::publish_nav_snapshot` | per-frame nav publish: insert/join slot, optional Play-local `SlotDedupDirectory` attach, rebuild, `WorldState::from_snapshot` |

Test-only accessors: the three helpers above are the production bodies exposed for offline drive (no alternate rebuild map). Panel production call site now invokes `publish_nav_snapshot` (same code path).

## Fixtures

Deterministic offline plant (aligned with `crates/api/tests/snapshot_dedup_proto.rs`):

- Main modal widgets 1000/1001/1002 (Hello text), side tabs 1100/1500, one wall loc "Large door"
- Gen bumps: `IF_SETTEXT`, `UPDATE_INV_FULL`, `REBUILD_NORMAL`
- No network / live client

Observable transcript (`FamilyObs`): widget ids/texts, side-tab roots/available, loc ids/names, gens.iface/inv/scene.

## Commands and results

```bash
export CARGO_TARGET_DIR=.../target-frame-eq

# Feature off (default)
cargo test -p host-play --test snapshot_frame_equivalence --offline
# 5 passed

cargo test -p panel --lib publish_nav_snapshot --offline
# 2 passed

# Feature on
cargo test -p host-play --test snapshot_frame_equivalence --features snapshot-dedup --offline
# 11 passed (5 base + 6 with_dedup)

cargo test -p panel --lib publish_nav_snapshot --features snapshot-dedup --offline
# 3 passed (2 base + 1 share)
```

Smoke (existing unit tests still green after expose):

- `cargo test -p host --lib after_drain` → `client_frame_kicks_guardian_after_drain` ok
- `cargo test -p host-play --lib observe_rebuild_snapshot` → `observe_rebuild_snapshot_skips_off_tick_edge` ok

## Evidence matrix

### Feature off (observables / gates)

| Case | Result |
| --- | --- |
| Host first publication | widgets 1000/1001 + Hello, side tab 1100, one Large door loc |
| Host quiet drain | `dirty.any()==false`; widget/side/loc storage ptrs unchanged |
| host-play tick edge | rebuild true, populated families |
| host-play off-edge | rebuild false; last blob retained (ptr stable) |
| Equal repeated edge | same FamilyObs (gate quiet) |
| Host vs host-play FamilyObs | exact `assert_eq!` |
| Retained old observation after widget change | old "Hello" copy stable; new snap "Changed" |
| Panel first publish | widgets/side/loc populated; rebuild returns true |
| Panel quiet publish | rebuild false; body ptrs stable |

### Feature on (Arc identity + lifecycle)

| Case | Result |
| --- | --- |
| Host + nav same instance equal bodies | `Arc::ptr_eq` widgets/side_tabs/locs; equality_hits ≥ 1 |
| Three owners equal | all share; unequal second client → no share + equality_misses ≥ 1 |
| Quiet observe | quiet_skips advance; equality_comparisons/hits unchanged |
| Slot restart (directory remove + new instance) | no Arc share with prior body; fresh hits == 0 |
| Two Play directories, same username | no Arc share; FamilyObs still equal |
| Retained Arc while peer equal rebuild | ptr_eq retained vs peer; text still Hello |
| Panel + directory + peer owner | panel nav_states shares Arc with peer after rebuild |

## Gaps (faithful)

1. **Full multi-thread SlotLoop / Play loop** not driven end-to-end: proof uses the extracted production publication bodies with offline Client gens, not a live `run_with_io` thread pumping packets. The dirty-family map and tick-edge gate are the real ones.
2. **Cross-build golden transcript dump** not written to disk: equivalence is asserted in-process (FamilyObs PartialEq) under both feature configs in the same suite; feature-off and feature-on are separate cargo invocations, each asserting the same observable expectations.
3. **Focus/render GameView owner** not in scope (no client/GPU edits); coexistence covered is Host snapshot owner + host-play nav owner + panel nav_states owner (and peer owner on same instance).
4. **Changed side tabs / locs** covered via plant + scene bump on first publish and unequal widget miss path; a dedicated side-tab-only mutation case is thinner than widgets (loc change would need wall rewrite + scene bump — first publish already proves loc path).
5. Production helpers were made `pub` for test drive; behavior of call sites is unchanged (thin wrapper / extract).

## Files

- `crates/host-play/tests/snapshot_frame_equivalence.rs` (new)
- `crates/host/src/lib.rs` — `drain_and_rebuild_snapshot` pub + after_drain uses it
- `crates/host-play/src/lib.rs` — `observe_rebuild_snapshot` pub
- `crates/panel/src/session.rs` — `publish_nav_snapshot` + unit tests
- `docs/memory/snapshot-frame-equivalence-report.md` (this file)

## Verdict

Required real-path proof is closed for Host drain rebuild, host-play observe rebuild, and panel nav_states publication under feature off and on, including Arc share/isolation and retained observations. No failing production defect found; no silent skips.
