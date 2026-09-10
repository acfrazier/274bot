# Selective harvest — Grok 4.6 closing acceptance review

Verdict: OPERATOR_TRADEOFF

This is the required closing Grok 4.6 acceptance review of the approved smaller harvest. It is not a campaign resumption, not a re-review of every previously approved file, and not authorization to rerun the superseded matrix.

Reviewer: grok-4.6 (xAI, `xai-oauth`), Hermes profile `branchreviewer`.
Prior code review: `selective-code-review.md`, session `20260909_223845_b90bbd`, verdict APPROVE of host `9527cc63` / client `daccb4ba` for continued validation only — not release acceptance.

No Git mutations, remotes, builds, live workloads, or publication were performed here. The only write is this report. Root owns Git publication and merge.

## Exact SHAs

Host (`codex/selective-integration`, clean HEAD):
- baseline / local main: `54cfcf8a33613735bde9f43dd9c9beb8ec06dae9`
- live/native/comparison freeze: `9527cc636187fdfaaca29c3e4af14819dd40dbe9`
- final source: `c5159d38534281af66f5267e56fc5f0995d09071`

Client submodule (`vendor/fr-client-rust`):
- baseline (main pin): `4f2048ea10f75b3bb92ff45610b35ba7313b0308`
- live/native/comparison freeze: `daccb4ba3ff5f8d1fce5b3b9487a1f0576ad50ac`
- final source (HEAD gitlink): `9b41e6e06b9fd42dc2247fc813a303c3fcb92941`

Native/live/comparison binaries were built and hashed at host `9527cc63` / client `daccb4ba`. They are not final-SHA binaries. There is no accepted SHA256 for a rebuilt `c5159d38`/`9b41e6e` executable.

Mac system-allocator freeze (`selective-integration-evidence/binaries/selected-build.json`):
- `tui-play` SHA256 `debf72cf91b61cf8609da13eb65113f861fd38e17504d21bde6d1f7773df91a5`
- `panel-play` SHA256 `c2f32149e85dfa7acd1952fee4075052e3bb8d5c195697ad1650868df5ed575a`

Windows/Linux freeze hashes remain those in `selective-native-validation.md` / `evidence/native/9527cc636187`.

## What changed after the prior APPROVE

Host `9527cc63..c5159d38`: 10 files, +301 / −165.

- `c6d748bd`: product docs only (`CHANGELOG.md`, `README.md`, `docs/README.md`, `docs/harness.md`).
- `9d2a0f65`: lint placement / Copy-Option idioms in `crates/api` and `crates/host-play/src/lib.rs`, plus gitlink pin to client `9b41e6e`.
- `c5159d38`: harness-feature lint — `ProfileSettings { auto_login: true, ..Default::default() }` and moving `HeapLifetime`/registry above the test module.

Client `daccb4ba..9b41e6e` (`9b41e6e`):
- test type alias for frame groups
- `DurationCounter: Default` delegating to `new()`
- equivalent minimap range `contains` in `fill_draw_area_rgba`
- comment wording

`FindOptions` is `Copy`; dropping `.clone()` while moving `ScriptRouteRequest` is equivalent. `FOOD_HEALS` and routing helpers moved as items, not rewritten. These match the stated mechanical lint cleanup. I did not re-open the previously approved preserve/exclude contract on unchanged files.

Shipping docs describe retained product and limits. They do not claim combined RSS, 32-bot CPU, capacity, stun-live proof, or final-binary identity. Source mapping and campaign artifacts stay on the campaign branch.

Recorded post-lint checks (not re-run here): workspace/all-target Clippy with warnings denied, including harness feature; `lint-client-regressions.log` 75+10+10+9 (library / `gpu_backend` / `gpu_texture` / `iface_model`); `lint-host-regressions.log` API 5 + host-play 146 = 151. macOS `gpu_texture` shade-16 passed in that log; Windows/Linux native shade-16 still fails as below.

## Functional evidence that may be claimed

Bounded selected-plan checks, all on freeze `9527cc63`/`daccb4ba` unless noted:

- Contested door, selected-baked pack: normal 57.34s and reverse 55.32s, both exact `(2817,3443,0)`, two ingame/scene2 slots. Pack SHA256 `c9dca67bcdc8d0c8a598fb316279851658c1113e56bbb8dd5d81356cdf773f17`.
- Mac sustained TUI N32: `tui-n32-functional-analysis.json` — `food22=32`, `closed22=32`, `return_resumed=32`, `errors=0`. Subsequent successful steals after return for all 32. Diagnostic only; concurrent activity; not a performance result. Spotanim 245 on all 32 is not Traveller stun-recovery proof.
- Mac native panel: root read the live window through CUA and toggled GPU→CPU→GPU with scene/minimap remaining visible. No saved Mac PNG. No live modal or scene1-timed freeze capture. No timed live reconnect-priority-order claim (queue/Play tests only).
- Windows panel PNG was read (SHA256 `c945307c03809e6d106e859881da9ba2a989a9cee1aec5e14a20e5ba0740e6a6`); scene/minimap/ingame scene2. Not a modal or freeze-interval capture.
- Native N1 active smokes: Windows TUI/panel and Concord TUI exit 0; bank to inventory 22 with bank closed; renewed Pickpocket after return. Windows panel observed a later successful steal (10 at 150.350s). Windows TUI and Concord TUI observed the renewed request only; no later success before the short window ended.
- Windows Stop/restart lifecycle: exit 0, 243.498s. Stop interval cleared active slots, live V8 isolates, V8 used bytes, and snapshot inflight bytes/capacity. Restart resumed Running with isolate peak capacity 284552 and script ticks 2→50. Not an RSS saving.
- Linux: compiled on existing Hyper-V `274bot-builder`, hash-checked and executed on Concord. No headed Linux desktop, panel scanout, or GPU frame-rate claim.

## Honest limits (must remain)

- Native/live/comparison evidence is freeze `9527cc63`/`daccb4ba`, not final `c5159d38`/`9b41e6e`. Do not invent a final binary SHA.
- Windows/Linux `gpu_textured_shade_scales_texel_brightness`: shade 16 expected ~223, got 255. Not weakened. Not a suite pass.
- Windows historical CRC unreachable-server timeout (~21s vs 5s bound) was not rerun and is not fixed.
- No controlled Traveller stun-recovery live claim (245 + eleven-tick inference remains unit/review only).
- No equal-productivity CPU win, latency percentile, additive RSS total, low-end budget, or 128-slot capacity.
- N1 comparison is one short pair, not a spread estimate. Seed sharing was common-API on both sides, so it does not measure the retained harness seed-sharing benefit.
- Mac N1 TUI observation ended during return walk; that cell alone is incomplete return evidence. The N32 TUI diagnostic is the 32-slot bank/return proof.

## Comparison result and selected-regression assessment

One sequential common-API comparison, ordinary Thiever, no sustain, N1 then N32, baseline then selected.

| Cell | Outcome | Mean current RSS | CPU cores | Player updates | XP |
|---|---|---:|---:|---:|---:|
| Main N1 | qualified | 289.88 MiB | 0.02999 | 100 | 234 |
| Selected N1 | qualified | 260.59 MiB | 0.02100 | 100 | 141 |
| Main N32 | qualified | 3267.72 MiB | 2.63358 | 3200 | 8890 |
| Selected N32 | failed before observation | ineligible | ineligible | ineligible | ineligible |

Selected N1 RSS is 29.29 MiB lower in this pair. XP 141 vs 234 means the lower CPU reading is not an equal-productivity CPU win. No latency evidence.

Selected N32: exit 1 at 152.300s, `actor readiness lost before observation`, no `observation_start`. Actor `live13d71_27` was `Paused` (XP 101520, 250 player updates, age 0.171558s, runner `Passed`, no script/start error). Later valid v7 save at exact Maze NW spawn `(2891,4597,0)`, SHA256 `825ee544d1441e114d56033091369dc34a76d7bc63f4d66f440a380ef81dd5bc`. Host `crates/host/src/random.rs` and `maze.rs` have empty diff vs baseline; `MAZE_SPAWNS[0]` remains `(2891, 4597)`. No captured transition/recovery. No N32 performance result. No rerun. No weakened predicate.

This does **not** establish a selected-code regression. Maze teleport plus a contemporaneous Paused row fit ordinary scene rebuild after a server random event. Selected food/routing can change *when* events fire; that is not a demonstrated bug. Focused reconnect was inactive (driver never prefers login). Presence gate and maze code are unchanged. The separate sustained Mac TUI N32 completed bank 22 / close / return / later steals on all 32, which contradicts a general selected 32-slot gameplay break.

It also does **not** become a passing N32 comparison, a proven benign race, stun-recovery failure, or packet stall. Main N32 success does not prove main would have avoided the same Maze gate.

Under the approved completion section: one bounded diagnosis was done; missing evidence is reported; implicated changes are not silently waived and are not rolled back on this record. The superseded large matrix is not required.

The operator tradeoff is whether to accept the harvest **without** a paired N32 performance/timing result (the scalar-experiment revisit the plan asked for), while keeping the failed cell visible.

## Concrete blockers

None that require selected-code changes, test weakening, or another comparison campaign.

If the operator refuses the missing N32 paired result, the next step is a newly specified bounded diagnostic (exact rejected SlotStatus + guardian/presence, or a controlled Maze exercise). That is optional follow-up, not this review demanding it.

## Publish / squash

Existing approval covers the selected code combination and this smaller plan. It does **not** by itself publish the client or squash-integrate the host.

The selected client **may** be published, and the host **may** be squash-integrated, **only if the operator accepts** the tradeoff above plus the honest limits. Campaign source mapping stays on the campaign branch; shipping docs already describe retained product without overclaiming the missing N32 number.

If accepted:
- Publish client `9b41e6e06b9fd42dc2247fc813a303c3fcb92941` so the host gitlink is fetchable.
- Squash-integrate host `c5159d38534281af66f5267e56fc5f0995d09071` with mapping in the merge description.
- Do not label freeze binaries as final-SHA builds.
- Root performs all Git publication and merge.

If the operator does not accept shipping without N32 paired performance, do not publish or squash on this review.

OPERATOR_TRADEOFF.
