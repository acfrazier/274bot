# Stage-1 whole-branch review

Reviewer: Hermes profile `branchreviewer`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11. Kanban card `t_fd3a8b47`.
Kind: required independent Grok 4.6 whole-branch review of the first
incremental multi-revision integration batch. Not a same-card task review,
not publication, not a release, not full catalog compatibility, not a
performance claim, and not the later-batch campaign finish line.

Read once: `AGENTS.md`, `docs/execution.md`, brief
`docs/compat/briefs/124-stage1-whole-branch-review.md`,
`08-incremental-integration.md` at the frozen candidate, historical
`01`/`02`/`02a`/`02b`/`03`/`03a` plus `client-integration-milestone.md`.
Those reports are evidence snapshots; their old next-actions were not used
as current directions. Branch checked first: `codex/rs2b0t-stage-1` (not
`main`). Work was read-only except this report and
`docs/compat/evidence/stage-1-review/`. No product/test edits, remotes,
`LIVE`, compile-cache mutation, merge, or push.

## Verdict

**APPROVE this first foundation batch** at host
`940531c3f7a4fe392f8ee5688702fe1f617d28b0` with client
`aef3952d1cd7bb3b93d39c497f0f476b68021c59`.

This is source-and-evidence approval of the reconciled 274/289 client,
immutable process-wide server/resource profile, revision-aware host
outbound/snapshot/reset boundary, selected-world navigation binding, and
panel/TUI production wiring. It is **not** publication. Root still owns
client fetchability on `FR-client-bothost` `r274-bh-modular`, a fresh
recursive checkout, host merge/push, and every remote. Existing incomplete
catalog behavior remains incomplete. Final campaign review is still
required for later batches.

Root `docs/compat/evidence/stage-1/validation-ready.json` was read after
the six remaining live cells finished. `ready_for_final_review` is true;
`approved_for_publication` remains false. Live cells were not inferred from
process exit or fixture seeding: the logs show host/script-caused walks,
NPC dialogue, loc replacement, nav arrival, lamp consume/hold/resume, and
logout clearing.

## Inspected refs

| Role | Exact value |
|---|---|
| Branch | `codex/rs2b0t-stage-1` (not `main`) |
| Host base | `b2bd5023489ab2e0b6ba690f228217e8984ac91b` |
| Host code candidate | `940531c3f7a4fe392f8ee5688702fe1f617d28b0` |
| Docs/evidence after candidate | `36e2fc44d509c9c18ea7fcf3b71631871d65d104` |
| Staging commit (fmt/clippy/workspace) | `659380651869bd12fc9d281a30037b3b0368b380` |
| Client published base | `9b41e6e06b9fd42dc2247fc813a303c3fcb92941` |
| Client candidate / gitlink | `aef3952d1cd7bb3b93d39c497f0f476b68021c59` |
| Host range | 52 ordinary-history commits |
| Client range | 73 commits |
| Reviewer model/provider | `grok-4.6` / `xai-oauth` |
| validation-ready SHA-256 | `a0587b2f35f7ec63d323344646e46346c29fa4e797389ea7e3d635ef54beb49e` |

Confirmed: `git branch --show-current` = `codex/rs2b0t-stage-1`.
`git ls-tree 940531c3f vendor/fr-client-rust` = client `aef3952d`.
Checkout HEAD after the candidate is `36e2fc44d` (docs/evidence only:
`08`, `STATE`, brief 124, and `evidence/stage-1`). Product, tests, and
client gitlink are unchanged versus `940531c3f`. The code pin was not
evaluated as a moving HEAD.

`659380651..940531c3f` changes only
`crates/host-play/tests/world_boundary_live.rs` (Catherby mainland skip,
`give macro_genilamp`, scene-2 door baseline). Product and client are
unchanged after `659380651`.

## Scope

In: 274/289 client merge and preservation corrections; immutable
`ServerProfile` / `ClientSessionProfile`; shared cache/iface/nav template;
named outbound mapping; snapshot/reconnect publication; isolate
generation discard; selected cache/nav manifests; frontend bind-once;
controlled 274/289 boundary and world proofs.

Out: later campaign banking/cake JS orchestration; new foreign JS runtime;
catalog completeness; Alpha 2 release; Linux/Windows qualification;
performance.

Product crate delta versus base has no cake-restock or named bank-opening
JS policy. Pre-existing food names, thieving cake-stall shims, and host
bank snapshot fields remain. Design docs `04-banking-design.md` and
neighbors are documentation only in this batch.

## Source findings

Immutable identity. `ProfileSelection::bind` freezes cache CRCs, RSA,
nav/cache manifests, and an `Arc<ClientSessionProfile>` with no mutable
connection fields. Unknown caches refuse without `--cache-manifest`.
289 nav requires a matching sidecar; 274 may use a legacy pack. Panel
bind is once-per-process; TUI binds before vault mutation.
`SharedClientTemplate` loads cache/ifaces/world once and clones Arcs.
`world.clone()` in this path is Arc sharing, not a per-read world copy.

Protocol/session. `Driver::revision` comes from the bound client.
`legal_sends_for` maps named `ClientProt` rows through `map_client_prot`.
Public cheat refusal remains before writes. `Pump::drain_client` rewinds
to `session_start_gens` on reconnect so pre-grant packets cannot publish.
`publish_snapshot` resets on logout and on `session_changed`. Script
ticks/interacts carry a work generation; reset discards late batches and
clears only host interaction state. `659380651` lifts the 289
`host-boundary-not-qualified` spawn gate after controlled action/world
proof; exhaustive revision matching is retained.

Selected resources. `CacheManifest` / `NavManifest` bind revision, archive
hashes, and pack identity. `nav-pack` verifies declared revision and
config archive before writing. Script loading is revision-agnostic;
catalog identity still binds Browse/warmup.

Rendering/memory. GPU last-FBO freeze while `scene_state==1` remains in
`gpu.rs` (`freeze_last_scene`). Client `static_loc_generation` and
per-component inventory packet facts are exposed; host loc rebuild still
gates on scene gen plus aggregated `model_stamp`, and inventory families
still gate on iface/inv gens. Live door replacement and lamp consume were
observed anyway. This is residual host consumption, not a failed cell.

Client later commits after the corrected integration product include
session publication watermarks, inventory-component freshness, static
scenery observation, and a mechanical `{revision}bot:` diagnostic label.
Opt-in 289 ground-trace hooks in present/renderer are diagnostic and
inactive unless armed.

## Evidence independently checked

Workspace formatting and strict Clippy passed on `659380651`. Remaining
workspace tests passed after skipping the two external-catalog failures
that reproduce on exact main `b2bd5023` / client `9b41e6e0` against
operator catalog `42011fb1` (STAFF_RUNES exports; stale declared ABI).
Separate client tests passed. Failures and baseline comparisons are
retained.

Live (actual progress, not exit-only):

| Rev | Case | Source | Observed |
|---|---|---|---|
| 274 | boundary | `659380651` | courtyard walks, Hans 4882, door 1530→1531, logout cleared |
| 274 | nav_full | `659380651` | arrived `(3220,3264,0)`, ticks 66, scene 2 |
| 274 | nav_door | `940531c3f` | 1530→1531, arrived `(2817,3443,0)`, ticks 15 |
| 274 | guardian_lamp | `940531c3f` | inv 2528, hold, consume, XP, walk to `(3221,3212,0)` |
| 289 | boundary | harness unchanged after `659380651` | walks, Hans 4882, door 1530→1531, logout cleared |
| 289 | nav_full | `940531c3f` | arrived `(3220,3264,0)`, ticks 62, scene 2 |
| 289 | nav_door | `940531c3f` | 1530→1531, arrived `(2817,3443,0)`, ticks 15 |
| 289 | guardian_lamp | `940531c3f` | inv 2528, hold, consume, XP, walk to `(3221,3212,0)` |

The initial old-fixture 274 `nav_door` failure on `659380651` is retained
(`ticks: 0`, deadline, tile `(2818,3436,0)`). 289 cells log
`snapshot load skipped` for missing unpack `models.bin`; scene 2 and the
predicates still held. Not a GPU/performance claim.

Log SHA-256 values in `validation-ready.json` match the receipts already
read. Elapsed times are not performance measurements.

## Residuals (non-blocking)

- Host snapshot does not yet read `World::static_loc_generation` or
  `Client::inventory_packet_states`.
- 289 unpack model snapshot is absent in this environment.
- Catalog completeness and declared-ABI drift against the newer operator
  catalog remain explicit exclusions.
- Linux/Windows, public 289 gameplay, and later banking/catalog shims are
  outside this batch.

No material correctness, 274-preservation, identity, protocol, ownership,
or accidental later JS-policy defect was found in the frozen range.
