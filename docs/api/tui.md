# TUI: headless operator panel (`tui-play`)

`crates/tui` is the **second view** of `host_play::Play` beside
[`panel-play`](panel.md). Same slots, same vault, same `--live script_*`
scenarios. Slots spawn `RasterMode::Off` and attach no `Renderer` — no
`panel` / imgui / wgpu. This is the VPS operator panel, not a MUD client.

## Run

```bash
export BOT_VAULT_PASS=bot
cargo run --release -p tui --bin tui-play
# same scenarios as panel-play --live:
cargo run --release -p tui --bin tui-play -- --live script_nav_routes
```

Flags match host-play / panel-play (`--vault`, `--vault-pass` /
`BOT_VAULT_PASS`, `--host`, `--port`, `--cache`, `--user`, `--live`).
Unit tests render to ratatui `TestBackend` (no TTY). `--live` headed
when a controlling terminal is present; otherwise it pumps headless and
still prints PASS/FAIL.

## Panes

| Pane | What it shows |
| --- | --- |
| Strip | vault / running slot names; Tab / click focuses (`Play::focus`) |
| Map | packed collision dots, town pins, `@` here, remaining-walk `*`. Walk-confirm is `host_play::arm_walk_on`. WASD one-tile walk. Empty title is `map (no nav pack)` when `$NAV_PACK` / `~/.274bot/274bot.navpack` did not load. |
| Chat | game chat ring + NPC dialogue Continue / Answer |
| Status | same `SlotStatus` + `RandomStatus` as the panel |
| Inv / stats / locs | focused snapshot (bank UI is 0.1.5) |
| Script | chrome shape only (unwired) |
| Settings | popup: `random_events`, `lamp_skill`, `lamp_auto` (persisted on the profile) |

`q` quits, `s` settings, `m` spawn the rest of the MultiBox wall.

## Not this tag

Script start/pause/stop wiring, bank snapshot, evade / plant / maze /
mime / box / lamp-use solvers (guardian detect + dialog act *are* this
tag; those solvers are 0.1.5).
