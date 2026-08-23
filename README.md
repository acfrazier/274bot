# 274bot

Headless **RuneScape revision 274** (~2004) **bot host**: N clients in one process, shared type tables, a login FIFO, an encrypted vault, a small agent API, and (campaign 2) a native panel.

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
exposes a small agent API (snapshot → interact → settle). Campaign 2 adds a
native dear-app/ImGui **panel** (`panel-play`): profile combo, editable
credentials, status/log, game renderer, click-through capture, amber chrome.
Campaign 3 adds a panel **WalkTo** picker: amber collision dots over the
baked nav pack, click to arm the focused slot's traveller. Campaign 4
adds the **MultiBox wall**: one panel process runs N vault slots on the
shared login FIFO as a 264px sidecar **rail** or a full-pane **grid** of
tiles, with bulk **Login all / Logout all** and a profile **chooser**.

## What it does not (yet)

- Not **scripts** — script/parameter chrome stays mocked (campaign 5).
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
pack cache. Bake the client RSA public half from the engine's `private.pem`
the same way Fairy Ring does (`vendor/fr-client-rust/tools/redeploy.sh`, or
`LOGIN_RSAN` / `LOGIN_RSAE` when compiling `client`). Default cache path if
unset: `$HOME/experiments/Server/engine/data/pack/client` (override with
`--cache`).

```bash
export BOT_VAULT_PASS=bot
# CLI: run one or more vaulted profiles
cargo run --release -p host-play -- --user test

# Panel: same vault, native UI (campaign 2)
cargo run --release -p panel --bin panel-play
```

`--release` matters: Pix3D is a CPU 3D painter. A debug `cargo run` will
look frozen and show a gigabyte-class Activity Monitor spike (and `cargo`
itself may dwarf the process). The panel only starts the **focused**
vault profile; switching the combo starts a parked name once, then
channel-changes among already-running slots.

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

There is **no client window** anywhere in this tree. The panel's Game Image
is a 765×503 texture at **1 fps** (rs2b0t rail) while you watch; capture
is 50 fps. Unfocused slots do not paint. A GPU 3D backend belongs on the
Fairy Ring client crate, not as a second painter here. `client-play
--window` is the real applet.

## Live tests

Require the local engine. Quiet unless `BOT_DEBUG=1`. Failures print `FAIL:`
and `exit(1)`. Wait `ingame && scene_state == 2`.

```bash
LIVE=1 cargo test -p e2e -- --ignored --test-threads=1
```

Without `LIVE=1`, ignored tests stay skipped (`cargo test` is green with no
engine). Agent API notes (committed): [`docs/api/`](docs/api/README.md).
