# 274bot

Headless **RuneScape revision 274** (~2004) **bot host**: N clients in one process, shared type tables, a login FIFO, an encrypted vault, and a small agent API. Campaign 1 is a working **kernel**, not a finished bot.

| | |
|--|--|
| **This repo** | [acfrazier/274bot](https://github.com/acfrazier/274bot) |
| **Client** | submodule [Fairy-Ring/FR-client-rust](https://github.com/Fairy-Ring/FR-client-rust) `rs2-r274` |
| **Status** | Kernel works (login, gens, auto-run proof, live e2e). Orange panel, nav, scripts: later |

## Attribution

This is **not** Jagex, **not** official Lost City, and **not** Fairy Ring. The playable/headless **client library** is Fairy Ring’s 274 Rust port (itself a Lost City derivation). 274bot **injects** that crate; it does not fork client packets or grow a bot API inside FR-client-rust.

See [NOTICE.md](NOTICE.md). MIT for **this** repo’s crates: [LICENSE](LICENSE). Client MIT/NOTICE stay in the submodule.

## What this tree is

| This repo | Not this repo |
|-----------|----------------|
| Host kernel: one OS thread per `Client`, 20 ms loop | Fairy Ring / Lost City client sources (submodule) |
| Encrypted vault, login FIFO (LC default throttle) | Orange panel / multibox UI (campaign 2) |
| Snapshot → interact → settle API, auto-run as send proof | Nav, rustyscript, randoms, gathering, AIO |
| Native live e2e (`LIVE=1`, not Playwright) | A TypeScript rs2b0t port |

## Crates

- `host` — slot threads, login FIFO, synthesized tick, auto-run
- `vault` — AES-256-GCM profiles (username key, stable uid)
- `api` — generation-stamped snapshot, `doAction` / `tryMove` / `out`, settle
- `host-play` — CLI
- `e2e` — live harnesses (`#[ignore]` unless `LIVE=1`)

Agent API notes (committed): [`docs/api/`](docs/api/README.md).

## Clone

```bash
git clone --recurse-submodules https://github.com/acfrazier/274bot.git
cd 274bot
```

If you already cloned without submodules: `git submodule update --init`.

You need a **local 274 engine** (game `43594`, HTTP `/crc` on `:80`) and the pack cache. This git tree does **not** ship Jagex assets. Bake the client RSA public half from the engine’s `private.pem` the same way Fairy Ring does (`vendor/fr-client-rust/tools/redeploy.sh`, or `LOGIN_RSAN` / `LOGIN_RSAE` when compiling `client`). A plain `cargo build` without those env vars rebakes the Java default key.

Default cache path if unset: `$HOME/experiments/Server/engine/data/pack/client` (override with `--cache`).

## Run

```bash
export BOT_VAULT_PASS=bot
cargo run -p host-play -- --user test
# second slot (FIFO spaces opcode 14/16 by 2.5 s):
# cargo run -p host-play -- --user test --user test2
```

`--vault-pass` is equivalent to the env. Empty passphrase is rejected. First run creates `~/.274bot/vault` and profiles (`password = username` unless you upsert). Headless **lowmem** is the default; `--highmem` if you need it. `--debug` or `BOT_DEBUG=1` prints slot logs.

`--mainland` (or `BOT_MAINLAND=1`) after scene 2 sends the rs2b0t tutorial skip (`tele` Lumbridge courtyard + `setvar tutorial 1000`). It does not relog; sidebar unlock is later. New accounts otherwise stay on Tutorial Island.

There is **no window** in this crate (that is campaign 2). To watch the world, run Fairy Ring `client-play --window` on a **different** account.

## Live tests

Require the local engine. Quiet unless `BOT_DEBUG=1`. Failures print `FAIL:` and `exit(1)`. Wait `ingame && scene_state == 2`.

```bash
export BOT_VAULT_PASS=bot
LIVE=1 cargo test -p e2e -- --ignored --test-threads=1
# verbose:
BOT_DEBUG=1 LIVE=1 cargo test -p e2e -- --ignored --test-threads=1 --nocapture
```

Without `LIVE=1`, ignored tests stay skipped (`cargo test` is green with no engine).

## Completeness disclaimer

We do **not** claim this tree is authentic, original RS, or done. The kernel is the smallest host that can log N clients without unpacking the cache N times or cloning the world every 20 ms. Humans and agents make mistakes.

## License

Bot crates: [MIT](LICENSE), Copyright (c) 2026 Austen Frazier.  
Client submodule: Lost City / Fairy Ring MIT — [NOTICE.md](NOTICE.md).
