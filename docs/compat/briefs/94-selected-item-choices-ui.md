# Resolve faithful selected-item choices in both native frontends

Use grok46 defaults after t_7bb8a58a completes SAME-card review. Own the bounded
04s-selected-item-parameter-choices.md implementation: typed descriptor on
SettingDef and catalog scan in rs2b0t_registry.rs; native resolver in
loadouts_store.rs (and script/lib.rs export if needed); TUI script_params/app/
bin argument composition; panel/app parameter editors; focused registry/
loadout/TUI/panel tests; docs/compat/04t-selected-item-choices.md and unique
 evidence/selected-item-choices-ui/. No load.rs/slot.rs/isolate/host-play/nav/
game-data/runtime/foreign changes or LIVE. Named-bank t_bced5c76 owns its
runtime paths; preserve all concurrent source. Do not edit unrelated test
fixtures except required SettingDef/ParamsPane constructor composition.

Follow04s's actual call-site finding: profile SelectedGameData is available
before Start in both frontends. Keep JsLibrary registration profile-neutral;
use the existing deferred-options resolver with borrowed selected facts.
Never execute arbitrary catalog code during Browse or copy the item table
per frame. Catalog inputs supply candidate keys/explicit labels, not authority
for game IDs/costs. Resolve selected aliases from native generated data;
filter absent aliases, sort by high-alchemy value descending then identity
label, retain persisted keys and display explicit identity labels. Preserve
plain literal options and loadout options; defaults stay the exact11 Alcher
keys from brief90, including any currently unavailable default.

Root requirements beyond04s:
- A typed descriptor must explicitly identify the supported high-alchemy
  metadata expression. Do not infer this policy for an arbitrary .map or
  every options array referencing FODDER. Verify the bounded frozen expression
  chain and operands before attaching the descriptor; unknown transformations,
  malformed declarations and unrelated computed maps remain unresolved.
- Preserve exact prefix and candidates from the declared source; never paste
  the36-entry foreign FODDER into Rust. Scope lookup to the actual sibling
  settings blob/imports. No unknown-expression partial success.
- Preserve label tie order for supported frozen facts; native comparison must
  match the demonstrated localeCompare ordering for these labels. No runtime
  policy/router/controller change. Cosmetic gp formatting is optional and
  should not expand this task.
- No SelectedGameData may expose only the verified non-item prefix custom;
  no unverified item key may become selectable. Profile mismatches retain
  existing refusal semantics. Resolver must not mutate shared card/schema.
- TUI/panel checkbox/choice edits persist values in the same option order as
  their current contract. Preserve e72 text/number/array editing and CAF
  terminal restoration. Avoid unrelated UI layout or lifecycle changes.

Tests exercise real registry descriptor extraction, unsupported expression
refusal, both pinned selected revisions, missing alias, absent facts, sorted
value ties (rune2h before platelegs; Air/Earth/Fire/Water battlestaves), explicit
dragonhide identity labels, shared schema with different bound facts, exact
unchanged defaults, plain literals/loadouts and UI value-vs-label selection.
Use existing focused TUI/panel tests as controls. No source-text snapshots or
V8 browsing evaluator. Root's native falsifier: before Start, Items shows
custom then rune_platebody; custom reveals Custom item; choosing a supported
value and fresh Start still performs alchemy.

Exact host/client Git export plus owned overlay and independent empty target,
focused affected tests and strict Clippy with pre-existing lint limits clearly
separated (TUI bin has four known type_complexity findings). No broad tests
once relevant checks pass. Check disk; root owns cache cleanup. Commit owned
paths, request SAME-card reviewer with exact identities/checks, then STOP.
