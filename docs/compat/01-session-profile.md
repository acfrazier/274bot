# Step 3: immutable server profiles

Implementation is in progress on `codex/rs2b0t-multirevision` and
`codex/bothost-274-289`. This report records the host backend first; client,
frontend, native macOS and independent review gates remain open. Step 3 is not
accepted yet. The architecture review is
`reviews/session-profile-design-grok46.md` (card `t_bfd3339c`, actual Grok 4.6).

## Host behavior

Both frontends use one shared argument parser and resolve a launch selection
before loading assets or mutating a vault. Explicit CLI inputs override the
environment, saved revision, and defaults. A named CLI profile must agree with
an explicit CLI revision and local/public selection. Unsupported revisions,
invalid ports, conflicting selections and unqualified public pairings return
an error. Relative paths are anchored to the captured working directory.

`ProfileSelection::bind` reads and validates all eight JAG identities, expected
CRCs, RSA public values, navigation pack/flags identity and selected script
catalog. It returns an immutable `Arc<ServerProfile>` with a shared client
binding. `SharedClientTemplate::load` validates and decodes the selected cache
and interfaces once, loads the selected navigation world once, and shares
those Arcs with real slot clients through `host::prepare_client_with_profile`.
Missing or malformed required cache tables are errors. The legacy additive
entry points remain available for existing callers and fixtures.

The production `Play` owns the template/profile rather than a second editable
copy of connection fields. Slot construction uses the same binding before
OnDemand setup, maininit, login and reconnect. A bound initialization failure
appears in the slot status before login. Public fixture-cheat checks read the
bound target. Script starts validate the captured catalog identity. Navigation
scatter uses one seed vector from the selected shared world.

| Profile | Game / asset port | Default cache | Default unpack / nav / vault under `~/.274bot` |
|---|---|---|---|
| local-274 | 43594 / 80 | `~/experiments/Server/engine/data/pack/client` | `unpack`, `274bot.navpack`, `vault` |
| local-289 | 44594 / 1080 | `~/experiments/lostcity-289/engine/data/pack/client` | `unpack-289`, `289/274bot.navpack`, `vault-289` |
| public-274 | 443 / 443 | `~/.274bot/unpack` | `unpack`, `274bot.navpack`, `vault-prod` |

Public 274 requires the known `w1.rs2b2t.com` game/asset pairing. Public 289 is
unavailable. Explicit resource-path overrides remain supported, with identity
validation. The bundled cache manifests identify the two inventoried local
fixtures. A new server/cache build needs an explicitly prepared manifest; the
application does not silently assign an unknown cache to the selected revision.
For a cache whose server revision has been established:

```sh
cargo run --locked -p host-play --example cache_manifest -- 289 /path/to/client-cache > /path/to/cache-289.json
# Add --cache /path/to/client-cache --cache-manifest /path/to/cache-289.json to launch flags.
```

The manifest tool hashes all eight archives and records the supplied revision.
Its output is an operator assertion about the prepared server/cache pairing,
not gameplay qualification. Maininit also checks the server's actual CRC table
against the frozen local identity before login.

A revision 289 navigation pack requires a `<pack>.json` manifest containing
`revision`, `cache_id`, `nav_sha256`, and optional `flags_sha256` (the exact
`NavManifest` schema). Existing 274 packs remain supported without this new
sidecar. A missing selected pack is explicitly unavailable; it never loads the
other revision's default. Actual 289 baking and qualification belong to step 5.

## Verification in progress

`crates/host-play/tests/session_profile.rs` uses authored synthetic JAG files,
with provenance and generator under `tests/fixtures/profile`. The fixtures
exercise real config/interface decoding and client construction; they do not
represent game content or a live session.

- Nine focused tests pass: parser/precedence, negative inputs, cache/nav mismatch,
  bad RSA, changed resources/catalog, both real revision constructors and shared
  Arcs, 289 refusal before slot/queue mutation, public cheat refusal, and selected
  shared navigation/scatter with flags validation.
- The initial affected API/host/host-play gate with `host-play/memory-profile`
  passed 448 tests with zero failures and seven ignored live tests. The final
  focused test additions and path anchoring passed separately. A coherent final
  gate will follow client/frontend integration.
- Strict Clippy passed for the three affected host crates with `--no-deps`.
  The earlier dependency-inclusive attempt identified an in-progress client
  constructor argument-count lint, handed back to the client implementer.
- The first synthetic constructor run exposed an incorrect expected default
  hostname in the test (`localhost` versus preserved `127.0.0.1`). Correcting
  that assertion passed. The initial failure log remains in the evidence folder.
- The real profile asset checks initialized both local fixtures on macOS,
  including matching server CRCs and shared client bindings. Revision 274 loaded
  its existing snapshot/nav; 289 used its separate cache/HTTP/unpack and reported
  navigation unavailable plus the expected bot-operation gate. These are
  development-candidate asset checks, pending the final integrated candidate.
  The first 289 engine launch exited after readiness; its probe was terminated
  and preserved as a failed preparation attempt. Keeping the launcher session
  alive allowed the new engine PID91467 to remain ready before and after the
  successful asset check. No account login or gameplay acceptance is claimed.
  The existing 274 engine PID1852 was left running.

Run the reusable asset diagnostic with `cargo run --locked -p host-play --example profile_check -- --revision 289 --initialize` (omit `--initialize` for construction only). It does not start account slots.

Raw logs: `evidence/session-profile/host/`. Client-specific tests and lifecycle
proof belong to the client report `docs/revision-289/session-profile-client.md`.
Frontend construction and presentation checks will be recorded in
`01-session-profile-frontends.md` and this report's final acceptance update.

## Remaining gates

Revision 289 construction is available for protocol qualification. Production
slot/script operation returns `host-boundary-not-qualified` before mutations
until step 4 qualifies the host action/snapshot boundary. No 289 navigation or
catalog/gameplay row is accepted by this step. The client and frontend same-card
Grok 4.5 reviews, host review, integrated Grok 4.6 review, coherent checks, and
native macOS frontend validation remain pending.

The operator requested macOS step-3 validation first. Linux and Windows need
separate 289 engines configured before later platform checks; neither platform
is qualified by these results. No remote push, main merge, release tag, or fresh
recursive-fetch acceptance has occurred.
