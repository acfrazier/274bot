# 0.1.8.1 home fixture

This is a real home-state fixture written by the 0.1.8.1 Rust code, not a
hand-authored approximation of its wire formats. The fixture password and all
account passwords are the throwaway value `bot`; the three account names begin
with `fixture-` and are not operator accounts.

## Provenance

- Host source: annotated tag `0.1.8.1`, peeled commit
  `a88d077644dadbda21546f702d9717d808004a74`.
- Client source: `c539f24315775c71a2b8c79935ae86b155f446fd`.
- Source/input archives:
  `.superpowers/release-0.1.9/tools/publish-0181/{source,inputs}.tar.gz`.
- Generator: [`generate.rs`](generate.rs), copied without modification to
  `crates/panel/examples/generate_upgrade_fixture.rs` in the extracted source.
- Command, run from that extracted source with the published inputs restored:

  ```text
  cargo run --locked --release -p panel --example generate_upgrade_fixture -- \
    <this-directory>/home
  ```

The encrypted vault uses fresh cryptographic randomness, so regenerating it
preserves meanings but intentionally produces a different vault digest.

## Committed bytes

| File | SHA-256 |
|---|---|
| `home/.274bot/vault-289` | `471a04848f896acdde11c42c2d2ae1e5cc692f8c4f5d6cd689f802889ab1809e` |
| `home/.274bot/panel-ui.json` | `624ef89121683a21cb7d05276233f2a54feb678aae2822c60aaeafb4e973d5c1` |
| `home/.274bot/loadouts.json` | `15edc0554ec1f9f779fae689921e193cfdec4b6cd3efc0d36609be10da179816` |
| `home/.274bot/script-settings.json` | `0c89ca9633b733234429c078929f692b078743e39353dc0837061b32b0aba109` |

`tests/upgrade_compat_0181.rs` copies these files to a fresh temporary directory,
opens them with current readers, asserts every represented meaning, saves with
current writers, reopens, and asserts that those meanings remain intact.
