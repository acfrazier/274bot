# Step 3 server profile design review

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-10. Session start ~2026-09-10T15:00:00Z.
Kind: bounded pre-implementation architecture review of
`docs/compat/briefs/06-session-profile-design.md` against current source
seams. Not runtime evidence, not step-4 host-boundary qualification, not
campaign release.

Read once: `AGENTS.md`, `docs/execution.md`, plan architecture and step 3
in `docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md`,
`docs/compat/briefs/05-session-profile-preparation.md`, and the named
proposal. Branch checked first: `codex/rs2b0t-multirevision` (not `main`).
Work was read-only except this report. No product edits, subagents, commit,
merge, or suite reruns. Design approval is not macOS step-3 validation.

## Verdict

**Proceed to step-3 implementation** after root incorporates the required
constraints below.

The ownership split is right: an immutable client connection/resource
value with no host/nav/catalog dependency; a host `Arc<ServerProfile>`
resolved before assets, vault mutation, or sockets; additive checked
construction rather than a broad constructor migration; 289 bot operation
refused until step 4 without dimming catalog cards. Current source does
not implement that contract. The proposal is implementable on the named
pins if ambient call sites are actually bound, OnDemand mismatch is
rejected before join, and PlayOptions does not keep a second mutable
endpoint copy.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Named host product base (source inspected) | `5bb500262fb4e7ab8507589c0784a36f9eac14ce` |
| Campaign HEAD at write-up | `f1a9cfcb2d6769d984f6b3ca09743192beba22f3` (docs-only: brief 06 + STATE) |
| Accepted client (submodule working tree) | `58120f28ee5208553ca07f41cb364f2cf98ea280` |
| Branch | `codex/rs2b0t-multirevision` |
| Proposal SHA-256 | `db4f722d7d892cf35c1c11c86530edf8b2e70d98def7cd5a9dc227b1184d3287` |
| Prep brief SHA-256 | `0155ab3bf81ab7d77e2031bbc38330c29128d68c8cb3f0bc1892be7148f40ab8` |
| 377 host constructor shape (read-only) | `a802ce1094f35d1257abc5f8b497f17675af3dde` `prepare_client_with_revision` |

Confirmed: product source matches named base `5bb50026`; client HEAD
matches the accepted candidate. A later docs-only commit `f1a9cfcb` landed
the proposal brief while this review ran; no product files changed.
`docs/compat/STATE.md` at `5bb50026` still named older host base
`b2bd5023`; this review used the brief's `5bb50026` for source, not that
dated pointer.

## What the proposal gets right

- Client owns framing/login/world/render and a general bindable connection
  identity. Host owns nav/content/catalog/vault, frontend parse, and the
  289 refusal. No bot action API in the client. No 377 import.
- Built-ins are local 274, local 289, known public 274. Public 289 and
  unknown revisions fail with a concrete reason. Ports 274 `43594/80` and
  289 `44594/1080` match `fixture-inputs.md`.
- `from_shared` HTTP default is the known incomplete seam
  (`new_with_revision_and_http_port` binds the asset port;
  `from_shared_with_revision` does not). Binding before OnDemand
  construction is the right constructor order.
- OnDemand today keys hubs by `(host, port)` only
  (`ondemand.rs` `HubMap` / `subscribe_hub`). A later subscriber's
  version/crc tables and `cache_dir` are discarded on join. Distinguishing
  target, revision, and cache/content identity, or rejecting mismatch
  before join, is required.
- `load_template` catch_unwind-to-empty and `Play::new` ignoring
  `NavWorld::load_pack` errors are fail-open. Checked production path plus
  legacy empty-cache tests is the correct migration width.
- Step 4 owns writers. Intermediate 289 refusal must precede raw 274
  `LEGAL_SEND` / guardian / run, while still allowing bound 289
  construction and mocked handshake tests.

## Required constraints

1. **Bind every ambient call site, not only struct fields.** A frozen
   `Arc` that login/HTTP/OnDemand still ignore is not a profile. Today
   these re-read process ambient after construction:

   - `ClientStream::connect` → `uses_secure_transport(bot_target())`
   - `Client::http_get` → same TLS vs TCP choice; port ignored on prod
   - `login` → `login_rsa::active_biguints()` (`BOT_TARGET`, `LOGIN_RSAN/E`,
     `$ENGINE_DIR/.../private.pem`, Java fallback on garbage)
   - `from_shared` construct → `http_port: jag_fetch_port_for(bot_target())`
     (local 80). `maininit` then `fetch_jag_checksums` on that port. Local
     289 needs 1080.
   - `maininit` → `load_on_demand(&self.config)` and
     `unpack_snapshot_dir()` → `unpack_dir()` (`CLIENT_UNPACK_DIR`)
   - OnDemand worker `open_socket` → `ClientStream::connect(&self.host, self.port)`
   - `cheat` / `mainland_hop` → `cheat_allowed(bot_target())`
   - `default_vault_path` / `profile_password` / TUI and panel defaults →
     `bot_target()` / `game_port_for` (local game port is 43594 only)
   - `Play::new` nav → `NAV_PACK` via `default_pack_path()`

   Bound clients must use the frozen identity at those sites. Legacy
   standalone constructors may keep current ambient defaults.

2. **Do not keep a second mutable endpoint copy.** `PlayOptions` today is
   `{host, port, cache_dir, lowmem, mainland}`. Carrying `Arc<ServerProfile>`
   plus independently mutable host/port/cache that “must agree before
   mutation” will desync (`spawn_slot_thread` reads `options.host`).
   Profile is source of truth. Old fields become read-through accessors or
   a one-time freeze at `Play` construction. `lowmem` stays per-slot vault
   settings, not profile identity. `mainland` stays a host session flag
   gated by the *bound* local target.

3. **OnDemand: reject mismatch before join.** Do not share a hub whose
   target, revision, or versionlist/cache identity differs. Do not start a
   second worker “just in case” when the identity is the same: keep one
   worker, subscriber counts, and socket lifetime. Pass bound transport
   into the worker so it cannot flip TCP/WSS from later `set_bot_target`.

4. **Bound adopt rejects before taking either socket.** Current
   `adopt_from` takes `other.stream` first, then copies `revision`. Bound
   path compares the frozen identity (revision, target, endpoints, RSA,
   cache/unpack) and returns `None` with both clients untouched on
   mismatch. Legacy `adopt_from` keeps copy-on-success. Do not use adopt
   to turn a 274 slot into 289.

5. **RSA and cheats are fail-closed on the bound path.** Resolve `(n, e)`
   at profile build; invalid decimals fail resolve. Do not call
   `active_biguints` (Java fallback) at handshake for bound clients.
   `cheat_allowed` / mainland must use the bound target: leftover
   `set_bot_target(Local)` plus a public-274 profile would otherwise
   allow fixture cheats on the public world.

6. **Wrong cache is not “empty tables.”** `engine_dir()` defaults to
   `$HOME/experiments/Server/engine`. `game_port_for(Local)` /
   `jag_fetch_port_for(Local)` are 274-only. `config_jag()` is
   `data/pack/config`; the 289 fixture config is
   `data/pack/client/config`. A 289 selection with ambient `ENGINE_DIR`
   pointing at 274 must fail identity validation, not unpack 274 config
   or load `~/.274bot/unpack` snapshots into 289
   (`load_snapshot_once` is currently non-fatal and process-shaped).
   Allow explicit matching copies of the same content.

7. **Vault and catalog are host identity, not client fields.** Distinct
   289 vault default (do not open/create `~/.274bot/vault` or `vault-prod`
   for local 289). `open_vault` must not create the 274 blob because the
   289 path was missing. Catalog source identity is recorded on the
   profile; the 289 refusal must not dim or drop cards. Account
   credentials stay on vault `Profile`.

8. **Parse all flags, then resolve once.** TUI `parse_args_from` bakes
   host/port/cache from `bot_target()` before `--prod`. Panel
   `Session::new` does the same via `play_endpoint_for`. After the flag
   loop: explicit CLI (`--revision`, `--profile`, `--prod`, `--host`,
   `--port`, `--cache`, `--vault`, unpack/nav overrides) > environment
   (`BOT_TARGET`, `ENGINE_DIR`, `CLIENT_UNPACK_DIR`, `NAV_PACK`,
   `LOGIN_RSAN/E`) > saved panel revision > built-in defaults. Named
   profile vs `--revision` conflict fails before vault or Play. Do not
   use `set_bot_target` as the production bind; bound slots ignore it.
   Host parse is exact `274|289`. Do not encode `!is_274()` as 289.

9. **289 bot refusal is an action gate, not a constructor skip.** Error
   token `host-boundary-not-qualified` before any current 274 writer,
   automatic guardian/run, mainland hop, or script Start. Keep
   `prepare_client_with_profile` + `maininit` + mocked handshake callable
   for tests. Login handshake is not a gameplay send.

10. **Keep the API additive.** Do not replace `ClientConfig`, change
    `from_shared` / `new` signatures, force engine presence on unrelated
    tests, put `ServerProfile` in the client crate, or migrate `LEGAL_SEND`.
    Reuse the 377 *shape* (thin host wrapper over
    `from_shared_with_revision`) as `prepare_client_with_profile`; do not
    import that host tree. `prepare_client` stays implicit 274.

## Concrete API guidance

Client crate only (no host/nav/catalog types):

```text
pub struct ClientBinding {
    pub revision: ClientRevision,      // R274 | R289 only
    pub target: BotTarget,             // Local | Prod; implies transport
    pub game_host: String,
    pub game_port: u16,
    pub asset_host: String,            // usually same host
    pub asset_port: u16,               // http_port; 80 / 1080 / 443
    pub cache_dir: PathBuf,
    pub unpack_dir: PathBuf,
    pub rsa_n: String,                 // decimal, already validated
    pub rsa_e: String,
    pub expected_crcs: Option<[i32; 9]>,
    pub versionlist_crc: Option<i32>,  // OnDemand / snapshot identity
}
```

`Arc<ClientBinding>` is cloned into every slot. Construction:

- `Client::from_shared_with_binding(Arc<ClientBinding>, cache, ifaces, ifaces_mut)`
  sets `revision`, `http_port`, OnDemand hub identity, unpack root, and
  login RSA *before* `load_on_demand`. Copies host/port/cache into
  `ClientConfig` for existing readers, but connect/HTTP/login/OnDemand/
  snapshots must not honor later `ClientConfig` mutations.
- Keep `from_shared`, `from_shared_with_revision`, `new`,
  `new_with_revision`, `new_with_revision_and_http_port` with current
  defaults. Public `ClientConfig` fields stay mutable for lowmem/members.

OnDemand hub key for the bound path:

```text
(target, revision, game_host, game_port, versionlist_crc, cache_dir)
```

Same key → existing hub (preserve `clients` / `ingame_n` / socket).
Different key occupying the same `(host, port)` with different content →
`None` / explicit error, no join, no table reuse. `new_unconnected`
unchanged.

Host crate:

```text
pub enum ServerSelection { Local274, Local289, Public274 }

pub struct ServerProfile {
    pub selection: ServerSelection,
    pub client: Arc<ClientBinding>,
    pub nav_pack: PathBuf,
    pub nav: NavAvailability,     // Loaded | Unavailable { reason }
    pub content_dir: PathBuf,
    pub catalog_id: CatalogId,    // path/hash already used by the loader
    pub vault_path: PathBuf,
}

pub fn prepare_client_with_profile(
    profile: &ServerProfile,
    uid: i32,
    cache: Arc<Cache>,
    ifaces: ...,
    ifaces_mut: ...,
    lowmem: bool,
) -> Client
```

Resolve `Arc<ServerProfile>` in one function used by panel, TUI, and
host-play after flags. Production `Play` construction takes that Arc,
runs the checked template/nav load, then `prepare_client_with_profile`
before `maininit`. Headless constructors that skip nav must record
`Unavailable`, not `Loaded` with an empty world.

Named CLI/panel ids: `local-274`, `local-289`, `public-274`. `--prod` is
`public-274`. Defaults when unset: local 274 paths and ports as now;
local 289 engine `$HOME/experiments/lostcity-289/engine` (or explicit
override), cache `{engine}/data/pack/client`, unpack
`~/.274bot/unpack-289`, vault `~/.274bot/vault-289`, nav unavailable
until step 5. Public 274 keeps `w1.rs2b2t.com:443`, baked RSA,
`unpack_dir` cache, `vault-prod`, local-only cheats.

Panel persist: add one optional revision/selection field to
`PanelUiState`. Do not persist last script. Existing UI persist
(focus/raster/lowmem/chrome) may stay. After `Play` exists, revision
widgets are display-only; changing them requires process restart.

## Test emphasis (implementer, not this review)

The proposal’s proof list is sufficient if it hits real call sites:

- Shared constructor: mock HTTP `/crc` on the bound asset port (1080 for
  local 289, not 80); login RSA bytes equal the frozen pair, not a later
  `LOGIN_RSAN` change; reconnect still uses bound host/port/transport.
- OnDemand: two bindings with the same host/port and different
  versionlist CRC must not share a worker; matching bindings still share
  one.
- Adopt: mismatched bound identities leave both streams in place.
- Frontend: parse `--revision`/`--profile`/`--prod` after all flags;
  conflict fails; persisted revision used only when CLI/env omit it;
  post-bind change refused.
- Checked vs legacy: empty-cache unit constructors still succeed; the
  frontend/host checked path refuses wrong/missing cache and missing 289
  nav as unavailable, not as 274 nav.
- 289 profile constructs and mocked-handshake tests pass; a writer,
  cheat, mainland, guardian auto, or Start returns
  `host-boundary-not-qualified` with zero `out.pos`.

Do not claim gameplay, performance, or Linux/Windows 289 engine setup.
Operator requested macOS step-3 validation on the implementation; this
document is not that run.

## What this review does not approve

- Product implementation, commits, or a selectable 289 bot session.
- Step 4 packet maps / live writers, step 5 nav bake, mixed fleets,
  public live smoke, 377, or whole-campaign Grok.
- Import of the 377 host/client trees.
- Any measured memory or performance result.
