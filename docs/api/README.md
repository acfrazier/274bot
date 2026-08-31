# Agent API

Alpha `0.1.0` kernel surface a bot agent codes against: reading world
state, acting, settling, navigating, and logging in — all through the
`api` crate (`crates/api`) plus `nav` (`crates/nav`) and the host tick.
Honest bot scripts are **not** in this tag; the script crate is the
runner kernel only.

- [snapshot.md](snapshot.md) — the full gen-stamped world read model
  (`GameSnapshot` + `ReadContext`)
- [query.md](query.md) — the fluent `Query<T>` read DSL + typed filters
- [interact.md](interact.md) — `Driver` + the `Interactions` orchestration layer
- [settle.md](settle.md) — pollable `Settle`/`Outcome`/`Evidence`
- [nav.md](nav.md) — whole-world collision + transport graph + Dijkstra
  router + `Traveller::follow` + WalkTo picker
- [login.md](login.md) — login FIFO throttle numbers
- [vault.md](vault.md) — encrypted profile vault, `BOT_VAULT_PASS` / `--vault-pass`
- [panel.md](panel.md) — native UI (`panel-play`): chrome, MultiBox wall, renderer, scripts
- [script.md](script.md) — compiled `tick` vs Load isolate; PLAYER_INFO wake

## Layout

| Crate | Role |
| --- | --- |
| `api` | Read model (`snapshot` + `query`), act primitives (`interact`), settle/evidence (`settle`), legal send table (`prot`), item/loc defs (`obj_names`) |
| `nav` | Whole-world collision bake, transport graph, Dijkstra router, pollable `Traveller::follow`, WalkTo picker grid |
| `host` | One OS thread per client slot; drains gens per frame; snapshot/think after drain |
| `vault` | Encrypted profile store (AES-256-GCM) |
| `host-play` | CLI: unlock vault (`BOT_VAULT_PASS` / `--vault-pass`) and run slots |
| `script` | Compiled `Script` trait + Load isolate (`rustyscript`); picker ids |
| `scenario` | Shared headed/headless live scenario runner (`panel-play --live` and `crates/e2e`) |
| `e2e` | Headless live twins (`LIVE=1`); ignored unless that env is set |
| `panel` | Native UI (`panel-play`): profile name + Profiles picker, Log in/Logout, WalkTo, debug heading, script Browse/Start/Pause/Stop, MultiBox wall |

The client is the `vendor/fr-client-rust` submodule (path dep as `client`).
The kernel talks to it through `api::interact::Driver` (real impl: `Client`)
and `api::prot::Out` (real impl: the client's ISAAC `Packet`). The kernel
never writes a bare opcode outside those two paths.

## Tick model

274 has **no tick-end opcode** in its server protocol. The host synthesizes
the tick edge:

1. One thread per slot runs `client.mainloop()` once every **20 ms** frame.
2. After each pass the drain `Pump` diffs `Client.gens` and returns
   `DrainResult.dirty` (computed **before** committing `last`).
3. Families whose gens moved are rebuilt on the slot’s `GameSnapshot`.

`player_info` true ⇔ `gens.player` moved since the last drain — that is the
**server-tick** edge (compiled scripts `tick` here).

## Live harnesses

`crates/e2e` lives here (nav / panel / scenario twins). Login, RSS, and
null-raster twins live in `crates/host-play`.

```bash
LIVE=1 cargo test -p e2e -- --ignored --test-threads=1
LIVE=1 cargo test -p host-play -- --ignored --test-threads=1
```

`panel-play` is the first-class **headed** harness (`--smoke` whole-window
capture, and the `crates/scenario` engine via `--live <scenario>` with
screenshots); `crates/e2e` is the headless twin. Failures print `FAIL:` and
`exit(1)`. Wait until `ingame && scene_state == 2`.
