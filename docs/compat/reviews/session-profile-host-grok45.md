# Step 3 host immutable profile backend review

Reviewer: Hermes profile `reviewer`, model `grok-4.5`, provider `xai-oauth`.
Date: 2026-09-10. Kanban card `t_30007871`. Round 1, artifact-first lens with
independent focused re-execution.
Kind: same-card host-backend review only. Not client review (`t_1b93f768`), not
frontend (`t_879d9607`), not coherent integration, not Grok 4.6 milestone, not
step-4 host-boundary qualification, not native Mac frontend acceptance.

Work was read-only on product sources. Deliverable of this review is this
report only. No product edits, merge, push, engine stop/restart, or
Linux/Windows work.

## Verdict

**APPROVE** the host backend at host commit
`0cc8b0b7d77ecbb533f9b0be7669b8967f1dc256` on branch `codex/rs2b0t-multirevision`
(implementation commits `932e480a` + `0cc8b0b7`), scoped to the host backend
card contract. Step 3 as a whole remains unaccepted until client, frontend,
coherent, and Grok 4.6 gates close.

## Inspected refs

| Role | Exact value |
|---|---|
| Branch | `codex/rs2b0t-multirevision` (not `main`) |
| Host HEAD / named commit | `0cc8b0b7d77ecbb533f9b0be7669b8967f1dc256` |
| Host implementation base | `932e480a` Bind host sessions to immutable server and resource profiles |
| Host exercise commit | `0cc8b0b7` Exercise bound local asset startup for both revisions |
| Host report | `docs/compat/01-session-profile.md` |
| Design contract | `docs/compat/reviews/session-profile-design-grok46.md` |
| Working-tree client dependency (handoff) | `c8e61557bf5b208eb4ffb589fd0a797fbf7d42ab` |
| Host gitlink at HEAD (still prior accepted) | `58120f28ee5208553ca07f41cb364f2cf98ea280` |
| Client WT tip observed | `2be1697060e4d2b8b709ad4d5e54d12513b38333` (docs on top of `c8e6155`) |
| Reviewer model/provider | `grok-4.5` / `xai-oauth` |

Confirmed before inspection: `git branch --show-current` =
`codex/rs2b0t-multirevision`; `git rev-parse HEAD` =
`0cc8b0b7d77ecbb533f9b0be7669b8967f1dc256`. Host-scope paths under
`crates/{api,host,host-play}` and `docs/compat/01-session-profile.md` are clean
at HEAD. Concurrent dirty panel/tui files and untracked host evidence logs are
outside this card’s product scope.

## Scope reviewed

- `crates/api/src/interact.rs` — `Driver::session_target`, bound cheat/mainland policy
- `crates/host/src/lib.rs` — `prepare_client_with_profile`
- `crates/host-play/src/profile.rs` — selection, bind, manifests, gates
- `crates/host-play/src/lib.rs` — `SharedClientTemplate`, `PlayConnection`, production spawn/script/cheat/wire gates
- `crates/host-play/src/scatter.rs` — selected shared world seed vector
- `crates/host-play/src/known-cache-identities.json`
- `crates/host-play/examples/{cache_manifest,profile_check}.rs`
- `crates/host-play/tests/session_profile.rs` + synthetic fixtures
- `docs/compat/01-session-profile.md` and `docs/compat/evidence/session-profile/host/`

Not reviewed as acceptance: panel/tui WIP, client product correctness beyond the
host call-surface dependency, live account login/gameplay, engines.

## Contract mapping

| Contract item | Evidence |
|---|---|
| CLI / env / saved precedence | `ProfileOptions::resolve_with_env`: named CLI profile wins; CLI revision > env profile revision > `BOT_REVISION` > saved > 274 default. Order-independent parse keeps frontend rest args. Relative paths anchored to captured working dir. Test `flags_are_order_independent_and_override_saved_or_environment_revision`. |
| Negative pairs | Unsupported revision, missing values, port 0, `--prod`+289, profile/revision conflict, local profile+`--prod`, public-274 host/port drift all fail before vault/unpack creation. Test `invalid_revision_public_pairing_and_conflicts_fail_before_vault_access`. |
| Immutable path/transport/revision/cache/RSA/CRC/nav/catalog/vault | `ProfileSelection::bind` freezes `Arc<ClientSessionProfile>` with hosts/ports/cache/unpack/RSA/expected CRCs/content id; host `ServerProfile` freezes nav/catalog/vault/content. Getters only on `ServerProfile`. |
| Shared resource ownership | `SharedClientTemplate::load` decodes cache/ifaces once; `prepare_client` shares Arcs; nav world shared; scatter seed vector once per template. Test `both_revisions_reach_real_shared_client_constructor_and_keep_the_binding` + nav/scatter test. |
| Actual host client construction | `host::prepare_client_with_profile` → `Client::from_shared_with_profile`. Production bound slots use template path in `spawn_slot_thread`, not ambient `bot_client_config`. Init failure recorded on slot status before login. |
| Production slot / reconnect binding | Bound `Play` stores `PlayConnection::Bound { template, mainland }` only (no second mutable host/port/cache copy). Slot thread rebuilds via `template.prepare_client`; reconnect flag only affects `client.login(..., reconnect)` on the already-bound client. |
| Bound public cheat refusal | `api::interact::{cheat,mainland_hop}` use `driver.session_target()`. Test mutates `client.config` toward local and still refuses. Play cheat path also uses `connection.target()` + 289 gate. |
| Explicit 289 action gate before mutation | `require_bot_operation` → `HOST_BOUNDARY_NOT_QUALIFIED` on spawn/script start/load/handle, cheat, wire, and non-empty `run_with_template`/`run_with_profile`. Constructor + empty Play allowed. Test `revision_289_refuses_slots_and_scripts_before_arms_or_queue_mutation` asserts empty statuses/queue and no vault create. |
| Cache / nav mismatch fail-closed | Declared manifest or known-identity match required; archive/revision drift errors; 289 nav requires sidecar; missing pack is `Unavailable`, not the other revision’s default. Tests cover cache/nav mismatch, flags change, catalog change. |
| Legacy paths preserved | `Play::new` / `run_with_io` / `prepare_client` legacy path retained; scatter process default retained for unbound callers. |
| Defaults table | local-274 `43594/80`, local-289 `44594/1080`, public-274 `443/443` + `w1.rs2b2t.com` only; vault `vault` / `vault-289` / `vault-prod`; unpack/nav paths match report. |
| No runtime acceptance from getters alone | Focused tests construct real clients, refuse writers, and exercise shared Arcs. Asset logs claim initialization only, not gameplay. Report correctly leaves step 3 unaccepted. |

## Independent checks run by this reviewer

1. Branch/HEAD identity as above.
2. Cold read of host diffs `932e480a^..0cc8b0b7` and full `profile.rs` / `session_profile.rs` / production binding in `host-play` / `host` / `interact.rs`.
3. Re-ran focused tests:

```text
CARGO_TARGET_DIR=.../274bot/target cargo test --locked -p host-play --test session_profile
# 9 passed; 0 failed; finished in 0.43s
```

4. Re-ran strict affected host Clippy:

```text
cargo clippy --locked -p api -p host -p host-play --features host-play/memory-profile --no-deps -- -D warnings
# Finished clean
```

5. Inspected evidence receipts under `docs/compat/evidence/session-profile/host/`:
   - `profile-tests-final.log`: 9/0 focused
   - `affected-tests.log`: initial affected gate
   - `affected-tests-client-c8e6155.log` + `.json`: 450 passed / 0 failed / 7 ignored on host `0cc8b0b7` with client product `c8e6155`
   - `clippy-host.log`: clean `--no-deps`
   - `local-274-assets.log` / `local-289-assets.log`: shared binding + init true; 289 nav unavailable + bot_operation_error set; gameplay_started false
   - `engine-lifetime-failure.txt`: retained failed 289 engine prep; engines not reviewer-owned

## Findings

No host-backend blocking defects found against the card contract.

Non-blocking scope notes (do not reopen this host card):

1. Host gitlink at HEAD remains `58120f2`; runtime verification used the working-tree client dependency `c8e6155` per handoff. Client same-card review owns that pin/update.
2. Dirty `crates/panel` / `crates/tui` WIP is frontend-owned and was not treated as host evidence.
3. Ambient/legacy `PlayOptions` callers remain ungated by design; production checked path is gated. Frontend must actually wire the checked parser/bind path (separate card).
4. Client ambient-site binding (transport/login RSA/OnDemand hub identity/adopt) is the client card’s contract; host correctly depends on `ClientSessionProfile` / `from_shared_with_profile` rather than re-implementing it.
5. No account login, gameplay, coherent final gate, native Mac frontend, Linux/Windows, or step-4 writer qualification is claimed or approved here.

## Approval metadata

```text
review_outcome: approved
review_round: 1
review_lens: artifact + independent focused execution
task_id: t_30007871
branch: codex/rs2b0t-multirevision
host_commit: 0cc8b0b7d77ecbb533f9b0be7669b8967f1dc256
host_implementation_commits: [932e480a, 0cc8b0b7]
client_dependency_handoff: c8e61557bf5b208eb4ffb589fd0a797fbf7d42ab
host_gitlink_at_head: 58120f28ee5208553ca07f41cb364f2cf98ea280
reviewer_profile: reviewer
reviewer_model: grok-4.5
reviewer_provider: xai-oauth
product_code_edited_by_reviewer: false
focused_tests: 9 passed / 0 failed (reviewer re-run)
clippy_host_no_deps: clean (reviewer re-run)
affected_tests_receipt: 450 passed / 0 failed / 7 ignored (client c8e6155 evidence)
report: docs/compat/01-session-profile.md
review_report: docs/compat/reviews/session-profile-host-grok45.md
```

Remaining root-owned gates: client review close + gitlink, frontend same-card
review, coherent integration recheck, native Mac frontend evidence, fresh Grok
4.6 milestone review.
