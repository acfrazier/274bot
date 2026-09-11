# Design a host-owned script paint-button input seam

Use grok46 defaults, read-only design. Own only docs/compat/04ad-paint-buttons-design.md
and unique evidence/paint-buttons-design/. No code, builds, LIVE, STATE or ledger
edits. Commit scoped report, then complete design card; root owns implementation
handoff. Do not create implementation cards yourself.

Root912 corrected paired Air289 preparation now reaches a shared both-started
barrier, then runner fails on Paint.buttons at tick27. Exact raw evidence:
evidence/paired-catalog-fixtures/live/r289-air-100adccc-91289e9b.*.
NatureCrafter Runner onPaint calls p.buttons([{id:'gobank',label:'Go bank'/'Resume'}])
and toggles its script-local goBank only if returned id is gobank. Existing
shim/paint.js has no buttons method, Proxy throws. This is missing host support,
not evidence of a broken imported script; do not dim the card for this failure.

Read both frozen Paint.buttons contracts and all enabled caller shapes, existing
ScriptPaint recording/posting, host-play slot command/lifecycle and panel/TUI
paint/input ownership. Propose the smallest bounded host-owned descriptor and
one-shot input path, including headless no-click behavior. No foreign canvas UI,
controller/runtime or input policy copy. No gameplay action implied by a label.
An actual user event may reach only its current actor/script generation and
advertised button id, consumed once. Stop/reload/stale events/paused or guardian
holds need explicit ownership treatment preserving current lifecycle behavior.
No deep-copy of world state. Existing Pause/Stop controls remain host-owned.

Rendering an unavailable label and returning null cannot qualify button support;
never fabricate a click. Headless default returns no selection while still
recording descriptors; frontends must make required controls reachable with
real input for eventual supported-option acceptance. Identify exact fields,
files, necessary focused checks and meaningful native/TUI proof. No speculative
full UI redesign; keep rendering and script input changes bounded. If a safer
existing channel suffices, prefer it. Root can complete cross-crate implementation
and independent review once design is concrete.
