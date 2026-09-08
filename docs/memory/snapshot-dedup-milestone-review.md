# Snapshot-dedup corrected ownership + accounting — pre-native integration review

Task: `t_3e707aee`
Reviewer: Hermes profile `branchreviewer` (Grok 4.6 / xai-oauth)
Verdict: APPROVED as a pre-native instrument/integration milestone only
Not: whole-branch review, performance acceptance, RSS/10 MiB/CPU claim, or native overlap evidence

Workspace: `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`
Branch: `codex/memory-diagnostics`
Independent of prior Grok 4.5 favorable labels on `t_cc437480` / `t_df016942`.

## Frozen scope

From `docs/memory/snapshot-dedup-milestone-inputs.json` (HEAD may add unrelated docs/control; those are ignored):

| Ref | Value |
| --- | --- |
| Host base | `9f32a0d2cb440d315d2e54469864eb9d101f922c` |
| Host head (source freeze) | `27d1fe72215807e91ce4ebc3c0a7c4ebfac5a492` |
| Client | `3456edc8dabf7b25ada78110ffa56327af9f67a4` (unchanged; gitlink matches) |
| Feature | `snapshot-dedup`, default off |
| Native overlap evidence | false |
| Scratch peak measured | false |
| Memory savings accepted | false |

Combined crate diff `9f32a0d..27d1fe7` is 13 files / +2144 / −105. Landing commits inside that range:

- `7c05f5c` feat: Arc Widgets/SideTabs/Loc + intern behind the flag
- `d1de456` fix: Play-local `SlotDedupDirectory` / `SlotDedupInstance` + memory-profile envelope
- `27d1fe7` fix: real registered-owner census, honest null scratch, one `census_sample`

Other commits in the same range (lazy-upload/docs) are out of this review. Client tree is untouched.

Inputs read: freeze JSON, `behavior-contract.md`, `snapshot-dedup-candidate-report.md`, production landing report, combined 13-file crate diff, host/host-play/panel attach sites, prior cards `t_cc437480` and `t_df016942`.

## Verdict mapping

| Question | Finding |
| --- | --- |
| Feature-off behavior change? | No. Default features stay empty. Feature-off storage is still `Vec`; rebuild gates and walks match `9f32a0d`. `cargo test -p api --test snapshot` 60 ok; `--test snapshot_dedup` 0 tests. |
| Combined Arc builders + old gates? | Yes. Quiet miss returns before walk/equality/intern. Dirty path walks the original trees then interns only when a handle is attached. Getters still return `&[T]`. |
| Owner drop / Play-local directory / panel / nav / host? | Coherent in source. Process-global `process_slot_table` / `attach_owner_for_slot` are gone from `crates/`. |
| Quiet path work? | Feature-on quiet still skips equality and body walk. It does bump counters; with a handle attached that dual-writes the registry under a mutex. Feature-off has no extra work. |
| Census fake owners via `strong_count`? | Diagnostics path does not. `account_registry_live` upgrades each registered cursor weak once and counts with `Arc::ptr_eq`. |
| Scratch unknown / slot+aggregate? | `scratch_peak_bytes: Option` serializes JSON null. Memory-profile publishes `scratch_peak_available: false` and `census_sample(None)` for slots+aggregate. |
| Full client-frame Arc identity proved? | No. Mechanism + proto tests prove intern/share/teardown. Host/panel lib tests do not assert pointer identity across SlotLoop + nav_snapshot + panel `nav_states` in one client frame. That is a residual, not a source contradiction. |
| RSS / 10 MiB / CPU accepted? | No. |

## Arc bodies and rebuild gates

`GameSnapshot` Widgets / SideTabs / Loc are `Vec` off, `Arc<Vec<_>>` on. `rebuild_widgets` / `rebuild_side_tabs` still gate on `InvIfaceGate::moved`. `rebuild_loc` still gates on scene gen + `loc_model_stamp`. Feature-off bodies of those three functions are the `9f32a0d` walks (clear + tree / 104×104×4 sweep).

Feature-on dirty path: `family_body_mut_*` takes a unique Arc (fresh empty Arc if shared), walks into it, then `intern_*_if_attached`. No handle → no intern, unique Arc retained. Equality is total field compare (`PartialEq` for widgets/side-tabs; explicit `loc_eq` covering every `LocView` field). Self-cursor is skipped; first other-cursor hit returns that Arc.

Host `rebuild_dirty` still only calls Widgets/SideTabs on `dirty.iface \|\| dirty.inv` and Loc on `dirty.scene`. Nav `observe_rebuild_snapshot` still full-`rebuild`s on tick edge. Panel `nav_states` still `rebuild`s on observe. Those call patterns pre-exist; intern rides them.

Immutable-observation contract: sharing is pointer identity of completed bodies. A retained owner keeps its Arc; a later intern cannot mutate a shared body (`Arc::get_mut` fails → fresh candidate). Older publication stability is proto-covered (`older_publication_immutability`).

## Ownership and lifecycle

`SlotDedupInstance` is a slot-run token (registry Arc), not a username key. `SlotDedupDirectory` is Play-local.

Wiring:

- host-play `spawn_slot`: mint instance, `install(username)`, pass into `spawn_slot_thread` before the thread starts.
- `Host::run_client(..., Some(instance))` from host-play; attaches `instance.attach()` and holds `_dedup_keep_alive` for the run. Direct/host-test sites pass `None` and get a run-local instance (no process-wide name leak).
- host-play observe: `nav_snapshot.attach_dedup(dedup_instance.attach())`.
- panel: builds a shared directory before `run_with_io`, clones it into the per-frame hook, `replace_dedup_directory` after Play construction (empty profile list at that call — no spawn-before-replace). Nav attach is `nav_dedup.get(name)` join-only; it does not mint a global registry. Miss on first insert leaves that snapshot unattached for the entry lifetime; spawn installs before thread start, so the first observe should see the instance.
- `Play::stop_slot` removes the directory entry after join; panel `stop` also `nav_states.remove(name)` so Drop unregisters the UI cursor.
- `GameSnapshot` is not `Clone`; feature-on `Drop` detaches. `attach_dedup` unregisters a replaced handle.

Same username on two Plays cannot share: two directories, two instances. Slot restart replaces the instance. Tests: `directory_isolates_same_username_across_plays`, `slot_restart_replaces_instance_without_process_global_leak`, `cx5_directory_lifetime_boundary`, cursor Drop reclaim.

TUI only gained the feature flag. Third-owner UI attach is panel `nav_states`; TUI still gets host + nav sharing through host-play.

## Census and diagnostics

`account_registry_live` (the memory-profile path):

1. One registry lock in `SlotDedupInstance::diagnostic`.
2. Upgrade each cursor's three weaks once.
3. Unique payload once per distinct body identity.
4. Private counterfactual = payload × count of registered owners holding that identity.
5. No `Arc::strong_count` owner synthesis. Remaining `strong_count() == 0` uses are dead-weak cleanup only.

`account_allocations_arcs` still uses `.max(1)` when a unique body has zero matching holders. That helper is not the published census path. Prior Grok 4.5 caveat stands; it does not fabricate overlap in `snapshot_dedup` JSON.

Scratch: `Option<usize>` / JSON null when unmeasured. Envelope `scratch_peak_available: false`. Harness never publishes silent 0.

`SlotDedupDirectory::census_sample` folds aggregate from the same diagnostic rows. `aggregate_diagnostic` still exists and re-samples; the harness uses `census_sample`.

Asymmetric fixtures: two distinct capacities → duplicate 0; three-owner shared+distinct → duplicate == shared payload; dropped cursor / last-owner drop → no invented payload.

JSON has no `rss` key (`cx6`).

## Quiet path

Feature-off: gate miss returns false, no extra work.

Feature-on: gate miss increments snapshot family `quiet_skips` and, if attached, locks the registry to bump the same counter. No equality, no walk, no intern. Proto `quiet_read_zero_work` still passes.

The dual-write lock is feature-on diagnostic cost on every panel `rebuild()` / nav tick-edge `rebuild()` quiet family (three families). It is not a feature-off change and is not CPU proof.

## What this review does not prove

- Pointer identity of Widgets/SideTabs/Loc across SlotLoop + nav_snapshot + panel `nav_states` in one live client frame. Source order in observe is panel rebuild, then nav rebuild, then host `rebuild_dirty` after observe returns — that can share within a dirty tick, but no test asserts it.
- Real-population overlap, scratch peak, RSS delta, 10 MiB discriminator, or CPU.
- Cross-slot body cache (intentionally absent).
- Families other than Widgets / SideTabs / Loc.

Generic host `--lib` counts would not close the Arc-identity gap; they were not treated as integration proof.

## Checks run this review

Dedicated `CARGO_TARGET_DIR=…/target-snapshot-dedup-proto` (no primary default target, no rustfmt):

| Command | Result |
| --- | --- |
| `cargo test -p api --features snapshot-dedup --test snapshot_dedup --test snapshot_dedup_proto` | 18 + 12 ok (matches freeze log SHA `702a648a95d6baee0db94ad66cfeeceb657334d4e5335c281b896a2d27c50424`) |
| `cargo test -p api --test snapshot --test snapshot_dedup` | 60 ok + 0 tests |
| `cargo check -p host-play --features memory-profile-no-alloc,snapshot-dedup` | ok |
| `cargo check -p host -p host-play -p panel --features snapshot-dedup` | ok |
| `cargo check -p host -p host-play -p panel -p api` | ok |

No source/client/test edits. No live/native/network.

## Residual (non-blocking for this milestone)

1. No end-to-end host/panel Arc-identity assertion through a full client frame.
2. Feature-on quiet counter dual-write takes the registry mutex; do not read that as a CPU saving.
3. Scratch peak remains unmeasured (`null` / `scratch_peak_available: false`).
4. `account_allocations_arcs` `.max(1)` still exists off the diagnostics path.
5. TUI has no extra UI cursor; panel is the third owner.

## Next root step after this approval

Native overlap screen on this frozen source, with real registered-owner diagnostics. Do not treat this review as performance acceptance. Whole-branch Grok 4.6 remains required at campaign end.
