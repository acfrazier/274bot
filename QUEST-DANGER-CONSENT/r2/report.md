# QUEST-DANGER-CONSENT-R2 report

## Result

This report covers ten requested items: H1, H2, M1–M7, and Low. It records observed results and limits.

The Rust test, Clippy, and format matrix passed on commit `5c3b99e28`. Nineteen of 23 live cells passed. Four live cells failed.

The TUI capture shows the zone list. No headed panel capture shows it.

The red-before/green-after check remains incomplete for some items. M1 and Low pass on the old revision. Their behavior or measured size did not change. M3 is a documentation check. The M5–M7 baseline attempts failed to compile because the tested APIs did not exist. M4 is a live-coverage check, not a unit test.

Evidence files are under `/Volumes/dev-scratch/274bot-evidence/QUEST-DANGER-CONSENT/r2/` unless a citation starts with `../`.

## Findings

### H1 — A declared crossing needs a survival control

- **Fix:** `crates/script/src/native/actions.rs:9-38` arms the food guard and allows food for each non-empty declared crossing. `CHANGELOG.md:73-76` states this policy.
- **Before:** `native::actions::tests::plain_walk_under_stage_grant_arms_food_guard` failed. Zero tests passed and one failed. See `baseline/baseline-tests-aubaine.log:163-176`.
- **After:** The test passed in the full suite on `5c3b99e28`. See `validation-chardonnay-final-5c3b99e28.log:753`. `baseline-tests.log:8-10` also records a post-fix pass. That log does not identify the tested commit.

### H2 — Desert Rescue’s resumed captain walk needs the same grant

- **Fix:** `crates/script/paths/289/desertrescue.json:2036-2054` gives `walk-to-captain-again` the same three Ugthanki ids as `walk-to-captain`.
- **Before:** `desert_captain_resume_step_declares_the_same_pack_zones` failed. Zero tests passed and one failed. See `baseline/baseline-tests-aubaine.log:452-465`.
- **After:** The test passed in the full suite. See `validation-chardonnay-final-5c3b99e28.log:12893-12895`.

### M1 — Empty Quester scope must not inherit global danger grants

- **Fix:** `CHANGELOG.md:75-76` states this policy. `crates/script/src/native/actions.rs:32-38` applies the route policy.
- **Before:** `admission::tests::empty_quester_stage_forbid_avoids_a_globally_allowed_undeclared_zone` passed on the baseline. One test passed. See `baseline/baseline-m1-aubaine.log:9-12`. The test supplies `WalkBit::Forbid`. It is not a red-before regression test.
- **After:** The test passed in the full suite. See `validation-chardonnay-final-5c3b99e28.log:10892`. The behavior already existed on the baseline. The change states the policy.

### M2 — Remove the misleading danger setting

- **Fix:** `crates/script/src/quester/card.rs:53-55,801-834` hides the old override and keeps saved data. `crates/host-play/tests/quester_path_live.rs:232-240` no longer adds the old live setting. `CHANGELOG.md:70-74` documents the change.
- **Before:** `quester::card::walk_permission_settings_tests::walk_permission_settings_keep_teleports_and_legacy_danger_data` failed because the Quester card still showed the override. See `baseline/baseline-m2-aubaine.log:7-19`.
- **After:** The test passed in the full suite. See `validation-chardonnay-final-5c3b99e28.log:1040`.

### M3 — Document the user-visible change

- **Fix:** `CHANGELOG.md:64-77` adds an Unreleased Quester entry. It describes the disclosure, removed setting, named grants, and food guard.
- **Before:** The review says the earlier commits did not change the changelog. See `../review-opus.md:52-53`. No Cargo test covers this documentation finding.
- **After:** The required entry appears at `CHANGELOG.md:64-77`. This is a document check, not a behavioral test.

### M4 — Live proof must cover the changed paths

- **Check:** The round-2 brief requires one run of each mapped live cell. The result is incomplete.
- **Before:** The earlier evidence covers one run of one cell. See `../live-default.log:212,341` and `../review-opus.md:55-56`.
- **After:** `live-cells-chardonnay-once.log` records 19 of 23 passing cells. Four cells failed:
  - `panel.headed-quester-zone-list`: world speed timed out after 30 seconds. The panel refused to restore it.
  - `panel.manual-start-live`: the copied R289 cache was missing.
  - `panel.state-gap-stop-live`: the copied R289 cache was missing.
  - `tui.live-jive-kq`: the `/crc` connection failed.

### M5 — Bulk Start must disclose each Quester queue

- **Fix:** `crates/frontend-core/src/fleet_plan.rs:94-120,275-357` and `crates/frontend-core/src/session/fleet_plan.rs:275-450` freeze each bot’s disclosure and require confirmation. `crates/frontend-core/src/session/scripts.rs:677-687` refuses unplanned bulk starts and skips bots with changed queues.
- **Before:** The baseline attempt failed to compile because the disclosure APIs did not exist. It did not reach an assertion. See `baseline/baseline-m5-aubaine.log:11-77`. This is not a failing test result.
- **After:** `quester_fleet_start_discloses_each_queue_and_skips_changed_consent` passed. `unplanned_bulk_start_refuses_quester_without_a_disclosure_plan` passed. See `validation-chardonnay-final-5c3b99e28.log:6933-6945`.

### M6 — Panel and TUI must share disclosure, including after an error

- **Fix:** `crates/frontend-core/src/scripts/prestart_zones.rs` owns the shared view. The panel uses `crates/panel/src/app.rs:2821-2845`. The TUI uses `crates/tui/src/script_shape.rs:471-496,663-670`.
- **Before:** The baseline attempt failed to compile because the disclosure APIs did not exist. It did not reach an assertion. See `baseline/baseline-m6-tui-aubaine-2.log:81-105`. This is not a failing test result.
- **After:** `panel_driver_shows_quester_zone_disclosure_before_start` passed. See `validation-chardonnay-final-5c3b99e28.log:8236`. `arthur_zone_scope_renders_all_eight_names_and_reasons_at_common_sizes` passed and rendered the disclosure in Error state. See line 10099 of the same log.

### M7 — Avoid rebuilding the full settings bag each frame

- **Fix:** `crates/frontend-core/src/scripts/mod.rs:989-1030` and `crates/frontend-core/src/scripts/prestart_zones.rs:141-222` cache durable settings by profile generation. The code rebuilds the view only when the profile generation or registry revision changes. The common refresh path reads the revision without allocation.
- **Before:** The baseline attempt failed to compile because the cache APIs did not exist. It did not reach an assertion. See `baseline/baseline-m7-aubaine.log:3-107`. This is not a failing test result.
- **After:** `scripts::tests::warm_quester_script_section_shares_its_zone_text_without_allocating` passed in the full suite. See `validation-chardonnay-final-5c3b99e28.log:7262` and `7942`.

### Low — Pin the Linux SlotScript size exactly (L1)

- **Fix:** `crates/script/tests/surface_facts_alloc.rs:326-334` asserts that `SlotScript` is exactly 1544 bytes.
- **Before:** `slot_size_matches_the_measured_linux_parent_layout` passed on the baseline because the size was already 1544 bytes. See `baseline-chardonnay-tests.log:62-70`. The finding was the loose size limit, not a runtime size change.
- **After:** The exact size assertion passed in the full suite. See `validation-chardonnay-final-5c3b99e28.log:4516`. No red-before result is possible because the measured size did not change. The macOS aarch64 size remains unmeasured.

The M5–M7 baseline compiler failures do not provide behavioral red results. The M1 and Low tests also pass on the baseline. M3 and M4 have no Cargo regression test. The evidence does not show a valid red-before/green-after test for every item.

## Expectation update in `80d5ac948`

`crates/script/src/quester/families/combat_tests.rs` changed both combat-walk assertions from `aborted && !protect` to `cross || (aborted && !protect)`.

This change follows `crates/script/src/native/actions.rs:35-38`. When a request declares a crossing, the code sets `food_guard = true` and `allow.food = true`. The old assertion term still covers an unprotected abort walk. The new assertion covers either case.

The updated test message says the same thing at `combat_tests.rs:424-425`.

## Validation

`validation-chardonnay-final-5c3b99e28.log` records exit 0 for the final Rust validation matrix.

- **Tests:** `script` with default, no-default, and all features. `frontend-core`, `panel`, and `tui` with default and no-default features. `host-play` with default, no-default, and `live-harness,test-support` features.
- **Clippy:** `-D warnings` in the listed default, no-default, TUI no-default, and host-play live-feature modes.
- **Format:** `cargo fmt --all -- --check`.

The log records `MATRIX_EXIT=0` at line 17807. The live-cell matrix remains separate and has four failures. The interrupted Aubaine log is not a green result.

The full matrix and after-results above are from descendant `5c3b99e28`. The matrix did not run at the exact requested revision `80d5ac948`. The post-fix H1 receipt does not identify its commit. This report does not claim a per-item green run at exactly `80d5ac948`.

## Live UI evidence and limits

The TUI capture test wrote five pages at 120x40 and fourteen pages at 80x24. The first and last pages show the Start zone scope, food-guard notice, and Start control. They also show Arthur stage-3 and stage-4 zones with ids and guide reasons.

The captures are under `live-20261030-once/tui-app-headless/run-1/tui-zone-list/QUEST-DANGER-CONSENT/`.

No headed panel image shows the zone list. The live headed cell failed before it produced an image. `live-cells-chardonnay-once.log` records the world-speed timeout and refused restore.

The retry log `live-headed-retry.log` records a 60-second Quester scenario timeout. Its screenshot `live-headed-retry/quester_path.png` shows the bot in the desert, not the zone list.

The null-raster attempts also failed to produce a usable image:

- Attempt 1 shows the login queue and a collapsed Script section. See `live-headed-nullraster.log`.
- Attempt 2 did not select a bot. Automation targeted the toolbar’s Script preferences. See `live-headed-nullraster-attempt2.log`.
- Attempt 3 produced black root screenshots and did not locate the Script section. See `live-headed-nullraster-attempt3.log`.

The headed zone-list check remains unverified.
