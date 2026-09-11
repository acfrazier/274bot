# Brimhaven qualified actor fixture

This bounded fixture correction prepares the existing `BrimhavenAgility` scenario with the same ordinary trained Agility 52 actor used by the Wilderness course. It changes only scenario preparation, catalog baseline qualification, focused regressions, and this evidence. It does not change the imported controller, native runtime, client, navigation, settings, support matrix, ledger, clocks, or post-Start witnesses.

## Pre-Start preparation and acknowledgement

Before catalog Start, the scenario now sends `advancestat agility 52` alongside the preserved Coins 995 x1000 and Lobster 379 x10 seed, then teleports to the unchanged entrance `(2809, 3194, 0)`. The existing dialog janitor drains every resulting level-up chat before the remaining baseline checks and Start.

The catalog observation records both native `base` and `effective` values from each snapshot stat row. The Brimhaven baseline requires Agility base 52 and effective 52, so an actor with base 51 or an effective level drained to 51 fails closed. The baseline still requires the exact entrance vicinity, enough fee coins, ten Lobsters, no ticket 2996, and an unpaid arena varp 309. Its failure diagnostic now names the actual entrance `(2809,3194,0)`.

## Preserved post-Start contract

All existing ordered witnesses remain unchanged: pay the 200-Coin fee; observe the paid bit; Climb-Down to platform 24; gain real obstacle XP before the first Tag; observe a fresh `tag the next` line and tagged varp; prove the first Tag grants no ticket; obtain the first later ticket; then observe independently fresh obstacle XP after that ticket. No ticket, progress varp, arena progress, or post-Start stat is seeded.

`stealRestock=false`, `bankAtTickets=1000`, `SCRIPT_GOLD_DEADLINE=180s`, and `SCRIPT_GOLD_WATCH_TICKS=150` are unchanged. The prior Agility 1 failures remain failures; this fixture correction is not LIVE acceptance and does not decide the climb-animation freshness question.

## Verification and limits

Implementation commits are `a6c34270cb77668bc0e506722df2252f38c434d8` and `9429357d1124a9a3451c33e2fae1725fd5296be0`, with the owned path diff based on `926e9bcee7d988ac161350734eb5fb4ba825b42d`. The final exact committed export is `.superpowers/review-exports/t_83737de2-r2`, with client gitlink `aef3952d1cd7bb3b93d39c497f0f476b68021c59` and immutable selected-catalog inputs linked from the campaign checkout.

From that export, 104 scenario tests passed; the catalog harness passed 41 tests with its one LIVE test ignored; strict affected Clippy, affected-file rustfmt, the owned diff check, and both frozen catalog-source identity checks passed. No LIVE process was launched. Raw output and a machine-readable receipt are under `docs/compat/evidence/brimhaven-qualified-actor/`. Root retains all catalog x client-revision LIVE rechecks and acceptance.
