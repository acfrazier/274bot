# 274bot

Private bot host workspace.

- `host` — bot host kernel (one OS thread per client slot, login FIFO)
- `vault` — credential/state storage crate
- `api` — server-facing API crate
- `host-play` — CLI: run vaulted profiles through the host kernel
- `e2e` — live harnesses (rs2b0t-style, `#[ignore]` unless `LIVE=1`)

The RuneScape-era client primitives live in the `vendor/fr-client-rust` submodule
(`Fairy-Ring/FR-client-rust`, branch `rs2-r274`), exposed to the workspace as the
`client` crate. See `docs/` for documentation.

## Live tests

Require the local 274 engine (`127.0.0.1:43594`) with the pack cache at
`$HOME/experiments/Server/engine/data/pack/client`. Quiet unless `BOT_DEBUG=1`.

```bash
cd /Users/acfrazier/experiments/274bot-host
export BOT_VAULT_PASS=bot
# quiet:
LIVE=1 cargo test -p e2e -- --ignored --test-threads=1
# verbose:
BOT_DEBUG=1 LIVE=1 cargo test -p e2e -- --ignored --test-threads=1 --nocapture
```

Without `LIVE=1` the ignored tests stay skipped, so plain `cargo test` is
green with no engine.

The CLI run is the same flow without the harness assertions:

```bash
export BOT_VAULT_PASS=bot
cargo run -p host-play -- --user test --user test2
```
