# Selected-revision game metadata source review

Reviewer: Hermes profile `reviewer`, model `grok-4.5`, provider `xai-oauth`.
Date: 2026-09-11 (UTC). Kind: corrective gate after implementer card
`t_f1572cf0` self-completed without the brief-mandated same-card
`kanban_request_review` handoff (board refuses review from done). Not LIVE,
not option-ledger acceptance, not whole-branch review.

Read once: `AGENTS.md`, `docs/execution.md`, brief
`docs/compat/briefs/43-item-metadata.md`, ownership note
`docs/compat/04-item-metadata.md`. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Work was read-only except this
report. No product source edits, stash/reset/checkout of concurrent WIP,
merge, remotes, gitlink, fixtures mutation, or release actions. Concurrent
startup progress and scenario WIP stayed untouched.

## Verdict

**Approve frozen source `7852b5a0`.** Selected-revision generated metadata
loading, immutable profile ownership, one-shot publication before module
evaluation, cache/profile isolation, food-heal and pickpocket joins, and
frozen Alcher composition match brief 43 and the ownership note. No blocking
defects found.

This approval is source/contract acceptance of the frozen implementation and
its unit/composition evidence. Ordered/custom LIVE cells and ledger updates
remain root-owned and are not claimed here.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Frozen implementation | `7852b5a053fd27e292221be78ea0783e805e3454` |
| Implementation parent / base | `76eb4cd040f487858fde17b6c390744c6ae68e06` |
| Campaign HEAD at review | `1a2f2dcf047de80e8793cb430c8044134163c7ac` |
| Branch | `codex/rs2b0t-multirevision` |
| Kanban card | `t_e6ec9679` (corrective; parent of integration `t_dde220ef`) |
| Prior implementer card | `t_f1572cf0` |
| Brief SHA-256 | `83c918053c0bd0621544d8ee352312018548f5d1d282e6f63eafb750fbcf62f8` |
| Ownership note SHA-256 | `563333a2ef9fd970fecaa37f5d205c28722fe3043c2132309f37f2672a1a89d0` |
| Generated 274.json SHA-256 | `e1a95f255022587528008c7b5690d13b0008511e05c22eb4ccffc71b1cb1c192` |
| Generated 289.json SHA-256 | `baef264dfd146fdc202069bc68e6a144dd0592af8362b1d3f53d27a47a01729c` |
| Exact commit export | `.superpowers/task-exports/t_f1572cf0-7852b5a0` |
| Review target dir | `target-review-t_e6ec9679` |

`git show --stat 7852b5a0` is 17 paths (668 insertions, 110 deletions):
api loader/tests + content removals, script load/shim/slot + tests,
host-play wiring/memory/session_profile, nav-input-audit, ownership doc,
Cargo.lock/`api` serde_json pin. Generated `crates/api/data/game-data/*`
were not modified by this commit (ancestors brief 44/45 assets). Concurrent
WIP paths (`scenario`, `catalog_boundary_live`, panel startup briefs) are
absent from the commit.

Exact export content hashes for primary product files at `7852b5a0`:

```text
bd369280bda19844fdf1f7df04b7740f33f07e93a0b9882629b5a472b879bc25  crates/api/src/game_data.rs
569f44e872cb037dd16fb39feef7c6e570cda964a36b08e9766987108801e67b  crates/api/src/content.rs
98c94be7c8dc14d34c62db5c36aa10fefdd03ee76f7dd3a44b37d97e317a3e62  crates/script/src/shim/mod.rs
e4700593ff7fd6f051504db9d5ec53004733495ff68b883b7a7546bc3e3893a6  crates/script/src/load.rs
3fb015e4d019605edd8ad3d67ab8a42f2c4320cda25811eced9e01182e7ad599  crates/script/src/slot.rs
bc1220da1056b0476cc42c1788a146e6422aee89c2a6664a284ee81a5a0bc59e  crates/host-play/src/lib.rs
f6fb94a840b0b356818e5641d0ab62263e9fd622db5ac14e3bea599101a1d75f  crates/host-play/src/memory.rs
ced539c5c0613e44af269f6711969abbcc0d225ef5568b634556c92ea29ea47b  crates/nav/src/bin/nav-input-audit.rs
d37571dd1b58c30c28d94f503c90b5a7490d68862c25c79fc8b18159aaead49d  crates/api/tests/game_data.rs
ca3bd914f507ccf70448dbc4108378f300f94f95ef26ad8abcd4109c9a2535b5  crates/script/tests/gold_stubs.rs
563333a2ef9fd970fecaa37f5d205c28722fe3043c2132309f37f2672a1a89d0  docs/compat/04-item-metadata.md
```

Export `cmp` matched git objects for `game_data.rs`, `shim/mod.rs`,
`host-play/src/lib.rs`, and `gold_stubs.rs`. No later branch commits retouch
those task product paths after `7852b5a0`.

## Contract map (brief 43 → source)

### Generated serde assets, once per revision

- `api::game_data` embeds schema-3 `274.json` / `289.json` via
  `include_bytes!`, decodes with `serde_json`, checks
  `schema_version == 3` and revision match, stores
  `OnceLock<Result<Arc<SelectedGameData>, String>>` per revision.
- `for_revision` returns a cloned `Arc`; `Arc::ptr_eq` holds across repeated
  loads of the same revision and fails across 274 vs 289 (api test).
- Runtime never opens a server checkout or generator path.

### Immutable profile ownership / cache isolation

- `SharedClientTemplate::load` calls
  `for_optional_profile(revision, profile.cache_id())`: matching audited
  cache identity yields `Some(Arc)`; foreign/synthetic cache yields `None`
  without failing template load.
- `Play` / `ScriptStartHandle` carry `Option<Arc<SelectedGameData>>` from the
  template; legacy offline `Play` assembly passes `None`.
- `session_profile::bound_public_client_refuses_fixture_cheats...` asserts
  synthetic public-289 cache gets `game_data().is_none()`.
- `for_profile` (nav audit) hard-fails on identity mismatch.
- No per-tick / snapshot deep-copy of the table: only Arc clones into start
  paths; JS owns one startup JSON blob.

### One-shot publication before module evaluation

- `wire_runtime` order: register helpers → eval shim PRELUDE →
  `globalThis.__rs2b0t_host.content = content_json(...)` →
  `load_modules` (bot + siblings). Content is therefore present when
  `itemdb.js` / food / thieving modules evaluate `ITEM_DB` / `FOOD_HEAL` /
  pickpocket rows.
- `content_json` builds `items` from generated alias+name+id+cost rows (no
  underscore alias invention), `food_heals` from unambiguous fixed HP heals,
  and joins `required_thieving` onto curated `PICKPOCKET_SPOTS` by display
  name while leaving stands/leashes host policy.
- Offline / `None` game_data publishes empty items and food tables; pickpocket
  rows omit `required_thieving` so JS `requiredThieving` stays not-impl.

### Handwritten tables removed

- `api::content::ITEMS`, `ItemRecord`, `FOOD_HEALS`, and spot
  `required_thieving` fields deleted. `git grep` on `7852b5a0` product Rust
  finds no residual `FOOD_HEALS` / `ItemRecord` / `pub const ITEMS`.
- Loadout food detection (`__rs2b0t_food_of`) and nav-input-audit food
  enumeration consume `fixed_food_heals()` from the selected Arc.

### Fail-closed food and pickpocket

- Fixed heal requires `qualification == "fixed_hp_heal"`, single hitpoints
  heal with `percent == 0` and `base > 0`. Same display name must agree on
  that heal across all consumption rows; conditional or conflicting names
  stay absent (Cabbage / Ugthanki kebab covered in api + gold_stubs).
- Bread=4, Anchovies=3, Lobster=12 verified against assets and tests
  (operator correction of old 5/1).
- Guard required thieving = 40 from generated pickpocket facts; curated
  Knight/Paladin/Hero levels 55/70/80 present in JSON and joined when spots
  match.

### Alcher composition (brief regression intent)

Frozen `AlcherLogic` composition test (ignored unless `RS2B0T` set) asserts
for both R274 and R289 data: 37 options / 36 items, ordered selection
`rune_platebody` then `rune_chainbody`, custom alias and display name
resolve to adamant scimitar id 1331, four distinct dragonhide_body ids,
unknown custom is null. Independently re-run against both frozen catalog
roots (see checks).

## Independent checks (exact export)

All commands used `CARGO_TARGET_DIR=.../target-review-t_e6ec9679` and cwd
`.superpowers/task-exports/t_f1572cf0-7852b5a0` so concurrent worktree dirty
files were not required.

| Check | Result |
|---|---|
| `cargo test -p api` | ok (lib 5 + game_data 2 + interact 65 + obj_names 7 + query 22 + settle 14 + snapshot 62) |
| `cargo test -p script --test gold_stubs` | 12 passed, 1 ignored |
| `selected_game_data_is_published_before_modules_and_offline_stays_empty` | ok |
| `RS2B0T=.../rs2b0t-100adccc... selected_game_data_composes_with_frozen_alcher_logic --ignored` | ok |
| `RS2B0T=.../rs2b0t-8e7d965b... selected_game_data_composes_with_frozen_alcher_logic --ignored` | ok |
| `cargo test -p script --test load_isolate` | 158 passed |
| `cargo test -p host-play --lib --features memory-profile` | 156 passed |
| `cargo test -p host-play --test session_profile --features memory-profile` | 11 passed |
| `cargo test -p nav --lib` | 254 passed |
| `cargo check -p host-play --features memory-profile --all-targets` | ok |
| `cargo clippy -p api -p script -p host-play -p nav --all-targets --all-features -- -D warnings` | ok |
| `rustfmt --edition 2021 --check` on task-owned Rust paths | ok |
| `git show --check 7852b5a0` | ok |
| Asset sample (jq): plate cost 65000 > chain 50000; 274 lacks castlewars_armour_body; 289 has it; 8 Dragonhide rows; Guard 40 | confirmed |

## Non-blocking notes

1. `for_profile` mismatch wording lists asset cache_id as “expected” and the
   caller cache_id as “got”. Fail-closed behavior is correct; only the label
   order is slightly counter-intuitive for profile-first callers.
2. Full selected item tables (~3.9k / ~4.1k rows) are stringified once per
   isolate start into JS. That matches the brief’s one-shot module ownership
   allowance; it is not a per-tick copy.
3. Rows missing alias or display name are omitted from published `items`
   (filter_map). Enabled Alcher / custom paths rely on generated aliases and
   names present in the assets; no invented underscore normalization.
4. Implementer process defect (self-complete instead of request-review) is
   why this corrective card exists; it does not alter the source verdict.
5. Root still owns ordered/custom LIVE acceptance and ledger updates after
   this gate.

## Limitations

- No LIVE harness launch.
- No whole-branch or integration review (child `t_dde220ef` / final
  branchreviewer remain separate).
- Broad `cargo test -p script` outside gold_stubs/load_isolate was not
  re-run; implementer noted pre-existing catalog_start STAFF_RUNES and
  concurrent js_declared_abi noise outside this commit’s scope.

## Outcome

Approve `7852b5a0` for brief 43 selected-revision game metadata. Completing
this corrective card releases the integration dependency edge that required
actual Grok 4.5 review after the missed handoff.
