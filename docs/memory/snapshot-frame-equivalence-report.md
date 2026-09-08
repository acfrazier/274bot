# Snapshot frame equivalence report

Branch: `codex/snapshot-frame-equivalence`  
Worktree: `/Users/acfrazier/experiments/274bot/.worktrees/snapshot-frame-equivalence`  
Dedicated target: `CARGO_TARGET_DIR=.../target-frame-eq`  
Scope: offline Client fixtures through real Host / host-play / panel publication paths.  
No client/GPU/network edits. Optional `snapshot-dedup` remains default off.

## Production paths exercised

| Path | Symbol | Call site |
| --- | --- | --- |
| Host after_drain body | `host::drain_and_rebuild_snapshot` | `SlotLoop::after_drain` → pump.drain + `rebuild_dirty` (widgets/side tabs via dirty.iface/inv; loc via dirty.scene) |
| host-play nav/script | `host_play::observe_rebuild_snapshot` | tick-edge observe → `GameSnapshot::rebuild`; off-edge keeps last blob |
| Panel nav_states | `panel::session::publish_nav_snapshot` | per-frame nav publish: insert/join slot, optional Play-local `SlotDedupDirectory` attach, rebuild, `WorldState::from_snapshot` |

Test-only accessors: the three helpers above are the production bodies exposed for offline drive (no alternate rebuild map). Panel production call site invokes `publish_nav_snapshot` (same code path). Behavior of call sites is unchanged (thin extract / pub visibility).

## Fixtures

Deterministic offline plant (aligned with `crates/api/tests/snapshot_dedup_proto.rs`):

- Main modal widgets 1000/1001/1002 (Hello text), side tabs 1100/1500 with nested texts tab3/tab5, one wall loc "Large door"
- Mutations: widget text → "Changed"; side-tab nested text → "tab3-mut"; loc typecode → second LocType "Open door"
- Gen bumps: `IF_SETTEXT`, `UPDATE_INV_FULL`, `REBUILD_NORMAL` as appropriate
- No network / live client

Observable transcript (`FamilyObs`): widget ids/texts, side-tab roots/available/active/nested texts, loc ids/names, gens.iface/inv/scene. Gate bits on host steps (`dirty_*`) and host-play steps (`rebuilt`).

## Cross-feature golden / transcript

One deterministic multi-frame transcript is produced by the real Host and host-play paths (`run_cross_path_transcript` in `snapshot_frame_equivalence.rs`):

1. Host: first → quiet → widget change → quiet → side-tab change → quiet → loc change → quiet  
2. host-play: first → off-edge quiet → equal edge → widget change → off-edge → side-tab change → off-edge → loc change → off-edge  

Committed golden (feature-agnostic observables + gate history):

- `crates/host-play/tests/fixtures/snapshot_frame_transcript_golden.json`  
  schema: `snapshot-frame-equivalence-v2`  
  Regenerate: `UPDATE_SNAPSHOT_FRAME_GOLDEN=1 cargo test -p host-play --test snapshot_frame_equivalence transcript_matches_golden_and_peer_feature_build --offline -- --exact`

Each feature build:

- Asserts live transcript **exactly equals** the committed golden (same values/order/gate histories).
- Writes `CARGO_TARGET_DIR/snapshot_frame_transcript_feature_{off|on}.json`.
- If the peer feature dump already exists, asserts **exact equality** live vs peer (true cross-build load/compare).

This is not same-build self-consistency alone: feature-off and feature-on both bind to one golden, and the second cargo invocation loads the first’s dump.

## Commands and results

```bash
export CARGO_TARGET_DIR=.../target-frame-eq

# Feature off (default) — 9 passed
cargo test -p host-play --test snapshot_frame_equivalence --offline

# Feature on — 16 passed (9 base + 7 with_dedup)
cargo test -p host-play --test snapshot_frame_equivalence --features snapshot-dedup --offline

# Peer dump present after both runs; re-run either for live↔peer assert
ls "$CARGO_TARGET_DIR"/snapshot_frame_transcript_feature_*.json

cargo test -p panel --lib publish_nav_snapshot --offline
# 2 passed

cargo test -p panel --lib publish_nav_snapshot --features snapshot-dedup --offline
# 3 passed (2 base + 1 share)
```

Smoke (existing unit tests still green after expose):

- `cargo test -p host --lib after_drain` → `client_frame_kicks_guardian_after_drain` ok (not re-run this pass; prior round)
- `cargo test -p host-play --lib observe_rebuild_snapshot` → `observe_rebuild_snapshot_skips_off_tick_edge` ok (prior round)

## Evidence matrix

### Feature off and on — shared observables / gates (via golden)

| Case | Host | host-play |
| --- | --- | --- |
| First publication | dirty_any true; Hello / tab3 / Large door | rebuilt true; same FamilyObs |
| Quiet after first | dirty_any false; obs unchanged | off-edge rebuilt false; equal edge same obs |
| Widget changed | dirty_iface; text Changed | rebuilt; same obs as host |
| Quiet after widget | dirty empty; retain | off-edge retain |
| Side-tab changed | dirty_iface; nested tab3-mut; retained old tab3 | rebuilt; same; retained copy |
| Quiet after side-tab | dirty empty; body ptr stable | off-edge retain |
| Loc changed | dirty_scene; Open door; retained Large door | rebuilt; same |
| Quiet after loc | dirty empty; body ptr stable | off-edge retain |
| Host vs host-play FamilyObs | exact at first / widget / side-tab / loc steps | (same) |
| Golden + peer feature dump | exact PartialEq | exact PartialEq |

### Feature on only (Arc identity + lifecycle)

| Case | Result |
| --- | --- |
| Host + nav same instance equal bodies | `Arc::ptr_eq` widgets/side_tabs/locs; equality_hits ≥ 1 |
| Three owners equal | all share; unequal second client → no share + equality_misses ≥ 1 |
| Quiet observe | quiet_skips advance; equality_comparisons/hits unchanged |
| Slot restart (directory remove + new instance) | no Arc share with prior body; fresh hits == 0 |
| Two Play directories, same username | no Arc share; FamilyObs still equal |
| Retained Arc while peer equal rebuild | ptr_eq retained vs peer; text still Hello |
| Side-tab / loc change under dedup | prior Arc not shared after change; quiet_skips after |
| Panel + directory + peer owner | panel nav_states shares Arc with peer after rebuild |

### Panel (first + quiet + share)

| Case | Result |
| --- | --- |
| First publish | widgets/side/loc populated; rebuild true |
| Quiet publish | rebuild false; body ptrs stable |
| Directory join (feature-on) | Arc share with peer |

Panel does **not** re-drive side-tab/loc mutation histories (Host + host-play cover those; panel fixture is thinner on nested side-tab widgets).

## Gaps (faithful)

1. **Full multi-thread SlotLoop / Play loop** not driven end-to-end: proof uses the extracted production publication bodies with offline Client gens, not a live `run_with_io` thread pumping packets. The dirty-family map and tick-edge gate are the real ones.
2. **Focus/render GameView owner** not in scope (no client/GPU edits); coexistence covered is Host snapshot owner + host-play nav owner + panel nav_states owner (and peer owner on same instance).
3. **Slot restart / two Plays** are directory-level stand-ins (remove/install and two `SlotDedupDirectory`s), not full thread stop/spawn.
4. **Feature-on quiet_skips** path is host-play observe-only; Host quiet under dedup is storage-ptr / empty dirty only (no quiet_skips counter on Host drain when gens unchanged without calling family rebuilds).
5. **Panel side-tab/loc mutation** not mirrored; Host + host-play prove those change+stale histories.
6. Production helpers remain `pub` for test drive; call-site behavior unchanged.

## Files

- `crates/host-play/tests/snapshot_frame_equivalence.rs` (expanded)
- `crates/host-play/tests/fixtures/snapshot_frame_transcript_golden.json` (committed golden)
- `crates/host-play/Cargo.toml` — serde dev-dep for transcript serde
- `crates/host/src/lib.rs` — `drain_and_rebuild_snapshot` pub + after_drain uses it (prior)
- `crates/host-play/src/lib.rs` — `observe_rebuild_snapshot` pub (prior)
- `crates/panel/src/session.rs` — `publish_nav_snapshot` + unit tests (prior)
- `docs/memory/snapshot-frame-equivalence-report.md` (this file)

## Verdict

**Closed for the reviewer’s three blocking items:**

1. Cross-feature exact compare: committed golden + per-feature dumps under `CARGO_TARGET_DIR` with peer load when present. Feature-off and feature-on both match the same golden transcript (values, order, gate histories).
2. Side-tab and loc change + quiet/stale retain proven on real Host and host-play paths (not first-plant only).
3. This report does not claim full multi-thread Play/GameView coverage; gaps above stay open.

Still out of scope / incomplete vs the absolute widest reading of the task: live multi-thread loop, GameView owner, full slot thread restart. No failing production defect found; no silent skips.
