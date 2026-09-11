# Map native observed ticks to BotHost subscribers

Implement the approved bounded design in 04aa-native-tick-subscriber-boundary.md
and its evidence. Use grok46 defaults after brief105 review; root serializes
this before special. Follow AGENTS and docs/execution. Check named campaign
branch. Root7c Duel failures in both revisions are the missing subscriber,
not a missing publisher. The actual design run1300 was Grok4.6.

Own only shim/bot_host.js, shim/execution.js (pump fire only), src/load.rs
(COMPAT_RUNNER fire only), new tests/tick_subscriber.rs, report
04ac-native-tick-subscribers.md and evidence/native-tick-subscribers/.
No host, client, slot, registry, fixture, other adapters or foreign edits.
No LIVE, compiler-cache cleanup, tree/index reset, or foreign runtime clone.

Map addTickListener to a module-local Set and return an unsubscribe callback.
Fire after the posted snapshot/tick and before onStart/loop, or before parked
settle. Do not fire in hold/pause/generation skip/stale tick drain. Do not add
onStop/onPause/onResume hooks, frame/draw listeners, packet attach, timers or
opcodes. Preserve all existing tick/loop/paint/chat ordering. Callback errors
are isolated and logged without marking the script failed; never log a
synthetic 'tick N:' failure. No per-read world copies.

Use the meaningful lifecycle tests specified in 04aa, including parked
observation before settle, pause/hold, unsubscribe/dedup/error isolation,
fresh snapshot visibility and separate isolates. Preserve existing hold,
park, pause, paint and generation tests unchanged. Exact host/client source
export plus only owned overlay and exclusive target; honest functional cache
reuse is allowed. Focused tests and affected strict Clippy, source hashes and
raw logs. Scoped commit must contain ONLY owned paths; populated isolated
index if needed, verify actual parent-to-commit paths before review.
Same card kanban_request_review to profile reviewer, then stop. Root owns
live paired qualification and acceptance, including full combat/reset cycle.
