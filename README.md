# 274bot

Headless **RuneScape revision 274** (~2004) **bot host**: N clients in one process, shared type tables, a login FIFO, an encrypted vault, a small agent API, and a native panel (MultiBox wall, --live).

| | |
|--|--|
| **Revision** | RuneScape 274 (~2004) |
| **Engine** | A **local** Lost City engine (game `43594`, HTTP `/crc` on `:80`) |
| **Client** | submodule [acfrazier/FR-client-bothost](https://github.com/acfrazier/FR-client-bothost) `r274-bothost` (fork of Fairy-Ring `rs2-r274`) |
| **This repo** | [acfrazier/274bot](https://github.com/acfrazier/274bot) — **MIT** ([LICENSE](LICENSE), [NOTICE.md](NOTICE.md)) |

## What it is

A Rust bot host over the 274 client. The kernel runs one OS thread per
`Client` on a 20 ms loop, shares unpacked type tables across slots, throttles
login handshakes through a FIFO, keeps profiles in an AES-256-GCM vault, and
exposes a small agent API (snapshot → interact → settle). **`panel-play`**
is the operator window (dear-app/ImGui): credentials, status, WalkTo picker,
game blit, click-through capture, MultiBox rail/grid, `--live` harness.

## What it does not (yet)

- Script **ports** are still thin: compiled picker cards exist; **WalkTo
  Start is `not ported`** until `ctx.walk` is wired. Load’d JS is isolate +
  a stub `@rs2b0t` prelude, not a full rs2b0t ABI. Parameters **Edit** is
  gray (`not in v1`).
- The **wall is in** (campaign 4). Unlocking the vault starts the **first**
  profile as a live slot; other vault rows stay parked until selected (or
  loaded onto the wall). MultiBox raises the running set as a sidecar rail
  or a grid, with bulk **Login all / Logout all** and a profile chooser.
  Auto-login defaults **off** per profile; closing the rail does not log
  anyone out.
- No hosted product, no official anything: this is **not** Jagex, **not**
  official Lost City, **not** Fairy Ring, **not** a file-port of rs2b0t
  (rs2b0t's chrome is mimicked in the panel; the code is our own).
- The playable/headless **client library** is [FR-client-bothost](https://github.com/acfrazier/FR-client-bothost)
  `r274-bothost`, a bot-host fork of Fairy Ring's 274 Rust port (a Lost City
  derivation). Packet timing and `doAction` stay Java-shaped; viewport opti
  and read-only instrumentation land on the fork. Still no bot action API
  inside `client`. Client MIT/NOTICE stay in the submodule.

This tree ships **no Jagex assets**; you bring your own local engine and pack
cache. AI-assisted development is explicit and TDD-driven, and we do **not**
claim this tree is authentic, original, or done — humans and agents make
mistakes.

## Quick start

```bash
git clone --recurse-submodules https://github.com/acfrazier/274bot.git
cd 274bot
```

You need a **local 274 engine** (game `43594`, HTTP `/crc` on `:80`) and the
pack cache. Default cache path if unset:
`$HOME/experiments/Server/engine/data/pack/client` (override with `--cache`).

**RSA bake:** cargo’s `TARGET` is the rustc triple, so the live/prod switch
is **`BOT_TARGET`**. Local (default) still uses `LOGIN_RSAN` / `LOGIN_RSAE`
or `vendor/fr-client-rust/tools/redeploy.sh` from the engine `private.pem`.
Live rs2b2t is **not** that pem — scrape the public modulus from the served
client (same as rs2b0t `b0t.sh`):

```bash
MOD=$(curl -s --max-time 15 https://w1.rs2b2t.com/client/client.js \
  | grep -oE '[0-9]+' | awk 'length($0) >= 250 { print; exit }')
BOT_TARGET=live LIVE_RSAN="$MOD" cargo build -p client
# host-play live host: TARGET=live (runtime env, not the bake triple)
```

Login response **6** retries once after `/loginkey` then that `client.js`
scrape. `prod` bake requires `PROD_RSAN`. TCP `w1.rs2b2t.com:43594` — no WSS.

```bash
export BOT_VAULT_PASS=bot
# CLI: run one or more vaulted profiles
cargo run --release -p host-play -- --user test

# Panel: same vault, native UI (MultiBox, --live)
cargo run --release -p panel --bin panel-play

# Headed live (BOT_VAULT_PASS unused): FAIL+exit 1; PASS keeps the window up
cargo run --release -p panel --bin panel-play -- --live null_raster
# or BOT_LIVE=null_raster; headless twin: LIVE=1 cargo test -p e2e --test null_raster -- --ignored --test-threads=1

# Headless RSS ladder (all slots Null / set_draw=false). One N per process.
LIVE=1 RSS_N=1 cargo test -p e2e --test rss_ladder -- --ignored --test-threads=1 --nocapture
LIVE=1 RSS_N=2 cargo test -p e2e --test rss_ladder -- --ignored --test-threads=1 --nocapture
LIVE=1 RSS_N=4 cargo test -p e2e --test rss_ladder -- --ignored --test-threads=1 --nocapture
# Does not fail on RSS size. Do not run two N in one process (peak RSS).

274bot defaults to **lowmem**. Renderer-off / `set_draw(false)` still runs
`mainloop` and collision, but does not decode loc meshes. Flip the renderer on
to rebuild 3D from the same map bytes. **Music / SFX** (panel rendering
section) sets `Profile.settings.lowmem = false` for that username; it applies
the next time the profile starts. Highmem/midi code stays in bothost
(`client-play --window` / Fairy-Ring).

# 50-bot MultiBox wall watch (temp vault s00…s49; 10 min timeout; local engine)
cargo run --release -p panel --bin panel-play -- --live stress50
# or BOT_LIVE=stress50; FIFO login takes minutes; does not fail on RSS size

# Unit tests (no engine). CI: fmt + clippy --no-deps + cargo test (no LIVE=1)
cargo test -p script --offline
```

**Scripts (campaign 5 kernel):** panel **Browse / Start / Pause / Stop** are
live. Compiled cards tick on the **PLAYER_INFO** edge (not every 20 ms). Idle
slots have no V8. **Load** a `.ts`/`.js` to add a picker card tagged JS
(same name overwrites; compiled names like WalkTo are reserved). V8 starts
only when you Start that card. Persist: `~/.274bot/js-scripts.json`.

`--release` matters: Pix3D is a CPU 3D painter. A debug `cargo run` will
look frozen and show a gigabyte-class Activity Monitor spike (and `cargo`
itself may dwarf the process). The panel only starts the **focused**
vault profile; switching the combo starts a parked name once, then
channel-changes among already-running slots. Last focus persists in
`~/.274bot/panel-ui.json` (`last_focus`). Collapsible section headings
persist per profile there too (**script** / **parameters** default closed).
Credentials are **2×2**: Save/Clear then Log in/Logout (Logout disabled
unless focused+ingame). The panel log is **per client thread** (per
username), not one concatenated process log. There is **no mainland
checkbox** in the panel.

`--vault-pass` is equivalent to the env (panel-play uses the env or the
in-panel prompt). Empty passphrase is rejected. First run creates
`~/.274bot/vault`. **host-play** upserts named users (`--user test` defaults
to `test`/`test`, `password = username`). **panel-play does not auto-create
`test`/`test`**: an empty first-run vault stays empty until you type a
username/password in credentials and Save (upsert, spawn that slot, select).
Headless **lowmem** is the default; `--highmem` if you need it. `--debug` or
`BOT_DEBUG=1` prints slot logs. `--mainland` (or `BOT_MAINLAND=1`) after
scene 2 sends the rs2b0t tutorial skip (`tele` Lumbridge courtyard +
`setvar tutorial 1000`); it does not relog. New accounts otherwise stay on
Tutorial Island.

**Windows:** `panel-play` is an OS window. It does **not** open the client's
`Present` applet. The Game pane is a **CpuPix3D** blit into ImGui, **never
below 765×503** (`fit_applet` scale floor 1.0; grid tiles may still
downscale). Watch **1 fps**, capture **50 fps**. Unfocused / renderer-off
slots skip `game_draw` (`draw=false`). The real 765×503 applet is
`vendor/fr-client-rust` `client-play --window` (bothost), for fidelity —
not a second 3D engine in 274bot. CpuPix3D is the **holding** painter for
the operator wall (Null / 1 fps). A GPU 3D backend is **not** this repo and
**not** Fairy-Ring; if it happens it is **last** on bothost — a tech demo
of **50 clients at full rate on one GPU**, not the 50-bot product path.
Scripts stay mocked.

## Live tests

Require the local engine. Quiet unless `BOT_DEBUG=1`. Failures print `FAIL:`
and `exit(1)`. Wait `ingame && scene_state == 2`.

```bash
LIVE=1 cargo test -p e2e -- --ignored --test-threads=1
```

Without `LIVE=1`, ignored tests stay skipped (`cargo test` is green with no
engine). Agent API notes (committed): [`docs/api/`](docs/api/README.md).
