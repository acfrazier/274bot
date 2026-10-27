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
[the harness guide](../dev/harness.md#core-gated-qualification). `--help` (or
`-h`) prints the usage on stdout and exits 0; any other bad flag prints
usage on stderr and exits 2.

The vault passphrase is never an argument or an environment variable:
`tui-play` asks on the terminal, or reads one line from a pipe with
`--vault-pass-stdin` (`printf '%s\n' "$PASS" | tui-play --profile local-289
--vault-pass-stdin`). A new vault needs a non-empty passphrase after trimming
surrounding whitespace; strength is the user's choice. An existing one opens
with the passphrase it was created with. In a pty, send the passphrase at the
prompt and end it with Enter (`\r` or `\n`). `BOT_VAULT_PASS` and
`--vault-pass` were removed; see [vault.md](vault.md#passphrase-sourcing).

At an interactive start the boot loads and logs in the first `--user`
profile, or the first profile in saved order without one. `tui-play --user`
only selects a profile already in the vault. It creates none (unlike
`host-play --user`, which mints missing users with random game
passwords). An unknown `--user` name exits 1 with a message that names
how to add it, and creates nothing. An empty vault does not exit: the
app opens on the **New profile** form (the shared profile form's blank
row, `OpenNewProfileForm`), where the operator types the username and
the account's own password. Enter on the password field saves through
`CreateProfile`. The new profile then loads, logs in and takes focus.
The same form opens later with `N` in the Fleet pane or the palette's
**New profile**.
Startup loading waits for shared preparation to adopt preferences and
reconcile saved profile order. Without `--user`, the default target is
chosen from that order only after preparation succeeds. An explicit
`--user` target is tracked by vault UID, so a rename while preferences
load does not lose the request.
An explicit focus change replaces only the automatic startup focus.
Queued startup profiles still load and log in. A profile saved from **New profile**
while the boot load waits joins the startup queue rather than replacing
the boot target. Each unique target loads and logs in once in saved
order, and the newly saved profile becomes the startup-focus target
unless the operator changes focus. Fleet **Log in** follows the same
rule when it selects the first loaded bot. Removing or deleting one
pending startup target cancels only that profile's queued load and login.
Removing a member before the default target is chosen cancels that
default choice, but keeps other queued profiles.

`--map-bundle OUT --revision 289 --cache JAG_DIR --unpack SNAPSHOT_ROOT` is a
release-packaging run, not a panel: it bakes the WalkTo map catalogue and
terrain for that offline client cache through the production map-cache path
and ships them under `OUT/map/<revision>/` (see
[release packaging](../../tools/release/README.md)), then prints the shipped
descriptions (`{"images": …, "catalogue": …}`: each identity, key, manifest
receipt and totals) as JSON and exits.

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
| Fleet | every profile in the saved Load order, loaded or not (a profile that is not loaded is dim, state `not loaded`): a header line `card: <card> · sort: <order> · N marked`, then `[x]` row mark, `>` cursor, `*` selected bot, world, lifecycle state; `/` filter; `N/M shown · marked K (H not shown)`; Load+login / Log out buttons. A click on the header's `card: …` part opens the card picker and a click on its `sort: …` part shows the next sort (the same as `c` and `s`). The panel's Fleet window, rail and Grid read the same rows, filter, order, marks and fleet card |
| Overview | the selected bot's buttons (Log in, Log out, Remove, Settings, Loadouts, Manual walk), status rows (same `SlotStatus` + `RandomStatus` as the panel, including active world), the status pane's script, phase and reason rows (the same rows as the panel's Status card) and inventory / stats / nearest locs |
| Map | packed collision dots, POIs, `@` here, remaining-walk `*`; Walk-confirm is `host_play::arm_walk_on`; Walk / Teleport / Group / Search buttons. Its info line shows `zones: avoided` or `zones: crossing (z)`. The catalogue is requested only while this tab is open. |
 | Script | Browse/Start/Pause/Stop/Load, Parameters, Reload, Start fleet / Stop fleet over the compiled native cards (Gatherer, Grinch, Quester, plus Sherlock in builds with the JS runtime) plus the same JS library as the panel; `$RS2B0T` / `--catalog` cards included ([script.md](script.md)). Sherlock is available only in builds with the JS runtime. Sherlock was meant to be native. In practice it became a port of the compatibility clue machine, so it needs the JS runtime build until the native clue rewrite replaces that machine. Sherlock itself never creates a V8 isolate. |
| Chat | game chat ring and NPC dialogue Continue / Answer; a recording script's paint shows here instead (`p` toggles back), with its buttons as `[N] label` rows. A script that uses the standard paint-controls helper shows working **Pause** (reads **Resume** while paused) and **Stop** rows; they act on the bot whose paint is shown and are dimmed when the run cannot take them ([script.md](script.md)). An open dialogue marks the tab `Chat!` and the BOT line `DIALOGUE`. |
| Logs | the shared structured log (same filters, search, follow, Save and session file as the panel) |
| Settings | popup: `random_events`, `lamp_skill`, `lamp_auto`, `allow teleports` / `allow wilderness` / `allow bank fetch` / **Danger routing** (Never, When survivable (the default), Always; When survivable currently acts as Never), **Pause script on manual movement**, **map bake** `ask`/`always` (`map_bake` in `panel-ui.json`), **memory** `lowmem` / `highmem` (persisted on the profile), Quester paths gate + folder + reload. A memory switch queues the entire mode for the **next login after a clean logout**: live textures, tabs, music and sound stay unchanged. A socket reconnect, automatic or explicit **Log in** after a dirty drop, keeps the applied mode and leaves the change queued. Settings and Overview show the applied and queued modes; logged-in slots offer **r Relog now** through the ordinary login queue only while a different mode is queued and no relog is in flight. After a clean logout, the notice says it applies at the next **Log in**, without a Relog now offer. Parked after a dirty drop, it instead explains that **Log in** keeps the current mode; reconnect before using **Relog now**. The TUI map is catalogue-only, so it never bakes terrain or asks. |

The Settings border shows **Saving preferences…** while a shared save is
pending. The process writer keeps the accepted save after you close the popup.
Refresh does not replace an unsaved checkbox or folder draft with an unchanged
saved value.

A saved row (permissions, danger routing, the manual-walk pause, map bake, the
Quest Paths gate and folder, the session log file, and the background-bots
notice) takes effect only after its save has reached the disk. While it is
pending the row shows your choice followed by `— saving…`. If the save fails the
row shows the saved value again followed by the reason, for example
`— Not saved: …`. The reason leaves when you act on that row again, or after 15
seconds. When the selected row with its reason is taller than the popup (a long
path on an 80x24 terminal), the popup shows part of that row and its last line
says more is below or above: Up and Down then scroll through the row before
they move to the next one. The popup always keeps two lines for the selected
row, so that sign always has a line. When the notices under the rows (with the
reason of a failed dismiss) need more lines than the popup leaves them, the
notice area shows the last notices and its first line says
`↑ more notices above (PgUp)`, `↓ more notices below (PgDn)` or both: PgUp and
PgDn scroll through every notice line without moving the row. A failed
**Got it** keeps the background-bots notice on screen with the reason after
it. The saved Load order has no row: its
message line says
`Load order: saving…` while a reorder is written, and a failed save shows its
reason there. The Logs status line follows the session file whoever changed
it (you, an earlier run that left it on, or another program copy): it says
`session file: saving…` while the choice is pending, then whether the file
opened or could not be opened or written, also after it had been writing. The
failure leaves with the same 15 seconds, and nothing replaces it.

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
| Fleet | arrows / `j` `k` / PgUp PgDn Home End move the cursor; `Enter` selects that bot (a profile that is not loaded is not selected: mark it and press `m`); `Space` marks the row for group actions, loaded or not; `a` marks the shown rows, `A` unmarks them (hidden marks stay); `c` picks the fleet card; `s` shows the next fleet order (Load order, Name, Status, then Load order again; palette: **Cycle fleet sort**), and the cursor stays on its profile; `m` Load + log in and `U` Log out (a fleet plan over the marked rows, else over the loaded shown rows, and it asks first when nothing is marked); `W` opens the world picker for the cursor row's profile (palette: **Switch world of marked…** for the marked rows); `N` opens the New profile form (palette: **New profile**); `/` filters by name, `wN` / `world:N` or state (`not loaded` included); `Alt+Up` / `Alt+Down` move the cursor's profile one shown row up or down in the saved Load order (palette: **Move profile up**, **Move profile down**, and **Move marked to top**, which keeps the marked profiles' order); a profile hidden by the filter keeps its place |
| rs2b0t import (palette **Import rs2b0t profiles…**) | type the path of rs2b0t's `rs2b0t-profiles.json`, `Enter` previews; in the preview `o` switches Skip/Overwrite for existing profiles, `u` (when the file has accounts the 289 client cannot log in with) imports those too or leaves them out, `Enter`/`y` imports, `Esc` goes back; in the report `d` asks to delete the export file (`y` deletes, `n` keeps; offered only once every chosen account is confirmed saved), `Esc` closes. See [vault.md](vault.md#importing-profiles-from-rs2b0t) |
| New profile (`N` in the Fleet pane, palette **New profile**, or an empty vault at start) | type the username, `Tab` (or `Enter`) moves to the password, type the account's own password (shown only as a mask), `Enter` saves the profile and loads, logs in and focuses it; a refusal or a failed write shows in the form and keeps the typed draft; `Esc` closes without saving (it waits while a save is being written) |
| Overview | `i` Log in, `u` Log out, `x` Remove…, `o` settings, `l` loadouts, `w` Manual walk, `n` Got it (background-bots notice) |
| Map | arrows / `hjkl` pan, `+` `-` zoom, `Enter` select centre then Walk, `/` search (name or `x,z,plane`), PgUp PgDn `0`-`3` plane, `d` `c` `r` dots / collision / reach layers, `w` wilderness overlay (persisted as `nav.show_special_areas` in the shared panel UI settings), `z` route through danger zones for this Map open (superseded while Danger routing is Always), `g` group send, `Space` select the selected bot for the group, `t` teleport (local), `R` recenter, `Esc` clear selection, then back |
| Script | `b` Browse, `t` Start, `P` Pause/Resume, `e` Stop, `f` Load, `v` Parameters, `R` Reload/Confirm, `C` Cancel, `T` Start fleet (the fleet plan), `E` Stop fleet; in Browse, Up/Down pick and `Enter` or `Esc` close it (the pick stays for `t`); in Parameters, `Space` steps through an option's choices and `Enter` opens its choice list (searchable past 16 options) |
| Chat | arrows / `j` `k` choose, `Space` / `Enter` continue or answer, `1`-`9` script paint buttons, `p` paint / game chat |
| Logs, log drawer | `/` search, `v` level, `s` source, `b` scope, `f` follow, arrows PgUp PgDn Home End scroll, `w` save, `F` session file |
| Manual walk | `W` `A` `S` `D` / arrows walk one tile; `Esc` disarms |

Commands ending in `…` confirm first: Remove names the bot it will remove
(and keeps the vault profile). The fleet actions confirm only when core's
plan says so (below).

**Marked rows.** The `[x]` marks are the one selection every fleet command
reads, including the Map's group send. A mark stays when the filter hides
its row or the bot is unloaded; deleting the profile drops it. `a` marks
every shown row (loaded or not) and `A` unmarks the shown rows; marks the
filter hides are kept. The palette adds **Invert shown** and **Mark by
status: ready / idle / running / logged out / login error / not loaded**,
which mark the shown rows in that status (other marks stay). Core decides
which rows each of these touches.

**Fleet card.** The header line of the Fleet pane reads `card: <card> ·
sort: <order> · N marked` (it wraps by part in a narrow pane). The card is
the one the fleet actions use: **Each bot's own card** (the default),
**Focused bot's card: X** (follows the focus; `(pending)` while X is an
unsaved Browse pick) or one pinned card. `c` (palette **Pick fleet card…**)
opens the picker over Each bot's own card, Focused bot's card and then every
Browse card, grouped by category as the Script tab's Browse groups them.
`Up` / `Down` / `j` / `k` / PgUp PgDn Home End move; `Enter` pins the
highlighted card, `Esc` (or a click outside) keeps the card as it was.
Picking changes no assignment, no Browse pick, no profile form and no run.

**Switch world.** `W` (palette **Switch world of cursor row…**) opens the
world picker for the profile under the fleet cursor, not the selected bot.
The palette's **Switch world of marked…** opens the same picker for the
marked rows, or the loaded shown rows when none is marked. The picker lists
the worlds of the server the session is bound to, as that server's world
list was read when the session started (opening it fetches nothing), and
ticks the bot's own world with `[x]`. `Up` / `Down` / `j` / `k` / PgUp PgDn
Home End move; `Enter` or a click picks; `Esc` (or a click outside) leaves
everything as it was. Picking the world the bot is already on closes the
picker and does nothing. Picking another world asks core for a Switch world
plan over that profile alone (a marked row does not join a `W` pick) or over
the marks, and the confirm overlay shows core's prompt as written: a Switch
world plan always asks, `Enter` / `y` runs it and `Esc` / `n` cancels it
(nothing is saved or relogged before the plan runs). A confirmed switch saves
the bot's world on its profile and only then logs the bot out and back in on
the new world through the login queue; a failed save leaves the bot logged
in. A running script continues after the bot reconnects and a script that
was already paused stays paused. The message line shows the result, with
each bot's real outcome rather than that the relog started. A bot that is
queued for login, logging in, logged out or not loaded is refused with its
reason on the message line (`Switch world: no bot to act on (alice: queued
for login)`). On a server with one world (a local one) `W` and the palette
entries say `Switch world: this server has one world` and open nothing.

**Fleet actions.** `m` Load + log in, `U` Log out, `T` Start fleet and `E`
Stop fleet (Script tab), and the palette's **Assign**, **Assign & restart** and **Copy
settings** each ask core for a plan. With rows marked the plan covers every
marked profile, hidden ones too; with nothing marked it covers the loaded
bots the filter shows and asks first. Unloaded profiles are reached only by
marking them (`a`, then `m`). Core freezes the targets, the card and the
skips with their reasons when the plan is made: a bot marked, loaded or made
ready afterwards never joins it. A plan runs at once unless it covers the
shown bots (nothing marked), would stop a running script, or copies
settings; then a confirm overlay shows core's prompt as written. `Enter` /
`y` runs the plan, `Esc` / `n` cancels it (nothing is written; a settings
copy releases its hold). Only one plan waits: a newer action replaces it and
its prompt starts "Replaces the waiting … plan." A refused action (for
example "no bot to act on") shows its reason on the message line.
**Assign**, **Assign & restart** and **Copy settings** need a card: while the
fleet card is Each bot's own card they are unavailable ("pick a card first").
**Start** with that card starts each bot's own assignment, paced through the
same one-per-frame Start permit. **Assign & restart** stops a running script
first; **Copy settings** copies the focused bot's parameters for the card to
the targets on that card (the prompt names them and each skip).

**Result.** Each action ends in one result on the message line (`msg:`),
failures first, then cancellations (each with its reason when one is known,
such as `a later Relog now took over` or `the automatic login guard stopped
repeated logins`; otherwise `cancelled`), then skips, then
done, with a `pending N` tail until every
member settles (a paced Start settles with its last queued bot). It is
written once, not repeated as a banner; core also writes one log line when
it settles.

**Script settings restarts.** Saved settings needing a restart show a `!`
badge in the fleet; Parameters says `Restart required to apply saved
settings.` The prompt offers **Restart** and **Later**. **Restart** applies
only to listed bots still running the same assigned script with its pending
settings. Stopped, logged-out, reassigned or different-script bots are
skipped and named in the status line. **Later** leaves their current scripts
running and suppresses the prompt until settings change again.

Grinch's saved Parameters apply only after a new Start. That Start validates
the settings and captures a new same-plane anchor. Its `radius` bounds an
inclusive 1–16 tile square around that anchor, and the live-zone filter
rejects stale rows. See [Grinch ground-item collection](script.md#grinch-ground-item-collection)
for all 18 default IDs, selected-only banking, exact-anchor return and status.

With the Map's group send (`g`), `Enter` walks every marked bot to the tile
from its own position. Pressing `g` while no bot that can walk is marked
marks the loaded bots the fleet filter shows that can walk; a bot the filter
hides is not added. Each of these prints one report with every marked bot
counted once, failures first (`Walk marked: walking 3, skipped 2: gwalk2:
running a script, gwalk3: not logged in`); zone refusals include the danger
zone names for each affected bot. On legacy grid packs `z` cannot enable
checks and the report says `zones: unavailable (legacy grid pack)`. Bots that
are logged out, have no position yet or run a script are skipped with that
reason. The `z` choice starts with zones avoided and resets whenever Map
closes or opens; it is not persisted. While Danger routing is Always, the per-walk checkbox is superseded.

**Mouse** (optional; every workflow works from the keyboard): left click
focuses the pane, selects a fleet row (its checkbox column ticks the row
instead), opens the card picker from the fleet header's `card: …` part or
shows the next sort from its `sort: …` part (the marked count does nothing),
presses a button or tab, answers a dialogue option or selects a
map tile (walking it is a second action); right click opens a context menu
for the row or pane and never acts as a left click; the wheel scrolls the
list, log or map under the pointer. The palette's **Mouse capture off**
hands the mouse back to the terminal for selecting and copying text. A
resize re-lays the shell at once; click targets always come from the
current layout. An open overlay or Script popup swallows clicks outside
itself and takes the wheel: Browse and Load close on an outside click, the
catalog prompt stays (dismissing it means Not now).

### Changed keys (earlier layouts)

The old strip layout had global letters that also fired from popups.
They now live in their pane: `i` / `u` / `x` / `o` / `l` on the Overview,
`m` / `U` in the Fleet, the script letters in the Script tab and `p` in
Chat. `Tab` moves keyboard focus instead of cycling the bot (select bots in
the Fleet with `Enter`, a click, or the palette's Select next / previous
bot). WASD walks only in the armed Manual walk box. `q` asks before
quitting while bots are loaded. `x` and the fleet actions that ask (see
Fleet actions above) confirm first.
Esc no longer dismisses the background-bots notice (use `n` or its **Got
it** button). A dialogue is answered from the Chat tab (`F6`). The map
group checklist is the fleet's row selection.

## Limits

There is no Debug tab here: server cheat commands live in the panel's
Debug tab on local-engine profiles only. Catalog compatibility remains
partial. Catalog/file bots run through the compatibility shim, not as
ports; guardian solvers are host-side.
