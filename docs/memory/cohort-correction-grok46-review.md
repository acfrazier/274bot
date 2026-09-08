# Cohort correction and native input evidence (Grok 4.6)

Bounded follow-up to `cohort-integration-grok46-review.md`. Not a per-task
re-review and not whole-campaign approval.

## Identity

| Field | Value |
|---|---|
| Reviewer profile | `branchreviewer` |
| Actual model / provider | `grok-4.6` / `xai-oauth` (no task overrides) |
| Branch | `codex/memory-diagnostics` |
| Checkout HEAD | `ef56d3b7b8d2bf0786a43018289b29abe129e1e9` (STATE tracking only; crate blobs match `142f7c3`) |
| Frozen host cache | `142f7c3a99eb197530481504f306b459bc2d8eb3` |
| Prior integration source | `f9d729ce6e986a9effeac3134e262eeeb9209cb7` |
| Cache commit parent (primary) | `86237c31b5cbdb8dad0d782a6487568dc4960146` |
| Native reference (cache-applied) | `e25f32806957b1a44c75a598cbdb7ab53afc5383` (parent `8d3bd4ff36c28485b90df840da1623705ab66676`) |
| Native candidate (cache-applied) | `ca56e14371d1edb8c09df1276638ce196d502e36` (parent `e3a2cbfbdfdab37a4f35ee330e91a7e6264c14d7`) |
| Native parents | reference `9268890217d968cfeb7c66ebb11dd5c3dd2c084f` / candidate `fb3589ac28583242b999ac864ea69c4ef8fa5923` |
| Functional-proof binary (pre-cache) | host `e3a2cbfbdfdab37a4f35ee330e91a7e6264c14d7`, client `fd956c91bf09e059359c8e182a33583e2c626cd3`, SHA256 `94b55e21e22be3c45aaa2c50b878965e4e730abd4e9dd69ab0533af7b850d330` |
| Primary client (unchanged) | `3456edc8dabf7b25ada78110ffa56327af9f67a4` |
| UTC | 2026-09-08T13:24:36Z |

No SSH and no native execution. Local inspection only.

## Verdict

**BOUNDED ACCEPT** of this correction/evidence reconciliation gate.

Independently confirmed:

1. Capture-corrected N=1 diagnostic `latency-diagnostic-input-trace-20260908-b` functionally proves the Game Image capture channel (stream `channel=1` through all 40 pulse edges; host drain and metric admission on the live slot).
2. Declared decode and input cohort membership is complete in the raw journal (300 Decode + 20 Panel, all starts in `[START,END)`, all `Completed`, zero losses, producers joined).
3. Host `BOT_DEBUG` environment lookup is cached on `142f7c3` (and identically patched onto both native role heads).
4. Native role source symmetry versus parents `9268890` / `fb3589a` holds after the cache commit.

This does **not** release:

- cache-included native rebuilds/tests of `e25f328` / `ca56e14`
- the predeclared matched N=16 companion
- performance / p99 / RSS / lifecycle / scaling acceptance
- whole-campaign Grok 4.6

Diagnostic b used the capture-corrected candidate binary `e3a2cbf`, which predates the host debug cache. That is the authorized functional localization run. It is not a cache-included native proof.

## What was inspected

Source: `git diff f9d729c 142f7c3 -- crates` is only `crates/host/src/lib.rs` (`+57/-5` cache). HEAD crate tree matches `142f7c3`. `debug_enabled()` is `debug_flag() || *BOT_DEBUG_ENV.get_or_init(read_bot_debug_env)`; `read_bot_debug_env` uses `var_os` + exact `OsStr("1")`. `f9d729c` still had per-call `std::env::var("BOT_DEBUG")`. `input_seam_trace::enabled()` still uses `debug_flag()` plus its own atomic env cache, not `debug_enabled()`.

Contract: prior Grok 4.6 integration review; `native-cohort-input-followup-protocol.md`; `native-cohort-input-followup-report.md`; companion plan §§4–7.

Raw evidence under `diagnostics/windows-input-trace-cohort-b/` (untracked archive; not git source):

- archive SHA256 `2a28ad9a143a08beb0ebef12fba84effafa7c547f9e343adba1b3e9f815f8b9e`
- manifest SHA256 `85a358418d167cc62e2aad3f2fec94cff72aa4baee7d2c46b143ab59122dd028`
- all 31 manifest entries: path, length, and SHA256 match; no extra files in the run basename
- `samples.cohort.jsonl` journal, `stderr.log` input-seam trace, `terminal.json`, `launch.json`, `tooling/stage-receipt.json`, `root-cohort-analysis.json`, `root-archive-verification.json`

Reader SHA256 `cd383fc3c9ced01efd12c5bd077d74b20d70db2feb4b7001e0757e4c84603d40` (same as the frozen integration manifest). Stimulus helper in the archive is SHA256 `04822c0e9f5ade2d555e408cc707722c234bc1e7b442676975a16e7efcaebbd1` (`input-stimulus-f64b81d.ps1`), matching the prior contract hash.

## Capture channel

`launch.json` / stage receipt bind N=1, `focused-one`, 120s warmup, 180s observe, responsiveness + input-seam, binary hash `94b55e21…`, stage `cohort-candidate-e3a2cbf`. `terminal.json`: exit 0 at `2026-09-08T13:12:17.7976723Z`, `timedOut=false`.

Independent `stderr.log` input-seam counts:

| stage | n |
|---|---|
| `win_arrow` | 40 |
| `imgui_arrow` | 40 |
| `stream` | 40, all `channel=1` |
| `drain` | 40, all `enabled=1` |
| `metric_start` | 20, all `slot_id=0xcda89722ead9ad9b` |

`0xcda89722ead9ad9b` = `14819260750087433627` (live slot, generation 2). One startup `capture_tx=0` on the first gate before draw; later gate lines are `capture_tx=1`. No `saturate` stage. This is the postfix of the prior diagnostic-a confounder (`channel=0`, drain/metric_start 0).

## Cohort membership

Header: frontend `panel`, input `{kind:focused-one,slots:[0]}`, decode `{kind:all-run-slots,n:1}`, boundaries `START=144359848700` `END=324359848700` `tail=5e9`.

Raw journal (180 batches): 320 unique identities, 300 Decode + 20 Panel, sequences 1..320 with no duplicates, one slot/generation, every start in `[START,END)`, every outcome `Completed`, completion ≥ start and ≤ `END+tail`, zero loss receipts. Terminal: `available=true` `records_n=320` `losses_n=0` `pending_n=0` `producers_joined=true`.

Direct `read_cohort` with qualifier-mapped metadata (`n=1`, panel, focused-one, both responsiveness flags true): decode `available` event_n=300; input `available` event_n=20. Canonical `analyze_run` remains `missing metadata.json` for both cohort gates. That mapped call is structural verification only — not matched provenance and not performance acceptance. Coarse bucket uppers 20 ms decode / 25 ms input are diagnostic descriptions, not an N=16 cell.

`root-cohort-analysis.json` matches these independent counts. It does not create canonical metadata.

## Debug-env hot path

Finding 1 of the prior Grok 4.6 review is corrected on `142f7c3`. Production `debug_enabled` has one `get_or_init(read_bot_debug_env)` path; repeated calls do not allocate an environment string. `set_debug` remains a live atomic OR. Exact `OsStr("1")` match. Native cache commits `e25f328` and `ca56e14` have the same `crates/host/src/lib.rs` patch-id as `142f7c3`:

`54435b106fe40e88b8a843103a049adfbbc2485d`

The full cache commit (lib.rs + report) patch-id is identical across primary `86237c3..142f7c3` and both native parents-to-cache:

`82c16a810932a6f4be876042a6c4d50a608f9151`

Primary `lib.rs` blob `2a7589f…` is not byte-identical to native `0a366d9…` because the native parents already differ in host snapshot-dedup / owner-census. The cache hunk is the same; the blob difference is the preserved role base, not a divergent cache.

## Source symmetry

Independent `git diff --patch-id --stable`:

| Diff | patch-id |
|---|---|
| `9268890..e25f328` crates | `cb00240445cb30c9d361707a1295954a60157c1a` |
| `fb3589a..ca56e14` crates | `cb00240445cb30c9d361707a1295954a60157c1a` |
| `e25f328..ca56e14` crates (role) | `6d29db24d5ccc97cdd2e5271a6a849a2dc3681c6` |
| `9268890..fb3589a` crates (role) | `6d29db24d5ccc97cdd2e5271a6a849a2dc3681c6` |
| `e25f328..ca56e14` full tree | `e95ca64e3d1d5caa8735fcec9f2360a5e7383b4b` |
| `9268890..fb3589a` full tree | `e95ca64e3d1d5caa8735fcec9f2360a5e7383b4b` |

Clients remain `abb811bd0afa1acd99319ccd5bc36bfb241080f9` (reference) and `fd956c91bf09e059359c8e182a33583e2c626cd3` (candidate).

## Local checks here

| Command | Result |
|---|---|
| Manifest 31/31 hash+length | pass |
| Archive SHA256 | `2a28ad9a…815f8b9e` |
| Independent journal membership | 320/300/20 complete, 0 loss |
| Direct `read_cohort` (mapped meta) | decode+input available |
| `analyze_run` | `missing metadata.json` (expected) |
| `python3 -m unittest discover -s docs/memory -p 'test_cohort_reader.py' -q` | 60 passed |

No cargo native rebuild. No N=16. No performance claim.

## Remaining prerequisites

Root prepares these independently, in order:

1. Build and test the cache-included native roles `e25f328` (reference) and `ca56e14` (candidate). Record new binary hashes. This is a hard prerequisite to matched N=16.
2. Freeze canonical matched metadata/provenance and execute the predeclared Windows `focused-one` N=16 pair once.
3. Independently recompute both sides from raw evidence. Incomplete declared populations remain incomplete.

Not allowed from this review: treating diagnostic b as a cache-included native binary; using qualifier-mapped reader output as matched `metadata.json`; performance or campaign acceptance.

Final finish-plan / whole-branch Grok 4.6 remain open.
