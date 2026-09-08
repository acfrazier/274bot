# Independent review: current TUI N1/N16 attribution evidence

**Task:** `t_2a69012e`  
**Reviewer:** profile `reviewer` (Grok 4.5)  
**Checkout:** `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`  
**Branch:** `codex/memory-diagnostics`  
**Report commit under review:** `8b786b96bea16d6bb9470f04fa017d603ff54739` (`8b786b9`)  
**Artifacts reviewed:**  
`docs/memory/current-tui-n1-n16-attribution-report.md`,  
`docs/memory/current-tui-n1-n16-attribution-evidence.json`,  
`docs/memory/performance-finish-plan.md` §§1, 3, 4A,  
archives under `diagnostics/current-tui-n1-1727/` and `diagnostics/current-tui-n16-1616/`.

**Scope:** evidence review only. No live runs, raw edits, product changes, or VPS/VM/Windows operations. Root owns source integration and further execution. This review does not change `STATE` or product code.

---

## Verdict

**APPROVE the attribution report’s arithmetic, provenance framing, and uncertainty limits.**

**SUPPORT next step = bounded per-bot ownership attribution design proposal only** (plan §4A style: purpose, lifetime, sharing, CPU consumers). Do **not** treat this pair as performance acceptance, an authorized optimization implementation, a live remeasure release, or proof of idle/fixed/linear cost.

**Working-target misses (descriptive, not acceptance):**

| Gate (plan §1 TUI working target) | Observed | Result |
|---|---:|---|
| N1 steady median RSS ≤ 256 MiB | 176.322 MiB | met |
| N1 native peak RSS ≤ 384 MiB | 176.902 MiB | met |
| N16 steady median RSS ≤ 512 MiB | 520.779 MiB | **miss (~8.8 MiB)** |
| N16 native peak RSS ≤ 768 MiB | 597.957 MiB | met |
| Active incremental ≤ 16 MiB / added bot | 22.964 MiB | **miss (~7.0 MiB)** |
| N16 mean process CPU ≤ 0.5 cores | 0.561746 | **miss (~0.062 cores)** |
| ≥40 client iters/slot/s | 49.669 / 49.479 | met |

**Explicit non-acceptance blockers (must remain open):**

1. Raw resource/shaped gate: `missing_resource_provenance` (both cells).  
2. Instrumentation / helper overhead: **unmeasured** (`overhead.measured=false`, host `collector_overhead=unmeasured`).  
3. Single sequential pair (N16 then N1); no repeatability, CI, or linearity proof.  
4. `performance_acceptance: false` everywhere checked; pair_eligible false on both bindings.  
5. No p99 simulation/input/decode latency or rendering proof in this evidence.  
6. Logical V8 heap fields do not attribute resident RSS.  
7. Server/helper CPU must not be subtracted as “measured overhead.”

None of those block **writing a design proposal**. They block any claim that the 22.964 MiB slope is a pure per-bot object size, a validated saving, or a finished low-end acceptance.

---

## Method

Independent recompute from archive bytes and raw `samples.jsonl` / `samples.qualification.jsonl`. The author script `diagnostics/current-tui-n1-1727/recompute-n1-n16.py` was read as **reference only** and was **not** executed or imported. Formulas re-derived:

- Observe window: `phase == "observe"`.  
- Median RSS = median(`resident_bytes`) over observe samples.  
- CPU cores = Δ(`process_cpu_user_s` + `process_cpu_system_s`) / Δ`elapsed_s` on first→last observe sample.  
- Client ticks/slot/s = Δ`client_tick_count` / Δ`elapsed_s` / N.  
- Steal gains = (Δ Coins inventory) / 30 with Running/error-free slots.  
- Two-point slope = (metric₁₆ − metric₁) / 15; intercept = metric₁ − slope (extrapolation only).  
- Archive: SHA-256 of `.tar.gz`; every `archive-manifest.json` entry size+SHA verified on extracted tree.

Claimed evidence JSON matched the independent recompute with **zero** core or process-accounting field diffs (float tolerance 1e-9).

---

## Archive and binding integrity

### N1 (`diagnostics/current-tui-n1-1727/`)

| Check | Result |
|---|---|
| Archive SHA-256 | `2fbbc0755a755d623aa9691681eafa6933d7f2e53729083b84d84324761a3a92` — match |
| Manifest payload files | 101/101 size+hash OK (manifest is the payload map; count matches report) |
| Binding | `independent-native-binding.json`: `binding_ok=true`, `qualified=true`, status `bound` |
| Managed resources | `available` |
| Raw gate | `missing_resource_provenance` (raw-reference-metrics.log) |
| Observe samples | 588; all ready=1 and active=1 |
| Qualification | phase span 600.031 s; steals `{live7700_0: 66}`; exit_code 0 |
| originals-verified | `unchanged_after_readers: true` |

### N16 (`diagnostics/current-tui-n16-1616/`)

| Check | Result |
|---|---|
| Archive SHA-256 | `7612191cfe88f61303e1d0409952032a552c53ff87370f90b03fd4d17a2941c4` — match |
| Manifest payload files | 93/93 size+hash OK |
| Binding used for metrics | `binding-replay-82057de/binding.json` (reader `82057de`): `binding_ok=true`, `qualified=true`, status `bound` |
| Archive stub binding | `current-tui-n16-1616-final/independent-native-binding.json` is **unavailable** (`host_conditions_invalid`) — correctly **not** used as the metric binding; report’s use of the replay binding is appropriate |
| Managed resources | `available` |
| Raw gate | `missing_resource_provenance` |
| Observe samples | 580; all ready=16 and active=16 |
| Qualification | phase span 600.043 s; steals 49–82 across 16 slots; exit_code 0 |
| Replay provenance | `raw_artifacts_modified: false`, `new_live_run: false`, original archive SHA matches |

N16 was preserved and not rerun; binding replay is offline. That is consistent with the report.

---

## Independently recomputed ledger (matches evidence JSON)

| Quantity | N1 | N16 |
|---|---:|---:|
| Steady median RSS | 184887296 B (176.322 MiB) | 546076672 B (520.779 MiB) |
| Native lifetime peak RSS | 185495552 B (176.902 MiB) | 627003392 B (597.957 MiB) |
| Sampled peak RSS | 185556992 B | 626692096 B |
| Mean process CPU | 0.048992949 cores (29.347 s / 599.009 s) | 0.561746044 cores (336.436 s / 598.912 s) |
| Client iters/slot/s | 49.668726 | 49.479004 |
| Observe sample count | 588 | 580 |
| Sample bracket | 599.008722 s | 598.911869 s |
| Median V8 used / total | 6097876 / 9437184 | 94923480 / 118226944 |
| Post-stop resident | 171008000 B | 433799168 B |
| Post-stop active / V8 isolates / V8 bytes / inflight snapshots | 0 / 0 / 0 / 0 | 0 / 0 / 0 / 0 |

**Two-point descriptive finite difference (not a causal owner):**

- RSS per added active bot: **22.963802 MiB** (report rounds 22.964)  
- CPU per added bot: **0.03418354 cores**  
- Extrapolated intercept: **153.358 MiB**, **0.014809 cores** — **not** measured zero-bot or idle cost  

Report table values and evidence JSON agree with these recomputes. Steal ranges and server median RSS ~631.957 MiB / ~0.04465 cores on N1 also match.

---

## Provenance and match keys

### Shared runtime / cache / server provenance (identical)

From binding `side_provenance` (N1 == N16) and both run `metadata.json` files:

- Host build commit: `c0709aba2f8b45e42193225cf8f4e7325b5ca9bf`  
- Client commit: `3456edc8dabf7b25ada78110ffa56327af9f67a4`  
- Binary SHA-256: `a0c6eb0bed428fefad58530b177caae16ba2df6c7153544bb0852e16485591f9`  
- Host/client source digests stable; feature `memory-profile-no-alloc`; System allocator; allocation counting off  
- Terminal 120×40, warmup 120 s, observe 600 s, sustain, render_policy none  
- Nav pack / nav flags / catalog SHAs identical  
- Server start identity: `linux_proc_start_ticks:1761`, pid **726** on both managed results  
- Kernel/platform: Linux 6.8.0-139, same boot_id `08c031f9-c44f-42e5-ac32-821bbdec7759`  
- 36 of 39 match keys equal (including server_configuration, cache_settings, sampler contract, terminal, workload=active)

`source_binary_identical: true` in the evidence file is supported by identical binary SHA and side_provenance.

### Expected match-key differences only

Independent set difference is exactly:

1. **`n`** — 1 vs 16 (intentional).  
2. **`renderer_settings`** — per-ordinal `disabled_profile_off` lists of length 1 vs 16 (corresponds to N; not a policy change).  
3. **`host_conditions`** — separately captured wall times and meminfo snapshots (e.g. MemAvailable ~1000228 kB at N1 capture vs ~1049784 kB at N16); same boot/kernel/platform; `collector_overhead: unmeasured` both. Account populations differ because N and capture instant differ.

No unexpected match-key drift was found.

---

## Controller-only N1 extension receipts

N1 used the reviewed controller selection extension (`e707e2d`), not a runtime binary change.

| Receipt | Finding |
|---|---|
| `n1-controller-e707e2d/install-manifest.json` | `runtime_binary_changed: false`; 1113 frozen files verified before/after; only two untracked controller tools changed |
| Tool bytes in archive | `run_current_tui_calibration.py` / `test_current_tui_calibration.py` after-hashes match install `after`; `original-*` match install `before` |
| N16 reader-source copies of those tools | Match install **before** hashes (`addbdd4…` / `6b3908f…`) — N16 path is pre-extension controller tools |
| `native-controller-tests.log` | **24 tests, OK** (includes fail-closed invalid N, N1 CLI/env/spec/receipt retention, default N16 preserved) |
| Managed result | `"n": 1`, `status: completed`, `launched: true`, `performance_acceptance: false`, exit paths clean |

Controller change is bounded to N selection tooling; measurement binary SHA is unchanged across cells. That supports treating N1/N16 as the same runtime binary for resource comparison, with controller/account population differences called out (as the report does).

---

## CPU / RSS arithmetic, progression, cleanup, helpers

- **Arithmetic:** all primary RSS/CPU/tick/slope/intercept figures recomputed; match evidence and report.  
- **Progression:** both cells sustain ready=active=N across the observe window; coin steals positive and multiple-of-30 on every slot.  
- **Cleanup:** single teardown observation each; logical owners cleared (active/V8/inflight = 0); retained RSS 171008000 (N1) and 433799168 (N16). Not a repeated lifecycle plateau — correctly caveated.  
- **Server/helpers:** roles `game_server`, `controller`, `ssh_parent`, `launcher`, `collector` present on both; server pid/start identity shared; medians and CPU intervals recorded separately in evidence. Helper CPU must not be folded into host process overhead.  
- **Order:** N16 started ~1788884228, N1 ~1788888438 (N16 first); non-randomized sequential pair — correctly caveated.  
- **Overhead:** binding `overhead.measured=false` / `helper_overhead_accounting_missing` on both.  
- **Forbidden claims avoided by report:** no idle measurement, no intercept-as-fixed-cost proof, no linearity, no causal allocation owner, no final budget acceptance. Report language is consistent with that discipline.

Minor note: N16 archive’s embedded `independent-native-binding.json` stub is a historical reader failure artifact; metrics correctly come from `binding-replay-82057de`. Reviewers should not treat the stub as the N16 binding.

---

## Plan alignment (§§1, 3, 4A)

- **§1 budgets:** one-bot TUI steady/peak met; 16-bot steady RSS and CPU miss; active incremental miss. Simulation iter floor met. Local server is outside client budgets and is accounted separately — complied.  
- **§3 reference:** 1 and 16 PTY-backed TUI cells exist with frozen provenance, ready/active/progress, and separate server accounting. Raw resource provenance gate still open; overhead still open — so this is a **diagnostic reference pair**, not a closed “trustworthy low-end reference” deliverable.  
- **§4A:** N1 is now a real single-bot TUI control far under the old ~600 MiB one-bot narrative; the **~23 MiB/bot two-point active slope** and N16 misses justify opening **bounded ownership attribution design** (fixed vs per-bot residents, sharing, lifetime, CPU consumers). Evidence does **not** yet identify concrete Rust fields/owners — that is the design/investigation job, not something this pair already proved.

**Recommendation to root:** proceed to a **written bounded per-bot ownership attribution proposal** (no implementation and no live cell authorized by this review). Keep raw-gate and overhead as explicit open items on any later measurement that would claim pure client incremental cost. Do not authorize blanket rewrite, specific optimization landing, or final Grok performance acceptance from this pair alone.

---

## Report quality

Strengths: conservative language; separate server/helper accounting; explicit single-pair and intercept caveats; archive SHAs and payload counts; controller vs binary separation; points to evidence JSON.

No blocking defects found in the claimed numbers or caveats. Residual process notes for root (non-blocking): keep the N16 binding-replay path obvious to future readers; pair_eligible remains false (expected with host_conditions / raw-gate limits).

---

## Final decision

| Question | Answer |
|---|---|
| Do N1/N16 archives and evidence recompute cleanly? | **Yes** |
| Is the report’s uncertainty framing adequate? | **Yes** |
| Final performance acceptance? | **No** (correctly withheld) |
| Supports next **bounded per-bot ownership attribution design**? | **Yes** |
| Supports implementation / live rerun / acceptance gate close? | **No** |

**Review outcome: approved evidence package for design-next; acceptance and causal-owner claims remain blocked as listed above.**
