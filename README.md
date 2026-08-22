# 274bot

Private bot host workspace.

- `host` — bot host binary crate
- `vault` — credential/state storage crate
- `api` — server-facing API crate
- `host-play` — scratch/playground crate

The RuneScape-era client primitives live in the `vendor/fr-client-rust` submodule
(`Fairy-Ring/FR-client-rust`, branch `rs2-r274`), exposed to the workspace as the
`client` crate. See `docs/` for documentation.
