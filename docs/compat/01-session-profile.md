# Step 3: immutable server profiles

Correction (2026-09-10): the public rs2b2t service moved to revision 289.
`public-289` is the supported public profile and requires the known
`w1.rs2b2t.com:443` game/asset pairing. Public revision 274 is unavailable.
The earlier step-3 receipts below remain historical evidence for the assumption
in force when they ran; local 274/289 construction evidence is unaffected.

Step 3 is accepted on macOS on `codex/rs2b0t-multirevision` and
`codex/bothost-274-289`. Client, host backend and frontend same-card Grok 4.5
reviews and the integrated Grok 4.6 source review are approved. Native panel
selection/binding and local 274 login passed; the actual macOS TUI PTY reached
scene 2 and exited cleanly. Final host product is `41d896b3`, client pin
`2be16970`. Linux/Windows and the remaining campaign steps are still open.
The architecture review is
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
| public-289 | 443 / 443 | `~/.274bot/unpack-289` | `unpack-289`, `289/274bot.navpack`, `vault-prod` |

Public 289 requires the known `w1.rs2b2t.com` game/asset pairing. Public 274 is
unavailable. Explicit resource-path overrides remain supported, with identity
validation. The bundled cache manifests identify the two inventoried local
fixtures. A new server/cache build needs an explicitly prepared manifest; the
application does not silently assign an unknown cache to the selected revision.

The correction's focused checks cover named CLI and environment profiles,
`--prod`, environment targets, saved local preferences, explicit revision
conflicts, all game/asset host and port drift, revision-derived resources,
explicit resource paths, unchanged local defaults, frontend resolution, and
the bound-target fixture-cheat refusal. Public assets remain an explicit setup
requirement; no public login or gameplay qualification was attempted. Raw logs
are under `evidence/public-profile-correction/`: all ten host profile tests and
the focused host-play, panel and TUI frontend tests pass; the three affected
crates pass `cargo check`, formatting is clean, and `git diff --check` passes.

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

## Verification

`crates/host-play/tests/session_profile.rs` uses authored synthetic JAG files,
with provenance and generator under `tests/fixtures/profile`. The fixtures
exercise real config/interface decoding and client construction; they do not
represent game content or a live session.

- Nine focused tests pass: parser/precedence, negative inputs, cache/nav mismatch,
  bad RSA, changed resources/catalog, both real revision constructors and shared
  Arcs, 289 refusal before slot/queue mutation, public cheat refusal, and selected
  shared navigation/scatter with flags validation.
- The affected API/host/host-play gate with `host-play/memory-profile` passed
  450 tests with zero failures and seven ignored live tests on client product
  `c8e61557`. The initial 448-test receipt is retained. Frontend checks are recorded below.
- The full client workspace on product `c8e61557` passed 1,001 tests with zero
  failures and two ignored GPU tests. Both explicit GPU tests then passed on
  the same product with `SKIP_GPU` absent.
- Strict Clippy passed for the three affected host crates with `--no-deps`.
  The earlier dependency-inclusive attempt identified an in-progress client
  constructor argument-count lint, handed back to the client implementer.
- The first synthetic constructor run exposed an incorrect expected default
  hostname in the test (`localhost` versus preserved `127.0.0.1`). Correcting
  that assertion passed. The initial failure log remains in the evidence folder.
- The real profile asset checks initialized both local fixtures on macOS,
  including matching server CRCs and shared client bindings. Revision 274 loaded
  its existing snapshot/nav; 289 used its separate cache/HTTP/unpack and reported
  navigation unavailable plus the expected bot-operation gate. The final
  integrated constructor repeated both asset checks successfully on `41d896b3`;
  its receipts are in `evidence/session-profile/native-macos/profile-assets.json`.
  The first 289 engine launch exited after readiness; its probe was terminated
  and preserved as a failed preparation attempt. Keeping the launcher session
  alive allowed the new engine PID91467 to remain ready before and after the
  successful asset check. No account login or gameplay acceptance is claimed.
  The existing 274 engine PID1852 was left running.

Run the reusable asset diagnostic with `cargo run --locked -p host-play --example profile_check -- --revision 289 --initialize` (omit `--initialize` for construction only). It does not start account slots.

Raw logs: `evidence/session-profile/host/`. Client-specific tests and lifecycle
proof belong to the client report `docs/revision-289/session-profile-client.md`.
Frontend construction checks are in `01-session-profile-frontends.md`: 384 panel
and 89 TUI tests pass with memory-profile; host-play passes 131 tests with seven
live tests ignored. Final launch binaries built with all three memory-profile
features. Nine actual binary negatives reject unsupported revisions, conflicts,
wrong cache pairing and unqualified 289 operation without creating vaults.

Frontend review accepted `7aaa8c39`; root follow-up `a02c9dd5` routes the remaining
fixture-button presentation helper through the bound target. Existing local and
public button tests passed. This follow-up and the catalog identity helper are
included in the completed integration review. The reviewed source candidate is
`a02c9dd5`; its frozen binary hashes and build command are in
`evidence/session-profile/frontends/integrated-binaries.json`.

## macOS acceptance

Actual Grok 4.6 / xai-oauth completed card `t_80dd7fd0`, run 1094, with source
approval at host `a02c9dd5` and client `2be16970`. It independently verified all
57 frozen evidence hashes and passed the nine profile tests. The report and
actual session receipt are `reviews/session-profile-integration-grok46.{md,json}`.

Native inspection then exposed clipped server/revision text in the narrow
panel pane. Root correction `41d896b3` wraps the server label and places the
revision caption above the combo. This presentation-only change passed diff,
format, build and visual checks under the proportional verification guidance;
it follows the integrated source review. Its binary hash/build receipt is
`evidence/session-profile/native-macos/layout-build.json`.

| Native check | Result |
|---|---|
| Default panel selection | Complete local-274 server label and revision 274 visible before binding |
| Select 289 before binding | Effective server changes to local-289, port 44594; saved revision becomes 289 |
| Attempt 289 startup | Explicit host-boundary refusal, no slots, synthetic vault SHA unchanged |
| Change revision after binding | Active/saved 289 retained; visible restart-required error |
| Explicit 274 over saved 289 | Effective revision 274 shown before binding, then retained by the live slot |
| Real panel client | Local 274 Login reaches visible `ingame scene 2`, tile 3094,3106; rendered game and bound profile captured |
| Real TUI client | Local 274 title/endpoint, `ingame scene 2`, then `q` and exit 0 in the macOS PTY |
| Both final asset constructors | 274 and 289 initialize with matching CRC and shared cache/binding; distinct endpoints and expected nav availability |

Ten original F12 PNGs were inspected directly, including the initial clipped
layout and the title-screen return after panel Logout. Capture mapping, hashes,
exact launch arguments and limits are in
`evidence/session-profile/native-macos/proof.json`. F12 deliberately writes a
default snapshot JSON, so those sidecars do not establish scene readiness.
The rendered game and native status text provide the panel observation.
The title-screen capture retains the status label `logging in...`; this proof
does not qualify reconnect/lifecycle behavior.

CUA denied access to iTerm2, so TUI evidence is an actual local PTY transcript,
without a native terminal-window visual claim. The first reader missed cursor
sequences between the state words; its failed receipt is preserved. A fresh run
with the same-row cursor-aware reader passed and exited 0. The original panel
preferences were restored byte-for-byte, proof apps closed, and owned 289
engine PID91467 stopped after the final asset checks. Existing 274 PID1852
remained listening. The account and vault were created for this local proof.

## Remaining gates

Revision 289 construction is available for protocol qualification. Production
slot/script operation returns `host-boundary-not-qualified` before mutations
until step 4 qualifies the host action/snapshot boundary. No 289 navigation or
catalog/gameplay row is accepted by this step. The client and host backend same-card Grok 4.5 reviews are approved (cards
`t_1b93f768` and `t_30007871`; completed actual-model receipts in `reviews/`).
The host pins the reviewed client candidate `2be1697060e4d2b8b709ad4d5e54d12513b38333`
locally; its product commit is `c8e61557`. Root owns the gitlink and integration.
The frontend same-card Grok 4.5 review is approved (card `t_879d9607`, actual
session `20260910_123250_73d488`). Integrated Grok 4.6 source review and macOS
frontend validation are complete. Captured catalog identity also prevents
pre-bind Browse/warmup from retaining a different root or edited source. Custom
file cards remain present when binding refuses a catalog mismatch.

The operator requested macOS step-3 validation first. Linux and Windows need
separate 289 engines configured before later platform checks; neither platform
is qualified by these results. No remote push, main merge, release tag, or fresh
recursive-fetch acceptance has occurred.
