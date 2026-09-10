# Step 3 integrated server profile review

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-10. Kanban card `t_80dd7fd0`.
Kind: independent Grok 4.6 coherent integration milestone for step-3
immutable server profiles. Not a same-card Grok 4.5 task review, not
native macOS frontend acceptance, not step-4 host-boundary qualification,
not the final whole-campaign `branchreviewer` pass.

Read once: `AGENTS.md`, `docs/execution.md`, plan architecture plus step 3
in `docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md`,
`docs/compat/01-session-profile.md`, `docs/compat/01-session-profile-frontends.md`,
brief `09-session-profile-integration-review.md`, and frozen
`reviews/session-profile-integration-inputs.json`. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Work was read-only except this
report. No product edits, subagents, live/native launches, commits, merge,
push, or client gitlink movement.

Review was cold against the exact manifest inputs. Existing full
client/GPU/backend and final frontend suites were not rerun; their raw
receipts hashed as declared. One focused host-play `session_profile`
reproduction was run for combined bind/refusal risk.

## Verdict

**APPROVE the integrated source candidate** at host product head
`a02c9dd531124a5bb8dfb936bed20bd92e52730d` with reviewed client pin
`2be1697060e4d2b8b709ad4d5e54d12513b38333` (product `c8e61557`).

This is **source approval** of the combined client binding, host
profile/template/Play ownership, and panel/TUI/host-play production paths.
It is **not** native/evidence acceptance: root still owns macOS panel
visual proof and TUI PTY startup. 289 bot operation remains explicitly
unqualified until step 4. Linux/Windows engines remain unqualified. This
milestone does not replace the final Grok 4.6 whole-branch review.

## Inspected refs

| Role | Exact value |
|---|---|
| Branch | `codex/rs2b0t-multirevision` (not `main`) |
| Frozen metadata HEAD (inputs only) | `5e51e86a0bc5e3531ab202e8d369917a977fde3c` |
| Host product base | `5bb500262fb4e7ab8507589c0784a36f9eac14ce` |
| Host product head | `a02c9dd531124a5bb8dfb936bed20bd92e52730d` |
| Client base | `58120f28ee5208553ca07f41cb364f2cf98ea280` |
| Client head / host gitlink | `2be1697060e4d2b8b709ad4d5e54d12513b38333` |
| Client product | `c8e61557bf5b208eb4ffb589fd0a797fbf7d42ab` |
| Frontend same-card commit | `7aaa8c390533fa0cd994cca7201ea739865ef94d` |
| Catalog capture helper | `08e8d9f25cdbdf646fd905f607049aeed5a7ecb1` |
| Bound fixture-button follow-up | `a02c9dd531124a5bb8dfb936bed20bd92e52730d` |
| Brief SHA-256 | `fd90af3f95353e0a88226fd3242fd054e7fae2380210e75c4d7d7de822c8290f` |
| Inputs JSON | `docs/compat/reviews/session-profile-integration-inputs.json` |
| Reviewer model/provider | `grok-4.6` / `xai-oauth` |

Confirmed: `git branch --show-current` = `codex/rs2b0t-multirevision`;
`git rev-parse HEAD` = `5e51e86a`. `git diff --name-only a02c9dd5 HEAD`
is docs/evidence/inputs only. Host gitlink at `a02c9dd5` and HEAD is
`2be16970`. Client `c8e61557..2be16970` is docs-only. Named
`host_product_changes` match the crate product diff; extras in the range
are `Cargo.lock`, `vendor/fr-client-rust`, and campaign docs/evidence.

## Prerequisite actual-model reviews

| Lane | Card / run | Receipt |
|---|---|---|
| Client | `t_1b93f768` run 1092 | `session-profile-client-grok45.json`; actual `grok-4.5` / `xai-oauth` session `20260910_114843_501760`; approved `c8e6155` + docs `2be1697` |
| Host backend | `t_30007871` run 1091 | `session-profile-host-grok45.json` + `.md`; actual `grok-4.5` / `xai-oauth` session `20260910_114743_5b127f`; approved `0cc8b0b7` (`932e480a`+`0cc8b0b7`) |
| Frontends | `t_879d9607` run 1093 | `session-profile-frontends-grok45.json`; actual `grok-4.5` / `xai-oauth` session `20260910_123250_73d488`; approved `7aaa8c39` |

All three SHA-256 values match the frozen manifest. Design contract
`session-profile-design-grok46.md` (card `t_bfd3339c`) remains the
ownership baseline this integration is judged against.

## Evidence hashes

All 57 files listed in `inputs.json` `evidence[]` were hashed with
`shasum -a 256`. Every SHA-256 matched the frozen value, including empty
`diff-check.log` / `fmt.log` (`e3b0c442…`). Declared suite counts were
not recomputed from scratch; they are accepted as hashed receipts:

- client workspace 1001/0/2; explicit GPU 2/0
- api/host/host-play + memory-profile 450/0/7
- catalog-capture focused 9/0
- panel memory-profile 384/0; TUI 89/0; host-play 131/0/7
- bound fixture-button tests 2/0
- real CLI negatives on `7aaa8c39`: 9 passed, 0 vaults created
- final integrated binary build pass on `a02c9dd5` with all three
  memory-profile features (`integrated-binaries.json`)

## Contract mapping

| Criterion | Combined evidence |
|---|---|
| CLI > env > saved > default | Shared `parse_profile_args` then `ProfileOptions::resolve_with_env`: named CLI profile wins; CLI revision > env named > `BOT_REVISION` > saved panel revision > 274. Relative paths anchored to captured cwd. Host-play/panel/TUI consume this parser before frontend flags. Test `flags_are_order_independent_and_override_saved_or_environment_revision`. |
| Negative named/revision/target | Unsupported revision (`377` etc.), port 0, `--prod`+289, profile/revision conflict, local profile+`--prod`, public-274 host/port drift fail in resolve before vault. CLI receipts: 9/9, `vault_created: false`. Panel `invalid_profile_refuses_vault_reset_without_deleting_explicit_path`. |
| Unchanged 274 defaults; distinct 289 paths | local-274 `43594/80`, `unpack`, `274bot.navpack`, `vault`; local-289 `44594/1080`, `unpack-289`, `289/274bot.navpack`, `vault-289`; public-274 `w1.rs2b2t.com:443/443`, `vault-prod`. Public 289 has no pairing. |
| Bind and checked assets before vault/slot/OnDemand/HTTP/login | `ProfileSelection::bind` validates eight JAGs, known/explicit cache identity, CRC, RSA, nav/flags, catalog. `SharedClientTemplate::load` decodes cache/ifaces once and loads selected nav. host-play `main`, TUI `run`, panel `start_vault` / `boot_execute` bind+load then `require_bot_operation` then vault open/upsert. |
| One immutable process profile and shared Arcs | `ServerProfile` getters only; `Arc<ClientSessionProfile>` cloned into slots via `prepare_client_with_profile` → `Client::from_shared_with_profile`. Template cache/ifaces/world/scatter seed are shared Arcs/`OnceLock`. Panel `server_profile` is process-lifetime after first bind; rebind and `set_server_revision` return restart-required. Lock/unlock of the vault does not construct a second profile. |
| No second effective endpoint config | Production `Play` is `PlayConnection::Bound { template, mainland }` only. Slot thread uses `template.prepare_client`, not `PlayOptions.host/port/cache`. `ProfilePlayOptions` carries profile + mainland. Legacy `PlayOptions` remains for old `run_with_io` callers. After bind, panel/TUI copy host/port/cache into a snapshot used for debug chrome; spawn still uses the template. |
| Panel selection before session; labels; CLI/env precedence | Locked UI shows `server_label` / `effective_revision_label` from unresolved selection. Combo text is "Revision before session" until bound, then "Bound session revision". Explicit CLI/env beat saved `server_revision` (migrates to 274). Post-bind change refused. |
| Invalid selection never resets fallback vault | `vault_path()` returns `Err` on resolve failure; `unlock`/`reset_vault` surface the error and do not call `default_vault_path()`. |
| TUI title uses selected profile | `TuiApp::new(format!("… · {}", session.profile_label()))` after `new_bound`. |
| Live/memory entry points use checked construction | Panel `boot_execute` and TUI `run` bind/load/require before `memory::Run::prepare_unseeded` or live unlock. |
| Cache/CRC/RSA/nav/flags/catalog retained and validated | Bind freezes identities; `validate_resources` / `validate_catalog` on later start. Maininit `fetch_jag_checksums` fail-closed on bound `expected_crc` mismatch without overwriting. Bound RSA from profile `rsa_biguints`, not `active_biguints`. |
| Wrong/missing 289 nav never falls back to 274 | Missing pack → `NavAvailability::Unavailable` (not the other default). 289 pack without sidecar → error. 274 without sidecar → `Legacy274`. Scatter seed from the selected shared world. |
| Catalog custom scripts remain on mismatch | `CatalogIdentity::capture` (`08e8d9f2`) used at bind and pre-bind Browse/warmup. Bind refuses root/source-hash change, including same-root edits, without clearing File cards. Test `prebind_explicit_catalog_beats_ambient_and_mismatch_preserves_custom_cards`. |
| No deep-copy regressions | Template load is one decode; slots clone Arcs. Scatter vector once per template. Additive client constructors unchanged. |
| Production 289 refuse before queue/status/vault | `HOST_BOUNDARY_NOT_QUALIFIED` on `try_spawn_slot`, script start/load, cheat, wire, non-empty `run_with_template`/`run_with_profile`, host-play main after template load, TUI `run` after bind, panel `boot_execute`/`start_vault` before `open_vault`. Empty bound Play/construction remains. Test `revision_289_refuses_slots_and_scripts_before_arms_or_queue_mutation`. CLI 289 host-play/tui-play: error fragment `host-boundary-not-qualified`, no vault file. |
| Bound 289 construction available | `SharedClientTemplate::prepare_client` / `from_shared_with_profile` remain callable; gate is the action/vault path, not the constructor. |
| Public cheats refuse | `Driver::session_target` on bound `Client` uses frozen target. `Play::cheat` uses `connection.target()` + 289 gate. Test mutates `client.config` toward local and still refuses. Fixture buttons after `a02c9dd5` use `debug_main_buttons_for(session.target(), …)`. |
| Client owns protocol/world/render; no host dependency | Client crate Cargo.toml has no host/nav/catalog/script deps. Host wrapper is `prepare_client_with_profile` over `from_shared_with_profile`. No 377 import. |
| Legacy APIs/fixtures retained | `Play::new` / `run_with_io` / `prepare_client` / ambient `from_shared` keep current defaults. Bound adopt compares identity before taking either stream. |

## Follow-ups in this integration range

`08e8d9f2` exposes `CatalogIdentity::capture` and uses it in `bind`. Panel
pre-bind Browse/warmup and same-root edit refusal call that helper.

`a02c9dd5` routes panel debug fixture controls through
`debug_main_buttons_for(session.target(), …)` and makes `Session::target`
`pub(crate)`. Existing local/public button tests passed (hashed receipts).
Production debug chrome after bind follows the bound target, not leftover
`BOT_TARGET`.

## Independent checks run by this reviewer

1. Branch / HEAD / named base-head / gitlink / client product identity as above.
2. SHA-256 of all 57 frozen evidence files; all matched.
3. Cold read of host product crate diff `5bb50026..a02c9dd5`, client
   `58120f28..2be16970`, plus `profile.rs`, `PlayConnection`,
   `prepare_client_with_profile`, panel bind/unlock/catalog/debug,
   TUI `run`/`new_bound`, client `session.rs` / `from_shared_with_profile`
   / OnDemand hub identity / adopt / login RSA / asset HTTP.
4. Re-ran:

```text
CARGO_TARGET_DIR=/Users/acfrazier/experiments/274bot/target \
  cargo test --locked -p host-play --test session_profile
# 9 passed; 0 failed; finished in 0.29s
```

Did not launch live/native panel or TUI. Did not rerun the 1001/450/384/89
suites.

## Findings

No blocking combined-ownership, immutability, refusal-before-mutation, or
frontend-path defects against the step-3 integration contract.

Non-blocking notes (do not reopen this source gate):

1. Leftover ambient helpers remain on **legacy** surfaces by design:
   `debug_main_buttons` still wraps `bot_target()` but is unused after
   `a02c9dd5`; TUI `validate_startup_host` still consults ambient target
   and is not on the production `run()` path (resolve already validates
   the selected target). `http_get` / `ClientStream::connect` keep ambient
   TLS; bound clients use `http_get_for` / `connect_for`.
2. Panel `debug_ui` still keys off the `Session.options.host` snapshot.
   Bind writes that snapshot from the frozen profile, so post-bind debug
   chrome agrees. Pre-bind locked debug chrome can still follow the
   Session::new ambient host until bind.
3. OnDemand hubs remain keyed by `(host, port)` and **reject** identity
   mismatch before join, then share the same worker. That matches the
   design constraint; it is not a second worker and not table reuse.
4. CLI negatives were recorded on `7aaa8c39`. `a02c9dd5` does not change
   parse/bind. Panel 289 refusal is in `boot_execute`/`start_vault`, not
   in those nine preconnect CLI cases; host-play and tui-play cover the
   binary 289/vault-negative.
5. `PlayConnection::Legacy` stays ungated. Production panel/TUI/host-play
   entry points configure and bind; they do not take that path.

## What this review does not approve

- Native macOS panel visual proof or TUI PTY startup (root after this gate).
- Step-4 host writers/guardians, step-5 289 navigation, catalog gameplay rows.
- Linux/Windows 289 engines, public live smoke, 377 gameplay, publication.
- Whole-campaign `branchreviewer` pass.
- Any memory/performance claim.

## Approval metadata

```text
review_outcome: approved_source
native_acceptance: not_this_review
review_round: 1
review_lens: artifact + hashed receipts + focused session_profile reproduction
task_id: t_80dd7fd0
branch: codex/rs2b0t-multirevision
host_base: 5bb500262fb4e7ab8507589c0784a36f9eac14ce
host_product_head: a02c9dd531124a5bb8dfb936bed20bd92e52730d
metadata_head: 5e51e86a0bc5e3531ab202e8d369917a977fde3c
client_base: 58120f28ee5208553ca07f41cb364f2cf98ea280
client_head: 2be1697060e4d2b8b709ad4d5e54d12513b38333
client_product: c8e61557bf5b208eb4ffb589fd0a797fbf7d42ab
prerequisite_reviews: t_1b93f768/1092, t_30007871/1091, t_879d9607/1093 (actual grok-4.5)
reviewer_profile: grok46
reviewer_model: grok-4.6
reviewer_provider: xai-oauth
product_code_edited_by_reviewer: false
subagents: false
live_native_launches: false
evidence_hashes_matched: 57/57
focused_tests: 9 passed / 0 failed (reviewer re-run host-play session_profile)
report: docs/compat/reviews/session-profile-integration-grok46.md
```
