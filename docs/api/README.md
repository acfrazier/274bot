# Agent API

The kernel surface a bot agent codes against: reading world state, acting,
and logging in — all through the `api` crate (`crates/api`) plus the host
tick and the encrypted vault.

- [snapshot.md](snapshot.md) — generation-stamped world reads (families + gens)
- [interact.md](interact.md) — acting (`doAction` path), the `LEGAL_SEND`/`ClientProt` table
- [login.md](login.md) — login FIFO throttle numbers
- [vault.md](vault.md) — encrypted profile vault, `BOT_VAULT_PASS` / `--vault-pass`

## Layout

| Crate | Role |
| --- | --- |
| `api` | Read model (`snapshot`, `query`), act primitives (`interact`, `settle`), legal send table (`prot`) |
| `host` | One OS thread per client slot; drains gens per frame and synthesizes `on_server_tick` |
| `vault` | Encrypted profile store (AES-256-GCM) |
| `host-play` | Scratch/playground crate (the future CLI that wires host + vault) |

The client is the `vendor/fr-client-rust` submodule (path dep as `client`).
The kernel talks to it through `api::interact::Driver` (real impl: `Client`)
and `api::prot::Out` (real impl: the client's ISAAC `Packet`). The kernel
never writes a bare opcode outside those two paths.

## Tick model: `on_server_tick`

274 has **no tick-end opcode** in its server protocol. The host synthesizes
the tick edge instead:

1. One thread per slot runs `client.mainloop()` once every **20 ms** frame.
2. After each pass the drain `Pump` diffs `Client.gens` (`crates/host/src/slot.rs`).
3. When the drain applied a `PLAYER_INFO` (the player gen moved), the host
   calls `on_server_tick(client, username, slot_state)` and host think hooks
   run there (`crates/host/src/lib.rs`).

`player_info` true ⇔ `gens.player` moved since the last drain — that is the
tick edge. A drain with only e.g. `NPC_INFO` marks families dirty but emits
no tick.

## Auto-run

The one behaviour on the tick hook today: when run energy crosses **20%**
(`RUN_ENERGY_THRESHOLD`, 0–100), the host presses the run orb
(`RUN_ORB_IFACE = 153`, an `IF_BUTTON` on controls overlay 147) via the
`doAction` path and tracks `run_on` so it sends once per crossing. Run state
is server-echoed; the caller decides from snapshot state whether to send.

## Live harnesses

`crates/e2e` live tests (run with `LIVE=1 cargo test -p e2e -- --ignored`)
land in a later task.
