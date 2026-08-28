# Settle: pollable evidence an interaction landed

`api::settle` is the pollable settle model. The host (or a scenario/nav
layer) polls one `Settle` per tick; nothing blocks.

## Outcome

```rust
enum Outcome<'a> {
    Refused { reason: SendReason, tick: u64 },      // produced by Interactions, not Settle
    Matched { arm: &'a str, now: ReadContext<'a>, before: ReadContext<'a>, tick: u64 },
    Expired { now: ReadContext<'a>, before: ReadContext<'a>, tick: u64 },
}
```

`Settle::new(options, before)` takes the arms and the "before" `ReadContext`;
`Settle::poll(&mut self, now) -> Option<Outcome>` runs one watch step:
check each arm's evidence against (now, before) → `Some(Matched)`; else if
disconnected/not-ingame or the tick/ms budget is exhausted → `Some(Expired)`;
else `None` (still watching). `SettleOptions { arms: &[(name, Evidence)],
budget_ticks, budget_ms }`.

## Evidence

`Evidence<'a> = Box<dyn Fn(&ReadContext, &ReadContext) -> bool + 'a>` — a
closure capturing its parameters.

- `arrived(tile, radius)` — standing within `radius` chebyshev on the tile's
  level.
- `item_delta(id, change, container)` — stacked-count delta in a container.
- `xp_gained(skill, at_least)` — xp rise since `before`.
- `engaged(target)` — local player targets it, or its health dropped.
- `modal_opened(root)` / `modal_closed(root)` — modal id transitions.
- `option_gone(target, action)` — the action no longer appears on the loc.
- `said(phrases)` — a new chat line since `before` contains a phrase.
- `server_refused()` — the map flag cleared without arriving.
- `scene_ready()` — `scene_state == 2` and a non-zero scene base.
- `inventory_changed()` — the inventory slot:count signature changed.

The poll order mirrors m8aq's `Settle.ts` watch loop: arms first, then the
disconnect/ingame check, then the tick-then-ms budget (`LIVE_TICK_MS = 600`,
`BACKSTOP_MULTIPLIER = 4`).
