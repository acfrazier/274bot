# Share Play NavWorld with memory seed preparation

Task `t_513c266a`. Implements proposal §4 of
[low-end-owner-attribution.md](low-end-owner-attribution.md) (`1f086b0`).

**Code change only.** No native profiling, live runs, or RSS acceptance in this
card. Reviewer code approval is not performance acceptance.

## Problem

At N=1 active, panel and TUI both decoded the nav pack twice:

1. `Play::new` → `NavWorld::load_pack` (production owner, `Option<Arc<NavWorld>>`)
2. `memory::Run::prepare` → separate `load_pack` into seed runners

Combined filter ~140.988 MiB alloc; MALLOC_LARGE resident 137.6M on both
frontends. ScenarioRunner-per-slot fan-out was already fixed earlier; this card
only removes the remaining Play vs prepare duplicate.

## Change

### API (`host-play` memory harness)

| Symbol | Role |
|:---|:---|
| `SeedNav::LoadDefault` | Unit tests without a Play: one `load_pack` for seeds |
| `SeedNav::FromPlay(Option<Arc<NavWorld>>)` | Frontend path: clone Play’s Arc, or keep `None` |
| `Run::prepare_unseeded` | Vault, names, card, outputs — **no** seed runners |
| `Run::bind_seed_nav` | Install seed runners from `SeedNav` **before** slot spawn |
| `Run::prepare` | `prepare_unseeded` + `SeedNav::LoadDefault` (unit-test convenience) |
| `Run::prepare_with_seed_nav` | `prepare_unseeded` + explicit `SeedNav` |

### Frontend sequence (single Play, single decode on success)

1. `Run::prepare_unseeded(config, frontend)` — mint vault/card; empty `SEEDS`
2. Construct **one** `Play` via existing unlock/`start_vault` → `run_with_io` →
   `Play::new` (sole successful `load_pack`, or `None` if pack missing)
3. `run.bind_seed_nav(SeedNav::FromPlay(play.world()))` — Arc clone only; **no**
   second decode; `FromPlay(None)` does not retry `load_pack`
4. Spawn slots / login

Call sites:

- `panel` `boot_execute` Memory + `Session::prepare_memory`
- `tui` memory branch in `run`

Production `Play::new` / interactive unlock paths are unchanged. No global nav
cache, no routing/bounds changes, no ScenarioRunner rewrite.

### Minimal accessor

`ScenarioRunner::shared_world()` returns `Option<Arc<NavWorld>>` (Arc clone) so
harness tests can assert pointer identity without exposing runner internals.

## Failure / missing-pack semantics

| Case | Behavior |
|:---|:---|
| Pack loads in `Play::new` | Seeds share that Arc |
| Pack fails in `Play::new` (`world == None`) | `FromPlay(None)` → seeds get `None`; walk/follow still fail with existing “no nav world” messages; **no** second decode attempt |
| Unseeded idle | No seed runners (empty map); Play may still hold its own world for operator routing |
| Unit `Run::prepare` without Play | `LoadDefault` still decodes once for non-idle seeds (test-only path) |

## Tests run

```
cargo test -p host-play --features memory-profile-no-alloc --lib
  → 146 passed (incl. bind_seed_nav_from_play_preserves_arc_identity,
    bind_seed_nav_from_play_none_keeps_missing_pack,
    prepare_with_seed_nav_from_play_matches_bind,
    seed_runners_share_world_but_keep_independent_state)

cargo test -p panel --features memory-profile-no-alloc --lib
  → 377 passed

cargo test -p tui --features memory-profile-no-alloc
  → 87 passed

cargo test -p scenario --lib -- walk_step_fails_without_a_nav_world
cargo test -p scenario --lib -- follow_step_fails_without_a_nav_world
  → both ok
```

No live / native owner / clean RSS proof on this card (Linux build may overlap).

## Provisional status

| Claim | Status |
|:---|:---|
| Dual successful decode removed on memory frontend path | **Code-complete** (Arc share) |
| Missing-pack still `None` without retry | **Tested** |
| ScenarioRunner third-copy regression | Not reintroduced (prior share retained) |
| Native owner count one ~62 MiB primary group | **Not measured** (follow-up) |
| Clean TUI n1/n16 RSS vs 380.7 / 1001.4 reference | **Not measured** (follow-up) |
| Budget pass (TUI ≤256 / ≤512, etc.) | **Not claimed** |

Expected RSS if second decode was fully resident: order-of **tens of MiB** fixed
base, not full 62 MiB a priori (attribution §4).

## Candidate release build commands (subsequent isolated proof)

Quiet host; no concurrent agent/test/build. System allocator; no stack logging
for clean cells.

```bash
# Reference-style release binaries (adjust cache/pack/RS2B0T to match frozen matrix)
cargo build --release -p tui --bin tui-play --features memory-profile-no-alloc
cargo build --release -p panel --bin panel-play --features memory-profile-no-alloc

# Record sha256 of target/release/{tui-play,panel-play}

# Optional native owner check (diagnostic_only; not clean RSS):
# BOT_MEMORY_N=1 BOT_MEMORY_WORKLOAD=active LIVE=1 BOT_MEMORY_DIAGNOSTICS=1 \
#   stack-logging-lite capture during observe — expect one load_pack primary group

# Clean matched TUI 1/16 (profiles-on short screen; no stack logging):
# BOT_MEMORY_N=1|16 BOT_MEMORY_WORKLOAD=active LIVE=1 \
#   render_policy=none — median RSS vs low-end-reference-screen-table.json
# qualify: python3 docs/memory/qualify_control.py RUN --no-write
```

Panel focused-one optional after TUI. Do not mix diagnostic lite RSS into clean
claims.

## Files touched

- `crates/host-play/src/memory.rs` — SeedNav, prepare_unseeded, bind_seed_nav, tests
- `crates/scenario/src/runner.rs` — `shared_world()`
- `crates/panel/src/app.rs`, `crates/panel/src/session.rs` — boot bind path
- `crates/tui/src/bin.rs` — boot bind path
- `docs/memory/shared-play-nav-world-report.md` — this report

Not modified: `reference_metrics.py`, `docker/`, `STATE.md`.

## Remaining

1. Isolated native owner lite N=1 (TUI ± panel): confirm single large `load_pack`
2. Clean quiet-host TUI n1/n16 RSS vs frozen reference
3. Re-attribute residual fixed cost after measured share
