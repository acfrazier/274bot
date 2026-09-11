# Preserve literal catalog settings and list visibility

Use grok46 defaults. This work is independent of current client_adapter and
host runtime owners. Own crates/script/src/rs2b0t_registry.rs and
crates/script/src/settings_store.rs, their existing focused registry/settings
integration tests or unique new tests, docs/compat/04r-static-settings.md,
and unique evidence/static-settings/. No load.rs/slot.rs/host/UI edits, no
foreign source, no LIVE, no ledger, no general JS evaluator. Use fail-closed-dispatch.

Read04p-catalog-parameter-metadata.md (root renamed the completed report from
its accidental own04p filename) and frozen Alcher/AutoFighter settings. Fix the
faithful literal subset now: sibling literal string-array defaults, quoted
string-constant defaults, inner anyOf identifiers that resolve to quoted
constants, and string[] master-value membership in shared setting_visible.
Keep aliases cycle-bounded and honor the existing settings blob import scope.
Do not resolve a similarly named unrelated declaration or comment as a value.
Mixed/computed arrays must not report a successful empty or partial parse;
unknown expressions remain explicitly unresolved. Real empty arrays stay valid.
Do not manufacture computed ALCH_OPTIONS from unfiltered FODDER, reorder its
choices, or copy FODDER into Rust. Computed selected-data choices are a separate
follow-up; this task must not claim they are fixed.

Preserve existing scalar visibility/equality behavior, including case sensitivity.
04p's suggested blanket case-insensitive comparison is NOT authorized. Array
membership should compare each element using the existing scalar convention;
add CSV handling only if the existing public setting contract already permits
that representation. Malformed/unresolved conditions must not silently become
an unconditional true or a fabricated empty successful condition. Preserve the
current documented unsupported-condition behavior; report a needed semantic
change explicitly instead of hiding it in this mechanical mapping.

Meaningful actual registry tests: frozen-shaped sibling imports carry the11
Alcher default keys unchanged and resolve custom anyOf; literal Fletcher and
SHOW_MELEE remain unchanged; wrong names/cycles/mixed unsupported RHS do not
partially resolve. Shared visibility tests use actual array bag values showing
custom selected/unselected, empty array, and scalar regression including case.
Do not snapshot implementation source. This does not need frontend tests that
mirror metadata. Check real frozen catalog schema locally if existing tooling
permits, preserving source identity. Root owns subsequent native Params check.

Exact host/client source export plus owned overlay and independent target,
focused tests/strict affected Clippy, scoped commit, SAME-card reviewer with
exact identity/evidence, then STOP. Do not delete compiler caches; root owns
cleanup. Preserve all concurrent shared files.
