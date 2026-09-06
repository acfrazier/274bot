# Bank publication and contested-door fixes — 2026-09-06

Two separate defects were diagnosed and corrected on `codex/memory-diagnostics`.
These are functional stability changes, not accepted memory/CPU savings.

## Bank return: premature Withdraw X success

Commit `1211d0a` changes `Bank.withdrawX` to wait for inventory publication.
Previously it returned true one posted tick after queuing AnswerCount. If the
inventory update arrived later, Rust's withdrawal sequencer saw no progress and
sent Withdraw-10 in addition to the pending 19. The backpack filled with 28 food.
Thiever explicitly excludes a full inventory from pickpocket validation, so it
idled at full HP with its last status still saying “returning from the bank”.
The previous completion-delivery hypothesis was not the root cause.

The new completion condition observes the expected count (capped by the posted
bank availability), or partial progress with a full backpack, within 4000ms.
The withdrawal sequencing remains owned by Rust. A delayed-publication isolate
regression failed before the change and passes after it. The older test that
asserted success immediately after AnswerCount now publishes inventory first.
The complete `script` suite passed with feature `load`. Grok-4.5 `reviewer`
approved this commit; its receipt is `grok-4.5-bank-wait-review.txt`.

Live proof: `diagnostics/20260906T020818Z_panel_n32_active`, System allocator,
32 active Thiever bots, one renderer, diagnostics/debug/captures enabled,
one-second warmup, 300-second observation and 60-second teardown. Exit 0.
All 32 restocked to exactly 22 lobsters and completed their bank return. Each
made at least 10 further steals after its return; observation gains were 16–51.
No bot ended in a banking status. Stop left active scripts, live V8 isolates,
V8 used bytes and in-flight snapshot bytes all zero. Reduced per-slot evidence
is `bank-wait-live-evidence.json`.

The immutable bank-test binary is
`diagnostics/bank-wait-build/panel-play-system`, SHA-256
`e86dee3f7f3011fb36f24092c1a9db52d9fda9f56962b50d81a8bdd1997ff350`.
It was built before committing and before the door-probe change. Its launcher
manifest records source/diff hashes. It used the corrected pack hash
`2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30`.

## Door closer: redundant Open wastes the crossing opportunity

Commit `94adac4` adds one adjacent crossing probe after Open in Traveller's
existing door fallback. The cheap initial hop, fallback budget and arrival
predicate remain in place. Only a player on `edge.at`, with a same-level
cardinal adjacent destination in the edge's crossing direction, qualifies.
The next delivered tick sends the existing MOVE_GAMECLICK packet with one
waypoint, avoiding stale client collision for this pending door operation.
The server still validates collision; a submitted step never counts as arrival.
No JavaScript API, client source or wire opcode was added.

The unmodified closer failed with both old and corrected packs and with reversed
login order; it also passed once with additional position logging. That pass was
not accepted as a fix. A bounded server trace then showed the actual failure:

- Tick 1730531: walker opens the door successfully.
- Tick 1730532: a duplicate Open's approach packet targets the current tile;
  the door stays open but the walker makes no progress.
- Tick 1730533: the real crossing walk arrives, but the closer processes Close
  before the walker's movement and collision blocks it.

With the fix, tick 1731193 opens the door and 1731194 moves the walker from
(2816,3438) to (2816,3439) through the still-open doorway. Both corrected live
runs passed the original exact arrival predicate: normal login order tick 85,
56.2s; reversed login order tick 84, 58.0s. The closer still reacts every player
update; no predicate, timeout, server rule or closing frequency was weakened.
Reduced before/after server records are `door-probe-server-evidence.json`.

Server tracing used temporary wrappers around input decoding and interaction
processing, filtering this Catherby area and capped at 2000 records. Wrappers
were restored and trace globals removed; the loopback inspector was closed.
The server process was not restarted, and server source files were not edited.
Raw traces, diagnostic controls and the bounded trace installer are retained in
`diagnostics/door-probe-build/`. These live runs and the bank fleet overlapped
review/testing/tracing work and must not be used for CPU comparisons.

Regression coverage includes stale-closed snapshots after Open, exact movement
packet bytes compared with the real client's one-tile walk, cardinal distance,
plane and scene refusal gates, and arrival only after a position update.
The API/nav suites passed (254 nav library tests plus three pack-tool tests).
Host, host-play and scenario suites passed; the combined release panel builds.
No client code changed, so client integration tests were not rerun.

The memory campaign and its final whole-branch grok-4.6 review remain open.
Commit reviews here cover these bounded fixes only. A fresh scheduling run can
now assess the stable scenario without debug tracing and screenshots.

## Review, cleanup and active artifacts

Grok-4.5 `reviewer` approved the door commit; receipt:
`grok-4.5-door-probe-review.txt`. The reversed-login control also completed
successfully after that review was launched. The separate host-play suite with
`memory-profile` passed, including its measurement-specific tests.

The default `~/.274bot/274bot.navpack` was atomically refreshed to the verified
candidate after review and live proof. Its flags sidecar is byte-identical to
the candidate and was left in place. The old pack is backed up at
`~/.274bot/backups/before-door-fix-20260906T022611Z/274bot.navpack`;
`door-pack-install.json` records both hashes. The bank-return pack checker also
passes using this default path. Future measurements must still record the pack
and binary hashes. The combined release panel is saved as
`diagnostics/door-probe-build/panel-play-system`.

No merges or pushes were performed. The final report commit also corrects stale
fallback comments that previously described sending Open and walk together.
