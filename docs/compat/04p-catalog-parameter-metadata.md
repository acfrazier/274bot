# Catalog parameter metadata: computed choices and showIf

Reviewer: Hermes profile `grok46`, model `grok-4.6`, provider `xai-oauth`.
Date: 2026-09-11 08:43 UTC. Kind: bounded read-only architecture/fidelity
design for brief 88. Not implementation, LIVE, fixtures, ledger, STATE,
or an enabled-status change. Root owns acceptance and any follow-up hop.

Read once: `AGENTS.md`, `docs/execution.md`, brief 88, e72 TUI screens,
frozen Alcher sources, `register_rs2b0t` / `settings_blob` / `SettingDef`,
shared `setting_visible`, panel and TUI consumers. Branch checked first:
`codex/rs2b0t-multirevision` (not `main`). Work was read-only except this
report and `docs/compat/evidence/catalog-parameter-metadata/`. Concurrent
working-tree edits on this checkout are not the tested binary. Shared
host-play/runtime files belong to hostile observation `t_eeea20f0` then
count-dialog, special/teleport/shop/Make-X/fire. They were not edited.

## Verdict

**Reviewed e72 TUI Params is not a broken character editor.** Numeric and
string[] typing, invalid-number, cancel and persist work. Alcher opens as
free-text `Items to alch` with no chips, default `—`, and **Custom item
stays hidden after saving `custom`**, because the shared static scanner
drops identifier/computed catalog metadata before panel/TUI paint.

This is a host mapping gap in `rs2b0t_registry` + `setting_visible`, not
a foreign Alcher defect and not an e72 input bug. Both frozen catalogs
are byte-identical on `Alcher.ts` / `AlcherLogic.ts`. Native panel uses
the same `JsCard.settings_schema` and the same empty-options free-text
fallback, so it shares the omission.

Do not spawn V8 at Browse. Isolate evaluation posts `ITEM_DB` only at
Start (`load.rs` writes `__rs2b0t_host.content` then loads catalog
modules). Evaluating `AlcherLogic` at Params would see an empty item
list or would move schema past Start. Do not copy the foreign settings
runtime, invent option lists, change Alcher defaults, or add a general
JS evaluator.

Smallest host-owned correction: keep the existing static scanner, and
inline the three frozen Alcher shapes it already almost handles
(sibling string-array default, string-const inside `showIf.anyOf`,
FODDER `{ obj: '…' }` keys plus `custom`). Teach `setting_visible` the
already-supported chip-list membership rule. TUI/panel already have
choice UIs when `options` is non-empty; they do not need a new editor.

## Inspected refs

| Role | Exact commit / object |
|---|---|
| Campaign HEAD at write-up | `d49dccbdb533622101e67736a4ada5b7e2354078` |
| Client gitlink | `9d090ed04957e4efc254f073cda97bc5510ca72b` |
| Branch | `codex/rs2b0t-multirevision` |
| Brief 88 SHA-256 | `b9215778dd552fd9fa7a41f8c2d2542c5f538b3776452f01aff3c79eeee564bc` |
| Kanban card | `t_10ea7213` |
| TUI params commit | `e72cfb39ef086a89313961be8ca54d1d555e01f4` |
| Old catalog | `100adccc037d9f6898080e1cad58fcfc43364775` |
| Newer catalog | `8e7d965be2071d6ec65c3265e12af797082d720a` |
| Alcher.ts (both; `cmp` 0) | sha256 `1a09baa1117544a2bb80da145829e69c9ced029ad5af533616afa8e7a36f2032` |
| AlcherLogic.ts (both; `cmp` 0) | sha256 `90431043aa900332b3f269cc26c0a1c92bc6a743c5da9adf4fa85102871d4367` |
| `rs2b0t_registry.rs` | sha256 `8c58f819a7f955e48ac9b604edb14ec6349d2eccad1cecb0835f51bddba045d1` |
| `settings_store.rs` | sha256 `d1589ae5dea5c62d6b8cb92519d14c7e16736aefd258aa32a52a478c7b6c1b45` |
| Concord r289 choices.txt | sha256 `55826fb34d6177b5cad1282ef9613f17388b907e3b3ae387cf1f37c7975e6c8a` |
| Concord r289 custom-saved.txt | sha256 `e035bf5ad7ee06c68225e87d2a476ab6b98667a72cf5ff6ee8c44532ed2abc2a` |
| Mac r274 editor-open.txt | sha256 `6291d64a1d41f20d5436d0a22d4fc1bc4d40867909acc5ba56fd7346836a356e` |

Machine-readable copies: `evidence/catalog-parameter-metadata/{refs,observed-alcher-schema,fodder-objs,showif-eval}.json`.

## 1. Observed vs required

Concord289 e72 (`tui-play --live script_alcher`, sel Alcher, scene 2):

- form.txt: `Items to alch: —` and `Alchs per trip: 27.0`, hint
  `enter edit · space toggle`. Two rows. No Custom item.
- choices.txt: free-text caret on Items to alch (`_`), not a chip list.
- custom-saved.txt: `Items to alch: custom` saved; still two rows; Custom
  item absent.
- invalid-number / cancelled / persisted: number and string[] edit work.

Mac274 editor-open.txt is the same empty item list plus `27.0`.

Required catalog shape (frozen `ALCHER_SETTINGS`, both trees):

```
items:      type string[], default DEFAULT_ALCH_ITEMS, options ALCH_OPTIONS,
            optionLabels ALCH_OPTION_LABELS
customItem: type string, default '', showIf { key: 'items', anyOf: [CUSTOM_ALCH_KEY] }
alchs:      type number, default 27, min 1, max 1000
```

`CUSTOM_ALCH_KEY = 'custom'`. `DEFAULT_ALCH_ITEMS` is an 11-entry string
array literal in sibling `AlcherLogic.ts`. `ALCH_OPTIONS` is computed:
`[CUSTOM_ALCH_KEY, ...ALCH_ITEMS.map(i => i.key)]`. `ALCH_ITEMS` is
`FODDER.flatMap` against `ITEM_DB` then richest-first. `FODDER` is 36
`{ obj: 'maple_longbow' | … }` literals.

Actual loader dump (`catalog-inputs` via `register_rs2b0t`, both
catalogs, Alcher `source_sha256` matches Alcher.ts):

| field | observed | required |
|---|---|---|
| items.type | `string[]` | `string[]` (kept) |
| items.default | `null` | the 11 DEFAULT_ALCH_ITEMS keys |
| items.options | `[]` | `custom` plus FODDER obj keys |
| items.option_labels | `[]` | Record key→label (deferred) |
| customItem.show_if | `{ key: 'items', anyOf: [CUSTOM_ALCH_KEY] }` | `{ key: 'items', anyOf: ['custom'] }` |
| alchs | number 27, min 1, max 1000 | same (kept) |

Type/label/min/max/help already survive. The loss is identifier default,
computed options, Record labels, and the const inside `anyOf`.

## 2. Loss through the shared loader

Catalog Browse does **not** call `settings_schema_from_source` (that is
File Load, `export const SETTINGS` only). `JsLibrary::register_rs2b0t`
reads sibling `./AlcherLogic.ts`, concatenates it in `settings_blob`,
and parses `ALCHER_SETTINGS` from the named index import
(`settingsSchema: ALCHER_SETTINGS`). Fletcher already proves sibling
**literal** option arrays inline (`parse_registry_inlines_same_dir_logic_option_arrays`).

Alcher fails on three scanner limits, not on missing siblings:

1. **`default: DEFAULT_ALCH_ITEMS`.** `scan_key_literal` accepts quoted /
   bool / number only. The sibling array literal is never consulted, so
   `merge_bag` has no items default and the form shows `—`.
2. **`options: ALCH_OPTIONS`.** `resolve_string_array_ident` calls
   `parse_string_array` on `[CUSTOM_ALCH_KEY, ...ALCH_ITEMS.map(i => i.key)]`.
   The first element is unquoted, the walker breaks, and the function
   still returns `Some([])`. Empty success hides the computed RHS.
   Fletcher-style literals never hit this.
3. **`showIf: { key: 'items', anyOf: [CUSTOM_ALCH_KEY] }`.**
   `scan_key_show_if` inlines a whole-object ident (`SHOW_MELEE`) but
   leaves idents inside a literal object. `extract_any_of` keeps only
   quoted strings, so `anyOf` becomes `[]` and `any()` is false.

Same-file / shim literals (`FOOD_OPTIONS`, `Object.keys(SPELL_DB)` stub,
`...PERIODIC_BANK_SETTINGS`) are not this bug.

`ITEM_DB` is `host().content.items` and is empty until isolate spawn.
`content_json` runs in `LoadIsolate` after Start. Existing isolated
module evaluation cannot supply schema at Browse without violating
load/start lifecycle and the untrusted-script boundary.

## 3. Why Custom item stays hidden after save

Two independent host defects; fixing only the const is not enough.

TUI `value_from_scratch` for `string[]` stores `coerce_list("custom")` =
`["custom"]`. `custom-saved.txt` displays `custom` via array join.

`setting_visible` then:

- treats `{ … }` as evaluable (not fail-open);
- parses `anyOf: [CUSTOM_ALCH_KEY]` as empty;
- converts array bag values to `""` (`value_as_setting_str` default arm).

So the row is hidden even if the operator types the custom chip key.

Foreign `paramControls.isVisible` (do not copy the file) splits the
master value on comma and tests membership, with an explicit test:
`isVisible: a showIf on a chip list matches any ticked chip, not the
joined string`. Mapping that condition onto host `setting_visible` is
the supported `showIf` semantics for `string[]`, not a new runtime.

AutoFighter `anyOf: [CUSTOM_COORDINATES]` is the same inner-const shape
and should ride the scanner hop; it is not a second design.

## 4. Native panel

`panel/src/app.rs` `script_parameter_editors` uses
`setting_visible` and `resolve_setting_options`. Empty `string[]`
options fall through to free-text `input_text`, same as TUI. No unique
Mac native Alcher Params screenshot is in this evidence set; the shared
schema dump is enough to say the panel shares this specific omission.

TUI `alcher_schema()` in `script_params.rs` tests the empty-options
editor on purpose. Keep that fixture. Do not treat it as loader output.

## 5. Smallest host-owned correction

Owner: `crates/script` scanner + visibility. TUI/panel consume filled
`SettingDef`; choice/chip UI already exists.

**In `rs2b0t_registry.rs` (catalog `settings_blob`, no V8):**

- `default:` ident → `resolve_string_array_ident` (sibling literal
  arrays) or quoted string const; store JSON array text so
  `default_for_type("string[]")` parses it.
- `showIf` object: rewrite unquoted `anyOf` idents to quoted sibling
  string consts (`CUSTOM_ALCH_KEY` → `'custom'`). Leave unknown idents
  unresolved; do not drop the condition.
- `options:` ident whose array literal is mixed/computed: do not accept
  `parse_string_array` empty-success. For the frozen Alcher RHS
  `[CONST, ...IDENT.map(i => i.key)]`, take the resolved const plus
  quoted `obj:` values from the mapped source's `FODDER` (or equivalent)
  array literal. That is the catalog's declared key list, not an invented
  table.
- Optional same-hop, only if `SelectedGameData` is already in hand at
  register: filter missing aliases and sort by `floor(cost * 0.6)`.
  Do not spawn an isolate or rebuild schema from the live snapshot to
  get that order. FODDER source order plus `custom` is enough to expose
  choices before Start. Richest-first chip order is native item metadata
  and must not block the hop.

**In `settings_store.rs`:**

- `setting_visible`: for array (or comma-joined string) master values,
  membership of any `anyOf` token, case-insensitive. Scalar equality
  stays for string/bool/number. Empty parsed `anyOf` after rewrite stays
  hidden only when the condition is well-formed and matches nothing —
  do not fail-open a real empty chip list.

**Do not:**

- edit TUI/panel except if a later hop owns `optionLabels` Record
  display;
- File-Load `ALCHER_SETTINGS` under the `SETTINGS` name (not the
  Concord/Mac TUI path);
- evaluate `AlcherLogic` / `ITEM_DB` at Browse;
- clone foreign `paramControls.ts` or `Settings.ts`;
- change Alcher script defaults;
- generalize to `SHOP_PRESETS.map(p => p.label)` or other computed maps
  in this hop;
- implement gp chip labels (`optionLabels` is a Record; host
  `option_labels` is `Vec<String>` and unused in TUI/panel). That is an
  intentionally deferred optional native feature.

## 6. Bounded file plan

| File | Change |
|---|---|
| `crates/script/src/rs2b0t_registry.rs` | default ident; anyOf const rewrite; refuse empty mixed-array parse; FODDER `obj` extract for Alcher-shaped `options` |
| `crates/script/src/settings_store.rs` | chip-list membership in `setting_visible` |
| `crates/script/tests/rs2b0t_registry.rs` | Alcher-shaped sibling fixture (no live ITEM_DB) |
| `crates/script/tests/settings_bag.rs` | visibility cases for array / csv / custom chip |

No `crates/tui`, `crates/panel`, foreign catalog, isolate, host-play,
ledger, or STATE in this hop. Root owns LIVE proof on a reviewed build.

## 7. Meaningful tests

1. `parse_registry_with_sources` on Alcher-shaped sources: `items.options`
   contains `custom` and `rune_chainbody`; `items.default` is the 11
   DEFAULT_ALCH_ITEMS keys; `customItem.show_if` contains `'custom'` not
   `CUSTOM_ALCH_KEY`.
2. Mixed computed array does not become `Some([])`.
3. `setting_visible`: `["custom"]` and `"yew_longbow, custom"` show
   customItem; `["yew_longbow"]` hides it; `{ key: 'style', anyOf: ['mage'] }`
   still works as today.
4. Existing Fletcher sibling-literal and `SHOW_MELEE` tests stay green.
5. TUI empty-options fixture stays a free-text editor test, not Alcher
   loader output.

Do not add a test that snapshots scanner source text.

## 8. One UI falsifier

On a build that includes the scanner hop, Concord (or Mac) TUI Params
for catalog Alcher before Start must open a **choice list** for Items to
alch (not `Items to alch: _` free text) that includes `custom`, and
ticking/saving `custom` must reveal the **Custom item** row. Today's
e72 Concord `choices.txt` / `custom-saved.txt` and Mac `editor-open.txt`
are the failing captures. Character input of numbers remains out of
scope for that proof.

## 9. Ownership

Implementation: `crates/script` only, after root assigns it. Current
shared script files remain hostile `t_eeea20f0` then count-dialog /
special / teleport / shop / Make-X / fire — serialize or isolate the
commit. LIVE and platform TUI rebuilds stay with root. This card is
design only.
