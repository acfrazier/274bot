# Diagnose missing imported catalog parameter metadata

Use grok46 defaults. Bounded read-only design; own04p-catalog-parameter-metadata.md
and unique evidence/catalog-parameter-metadata/. No runtime/loader/frontend,
foreign, ledger or fixture edits, no LIVE, no subagents.

Actual reviewed e72cfb39 TUI Params opens Alcher with Items to alch string[]
as free text and no choices. Saving custom succeeds but Custom item remains
hidden. Numeric and string[] editing/cancel/reopen work. Screens/input/raw under
evidence/platform-isolated/concord-tui-e72cfb39/r289-params/controls/{choices,
custom-saved,invalid-number,cancelled,persisted}.*. Mac274 equivalent form shows
empty item list and27number. Same source/binaries are hash-qualified; this is
separate from the completed e72 editor implementation, which consumes shared
SettingDef metadata. Do not call its character input broken.

Both frozen Alcher sources import ALCH_OPTIONS, DEFAULT_ALCH_ITEMS and
CUSTOM_ALCH_KEY from sibling AlcherLogic. Settings items options/default refer
to those constants; Custom item showIf.anyOf references CUSTOM_ALCH_KEY. Current
LoadLibrary uses rs2b0t_registry::settings_schema_from_source. Diagnose exact
loss of type/default/options/optionLabels/showIf through loader to shared
SettingDef/store and panel/TUI. Check whether the native panel shares this
specific omission. Account for definitions that depend on native item metadata
and imported static expressions, not only literal objects.

Design the smallest host-owned correction that exposes existing supported
catalog settings faithfully before Start. Do not copy a foreign settings
runtime, execute arbitrary catalog actions during Browse, invent option lists,
change script defaults, or silently remove supported conditions. Preserve
untrusted-script boundary, load/start lifecycle, responsiveness and current
snapshot ownership. Scope correction by concrete frozen catalog examples;
avoid a new general JavaScript evaluator or speculative AST framework. If
existing isolated module evaluation can provide schema safely, inspect its
actual point of evaluation and native content timing; do not assume it can.

Report exact source provenance, observed shape vs required shape, owner and
bounded file plan, meaningful tests and one actual UI falsifier. Distinguish
missing metadata from intentionally deferred optional native features. Root
controls implementation ownership and LIVE; current script shared files belong
to hostile t_eeea20f0 then count-dialog, special/teleport/shop/Make-X/fire.
