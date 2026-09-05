# Bounded native thieving stun recovery

This is a workload reliability prerequisite for the memory campaign, authorized by the operator after the TUI32 bank walk failed. No accepted memory baseline exists yet.

Server evidence: skill_thieving/scripts/thieving.rs2 emits stunned_thieving one delay before installing %stunned; player/scripts/movecheck.rs2 cancels movement while map_clock < %stunned. The packed spot animation ID is 245. Pickpocket rows specify eight or ten ticks. The exact deadline is not exposed as a confirmed transmitted varp. Native GameSnapshot retains the observed animation onset, keyed by the decoder's spotanim_last_cycle, independently of visual expiry. It is deliberately omitted from serialization and the JS/FB wire.

Traveller waits eleven distinct PLAYER_INFO snapshot ticks from onset (ten plus animation lead), once per FollowRun. An affected pending walk is rearmed after that window, using the retained route and current local collision checks. Global route search is not repeated by recovery. Already-arrived walks are not replayed. A stun observed while follow is held can recover on release. Disconnect remains terminal through existing handling. Subsequent stuns do not extend that run; ordinary hop limits remain in force. Transport approaches can rearm their existing walk before interacting.

This is a bounded inference from the specific server protocol/content, not an exact general-purpose stunned flag. The guard-specific eight-tick stun may wait two extra ticks. The live run must establish that this conservative wait reliably enables bank navigation.

Opt-in memory diagnostics add a separate ring of 32 navigation events per slot, capped at 512 characters each: FindStart/FindEnd with generation, WalkAttempt with tick and refusal, StunDeferred/StunResumed, and FollowEnd with terminal outcome. Sidecar snapshots also include local spot animation and its decoder timestamp. Normal builds do not retain these diagnostic events.

Regression evidence: the duplicate-snapshot deferral test failed before implementation and passed afterward. All 249 nav library tests pass, including pending-walk recovery, one-recovery bound with ordinary failure, guardian hold/arrival, onset refresh/visual expiry, transport approach, and disconnect. API library (5) and host-play library (129) tests pass with memory-profile. Frontend/build/review/live validation pending.

Frontend validation: panel 371 and TUI 87 integration tests pass with memory-profile; joint panel/TUI release build passes. API integration suites also pass (159 integration tests plus 5 library). One-bot actual-TUI pilot diagnostics/20260905T164748Z_tui_n1_active exited 0 after 180 observation seconds and teardown, but had no bank trip or navigation events. It does not prove stun recovery. Review and TUI32 validation remain pending.

## Final review and TUI32 result

Required whole-branch grok-4.6 review APPROVE: grok-4.6-stun-review.txt; completed model/provider receipt grok-4.6-stun-usage.json. Review notes existing script Pause continues polling Traveller; guardian hold suspends it. No pause behavior was changed.

Actual PTY TUI32 run diagnostics/20260905T165834Z_tui_n32_active exited 0. All slots passed initial XP proof, then completed 120-second warmup, 600-second observation and 60-second script teardown. All 585 observation samples had ready=32, active=32 and 32/32 V8 heap coverage. Every slot gained observation XP (2,293–4,165) and completed one food bank trip. No failures. All 96 route requests returned Routed and all 96 follows ended Arrived. There were 132 walk attempts and NO StunDeferred/StunResumed events. Therefore this live run verifies ordinary banking compatibility, not the inferred stun recovery path. A controlled live stun case remains necessary for that specific claim; do not attribute this run's success to the fix.

Observation median resident bytes: 3,796,992,000. Process lifetime peak: 3,900,784,640. Observation V8 used median: 202,069,704 (range 135,783,880–274,851,416); V8 total median: 254,017,536. Snapshot inflight capacity median 0 / sampled max 854,040. TUI tracked GPU bytes 0. At final teardown active=0, V8 used/total=0, snapshot inflight bytes/capacity=0, resident bytes=3,540,156,416, Rust tracked live bytes=3,071,466,508; the 32 client slots remain loaded.

Observation means: client tick 11.9241 ms, script tick 0.15057 ms, UI draw 0.14440 ms, full UI frame 0.23973 ms. These measure their documented domains, not network/server latency. See qualification-summary.json for full coverage/ranges. navigation-events.json captures unique per-slot event strings. provenance-check.json confirms source and binary fingerprints still match launch.

This is one complete TUI32 diagnostic cell, not the repeated panel/TUI baseline or a savings result. The earlier failed run is not a valid latency or memory control. No bytes-per-additional-bot estimate can be inferred from the shorter one-bot pilot. Remaining campaign evidence includes controlled stun recovery, repeats, panel, the other slot counts/workloads and lifecycle soak. No commit, merge or push performed.
