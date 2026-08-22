# Panel: native UI (`panel-play`)

`crates/panel` is the campaign-2 UI: a native **dear-app / ImGui** window over
the same kernel slots `host-play` runs ([login.md](login.md), [vault.md](vault.md)).
It does **not** reimplement the client UI — there is **no Present**, no client
window feature. The client stays headless (`set_draw` only rasters into a
pixel buffer); the panel displays those pixels and feeds input back.

## Run

```bash
export BOT_VAULT_PASS=bot
cargo run -p panel --bin panel-play
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

`Session::unlock` spawns every vault profile as a host slot via
`host_play::run_with_io`, giving each its own `PixelBuf` + `SlotInput`
(never shared across slots), then **selects the first name** so the Game
Image is not stuck on `renderer off`. A per-frame observe hook applies the
focus `set_draw` switch and the mainland hop, so slot threads and the UI
never share a lock hot path. The runner is configured with **docking on, multi-viewports off** (single
main viewport). Default dock: **game left**, **330px-class panel right**.
The Game pane title is the focused profile name. Closing the Game pane sets
`game_pane_open = false` (`set_draw` off) and turns capture off. The UI
thread is capped at **50 fps** (`RedrawMode::WaitUntil`) so it does not
Poll-spin against the 20 ms slot; pixel uploads skip while `PixelBuf`
generation is unchanged.

## Renderer vs capture

Two independent checkboxes, both gated on a focused profile with its pane
open:

- **game renderer** — `set_draw(draw_for_slot(...))`: only the focused slot
  rasters. The Game Image is an RGBA8 **765×503** texture (the client applet
  size, never mutated). The widget is the largest 765:503 box that fits the
  left-hand Game pane (no extra Retina multiply — HiDpi already maps logical
  pixels). Rendering never pauses the bot.
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

Chrome sections the panel does not implement yet render **disabled** with the
owning campaign as a tooltip: the script and parameters sections, plus the
`Browse…` `Start` `Pause` `Stop` `Global settings` `Nav settings` `Loadouts`
`MultiBox` buttons — mock until campaign 5 and later. `chrome.rs` keeps the
rs2b0t section inventory (`wired: bool`) as the single source of truth.

## Headless proof

The same wiring is exercised live without a window in
`crates/e2e/tests/panel_view.rs` (renderer pixel proof + capture walk):

```bash
LIVE=1 cargo test -p e2e --test panel_view -- --ignored --test-threads=1
```
