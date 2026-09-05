# Panel 32: one drawing, 31 simulation clients

Operator request: repeat the completed TUI32 active cell using actual panel-play with one drawing client. Keep thieving level 50 and all other sustain-fixture inputs unchanged to isolate frontend/rendering costs. A lower-level run is a separate stress experiment and must not be mixed with this comparison.

Prior panel benchmark code enabled all renderers and rotated focus. The new opt-in launcher --single-renderer (panel only) sets BOT_MEMORY_SINGLE_RENDERER=1. Run retains that setting, pins focus to slot zero, and applies only-render-selected plus per-rail renderer Off. Default all-render benchmark behavior remains. Fixed focus avoids growing a renderer on each bot through rotation. The focused Game seat uses its existing focused cadence. No normal application defaults or persisted operator settings change.

Metadata records single_renderer and render_policy. Diagnostic snapshots now record actual client.draw for every slot, so one drawing and 31 non-drawing clients can be checked throughout observation. The focused bot remains one of the 32 active scripts.

Tests cover 32 slots, the single selected renderer, rail override, and preservation of the existing all-render benchmark mode. Full-branch Grok review and live measurement pending.

## Validation and result

Host-play tests (129 library plus non-live integration tests) and panel tests (372) pass. Joint release panel/TUI build passes. Required whole-branch grok-4.6 review APPROVE in grok-4.6-panel-one-review.txt; completed xai-oauth receipt in grok-4.6-panel-one-usage.json. No commit, merge, or push.

Pilot diagnostics/20260905T174618Z_panel_n1_active exited 0, one drawing client, 18,292,776 tracked GPU bytes and V8 zero after Stop.

Actual panel32 diagnostics/20260905T175116Z_panel_n32_active exited 1 after approximately 351 seconds. All 32 passed initial XP proof and completed warmup; only 74 observation samples spanning 73.7567 seconds were collected. All those samples had ready=32 and active=32; all 73 diagnostic records within their elapsed bounds had exactly one drawing and 31 non-drawing clients, no unavailable draw states. Focus remained slot zero. All 32 banked once, but live1418f_12 could not return to the pickpocket spot and stopped at script tick 485 after its 180-second walk timeout.

Return-route evidence: FindStart generation3 from (2655,3286,0) toward (2661,3306,0), radius2, then Routed. First walk at tick186 aimed at (2656,3291,0). The bot moved around the bank, later oscillated between (2655,3293,0) and (2653,3294,0), and native follow ended Stalled/Expired at tick323, aiming at (2651,3292,0), tries1. The script continued waiting until timeout. No StunDeferred or StunResumed event anywhere in this run. There were 96 route requests, 96 route results, and 96 follow terminals including this failure. This is a distinct return-route lead; the exact transport/collision cause is not established by the existing walk-only event stream. Do not infer GPU or memory pressure as its cause.

Partial observation median RSS 4,131,135,488 bytes; GPU logical buffer bytes 1,379,440 + texture bytes 17,243,992 = 18,623,432. These are separate memory domains. This failed, short observation is NOT an accepted panel baseline or a valid paired frontend-overhead measurement against the completed TUI32 run. No normal teardown samples were collected. Full artifacts: qualification-summary.json, failure-detail.json, navigation-events.json, behavior-checks.json and provenance-check.json. Source and binary fingerprints matched launch.

## Visual evidence

Operator F12 triggered five built-in GPU readbacks during warmup (17:54:31–17:54:55 UTC). The final screenshot was inspected: focused slot live1418f_0 shows the 3D scene, minimap, inventory and Thiever overlay; only-render-selected is checked. render-evidence.png is a preserved copy with screenshot-provenance.json listing original paths under ~/.274bot/smoke. This is focused rendering evidence, not a screenshot of failing slot12. MacOS external screenshot permission was unavailable, but the panel's own capture works. No restart was needed.

## Follow-ups identified during the run

- Controlled return-route reproduction before lowering thieving or blindly repeating the memory cell. Capture the active leg kind/from/to/loc identity and its interact outcomes alongside current walk outcomes, so door/transport handling can be distinguished from ordinary movement. Keep the native stun regression tests and separate controlled stun proof outstanding.
- Embed screenshot checkpoints in the diagnostic scenario: scene-ready, bank arrival, return-route start, terminal failure. Use the scenario sink with the actual matching slot snapshot, tick, route generation and events; do not use manual F12's default-empty snapshot as state evidence. Capture the bot under test. Wait for failure-shot readback/write before exit. Keep captures in a diagnostic run or outside timed steady-state observation and record their timings. The memory runner currently clears terminal_shot and does not wire its seed runner to the panel shot sink; this follow-up is planned, not implemented.
- Login-order observation: operator saw focused login delayed, likely during fixture relog but unsure. Initial sampled scene2: bot0 at2.066s, next bots3.065s. Login-all prefers focus in the FIFO, but the fixture later performs Relog through ordinary queue requests; preference is not persistent after grant. A permit does not guarantee completion ordering. Also batch preparation starts auto-login-enabled slots before applying final focus/prefer, a race worth testing independently. Do not present this as a proven initial-login failure. Add login-round/permit/start/ready events to a focused regression before changing ordering.
- Headless retained memory: host drops renderer/mailbox and overlay meshes when draw is off; core world documents render-side lazy model resolution. Still-render-specific world allocations include occlusion_cycle (4x105x105 i32 =176,400 requested bytes for a104x104 scene) and occluder tables. Measure these and other sim ownership rather than assuming all retained RSS is 3D. This is a concrete allocation candidate, not an accepted optimization.
- TUI feature/layout follow-up is drafted in tui-parity-plan.md; no TUI behavior changed.
