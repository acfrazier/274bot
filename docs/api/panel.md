# Panel: native UI (`panel-play`)

`crates/panel` is the native **ImGui** UI (panel-owned winit + wgpu loop)
over the same kernel
slots `host-play` runs ([login.md](login.md), [vault.md](vault.md)).
Single-bot chrome plus the **MultiBox wall** (sidecar rail, grid mode,
profile chooser). The headless twin is [`tui-play`](tui.md) (raster Off,
no GPU).
It does **not** reimplement the client UI — there is **no Present**, no client
window feature. The client rasters into a frame (wgpu GPU 3D by default,
CpuPix3D if `BOT_CPU=1`); the panel blits that frame and feeds input back.

Bind one immutable entry from `~/.274bot/servers.json` for the process
(`--profile NAME` or `--rs2b2t`; see [README.md](../../README.md)).
Shared flags match host-play (`--engine`, `--cache`, `--vault`, `--catalog`,
`--lowmem`/`--highmem`, `--nav-paints on|off`, `--live`, optional
`--external-ts` for the dedicated loader smoke).

## Run

```bash
cargo run --release -p panel --bin panel-play -- --profile local-289
# 50-head RAM:  cargo run --release -p panel --bin panel-play -- --profile local-289 --live stress50
# 50-head 50fps Game+sidecar: --live stress50_full
```

A passphrase is required and an empty or whitespace-only one is rejected. A
new vault accepts any non-empty passphrase; strength is the user's choice. An
existing vault opens with whatever passphrase it was created with. First run **Create
vault** writes `~/.274bot/vault` **empty** — panel-play does **not**
auto-create `test`/`test` (that is host-play CLI: `--user test` defaults,
`password = username`). A wrong passphrase never replaces the file;
**Reset vault** (confirm + I understand) is the forgotten-password wipe.
Open **Profiles**, **New profile** or **Edit**, type a username/password,
and **Save** to upsert, spawn that slot on the login FIFO, and select it.
The passphrase is **never** an argument or an environment variable (other
users can read both): the in-panel prompt is the interactive channel, and
`--vault-pass-stdin` reads one line from a pipe (or asks on the launching
terminal) and unlocks **before** the window opens, for launchers and
harnesses that cannot type; see [vault.md](vault.md#passphrase-sourcing).
`BOT_VAULT_PASS` and `--vault-pass` were removed. There is **no mainland checkbox** in the
panel: `BOT_MAINLAND=1` or host-play `--mainland` still queues
`mainland_hop` after scene 2. On a **Local loopback** engine the **Debug**
section retains **TutSkip**, **Lumbridge**, **maxme**, and **Teles**, and links
to the **Debug** tab. The tab groups commands from the selected content
catalog by `~help` category, with search, typed arguments, content-derived
name pickers, favourites and recent commands. Item pickers share Loadouts'
item search, with exact aliases/names ranked before prefixes and substrings.
Commands target the focused running profile or the same marked selection used
by Fleet commands. Marked sends check each bot separately and report queued
targets and skipped targets with their refusal reasons. Destructive commands
require confirmation naming the targets. Public and Remote profiles cannot
send cheats; production-only commands are hidden.
The separate Debug catalog is loaded only while a Local session opens the tab
and released when it closes. Its source verification is independent: drift in
debug-only inputs disables Debug without withholding other generated facts.
Headless builds do not embed the catalog unless explicitly built with the
`debug-catalog` feature. Host logs record operator commands and each nearby
chat/modal reply candidate once, listing the pending commands (temporal
correlation, not server acknowledgement). Host-internal tutorial probes do
not create Debug reply-log entries. The Profiles editor
offers `auto` or each listed rs2b2t world; the selected world is used on the
next slot start. Live slot rows display the current world.

Last focused profile is restored from `~/.274bot/panel-ui.json`
(`last_focus`). Collapsible section open/closed state persists there per
profile; **script** and **parameters** default closed.

## Log and panel layout

The structured **Log** section remains inline in the panel. The title-row
**Log** button opens the same view as a dockable **Log** tab beside the panel
and other panel windows; the tab's **X** closes it, and dragging it out
undocks it into a floating window. Closing the tab does not move or hide the
inline section. When the Log window draws its body, the inline section shows a
one-line hint instead; an open but unselected docked tab leaves the inline
body visible. Both views share filters, follow state and retained history. An
older saved `log_detached: true` preference opens the tab once on startup,
then is discarded.

General config's **session log file** checkbox shows the full rotated path as
`writing <path>` on wrapped lines below while it is enabled. The file is under
`~/.274bot/logs/`.

Status omits walk and queue rows when their value is empty, `—`, or `-1`, and
omits the modal row unless the selected bot is in game; meaningful values
remain visible. The active server revision appears once when a session is
bound, while the revision selector is only shown before binding. When the
docked Profiles window runs past the right edge of the app window (for
example on a 1024×768 desktop), its rows, Save/Cancel and Close are laid out
within the part that is on screen, so every profile's Edit and ✕ and the
form's buttons stay reachable without scrolling.

## Wiring

`Session::unlock` starts an empty `Play` (shared `Arc<Cache>`, login FIFO)
via `host_play::run_with_io`, then selects the restored `last_focus` (or
the first vault name), which spawns **that one slot**. Parked profiles do
not get a `Client` until you select them; once spawned they stay up so the
picker can change focus.
Each running slot has its own `PixelBuf` + `SlotInput`. A per-frame
observe hook applies the focus `set_draw` switch and the mainland hop.
The runner is configured with **docking on, multi-viewports off** (single
main viewport). Default dock: **game left**, panel right. At 100% scale the
panel is **330 px** wide and the MultiBox rail is **264 px**; both keep that
logical width as the window grows vertically. The rail's tile body is
**236×155 px** at 100% scale. Splitters, undock, the tab-bar corner menu,
and imgui.ini restore are off.
Only grid mode fits the 274 blit to the resizable pane; single-bot and rail
keep the native **765×503 logical** applet centered in the leftover pane,
scaled by display DPI in physical framebuffer coordinates. The window loop
uses `HiDpiMode::Locked(1.0)` so ImGui's layout coordinates are physical
pixels. The 3270 font remains **14 logical px** and is rasterized at the
monitor scale; `ScaleAllSizes` and the panel's custom dimensions use that
same scale, including scrollbar widths. A monitor change restores and rescales
the unscaled base style before layout resumes. With renderer-managed textures,
ImGui lazily re-bakes fonts at the new scale on the next frame.
The panel owns its IME window association independently of the backend's
userdata and submits text-input caret areas in physical pixels, including at
fractional display scales.
Single-bot hides the dock tab strip (`AUTO_HIDE_TAB_BAR`). The MultiBox
**rail** keeps a tab so its close X is visible; closing it turns MultiBox
off and **shrinks** the OS window by the 264 logical px strip (same falling
edge as the 274bot MultiBox toggle). Opening the sidecar **grows** the OS
window if the 765×503 logical blit would sit under the panel or rail; a
window the operator already stretched keeps the extra. The Game blit is
panel (right-aligned in the leftover pane).
The Game pane title is the focused profile name when the tab bar is visible. Closing the Game pane sets
`game_pane_open = false` (`set_draw` off) and turns capture off. The UI
thread is capped at **50 fps** (`RedrawMode::WaitUntil`) so it does not
Poll-spin against the 20 ms slot; pixel uploads skip while `PixelBuf`
generation is unchanged.

The Game Image is an ImGui **wgpu texture**. The 274 scene behind it is
the client submodule's **wgpu GPU 3D** renderer by default (CpuPix3D
when `BOT_CPU=1`). That is not the client's `Present` applet
(`client-play --window` on bothost). Unfocused / renderer-off slots skip
`game_draw`. Watch-only is a **1 fps rail**; capture is 50 fps.

Run **`--release`**. A default debug Pix3D pins a core. One live client
still holds on the order of a gigabyte (process-wide model/anim stores +
scene); that is the 274 painter, not the ImGui chrome.

## Renderer vs capture

274bot defaults to **lowmem** (`Profile.settings.lowmem = true`). The
checkboxes on a focused profile:

- **game renderer** — default **on**, **1 fps rail** (rs2b0t). Checking
  it after off paints **this tick** (no cold wait). Capture raises the
  focused slot to 50 fps; sidecar 50 fps raises unfocused wall members;
  full rate (this run) raises every drawing slot (below). Unfocused slots
  do not raster. The Game Image is an RGBA8 texture that is **never below
  765×503** (native, non-grid). Grid tiles `fit_applet` into their cell.
  Rendering never pauses the bot. Renderer-off /
  `set_draw(false)` detaches the head — GPU textures, chrome, and the
  decoded 3D scene are freed — while `mainloop` and collision keep
  running; flipping the renderer on reattaches 3D from the same map
  bytes on the **same** client (never a logout or restart).
- **sidecar 50 fps** — unfocused wall members at 50 fps. Interactive /
  non-test. Scenarios never flip it.
- **full rate (this run)** — `--live script_*` / smoke / `stress50_full`.
  Every drawing slot at 50 fps, focused included. Ephemeral.
- **lowmem / highmem** — default **lowmem**. Click the current mem
  button (under none/GPU/CPU) for a sticky picker like Teles. Highmem is
  `Profile.settings.lowmem = false`. The switch queues the entire mode for
  the **next login after a clean logout**: live textures, tabs, music and
  sound stay unchanged. A socket reconnect, automatic or explicit **Log in**
  after a dirty drop, keeps the applied mode and leaves the change queued.
  While a logged-in slot has a different mode queued, the picker and the
  persistent status notice offer **Relog now**, which logs out and back in
  through the ordinary login queue. Closing the picker or folding Status
  does not hide that offer. After a clean logout, the notice says the next
  **Log in** applies the queued mode, without offering Relog now. Parked
  after a dirty drop, it instead explains that **Log in** keeps the current
  mode; reconnect before using **Relog now**. A running or queued script
  Start is disclosed before
  relogging, and the queued label clears when login succeeds, fails
  terminally, or the operator takes over with Log in / Log out.
- **capture input** — click-through: while on and the Image is hovered,
  local coords stream `InputEv::Move`, mouse buttons send `Down`/`Up`
  (left=1, right=2). New keys/text go to that slot only while the native
  window is focused and no panel widget wants the keyboard. Releases for
  game-held keys do not depend on Image hover: focusing a panel field or
  leaving the native window releases outstanding user input. Turning capture
  off or switching bots releases the old slot's held keys and mouse buttons
  at its next input-consumer tick. That release survives a quick reselect or
  capture off/on cycle, even if the input channel is replaced before the tick.
  Off means watch-only: no new input is forwarded, and buffered presses from
  detached input are discarded rather than replayed when re-enabled.

ImGui settles keyboard focus over frames. A key typed in the same frame as
clicking back to the game may be dropped; subsequent game input is forwarded
once the panel field has relinquished keyboard focus.

Capture follows focus (never two keyboards) and implies renderer.
Capture still keeps the focused slot on the 20 ms loop for click-through;
it is not how tests get fps.

## MultiBox wall

The title-row **MultiBox** toggle raises the wall. On the first toggle the
wall seeds with every already-running slot and opens the **chooser** once;
later toggles go straight to the rail. **Closing the rail does not log
anyone out** — MultiBox off drops the grid, closes the chooser, and stops
extra rasters (`wall_open = false`); every slot keeps running.

### Rail

A 264px sidecar on the far right (`RAIL_W`): a sticky bulk row with
**Login all** / **Logout all**, then **Start all** / **Stop all** (script
bulk — separate from login bulk; see [script.md](script.md)), an
**only render selected** checkbox, one
tile per wall member, **+ add bot**, and the 1 Hz resource card. A tile is
a cap — traffic-light dot, name, **✕** — over a 236×155 body that blits the
member's `PixelBuf` (or a renderer-off placeholder). Clicking a name or
body focuses the member. The dot is error **red**, then ingame **amber**,
then FIFO-queue **warn**, else grey. **only render selected** mirrors
`Focus.only_render_selected`: when off, unfocused wall members also
`set_draw` (the tile body shows their raster); when on, only the focused
member paints. The Game pane always follows **focus** — a rail-Off
member still grows a head while it is the selected slot (stress50: one
GPU seat, not glued to `s00`).

### Grid

A MultiBox submode (the toggle sits behind MultiBox, unreachable until it
is on). The Game pane is divided into equal cells — one per member, each
the largest 765:503 box that fits its slot — row-major, with `cols` chosen
to maximise the cell area. Clicking a cell selects that member; capture
reaches **only** the focused cell; the queue card overlays it while queued.

### Chooser

A fitted **Profiles** window (same one as the strip **Profiles** button
and rail **+ add bot**), not a blocking modal. Clicking a row focuses it
(and loads it onto the wall while MultiBox is on; stays open for more).
Single-bot pick closes the picker. **Load all** is MultiBox-only.
**Close**/Esc closes without loading. **Edit** (next to **✕**) shows
username/password; **New profile** is a blank edit. The editor stays on
the profile it opened on while other rows are loaded or focused. **Save**
upserts and selects; changing the username renames the profile. A new
profile or a rename onto a username another profile already has is
refused with an error and nothing is written. The row **✕** deletes the
**vault profile only** — a live wall member keeps running and stays on
the rail (Save re-creates the row).
The rail tile **✕** is the opposite: it removes the member from the wall,
arms a clean IF logout when ingame, stops the slot, and never touches
the vault. A member removed and loaded again while its old worker is
still stopping starts once that worker has exited, with any Log in or
Log out issued meanwhile.

### Resource honesty

The resource card is the operator measurement surface: **bots**, **CPU**,
**RAM**, **traffic**, and **draw**, sampled once a second. These are live
samples for the current process, not published performance guarantees or
historical benchmark claims. The first CPU and traffic samples read
"measuring…". Traffic is the sum of each live slot’s `ClientStream`
payload `bytes_in + bytes_out` over that second — never a fake `0 B/s`
before two samples, and never `0 B/s` when there are no slots (still
measuring…). If a slot drops (or the byte sum shrinks), traffic
**re-baselines** instead of inventing a wrap spike. A failed process sampler
shows "monitor error" for CPU/RAM; it does not invent traffic. Process RSS is
the whole host; on macOS the
RAM row is **peak** (`ru_maxrss`). Draw is the focused slot’s `game_draw`
enters plus paint/skip counts. `BOT_DEBUG=1` also prints 1 Hz loop vs raster
timings per slot and the RSS sample. Draw-off **detaches** the head, so
unheaded slots hold only their mutable sim + the shared decode pile;
the RSS ladder is the measurement surface and it prints `rss=…` — it
never fails on size.

## Headed live

`--live` needs no vault passphrase (it makes a throwaway vault). FAIL prints and exits 1. On PASS
the window stays up and remains interactive (operator may click). Local
engine required.

### `--live null_raster`

Two-slot headed twin of the headless e2e test. Waits until both slots are
`ingame && scene_state==2`, prints RSS/counters, PASS. The 3 s
unfocused-`game_draw` freeze is the **headless** twin only.

```bash
cargo run --release -p panel --bin panel-play -- --profile local-289 --live null_raster
# or: BOT_LIVE=null_raster cargo run --release -p panel --bin panel-play -- --profile local-289
```

Headless twin:

```bash
LIVE=1 cargo test -p host-play --test null_raster -- --ignored --test-threads=1
```

### Headless `rss_ladder`

Measure-then-cut: N=1, then 2, then 4 headless Clients, **every** slot
`set_draw(false)`. One N per process. Names `r0`…`r{N-1}`. Wait scene 2
(180s), hold 10s, print Darwin/Linux peak RSS plus `ondemand=` worker
count and unique ESTABLISHED TCP to the engine port. Does **not** fail
on RSS size. FAIL if `rss=0`, if OnDemand workers ≠ 1, or if TCP exceeds
n+1 (game + one update socket). This ladder is **Null / draw-off**
clients. It does **not** measure Started JS isolates (64 MB heap **cap**,
grows from small; wall isolate RSS is unmeasured alpha — see
[`script.md`](script.md)).

```bash
LIVE=1 RSS_N=1 cargo test -p host-play --test rss_ladder -- --ignored --test-threads=1 --nocapture
LIVE=1 RSS_N=2 cargo test -p host-play --test rss_ladder -- --ignored --test-threads=1 --nocapture
LIVE=1 RSS_N=4 cargo test -p host-play --test rss_ladder -- --ignored --test-threads=1 --nocapture
```

### `--live stress50`

Fifty-slot **flat** wall (temp vault `s00`…`s49`, password = username).
Every member is a full `Client` — no lean extras, no channel-head.
Chooser closed, only-render-selected (rail caps, no 50 blits), Game pane
at focused 50 fps, rail skip-paint. Focus `s00` (FIFO head), scatter-seed
after scene 2, then `login_all`. **Release RAM check** — debug 50-heads
spike RSS and look frozen. Timeout **600s**. Announces at 1/50 and 10/50
up (`ingame && scene_state==2`); at 50 prints
`PASS: live stress50 rss=… up=50/50 ondemand=… tcp=…` and **keeps the window up**. Does
**not** fail on RSS size. FIFO login bursts up to 30 per 60 s (production
address cap); a 50-head run still waits out the rolling window.

```bash
cargo run --release -p panel --bin panel-play -- --profile local-289 --live stress50
# or: BOT_LIVE=stress50 cargo run --release -p panel --bin panel-play -- --profile local-289
```

### `--live stress50_full`

Same 50-head wall as `stress50`, but **every** member paints: only-render-
selected off, live full-rate overlay so Game **and** sidecar run at 50 fps
(GPU / lowmem defaults). Run after `stress50` holds. Still `--release`.
PASS line is `PASS: live stress50_full rss=… up=50/50`.

```bash
cargo run --release -p panel --bin panel-play -- --profile local-289 --live stress50_full
# or: BOT_LIVE=stress50_full cargo run --release -p panel --bin panel-play -- --profile local-289
```

## Amber

Accent color **`#FFB000`** (hover `#FFC14D`) — amber CRT over Theme::Dark
(title bars, tabs, frames, buttons). Not default imgui blue, not rs2b0t
green `#04A800`. Panel background `#111`.

## Chrome

Right strip is **330px**-class. Section headings are collapsible (persisted
per profile in `panel-ui.json`; **script** / **parameters** default closed)
and **drag-reorderable** (order in `panel-ui.json`). Default order:
**status**, **profile**, **script**, **parameters**, **debug**, **log**.
Login / WalkTo / config stay at the top.

**Log in** / **Logout** sit above WalkTo (always shown; disabled while
the vault is locked or no profile is focused. Logout also needs the
focused slot ingame). WalkTo is full-width under that row, then
**General config** / **Nav config** / **Loadouts** (wraps so labels are
not clipped). **Profile** is below those buttons and above **debug**
when the local-engine heading is shown: a black combo with orange current
name and a black-on-orange dropdown arrow, then a full-width **Profiles**
button (same window as MultiBox **+ add bot**).
Username/password only appear when **Edit** or **New profile** is used
in the picker. **Auto-login on title** is per-profile, under General
config → slot.

**Log** is **per client thread** (per username status-transition lines),
not one concatenated process log. When nothing is focused the view shows
the `PROCESS` key.

**Script** Browse / Load / Reload / Start / Pause / Stop are wired
([script.md](script.md)). Load is enabled except while a script is active.
**Reload** hashes the selected File/catalog card; unchanged origins report
“Nothing changed; nothing to reload” and skip transpile; confirm restarts
matching running bots and **Stops** matching paused bots. Browse’s
**Refresh catalog** re-scans the catalog root (“Nothing changed.” when the
scan is identical) with the same confirm/restart/stop policy when owners
are affected. MultiBox rail **Start all / Start selected / Stop all** are
script bulk controls (not login): Start all and Start on marked bots start
one bot per frame instead of all at once, waiting rows show `queued k/n`,
and Start selected with a heading card starts that card on every marked
row. One running report (banner and fleet report) counts every bot once as
started, queued, skipped or failed while any bot of it still waits or sets
up, so a second click joins it instead of hiding the first click's results;
each skipped or failed bot also gets a line in its log. Stop and Stop all
cancel bots that are still waiting (Stop on marked rows reports them as
cancelled); Stop all also stops running and paused wall members and live
slots.

The **Fleet** window (the button beside WalkTo) is the panel's view of the
shared marked selection: one row per vault profile with a checkbox mark,
filter and sort. Its action bar runs on the marked rows only and each action
puts one report in the window, counting every marked bot once with failures
first: **Start selected card**, **Stop**, **Assign selected card** (saves the
card without starting; running bots are skipped), **Assign & restart…** (a
confirmation names the bots whose running script it stops; the Starts are
paced like Start all), **Log in** / **Log out**, **Walk N to…** (opens the
WalkTo picker in group mode; each marked bot walks to the chosen tile from
its own position, and bots that are logged out, have no position or run a
script are skipped with that reason) and **Apply focused bot's settings to
marked…** (the same command as the TUI palette: the focused bot's parameters
for the selected card go to the marked same-card bots only, with a
confirmation that names the skipped marked bots and counts the unmarked ones).
Checkboxes under the buttons toggle the status columns: world and login
state, card, run state (the script error on hover), runtime, last log line,
time since the last progress (the watchdog's gameplay clock; `—` while
paused, out of game or idle) and levels gained since the run's Start (the
per-skill breakdown on hover). The choice is saved in `panel-ui.json` as
`fleet_columns` (only the toggles you changed). A hidden column, or a closed
window, reads nothing from the host.

Parameters **Edit** is live for a loaded card with a settings schema
(typed editors honour `showIf` / `group`; File Load parses
`export const SETTINGS` with no V8) and writes the focused profile’s vault
settings bag. Start-only keys apply on the next Start; live edits reach a
matching running or paused isolate without restart. Uncollapse shows
merged rows, or `(no parameters)` when the schema is empty. Successful
Start persists the per-profile script assignment. **Nav config**
is live (debug paints / labels / FindOptions toggles) as its own
non-blocking window. **General config** (under WalkTo, above profile) is
**slot** (capture, auto-login on title), **render** (none/GPU/CPU; click
the lowmem/highmem button for a sticky picker like Teles), and **global**
(sidecar 50 / only-render-selected). **Loadouts** is a live window (CRUD
presets; `optionsFrom: 'loadouts'` combos in Parameters). Text and buttons
wrap or equal-width-squish — **no horizontal scroll**.
`chrome.rs` keeps the section inventory (`wired: bool`). Title (MultiBox
is a live toggle), dim **build line** (`beta 1 ·` git short SHA,
`-dirty` when the tree was dirty; hover is crate version then full
commit + built time), banner, profile, **debug** (loopback), and status
key/value rows (including **mem**: highmem/lowmem) fill out the strip.

**WalkTo** (title row) fills the Game pane: north-up terrain from the
process-wide map-cache images demand (catalogue first; real bake progress
in the footer) or a grid and `map imagery unavailable — cache not bound`
until that cache is ready. Opening WalkTo starts the demand against
`~/.274bot/map-cache` for the bound profile; closing it releases the
demand and GPU/CPU pixels. Release packages ship the 289 terrain baked
from the pinned client cache (`map/289/` beside `nav/289/`); when its
image identity matches the bound client cache exactly, the first open
installs it into the map cache (progress `installing shipped terrain`)
instead of baking, without asking. When no ready or shipped terrain
matches (a local bake would run), the map first
shows the bake warning from `frontend_core` (CPU for about 15 s, up to
~15 MiB once) with **Bake now**, **Always bake** and **Not now**; it
stays catalogue-only until the operator bakes (**Bake terrain** after Not
now). Shipped terrain that fails verification is never installed and
leads to the same warning. A ready cache, baked earlier or installed,
opens without asking.
**Always bake** and Nav config's **ask before baking terrain** set the
shared `map_bake` key of `panel-ui.json` (`ask` when absent). POIs come from `Catalogue::from_ready` (client
records, authenticated `navpois`, game-data names) plus live
`observed_services` for the focused bot. Search uses that catalogue.
Place names keep `/` as a stored world-map line break (`Port/Sarim`); lists
and canvas labels show a space (`Port Sarim`). Canvas labels are ranked by
content `priority`, then kind, then distance to the view centre, and a label
is omitted when its text rectangle would overlap an already placed one
(markers still draw; at most 32 labels). Search still splits on `/`.
Optional map-owned overlay toggles (not per-tile quads), vector route
and destination (pending selection vs armed dest are distinct), wheel-zoom toward
the cursor, click-to-pick uses the canvas rect (`is_mouse_hovering_rect`),
header plane/zoom/search/layer controls that wrap at the default Game pane
and narrower widths, footer **Route through danger zones** / **Recentre** /
**Walk** / **Send**, and **Teleport** (Local target on a loopback host,
guarded by `host_play::walk_map`). Walk needs a snapped walkable target. The
zone checkbox starts unchecked and resets every time WalkTo opens; its hover
text is `Allows routes past monsters that may kill your bot.` It applies only
to that WalkTo's focused or group Walk confirmation and is not saved in Nav
config. On a legacy grid pack the toggle cannot enable zone checks, and the
report says `zones: unavailable (legacy grid pack)`.
The map selection is a destination (tile/POI, plane, nav identity), not a bot.
**Send** chooses **Focused bot** (default) or **Group** — a checklist of
wall bots with **All eligible / None**. Ineligible bots stay listed,
greyed, with a host reason (not logged in, no position yet, running a
script). Group confirm reads **Walk N bots**; each eligible bot gets its
own command and a Start-all-style summary. A refusal names the dangerous
zones for each bot that could not route. Debug **Teleport** uses the
clicked tile even when blocked, stays focused-only, and is enabled
whenever there is a selection and Teleport is authorized. Confirmations
consume the pending `MapModel` selection once and act on the bot focused
at confirm; missing origin or a running script refuse instead of storing
a later login dest. A different nav pack invalidates the destination.
Status `walk` mirrors the armed dest.

## Headless proof

The same wiring is exercised live without a window in
`crates/e2e/tests/panel_view.rs` (renderer pixel proof + capture walk):

```bash
LIVE=1 cargo test -p e2e --test panel_view -- --ignored --test-threads=1
```
