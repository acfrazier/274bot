# Viewport overlay and focused reconnect follow-up

Operator approved these separate corrections after watching the scalar-delay experiment. Visual approval is recorded; mixed throughput/timing results in scalar-delay-experiment.md remain a measurement caveat.

Viewport regression: CPU navigation/modal pixels share the chrome atlas, but upload invalidation followed chat/sidebar redraw flags. GPU readback reproduces a moved overlay leaving its old pixels visible. The correction compares visible scene RGBA against the existing last-upload buffer, including coverage-only removal; no extra retained image or allocation. Invisible RGB changes do not force upload. Scene-ready also covers the held scene texture during loading. The regression holds a zero-initialized GPU scene, moves and removes overlays without a UI redraw, and checks presented pixels and lazy unchanged uploads. Original invalidation fails, corrected invalidation passes. Library66, GPU backend10, CPU/fallback backend5 pass. The existing four iface_model fixture failures still occur and are not claimed fixed.

Focused reconnect: the operator clarified that losing focused-head login priority happens after the mainland hop. Previously prefer() only inserted the uid once; after the permit, a subsequent handshake joined FIFO at the back. Regression reproduces queue[2,1] instead of[1,2] after focused uid1's first login. The queue now remembers preference for subsequent actual requests; focus changes update it without reserving an entry for an online/unarmed slot. Pending focused requests still obey spacing, IP and device limits. Existing handshakes cannot be preempted, and this is permit priority, not a guarantee that a slower connection finishes first. Removing the focused slot clears the preference. Unit tests cover reconnect, changing/clearing focus, no phantom queue reservation, preserved limits, and Play focus wiring.

Source changes are separated by client and host commits. Final grok-4.6 review APPROVE; captured live check passed. The captured live run is functional evidence and must not be compared with uncaptured CPU cells as an application performance result.


Grok's first review requested a correction: an overlay-only freeze upload would seal the atlas and drop the held minimap. Extended readback regression reproduced black minimap pixels instead of blue. The correction preserves the held punch for overlay-only uploads while explicit chrome redraws retain their existing seal behavior. Library66, GPU backend10 and CPU/fallback backend5 pass after correction. Host118, host-play131 and panel374 tests passed. Final grok-4.6 review approved the correction; the model receipt confirms successful completion.

Local commits: client bc8bb4e (overlay invalidation),451759f (freeze interaction); host d32b493 (focused reconnect),c63fc76 (client pin). The final captured-run binary is saved in diagnostics/overlay-login-final-build with its commit/SHA manifest. Earlier pre-freeze build is not used for live evidence.


## Captured live result

Run diagnostics/20260905T235453Z_panel_n32_active exited0 and passed qualification with32 ready/active throughout observation. Setup initially reached28 while four reconnects waited; all32 subsequently completed setup. The focused live2fda0_0 was first in both initial and reconnect handshake-begin log order. This demonstrates this run's ordering; deterministic tests establish priority when requests are pending and rate-limited.

Every bot increased its steal count during the180-second observation (gains6–28), with no terminal script errors. Stop completed with active0, V8 used0 and snapshot inflight0, and the process exited after teardown. Captures retain scene-ready, bank-arrival, and return-route-start checkpoints. Inspected scene-ready and return-route images show the live game/minimap and navigation paint; still images alone do not prove temporal alignment, which is covered by the GPU readback regression. Focused bot subsequently resumed thieving after its bank return.

CPU averaged0.831 cores and client throughput40.86ticks/bot/s in this diagnostic run. Captures/debug/diagnostics were enabled, so this is functional evidence, not an acceptance comparison against the earlier uncaptured CPU cells. Remaining campaign performance qualification stays separate. Existing four iface_model fixture failures remain documented; no full client-suite pass is claimed.
