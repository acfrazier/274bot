# Review: real cold-load gate failure and tiled candidate disposition

**Card:** `t_8693907a`  
**Reviewed commit (report):** `492e485558eb8a3794d6ae8e5256e01e885dcf85`  
**Branch HEAD at review:** `3b2eb36c8c0917ca5eef0a40c23f9be6489b9080` (STATE-only follow-on; report body unchanged from `492e485`)  
**Lens:** artifact (round 1) — independent local archive/read-only audit; no probes, builds, SSH, source/tool edits, or STATE edits by reviewer  
**Reviewer model/provider (actual):** `grok-4.5` / `xai-oauth` (profile `reviewer` defaults; no task model/provider override)  
**Worker session:** `20260909_141247_c6b89d`  
**Outcome:** **APPROVED** — cold/peak evidence and park disposition are correct under the parent-approved necessary-gate method.

Park means: no performance acceptance, no Stage B, no 59-case route-budget increase, no +10% threshold waiver, no favorable-sample re-run. Source reconciliation and final whole-branch Grok 4.6 remain root responsibilities.

---

## Scope checked

- Root report: `docs/memory/nav-stage-a-cold-screen-report.md` at `492e485`
- STATE top boundaries (14:11 park result; 14:17 review-in-progress pointer only; no STATE edit by reviewer)
- Local `diagnostics/nav-stage-a-native-preparation/concord-cold-screen-01.tar.gz` + manifest, `root-cold-screen-audit.json`, `root-cold-original-git-audit.json`, `concord-cold-screen-01-metadata/`
- Parent method approval `t_eb56e9b6` / `1a717e9` (cold/peak-only; not full 59-case routing)
- Prior binary/tool pins from first-attempt review (`f24de7c`, dense `29b7aea`, tiled `8385bab`, client `3456edc`)

---

## 1. Archive, auth, binding (independent)

| Artifact | Claim | Independent check |
|---|---|---|
| `concord-cold-screen-01.tar.gz` | 5169413 B, 720 files, SHA `eb545cb7…7f5ba6` | Stream SHA + size match; tar file count 720; every manifest member length/SHA matches streamed member (0 mismatches / 720) |
| Auth | SHA `c967039a…24bf65` | File SHA of metadata `root-cold-authorization.json` exact |
| Auth scope | cold/peak only; `full_route_acceptance: false` | Present; `accepted_metric_scope` = startup_peak + cold_load; proposal_review `t_eb56e9b6` |
| Caps | 120 wall / 90 CPU / 1GiB RSS / 4GiB AS / 256KiB output | Auth limits and all 12 clean receipts match |
| Order | 6-pair AB/BA | Auth order dense,tiled,tiled,dense,dense,tiled,tiled,dense,dense,tiled,tiled,dense |
| Pack | 73438581 B, SHA `2f393138…4a30` | `root-input-stage/input.navpack` full hash match |
| Routes | row40 only TSV SHA `0694cd20…c9516` | `cold-control.tsv` and `real-release/routes.tsv` both match; content `3222 3218 0 1855 1280 0 0 0 0 0` |
| Tool commit | `f24de7c…` | Auth `tool_commit`; staged `stage_a.py` / `stage_probe.rs` / allocator hashes match auth `tools` |
| Four Linux exes | same as first-attempt | Stream SHA: dense-clean `fe0c89bf…`, tiled-clean `08d92e97…`, dense-counting `a08fcee7…`, tiled-counting `15dd17b1…` |
| Admissions | relocated, unchanged | Admission file SHAs match relocation receipt; dense commit `29b7aea…`, tiled `8385bab…`, client `3456edc…` |
| Original Git bind | 209 dense / 210 tiled | Every manifest `git_blob` bytes hash+size match (209/209, 210/210). Host spans `ab8284ad…` / `9a094d8b…` match `root-cold-original-git-audit.json`. Probe-staged `Cargo.toml` / `collision.rs` / `router.rs` differ from original blobs as expected instrumentation; originals still git-bound |
| Preflight | SHA `a4a2ad37…` | Metadata preflight file SHA match; progress wrappers returncode 0 at ~98.086s clean + ~17.929s counting |
| Relocation | new `concord-cold-screen-01` | Receipt: admissions unchanged, no rebuild/re-admit; binaries SHAs match |

No production function change, rebuild, or re-admission is indicated by these receipts.

---

## 2. Cold / peak recompute from raw phases

Definition used (ownership cold): per process  
`cold_ns = retained_input.elapsed_ns + decoded_converted_retained_input.elapsed_ns`  
`peak_B = decoded_converted_retained_input.process_peak_rss_bytes` (construction complete, input retained)  
`world_live_B = dropped_input_world_live.current_rss_bytes`

All 12 clean processes: `returncode 0`, hard-AS active, full phase set through `world_drop` + summary. Aggregate `real-release/result.json` has 12 results.

| # | Arm | cold_ns | peak_B | world_live_B | NoPath |
|---|---|---:|---:|---:|---:|
| 00 | dense | 109754611 | 149553152 | 76275712 | 24 |
| 01 | tiled | 453578643 | 153616384 | 7020544 | 24 |
| 02 | tiled | 456610822 | 153485312 | 6987776 | 24 |
| 03 | dense | 111225850 | 149553152 | 76230656 | 24 |
| 04 | dense | 106447739 | 149553152 | 76275712 | 24 |
| 05 | tiled | 449501422 | 153616384 | 7024640 | 24 |
| 06 | tiled | 448445286 | 153616384 | 7012352 | 24 |
| 07 | dense | 103660342 | 149553152 | 76230656 | 24 |
| 08 | dense | 107518611 | 149553152 | 76271616 | 24 |
| 09 | tiled | 558813724 | 153616384 | 7020544 | 24 |
| 10 | tiled | 451531659 | 153616384 | 7020544 | 24 |
| 11 | dense | 110045681 | 149553152 | 76292096 | 24 |

Pair table in the report matches this order (including pair5 tiled **558.814 ms**). No sample dropped.

**Medians (independent):**

- Cold dense median **108636611 ns** (108.636611 ms → report 108.637 ms)
- Cold tiled median **452555151 ns** (452.555151 ms → report 452.555 ms)
- Δ **343918540 ns** = **343.91854 ms** → report **343.919 ms**; ratio **+316.577015%** → report **+316.6%** / task **+316.577%**
- Gate ≤+10%: **FAIL** (failure not masked by baseline range)
- Baseline dense range 7565508 ns = 7.565508 ms = **6.964050%** of median (report 6.964% / 7.566 ms)
- Peak dense median **149553152 B** (142.625 MiB); tiled **153616384 B** (146.500 MiB)
- Peak Δ **4063232 B** = **+3.875 MiB** ≤ +8 MiB: **PASS**
- World-live RSS medians **76273664** / **7020544** B match audit; diagnostic microprobe only — **not** deployed Stage B / per-bot savings

`root-cold-screen-audit.json` metrics/samples/layouts match the above exactly; cold classification `fail`, peak `pass`, `route_metrics_accepted: false`, disposition park.

Raw one-row route CPU/p99 labels (if emitted by the tool) are **diagnostic only** — not 59-case routing verdicts, not Stage A pass, and not a basis to claim route regression from this screen.

---

## 3. Counting layout (separate; not clean timing)

Both counting processes returncode 0, full phases, layouts:

- Dense requested elements 65142784 + 8142848 = **73285632**; usable 65146864 + 8146928 = 73293792; header **104**
- Tiled directory **63616 × 8 = 508928**; pool **3090 × 1152 = 3559680**; total requested **4068608**; usable 511984 + 3563504 = 4075488; header **120**; DenseTile align 8
- Reduction 73285632 − 4068608 = **69217024 B** = **66.010498 MiB** (report 66.0105 MiB)
- Narrow warmed collision-read: **0 allocations / 0 requested bytes** on both arms

Do **not** treat counting timing/RSS as clean, and do **not** promote allocation reduction or world-live RSS medians to accepted Stage B / per-bot savings. Earlier 59-case CPU-cap-consistent baseline failure remains separate and unresolved.

---

## 4. Control / interruption caveat

- `root-cold-interruption.json`: `terminated_owned_pids: []`; reason records that outside-destination is **not** an entry short-circuit of `find_bounded_impl`
- `root-cold-control-readback.json`: completed_schedule 12, completed_counting 2, remaining probes empty; stop check was a **no-op** (no PID / no signal) after the schedule had already finished
- Successful completed runs are not contradicted by the no-op receipt. Do not describe the control as immediate-return. Route values remain diagnostic; cold/peak gates remain valid.

---

## 5. Disposition judgment

| Question | Answer |
|---|---|
| Cold necessary-gate failed under approved +10% method? | **Yes — FAIL at +316.577% / +343.919 ms** |
| Peak gate passed (+3.875 ≤ +8 MiB)? | **Yes** |
| All six pairs retained, including 558.814 ms candidate? | **Yes** |
| Baseline range masks failure? | **No (6.964% ≪ 316%)** |
| Route/CPU/p99 or Stage A routing accepted? | **No** |
| Counting/layout observations over-claimed as Stage B? | **No — correctly scoped** |
| Park (no acceptance / Stage B / budget raise / waiver / cherry-pick)? | **Yes — required** |
| Material corrections to report arithmetic or binding? | **None found** |

**APPROVED.** Evidence supports parking the tiled navigation candidate for campaign performance acceptance under the existing contract. Root may leave the +10% cold limit in force; any acceptance of the measured ~344 ms startup tradeoff needs an explicit future contract change (none is made or inferred here). Optional operator question noted in STATE 14:17 is unanswered and is not consent.

---

## Reviewer checks (machine-oriented)

- report commit `492e485` docs-only report+STATE park boundary; HEAD `3b2eb36` STATE-only tracking of this review
- archive SHA `eb545cb73f725cc07e307d24853be935614d1aea0c312ca1a617a823b87f5ba6` size 5169413 members 720; manifest 720/720 hash match
- auth SHA `c967039a1ad535d0d830bb4eb0b0057177ec9517b0b2e3ce73cf7d2a4624bf65`; routes SHA `0694cd204e005868d3c772ccf65f389d9c57b8fcee0166279ac175c2c75c9516`; pack SHA `2f393138…` 73438581 B
- four exe SHAs match first-attempt; dense/tiled/client/tool pins 29b7aea / 8385bab / 3456edc / f24de7c
- original git_blob bind 209/209 and 210/210; host spans match original-git-audit
- 12 clean + 2 counting returncode 0; phases complete; NoPath 24×12; caps unchanged
- cold medians 108636611 vs 452555151 ns; delta +316.577% / +343.91854 ms FAIL vs 10%; baseline range 6.964%
- peak 149553152 vs 153616384 B delta 4063232 B (+3.875 MiB) PASS 8 MiB
- layouts 63616×8 / 3090×1152; requested 73285632 vs 4068608; narrow 0/0
- interruption no-op; control not immediate short-circuit
- reviewer grok-4.5 xai-oauth session 20260909_141247_c6b89d
