# Native panel visual proof tools

Diagnostic capture only; no performance or scene-state inference.

`capture-panel-window.ps1` runs as BotTest in the local interactive session,
against the exact frozen fb3589a panel binary. The caller supplies the observed
PID and its PowerShell UTC start identity, a prepared dedicated visual-proof
output directory, and a unique label. It requires the target window to be
foreground and not minimized, captures its window rectangle, and records bounds,
start/end UTC, identity and PNG SHA-256. It does not focus a window, click, send
keys, launch the client, or change the runtime. Read every resulting capture;
a PNG alone does not establish scene1, state transitions, cadence or lifecycle.
Foreground checks do not prove absence of transient overlays; inspection must
reject obscured evidence. Native parser/compilation and actual capture validation
remain required before calling this helper proven.
