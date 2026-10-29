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
(`--profile NAME` or `--rs2b2t`; see [server-profiles.md](../reference/server-profiles.md)).
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
vault** writes the bound profile's vault (`~/.274bot/vault-289` for `local-289`; see [vault.md](vault.md#which-vault-file)) **empty** — panel-play does **not**
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

After unlock, the next preference-ready poll reconciles saved profile order
before restoring `last_focus` from `~/.274bot/panel-ui.json`.
An earlier explicit selection keeps its focus instead.
Preference adoption during a command never changes that command's target.
This includes the first bot selected by Fleet **Log in**.
Removing or deleting a pending restore target cancels that restore.
Before preferences identify the saved target, any removal cancels the restore.
Collapsible section open/closed state persists there per profile.
**script** and **parameters** default closed.

## Log and panel layout

The structured **Log** section remains inline in the panel. The title-row
**Log** button opens the same view as a dockable **Log** tab beside the panel
and other panel windows; the tab's **X** closes it, and dragging it out
undocks it into a floating window. Closing the tab does not move or hide the
inline section. When the Log window draws its body, the inline section shows a
one-line hint instead; an open but unselected docked tab leaves the inline
body visible. Both views share filters, follow state and retained history. An

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

The Status card shows what the selected bot's script is doing and why it
waits: the script line, its phase, and the reason when the script is parked.
A Quester that finds no step for its stage parks and shows `no step for
stage`. A bot that runs no script shows no script rows; a script refusal
still showing after its run stopped keeps its rows.

## Wiring

`Session::unlock` installs an empty `Play` (shared `Arc<Cache>`, login FIFO).
It does not load or select a profile. A later poll waits for the unlocked vault
and durable preferences, then reconciles saved profile order before choosing
the restored `last_focus` or the first profile. It loads only that profile.
Parked profiles receive no `Client` until selection. Loaded clients remain
available when focus changes.
Each running slot has its own `PixelBuf` + `SlotInput`. A per-frame
observe hook applies the focus `set_draw` switch and the mainland hop.
The runner is configured with **docking on, multi-viewports off** (single
main viewport). Default dock: **game left**, panel right. At 100% scale the
panel is **330 px** wide and the MultiBox rail is **264 px**; both keep that
logical width as the window grows vertically. The rail's tile body is
**236×155 px** at 100% scale. Splitters, undock, the tab-bar corner menu,
and imgui.ini restore are off.
Only grid mode fits the game blit to the resizable pane; single-bot and rail
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

The Game Image is an ImGui **wgpu texture**. The game scene behind it is
the client submodule's **wgpu GPU 3D** renderer by default (CpuPix3D
when `BOT_CPU=1`). That is not the client's `Present` applet
(`client-play --window` on bothost). Unfocused / renderer-off slots skip
`game_draw`. Slots that draw without a 50 fps knob run the **1 fps**
watch cadence; capture never raises fps.

Run **`--release`**. A default debug Pix3D pins a core. One live client
still holds on the order of a gigabyte (process-wide model/anim stores +
scene); that is the client painter, not the ImGui chrome.

## Renderer vs capture

274bot defaults to **lowmem** (`Profile.settings.lowmem = true`). The
checkboxes on a focused profile:

- **game renderer** — default **on**. Checking it after off paints
  **this tick** (no cold wait). Frame cadence is three separate knobs:
  **focused 50 fps** raises the Game pane slot, **sidecar 50 fps** raises
  unfocused wall members, and the ephemeral **full rate (this run)**
  overlay raises every drawing slot (below); any other drawing slot runs
  the 1 fps watch cadence. Capture (below) never raises fps. The Game
  Image is an RGBA8 texture that is **never below 765×503** (native,
  non-grid). Grid tiles `fit_applet` into their cell.
  Rendering never pauses the bot. Renderer-off /
  `set_draw(false)` detaches the head — GPU textures, chrome, and the
  decoded 3D scene are freed — while `mainloop` and collision keep
  running; flipping the renderer on reattaches 3D from the same map
  bytes on the **same** client (never a logout or restart).
- **sidecar 50 fps** — unfocused wall members at 50 fps. Interactive /
  non-test. Scenarios never flip it. Both 50 fps checkboxes are saved in
  `panel-ui.json` (`sidecar_50` defaults off, `focused_50` on); a restored
  setting costs nothing for a slot that isn't drawing.
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

A 264px sidecar on the far right (`RAIL_W`). At its top is the [fleet
toolbar](#fleet-toolbar) under a **Fleet** header; the toolbar stays in
place while the region below it scrolls on its own. That region starts with
the **only render focused** checkbox, then one tile per shown wall member,
**Profiles…**, and the 1 Hz resource card. The
tiles follow the toolbar's filter and sort (one order for the rail,
the Grid and the Fleet table; **Load order** by default). In Load order,
drag a tile's name onto another
tile to move that bot before or after it; the new order is saved (see the
Fleet window). Profiles that are not loaded are listed
below the tiles in a folded **Not loaded (N)** group: each has a mark
checkbox and no tile; mark them and press **Log in** in the toolbar to load
and log them in. A tile is
a cap — traffic-light dot (on a server with public worlds a disc that
carries the bot's world number, which opens the [world picker](#world-picker)),
name, **✕** — over a 236×155 body that blits the
member's `PixelBuf` (or a renderer-off placeholder). Clicking a name or
body focuses the member. Filtering never changes focus. The dot is error **red**, then ingame **amber**,
then FIFO-queue **warn**, else grey. **only render focused** mirrors
`Focus.only_render_selected`: when off, unfocused wall members also
`set_draw` (the tile body shows their raster); when on, only the focused
member paints. Unchecking it opens a **Render all wall members?** confirm:
**OK** (once **I understand** is ticked) applies for this launch. **Don't
show again** saves `render_all_warning_ack` in `panel-ui.json` and renders
all only when that save lands: while it is pending the box stays checked and
only the focused member paints, and if it fails the dialog shows the reason
and nothing changes. After the saved acknowledgement, unchecking applies at
once. The box starts
checked on every launch and is never saved. The mark checkbox's tooltip says
"Fleet and Debug actions" on servers that allow Debug and "Fleet actions"
elsewhere. The Game pane always follows **focus** — a rail-Off
member still grows a head while it is the selected slot (stress50: one
GPU seat, not glued to `s00`).

### Grid

A MultiBox submode (the toggle sits behind MultiBox, unreachable until it
is on). The Game pane is divided into equal cells — one per shown member,
in the Fleet window's filter and order, each
the largest 765:503 box that fits its slot — row-major, with `cols` chosen
to maximise the cell area. Clicking a cell selects that member; capture
reaches **only** the focused cell; the queue card overlays it while queued.
The rail is hidden in Grid, so the right-hand panel shows the same 1 Hz
**resource** readout there (the one sample the rail card reads; the panel
section stays hidden only while the rail is shown), and the [fleet
toolbar](#fleet-toolbar) in its **fleet** section.

### Fleet toolbar

One toolbar carries every fleet action. The rail draws it at its top under
a **Fleet** header; while the rail is hidden (Grid, or MultiBox off) the
right-hand panel draws it in its **fleet** section (a heading like the
others: drag it to reorder, turn it off under General config → Panel); the
Fleet window draws it above its table. Every copy shows the same card,
marks, filter, order, waiting plan and result, and every click goes through
the same core commands. Every decision behind it (the card, the targets,
what is skipped, whether to ask first, the prompt and the result text) is
made in `frontend-core`; the panel only stages and submits.

A click on the **Fleet** header (in the panel: the **fleet** heading)
folds the toolbar to one summary line such as `5 marked · Gatherer`, and
the filter while one is set. A waiting plan and the last result stay
visible under it. The choice is saved in `panel-ui.json` as
`fleet_toolbar_open`, apart from the tile folds. In the rail the toolbar
stays in place while the tiles under it scroll.

- **Card** and its drop-down set the fleet card, the card the
  Start, Assign, Assign & restart and Copy settings buttons use. Its entries
  are **Each bot's own card** (the default), **Focused bot's card: X** (it
  follows the focus and reads `(pending)` while the focused bot's Browse
  pick is not yet its assignment) and then every Browse card, grouped by
  category in the Browse window's category order. A search field in the
  open list narrows the entries. Picking a card changes only the fleet
  card: no assignment, no Browse pick, no profile form change, and nothing
  starts.
- The **marks row** has a tri-state box over the shown rows, a **Marks**
  menu and a count such as `5 marked, 2 not shown`. With no shown row
  marked, a click on the box marks the shown rows; with some marked, it
  marks the rest; with all marked, it unmarks them. The Fleet table's Mark
  column header is the same box. The menu has **Mark shown**, **Unmark
  shown**, **Invert shown** and **Mark by status** (ready, idle, running,
  logged out, login error, not loaded). These act on the shown rows only; a
  mark the filter hides stays. Below them **Move marked to top** moves the
  marked profiles to the top of the saved Load order (only in Load order).
- The **filter** (name, status or world such as `w3`; every word must
  match) and the **sort** drop-down (**Load order**, **Name**, **Status**)
  share a row while the filter keeps room, else they stack.
- Nine **action buttons**: **Start**, **Stop**, **Assign** (saves the card
  without starting), **Assign & restart**, **Log in**, **Log out**,
  **Walk** (opens the WalkTo picker in group mode; each bot walks to the
  chosen tile from its own position, and bots that are logged out, have no
  position or run a script are skipped with that reason; with nothing
  marked, group mode marks the loaded shown bots that can walk, never a bot
  the filter hides), **Switch world** (opens
  the [world picker](#world-picker)) and **Copy settings to**. Each label
  names its scope: `Stop 3` with marks, `Stop all 12` with none. With marks,
  an action covers every marked profile, hidden marks included. With no
  marks it covers the loaded shown bots and asks first. A button is disabled
  when it has no bot to act on. While the fleet card is **Each bot's own
  card**, **Assign**, **Assign & restart** and **Copy settings to** are
  disabled with the tooltip `pick a card above`; Start then runs each bot's
  saved assignment. The buttons share rows in equal columns while the widest
  label fits, else pair up two per row, else take one row each; a label is
  never cut and nothing scrolls sideways. The count-bearing labels are
  rebuilt only when the fleet model changes, not every frame.

A click on an action button plans the action. The core freezes the target
bots, the card, the bots a run would interrupt and the rows it skips, and
refuses with a reason (shown on the banner) when there is nothing to do. A
plan that needs no confirmation runs at once. A plan asks first when it
covers every shown bot, interrupts a running script, copies settings or
switches world.
Then a **plan row** under the buttons shows the core's prompt, wrapped,
with **Confirm** and **Cancel**. Cancel writes nothing. Only one plan
waits: a newer action replaces it and its prompt starts with `Replaces the
waiting ... plan.` A run acts on the frozen bots: a profile marked or
loaded after the plan does not join it, and a bot that changed state is
skipped with its new reason. With two toolbars on screen (the rail and the
Fleet window) both show the plan; the first Confirm runs it once.

Each run ends in one **result line** under the plan row: failures first,
then cancellations (each with its reason when one is known, such as
`a later Relog now took over` or `the automatic login guard stopped
repeated logins`; otherwise `cancelled`),
then skips, then done, with counts. It reads as pending until every member
has really finished (a paced Start counts its queued bots until they
start or fail). The full text is in a tooltip, and the core logs one line
when it settles. The banner does not carry a second copy of it.

### World picker

On a server with public worlds (rs2b2t) a cap's status dot is a disc with the
bot's world number, and it is a button. A click on it (in the rail or on a
Grid cell), a click on a loaded bot's **World / login** cell in the Fleet
window (its tooltip reads `click to switch world`), or the toolbar's
**Switch world** button opens the one world
picker: a small popup at the pointer that lists the server's worlds, with the
bot's own world ticked. The list is the server's world list as it was read
when the session started, so opening the picker fetches nothing and reads no
file. Click away or press `Esc` to leave it as it was.

Picking the world the bot is already on closes the picker and does nothing.
Picking another world plans **Switch world** for that bot alone (a bot that
is marked elsewhere does not join). The plan row of every fleet toolbar on
screen shows the core's prompt with **Confirm** and **Cancel**, as for every
fleet plan; when no toolbar is on screen, the Fleet window opens (or comes
forward) to show it. The toolbar's **Switch world** button plans over
the marked bots, hidden marks included, or over the loaded bots shown when
none is marked; its prompt names the bots it skips, such as one already on
that world. A Switch world plan always asks first. Nothing is saved or
relogged before **Confirm**, and **Cancel** writes nothing. On a server with
one world the button is disabled with the tooltip `this server has one
world`.

A confirmed switch saves the bot's world on its profile and only then logs
the bot out and back in on the new world through the login queue, so several
bots relog at the queue's pace, not all at once. A save that fails leaves
the bot logged in. A running script continues after the bot reconnects; a
script that was already paused stays paused. A bot that is queued for login,
logging in, logged out or not loaded is refused with its reason (`Switch
world: no bot to act on (alice: queued for login)`, shown on the banner).
The result line reports each bot's real outcome, not only that the relog
started.

On a server with one world (a local server) the dot stays a status dot, the
World / login column is plain text and the **Switch world…** entry is
disabled with the reason `this server has one world`.

### Chooser

A fitted **Profiles** window (same one as the strip **Profiles** button
and rail **Profiles…**), not a blocking modal. Clicking a row focuses it
(and loads it onto the wall while MultiBox is on); the window stays open
either way, with any open edit form as it was. The focused profile's own
script card shows, and no script starts. **Load all** is MultiBox-only.
Unlocking the vault while MultiBox is already on focuses the remembered
(or first) profile the same way, so it joins the rail.
**Close**/Esc closes without loading. **Edit** (next to **✕**) shows
username/password; **New profile** is a blank edit. The editor stays on
the profile it opened on while other rows are loaded or focused. **Save**
upserts and selects; changing the username renames the profile. A new
profile or a rename onto a username another profile already has is
refused with an error and nothing is written. A new profile with an empty
password is refused with `credentials: password required` and nothing is
written. The row **✕** deletes the
**vault profile only** — a live wall member keeps running and stays on
the rail (Save re-creates the row).
**Import from rs2b0t…** (vault unlocked) reads rs2b0t's
`rs2b0t-profiles.json` export: a path field, then a preview with Skip or
Overwrite for profiles already in the vault (and, when the file has
accounts the 289 client cannot log in with, a box to import those too),
then **Import** and a report that offers to delete the export file (with
an I understand confirm) once every chosen account is confirmed saved. See
[vault.md](vault.md#importing-profiles-from-rs2b0t).

Layout: **Load all** (MultiBox), then **New profile** and **Import from
rs2b0t…** on one row; under them the profile list and the editor pane,
each scrolling on its own, with a draggable divider between them; then
**Close**, which always stays in view (the window itself never scrolls).
The editor pane holds the edit form (its **Save** / **Cancel** and the
save notice stay under the scrolling fields) or the import, one at a
time; with neither open the list takes the whole area. From a content
width of 502 logical px (the form's narrowest, 314 px — the 330 px
docked panel less its padding — plus the 8 px divider and a 180 px
list) the list sits on the left and the editor on the right; narrower,
as in the default 330 px dock, the editor sits under the list. Opening
the import while the form has unsaved edits (or a save still being
written) asks **Discard** / **Keep editing** first; opening an edit or a
new profile while an import preview shows drops the preview (it has
written nothing). While an import's writes are being saved, **New
profile**, **Edit** and the report's **Done** wait until they settle.
The rail tile **✕** is the opposite: it removes the member from the wall,
arms a clean IF logout when ingame, stops the slot, and never touches
the vault. A member removed and loaded again while its old worker is
still stopping starts once that worker has exited, with any Log in or
Log out issued meanwhile.

### Resource honesty

The resource card is the operator measurement surface: **bots**,
**background**, **CPU**, **RAM**, and **traffic**, sampled once a second. These are live
samples for the current process, not published performance guarantees or
historical benchmark claims. The first CPU and traffic samples read
"measuring…". Traffic is the sum of each live slot’s `ClientStream`
payload `bytes_in + bytes_out` over that second — never a fake `0 B/s`
before two samples, and `no live slots` when there are no slots. If a slot drops (or the byte sum shrinks), traffic
**re-baselines** instead of inventing a wrap spike. A failed process sampler
shows "process sample failed" for CPU/RAM; it does not invent traffic. Process RSS is
the whole host; the RAM row reads the current resident size plus the lifetime peak
(`<now> process, peak <peak>`, falling back to peak-only when resident is unavailable).
`BOT_DEBUG=1` also prints 1 Hz loop vs raster
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
grows from small; wall isolate RSS is unmeasured — see
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
**status**, **resource**, **profile**, **script**, **debug**, **log**; **parameters**
is drawn after them and is not reorderable.
Login / WalkTo / config stay at the top.

**Log in** / **Log out** sit above WalkTo (always shown; disabled while
the vault is locked or no profile is focused. Log out also needs the
focused slot ingame). WalkTo is full-width under that row, then
**General config** / **Nav config** / **Loadouts** (wraps so labels are
not clipped). **Profile** is below those buttons and above **debug**
when the local-engine heading is shown: a black combo with orange current
name and a black-on-orange dropdown arrow, then a full-width **Profiles**
button (same window as MultiBox **Profiles…**).
Username/password only appear when **Edit** or **New profile** is used
in the picker. **Log in automatically** (saved as
`ProfileSettings.auto_login`) is per-profile, in that edit form.

Non-toggle window buttons — **Profiles** / **Profiles…**, **General
config**, **Nav config**, **Loadouts**, **Script prefs**, **Log**,
**Fleet** (and Debug's **Open Fleet**), **Panel** / **maxme** (Debug),
**Browse…**, **Load**, **Import catalog…** and Fleet's **Walk N marked bots** —
open a closed window where it first appears (docked or floating as
before) and bring an already-open one to the front, selecting its dock
tab. The focus is requested once per click (`Session::open_window`), never
every frame; an open Loadouts or file browser keeps its selection, draft,
folder and search. **WalkTo**, **MultiBox** and **Grid** stay toggles.

**Log** is **per client thread** (per username status-transition lines),
not one concatenated process log. When nothing is focused the view shows
the `PROCESS` key.

 **Script** Browse / Load / Reload / Start / Pause / Stop are wired
 ([script.md](script.md)). Browse lists compiled native cards
 (Gatherer, Grinch, Quester, plus Sherlock in builds with the JS runtime) first, then loaded JS/catalog cards.
 Sherlock is available only in builds with the JS runtime. Sherlock was meant to be
 native. In practice it became a port of the compatibility clue machine, so it needs the
 JS runtime build until the native clue rewrite replaces that machine. Sherlock itself
 never creates a V8 isolate. Load is enabled except while a script is active.
**Reload** hashes the selected File/catalog card; unchanged origins report
“Nothing changed; nothing to reload” and skip transpile; confirm restarts
matching running bots and **Stops** matching paused bots. Browse’s
**Refresh catalog** re-scans the catalog root (“Nothing changed.” when the
scan is identical) with the same confirm/restart/stop policy when owners
are affected. The [fleet toolbar](#fleet-toolbar)'s **Start** starts one
bot per frame instead of all at once, and waiting rows show `queued k/n`.
Its result line counts every bot once as started, queued, skipped or
failed and reads as pending while any bot of it still waits or sets up.
Each skipped or failed bot also gets a line in its log. The toolbar's
**Stop** cancels bots that are still waiting and reports them as
cancelled.

A script's paint draws over the Game chatbox of the focused bot, with its
buttons below the text. A script that uses the standard paint-controls helper
(`ScriptRunner.paintControls`) gets working **Pause** and **Stop** buttons
there. **Pause** reads **Resume** while the script is paused, also on the
frame the script painted before it paused, and the buttons are disabled while
the run is stopping or idle. They issue the same commands as the toolbar's
Pause and Stop, for the bot and run the paint was drawn for: a click is
refused, not moved to another bot, when the focus or the run changed before
it was applied ([script.md](script.md)).

Saved settings that need a restart show a `[restart]` badge and open a
prompt. **Restart** applies only to listed bots still running the same
assigned script with its pending settings. Bots that stopped, logged out,
were reassigned or run a different script are skipped and named in the
status line. **Later** leaves their current scripts running and suppresses
the prompt until settings change again.

The **Fleet** window (the button beside WalkTo) is the wide view of the
shared fleet model: the [fleet toolbar](#fleet-toolbar) over a table with
one row per profile, loaded or not (a profile that is not loaded reads `not
loaded`, distinct from a loaded bot that is `offline`), each with a checkbox
mark. The rail and the Grid show the same filter and order. A mark stays
when the filter hides its row or the profile is unloaded; it is dropped
only when the profile is deleted. A click on a loaded bot's name focuses
it, as a click on a rail cap does; its checkbox only marks it, and a click
elsewhere in the row does neither.
**Load order** is the saved order: drag a row's name onto another row, or
choose **Move marked to top** in the toolbar's **Marks** menu (it keeps the
marked profiles' order), and the
new order is saved as `profile_order` in the shared preferences. A profile
the filter hides keeps its place. Load all logs in bots in this order, a new
profile goes to the end, a renamed one keeps its place and removing a bot
from the wall keeps its place. **Name** and **Status** only sort the view:
they never change the saved order, and reordering needs **Load order**. A
reorder stays pending until the file is written; if the write fails, the
error is shown and the last saved order comes back.

Shared preferences still use `panel-ui.json`. Reads use the latest cached
saved state. Saves run on the process writer, so a slow disk does not block
the panel, TUI or bot thread. Closing a settings window does not cancel an
accepted save. Reopening it retains the pending save. Until the first read
finishes, displayed defaults are provisional and walk permissions stay closed.
Refresh preserves unsaved checkbox and folder entries when their saved values
did not change.

A saved operational choice (capture input, the sidecar and focused 50 fps
switches, the walk and danger permissions, the manual-walk pause, the map-bake
choice, the Quest Paths source, the session log file, and "Don't show again" on
the notices) takes effect only from the saved state. So do the remembered
launch choices: the server revision, the Game pane's none/GPU/CPU raster and
its highmem/lowmem mode. Until the save lands, the profile's saved raster and
memory mode, the slot's GPU/CPU latch and the revision a session binds with
stay as they were, and the control shows your choice. While its save is pending
a control shows your choice with **Saving…**. If the save fails, the control
shows the saved value again and the reason (for example "Not saved: …") appears
beside it. The reason leaves when you act on the setting again, or after 15
seconds. A failed "Don't show again" keeps its dialog open with the reason
under the buttons. The state belongs to the session, so closing and reopening a
window shows it again. A change another program copy saved reaches every
control each time it saves (a control you are typing in, or one whose own save
is still pending, keeps what you typed or chose until its save ends). The
raster and memory controls follow it while no bot is focused; with a bot
focused they show that bot's own saved modes. When focus ends (for example
you remove the last focused bot), they show the saved value at once, also a
value another copy saved while the bot was focused. The server revision follows it
while no profile is bound; a bound session keeps the revision it bound with.
A pending raster or memory
choice, its **Saving…** and its reason belong to the bot it was made for:
focus another bot and its controls show that bot's values, and a choice made
there is that bot's own. When several saves of one setting end at once, the
newest raster or memory choice takes effect, and an older save that landed
never clears a newer one's failure. Layout, colour, order, fold and
remembered-folder choices show at once and go back to the saved value if their
save fails; each shows **Saving…** and the reason at its own control (the
section order, the category chips, the folds, the rail and Grid preview folds
inside their tile, the file dialogs, the Debug tab and the Fleet toolbar fold
included). A failed save is final: the panel never tries it again by itself,
also not after the other program releases the file. Only a new change of
that setting starts a new save. The Browse window saves the category order
only when you drop a chip; a category that the saved order does not name
shows after the named ones and is not saved. With the session
log file on, the Log window also says if the file could not be opened or
written, also after it had opened.

The table's columns share the
window's width, so it never scrolls sideways, and the toolbar and column
toggles wrap at a narrow width.




The Scripts window's **Apply to same-card members** button copies the
focused bot's parameters for its own card through the same plan: it stages
that card, plans **Copy settings to**, and shows the same confirmation in
every fleet toolbar on screen; while no toolbar is on screen, the Scripts
window draws the plan row and result line itself. The copy goes to the
marked same-card bots, or with no marks to the loaded shown same-card bots;
the prompt names the skipped bots.

Checkboxes under the result line toggle the status columns: world and login
state (on a server with public worlds a loaded bot's cell, such as `w2
ready`, opens the [world picker](#world-picker)), card, run state (the
script error on hover), runtime, last log line,
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
merged rows, or `(no parameters)` when the schema is empty. Successful Start
persists the per-profile script assignment.

Grinch settings use this same Script prefs editor and per-profile vault bag.
Its saved settings require a restart; the running copy keeps its prepared
settings until the next Start. The new Start validates the saved choices and
captures a new anchor. Grinch's `radius` is 1–16 tiles around that exact
same-plane anchor, with the boundary included; live-zone filtering also
rejects stale rows. See [Grinch ground-item collection](script.md#grinch-ground-item-collection)
for its defaults, selected-only bank trip and status contract.

**Nav config** is live as its own non-blocking window (Routing, Display, Path paint, Debug, WalkTo map
groups): the durable walk permissions — teleports, wilderness
and bank fetch (all default off) plus **Danger routing**, a full-width drop-down
(Never, When survivable (the default), Always; When survivable shows the note
`Not available yet: acts as Never.` under it and admits routes as Never) — **Pause script on
manual movement** (default on; manual movement pauses any running rs2b0t
script, or a built-in script with an active walk or native action), and debug
paints / labels. A script allows danger zones for its
own walks in its Script prefs Walk permissions; a single WalkTo walk is allowed
with its own checkbox (below). WalkTo map holds **ask before baking terrain**.
The Quest Paths folder loader (folder, reload) sits under the Quester card's
parameters in Script prefs and applies to every bot. **General config** (under
WalkTo, above profile) is
**slot** (capture), **render** (none/GPU/CPU; click
the lowmem/highmem button for a sticky picker like Teles), and **global**
(sidecar 50 / only-render-selected). **Loadouts** is a live window (CRUD
presets; `optionsFrom: 'loadouts'` combos in Parameters). It saves through
the process local-state service, the same writer the bots read: a save is a
typed edit per changed row against the list it loaded, a change made
meanwhile (another 274bot on the same home) is reported as a conflict and
the edit kept, and a rename is one edit that keeps the loadout's place in
the list (a rename onto another loadout's name is refused). A
`loadouts.json` whose loadouts share a name (an older editor could save
one) is renamed apart once, when it is read and before anything is leased
or saved: its exact bytes are copied to `loadouts.json.duplicates-<time>`,
then the second loadout of a name becomes `<name> (2)`, the third
`<name> (3)`, and so on (a name another loadout has is skipped), each
keeping its gear and its place, with a warning and an audit line in the
log; while that cannot be done the file is refused. Text and
buttons wrap or equal-width-squish — **no horizontal scroll**.
`chrome.rs` keeps the section inventory (`wired: bool`). Title (MultiBox
is a live toggle), dim **build line** (`beta 2 ·` git short SHA,
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
config. While Danger routing is Always, the per-walk checkbox is superseded.
On a legacy grid pack the toggle cannot enable zone checks, and the
report says `zones: unavailable (legacy grid pack)`.
The map selection is a destination (tile/POI, plane, nav identity), not a bot.
**Send** opens a window, on screen and sized to its list (it scrolls only
when the list is taller than the screen), that chooses **Focused bot**
(default) or **Group** — a checklist of wall bots with **All eligible /
None**. The checks are the shared marked-bots set (the rail, Grid and Fleet
marks), so a mark made anywhere updates the **Walk N bots** count at once.
Choosing **Group** while no bot that can walk is marked marks the loaded
bots the fleet filter shows that can walk; **All eligible** marks every
bot of the list.
Ineligible bots stay listed,
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
