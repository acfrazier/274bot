# Revision-274 nightly engine files

These files are installed into the 274 nightly guest's
`/srv/nightly/engine-pristine/` at setup (see the lane's runbook in
the NIGHTLY-274 evidence). The runner copies that folder to
`engine-run/` for each run and starts it with `start-274.sh`.

- `start-274.sh`, `stop-274.sh`, `status-274.sh`: twins of the 289
  nightly engine's scripts. They record the engine process identity
  (pid, command, cwd, start time) and signal only a pid whose
  identity still matches. Status fails when any listener the engine
  owns is not on loopback. Ports: web 80, game 43594, management
  8898. These are the built-in `local-274` launch profile's ports.
  Port 80 needs `cap_net_bind_service` on the node binary (set at
  setup) or an equivalent guest setting.
  On zinfandel, the scripts use `sudo -n lsof` for process and listener checks.
  User `bot` must have non-interactive sudo for these read-only checks.
  Unprivileged `lsof` returned `readlink: Permission denied` for the engine CWD.

- `world.json`: installed as `engine/data/config/world.json`.
  Revision 274, debug on, production off, no live reload, content
  beside the engine (`../content`). The login, friend and logger
  workers stay off. Their hosts name loopback anyway.

The engine and content sources are pinned at engine `4c95f87e` and content
`000c19997` on branch `274`. The engine patches in
`tools/engine-patches/274/` are applied. No account database or player save is
copied in. `npm run sqlite:migrate` makes the empty account database.

The Rust revision-274 nav build reads `data/pack/config` beside the client
cache, and expects `data/pack/client/wordenc`. The pinned engine pack places
those exact archives at `data/pack/client/config` and `data/raw/wordenc`.
Create relative links after `npm run build`:

```sh
cd /srv/nightly/engine-pristine/engine
ln -s client/config data/pack/config
ln -s ../../raw/wordenc data/pack/client/wordenc
```

The resolved `config` and `wordenc` hashes must match the revision-274 row in
`crates/host-play/src/known-cache-identities.json`; this exposes the original
bytes without copying or rebuilding them.
