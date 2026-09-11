# Native startup preparation evidence

Recorded 2026-09-11 00:25 UTC. Startup source `814e5293` and its report
`6eba96ac` passed corrective Grok 4.5 review 1162, session
`20260910_195251_648621`; review report `8c10ac3a`. This report records root's
native observations on later frozen binaries containing that source.

## Mac

Native `6c6bb2d5` / client `56d8027` ran BankFletcher on local 289. A three-second
sample during preparation at approximately +20 seconds placed 220 of 231 main
thread samples in rendering, with no main-thread `hash_file`. All 231 samples
of the identified preparation worker were in resource validation/hashing.
The retained sample is
`evidence/panel-startup-preparation/native-6c6bb2d5-startup-sample.txt`.
This supports the removal of large validation reads from the event thread.
Total startup still took about 38 seconds; this is not a startup speedup claim.

The run completed the full fletching bank cycle. Subsequent `41cfc85e` added
capture diagnostics. A process launched from an owned minimal app bundle and
raised through native accessibility produced the existing internal GPU capture:
`evidence/catalog-headed/r289-bank-fletcher-100adccc-41cfc85e-bundled/shots/2026-09-11T00-09-01_52246/2026-09-11T00-10-15_bank_fletcher_terminal.png`.
The read 2240x1160 image shows the complete application window, active game
scene, knife, one new bow, remaining logs and one bank trip. Its sidecar is an
actual scene-2 snapshot. The complete loop is established by the gameplay
observations, rather than the paint counter alone.

Earlier unbundled runs armed and requested captures but did not finish readback.
There were no recorded map/poll errors. Making the bundled window visible
restored capture, which suggests a presentation/visibility cause; no explicit
occlusion event was recorded. No second capture backend was introduced.

## Windows

Native `6c6bb2d5` / client `56d8027` on the dedicated BotTest desktop completed
30 alchs, deposited 900000 coins and stopped without runtime errors. Its outer
run was 197.962 seconds. Startup was about 17 seconds. Exact build/source and
process identities are under
`evidence/platform-preparation/catalog-smoke/windows/` and its
`native-6c6bb2d5/receipt.json`.

A bounded WM_NULL probe sampled visible windows owned by the exact process
for the first 60 seconds. HWND 917560 responded to all 113 probes; HWND 1179932
responded to 110 of 112. The two timeouts were approximately 501 and 509 ms,
at elapsed 1.322 and 17.131 seconds. Window title/class was not recorded, so
the observations do not identify which handle was the main window. These
probes do not show a sustained validation stall, but do not establish perfect
responsiveness or lower total startup time.

The read internal 1688x870 PNG shows the complete native window and game scene:
`evidence/platform-preparation/catalog-smoke/windows/native-6c6bb2d5/shots/2026-09-11T00-04-03_4712/2026-09-11T00-04-37_alcher_terminal.png`.
The sidecar records an actual scene-2 snapshot. The paint counter lags the first
cast; the runtime observations independently establish all 30 casts and banking.

## Changed resources and limits

Both native platforms were given a disposable full-size navflags copy with one
byte flipped before preparation. Both exited 1 with `navigation/profile mismatch`,
without starting a slot or creating a vault. The original flags were unchanged.
Receipts and hashed logs are under
`evidence/panel-startup-preparation/native-mac-mismatch-41cfc85e/` and
`native-windows-mismatch-6c6bb2d5/`. The 261 MB disposable input is retained
locally and excluded from committed evidence. Windows cleanup removed only the
owned proof task after it reported successful completion of the expected refusal.

Receipt log/timeline hashes and all five Windows run evidence-file hashes were
verified after collection. The source review covers generation cancellation and
the consuming final-validation ticket. These native checks cover live auto-boot
and a pre-bind mismatch. Interactive Unlock and a disk mutation after binding
were not independently exercised here. Fleet, reconnect, CPU/GPU freeze and
full frontend acceptance remain separate campaign gates.


## Interactive Mac follow-up at 00:36 UTC

An isolated interactive entry was built against frozen host 41cfc85e/client
56d8027. Its only additional code installs the existing `IsolatedEnv` before
calling the production panel. No product source changed. The exact entry,
source manifest and binary SHA a7a808b98c1db8242f3ad2b4381aab27d27bb8240d18a78d49eac4fe448716d5
are recorded beside the proof. The production panel-play binary also built,
but was not launched against the operator's stores.

After native preparation completed, root created an empty disposable fixture
vault with the unchanged vault crate, then flipped one byte in the copied flags
file. An actual click on Unlock started background final validation. The UI
reported `navigation flags changed after profile binding; restart required`,
kept the vault locked, and started no slot. Its vault hash remained unchanged.
Root restored the original flags bytes and clicked Unlock again. The profile
selector and Profiles button replaced the passphrase controls, and the error
and preparation banner cleared. The same empty vault file remained unchanged.
Closing the window exited 0.

Both 2240x1160 internal F12 PNGs were read; they show refusal and successful
Unlock, respectively. These images prove UI state only. The current manual-shot
sidecars serialize default empty snapshots and are not gameplay evidence. Files,
verified hashes, mutation/restoration timestamps and process receipt are under
`evidence/panel-startup-preparation/interactive-mac-41cfc85e/`. This closes the
Mac interactive Unlock and after-bind mutation observations left open above.
Equivalent Windows interactive coverage remains separate; Windows auto-boot
and pre-bind refusal are already recorded.
