# Native observed-tick subscribers

Task `t_e8541da5` implements brief 106 on `codex/rs2b0t-multirevision` after
brief 105 review. Isolated checks used exact Git regular blobs of host
`2744b8be4619d5af9bb6d91755b6bbe339f5ba45` plus the owned overlay. Client
gitlink `9d090ed04957e4efc254f073cda97bc5510ca72b`. 3747 exclusive new files;
no archive extraction, overwrite or deletion. One exclusive target
`.superpowers/review-exports/native-tick-subscribers-t_e8541da5-target`.
Not LIVE.

## Bridge

`BotHost.addTickListener` is a module-local JS `Set`. `addTickListener(cb)`
adds the function once and returns `() => Set.delete(cb)`.
`globalThis.__rs2b0t_fire_tick_listeners` iterates the Set with per-callback
try/catch. Errors go to `host.log` / `console.error('[rs2b0t] listener error', …)`
and never to `host.lastError` or a `tick N:` line.

Fire sites only:

- COMPAT_RUNNER `__rs2b0t_tick_async`: after `host.tick = n`, before `onStart`.
- `__rs2b0t_pump`: after `state.tick = n`, before `park.settle`.

One function lookup; missing/empty fire is a no-op. Pause, hold, generation
skip, and stale-tick drain are unchanged. No `onStop`/`onPause`/`onResume`,
frame/draw listeners, packet attach, timers, or opcodes. NATIVE_MAIN is
untouched.

## Proof (isolated empty target)

Export `.superpowers/review-exports/native-tick-subscribers-t_e8541da5` with
`CARGO_TARGET_DIR=.../native-tick-subscribers-t_e8541da5-target`
(`isolated_build=true`). Raw logs: `docs/compat/evidence/native-tick-subscribers/`.

- `onStart` `addTickListener` does not set `last_error` / `tick N: not impl`.
- Zero fires on the start tick; one fire per later posted tick; `BotHost.tickCount`
  and `Game.tick()` match the posted snapshot tick.
- Parked `delayUntil`: listener count increases while `__rs_loops` stays 1.
- `pause()` freezes listeners; `resume()` continues.
- Posted `hold=true` freezes listeners; paint still forwards.
- Unsubscribe stops further fires. Same function added twice fires once.
- Throwing listener: other listeners run; `drain_logs` has no `tick ` last_error.
- Two isolates do not share the Set.
- `reset_session_work` does not fire; next Snapshot+Tick fires once.

`cargo test --locked --offline -p script --test tick_subscriber -- --test-threads=1`:
10 passed. Clippy `-D warnings` on `script --lib` and
`script --test tick_subscriber` passed.
`rustfmt --edition 2021 --check` on `load.rs` and `tick_subscriber.rs` passed.

## Limits

Root owns paired Duel LIVE qualification and the full combat/reset cycle.
`addFrameListener` / `addDrawListener` / `attach` stay `not impl`. Native still
does not call `onStop`. No foreign runtime clone. Exclusive target cache 3.4G;
root owns cleanup after review.
