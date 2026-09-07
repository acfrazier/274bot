# Native Windows server and TUI functional proof

The isolated Lost City server and one real-console TUI bot completed a local
Windows functional smoke as standard user `DESKTOP-SL99R6C\BotTest`, session 3.
The frontend exited **0**, without a timeout, after the normal teardown.
This establishes the local server topology needed for subsequent managed
accounting. It is **not** a managed cell, matched comparison, resource-budget
pass, latency proof, or accepted performance saving.

## Evidence and source

Raw evidence is under `diagnostics/windows-native-server-20260907a/`.
`bundle-b/` contains both the failed and corrected frontend runs, the server
setup/startup receipts and failures, and the short three-role collector probe.
`smoke-summary.json` derives bounded workload observations from those raw files;
`artifact-sha256.json` binds the local artifact files. The transferred archive
SHA-256 was independently checked before extraction:
`742ca721b5158420e16b59e3d679128bb541c3b75976c263e9733673ad5d3540`.

- Frontend: frozen host `952ba22`, client `2b1af85`; TUI executable SHA-256
  `5addcb8bcdf1211834d0934e3ae10db4cfc0792183df1eeaa8034731f29cde17`.
  Source and native-build evidence remain in the preceding Windows reports.
- Server base: `4c95f87efe00b068cadbd229d94736626907bd1a`, plus the explicitly
  recorded operator working-copy seed handler and loopback listener changes.
  The Mac source checkout was not changed.
- Runtime: portable official Node `v24.19.0`, verified download checksum and
  valid OpenJS Authenticode signature; executable SHA-256
  `3602f2bba10f2cbab4c36886218a33c1ab3db87290e73b033c46c77147d0237`.
  Locked `npm ci --ignore-scripts --no-audit --no-fund` exited 0.
- Fixture: fresh local RSA keypair, fresh SQLite schema from the repository's
  single-world migration, existing packed game assets, explicit isolated world
  configuration, and recorded runtime asset additions. No existing player
  saves, database, or private key were transferred. The newly generated private
  key remains on the Windows machine and is excluded from the evidence archive.

The 683 original staged files were checked on the live Windows fixture with
no unexpected hash differences. The intentional `world.json` update is bound
separately, as are `wordenc`, `multiway.csv`, and `free2play.csv`. The ordinary
`Get-FileHash` attempt could not open the in-use cache data file; the preserved
follow-up script used an explicit read stream with read/write sharing. This
was a live hash check, not a claim that mutable files were frozen during reads.

The source manifest describes the initial staged fixture. It must be consumed
together with the runtime configuration/addition receipts; it does not alone
describe the corrected runtime. The corrected frontend launch records all
three addition/configuration hashes and the public-key hash.

## Setup failures preserved and resolved

1. Initial startup (`startup-failure-01`) exited 1 because
   `WordEnc.load` directly opens `data/raw/wordenc`. Added that exact existing
   runtime asset, recording its hash; no server gameplay code changed.
2. First TUI smoke (`bottest-native-tui-20260907a`) logged in and reached scene 2,
   but failed seed step 6 after the existing 150-tick allowance: zero thieving
   XP gain. It exited 1 without an outer timeout. Raw diagnostics show no guards
   and no emitted requests. `GameMap.init` returns before reading the packed map
   cache if `${Environment.build.srcDir}/maps` is absent. The packed cache was
   present, including the 15,253,899-byte `maps-server.zip`; the separately
   configured content path was missing. Added the exact runtime map CSVs under
   `runtime-content/maps` and set `build.srcDir` accordingly. The world and
   forced stop receipt are preserved under `world-without-map-content-02`.
3. The first PowerShell configuration write added a UTF-8 BOM. Node rejected
   that JSON and the server used defaults; no frontend was launched against
   that configuration. Preserved `config-bom-failure-03`, wrote UTF-8 without
   BOM, and verified parsing with the actual Node runtime before restart.

These were distinct, diagnosed fixture corrections. The failed runs were not
reclassified as passing or included in a performance average. No timeouts,
seed requirements, workload behavior, or pixel assertions were weakened.

The operator correctly identified Lost City's setup path: wrapper `start.js`
invokes `npm run setup` when neither legacy `.env` nor `world.json` exists.
Here, `src/setup.ts` runs the configuration UI and conditionally migrates a
support-server database. The staged fixture already supplied explicit world
configuration and a fresh local schema. The normal app also conditionally packs
missing caches; its startup checks did not establish the missing content/maps
runtime prerequisite. This report describes a frozen runtime fixture, not a
complete replacement for Lost City's general installation workflow.

## Corrected live result

Native server PID **24224** started at `2026-09-07T15:54:06.0815336Z`.
Its startup log reports **7,322 static NPCs** and world ready, with an empty
stderr log in the captured proof. Actual listeners on ports 80, 43594, and 8898
all belonged to that PID at `127.0.0.1`. Root stopped only the previously owned
SSH reverse-forward tunnel PID 686; the Mac server and supervisor were untouched.

Corrected TUI run `bottest-native-tui-20260907b`, PID **15380**, started at
`15:55:09.6582864Z` and exited 0 at `15:59:03.5842508Z`, timeout false.
The launcher retained the real process handle and used the real console path.

| Observation | Raw evidence |
| --- | --- |
| Native observation boundaries | 53.6836893 to 173.7097243 harness seconds: 120.026035 seconds |
| Ordinary observation samples | 117 rows, all ready=1 and active=1 |
| Diagnostic observation rows | 117 inside the native boundaries; no failure or slot error, all ingame and scene_state=2 |
| Renderer | Absent throughout observed TUI rows; no GPU claim |
| Steals | Boundary paint 3 to 11, with pickpocket requests and XP progression in diagnostics |
| Eating | Boundary paint 0 to 1; a Lobster Eat request and inventory changes recorded |
| Banking and withdrawal | At 113.4196448s, bank open with 3 food and no coins; at 114.4389716s, bank open with 22 food; at 115.4605944s, bank closed with 22 food |
| Return to activity | Later pickpocket requests and coin increases after return to the guard area; final boundary bank trips=1 |

Banking evidence includes `Deposit Coins`, `Withdraw Lobster / Withdraw X`,
`AnswerCount 19`, and close/return requests, along with the actual inventory
transitions. It does not rely on paint alone. No screenshot or input-latency
proof is claimed by this TUI run. This custom smoke does not have the managed
receipt/cache enclosure required by the full qualification and matched gates.

## Native three-role sampler capability

Frozen collector source `e425799` ran from the Austen SSH account against the
explicit BotTest frontend PID 15380 and server PID 24224, plus collector self
PID 14136. Fifteen required-grid rows at 0.2-second cadence completed with
status `ok`, no role errors, and process exit 0. Current working set and CPU
counters were available for each role with stable Win32 creation identities.

This was a three-second configured probe with approximately **2.795 seconds**
of observed common span. Its summary explicitly denies full configured-duration
coverage. It proves this access/topology capability only: no graceful managed
stop, complete frontend observation coverage, six-role enclosure, perturbation
acceptance, descendant accounting, or savings follows. Overlap with this sampler
and setup hash checks also excludes treating the smoke as a clean resource cell.

## Remaining work

The managed stop/cleanup/parent-ownership and diagnostic Windows-import/home
changes still require their separate native validation. Real Windows TUI
pseudoconsole transport and input probes, managed binding qualification,
matched builds/runs, resource and latency targets, panel/rendering/lifecycle
coverage, and final whole-branch Grok-4.6 remain incomplete. The frozen frontend
predates the main-modal restoration task; this TUI proof cannot validate that
client change or a later binary. No performance acceptance is recorded.
