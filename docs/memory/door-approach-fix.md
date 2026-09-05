# Door recovery approach correction

Operator authorized correcting the open-door approach reversal after the existing live one-tick closer scenario passed with recovery enabled and failed with recovery disabled. Keep recovery and its existing budgets.

In troll_door, directional crossing on the destination level now continues toward the exit without looking up/reopening the door behind the walker. Once within the existing arrival radius it yields to normal settlement without another send. Directionless edges cannot infer crossing. Before crossing, distance-based approach is needed only when the door does not read open; an open door permits the exit walk without countermanding it with an approach.

Two new regressions failed on the old code, sending approach(2,1) instead of exit(5,0). Tests cover a multi-snapshot open-door walk with and without direction, all four crossing directions with a closed door behind, and no extra action at arrival. Existing closer/reopen, offset leaf, interaction refusal, pause, stun and transport tests remain in the suite.

No timeout, script API, route search, or memory ownership redesign in this change. The directional side predicate is the existing door_crossed used for arrival; it is not a full path-history proof. Review this assumption against actual route semantics. An open-state read before approach uses existing edge_loc_open; live leaf matching remains otherwise unchanged.

## Verification and outcome

Both new regressions failed before the correction, then passed. All 251 nav tests, 114 default host-play tests and 130 memory-profile host-play tests passed. Joint release panel/TUI memory-profile build passed. The live nav_door closer scenario passed at tick 88 (57.687 seconds), exercising reopening recovery. It did not reopen behind the walker after crossing.

The captured panel32 run `diagnostics/20260905T191752Z_panel_n32_active` completed exit 0 after 120 seconds warmup, 600 seconds observation and 60 seconds teardown. All 594 observation samples (598.757 seconds between first/last samples) report ready32/active32. Every bot banked once and gained 2,152–3,838 XP during observation. No script or navigation failure occurred. All three milestone PNG/JSON pairs were saved and visually inspected; no failure checkpoint was needed. Pixel timing remains asynchronous as documented in nav-capture-diagnostic.md.

Bot live15b10_27 exercised the exact troublesome door leg: recovery at (2654,3296), door at (2656,3292), exit (2651,3292). Walks at ticks 264–266 all targeted the exit, it reached the exit at 267, and the remaining route arrived at 274. This directly demonstrates forward progress where the previous run reversed until expiry. navigation-events.json contains the deduplicated trace.

Stop left active0, V8 used0, queued snapshot bytes0 and ready32 clients. Median observation RSS was 4,077,060,096 bytes; final retained RSS was 3,834,937,344 bytes with clients loaded. These are diagnostic observations, not savings: capture instrumentation, concurrent review during part of the run and a five-second CPU sample rule out accepted baseline/latency comparisons.

Source and binary hashes matched launch through completion (provenance-check.json). Commits were created afterward without changing those source bytes. CPU leads and the operator-approved deferral of 128-bot measurements are recorded in cpu-follow-up.md.

Whole-branch grok-4.6 returned APPROVE; the usage receipt confirms completed=true, failed=false, model=grok-4.6. See grok-4.6-door-approach-review.txt. No push or merge is implied by this qualification.

