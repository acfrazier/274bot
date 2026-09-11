# Implement the native script paint-button seam

Use grok46 defaults after active special card t_68de6f48 SAME-card review
releases shared runtime ownership. Implement the accepted design in
04ad-paint-buttons-design.md, sections4 through verification, with exact
actor/script generation validation and one-shot consumption. Own only its
explicit named paint/IPC/slot/Play/panel/TUI files and required literal
ScriptPaint construction updates, a new paint_buttons.rs test, report
04af-native-paint-buttons.md and unique evidence/native-paint-buttons/.
No client, API game interactions, foreign code, catalog fixtures, ledger,
STATE, LIVE or broad UI changes. Root owns final native/TUI controls proof.

The complete requested slice includes real reachable panel/TUI controls,
not only headless null or text labels. Keep existing Pause/Stop, game modal
input, collapse and guardian behavior. Invalid/stale actor/generation/id
must not reach another script; reject a queued click from an old rendered
frame even when the new script advertises the same id. Retain explicit
owned generation across frontend dispatch. Hold can consume script-local
paint input on existing paint-only ticks, but cannot enable loop/gameplay.
No snapshots/world deep copies or foreign canvas runtime. Empty old Paint
buffers remain readable, unchanged paints remain quiet. No packet opcode.

Exact source export plus named overlay, one exclusive target reused for
implementation/review; check free disk and avoid duplicate full builds.
Meaningful design checks for serialization, callbacks and lifecycle;
focused frontend input tests plus affected crates and strict Clippy.
Commit scoped files, SAME-card reviewer with checked identities and
remaining native/TUI proof limits, then STOP. Do not launch a LIVE actor.
