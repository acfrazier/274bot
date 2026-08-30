# Panel: native UI (`panel-play`)

`crates/panel` is the native **dear-app / ImGui** UI over the same kernel
slots `host-play` runs ([login.md](login.md), [vault.md](vault.md)).
Single-bot chrome plus the **MultiBox wall** (sidecar rail, grid mode,
profile chooser).
It does **not** reimplement the client UI — there is **no Present**, no client
window feature. The client rasters into a frame (wgpu GPU 3D by default,
CpuPix3D if `BOT_CPU=1`); the panel blits that frame and feeds input back.

## Run

```bash
export BOT_VAULT_PASS=bot
cargo run --release -p panel --bin panel-play
# 50-head RAM:  cargo run --release -p panel --bin panel-play -- --live stress50
# 50-head 50fps Game+sidecar: --live stress50_full
```

A passphrase is required and an empty one is rejected. First run **Create
vault** writes `~/.274bot/vault` **empty** — panel-play does **not**
auto-create `test`/`test` (that is host-play CLI: `--user test` defaults,
`password = username`). A wrong passphrase never replaces the file;
**Reset vault** (confirm + I understand) is the forgotten-password wipe.
Open **Profiles**, **New profile** or **Edit**, type a username/password,
and **Save** to upsert, spawn that slot on the login FIFO, and select it.
Unlike the CLI there is **no
`--vault-pass` flag** — the passphrase comes from `BOT_VAULT_PASS`, or from
the in-panel prompt (which also covers interactive use). When
`BOT_VAULT_PASS` is set, the panel unlocks **before** the window opens so the
headless path works unchanged. There is **no mainland checkbox** in the
panel: `BOT_MAINLAND=1` or host-play `--mainland` still queues
`mainland_hop` after scene 2. On a **loopback** engine the **Debug**
section is shown: **TutSkip** (`setvar tutorial 1000`, hidden once the
profile is known skipped; unknown profiles `getvar tutorial` first),
**Lumbridge** (`~home`), **maxme** (19× `setstat`
99), **Teles** popup, and a disabled **DebugPanel** stub (v2 later).
Public `w1.rs2b2t.com` hides that heading.

Last focused profile is restored from `~/.274bot/panel-ui.json`
(`last_focus`). Collapsible section open/closed state persists there per
profile; **script** and **parameters** default closed.

## Wiring

`Session::unlock` starts an empty `Play` (shared `Arc<Cache>`, login FIFO)
via `host_play::run_with_io`, then selects the restored `last_focus` (or
the first vault name), which spawns **that one slot**. Parked profiles do
not get a `Client` until you select them; once spawned they stay up so the
picker can change focus.
Each running slot has its own `PixelBuf` + `SlotInput`. A per-frame
observe hook applies the focus `set_draw` switch and the mainland hop.
The runner is configured with **docking on, multi-viewports off** (single
main viewport). Default dock: **game left**, **330px panel right**.
Panel and rail widths are **fixed** (330 / 264); they grow **vertically**
with the OS window. Splitters, undock, the tab-bar corner menu, and
imgui.ini restore are off.
**Only grid mode** scales the 274 blit with the window; single-bot and
rail keep a native **765×503** (logical) applet centred in the leftover
pane. DPI is **winit + imgui `HiDpiMode::Default`** — we do not
`ScaleAllSizes` on top of that (it would double Retina).
Single-bot and the MultiBox **rail** hide the dock tab strip
(`AUTO_HIDE_TAB_BAR`, `NO_TITLE_BAR` on the rail). Opening the sidecar
**grows** the OS window if the 765×503 blit would sit under the panel or
rail; a larger window is not shrunk. The Game blit is flush to the panel
(right-aligned in the leftover pane).
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
  button (under none/GPU/CPU) for a sticky picker like Teles. Highmem
  is `Profile.settings.lowmem = false`; switching mem (like GPU↔CPU)
  drops + reattaches the head on the live `Client` — never a logout
  or restart.
- **capture input** — click-through: while on and the Image is hovered,
  local coords stream `InputEv::Move`, mouse buttons send `Down`/`Up`
  (left=1, right=2), and keys go to `InputEv::Key` on that slot only.
  Off means watch-only with zero input work (`tx` is `None`; the slot does
  no `try_recv`).

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
**Login all** / **Logout all**, an **only render selected** checkbox, one
tile per wall member, **+ add bot**, and the 1 Hz resource card. A tile is
a cap — traffic-light dot, name, **✕** — over a 236×155 body that blits the
member's `PixelBuf` (or a renderer-off placeholder). Clicking a name or
body focuses the member. The dot is error **red**, then ingame **amber**,
then FIFO-queue **warn**, else grey. **only render selected** mirrors
`Focus.only_render_selected`: when off, unfocused wall members also
`set_draw` (the tile body shows their raster); when on, only the focused
member paints.

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
username/password; **New profile** is a blank edit. **Save** upserts and
selects. The row **✕** deletes the **vault profile only** — a live wall
member keeps running and stays on the rail (Save re-creates the row).
The rail tile **✕** is the opposite: it removes the member from the wall,
arms a clean IF logout when ingame, stops the slot, and never touches
the vault.

### Resource honesty

The resource card is the operator measurement surface: **bots**, **CPU**,
**RAM**, **traffic**, and **draw**, sampled once a second. The first CPU and
traffic samples read "measuring…". Traffic is the sum of each live slot’s
`ClientStream` payload `bytes_in + bytes_out` over that second — never a fake
`0 B/s` before two samples, and never `0 B/s` when there are no slots (still
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

`BOT_VAULT_PASS` is unused for `--live`. FAIL prints and exits 1. On PASS
the window stays up and remains interactive (operator may click). Local
engine required.

### `--live null_raster`

Two-slot headed twin of the headless e2e test. Waits until both slots are
`ingame && scene_state==2`, prints RSS/counters, PASS. The 3 s
unfocused-`game_draw` freeze is the **headless** twin only.

```bash
cargo run --release -p panel --bin panel-play -- --live null_raster
# or: BOT_LIVE=null_raster cargo run --release -p panel --bin panel-play
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
n+1 (game + one update socket).

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
cargo run --release -p panel --bin panel-play -- --live stress50
# or: BOT_LIVE=stress50 cargo run --release -p panel --bin panel-play
```

### `--live stress50_full`

Same 50-head wall as `stress50`, but **every** member paints: only-render-
selected off, live full-rate overlay so Game **and** sidecar run at 50 fps
(GPU / lowmem defaults). Run after `stress50` holds. Still `--release`.
PASS line is `PASS: live stress50_full rss=… up=50/50`.

```bash
cargo run --release -p panel --bin panel-play -- --live stress50_full
# or: BOT_LIVE=stress50_full cargo run --release -p panel --bin panel-play
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

**Script** Browse / Start / Pause / Stop are wired ([script.md](script.md)).
Load is enabled except while a script is active. Parameters **Edit** stays
disabled (`not in v1`); uncollapse shows schema defaults (empty until a
port fills them). **Nav config** is live (debug paints / labels / FindOptions
toggles) as its own non-blocking window. **General config** (under WalkTo, above profile)
is **slot** (capture, auto-login on title), **render** (none/GPU/CPU;
click the lowmem/highmem button for a sticky picker like Teles), and
**global** (sidecar 50 / only-render-selected).
**Loadouts** stays mocked until the TS
shim. Text and buttons wrap or equal-width-squish — **no horizontal
scroll**.
`chrome.rs` keeps the section inventory (`wired: bool`). Title (MultiBox
is a live toggle), dim **build line** (`alpha 1 ·` git short SHA,
`-dirty` when the tree was dirty; hover is crate version then full
commit + built time), banner, profile, **debug** (loopback), and status
key/value rows (including **mem**: highmem/lowmem) fill out the strip.

**WalkTo** (title row) fills the Game pane: north-up collision dots,
click-to-pick uses the canvas rect (`is_mouse_hovering_rect`), footer
**Recentre** / **Walk**, and **Teleport** (local-engine cheat to the
highlighted tile). Status `walk` mirrors the armed dest.

## Headless proof

The same wiring is exercised live without a window in
`crates/e2e/tests/panel_view.rs` (renderer pixel proof + capture walk):

```bash
LIVE=1 cargo test -p e2e --test panel_view -- --ignored --test-threads=1
```
