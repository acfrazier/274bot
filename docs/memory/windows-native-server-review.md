# Native Windows local server and TUI proof — Grok-4.5 review

**Review task:** t_ff5e7115  
**Proof under review:** `docs/memory/windows-native-server-proof.md`  
**Immutable raw evidence:** `docs/memory/diagnostics/windows-native-server-20260907a/`  
**Reviewer profile:** `reviewer` (Grok-4.5)  
**Branch:** `codex/memory-diagnostics` (verified not `main`)  
**Workspace:** `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`  
**Scope:** read-only cold check of the proof against frozen bundle-b / smoke-summary / artifact hashes. No code/git/remote/UI/live/build mutation. This file is the only intentional write.  
**Out of scope:** managed stop/cleanup ownership native validation; ConPTY/input probes; matched resource/latency; panel modal client proof (binary predates that work); final whole-branch Grok-4.6 / `branchreviewer`.

---

## Verdict

**APPROVED**

The proof’s native Windows isolated-server + BotTest session-3 TUI functional smoke claims match the frozen receipts and samples. Corrected frontend exit 0 / timeout false, 7322 static NPCs, loopback PID 24224 ownership of ports 80/43594/8898, preserved wordenc / missing-map-content / BOM failures, 120s continuous observe rows with inventory+request banking (not paint-only), and a short three-role fixed-grid 15-sample ~2.795s capability probe without full-coverage or savings claims are all evidence-backed. Scope fences (not managed cell, not matched comparison, not performance acceptance, not full Windows readiness, not modal-client validation on this frozen binary) hold. No blocking overclaim found.

---

## Method

1. Confirmed branch `codex/memory-diagnostics` before reading artifacts.
2. Cold-read `windows-native-server-proof.md`, then independently parsed:
   - `artifact-sha256.json` (re-hash every listed path)
   - `smoke-summary.json`
   - `bundle-b/` server launch/stdout/stderr, fixture verification, node/dep/init receipts, addition receipts, startup-failure-01, world-without-map-content-02, config-bom-failure-03
   - `bundle-b/bottest-native-tui-20260907{a,b}/` launch/completion/samples/qualification/diagnostics
   - `bundle-b/native-three-role-20260907b/` receipt + samples
   - setup scripts `verify-native-fixture*.ps1`, launchers, three-role prover
3. Recomputed observation counts, ready/active/scene/renderer continuity, bank inventory+request timeline at claimed timestamps, three-role grid/span/identities, and launch hash bindings to config/maps/wordenc/public key/manifest.
4. Read-only Mac Lost City source (`/Users/acfrazier/experiments/Server`) for setup path / `GameMap.init` / `WordEnc.load` consistency with failure diagnoses. Did not alter user server or any checkout.
5. Did not re-run live Windows work.

---

## Checklist vs evidence

### 1. Artifact integrity — PASS (extracted tree)

- `artifact-sha256.json`: **71/71** listed paths present; **0** SHA-256 mismatches on independent re-hash.
- Self-hash of `smoke-summary.json` matches its manifest entry  
  (`2b7045bca4ead57e978207ab5f63295b76a9ccaf306acc9169e8bfa08419c082`).
- Private key material absent from the archive (`public.pem` only; `privateKeyContentsExcluded: true` on fixture receipt; no `*private*` paths).

**Non-blocking:** transfer-archive SHA-256 `742ca721…` is stated in the proof but the archive blob is not in this tree, so that outer hash is not re-computable here. Extracted content is fully covered by `artifact-sha256.json`.

### 2. BotTest / session 3 / frontend exit 0 / timeout false — PASS

Corrected run `bottest-native-tui-20260907b`:

| Field | Evidence |
| --- | --- |
| user | `DESKTOP-SL99R6C\BotTest` |
| sessionId | 3 |
| isAdministrator | false |
| pid | 15380 |
| startTimeUtc | `2026-09-07T15:55:09.6582864Z` |
| exitCode | 0 |
| timedOut | false |
| endedUtc | `2026-09-07T15:59:03.5842508Z` |
| processHandleRetained | true |
| hostCommit / clientCommit | `952ba22` / `2b1af85` |
| binarySha256 | `5ADDCB8B…F29CDE17` (launch + launcher gate) |
| diagnosticOnly / acceptedPerformance | true / false |
| serverPlacement | `native Windows isolated fixture` |
| serverPid | 24224 |

Launcher `run-bottest-native-tui-smoke-b.ps1`: BotTest-only gate; `Start-Process -NoNewWindow` with stderr redirect only (no stdout redirect → real console inherit); captures `$p.Handle` before wait; 480s backstop; refuses overwrite. Matches “real process handle / real console path” without ConPTY/managed runner claims.

### 3. Native server PID / 7322 NPCs / loopback listeners — PASS

- `server-launch.json`: pid **24224**, session 3, BotTest, start `2026-09-07T15:54:06.0815336Z`, `loopbackOnly: true`, directory `C:\Users\BotTest\274bot-server-4c95f87`, placement native isolated fixture.
- `server.stdout.log`: `7322/16383 static NPCs added` and `World ready`.
- `server.stderr.log`: empty (0 bytes).
- `fixture-live-verification.json` listeners all `127.0.0.1` ports **80, 43594, 8898** with `OwningProcess: 24224`.

SSH reverse-forward tunnel PID 686 stop is operator narrative not re-proven from this bundle (non-blocking; Mac server/supervisor non-touch claim is outside this fixture tree).

### 4. Isolated fixture boundaries (source / addition / config / public key) — PASS

Corrected TUI launch binds, and independent re-hash confirms:

| Binding | Launch field | Matches on-disk |
| --- | --- | --- |
| Fixture manifest | `serverFixtureSha256` | `windows-fixture-manifest.json` |
| Runtime world.json | `serverConfigSha256` | `data/config/world.json` → `828952B1…9844B4` |
| Maps addition receipt | `serverMapAdditionSha256` | `native-server-maps-addition.json` |
| Wordenc addition receipt | `serverWordencAdditionSha256` | `native-server-wordenc-addition.json` |
| Public key | `serverPublicKeySha256` | `data/config/public.pem` |

- Init/dep: fresh local fixture (`initialize-receipt` exit 0, `serverStarted: false`); source commit `4c95f87efe00b068cadbd229d94736626907bd1a`; `npm ci --ignore-scripts` exit 0 (“added 296 packages”).
- Node: portable `v24.19.0`, OpenJS Authenticode `signatureStatus: Valid`, exe SHA-256 in receipts  
  `3602F2BB1A10F2CBAB4C36886218A33C1AB3DB87290E73B033C46C77147D0237`  
  (also on `server-launch.json` / three-role server block).
- Runtime world `build.srcDir` = `runtime-content` (corrected); maps addition lists `multiway.csv` + `free2play.csv` only; wordenc path `data/raw/wordenc`.
- Fixture live check: **683** original staged files, **0** unexpected mismatches; `configChangeSeparatelyBound: true`; maps/wordenc verified; acceptedPerformance false.

**Root hash / immutability wording — PASS (appropriately bounded):**  
`verify-native-fixture.ps1` uses ordinary `Get-FileHash`; `verify-native-fixture-shared.ps1` uses `FileShare.ReadWrite` stream hash for in-use files. Successful `fixture-live-verification.json` matches the shared-script field shape (683 / empty mismatches / separately bound config). Proof correctly frames this as a live hash check, not atomic freeze of mutable files during reads. Ordinary Get-FileHash failure itself is not retained as a separate failure receipt (narrative + scripts only) — non-blocking given successful shared verification artifact.

### 5. Preserved setup failures (not reclassified) — PASS

1. **startup-failure-01:** server completion exit **1**; stderr `ENOENT` open `data\raw\wordenc` via `WordEnc.load` / `Jagfile.load`. Wordenc addition receipt preserves reason and hash. Mac source `WordEnc.load` opens `data/raw/wordenc` — consistent.
2. **bottest-native-tui-20260907a + world-without-map-content-02:** TUI exit **1**, timedOut **false**; failure `seed step 6 … stat_xp_gain(17)>=1 not seen within 150 ticks`; paint steals **0**; **0** diagnostic `recent_requests`; **0** guards on last diag while still ingame/scene_state=2. Forced stop receipt for server after failed seed. W02 world still had `srcDir: '../content'`; corrected runtime uses `runtime-content`. Mac `GameMap.init` returns early if ``${srcDir}/maps`` missing before packed `maps-server.zip` load — consistent with “packed cache present, content path missing.”
3. **config-bom-failure-03:** preserved `world.json` begins with UTF-8 BOM `EF BB BF`; stderr Node JSON parse failure “Using defaults”; completion exit **-1**. Corrected `bundle-b/data/config/world.json` starts with `{` (no BOM). No claim that a frontend ran against the BOM config.

Failed runs remain separate directories with failing exits; not folded into performance averages.

### 6. 120s qualified functional observe + banking inventory/requests — PASS

| Claim | Independent recompute |
| --- | --- |
| Observe boundaries | quals `observe-start` 53.6836893 → `observe-end` 173.7097243; duration 120.026035s |
| Sample rows in bounds | **117** samples; all `ready=1` `active=1` |
| Diagnostic rows in bounds | **117**; `failure is null` on all; slot `error is null` on all |
| Ingame / scene_state=2 | **117/117** diag client fields |
| Renderer | samples `renderer_profile[].renderer_present=false` throughout; no GPU claim |
| Boundary paint steals/ate/bank | start 3/0/0 → end 11/1/1 (quals + first/last observe diags) |

Banking at exact claimed timestamps (from diagnostics slot runtime + recent_requests):

| elapsed_s | bank_open | food (Lobster rows) | coins | Requests include |
| --- | --- | --- | --- | --- |
| 113.4196448 | true | 3 | 0 | Deposit Coins, Withdraw Lobster/Withdraw X, Eat, OpenBooth |
| 114.4389716 | true | 22 | 0 | + AnswerCount 19, Close |
| 115.4605944 | false | 22 | 0 | same banking set retained in recent_requests |

Post-return coin/steal progression from inventory+paint (not paint alone): coins 30→60→90→120→150 with steals 7→11 and bank_trips=1 after return to guards. Matches proof and `smoke-summary.bankEvidence`.

### 7. Short three-role collector capability only — PASS

- Receipt: collector source `e425799`, user `DESKTOP-SL99R6C\Austen`, frontend pid 15380 / server 24224, exitCode **0**, purpose fixed-duration native three-role capability, `acceptedPerformance: false`.
- Samples: metadata + **15** `type=sample` + summary; all 15 samples status `ok` for collector/frontend/game_server; stable Win32 `windows_creation_filetime:…` identities; current `resident_bytes` and CPU counters present each role each sample.
- Summary / smoke collectorSummary: `status=ok`, `ok_sample_count=15`, `required_grid_sample_count=15`, `configured_duration_s=3.0`, `observed_covered_span_s≈2.795`, **`full_duration_claim: false`** for every role, scope_gaps list descendants/perturbation/etc., sampling_overhead unmeasured.
- Proof correctly denies graceful managed stop, complete frontend observation coverage, six-role enclosure, matched savings.

Collector self pid **14136** matches metadata roles and smoke role_lifetimes.

### 8. Lost City setup distinction (read-only source) — PASS

Mac source (not modified):

- `Server/start.js`: runs `npm run setup` only when neither `engine/.env` nor `engine/data/config/world.json` exists.
- `engine/src/setup.ts`: setup UI + conditional support-server migration — not a substitute for the frozen runtime fixture used here.
- Fixture already supplied world config + local schema; map CSV content path was a separate runtime prerequisite that normal startup packing checks did not establish — consistent with preserved seed failure and maps addition.

Proof’s “frozen runtime fixture, not complete Lost City install replacement” fence is accurate.

### 9. Scope fences / no full Windows readiness / no modal client proof — PASS

Proof and receipts repeatedly set `acceptedPerformance: false`, `functionalOnly` / diagnostic-only, and list remaining managed stop, ConPTY/input, matched builds, resource/latency, panel lifecycle, and final Grok-4.6 work as incomplete. Frozen frontend commits `952ba22`/`2b1af85` with binary `5ADDCB8B…` — same TUI pin as earlier BotTest live proof; proof states binary predates main-modal restoration and cannot validate that client change. No complete observation/managed/matched savings language attached to the three-role probe or the smoke.

---

## Overclaim / negative checks

| Risk | Result |
| --- | --- |
| Managed cell / matched comparison / resource-budget pass | Explicitly denied up front and in receipts |
| Performance / savings acceptance | `acceptedPerformance: false` everywhere checked |
| Full configured-duration collector coverage | `full_duration_claim: false`; ~2.795s of 3.0s only |
| Six-role / descendant / perturbation accounting | Listed in collector scope_gaps; not claimed |
| Reclassifying failed runs as pass | Failures kept in separate dirs with failing exits |
| Atomic immutability of live fixture files | Proof and shared-hash script avoid this; 683 check is live verification |
| Modal / GPU client restoration proof | Explicitly out of scope; renderer absent on TUI |
| Full Windows readiness / branchreviewer | Explicitly remaining; this is not final branch review |
| Private key exfiltration in evidence | Public key only |

---

## Findings summary

1. **No blocking defects.** Functional topology/smoke claims match immutable `windows-native-server-20260907a` evidence for all task-required checks.
2. **Non-blocking editorial:** proof prose Node exe SHA-256 is **63** hex chars and omits a `1` (`…f2bba10f…` vs receipt `…F2BB1A10F…`). Authoritative pins remain `node-receipt.json`, `server-launch.json`, and three-role server block:  
   `3602F2BB1A10F2CBAB4C36886218A33C1AB3DB87290E73B033C46C77147D0237`.  
   Fix the proof string if it will be copy-pasted as a pin; do not treat the typo as disproof of the Node binding.
3. **Non-blocking:** outer transfer-archive SHA not re-hashable without the archive blob in-tree; extracted set is hash-complete.
4. **Non-blocking:** ordinary Get-FileHash in-use failure is script-supported narrative without a separate failed receipt file; shared-read success receipt is present (683 / 0 unexpected).

---

## Approval boundary

This approval is **only** for `windows-native-server-proof.md` as a native Windows isolated Lost City server + standard-user TUI functional smoke evidence write-up against `diagnostics/windows-native-server-20260907a`. It does **not** approve managed-runner stop/ownership native validation, ConPTY/input latency, matched resource/latency gates, panel/modal client changes on a later binary, full Windows suite green, or final branch merge (`branchreviewer` / Grok-4.6).
