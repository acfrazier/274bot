# Posted settings Tile shape

## Candidate

Task `t_701d17f4` corrects the settings shape and connects Tile distance to native geometry. The frozen proof export is host `bc0f05c6867493559fd321fd3708baf3af2ab4a6` with client `56d80272bcbda3eb1e22db096c1c5e21d3497de4`. Root verified all six frozen overlay file hashes. Five staged source/test files match that overlay exactly; load.rs also contains the separately implemented PeriodicBank registration/lifecycle hooks. This composed candidate requires independent cold checks in its same-card review.

The preserved failed LIVE artifacts remain under `evidence/catalog-harness/live/r274-door-opener-100adccc-50f2be8a.*`. This task did not launch LIVE, edit the frozen catalog, or alter scenario, navigation, policy, client, UI, ledger, or fixture code.

## Source findings

Both selected frozen catalogs have the same `SettingsBag.tile`: it returns the stored value only when it is a `Tile`. `DoorOpener` imports that `Tile`, posts `stand` through `this.settings.tile`, and then calls `stand.distanceTo(...)`. The Rust settings store already normalizes tile defaults and overrides into `{x,z,level}` while preserving fallback and list/scalar posting. The compatibility loss was later: PRELUDE returned that JSON object directly. The explicit shim `SettingsBag` also lacked `tile`, while enabled `WalkToBot` constructs that bag and calls the member.

## Correction

`tile.js` now owns one `tileFromPosted` adapter. It accepts native 14-bit integer world coordinates and level 0 through 3, preserves an existing valid imported `Tile`, and otherwise constructs the same `Tile` class. Missing, fractional or out-of-world values return `null` rather than creating a partial or silently truncated coordinate object.

PRELUDE uses that registered adapter for `this.settings.tile` and returns the caller's fallback unchanged for missing/invalid data. The explicit shim `SettingsBag.tile` imports and uses the same adapter. Existing `Tile` exports and methods, posted coordinates and levels, scalar/list behavior, and fallback identity remain intact.

`Tile.distanceTo` now calls the synchronous Rust `__rs2b0t_tile_distance` adapter. The adapter validates both Tile-like values before constructing `api::WorldTile`s and delegates same-plane distance to the existing `api::query::chebyshev_to` primitive. That primitive's policy is unchanged: a level mismatch remains `i32::MAX`. For observable Tile compatibility only, the adapter asks the same primitive for the planar distance and maps a level mismatch to the existing JavaScript representation `1_000_000 + planar distance`. The 14-bit coordinate bound makes both calculations overflow-safe; invalid values raise an explicit isolate error.

## Isolate regression proof

The real `LoadIsolate` regression posts `{x:3208,z:3212,level:2}` through `post_settings_bag`. Inside the loaded module it verifies imported-class identity; same-plane, cross-plane and direct native-callback distance; overflow-safe invalid-coordinate rejection; `translate`; `equals`; nonzero level; exact missing, malformed, fractional and out-of-world fallback identity; explicit `SettingsBag.tile`; and unchanged list/number/boolean/string accessors.

The proof used a new empty target at:

`/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision/.superpowers/task-exports/t_701d17f4-bc0f05c6/target-t_701d17f4-native`

Results:

- RED on the frozen source plus test overlay: exit 101; the loop aborted before producing the probe because the posted object had no Tile methods.
- Native-distance RED on the shape-only overlay: exit 101; the isolate had no registered Rust callback.
- Focused GREEN: 1 passed, 0 failed.
- Full `settings_bag` integration test: 5 passed, 0 failed.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --locked -p script --test settings_bag --no-deps -- -D warnings`: passed.

Raw logs and the host/client/overlay hash manifest are in `docs/compat/evidence/settings-tile-shape/`.

## Root source reconciliation

PeriodicBank commit 41cf8eac accidentally captured this task's 68 lines of native
Tile helpers/registration without the API visibility and shim changes. Root
separated those hunks from committed PeriodicBank source at 922dac24 using an
isolated Git index, leaving the working source and ordinary index untouched.
They are committed here with their complete Tile dependency. No history was
rewritten and no Tile implementation was discarded.

Two GREEN logs were replaced by a final rerun after the original evidence
manifest was written. The original expected hashes remain preserved in that
manifest; root records the actual current hashes separately. The rerun logs
show the same frozen export/target and all five tests passing, but neither
those logs nor the earlier overlay prove the later PeriodicBank composition.
The required same-card reviewer must run the composed commit in a fresh empty
target before root releases LIVE.
