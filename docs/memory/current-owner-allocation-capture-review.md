# Current N1 allocation capture — independent evidence review

Bounded evidence review of root report `eba3f97` (failed raw-size-guard capture with
usable common-time global-peak analysis). Not whole-branch review and not campaign
acceptance. No code changes, live/replay capture, remote work, STATE edits, or
Windows/Hyper-V operations in this review.

## Identity

| Field | Value |
| --- | --- |
| Reviewer profile | `reviewer` |
| Actual model / provider | `grok-4.5` / `xai-oauth` (no task model/provider overrides) |
| Branch | `codex/memory-diagnostics` |
| Reviewed report commit | `eba3f976937a50c16eeec5f9028810b3220271dc` |
| Checkout HEAD at review | `0bc21513b7011c36d8ee9b089a8744e4fce01311` (descendant; result.md/.json byte-identical to `eba3f97`) |
| Approved capture plan | `e8c3c8072ef580b2b2e33b448189dd998572dde0` |
| Frozen host / client | `c0709aba2f8b45e42193225cf8f4e7325b5ca9bf` / `3456edc8dabf7b25ada78110ffa56327af9f67a4` |
| Reviewed tooling | `1fe21610b6446779133b5ff0dc229ba93599ecc0` |
| Capture id | `current-owner-heaptrack-n1-2003` |
| Local evidence | `diagnostics/owner-capture-evidence-2015/` |
| UTC | 2026-09-08T20:33:52Z |
| Kanban task | `t_9d3ae118` |
| Review round / lens | 1 / artifact (cold evidence + independent recompute) |

## Verdict

**APPROVED** as a failed-bound N1 allocation capture with honest incomplete
qualification and a usable **startup** common-time global-peak population only.

Accepted conclusions:

- The sole game capture failed the reviewed 2 GiB raw-output guard and does **not**
  qualify the full 600 s observe / Stop / post-join procedure.
- No performance acceptance, RSS savings, private per-bot owner ranking, or
  replacement capture/optimization is established.
- No ownership candidate is selected from this peak.
- Offline analysis of the incomplete trace succeeded under the reviewed analysis
  bounds and identifies a navigation `load_pack` startup peak (file buffer +
  decoded walk/blocked coexistence), not a retained active per-bot population.
- Existing-data replay capability remains a separate pending audit (`t_227f43d3`);
  this report does not claim that capability.

## What was inspected

- `docs/memory/current-owner-allocation-capture-result.md`
- `docs/memory/current-owner-allocation-capture-result.json`
- Plan `docs/memory/current-owner-allocation-capture-plan.md` at `e8c3c80`
- Evidence pack `diagnostics/owner-capture-evidence-2015/` (manifest, peak stacks,
  peak-analysis log, metadata, qualification, receipts, smoke, tooling receipts,
  launch logs, process accounting, recompute script/output)
- Frozen source at `c0709ab`: `crates/nav/src/world.rs:49-55`,
  `crates/nav/src/pack.rs:338-350`, `crates/host-play/src/lib.rs:3181`
- Concord anchor `diagnostics/concord-post-updates-1823/post-updates-1823/receipt.json`
- Local independent recompute scratch:
  `docs/memory/_review_owner_capture_r1_recompute.py`,
  `docs/memory/_review_owner_capture_r1_results.json`

## Independent checks

### Evidence integrity

| Check | Result |
| --- | --- |
| Manifest records | 48 total: **46 included**, **2 remote-only** (`alloc.raw`, `alloc.interpreted`) |
| Included file SHA-256 + byte length | **46/46 match** local files |
| Export archive SHA `1a73be129f22b2e78a2dc220b8305709aff83e4fe4c046050bd924e95412a9a1` | **Claim retained, not re-hashed here** — no local tarball present; integrity rests on the included-file hash set above |
| Result.md/.json vs `eba3f97` | byte-identical on this checkout |
| Recompute script SHA | `124a3726e4ff91459fa1ca2ed6d5b8463801f7ed0d62cd0eea9e1a00fb66a457` matches result JSON field |

### Capture failure and qualification

| Claim | Evidence | Verdict |
| --- | --- | --- |
| Guard reason `raw output 2149010072 >= 2147483648` | `metadata.json` heaptrack.guard | **accepted** |
| Frontend exit -15 / launcher exit 1 / root controller exit 1 | metadata exit -15; result/receipt launcher 1; completion.json exit 1; launch log "Capture completed exit 1" | **accepted** |
| Frontend PID 2096 / start `603986`; launcher 2083; controller 2066; server 726/start 595 | result.json role_identities + metadata pid | **accepted** |
| Only `observe-start` qualification | `samples.qualification.jsonl` single row | **accepted** |
| Observe-start: Running, ingame, scene_state 2, Steals 26 | qualification slot paint/client | **accepted** |
| ~455.524 s remain after observe-start until frontend wait return | `ended_unix - observe_start.after_unix` = 455.52433156967163 | **accepted** |
| No observe-end / script-Stop / after-Stop / join population | qualification phases + binding_errors | **accepted** |
| Qualifier exit 1 / qualified=false with missing boundary, observation incomplete, process failed | qualifier-exit/output JSON | **accepted** |
| Cleanup: collector 0, frontend orphan_risk false | result.json cleanup | **accepted** |
| Guard peak frontend RSS 195,039,232 < 512 MiB | guard.peak_rss_bytes; printer also reports peak RSS 195.33M | **accepted** (profiled total RSS, not overhead) |
| Frontend metadata wall 590.664 s vs printer trace clock 589.61 s | different clocks; report states non-interchangeable | **accepted** |

### Printer flags and analysis bounds

| Claim | Evidence | Verdict |
| --- | --- | --- |
| Exact argv `--merge-backtraces=0 --flamegraph-cost-type=peak` | metadata heaptrack.analysis.printer.argv | **accepted** |
| Interpreter exit 0 in 58.556 s, RSS 114,855,936 | metadata | **accepted** (within 180 s / 512 MiB) |
| Printer exit 0 in 13.515 s, RSS 58,503,168 | metadata | **accepted** |
| Preload Heaptrack 1.5.0 SHA `134760db...` | metadata + plan smoke path | **accepted** as recorded |
| Raw size/hash remote-only | manifest included=false; size/hash recorded | **accepted** as remote retention claim |

### Global peak recompute (independent)

From local `allocation/peak-stacks.txt` (SHA
`92b33eac544a6b3f082058fcc1d4c23655f2eea457b1578d0a3f47e03c51ce6e`, 38,699,685 bytes):

| Metric | Independent | Report |
| --- | --- | --- |
| All rows | 14903 | 14903 |
| Positive rows | 194 | 194 |
| Negative rows | 0 | 0 |
| Malformed rows | 0 | 0 |
| Positive-cost sum | **160,842,811** | 160,842,811 |
| Printer line `peak heap memory consumption` | **160.84M** | agrees with rounded sum |
| NavWorld::load_pack subtree | **147,272,041** (91.5627%) | 147,272,041 (91.563%) |
| IfType::unpack | 6,491,160 | 6,491,160 |
| Cache::unpack | 4,783,967 | 4,783,967 |
| load_templates | 1,933,184 | 1,933,184 |
| Other | 362,459 | 362,459 |
| Partition sum | 160,842,811 | equals total |

Top rows (row, bytes): 12243/73,438,581 (fs::read under load_pack);
10154/65,142,784 (pack::decode walk); 10155/8,142,848 (pack::decode blocked).
Exact match to report top_rows and nav size claims.

This is one common-time peak population under `--merge-backtraces=0
--flamegraph-cost-type=peak`, not a sum of independent stack peaks.

### Nav header / source mapping

Claimed header hex `32373456084007000000050000000000000007000080230000` decodes at
offset 17 as width **1792**, height **9088**.

| Derived size | Value | Matches peak row |
| --- | --- | --- |
| 4 × w × h walk cells | 65,142,784 | row 10154 |
| ((cells+63)//64)×8 blocked words | 8,142,848 | row 10155 |
| File buffer claim | 73,438,581 | row 12243 |

Frozen source at `c0709ab` (also current tree):

- `nav/world.rs:49-55`: `std::fs::read` then `decode(&bytes)` — temporary file
  buffer coexists with decode outputs until `load_pack` returns.
- `nav/pack.rs:338-350`: `walk = vec![0u8; cells]`; blocked words via
  `cells.div_ceil(64)` and `read_u64` loop — sizes match; blocked **line** is
  size/context inference, not proven inline debug (report states this).
- `host-play/lib.rs:3181`: `world: NavWorld::load_pack(...).ok().map(Arc::new)` —
  single shared Arc load in `Play::new`, not a per-bot private owner ranking.

Lifetime: startup overlap + shared navigation storage. **Cannot** rank retained
Client/World/snapshot/script owners per bot or justify a startup optimization
substitute. Report correctly refuses that jump.

### Tooling / smoke / tests / identity

| Claim | Evidence | Verdict |
| --- | --- | --- |
| Tooling stage 85 files, archive SHA `80715886...`, commit `1fe2161` | tooling-install-receipt.json | **accepted** as staged receipt |
| Frozen original files 1113 unchanged; no Rust rebuild | frozen-source-post-overlay.json; tooling receipt flags | **accepted** as recorded |
| Native tests 101 run, 2 skips, OK | native-affected-tests.log `Ran 101 tests` / `OK (skipped=2)` | **accepted** |
| Owned smoke `heaptrack-seam-smoke-2001` passed | receipt passed=true; parent limits/env unchanged | **accepted** |
| Smoke common-time peak sum 5,730,024; bytearray family 4,196,352 | independent smoke peak-stacks recompute | **accepted** (report text) |
| Binary SHA `a0c6eb0b...`; navpack SHA `2f393138...`; cache provenance SHA `d2b65fdd...` | metadata/receipt | **accepted** as recorded |
| 2002 preflight failed attempts=0 wrong server-identity path; not a second game capture | owner-capture-launch-2002.log | **accepted** |
| Live boot assert `2217ec26-...`; server 726/start 595 | run-owner-capture.py asserts; server-identity; concord receipt | **accepted** |

### Forbidden / over-claim scan

Report **does not** claim: completed 600 s observe, observe-end population,
script-Stop/post-join owner set, active private owner ranking, RSS savings,
performance acceptance, selected candidate, or substitute startup optimization.
Mentions of those phrases are negations only.

## Material limitations (accepted, not blocking)

1. **Incomplete procedure by design of the failure:** raw 2 GiB soft-stop fired;
   observation unfinished; no Stop/join allocation population.
2. **Peak semantics:** common-time global peak is dominated by startup
   `NavWorld::load_pack` coexistence (file bytes + walk + blocked). Not an
   active steady-state per-bot owner census.
3. **No phase slice / private ranking / RSS causality / overhead / savings.**
4. **Raw + interpreted remain remote-only** (~2.15 GiB + ~0.81 GiB); local review
   trusts recorded hashes and the downloaded peak/receipt set.
5. **Export archive SHA** stated in the report was **not re-hashed** here (tarball
   absent locally). Included 46-file hash set verified instead.
6. **Nav header hex** is a root post-capture probe claim; local verification is
   structural (decode math + size match to peak rows), not a second remote
   navpack read.
7. **`host-conditions.json` template residue:** `boot_id` still shows prior boot
   `08c031f9-...` and `purpose` still says “N16 allocation-unprofiled…”. Live
   launch script asserted boot `2217ec26-...` and N1 argv; server-identity and
   concord receipt match the post-update boot. The result report does not claim
   the stale host-conditions boot_id string. Treat host-conditions prose fields
   as partially stale template carry-over, not as the live identity authority.
8. **Controller MemAvailable guard** (128 MiB floor in managed path) did not
   trigger; admission MemAvailable in host-conditions sample was ~910 MiB
   (≥768 MiB plan gate) at preflight sample time.
9. **Separate pending work:** offline replay/capability audit `t_227f43d3` only;
   matched campaign gates and final branch review remain open.

## Claims accepted vs rejected

| Claim class | Decision |
| --- | --- |
| Failed raw-size guard; unqualified incomplete observe | **accept** |
| Analysis success; exact merge0/peak flags; row/sum/partition numbers | **accept** |
| Startup nav load_pack peak + size/source mapping + Arc shared world | **accept** |
| No candidate / no savings / no full lifecycle owner answer | **accept** |
| Tooling 1fe2161 + frozen c0709ab/client3456 + smoke/tests as staged | **accept** |
| Full 600 observe / Stop / post-join / private owner / RSS savings / startup opt release | **reject if asserted** (report does not assert them) |
| Export archive SHA as locally re-verified in this review | **not accepted as re-hashed**; accepted as remote export claim backed by included-file hashes |
| host-conditions boot_id/purpose strings as live truth | **reject as authority**; live asserts + server-identity win |

## Reviewer actions

- Wrote this review only: `docs/memory/current-owner-allocation-capture-review.md`
- Local read-only recompute scratch under `docs/memory/_review_owner_capture_r1_*`
- Did **not** edit result report, STATE, plan, product code, or create cards
- No remote/game/profile/Windows work

## Bottom line

Approve the report’s bounded failure + startup-peak evidence package. The
discriminator for retained per-bot owners remains unresolved; any next procedure
needs a new reviewed card. Replay of saved data stays pending on `t_227f43d3`
only.
