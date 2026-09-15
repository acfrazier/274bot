# Agent API

Alpha kernel surface a bot agent codes against: reading world state,
acting, settling, navigating, and logging in — through the `api` crate
(`crates/api`) plus `nav` (`crates/nav`) and the host tick. The script
crate is the runner kernel (compiled cards + Load isolate + catalog
registration). Catalog helper compatibility is **partial** and fail-closed;
honest skilling bots are not “all green by default.” Nav **execute**, the
random-event guardian, revision server profiles, and `tui-play` are in this
tree.

- [snapshot.md](snapshot.md) — the full gen-stamped world read model
  (`GameSnapshot` + `ReadContext`)
- [query.md](query.md) — the fluent `Query<T>` read DSL + typed filters
- [interact.md](interact.md) — `Driver` + the `Interactions` orchestration layer
- [settle.md](settle.md) — pollable `Settle`/`Outcome`/`Evidence`
- [nav.md](nav.md) — whole-world collision + transport graph + Dijkstra
  router + `Traveller::follow` + WalkTo picker
- [login.md](login.md) — login FIFO throttle numbers + server profiles
- [vault.md](vault.md) — encrypted profile vault, assignments, `BOT_VAULT_PASS`
- [panel.md](panel.md) — native UI (`panel-play`): chrome, MultiBox, scripts
- [script.md](script.md) — compiled `tick` vs Load isolate; assignment, reload, cache
- [tui.md](tui.md) — headless operator panel (`tui-play`): same `Play`, raster Off

## Layout

| Crate | Role |
| --- | --- |
| `api` | Read model (`snapshot` + `query`), act primitives (`interact`), settle/evidence (`settle`), legal send table (`prot`), item/loc defs (`obj_names`) |
| `nav` | Whole-world collision bake, transport graph, Dijkstra router, pollable `Traveller::follow`, WalkTo picker grid |
| `host` | One OS thread per client slot; drains gens per frame; snapshot/think after drain |
| `vault` | Encrypted profile store (AES-256-GCM), per-profile script assignment/settings |
| `host-play` | CLI + shared `Play`: resolve server profile, unlock vault, run slots |
| `script` | Compiled `Script` trait + Load isolate; catalog/file cards; content-addressed JS cache |
| `scenario` | Shared headed/headless live scenario runner (`panel-play --live` and `crates/e2e`) |
| `e2e` | Headless live twins (`LIVE=1`) + `e2e-suite` ordered runner |
| `panel` | Native UI (`panel-play`): profiles, Log in/Logout, WalkTo, scripts, MultiBox |
| `tui` | Headless operator panel (`tui-play`): ratatui + crossterm, same `host_play::Play`, raster Off |

The client is the `vendor/fr-client-rust` submodule (path dep as `client`).
The kernel talks to it through `api::interact::Driver` (real impl: `Client`)
and `api::prot::Out` (real impl: the client's ISAAC `Packet`). The kernel
never writes a bare opcode outside those two paths.

## Tick model

There is **no tick-end opcode** in the server protocol. The host synthesizes
the tick edge:

1. One thread per slot runs `client.mainloop()` once every **20 ms** frame.
2. After each pass the drain `Pump` diffs `Client.gens` and returns
   `DrainResult.dirty` (computed **before** committing `last`).
3. Families whose gens moved are rebuilt on the slot’s `GameSnapshot`.

`player_info` true ⇔ `gens.player` moved since the last drain — that is the
**server-tick** edge (compiled scripts `tick` here).

## Live harnesses

`crates/e2e` lives here (nav / panel / scenario twins + suite runner). Login,
RSS, and null-raster twins live in `crates/host-play`.

```bash
LIVE=1 cargo test -p e2e -- --ignored --test-threads=1
LIVE=1 cargo test -p host-play -- --ignored --test-threads=1
```

`panel-play` is the first-class **headed** harness (`--smoke` whole-window
capture, and the `crates/scenario` engine via `--live <scenario>` with
screenshots); `crates/e2e` is the headless twin. Failures print `FAIL:` and
`exit(1)`. Wait until `ingame && scene_state == 2`. Suite contracts:
[../e2e-suite.md](../e2e-suite.md).
