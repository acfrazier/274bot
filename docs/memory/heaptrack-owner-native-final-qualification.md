# Corrected native fixture qualification — 2026-09-09

Status: qualification passed; retained saved-smoke pair was not run after its launcher failed before manifest creation. No production input, capture, replay, retry, or release authorization occurred.

## Frozen source and admission

The run used reviewed source `a55972aa5cce2902c2b09aa098ee825d84a5957a` and the staged tree `/home/acfrazier/owner-native-a55972a`. The fresh qualification admission at `2026-09-09T00:43:07.705221+00:00` recorded Concord boot `2217ec26-dc3e-47a3-a700-d3fafb34163f`, 1,003,257,856 available-memory bytes, 11,812,732,928 free-disk bytes, and no listed conflicts. Both qualification output and saved-smoke output were absent at admission.

The worker checked the native-stage-manifest SHA-256 `d00b2bb28af11ab9bbd4b21ea992d87d7f4d5d084b3e16febb563f04ca53adaf`, a stage-file listing digest, and the three staged executable hashes:

- child `98b5a2a20b0daf66057024f4618972fb34219b34568a09fa3be89a674d265b7f`
- fixture `658acea5f979e68fed27f472ae6191182503e13639ea6c8788b5d0e9fa236fc2`
- core tests `9aafaf124041877932483fef5b2b26f7e5512bb885d5d8f7acbcaf456ae45266`

This worker receipt records the checks actually performed; its stage-file listing digest is not claimed as a substitute for rehashing every named file. Root separately performed and retained the post-qualification all-41-file rehash against the bound manifest.

## Qualification result

The exact Linux command omitted `--portable-probe` and completed successfully in 33.954859 seconds. The retained raw output is `diagnostics/replay-native-a55972a-qualification-1/qualification.json`, SHA-256 `26185ab784edebeec9a667e2a83bfc8d63454463df5b07cffa72ee4d0025d786`.

- Native core tests: 16 passed, 0 failed, 0 ignored.
- Native/Python suite: 42 passed, 0 failed, 0 skipped.
- Declared and completed generated cases: 9.
- Timed repetitions: 27 (three serial repetitions per case).
- Six boolean necessary-rate cases passed unchanged thresholds; no boolean case failed.
- The three diversity/symbol/near-line-cap cases ran and retain rates but are not substituted for the necessary-rate gates.
- Qualification reports `production_read=false` and `production_retry_authorized=false`.

I independently recomputed 81 stored phase rates and 27 three-repetition medians from the individual phase CPU rows. All checks match the qualification JSON; the receipt is `diagnostics/replay-native-a55972a-qualification-1/arithmetic.json`.

The qualification's external Linux resource sampler is sparse. A one-sample cell does not establish a process peak or sustained sampling cadence; `max_sample_interval_s` excludes unsampled tail time. Native `cumulative_peak_rss_bytes` is a separate cumulative child metric and must not be reported as the sparse external sample maximum. These bounded warm fixtures do not establish production population, table, output, I/O, or capacity acceptance.

## Preserved launch deviation and saved-smoke boundary

Two qualification driver invocations are retained, not relabeled as one:

1. Driver PID 3492 exited 1 because the wrapper pre-created `qualification-1`; `qualify.py` failed at `root.mkdir` before executable hashing, core tests, differential tests, or timing cells. Raw evidence is preserved under `diagnostics/replay-native-a55972a-qualification-1/qualification-launch-wrapper-error-1/` and root's exact tool result is `diagnostics/root-native-a55972a-qualification/qualification-first-launch-error.json`.
2. Driver PID 3510 was the next invocation and produced the successful qualification above. Its launch/admission/driver/completion receipts are retained in `diagnostics/replay-native-a55972a-qualification-1/`.

After qualification passed, the one authorized saved-smoke attempt was started with a fresh admission. It failed before manifest creation because the unchanged `smoke_manifest` helper resolved the fixture inventory relative to `/home/acfrazier` rather than the staged root. No manifest was built, no Python or native runner was invoked, and no smoke output or equality result exists. The failed attempt is retained at `diagnostics/replay-native-a55972a-qualification-1/saved-smoke-1/`; root's exact raw result is `diagnostics/root-native-a55972a-qualification/saved-smoke-first-launch-error.json`. It was not retried. A later `saved-smoke-2` decision/evidence is outside this card and is not included here.

Historical failed qualification reports remain untouched, including `docs/memory/heaptrack-owner-native-corrected-qualification.md` and `.json`. This report does not authorize production replay or capacity claims.
