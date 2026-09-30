# TUI: headless operator panel (`tui-play`)

`crates/tui` is the **second view** of `host_play::Play` beside
[`panel-play`](panel.md). Same slots, same vault layout for the bound
server profile, same `--live script_*` scenarios. Slots spawn
`RasterMode::Off` and attach no `Renderer` — no `panel` / imgui / wgpu.
This is the VPS operator panel, not a MUD client.

## Run

```bash
# asks for the vault passphrase on the terminal (hidden; a new vault asks twice)
cargo run --release -p tui --bin tui-play -- --profile local-289
# same scenarios as panel-play --live:
cargo run --release -p tui --bin tui-play -- --profile local-289 --live script_nav_routes
# qualify under the shared core witness (panel catalog_watch / pair_watch):
cargo run --release -p tui --bin tui-play -- --profile local-289 --live script_thiever --catalog-core
```

Shared server-profile flags match host-play / panel-play
(`--profile NAME`, `--rs2b2t`, `--revision`, `--vault`,
`--vault-pass-stdin`, `--host`, `--port`, `--cache`, `--catalog`, `--user`,
`--live`, …). On `rs2b2t`, `--world N` chooses the starting world for
accounts stored as auto; pinned
accounts keep their vault setting. Unit tests render to
ratatui `TestBackend` (no TTY). `--live` headed when a controlling
terminal is present; otherwise it pumps headless and still prints
PASS/FAIL. `--catalog-core` / `--pair-core` (environment form
`BOT_LIVE_CORE=catalog|pair`) hold a `--live` PASS until the shared core
witness qualifies, exactly as the panel watches do; see
[the harness guide](../harness.md#core-gated-qualification).

The vault passphrase is never an argument or an environment variable:
`tui-play` asks on the terminal, or reads one line from a pipe with
`--vault-pass-stdin` (`printf '%s\n' "$PASS" | tui-play --profile local-289
--vault-pass-stdin`). A new vault needs a non-empty passphrase after trimming
surrounding whitespace; strength is the user's choice. An existing one opens
with the passphrase it was created with. In a pty, send the passphrase at the
prompt and end it with Enter (`\r` or `\n`). `BOT_VAULT_PASS` and
`--vault-pass` were removed; see [vault.md](vault.md#passphrase-sourcing).

`--map-bundle OUT --revision 289 --cache JAG_DIR --unpack SNAPSHOT_ROOT` is a
release-packaging run, not a panel: it bakes the WalkTo map terrain for that
offline client cache through the production map-cache path and ships it under
`OUT/map/<revision>/` (see [release packaging](../../tools/release/README.md)),
then prints the shipped description (identity, key, manifest receipt, tile
totals) as JSON and exits.

## Layout

The shell adapts to the terminal size:

- **80×24 (compact, below 100×30):** two header rows (title, fleet counts
  and resources; then `BOT <name> @world state` and the tabs), one main
  pane, a three-row drawer (last message, newest log line, unread counter)
  and the footer. The main pane shows the selected bot's tab, or the fleet
  table while the fleet has keyboard focus (**F2**, the "fleet drawer").
- **120×40 (standard, from 100×30):** the fleet table on the left, the
  selected bot's tab on the right under a `BOT <name> │ world │ state │
  script` line, the log drawer (message line plus log rows) and the footer.
- **Large (from 160×45):** as standard, with queue and tile columns in the
  fleet table and the selected bot's status and chat beside the tab.

The footer always starts with `KEYS <scope> ▸`: the pane, text field,
popup or overlay that receives keys. The focused pane also has a `[keys]`
label in its border. Everything is plain text as well as colour.

| Pane | What it shows |
| --- | --- |
| Fleet | loaded members: `[x]` row selection, `>` cursor, `*` selected bot, world, lifecycle state; `/` filter; `N/M shown · selected K of M`; Load+login all / Logout all buttons |
| Overview | the selected bot's buttons (Log in, Log out, Remove, Settings, Loadouts, Manual walk), status rows (same `SlotStatus` + `RandomStatus` as the panel, including active world) and inventory / stats / nearest locs |
| Map | packed collision dots, POIs, `@` here, remaining-walk `*`; Walk-confirm is `host_play::arm_walk_on`; Walk / Teleport / Group / Search buttons. Search hit rows use display names (`/` line-breaks become spaces). The map draws one-cell glyphs, not overlapping text labels. The catalogue is requested only while this tab is open. |
| Script | Browse/Start/Pause/Stop/Load, Parameters, Reload, Start all / Stop all over the same JS library as the panel; `$RS2B0T` / `--catalog` cards included ([script.md](script.md)) |
| Chat | game chat ring and NPC dialogue Continue / Answer; a recording script's paint shows here instead (`p` toggles back). An open dialogue marks the tab `Chat!` and the BOT line `DIALOGUE`. |
| Logs | the shared structured log (same filters, search, follow, Save and session file as the panel) |
| Settings | popup: `random_events`, `lamp_skill`, `lamp_auto` and **lowmem / highmem** (persisted on the profile); session nav opt-ins; **map bake** `ask`/`always` (`map_bake` in `panel-ui.json`). A memory switch applies to the live client immediately and shows the server login mode until the next login; `r` explicitly relogs through the ordinary login queue and is disabled while that relog is queued. The TUI map is catalogue-only, so it never bakes terrain or asks. |

## Keys

One routing model: an open popup or overlay takes every key (the Script
tab's Browse, Load file and catalog folder prompt included: only their own
Up/Down, `j`/`k`, `Enter` and `Esc` act until they close); then the
focused pane's text field (fleet filter, map search, log search) takes
typing; then the global keys; then the focused pane's own keys. No letter
is global, and moving the fleet cursor never changes the selected bot.

| Global | |
| --- | --- |
| `F1` / `?` | help: the focused pane's keys first, type to search |
| `Ctrl-P` / `:` | command palette: every command with its target and, when it cannot run, why (reaches every screen when the terminal swallows function keys) |
| `F2` | Fleet; `F3` Overview, `F4` Map, `F5` Script, `F6` Chat, `F7` Logs (switching never starts anything) |
| `Tab` / `Shift-Tab` | keyboard focus between fleet, tab and log drawer |
| `Esc` | close or go back one level (never runs a command) |
| `q` / `Ctrl-Q` | quit; asks first while bots are loaded (they log out) |

| Pane | Keys |
| --- | --- |
| Fleet | arrows / `j` `k` / PgUp PgDn Home End move the cursor; `Enter` selects that bot; `Space` selects the row for group actions; `/` filters by name, `wN` / `world:N` or state; `m` Load+login all…; `U` Log out all… |
| Overview | `i` Log in, `u` Log out, `x` Remove…, `o` settings, `l` loadouts, `w` Manual walk, `n` Got it (background-bots notice) |
| Map | arrows / `hjkl` pan, `+` `-` zoom, `Enter` select centre then Walk, `/` search (name or `x,z,plane`), PgUp PgDn `0`-`3` plane, `d` `c` `r` layers, `g` group send, `Space` select the selected bot for the group, `t` teleport (local), `R` recenter, `Esc` clear selection, then back |
| Script | `b` Browse, `t` Start, `P` Pause/Resume, `e` Stop, `f` Load, `v` Parameters, `R` Reload/Confirm, `C` Cancel, `T` Start all…, `E` Stop all…; in Browse, Up/Down pick and `Enter` or `Esc` close it (the pick stays for `t`) |
| Chat | arrows / `j` `k` choose, `Space` / `Enter` continue or answer, `1`-`9` script paint buttons, `p` paint / game chat |
| Logs, log drawer | `/` search, `v` level, `s` source, `b` scope, `f` follow, arrows PgUp PgDn Home End scroll, `w` save, `F` session file |
| Manual walk | `W` `A` `S` `D` / arrows walk one tile; `Esc` disarms |

Commands ending in `…` confirm first: Remove names the bot it will remove
(and keeps the vault profile), and the fleet-wide commands list the
members they cover. If the fleet changes before you confirm, the dialog
shows the new scope instead of running.

**Marked rows.** The `[x]` marks are the one selection every fleet command
reads. With rows marked, **Load+login all** and **Logout all** log in / out
the marked bots only (loading unloaded ones first), **Start all** / **Stop
all** start / stop them, and the palette adds **Assign script to marked…**
(saves the Browse pick as their assignment without starting; running bots are
skipped), **Assign & restart marked…** (stops a running script, then
starts the pick through the same one-per-frame Start permit as Start all) and
**Apply focused bot's settings to marked…** (copies the focused bot's
parameters for the Browse pick to the marked bots on that card; the dialog
names the bots it copies to, each marked bot it skips with the reason and how
many unmarked bots stay unchanged, and nothing is written until you confirm).
With the Map's group send (`g`), `Enter` walks every marked bot to the tile
from its own position. Each of these prints one report with every marked bot
counted once, failures first (`Walk marked: walking 3, skipped 2: gwalk2:
running a script, gwalk3: not logged in`); bots that are logged out, have no
position yet or run a script are skipped with that reason.

**Mouse** (optional; every workflow works from the keyboard): left click
focuses the pane, selects a fleet row (its checkbox column ticks the row
instead), presses a button or tab, answers a dialogue option or selects a
map tile (walking it is a second action); right click opens a context menu
for the row or pane and never acts as a left click; the wheel scrolls the
list, log or map under the pointer. The palette's **Mouse capture off**
hands the mouse back to the terminal for selecting and copying text. A
resize re-lays the shell at once; click targets always come from the
current layout. An open overlay or Script popup swallows clicks outside
itself and takes the wheel: Browse and Load close on an outside click, the
catalog prompt stays (dismissing it means Not now).

### Changed keys (0.1.9)

The old strip layout had global letters that also fired from popups.
They now live in their pane: `i` / `u` / `x` / `o` / `l` on the Overview,
`m` / `U` in the Fleet, the script letters in the Script tab and `p` in
Chat. `Tab` moves keyboard focus instead of cycling the bot (select bots in
the Fleet with `Enter`, a click, or the palette's Select next / previous
bot). WASD walks only in the armed Manual walk box. `q` asks before
quitting while bots are loaded. `x`, `m`, `U`, `T` and `E` confirm first.
Esc no longer dismisses the background-bots notice (use `n` or its **Got
it** button). A dialogue is answered from the Chat tab (`F6`). The map
group checklist is the fleet's row selection.

## Limits

Profiles, queue, catalog warm-up / reload, evidence and bulk actions over
the row selection are later screens; today the row selection feeds the map
group walk. Catalog compatibility remains partial. In-tree farming script
*ports* are not the product surface — catalog/file bots run through the
shim; guardian solvers are host-side.
