# Selective product-fix extraction

Prepared September 9, 2026. This is extraction under the approved selective merge
plan, not resumption of the memory campaign.

Code-only commit: `c5e0f03` on `codex/selective-product`, based on `54cfcf8`.
The orchestrator integrated it as `d6be8af`. No remote operation or client gitlink
commit was performed by this task. The test checkout uses the orchestrator's
selected client, most recently `d5f26e3`.

## Extracted behavior and source mapping

- Selected `a52b12c`: radius-aware WalkNear wire encoding/dispatch; one worker per
  slot, identity token and generation checks, latest pending request, failed
  search retaining the old route and clearing the failed request latch. Exact
  walks retain their existing route/session guard. Native optional TravelEvent
  callbacks remain; no capture sink, memory_diagnostics, or nav_capture module
  was selected.
- Selected `a52b12c`: spot-animation onset fields and bounded Traveller stun
  recovery; Rust food lookup and withdrawal sequencing; thin JS adapters;
  loadout command delivery before ticks; explicit fixture loadouts and ordinary
  default-store behavior. Remaining unsupported errors and helper timeouts are
  retained.
- `72804cd`: maintain forward progress past an opened/crossed door.
- `b9f40fe` + `7992f68`: door edges land on adjacent standable tiles; blocked
  scenery is not skipped; existing web traversal stays separate. The temporary
  campaign-specific check_bank_return example was not promoted; the production
  fix and focused regression tests were.
- `1211d0a`: Withdraw X waits for published inventory after submitting the answer,
  preserving the 4000 ms bound and preventing premature success.
- `94adac4` plus comments from `5e0f3ec`: existing one-tile movement packet after
  Open, with scene/level/cardinal-adjacency checks; affected API/e2e/scenario
  command observations accept this existing walk packet. No new opcode.

The extracted navigation representation remains dense. No snapshot deduplication,
borrowed fingerprint, tiled world, profiling framework, or campaign controller
was introduced. Harness accounting and sustained-scenario preparation were
coordinated with the separate harness extraction.

## Bounded validation

Commands ran natively on this macOS checkout, with the shared campaign Cargo
build directory. No live sessions were run by this task.

- `cargo test -p api -p nav -p host-play -p script --features script/load --lib`:
  API 5, host-play 113, nav 254, script 37 passed (409 total).
- API library and all API integration targets passed in the combined API/script
  test command (166 tests).
- `cargo test -p script --features load --tests`: final run passed 287 tests;
  two existing opt-in tests remained ignored. This includes all 144 Load isolate
  tests and the delayed-publication, adjacent-bank, host-loadout/food, wire,
  fail-closed and remaining-stub regressions.
- `git diff --check` passed before the code commit.

The first combined integration run exposed the documented stale expectation in
`isolate_hold_from_blob_freezes_loop_but_still_paints` (actual first-tick paint
count 2, expected 1). The exact test-only correction from `a52b12c`, original
hunk `@@ -1127,7 +1127,8`, was included and the full script test suite rerun.
Production tick execution and paint forwarding were not changed by this
extraction: both paint invocations already exist on main. This correction pins
that existing cadence; it does not wait for another tick or alter the runner.

Logs: `/tmp/selective-product-tests.log`,
`/tmp/selective-product-integration-tests.log`,
`/tmp/selective-product-isolate-retest.log`, and
`/tmp/selective-product-script-tests.log`.

## Limits for final acceptance

The orchestrator still owns combined-build checks, final independent review,
live bank/contested-door proof, and available native platform validation. Stun
recovery has unit coverage here but no newly established live trigger/recovery
proof; the 245/eleven-tick inference is revision/content-specific. Existing v8
navigation packs must be rebuilt to receive the door-edge fix. No memory or
CPU saving is claimed from this extraction validation.
