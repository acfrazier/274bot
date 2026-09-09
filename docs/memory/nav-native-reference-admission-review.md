# Native singleton reference admission review

**Task:** `t_00284081`  
**Reviewer:** profile `reviewer` (round 1, artifact lens)  
**Branch:** `codex/memory-diagnostics`  
**Scope:** reference/tooling provenance only — not F1/F2/all-59 performance, not campaign acceptance, not real routing release.

## Verdict

**APPROVED**

Root’s platform decision and Linux reference packaging preserve honest evidence identity under the fixed `run-05` comparison path. Independent byte and tool checks match the claimed original f24 Linux generated baseline; layout equality was not skipped or normalized; failures and prior macOS archives remain intact. This admits reference/tooling provenance for the Linux-ref singleton package only. Real F1 and performance acceptance remain withheld.

## Artifacts reviewed

| Artifact | Independent result |
| --- | --- |
| `docs/memory/nav-native-reference-boundary.md` @ `775f214` | Bound decision: macOS run-05 layout failed on Linux; select original f24 Linux outs; no skip/normalize; not performance acceptance |
| `diagnostics/nav-stage-a-native-preparation/root-linux-reference-manifest.json` | 12 files, platform `Linux-6.8.0-139-generic-x86_64-with-glibc2.39`, tool commit `f24de7cbcaaa3f419fc5485b77c4a117543a6afc`, source `/home/builder/274bot-campaign/nav-stage-a-f24de7c-1327`, 7 comparison keys, all `all_equal: true` |
| `linux-f24-reference/` (12 `.out` files) | SHA-256 of every file matches the root manifest |
| `builder-native-evidence-f24de7c.tar.gz` SHA `5b82a2c856485adc4e76cb06a760050455849885c2853acbad02c3bf1e8bb621` | Same 12 paths under `native-build-01/qualification-*` byte-identical to `linux-f24-reference/` |
| `concord-singleton-007346f-linuxref.tar.gz` | SHA `58a898896bf313204e060d830adca3f33f6a354e2dd5ebd4b60e495822805dd0`, 856 members, 5873883 bytes — matches receipt + `root-concord-singleton-package-verification.json` |
| Package path `…/run-05/qualification-*/…` | All 12 outs byte-identical to builder f24 Linux originals; all 12 differ from local macOS `docs/memory/nav-tiled-stage-a/run-05` and from older `native-singleton-source-007346ff.tar.gz` run-05 |
| Package `native-reference-manifest.json` | Identical to root Linux reference manifest |
| `native-singleton-source-007346ff-linux-reference-01.tar.gz` SHA `5ba2fdf224f9934c38b1a0d29ec7c1b9f17e69738d54e86643b22a3e5c054800` | Tools + native-reference-manifest match the Concord package |
| `audit_native_reference.py` | Read-only old f24 Linux vs new 007 Linux comparison; asserts the same 7 keys |
| `root-native-layout-comparison.txt` | new == old_linux layouts; old_mac differs at usable-size slots |
| Packaging helpers `package_reviewed_singleton_tool.py`, `build_native_singleton.py`, `package_native_singleton_for_concord.py` | Alias into `run-05` only after Linux manifest/hash checks; reuse of first seven steps labeled; Concord scope denies real release |
| Failure evidence | `native-e6958-scheduler-failure.log`, `scheduler-guards.err`, `root-inherited-as-reproduction.json`, layout comparison, prior mac packages present |

## Independent checks (executed)

1. **Package integrity.** `shasum -a 256` and `tar` member count 856 confirm package SHA/size/members vs root verification JSON.
2. **Triple byte identity for reference outs.** For each of the 12 cells: local `linux-f24-reference` == root manifest SHA == `builder-native-evidence-f24de7c.tar.gz` member == package `run-05/…` member.
3. **Not macOS run-05.** Package run-05 differs on all 12 from workspace historical run-05 and from the pre-linuxref `native-singleton-source-007346ff` package (sample uniform clean: mac/old `d69e5a41…` vs Linux `d253ba1d…`).
4. **Layout / platform allocator.** Parsed via `stage_a.output`: mac dense uniform layout indices 6–7 = `[16384, 2048]`; Linux = `[16392, 2056]`. Tiled directory usable bytes 128 (mac) vs 136 (Linux) match boundary text and `root-native-layout-comparison.txt`. Mismatch is platform usable-size introspection, not structural capacity drift. Selecting Linux historical outs is the correct platform fix; dropping layout from the key set would have been dishonest and was not done.
5. **Original f24 admissions/tools from builder archive.** All four admission JSON SHAs and all four `stage-a-probe` executable SHAs match `root-linux-reference-manifest.json`. All eight `original_tools` hashes match both the builder archive copies and `git show f24de7c:docs/memory/nav-tiled-stage-a/…`.
6. **Current tool binding `007346ffc6a06993141a169bb099201d9cc11985`.** All packaged stage/scheduler/README files match `git show 007346f:…`. Qualification `result.json` tool digests bind to those same package bytes. Scheduler set: `qualify_sharded.py` `e55bbf57…`, `sharded.py` `8bfda30e…`, `test_sharded.py` `b2643c8f…`, `test_sharded_guards.py` `45dd91f9…` (matches prior 007 fixture-fix review receipts).
7. **Frozen probe span continuity.** `stage_probe.rs` differs f24 (`bcf17562…`) → 007 (`1c3fb5f7…`), but qualifier-required spans `route_calls`, `warm_timed`, `owners` are byte-equal to f24; recomputed span digests match `singleton-qualification-03/result.json` `helper_spans`.
8. **New Linux admissions (rebuilt/reused binaries).** Package `concord-run-01` probe binaries match their own admission `executable_sha256` values and stage tool hashes (including current `stage_probe.rs`). They intentionally do **not** equal the original f24 executable SHAs — comparison is output/layout keys against frozen Linux reference outs, not binary identity to the historical f24 build.
9. **Fresh scheduler proof vs reused steps.** Package `builder-provenance/root-qualification-progress.json`: seven steps (`guards`…`integration`) `returncode=0` with `reused_from` / note “original completed result, not a fresh execution”; only `scheduler` is non-reused (`wall_seconds≈60.0`). `singleton-qualification-03/result.json`: `qualified=true`, `native_hard_as_qualified=true`, platform Linux, `scope=generated/local tooling only`, **12** comparisons. Guards err: `Ran 24 tests in 58.023s` / `OK`. Every comparison `old_sha256` equals the Linux reference manifest SHA for that cell; `raw_calls=840` on all twelve.
10. **Caps unchanged.** Packaged `sharded.py` still has `QUAL_LIMITS = dict(wall=360,cpu=300,rss=512*1024**2,address=4*1024**3,output=1024**2)`.
11. **Alias mechanism.** `package_reviewed_singleton_tool.py` copies `reference_run` outs into the fixed qualifier path `run-05/qualification-…` only when `native-reference-manifest` asserts Linux platform, f24 tool commit, and per-file SHA match. `qualify_sharded.py` still hardcodes `HERE/run-05/…` (path slot, not platform claim).

## Platform / `run-05` alias assessment

| Question | Assessment |
| --- | --- |
| Does fixed path `run-05` still hold macOS historical identity? | **No.** Under the linuxref package the path is an **alias slot** filled with original f24 Linux generated outs. |
| Is evidence identity honest? | **Yes**, via package-root `native-reference-manifest.json` (platform, source_root, f24 commit, admissions, 12 SHAs, 7 keys), boundary doc `775f214`, packaging assertions, and independent archive byte match — not via the path string alone. |
| Was layout skipped or normalized? | **No.** Full key set retained; mac reference attempt failed and is preserved; Linux reference selected. |
| Does `README-sharded.md` document the Linux alias? | **No.** It still says only that comparison requires preserved `run-05` fixture outputs and ancestor `f24de7c`. It does not name the Linux f24 content swap or `native-reference-manifest`. |

**Judgment:** For this admission card, the explicit manifest + packaging + boundary report are sufficient to preserve honest identity without a qualifier path/schema rename. Renaming the path would churn the reviewed `007346f` tool surface. README silence is a **residual documentation debt before real F1 release** (operator-facing), not a provenance falsification: anyone opening the linuxref package still finds `native-reference-manifest.json` and Concord payload scope text denying real release.

## What this does *not* approve

- Real routing F1 / F2 / all-59 performance or campaign acceptance  
- Fresh Concord-side generated qualification completion (task states a Concord run may still be active; this review did not supervise it)  
- Claim that original all-row temporal experiment is qualified  
- Treating path name `run-05` alone as proof of macOS historical continuity  
- Any cap change, dropped failed cell, retry-until-pass, or favorable timing selection  

## Residual (non-blocking for this card)

Before a **real** F1 release package, root should either (a) land a small reviewed tool/doc commit clarifying that native Linux packages stage original f24 Linux outs under the fixed `run-05` comparison path and point at `native-reference-manifest.json`, or (b) keep operator instructions exclusively on the boundary/manifest path so README cannot be misread as “macOS run-05 still qualifies Linux.” Not required to admit current reference/tooling provenance.

## Verdict line

**APPROVED** — native Linux reference alias and `007346f` linuxref package provenance verified independently; reference/tooling admission only.
