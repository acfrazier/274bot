# Panel: native UI (`panel-play`)

`crates/panel` is the campaign-2 UI: a native **dear-app / ImGui** window over
the same kernel slots `host-play` runs ([login.md](login.md), [vault.md](vault.md)).
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
Single-bot hides the dock tab strip on each pane (`AUTO_HIDE_TAB_BAR`);
stacking a second window in a node (MultiBox later) shows tabs again.
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

## Amber

Accent color **`#FFB000`** (hover `#FFC14D`) — amber CRT over Theme::Dark
(title bars, tabs, frames, buttons). Not default imgui blue, not rs2b0t
green `#04A800`. Panel background `#111`.

## Mocks

Chrome follows rs2b0t `BotPanel` in the **330px** right strip: title +
MultiBox, dim build line, banner, profile, credentials (Save / Log in /
Clear, auto-login mocked), script, parameters, status key/value rows, log,
rendering, input. Unwired controls are **disabled** with a per-item `campaign N`
tooltip (`SetItemTooltip`, not one window for every mock). Text and buttons wrap or equal-width-squish — **no horizontal
scroll**. `chrome.rs` keeps the section inventory (`wired: bool`).

## Headless proof

The same wiring is exercised live without a window in
`crates/e2e/tests/panel_view.rs` (renderer pixel proof + capture walk):

```bash
LIVE=1 cargo test -p e2e --test panel_view -- --ignored --test-threads=1
```
