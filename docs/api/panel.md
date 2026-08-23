# Panel: native UI (`panel-play`)

`crates/panel` is the native **dear-app / ImGui** UI over the same kernel
slots `host-play` runs ([login.md](login.md), [vault.md](vault.md)).
Campaign 2 built the single-bot chrome; campaign 4 adds the **MultiBox
wall** (sidecar rail, grid mode, profile chooser) below.
It does **not** reimplement the client UI — there is **no Present**, no client
window feature. The client stays headless (`set_draw` only rasters into a
pixel buffer); the panel displays those pixels and feeds input back.

## Run

```bash
export BOT_VAULT_PASS=bot
cargo run --release -p panel --bin panel-play
```

A passphrase is required and an empty one is rejected. First run creates
`~/.274bot/vault` **empty** — panel-play does **not** auto-create `test`/`test`
(that is host-play CLI: `--user test` defaults, `password = username`).
Type a username/password in credentials and **Save** to upsert, spawn that
slot on the login FIFO, and select it. Unlike the CLI there is **no
`--vault-pass` flag** — the passphrase comes from `BOT_VAULT_PASS`, or from
the in-panel prompt (which also covers interactive use). When
`BOT_VAULT_PASS` is set, the panel unlocks **before** the window opens so the
headless path works unchanged. `BOT_MAINLAND=1` (or the checkbox) queues the
mainland hop at scene 2.

## Wiring

`Session::unlock` starts an empty `Play` (shared `Arc<Cache>`, login FIFO)
via `host_play::run_with_io`, then **selects the first vault name**, which
spawns **that one slot**. Parked profiles do not get a `Client` until you
select them; once spawned they stay up so the combo can channel-change.
Each running slot has its own `PixelBuf` + `SlotInput`. A per-frame
observe hook applies the focus `set_draw` switch and the mainland hop.
The runner is configured with **docking on, multi-viewports off** (single
main viewport). Default dock: **game left**, **330px-class panel right**.
Single-bot hides the dock tab strip on each pane (`AUTO_HIDE_TAB_BAR`).
MultiBox rail mode splits a third **264px rail** on the far right (game
and panel shrink by `RAIL_W`; dear-app's `AddOns` cannot resize the OS
window, so the rail is split inside the current window), which brings the
strip's tab bar back.
The Game pane title is the focused profile name when the tab bar is visible. Closing the Game pane sets
`game_pane_open = false` (`set_draw` off) and turns capture off. The UI
thread is capped at **50 fps** (`RedrawMode::WaitUntil`) so it does not
Poll-spin against the 20 ms slot; pixel uploads skip while `PixelBuf`
generation is unchanged.

The Game Image is already a **wgpu texture** (GPU composite). The 274 scene
behind it is still **CPU Pix3D** into `draw_area` — same painter as the
headless client, not a GPU 3D backend, and not the client's `Present`
(softbuffer). Unfocused running slots skip `mainredraw` **this tick**. Watch-only is a
**1 fps rail**; turning the renderer on paints immediately. Capture is
full 20 ms for the minimenu. The slot sleeps the leftover of 20 ms after
the work.

Run **`--release`**. A default debug Pix3D pins a core. One live client
still holds on the order of a gigabyte (process-wide model/anim stores +
scene); that is the 274 painter, not the ImGui chrome.

## Renderer vs capture

Two independent checkboxes, both gated on a focused profile with its pane
open:

- **game renderer** — default **on**, **1 fps rail** (rs2b0t). Checking
  it after off paints **this tick** (no cold wait). Capture raises the
  focused slot to 50 fps. Unfocused slots do not raster. The Game Image is
  an RGBA8 **765×503** texture. Rendering never pauses the bot.
- **capture input** — click-through: while on and the Image is hovered,
  local coords stream `InputEv::Move`, mouse buttons send `Down`/`Up`
  (left=1, right=2), and keys go to `InputEv::Key` on that slot only.
  Off means watch-only with zero input work (`tx` is `None`; the slot does
  no `try_recv`).

Capture follows focus (never two keyboards) and implies renderer.

## MultiBox wall (campaign 4)

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

A modal listing every vault profile. Clicking a row loads it onto the wall
(stays open for more); **Load all** loads every profile; **Close**/Esc
closes without loading. The row **✕** deletes the **vault profile only** —
a live wall member keeps running and stays on the rail (credentials Save
re-creates the row). The rail tile **✕** is the opposite: it removes the
member from the wall, arms a clean IF logout when ingame, stops the slot,
and never touches the vault.

### Resource honesty

The resource card is the operator measurement surface: **bots**, **CPU**,
**RAM**, **traffic**, and **draw**, sampled once a second. The first CPU and
traffic samples read "measuring…". Traffic is the sum of each live slot’s
`ClientStream` payload `bytes_in + bytes_out` over that second — never a fake
`0 B/s` before two samples, and never `0 B/s` when there are no slots (still
measuring…). If a slot drops (or the byte sum shrinks), traffic
**re-baselines** instead of inventing a wrap spike. A failed process sampler
shows "monitor error" for CPU/RAM; it does not invent traffic. Process RSS is
the whole host (Null skip-paint does not free the ~1 GB scene); on macOS the
RAM row is **peak** (`ru_maxrss`). Draw is the focused slot’s `game_draw`
enters plus paint/skip counts. `BOT_DEBUG=1` also prints 1 Hz loop vs raster
timings per slot and the RSS sample (4.5b baseline; 4 bots at ~10 GB is a
suspected leak, not a closed RAM budget).

## Headed live

`BOT_VAULT_PASS` is unused for `--live`. Headed live waits until both
slots are `ingame && scene_state==2`, prints RSS/counters, PASS, and
**leaves the window interactive** (you can click the rail). The 3 s
unfocused-`game_draw` freeze is the **headless** twin only.

```bash
cargo run --release -p panel --bin panel-play -- --live null_raster
# or: BOT_LIVE=null_raster cargo run --release -p panel --bin panel-play
```

FAIL prints and exits 1. On PASS the window stays up. Headless twin:

```bash
LIVE=1 cargo test -p e2e --test null_raster -- --ignored --test-threads=1
```

## Amber

Accent color **`#FFB000`** (hover `#FFC14D`) — amber CRT over Theme::Dark
(title bars, tabs, frames, buttons). Not default imgui blue, not rs2b0t
green `#04A800`. Panel background `#111`.

## Mocks

Chrome follows rs2b0t `BotPanel` in the **330px** right strip: title
(MultiBox is a live toggle), dim build line, banner, profile, credentials
(Save / Log in / Logout / Clear, auto-login on title), status key/value
rows, log, rendering, input. Script and parameters stay mocked (campaign 5).
Unwired controls are **disabled** with a per-item `campaign 5`
tooltip (`SetItemTooltip`, not one window for every mock). Text and buttons wrap or equal-width-squish — **no horizontal
scroll**. `chrome.rs` keeps the section inventory (`wired: bool`).

## Headless proof

The same wiring is exercised live without a window in
`crates/e2e/tests/panel_view.rs` (renderer pixel proof + capture walk):

```bash
LIVE=1 cargo test -p e2e --test panel_view -- --ignored --test-threads=1
```
