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

Same vault rules as `host-play`: a passphrase is required and an empty one is
rejected, first run creates `~/.274bot/vault` with profiles
(`password = username` unless you upsert). Unlike the CLI there is **no
`--vault-pass` flag** — the passphrase comes from `BOT_VAULT_PASS`, or from
the in-panel prompt (which also covers interactive use). When
`BOT_VAULT_PASS` is set, the panel unlocks **before** the window opens so the
headless path works unchanged. `BOT_MAINLAND=1` (or the checkbox) queues the
mainland hop at scene 2.

## Wiring

`Session::unlock` spawns every vault profile as a host slot via
`host_play::run_with_io`, giving each its own `PixelBuf` + `SlotInput`
(never shared across slots). A per-frame observe hook applies the focus
`set_draw` switch and the mainland hop, so slot threads and the UI never
share a lock hot path. The runner is configured with **docking on,
multi-viewports off** (single main viewport).

## Renderer vs capture

Two independent checkboxes, both gated on a focused profile with its pane
open:

- **game renderer** — `set_draw(draw_for_slot(...))`: only the focused slot
  rasters. The Game Image is an RGBA8 **765×503** texture (the client applet
  size, never mutated); the widget scales the display by an integer DPI
  factor (2× on Retina). Rendering never pauses the bot.
- **capture input** — click-through: while on, clicks in the Game Image map
  to applet coords and enqueue `InputEv::Down` on the focused slot's channel.
  Off means watch-only with zero input work (the slot does no `try_recv`).

Capture follows focus (never two keyboards) and implies renderer.

## Amber

Accent color **`#FFB000`** (hover `#FFC14D`) — the rs2b0t amber, applied to
ImGui hover/header/tab colors. Panel background `#111`.

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
