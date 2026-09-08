# Native cohort pair 1430 evidence review

Bounded evidence review of the frozen native Windows fixed-window cohort
pair at commit `5be5607`. Not whole-branch review and not campaign acceptance.

## Identity

| Field | Value |
| --- | --- |
| Reviewer profile | `reviewer` |
| Actual model / provider | `grok-4.5` / `xai-oauth` (no task overrides) |
| Branch | `codex/memory-diagnostics` |
| Reviewed evidence HEAD | `5be56077098adba0d262ad94bab51237a40f9dd1` |
| Checkout HEAD at review | `dc51f53f52659532dae5fb5d8fbd49c85ac879d8` (descendant of freeze; pair docs byte-identical to `5be5607`) |
| Role heads (protocol) | reference `e25f32806957b1a44c75a598cbdb7ab53afc5383`, candidate `ca56e14371d1edb8c09df1276638ce196d502e36` |
| UTC | 2026-09-08T15:25:00Z |

No live runs, native changes, raw edits, or source fixes in this review.
Local Python read-only probes only.

## Verdict

**APPROVED** as fixed-window endpoint accounting evidence with honest incomplete
baseline input and **no** matched performance / latency / resource acceptance.

The report, root evidence JSON, visual receipts, and raw archives agree on the
material claims required by `current-latency-companion-plan.md` §§5–7 and
`native-cohort-pair-execution-protocol.md`. Independent recomputation from the
original-name-bound raw runs matches the published cohort membership, closure,
loss counts, boundaries, and per-slot fine p99 buckets exactly.

## What was inspected

- `docs/memory/current-latency-companion-plan.md` §§5–7
- `docs/memory/native-cohort-pair-execution-protocol.md`
- `docs/memory/windows-cohort-pair-1430-report.md`
- `docs/memory/windows-cohort-pair-1430-evidence.json`
- `docs/memory/windows-cohort-{baseline,candidate}-1430-visual-verification.json`
- Archives:
  - `diagnostics/windows-cohort-baseline-1430-archive/artifacts.tar.gz`
  - `diagnostics/windows-cohort-candidate-1430-archive/artifacts.tar.gz`
  - `diagnostics/windows-cohort-native-reader-1430/artifacts.tar.gz`
- Path rebinding sidecars, `metrics-original-path.json` / initial `metrics.json`,
  managed receipts, stimulus envelopes/receipts, native binding + legacy metrics
- Local `docs/memory/cohort_reader.py` via original-run-path binding

## Independent checks

### Archive integrity

| Archive | Bytes | SHA256 | Match report |
| --- | --- | --- | --- |
| baseline | 10682686 | `6583d20fe29c233ecc79db6cbf0e2a185cd3fcd2d2617fd11ec178ddd66996d8` | yes |
| candidate | 11294364 | `502d9eb13df31674148956ba9b5c5eeb2532cfe92f848392cdfd286f5019357d` | yes |
| native reader | 465149 | `c74bdabde25c09f40c2efe037177d034ed5286920f385c8dfd1e7ede38106bed` | yes |

Root `root-verification.json` marks baseline 60/60 and candidate 64/64 verified.
Local re-hash of every path listed in each `root-archive-manifest.json` against
the tarball: **0 SHA/length mismatches**. Each tarball contains exactly one
extra file not listed in its manifest (`root-archive-manifest.json` itself);
that is bookkeeping, not a content failure. Native reader root receipt claims
55 files verified; local tarball has 56 files with the same self-manifest pattern.

`rawBytesModified: false` on both path-rebinding sidecars.

### Original-name path binding

| Role | Original basename | Reader path resolves to immutable `raw-run-01` |
| --- | --- | --- |
| baseline | `20260908T143752Z_panel_n16_active` | yes (same resolved path) |
| candidate | `20260908T145453Z_panel_n16_active` | yes |

Initial baseline local read against directory name `raw-run-01` remains failed
as retained evidence: `metrics.json` reports
`gates.decode_cohort` / `gates.input_cohort` =
`sidecar_provenance_mismatch` (among other unavailable gates). Successful
cohort results are only under `metrics-original-path.json` after the
`original-run-path/<basename>` binding. Candidate uses the same explicit binding.
No raw bytes or reader rules were altered for the pass.

### Reader source distinction

| Reader | `cohort_reader.py` | `reference_metrics.py` SHA | Cohort gates | Notes |
| --- | --- | --- | --- | --- |
| Current local (evidence `readerSources`) | `cd383fc3…603d40` (matches workspace) | `4913d232…d3297e` | yes | Used for pair claims |
| Native installed archive | absent | `bda64033…e874dd` (≠ local) | no `decode_cohort` / `input_cohort` | Legacy GPU/input unavailable reasons retained |

Native legacy metrics (both roles): `resources=missing_resource_provenance`,
`input=no_slot_with_available_input_p99`,
`gpu=no_complete_qualified_stable_interval`, scheduling available. That output
must not replace current local cohort results. Report correctly separates them.

Evidence `readerSources` hashes for `cohort_reader.py`, `reference_metrics.py`,
and `qualify_control.py` match the workspace files used for recompute.

### Cohort recompute (local `cohort_reader.read_cohort`)

| Gate | Baseline evidence | Baseline recompute | Candidate evidence | Candidate recompute |
| --- | --- | --- | --- | --- |
| decode status | available | available | available | available |
| decode population / events / losses | 16 / 16000 / 0 | 16 / 16000 / 0 | 16 / 16000 / 0 | 16 / 16000 / 0 |
| decode aggregate fine p99 ms | [24, 25] | [24, 25] | [24, 25] | [24, 25] |
| decode boundaries | start 194334466000 end 794334466000 tail 5e9 | same | start 247676235700 end 847676235700 tail 5e9 | same |
| decode per-slot p99 | 16 slots, 0 mismatches | 16 slots | 16 slots, 0 mismatches | 16 slots |
| input status | unavailable `declared_population_incomplete` missing 1/1 | same | available | available |
| input population / events / losses | — | — | 1 / 120 / 0 | 1 / 120 / 0 |
| input fine p99 ms | — | — | [41, 42] | [41, 42] |
| input slot | — | — | slot `4211562201993209237` gen 17 | same |

One decode slot on each side has fine p99 [19, 20] ms (others [24, 25]);
evidence and recompute agree. Candidate input slot id equals the observe-start
slot0 identity from the controller publication.

Observe window length is 600s on both sides (`end - start`); tail_ns 5s matches
the protocol finite cohort tail.

### Stimulus / controller validation

**Baseline**

- `stimulus-run/post-run-envelope.json`: outcome `incomplete`,
  `incompleteReason` = observe-start trigger window missed; **spawnCount 0**;
  no helper samples.
- `managed-run/stimulus-validation.json`: status `incomplete`, reason
  `stimulus receipt run identity/outcome mismatch` (same string as
  evidence `stimulus_status.reason`).
- `managed-run/stimulus-wrapper-status.json`: spawnCount 0, same missed-trigger
  incompleteReason.
- No catch-up pulses. Capture/slot0 focus flags true on the incomplete envelope.
- Report “incomplete: trigger missed” and evidence incomplete status are both
  correct; the validation reason string is the controller’s receipt-check label
  for the missing complete receipt, while the wrapper/envelope name the root
  operational cause.

**Candidate**

- Trigger delay **74.90993900000467** s after observe-start publication
  (inside the predeclared 60–90 s window).
- Helper PID **15100**, start identity
  `windows_creation_filetime:134333532178500638`.
- Target PID **14872**, start identity
  `windows_creation_filetime:134333528941355755` (matches visual/helper
  receipts and launch-spec expected start/binary/session/rect).
- Helper exit 0; spawnCount 1.
- Immutable helper SHA `helperSha256` =
  `04822c0e9f5ade2d555e408cc707722c234bc1e7b442676975a16e7efcaebbd1`
  (protocol + both in-tree `invoke-panel-input-stimulus.ps1` copies).
- Accounted wrapper source SHA `sourceSha256` =
  `c9743b45f7a8f0c0bfbc4461b94529f316b80d9b2e2ebb713224c3bc738d5695`
  (`run_stimulus_accounted.py`).
- Helper receipt: durationSeconds 120, cadence 1000 ms, press 80 ms,
  requested/completed pulses **120/120**, missed/deferred 0.
- Events are strict alternating Left/Right from index 0; all SendInput
  down/up results 1; all releases succeeded.
- `inputCoveragePass: false` and host acknowledgement explicitly
  “not inferred from external stimulus” — SendInput completion is not host
  cohort membership (protocol). Host input cohort still closed with 120
  completed members on slot0.
- Helper accounting: wrapper-sampler pid 2460 → 121 valid / 0 unavailable,
  cpu 0.109375 s, rss 39120896; input-helper → 120 valid / **1 unavailable**
  final sample preserved, cpu 2.46875 s, rss 99172352. Resource accounting
  marks target excluded from managed helper totals. These are sampled
  summaries, not exact lifetime peak/CPU.

### Native managed binding

Both roles: `status=bound`, `binding_ok=true`, `qualified=true`,
`pair_eligible=false`, `final_acceptance_claim=false`,
`instrumentationOverheadMeasured=false`.

Full generic match-key diffs are exactly two keys:

1. `client_sources_sha256` (build-manifest side provenance labels differ:
   reference `ea640270…` vs candidate `fad2e792…`)
2. `host_conditions` (per-cell role/cell_id/preflight process snapshot hashes
   differ: `bb35b248…` vs `97eab9b3…`)

Native runtime profile flags used for the cell are equal across roles
(panel, n=16, active, focused-one, responsiveness/fine/render/scheduling/
gpu_completion profiles on, system sampler, no allocation counting, no
diagnostic sidecar, no tui input probes, debug false). Raw run `metadata.json`
also shares the same live `client_commit` /
`client_sources_sha256` / `host_sources_sha256` / nav hashes for both roles;
do not confuse those equal run metadata fields with the adapter match-key
client label difference above.

Cache content identity matches across roles
(`2a660d20…`); server_identity_sha256 differs while the long-lived game_server
pid/start identity `6728` /
`windows_creation_filetime:134332846721025908` is shared. Managed continuous
samples remain available for game_server/controller/bootstrap/launcher/collector
on both sides. Raw frontend resource **gate** stays
`missing_resource_provenance` even though diagnostic CPU/RSS numbers are
printed — availability of managed bindings does not promote that gate.

### Visual / qualitative

Both visual JSONs set `scope` and `performanceAcceptance: false`.
Root observed scene2, slot0 selected, capture input, focused 50 fps, only-render
selected; baseline 16 visible slots; candidate 14 at initial capture and 16 at
ready. Operator notes are qualitative only (no freeze on baseline; camera
movement with script still running on candidate). Not latency proof.

### Acceptance boundaries (confirmed)

- `performanceAcceptance: false` everywhere checked
- `matchedInputAvailable: false` (baseline input missing)
- Equal aggregate decode fine p99 [24, 25] is **not** matched acceptance
- No instrumentation overhead proof
- No display-scanout / hardware final-render claim from GPU callback diagnostics
- Protocol stopping rule respected: one pair, preserve incomplete baseline input,
  no rerun for a pass

## Material issues

**None.**

Non-blocking notes (do not change the verdict):

1. Evidence baseline `stimulus_status.reason` uses the controller validation
   string (`stimulus receipt run identity/outcome mismatch`) while the wrapper
   and envelope name the operational cause (missed 60–90 s trigger, zero spawn).
   Both are incomplete with spawnCount 0; report prose uses the operational
   cause. Keep both layers when mapping requirements.
2. Tarball file counts are manifest_n + 1 because `root-archive-manifest.json`
   is archived but not listed inside itself; listed entries still verify.
3. Workspace HEAD advanced one docs commit past the freeze; pair evidence files
   remain byte-identical to `5be5607`.

## Not claimed / still open

- Matched input latency comparison or p99 acceptance (≤100 ms targets remain
  plan targets, not satisfied claims from this pair)
- Matched resource non-regression / absolute budgets
- Instrumentation overhead measurement
- Lifecycle/scaling and whole-campaign `branchreviewer` / Grok 4.6 finish line
- Requirement-to-evidence mapping (root continues independently)

## Commit scope

This review writes and commits only
`docs/memory/windows-cohort-pair-1430-review.md`.
