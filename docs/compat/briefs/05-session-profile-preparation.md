# Session profile preparation

Read-only preparation for plan step 3. Do not dispatch implementation until the
client milestone reviews accept a frozen candidate. Root supplies that commit
and the final bounded implementation assignment at dispatch.

The process resolves one immutable server profile before loading shared assets
or spawning slots. The profile binds revision, local/public target, game and
asset endpoints, login RSA and CRC inputs, cache/unpack identity, navigation and
content identity, vault path, and catalog identity. Keep per-account credentials
and existing per-slot settings separate. All slots, reconnects, frontend labels
and script starts use the resolved profile. An override that conflicts with its
revision or identity must fail before mutation or connection.

## Observed seams in the campaign base

- `host::prepare_client` wraps the default-274 `Client::from_shared`.
  The audited 377 host commit `a802ce1094f35d1257abc5f8b497f17675af3dde`
  adds a suitable `prepare_client_with_revision` wrapper. Reuse the narrow
  pattern; do not import that host's older baseline or claim 377 operation.
- `PlayOptions` currently carries host, game port, cache directory, lowmem and
  mainland. `Play::new` decodes one cache/interface template and stores cloned
  options. `spawn_slot_thread` constructs the shared client, calls `maininit`,
  then enters its existing login queue/backoff loop. Bind the profile at this
  actual construction point, preserving shared `Arc` identity.
- `from_shared_with_revision` initializes the web port from the process target.
  Unlike standalone `new_with_revision_and_http_port`, it does not receive the
  local fixture's asset port. `maininit` fetches `/crc`; local 289 uses 1080,
  not the current default 80. A revision-only constructor change is incomplete.
- `Client::login` calls `login_rsa::active_biguints` on each handshake.
  HTTP helpers and `ClientStream::connect` select transport from the global
  target. The unpack snapshot root also comes from the environment. Trace
  OnDemand construction and reconnect too. A resolved host profile must bind
  these inputs without introducing a host dependency into the client or
  changing legacy standalone defaults. Use a general client connection/resource
  seam where necessary, with immutable values shared by the host's slots.
- `Play::new` reads the nav pack from `default_pack_path` and currently ignores
  load errors. `load_template` catches parse errors and substitutes empty tables.
  A selected profile must reject a revision/resource mismatch explicitly;
  replacing a required asset with an empty default is not qualification.
- Panel `Session::new` constructs default options. TUI `parse_args_from`
  resolves defaults before the CLI `--prod` override, then main adjusts paths.
  Apply one consistent precedence rule after parsing explicit overrides.
  Preserve existing 274 defaults and explicit paths, including Windows home
  handling. Select 289 before sessions start; changing an active profile
  requires process restart. Persist revision selection only, not last script.

## Pinned fixture identities

Use `fixture-inputs.json` and `fixture-inputs.md`, not guessed cache locations.
Local 274 is game 43594 / asset 80; local 289 is game 44594 / asset 1080.
Both config archives are under the engine's `data/pack/client/config`.
274 navigation is legacy v8 and needs rebaking; 289 navigation is not yet
qualified. Step 5 owns the bake/content/guardian implementation. Do not fall
back to 274 navigation for a 289 profile in the meantime.

The existing public profile's known pairing is the 274 target in current
configuration. Do not infer a public 289 server or reuse 274 assets/RSA for it.
Public configuration must require an explicit known revision/asset pairing and
retain the local-only fixture-cheat restriction.

## Required proof and limits

Use tests through real frontend parsing/session creation, shared template
construction, and client setup. Cover default 274, explicit 274, explicit 289,
revision/profile conflicts, unsupported revisions, resource mismatches, and
immutability across spawn/reconnect. Verify shared cache/interface/profile
identity without new deep copies. Test the selected HTTP endpoint and login
binding through actual construction, not only struct field assertions.

Step 4 owns direct host writers and host live login/action qualification; step 5
owns nav/content qualification. Those gates remain pending even if step 3's
construction tests pass. Do not report a selectable 289 configuration as proved
bot gameplay or silently send the current raw 274 host packets on it.
