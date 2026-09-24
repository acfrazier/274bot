# TUI: headless operator panel (`tui-play`)

`crates/tui` is the **second view** of `host_play::Play` beside
[`panel-play`](panel.md). Same slots, same vault layout for the bound
server profile, same `--live script_*` scenarios. Slots spawn
`RasterMode::Off` and attach no `Renderer` — no `panel` / imgui / wgpu.
This is the VPS operator panel, not a MUD client.

## Run

```bash
export BOT_VAULT_PASS=bot
cargo run --release -p tui --bin tui-play -- --profile local-289
# same scenarios as panel-play --live:
cargo run --release -p tui --bin tui-play -- --profile local-289 --live script_nav_routes
```

Shared server-profile flags match host-play / panel-play
(`--profile local-274|local-289|public-289`, `--revision`, `--prod`,
`--vault`, `--vault-pass` / `BOT_VAULT_PASS`, `--host`, `--port`,
`--cache`, `--catalog`, `--user`, `--live`, …). On `public-289`,
`--world N` chooses the starting world for accounts stored as auto; pinned
accounts keep their vault setting. Unit tests render to
ratatui `TestBackend` (no TTY). `--live` headed when a controlling
terminal is present; otherwise it pumps headless and still prints
PASS/FAIL.

## Panes

| Pane | What it shows |
| --- | --- |
| Strip | vault / running slot names with each active public world; Tab / click focuses (`Play::focus`) |
| Map | packed collision dots, town pins, `@` here, remaining-walk `*`. Walk-confirm is `host_play::arm_walk_on`. WASD one-tile walk. Empty title is `map (no nav pack)` when no bound/bundled nav world loaded. |
| Chat | game chat ring + NPC dialogue Continue / Answer; a recording script's paint shows here instead (`p` toggles back) |
| Status | same `SlotStatus` + `RandomStatus` as the panel, including active world |
| Inv / stats / locs | focused snapshot |
| Script | Browse/Start/Pause/Stop + Load over the same JS library as the panel; `$RS2B0T` / `--catalog` cards included ([script.md](script.md)) |
| Settings | popup: `random_events`, `lamp_skill`, `lamp_auto` (persisted on the profile) |

`q` quits, `s` settings, `m` spawn the rest of the MultiBox wall.

## Limits

Reload, Refresh catalog and bulk **Start all / Stop all** are native-panel
controls; the TUI keeps the focused script controls above. Catalog
compatibility remains partial. In-tree farming script *ports* are not the
product surface — catalog/file bots run through the shim; guardian solvers
are host-side.
