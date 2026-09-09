# Review: Stage A first real attempt failure + isolated cold/peak gate proposal

**Card:** `t_eb56e9b6`  
**Reviewed commit:** `8e959ed745912ac6d9065fd74511ffafc98225a6`  
**Lens:** artifact (round 1) — independent local archive/read-only audit; no probes, builds, SSH, source edits, or STATE edits by reviewer  
**Reviewer model/provider (actual):** `grok-4.5` / `xai-oauth` (profile `reviewer` defaults; no task model/provider override)  
**Worker session:** `20260909_095544_27c777`  
**Outcome:** **APPROVED** — bounded failure evidence and the narrow necessary-gate proposal are methodologically sound for root to freeze/release under the stated constraints.

This does not release a measurement run, Stage A performance acceptance, Stage B, or any 59-case retry.

---

## Scope checked

- Root report: `docs/memory/nav-stage-a-first-attempt-report.md` at `8e959ed`
- STATE top boundary (failure + proposal pointer only; no STATE edit)
- Local `diagnostics/nav-stage-a-native-preparation/` archives, manifests, root verification receipts, extracted metadata
- Prior correctness row40 outcomes: `diagnostics/native-nav-differential-preparation/results-760d3ac/metadata/root-real-audit.json`
- Prior tool approval context: parent `t_fa5a3462` APPROVED `f24de7c` (Grok 4.5); not re-litigated here

---

## 1. Archive and qualification facts (independent)

| Artifact | Claim | Independent check |
|---|---|---|
| `builder-native-evidence-f24de7c.tar.gz` | 5507455 B, 667 files, SHA `5b82a2c8…8bb621` | `shasum` match; `tar -tz` count 667; root `root-builder-archive-verification.json` `verified:true` members 667 |
| `concord-stage-a-attempt-01.tar.gz` | 5198802 B, 678 files, SHA `abffe5b9…70f943` | `shasum` match; `tar -tz` count 678; root `root-concord-attempt01-archive-verification.json` `verified:true` files 678 |
| Manifests | 667 / 678 file inventories | `jq` length on both manifests matches |
| Sample tar stream vs extract | receipt/out/result/postflight | Stream SHA of `00-dense.receipt.json` / `result.json` / `00-dense.out` / `root-failure-postflight.json` equals extracted files and manifest entries where present |
| Auth | SHA `2c14748c…17edf` | `shasum` of extracted `root-real-authorization.json` |
| Routes 59-row TSV | SHA `49e348ea…bc125` | proposal-01, root-input-stage, and real-release copies all match; 59 lines; line 40 = `3222 3218 0 1855 1280 0 0 0 0 0` |
| Pack | 73438581 B, SHA `2f393138…4a30` | Bound in authorization (`input_bytes` / `input_sha256`); full bytes present in archive as `real-release/input.bin` and `root-input-stage/input.navpack` |
| Tool commit | `f24de7c…` | Authorization `tool_commit`; admissions use dense `29b7aea…` / tiled `8385bab…` / client `3456edc…` |
| Four Linux executables | report table SHAs | Concord admissions `executable_sha256`: dense-clean `fe0c89bf…`, tiled-clean `08d92e97…`, dense-counting `a08fcee7…`, tiled-counting `15dd17b1…` — exact match |
| Concord qualification | guards0, clean0 (~218.5s), counting0 (~216.5s), integration0 (~14.8s) | `root-qualification-progress.json` four steps all `returncode:0` with those walls; clean/counting `qualified:true`; integration `qualified:true` |
| Guards 17 tests / hard-AS | both platforms tooling-only | Concord `guards.err`: **Ran 17 tests … OK**; `result.json` `qualified:true`, `native_hard_as_qualified:true`. Builder progress also guards/prepare/build×2/generated×2/test `returncode:0`. These qualify tooling only — not real performance acceptance. |
| Proposed-manifest thresholds | +8MiB peak / +10% cold / +5% routeCPU / +2ms p99 | Extracted manifest SHA `0a64a4ff…` matches auth binding; thresholds and baseline-repeat noise policy present as claimed |

Tooling/source/allocator/admission claims in the report match these receipts and prior `t_fa5a3462` APPROVED `f24de7c`. No re-admission or rebuild is asserted for Concord real attempt (admissions are the transferred builder binaries).

---

## 2. Failure classification (independent)

Primary evidence under Concord real-release:

- `00-dense.receipt.json`: `returncode: -9`, `failure: "exit"`, `wall_seconds: 90.2305…`, limits `cpu:90` / `wall:120` / `rss:1GiB` / `address:4GiB` / `output:256KiB`, `address_guard_active: true`, `output_bytes: 916`, `sampled_group_peak_bytes: 149721088` (~142.8 MiB)
- `00-dense.out` (916 B): phases **startup → retained_input → decoded_converted_retained_input → dropped_input_world_live → hot_lookup** only. **No** `hot_routes`, `world_drop`, `post_drop`, or summary marker
- Partial timings (diagnostic only, not accepted gates): retained_input ~61.558 ms; decode/convert ~70.346 ms; drop ~4.856 ms; hot_lookup ~40.363 ms. After hot_lookup, `process_cpu_ns` ≈ 185 ms cumulative — kill occurs long after these markers without a completed route aggregate
- `result.json`: `qualified: false`, `results: []`, failure string names bounded receipt `00-dense`
- Wrapper: `root-real-progress.json` single entry `returncode: 1`, `wall_seconds: 93.020…`; `real-step-00.err` RuntimeError stop after first probe — **no candidate arm, no counting diagnostic**
- Postflight `2026-09-09T13:50:52Z`: same boot `2217ec26-…`, `global_oom_kill_since_boot: 0`, `memory_available_kib: 959576`, `swap_total_kib: 0`, `remaining_probe_processes: []`
- Preflight: 2 CPU, ~2014852 KiB RAM, ~967068 KiB available, idle ~98.16%, steal 0, no conflicting campaign processes; separate ~642 MB `MainThread` service noted and not tuned

**Kill cause classification (approved):** consistent with the **hard CPU guard** (wall ≈ 90s CPU cap; SIGKILL `-9`; RSS peak far below 1 GiB; OOM-since-boot 0). **Final process CPU is not in the receipt**, so the exact kernel accounting source is not directly observed — report correctly refuses overclaim.  

**Not demonstrated:** tiled regression, dense↔tiled paired comparison, cold/peak/route performance pass, or accepted real route aggregates. **Stage A remains OPEN.** Do not retry this 59-case release or raise caps on the evidence of this card.

---

## 3. Proposal evaluation (method only — not a release)

Proposed next screen (report § “Proposed next decision”):

1. Same admitted binaries and caps; full original pack; new owned verified relocation of qualified artifacts (no overwrite of attempt-01, no rebuild/re-admit)
2. Selector **only** earlier-reviewed row40: `3222 3218 0 1855 1280 0 0 0 0 0`
3. Freeze **new** auth / input / selector hash / hardware preflight; same 6-per-arm AB/BA order from proposed-manifest
4. Evaluate **only** already-declared startup peak (+8 MiB) and cold-load median (+10%) under existing baseline-repeat noise policy
5. Route CPU/p99 and other emissions stay diagnostic; must not replace 59-case routing or Stage A acceptance
6. Clear cold/peak failure → park/review candidate; pass or inconclusive → Stage A still open and full routing needs a **separately reviewed** resource/phase plan
7. Explicit refusals: no 59-case retry, no cap escalation, no source/binary change for selector-only, no hidden acceptance, no rerun-until-favorable, no Stage B

**Row40 control verified:** `root-real-audit.json` `cases[39]` label `outside-destination`, selector matches, all three lanes NoPath (`fixed-model`, `fixed-tele-model`, `host-outcome`). Destination `1855,1280` is outside pack origin `(1856,1280)` (prior native differential geometry). Fast NoPath control is a legitimate way to let unchanged load/lookup/drop instrumentation finish without the expensive 59-row route batch.

**Methodological judgment: SOUND.**

- Isolates already-approved **necessary** startup/cold gates without pretending sufficiency for Stage A routing.
- Preserves failure/non-acceptance of the stopped 59-case attempt.
- Avoids binary churn for an explicit selector file change (new routes hash + auth freeze only) — consistent with task instruction not to require a new binary solely for that.
- Keeps noise policy and non-promotion rules aligned with frozen `proposed-manifest.json`.
- Pass/inconclusive correctly leaves Stage A open rather than smuggling a one-row route metric into full acceptance.

**Non-blocking caveats (do not block approval; root owns release hygiene):**

- One-row route metrics must remain labeled diagnostic in any future summary UI/report text (proposal already states this).
- OS file cache still not flushed (cold **ownership**, not cold FS) — already in manifest `cold_definition`; do not rebrand.
- Optional counting diagnostics only after clean schedule success, and never mixed into clean comparisons — already stated.
- Actual freeze/release/preflight remains root-owned after this review.

No material defect requiring a different minimal alternative was found. Expanding into production optimization or executing the screen is out of scope for this card.

---

## 4. Verdict

| Question | Answer |
|---|---|
| Bounded failure evidence accurate and preserved? | **Yes — approve** |
| Kill classification (CPU-guard-consistent; final CPU unavailable; not tiled regression / paired pass)? | **Yes — approve** |
| Qualification = tooling only (builder + Concord 17 tests / clean / counting / integration)? | **Yes — matches receipts** |
| Narrow cold/peak necessary-gate proposal methodologically sound for root freeze/release? | **Yes — approve** |
| Measurement / Stage A closed? | **No — Stage A OPEN; no run released by this review** |

**APPROVED.** Root may freeze and release the proposed isolated screen under the proposal’s constraints. This review does not itself authorize SSH, launch, or cap changes.

---

## Reviewer checks (machine-oriented)

- commit `8e959ed` docs-only report+STATE; reviewed report body against local diagnostics
- builder archive SHA `5b82a2c856485adc4e76cb06a760050455849885c2853acbad02c3bf1e8bb621` size 5507455 members 667
- concord attempt-01 archive SHA `abffe5b9c367b44ebbd0456beeb7dec9e9f986de0ce096ead5c9a4281870f943` size 5198802 members 678
- root verification JSONs verified:true for builder/attempt-01/payload/run-05
- auth SHA `2c14748c27676284462b89088dfd5984345c547c406509e3109b40182df17edf`; routes SHA `49e348ea78806c8278d720d27b171c54a0a209930fa8191ffcd38d2e9bbbc125` (59 rows)
- 00-dense returncode -9 wall 90.231s cpu cap 90; results []; phases through hot_lookup only; peak RSS 149721088; postflight OOM 0 / no remaining probes
- four exe SHAs match report; dense/tiled commits 29b7aea / 8385bab; client 3456edc; tool f24de7c
- concord guards Ran 17 OK native_hard_as_qualified true; qual clean/counting/integration qualified
- row40 audit index 39 outside-destination NoPath ×3 lanes
- proposed-manifest thresholds 8388608 / 0.10 / 0.05 / 2ms; noise_policy baseline range; no duplicate tool review
- reviewer grok-4.5 xai-oauth session 20260909_095544_27c777
