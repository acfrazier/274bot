# Design native tick-subscriber mapping for Duel Arena

Use grok46 defaults. Bounded read-only design; own ONLY
 docs/compat/04aa-native-tick-subscriber-boundary.md and unique
 evidence/native-tick-subscriber-boundary/. No source edits, compilation, LIVE,
cache cleanup or routine review hop. Root commits report/evidence.

Root7c exactsource/client9d first actual Duel Arena paired runs on274/289 oldcat
FAIL not impl: BotHost.addTickListener during onStart. Logs at
 docs/compat/evidence/paired-catalog-fixtures/live/*-duel-100adccc-7c2eef6e.log.
Both scripts are frozen. ImportedDuel's observeFightState callback is required
for the ordinarycombat loop, not an ancillary whale dependency to fake.

Compare frozen runtime/BotHost.ts addTickListener() contract (PLAYER_INFO
postprocess callback, Set registration returning unsubscribe, perlistener
exception handling) with our existing native observed-game-tick publication,
script load.rs IsolateCmd::Tick, __rs_tick prelude, execution.js parkedwaitpump,
BotHost.tickCount, nativeScriptRunner/eventshims, and pause/resume/stop/reload
semantics. Find minimum mapping onto EXISTING native tick edge aftersnapshot;
no foreignattach/runtime/networklistener/worldimport and no invented tick-end
opcode. Native remains owner of tick timing/lifecycle. Noframepoll/timer nor
synthetic loops for skippedticks. Host existinghold/pause refusal and order
must not be weakened to resemble foreignruntime. Consider normal and parked
loops, onStart failure, stopunsubscribe, duplicate callbacks, callback errors,
slotisolation and stale/reloaded tick generation. Callback glue alone belongs
inJS; gameplay scheduling/transport staysRust.

Find an existing callback/event registration seam if possible before proposing
new lifecycle changes. Identify exact ownedfiles and meaningfulfocusedtests.
Do not change async loop continuation/paint/lifecycle ordering for scripts
without subscribers. Clarify any unsupported event flavors honestly.

Current sharedruntime queue: cake97 -> special -> teleport -> Solshop -> MakeX
-> fire. reader101 owns client_adapter.js; UI94 ownsregistry/UI; paired103 owns
pairedtest preparation; no one else should touch those. Root decides serialized
implementation insertion after design, not worker. Complete bounded proposal
with actualsourceidentities and falsifiers; source review is not LIVE acceptance.
