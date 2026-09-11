# Wildy and Brimhaven full core fixtures

This bounded fixture wave keeps the frozen `WildyAgility` and
`BrimhavenAgility` source unchanged. It adds the two catalog-backed scenarios,
their independent ordered witnesses, and one narrow scenario-proof primitive
for work that must be newer than an earlier XP arm. It does not change gameplay,
the client, the engine, navigation, catalog source, ledger, or LIVE behavior.

## Identities and setup

- `wildy_agility`: `WildyAgility`, Agility 52, exact Lobster 379 x5,
  `acquireFoodAtStart=false`, `minFood=0`, and the south-ridge start
  `(2998,3916,0)`. The scenario starts with no course progress. Its ordered
  witness is ridge XP and north-side arrival, pipe, ropeswing, stepping stone,
  log, rocks plus the selected lap bonus, and a second pipe. Acceptance requires
  at least 598 Agility XP above Start and the selected second-pipe destination.
- `brimhaven_agility`: `BrimhavenAgility`, exact Coins 995 x1000, Lobster 379
  x10, no ticket 2996, `stealRestock=false`, `bankAtTickets=1000`, and surface
  entrance `(2809,3194,0)`. Its ordered witness is the 200-coin fee, arena paid
  varp 309, ladder landing `(2805,9590,3)`, obstacle XP before the first Tag,
  the fresh "tag the next" message with the tagged bits and still no ticket, a
  later ticket 2996, and independently fresh obstacle XP after that ticket.

The Brimhaven scenario uses `Proof::FreshStatXpGain` only for the post-ticket
arm. The runner captures that baseline when this exact step begins, clears it at
step and off-session boundaries, retries a missing stat row without fabricating
a value, and retains it only long enough for the matching terminal proof.
Existing `Proof::StatXpGain` remains cumulative from its first watch for that
skill, preserving all older scenarios.

## Frozen source identities

The two checked-in catalog snapshots are
`100adccc037d9f6898080e1cad58fcfc43364775` and
`8e7d965be2071d6ec65c3265e12af797082d720a`. Both carry identical script bytes:

- `WildyAgility.ts`: `71170d4bfe844161fe87d36e411ded9e99424ebadac875e11db9f781043c405f`
- `WildyAgilityLogic.ts`: `8e38f53a01cea150f1e9fa67ece422ea4f642c748f52326c3a0512125bc77942`
- `BrimhavenAgility.ts`: `771daff07bd4b3d6f2826ab1300d4fd66bcbae0f9d7a76e4a2ad07a4d050e859`
- `BrimhavenAgilityLogic.ts`: `fedf5f8e05fd43642efb0370352e71e5feebf258a784bc503a2e145c33b46d4d`

Selected Wildy destination/XP facts and Brimhaven ids/varp/platform facts are
documented in `05v-remaining-agility-design.md`; the catalog witness uses the
same selected facts independently of scenario step structure.

## Named false positives

Wildy rejects under-level or under-food setup, ridge-only, a lap without the
second pipe, 571 XP without further work, a queued second-pipe destination
without the ordered chain, a high-z obstacle pit, the wrong side of a selected
obstacle, and the wolf-pit failure chat.

Brimhaven rejects a pre-paid or seeded-ticket baseline, an underfunded setup,
fee-only, arena entry without a real movement/XP observation, first Tag without
a later ticket, ticket without subsequent platform/XP work, a stale first-Tag
message, and the wrong-hint Tag failure message. The scenario proof additionally
rejects prior XP, unchanged XP, and a missing fresh baseline before accepting a
later observed gain.

## Gold clock and limits

`SCRIPT_GOLD_DEADLINE` remains 180 seconds and
`SCRIPT_GOLD_WATCH_TICKS` remains 150. The full circuits are authored even if a
future LIVE cell cannot finish inside those bounds; a partial phase remains a
failed diagnostic, not acceptance. No LIVE process was launched for this task.
Root still owns the two catalogs by two client revisions by two core scenarios
acceptance matrix and any native/TUI evidence.

## Verification

The exact base/overlay hashes, client revision, commands, results, and target
path are recorded under
`docs/compat/evidence/agility-full-core-fixtures/`. Scenario tests passed 102;
the catalog harness passed 38 with its one LIVE test ignored; the frozen-source
ledger check passed; strict Clippy, rustfmt check, and `git diff --check` passed.
