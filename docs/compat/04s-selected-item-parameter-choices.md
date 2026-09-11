# Selected-item catalog choices before Start

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 09:00 UTC. Kind: bounded read-only architecture/fidelity
design for brief 91. Follow-up to `04p-catalog-parameter-metadata.md`.
Not implementation, LIVE, fixtures, ledger, STATE, File Load, or an
evaluator. Root owns routing and any follow-up hop. `t_7bb8a58a` owns
literal/default/anyOf scanner work in `rs2b0t_registry.rs` /
`settings_store.rs`; those files were not edited here.

Read once: `AGENTS.md`, `docs/execution.md`, brief 91, 04p, 04-item-metadata,
frozen Alcher sources, `register_rs2b0t`, `SettingDef`,
`resolve_setting_options`, `SelectedGameData`, TUI/panel Params call
sites. Branch checked first: `codex/rs2b0t-multirevision` (not `main`).
Work was read-only except this report and
`docs/compat/evidence/selected-item-parameter-choices/`. Concurrent
working-tree edits on `rs2b0t_registry.rs` and `settings_store.rs` are
brief-90 WIP, not the tested binary.

## Verdict

**04p is right that Alcher Params is a shared scanner/visibility loss,
not an editor bug. Root is also right that stuffing unfiltered FODDER
in source order into `SettingDef.options` is not faithful
`ALCH_OPTIONS`.** Frozen AlcherLogic keeps a declared 36-row FODDER
table, then drops missing `ITEM_DB` aliases and sorts by
`Math.floor(cost * 0.6)` descending, then `label.localeCompare`. Custom
is prepended. Eight dragonhide rows carry explicit labels because the
client name is shared. The gp `toLocaleString` suffix is cosmetic.

`ITEM_DB` is empty until isolate Start. Native `SelectedGameData` is
not: TUI binds the profile/`SharedClientTemplate` before Browse, and
panel already exposes `Session::selected_game_data()` at Params. Both
frontends already resolve combo contents at paint through
`resolve_setting_options`, which today handles inline options and
`optionsFrom: 'loadouts'` only.

Smallest host-owned seam: a typed candidate descriptor on `SettingDef`
(catalog-declared prefix + FODDER `{obj, label?}`), resolved at Params
with the bound `Arc<SelectedGameData>`. Do not overload
`JsLibrary::register_rs2b0t` with game data. Register is once per
session (`rs2b0t_filled`); panel profile rebind does not re-parse the
catalog. Baking filtered options into `JsCard.settings_schema` would
either freeze one revision or mutate process catalog schema.

No selected facts (offline, cache mismatch, unbound profile) must
publish only the non-item prefix `custom`, never FODDER aliases.
Defaults stay the frozen 11 keys even if a key is later absent.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `78cc99d07046a781fe8cd5ebf66bb63e663d23ea` |
| Client gitlink | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 91 SHA-256 | `8365895215fac6a94291f9c7476c4565b8d5ccbf9cf2e1ff7e425d0cad810a7a` |
| Kanban card | `t_89a57fc5` |
| Parent 04p | sha256 `3acb0197e7e7851516b16221fb2c4db3aca94444156e540b9a2f573d3b27dfb3` |
| `rs2b0t_registry.rs` HEAD | sha256 `8c58f819a7f955e48ac9b604edb14ec6349d2eccad1cecb0835f51bddba045d1` |
| `settings_store.rs` HEAD | sha256 `d1589ae5dea5c62d6b8cb92519d14c7e16736aefd258aa32a52a478c7b6c1b45` |
| `loadouts_store.rs` | sha256 `14bff326ed54d7d016cda3fd372508c7ad6bb06df4495fbbd29d805b20575bf0` |
| `game_data.rs` | sha256 `308ac8a02d342201d1f915a1323bfb541a3781b8bad62d1e9c99da47fb303c27` |
| Generated 274.json | sha256 `d310498c02aff231d4bd3a45095c9bb5d89007f899ada70f7b02863410b322c1` |
| Generated 289.json | sha256 `9e7336807bd8756d86dc52568eb009520696b1b3ba4ac7066b9c997c6ad8f380` |
| 274 cache_id | `4aac9b63312dcb75d5de8f686772d083ba0808c57985438246edf21ef522be1c` (3894 items) |
| 289 cache_id | `c4d8ab36bcfd2a7907535b4f619e28623b0a22e98d496fd2a9620d544c5b5b09` (4089 items) |
| Alcher.ts / AlcherLogic.ts (both catalogs) | sha256 `1a09baa1…` / `90431043…` (`cmp` 0) |

Machine-readable copies:
`evidence/selected-item-parameter-choices/{refs,frozen-expressions,lifecycle,resolved-options}.json`.

## 1. Exact native choices

Frozen expressions (source text, not executed):

```
CUSTOM_ALCH_KEY = 'custom'
ALCH_ITEMS = FODDER.flatMap(obj in ITEM_DB) sorted richestFirst
ALCH_OPTIONS = [CUSTOM_ALCH_KEY, ...ALCH_ITEMS.map(i => i.key)]
label = FODDER.label ?? rec.name
alchValue = Math.floor(rec.cost * 0.6)
richestFirst = b.alchValue - a.alchValue || a.label.localeCompare(b.label)
```

Joined with current generated facts via `jq` (`item_by_alias` + required
name, matching `content_json`): all 36 FODDER aliases exist on both
revisions with identical id/name/cost. Faithful `ALCH_OPTIONS` on both:

`custom`, `rune_platebody`, `rune_2h_sword`, `rune_platelegs`,
`rune_kiteshield`, `rune_chainbody`, `rune_sq_shield`, `rune_full_helm`,
`rune_scimitar`, `air_battlestaff`, `earth_battlestaff`,
`fire_battlestaff`, `water_battlestaff`, `black_dragonhide_body`,
`adamant_platebody`, `red_dragonhide_body`, `blue_dragonhide_body`,
`dragonhide_body`, `battlestaff`, `adamant_2h_sword`,
`adamant_platelegs`, `black_dragonhide_chaps`, `adamant_kiteshield`,
`mithril_platebody`, `red_dragonhide_chaps`, `blue_dragonhide_chaps`,
`dragonhide_chaps`, `black_platebody`, `mithril_2h_sword`,
`mithril_platelegs`, `magic_longbow`, `mithril_kiteshield`,
`steel_platebody`, `yew_longbow`, `steel_2h_sword`, `steel_platelegs`,
`maple_longbow`.

Unfiltered FODDER source order starts at `maple_longbow`. That is the
rejected 04p shortcut. Tie examples that source order would get wrong:
rune 2h before rune platelegs (both 38400); elemental battlestaves
Air/Earth/Fire/Water (all 9300).

Identity labels (capability): the eight dragonhide rows must not paint
as four identical `Dragonhide body` / `Dragonhide chaps`. Explicit
FODDER labels (`Green d'hide body`, …) distinguish them. Keys remain
the persisted values. Cosmetic gp suffixes (`Rune platebody (39,000)`)
are not required for this hop.

`castlewars_armour_body` is 289-only and not in FODDER. It must never
appear as a chip. Extra selected items are not candidates.

`DEFAULT_ALCH_ITEMS` stays the frozen 11-key array. Do not drop a
default because the selected table later lacks it. Runtime
`selectedAlchItems` already intersects with `ALCH_ITEMS`. Describe
absent-default as “key remains in default, not in resolved options /
not selected at Start”. Do not rewrite script defaults.

## 2. Lifecycle

```
profile bind  ->  SharedClientTemplate::load
                  for_optional_profile(revision, cache_id)
                  Option<Arc<SelectedGameData>>
Browse/register -> JsLibrary::register_rs2b0t  (once, no game_data, no V8)
Params          -> resolve_setting_options(def, loadouts)   // today
Start           -> content_json + load_modules  (ITEM_DB first non-empty)
```

TUI `run()` loads the template before `TuiSession::new_bound`. Params
can run with `template.game_data()` and without Play/isolate.
Panel `selected_game_data()` already exists and is unused by
`script_parameter_editors`. Catalog fill is `rs2b0t_filled` once;
`install_prepared_template` does not re-register.

`resolve_setting_options` is the existing deferred-options seam
(loadouts). Extending it is smaller than a register overload.

## 3. Choice: typed descriptor, not register overload

Add on `SettingDef` (default empty / `None`):

```
struct ItemOptionCandidate { key: String, label: Option<String> }
struct ItemOptionSpec { prefix: Vec<String>, candidates: Vec<ItemOptionCandidate> }
item_option_spec: Option<ItemOptionSpec>
```

Scanner (catalog `settings_blob` only, after brief 90 leaves mixed
`ALCH_OPTIONS` unresolved rather than `Some([])`):

- `options: ALCH_OPTIONS` whose sibling RHS is
  `[CONST, ...IDENT.map(i => i.key)]`
- prefix = resolved string const (`CUSTOM_ALCH_KEY` → `custom`)
- candidates = quoted `obj` / optional `label` from the mapped
  source's `FODDER` array literal
- do **not** write those keys into `options`
- do not evaluate `flatMap`, `ITEM_DB`, `sort`, or `toLocaleString`
- do not copy FODDER into a Rust static
- unknown computed maps (`SHOP_PRESETS.map`) stay unresolved

Resolver (`loadouts_store.rs`), both frontends already call it:

1. inline `options` non-empty → unchanged (Fletcher / literals)
2. `options_from == "loadouts"` → unchanged
3. `item_option_spec` present:
   - no `SelectedGameData` → return `prefix` only (`["custom"]`)
   - with facts → keep prefix, then candidates whose
     `item_by_alias(key)` has a name, `alchValue = floor(cost * 0.6)`,
     `label = candidate.label.unwrap_or(name)`, sort desc alch then
     label, persist keys
4. else empty

Return keys for persist. Return identity labels in a parallel vec or
small `ResolvedSettingOptions { values, labels }` so UI can paint
label and store key. Do not mutate `SettingDef` at resolve time.

Thread `Option<&SelectedGameData>`:

- TUI: `session.template.as_ref().and_then(|t| t.game_data())` into
  `ParamsPane` (same places that already pass `loadouts`)
- panel: `session.selected_game_data()` into the two
  `resolve_setting_options` call sites in `script_parameter_editors`
- paint `labels[i]` when present, else `values[i]`

Do not spawn V8, do not rebuild schema from the live snapshot, do not
deep-copy item tables per frame (borrow the template Arc).

## 4. Bounded file plan

| File | Change |
|---|---|
| `crates/script/src/rs2b0t_registry.rs` | **After** `t_7bb8a58a`. `ItemOptionSpec` on `SettingDef`; Alcher-shaped scan of prefix + FODDER `{obj,label?}`. `options` stays empty for this RHS. |
| `crates/script/src/loadouts_store.rs` | Resolver + selected facts; identity labels; loadout/literal paths unchanged |
| `crates/tui/src/{script_params,app,bin}.rs` | Pass template game_data; paint identity labels |
| `crates/panel/src/app.rs` | Pass `selected_game_data()`; paint identity labels |
| `crates/script/tests/rs2b0t_registry.rs` | Alcher-shaped sibling fixture (no live ITEM_DB): spec extracted, `options` empty |
| `crates/script/tests/loadouts_bag.rs` | Resolve 274/289, None, absent alias, ties, labels, loadout regression |

No `load.rs` register signature, no isolate, no `settings_store.rs`,
no foreign catalog, no File Load `ALCHER_SETTINGS`, no gp formatter,
no general expression interpreter. Root owns LIVE proof.

Hotspot: `crates/script/src/rs2b0t_registry.rs` — brief 90 already
owns this file. Serialize the scanner attachment after that commit.

## 5. Meaningful tests

1. Same frozen Alcher-shaped sources under `for_revision(R274)` and
   `R289`: resolved values equal the 37 keys above; dragonhide body
   ids `{1135,2499,2501,2503}`.
2. `game_data = None`: `["custom"]` only. No `maple_longbow`.
3. Fixture candidate `not_a_real_selected_item` dropped; remaining
   still sorted.
4. Tie order: `rune_2h_sword` before `rune_platelegs`; battlestaves
   Air/Earth/Fire/Water.
5. Identity labels: `dragonhide_body` → `Green d'hide body`, not
   `Dragonhide body`. Shared client names stay distinct.
6. Same `SettingDef` rebound from None → 274 → 289 without mutating
   `options`. 289-only `castlewars_armour_body` never appears.
7. Fletcher literal `options` and `optionsFrom: 'loadouts'` unchanged.
8. `DEFAULT_ALCH_ITEMS` 11 keys still the schema default (brief 90);
   this hop must not rewrite them.

Do not snapshot scanner source. Do not re-evaluate AlcherLogic in V8
for this hop (`gold_stubs` composition test remains the JS witness).

## 6. One UI falsifier

On a build that includes this hop **and** brief-90 visibility, Concord
or Mac TUI Params for catalog Alcher before Start must open a **choice
list** whose first item chip is `custom` and whose first fodder chip is
`rune_platebody` (not `maple_longbow`, not free-text `_`). Ticking
`custom` reveals **Custom item**. Then a fresh Start. Today's e72
`choices.txt` / `custom-saved.txt` remain the failing captures. gp chip
text is out of scope for that proof.

## 7. Ownership

Implementation after root assigns it, gated on `t_7bb8a58a` for the
registry file. `loadouts_store.rs` + TUI/panel threading can proceed
once `SettingDef` carries the spec, or behind that same card if root
wants one hop. LIVE and platform TUI rebuilds stay with root. This
card is design only; no review handoff and no implementation here.
