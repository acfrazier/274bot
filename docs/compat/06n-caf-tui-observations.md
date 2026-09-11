# CAF TUI control observations

2026-09-11. Host `caf6b809f`, client `9d090ed`, newer frozen catalog `8e7d965b`.
Actual Grok4.5 review1252 approved the deferred proof-output change. Root built
Mac and Linux copies from6292 verified package files in isolated targets. The
only private entry wraps production `tui::bin::main()` with `IsolatedEnv`.

Five actual140x40 PTY runs pass the existing Alcher smoke: Mac274 twice,
Mac289 and Concord274/289. Every raw PASS appears after leaving the alternate
screen. No PASS appears in an active-screen capture. The PASS snapshot contains
26noted chainbodies,26nature runes and30000coins, with the existing positive
MagicXP predicate. This is a bounded frontend smoke, not full catalog banking
or supported-option acceptance.

| Surface | Paused notes/runes/coins | After Resume | Stop |
|---|---|---|---|
| Mac274 second run |18/18/270000, stable twice|12/12/450000|idle, paint cleared|
| Mac289 |17/17/300000, stable twice|10/10/510000|idle, paint cleared|
| Concord274 |9/9/540000, stable twice|6/6/630000|idle, paint cleared; one in-flight cast to660000|
| Concord289 |22/22/150000, stable twice|18/18/270000|idle, paint cleared|

The first Mac274 run additionally shows actual Browse, Alcher selection and
Start causing new work after Stop. Concord289 shows the same path, a parameter
dialog opening, restart from18notes to12notes and450000coins, then a second Stop.
Inputs are ordinary terminal mouse/key sequences forwarded through the PTY;
there are no direct ScriptRunner calls from the control helper.

The Concord289 parameter dialog reveals an existing gap: numeric and string-array
fields cannot be edited, and background glyphs show through. Task t_32400aa2
owns that correction and subsequent independent review. Merely opening Params
does not qualify editing, persistence or settings-dependent behavior.

Concord274 received login5 during fixture relog, recovered through the existing
retry and completed actual alchemy. The transient response is retained. Both
owned engines are stopped; game/HTTP ports are absent and the original274
system service remains inactive. Concord did not compile or use Xvfb.

The Mac build was given a relative target path. Cargo resolved it inside the
exclusively new package source tree; `mac-caf6b809/actual-target.json` records
the actual compiler path and preserves the original receipt. It was isolated
from other source builds. The Linux target was absolute and started empty.

Recompute with `python3 docs/compat/evidence/platform-isolated/harvest_caf.py`.
Raw receipts, ANSI, interpreted screens and inputs live under
`evidence/platform-isolated/{mac,concord}-tui-caf6b809`; compiler receipts/logs
under `{mac,linux}-caf6b809`. `helpers-caf6b809` retains exact original launch,
input and capture helper sources with a manifest; their original workspace
locations are under `.superpowers/platform-preparation/`.

Pyte does not restore an alternate-screen buffer: a capture taken after process
exit can visually mix normal-screen PASS text with old cells. Raw escape ordering
is authoritative for the output fix. Concord274's saved `stopped-before-restore`
reconstruction isolates the last actual alternate-screen frame, which root read.
These runs overlap ordinary development work and make no performance claim.
