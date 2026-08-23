# Agent API

The kernel surface a bot agent codes against: reading world state, acting,
and logging in — all through the `api` crate (`crates/api`) plus the host
tick and the encrypted vault.

- [snapshot.md](snapshot.md) — generation-stamped world reads (families + gens)
- [interact.md](interact.md) — acting (`doAction` path), the `LEGAL_SEND`/`ClientProt` table
- [nav.md](nav.md) — baked nav pack, A* router, per-tick traveller, WalkTo picker
- [login.md](login.md) — login FIFO throttle numbers
- [vault.md](vault.md) — encrypted profile vault, `BOT_VAULT_PASS` / `--vault-pass`
- [panel.md](panel.md) — campaign-2 native UI (`panel-play`), renderer vs capture, mocks

## Layout

| Crate | Role |
| --- | --- |
| `api` | Read model (`snapshot`, `query`), act primitives (`interact`, `settle`), legal send table (`prot`) |
| `nav` | Nav pack bake/load, A* router, per-tick traveller, WalkTo picker grid |
| `host` | One OS thread per client slot; drains gens per frame; snapshot/settle/think after drain |
| `vault` | Encrypted profile store (AES-256-GCM) |
| `host-play` | CLI: unlock vault (`BOT_VAULT_PASS` / `--vault-pass`) and run slots |
| `panel` | Native dear-app/ImGui UI (`panel-play`): profile combo, credentials, status/log, game renderer, capture |

The client is the `vendor/fr-client-rust` submodule (path dep as `client`).
The kernel talks to it through `api::interact::Driver` (real impl: `Client`)
and `api::prot::Out` (real impl: the client's ISAAC `Packet`). The kernel
never writes a bare opcode outside those two paths.

## Tick model

274 has **no tick-end opcode** in its server protocol. The host synthesizes
the tick edge instead:

1. One thread per slot runs `client.mainloop()` once every **20 ms** frame.
2. After each pass the drain `Pump` diffs `Client.gens` (`crates/host/src/slot.rs`)
   and returns `DrainResult.dirty` (computed **before** committing `last`).
3. Families whose gens moved are rebuilt on the slot’s `GameSnapshot`. Settle
   runs when any family is dirty. Auto-run think runs every `after_drain`
   (energy can move on `UPDATE_RUNENERGY` without `PLAYER_INFO`).

`player_info` true ⇔ `gens.player` moved since the last drain — that is the
**server-tick** edge (scripts that must think once per cycle use it). A drain
with only e.g. `NPC_INFO` marks families dirty and still rebuilds/settles,
but is not a player-tick.

## Auto-run

The one behaviour on the think hook today: when run energy crosses **20%**
(`RUN_ENERGY_THRESHOLD`, 0–100) **and run is off**, the host presses the
run-on orb (`RUN_ORB_IFACE = 153`) via `set_run(true)` / `IF_BUTTON`.
`set_run(false)` presses **152**. `run_on` is not process-lifetime sticky:
energy 0 (cannot be running) clears it so a later 20 crossing sends again.
Already on (energy ≥20 and run echo) → no extra send.

## Live harnesses

`crates/e2e` lives here. Operator command:

```bash
LIVE=1 cargo test -p e2e -- --ignored --test-threads=1
```

`host-play` is the CLI (`BOT_VAULT_PASS` / `--vault-pass`). Verbose only if
`BOT_DEBUG=1`. Failures print `FAIL:` and `exit(1)`. Wait until
`ingame && scene_state == 2`.
