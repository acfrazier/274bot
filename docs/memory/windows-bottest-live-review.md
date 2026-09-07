# Standard-user Windows panel/TUI live proof — Grok-4.5 review

**Review task:** t_5cceaf2c  
**Proof under review:** `docs/memory/windows-bottest-live-proof.md`  
**Immutable raw evidence:** `docs/memory/diagnostics/windows-bottest-20260907a/`  
**Reviewer profile:** `reviewer` (Grok-4.5)  
**Branch:** `codex/memory-diagnostics` (verified not `main`)  
**Workspace:** `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`  
**Scope:** read-only cold check of the proof against frozen run artifacts. No code/git/remote/UI/live/build mutation; no test re-run. This file is the only write.  
**Out of scope:** sampler binding and modal provenance (other agents); final whole-branch Grok-4.6; managed ConPTY/accounting/input-latency; native Windows game server; matched performance.

---

## Verdict

**APPROVED**

The proof’s panel and TUI N=1 active BotTest session-3 claims match the frozen launch/completion receipts, qualification boundaries, continuous observation samples, diagnostics sidecars, launcher scripts, and cross-user collector pair. Exit 0 / timeout false, BotTest non-admin identity, source+binary hashes, ingame/scene_state=2, renderer distinction, continuous ready/active=1 observation rows, null diagnostic failure/slot error, TUI banking from inventory+requests+paint (not paint alone), and first CLI-failure retention are all evidence-backed. Scope fences (not ConPTY/managed runner, not native Windows server, not matched performance, not high-N banking qual) hold. No overclaim found that blocks acceptance.

---

## Method

1. Confirmed branch `codex/memory-diagnostics` before reading artifacts.
2. Cold-read `windows-bottest-live-proof.md`, then independently parsed:
   - `bottest-panel-20260907a/{launch,completion}.json`, samples + qualification + diagnostics JSONL, stderr/stdout
   - `tui-bundle/bottest-tui-20260907a/` same set
   - `run-bottest-panel-smoke.ps1`, `tui-bundle/run-bottest-tui-smoke.ps1`
   - `cross-user-sampler-8600/` and `cross-user-sampler-8600b/`
   - `artifact-sha256.json`, setup/tui receipts
3. Recomputed observation counts, ready/active continuity, resident ranges, boundary paint/inventory/renderer fields, diagnostics failure/error nulls, Deposit/Withdraw request presence, and SHA-256 of every manifest path.
4. Did not mutate evidence; did not re-run live Windows work.

---

## Checklist vs evidence

### 1. Child exit 0 / timeout false — PASS

| Run | launch pid | completion exitCode | timedOut | start/end UTC |
| --- | --- | --- | --- | --- |
| Panel | 8600 | 0 | false | 2026-09-07T15:23:01.2860357Z → 15:26:52.6071922Z |
| TUI | 20300 | 0 | false | 2026-09-07T15:28:10.1315322Z → 15:31:54.8360390Z |

Matches the proof table exactly.

### 2. Standard BotTest / session 3 / source+binary hash — PASS

Both `launch.json` files:

- `user`: `DESKTOP-SL99R6C\BotTest`
- `sessionId`: 3
- `isAdministrator`: false
- `processHandleRetained`: true
- `diagnosticOnly`: true
- `acceptedPerformance`: false
- `serverPlacement`: `Mac via loopback-only SSH reverse forwards`

| | hostCommit | clientCommit | binarySha256 (launch + launcher gate) |
| --- | --- | --- | --- |
| Panel | 61b7b7d | 4b35300 | `0FC3F79B…C2A6A424` |
| TUI | 952ba22 | 2b1af85 | `5ADDCB8B…F29CDE17` |

Proof correctly lists **distinct** panel vs TUI builds (not one shared binary). Launchers refuse SHA mismatch before start. `tui-receipt.json` records TUI path `bin-952ba22\tui-play.exe` and feature `memory-profile-no-alloc`. Panel runtime samples all have `allocation_counting: false` and null rust allocation counters (consistent with no-alloc build); panel cargo-feature string is explicit in TUI receipt only, not a separate panel feature receipt in this tree — non-blocking given binary hash + runtime flags.

### 3. Qualifications: ingame + scene_state 2 — PASS

Both observe-start and observe-end slots:

- Panel: `client.ingame=true`, `scene_state=2`; renderer `backend=gpu`, `renderer_present=true`, `draw=true`, `full_rate=true`
- TUI: `client.ingame=true`, `scene_state=2`; renderer `renderer_present=false`, `backend=null`, `draw=false`, `full_rate=false`

Observation-phase sample `renderer_profile` rows (117 panel / 118 TUI) are unanimous on the same distinction; every observe row also has profile `ingame=true`, `scene_state=2`.

### 4. Observation boundaries, continuous active rows, ticks/steals/eats/bank — PASS

| | Panel | TUI |
| --- | --- | --- |
| Qual boundaries (elapsed_s) | 47.0683838 → 167.0993882 | 44.4326139 → 164.4575371 |
| Observe sample rows | 117 | 118 |
| ready/active during observe | (1,1) on **all** rows | (1,1) on **all** rows |
| Observe elapsed spacing | ~1.00–1.04s; 0 gaps >2.5s | ~1.00–1.05s; 0 gaps >2.5s |
| Steals (paint) | 1 → 8 | 3 → 15 |
| Ate | 0 → 2 | 0 → 1 |
| Bank trips (paint) | 0 → 0 | 0 → 1 |
| last_completed_tick | 70 → 270 | 69 → 269 |
| resident_bytes observe min–max | 481275904–484151296 | 179339264–185384960 |

Proof table numbers match. Continuous active observation is real (not a single boundary snapshot).

### 5. Diagnostics: null failure / null slot error — PASS

- Panel: 223 diagnostic rows; `failure is null` and every slot `error is null`
- TUI: 221 diagnostic rows; same
- Samples: `diagnostic_sidecar=true`, `failure_capture=true` throughout
- Qual settings: scheduling / GPU-completion / responsiveness profiles **false**; `BOT_RENDER_PROFILE` true — matches proof

### 6. Bounded TUI banking from inventory + requests + paint — PASS (not paint-only)

At TUI observe-end:

- Paint: `Bank trips: 1`, `Steals: 15`, `Food: 22`, state Running / pickpocket near guards
- Actual inventory: **22** separate Lobster entries + Coins count **30**
- `bank_open=false`, `bank_loaded=false`, `bank=[]` (closed/unloaded as claimed)
- Diagnostics `recent_requests` contain exact strings:
  - `Deposit { name: "Coins" }`
  - `Withdraw { name: "Lobster", action: "Withdraw X" }`
- Also OpenBooth / WalkNear / Close / AnswerCount traffic consistent with a restock cycle; logs include food restock walk to Bank booth and “carrying 22 'lobster' food”

Panel had **no** Deposit/Withdraw in diagnostics (bank trips stayed 0) — proof does not claim panel banking. Scope text correctly rejects long-run/high-N banking qualification.

### 7. Cross-user collector capability + initial CLI failure retention — PASS

**8600 (first attempt, retained failure):**

- `receipt.json`: collector source `e425799`, Austen → BotTest pid 8600 session 3, `exitCode: 2`, purpose cross-user capability only, `acceptedPerformance: false`
- `cli.log`: positional CLI rejected `--pid` / `--output` (`unrecognized arguments: --pid --output`); usage shows `pid output`

**8600b (corrected, separate dir — failure not overwritten):**

- receipt `exitCode: 0`, same source/user/pid/session/purpose
- `samples.jsonl`: metadata + **10** sample rows + summary `status: ok`, `sample_count: 10`, duration 2.0s / interval 0.2s
- Stable `start_identity` = `windows_creation_filetime:134332681812860357` on all 10 samples
- `resident_bytes` = 481406976 (nonzero) every sample; provenance documents WorkingSetSize via GetProcessMemoryInfo and GetProcessTimes CPU
- Summary notes sampling overhead unmeasured — no overhead qualification claim

Proof’s capability fence (this token/process only; not six-role managed accounting; not universal cross-user permission) matches receipts.

### 8. Launcher ownership / timeout / Handle / TUI console path — PASS

Both scripts:

- Require `$env:USERNAME -ne 'BotTest'` throw
- Stage under `C:\ProgramData\274bot-Test`; runs only under `C:\Users\BotTest\274bot-runs\$RunName`; refuse overwrite
- Capture `$handle=$p.Handle` before wait; `processHandleRetained=($handle -ne [IntPtr]::Zero)` in launch.json
- `WaitForExit(480000)` (eight-minute backstop); on timeout CloseMainWindow then Kill; require non-null ExitCode
- Write completion with actual exitCode + timedOut

Panel: `Start-Process … -WindowStyle Normal -RedirectStandardOutput … -RedirectStandardError …` (visible normal window).  
TUI: `Start-Process … -NoNewWindow -RedirectStandardError …` **without** stdout redirect — inherits real console; stderr to file.  
Neither path is a Unix-PTY managed runner; no ConPTY construction; no input-latency measurement. Proof’s “not managed ConPTY/accounting/input-latency” fence is accurate.

### 9. Server placement / performance fences — PASS

- Launch `serverPlacement` and proof body: Mac game server via loopback-only SSH reverse forwards on Windows ports 80/43594; **no** native Windows `game_server` PID claimed
- Qual settings `host=127.0.0.1` `port=43594`
- `acceptedPerformance: false` on launches and cross-user receipts
- Resident ranges explicitly not baseline/candidate savings
- Remaining native suite failures and managed-runner work deferred to other docs — no full-Windows-green claim

### 10. Artifact integrity — PASS (extracted tree)

- `artifact-sha256.json`: **24/24** listed paths present; **0** SHA-256 mismatches on re-hash
- Setup receipts `enabled: false` are creation-time snapshots; proof correctly points identity to `launch.json` instead

**Non-blocking caveat:** transfer-archive SHA-256 values in the proof (`5fcd3ab8…` panel+collector, `b7bae58e…` TUI) are **not** accompanied by the archive blob bytes inside this evidence directory, so those two outer hashes are not re-computable here. Extracted content is fully covered by `artifact-sha256.json`. Do not treat missing archive blobs as disproof of the runs.

---

## Overclaim / negative checks

| Risk | Result |
| --- | --- |
| Managed ConPTY / PTY runner proof | Not claimed; scripts are direct Start-Process diagnostic launchers |
| Input-latency measurement | Not present / not claimed |
| Native Windows game server | Explicitly denied; Mac-via-SSH only |
| Matched performance / savings | Explicitly false / ranges not compared as savings |
| High-N or long-run banking qualification | Explicitly bounded to one observed TUI cycle |
| Cross-user universal permission / six-role accounting | Explicitly limited to OpenProcess on pid 8600 for tested accounts |
| Quiet-machine claim | Denied up front |
| Whole-branch Grok-4.6 / full Windows green | Explicitly not claimed |

No contradictory numbers found between proof table and frozen JSON/JSONL.

---

## Findings summary

1. **No blocking defects.** Proof is faithful to immutable evidence for all task-required checks.
2. **Non-blocking:** outer transfer-archive SHAs not re-hashable without archive files in-tree; extracted file set is hash-complete.
3. **Non-blocking:** panel `memory-profile-no-alloc` is runtime/hash-supported; explicit feature receipt string in this tree is on the TUI receipt.

---

## Approval boundary

This approval is **only** for `windows-bottest-live-proof.md` as a standard-user Windows panel+TUI live diagnostic evidence write-up against `diagnostics/windows-bottest-20260907a`. It does **not** approve managed-runner readiness, sampler-binding work, modal provenance, native full-suite green, or final branch merge (Grok-4.6).
