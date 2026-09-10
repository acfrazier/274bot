# Host boundary qualification

Step 4 is accepted for the controlled local 274/289 scope on 2026-09-10.
The same frozen host `4f43800ac0d1c2a617a1a4d03d60fd9b3824b3fc` / client
`6cb5a0b17aeef74da6b57b205e916681daee4f76` binary passed both revisions on this
Mac. Full campaign, catalog, frontend/fleet and world qualification remain open.

## Source and offline evidence

`02a-host-outbound.md` and `02b-host-snapshot-reset.md` record byte/state tests,
actual packet/session publication, reset and queued-work rejection. Actual Grok
4.5 reviewed both tasks, including the response-15 scene correction. Actual
Grok 4.6 run 1112 approved integrated source `fede3c0d` / client `6cb5a0b1`:
`reviews/host-boundary-integration-grok46.{md,json}`. The final whole-campaign
review remains required. The later harness fixture changes passed focused
compile, strict Clippy, format/diff and the controlled observations below.

The immutable export contains the reviewed world-source change `80720784` but
excludes concurrent banking WIP. Navigation is not used in this boundary test.
All 484 exported source files, including Cargo.lock, matched their recorded
hashes after compilation. Binary SHA-256:
`7929af93b334944619c0602f73d063926660f6a640de88397c59e6ae6bed5280`.

## Accepted controlled cells

Each process bound one revision to its matching loopback engine/cache, created
a fresh disposable account, observed mainland preparation, then logged out and
back in before the action baseline. Neither seed actions nor cleanup count as
accepted actions. Walking uses Rust Interactions with exact observed arrival;
NPC/door identities and actions come from the selected live snapshot.

| Revision | Result | Post-baseline observations |
|---|---|---|
| 274 | PASS, exit 0, 80.733 s | Five courtyard legs; natural Hans 2817 dialogue 4882; four perimeter approach legs; exterior Door 1530 replaced by Door 1531 offering Close; actual IF logout cleared actors, inventory, bank and scene. |
| 289 | PASS, exit 0, 29.067 s | Three courtyard legs; natural Hans 2825 dialogue 4882; arrival at exterior stand; Door 1530 replaced by Door 1531 offering Close; actual IF logout cleared actors, inventory, bank and scene. |

Raw evidence: `evidence/host-boundary/live/r{274,289}-4f43800a.{log,json}`.
The differing durations reflect different positions of the naturally roaming
Hans and resulting approach routes; they are not a performance comparison.
The door send in each run added 14 bytes and was followed by player movement
and the required live door replacement. Existing per-action deadlines remain.

## Retained failed cells and fixture corrections

| Source / revision | Result | Diagnosis and correction |
|---|---|---|
| fede3c0d / 274 | PASS, 36.290 s | Original one-tile walk, dialogue, door and logout passed. |
| fede3c0d / 289 | FAIL, 6.269 s | Walk passed; Hans absent from visible snapshot. Counts alone do not establish his exact position. Primary content confirms a perimeter patrol. |
| 490f7438 / 289 | FAIL, 38.624 s | Temporary local Hans preparation made dialogue observable, but the nearest door interaction timed out. The target was not logged, so its exact cause is unproven. |
| 8a2fafb9 / 289 | FAIL, 66.071 s | Five real courtyard legs and natural Hans dialogue passed. Nearest Door 1536 was inside the castle while the player stood outside its west wall; Open made no observed progress. |

The operator requested courtyard walking. The temporary NPC addition was
removed from the harness; no injected NPC was used by either accepted cell.
The corrected harness walks the courtyard until natural Hans is visible,
returns along the perimeter to the exterior door, and observes arrival before
interacting. It also records the exact door, outbound byte count and path.
This avoids choosing a castle-interior target across a wall while preserving
the Open and logout predicates. Earlier source reviews are historical evidence
for their named harness versions, not claims that they reviewed later changes.
Every failed cell and immutable binary/source receipt is retained under
`evidence/host-boundary/live/`; failures emitted FAIL and exited 1.

Primary fixture sources: 274 engine/content in `fixture-inputs.json`; 289 engine
`a275ea81` / content `92649430`, including the Lumbridge Hans patrol. Neither
engine was patched or restarted for these boundary cells. Source readiness,
imports, startup and asset probes are not counted as functional acceptance.
