# Capture-corrected native input proof

Run `latency-diagnostic-input-trace-20260908-b` follows the predeclared
[native protocol](native-cohort-input-followup-protocol.md). Candidate e3a2cbf,
client fd956c9, binary SHA256
94b55e21e22be3c45aaa2c50b878965e4e730abd4e9dd69ab0533af7b850d330.
This binary includes capture correction but predates the host debug cache.

## Functional result

Native process exited 0 at 2026-09-08T13:12:17.7976723Z without timeout.
Root inspected fresh scene/configuration/after-input images during the run.
The one guarded 20-pulse sequence completed; trace showed 40 Windows/ImGui
edges, 40 stream edges with channel attached, 40 enabled host drains,
20 admitted starts and 20 matched bind/present completions on live slot
14819260750087433627, generation 2. No saturation was observed.

Root independently enumerated the raw journal: 320 unique identities,
300 Decode and 20 Panel, all starts in [144359848700,324359848700) ns,
all Completed, each completion at or after its own start and within the tail,
no loss receipts. Terminal records320/losses0/pending0, producers joined.
Reader SHA256 cd383fc3c9ced01efd12c5bd077d74b20d70db2feb4b7001e0757e4c84603d40
classifies both declared populations available: decode300/input20.
Diagnostic bucket upper bounds are 20ms decode and 25ms input.

This is functional and structural proof only. The diagnostic supervisor has no
canonical metadata.json; direct reader metadata maps the recorded qualifier
settings (panel, N1, focused-one, both responsiveness flags true). It does not
establish matched provenance, performance acceptance, or campaign completion.
Initial extraction used directory name archive, which correctly failed sidecar
provenance binding. Restoring the original run basename fixed binding without
editing any archived bytes or reader rules.

## Evidence and remaining release gates

Archive: diagnostics/windows-input-trace-cohort-b/
latency-diagnostic-input-trace-20260908-b.tar.gz; SHA256
2a28ad9a143a08beb0ebef12fba84effafa7c547f9e343adba1b3e9f815f8b9e.
Root verified all31 manifest entries, file lengths and hashes, plus manifest
SHA25685a358418d167cc62e2aad3f2fec94cff72aa4baee7d2c46b143ab59122dd028.
Raw files are under that diagnostic directory's original run basename.
root-archive-verification.json and root-cohort-analysis.json preserve checks.

Host cache bf5f49c passed actual Grok4.5/xai-oauth review, integrated primary
142f7c3 and native referencee25f328/candidateca56e14. Root host lib213 PASS,
1 existing GPU ignore. Native builds/tests of these final roles remain required.
Common crates instrumentation patch-id from each native parent is
cb00240445cb30c9d361707a1295954a60157c1a; role difference patch-id remains
6d29db24d5ccc97cdd2e5271a6a849a2dc3681c6, equal to parent role difference.
Grok4.6 correction/evidence follow-up precedes matched N16 execution.
