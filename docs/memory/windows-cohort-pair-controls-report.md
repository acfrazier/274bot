# Native Windows cohort matched-pair controls

Status: prepared source-only; no Windows, network, native, live, build, or input execution was performed in this checkout.

## Contract

The controls prepare one native Windows panel `focused-one` matched pair with an active N=16 workload. Each role uses the same server, client cache, catalog, nav pack/flags, settings, geometry, backend, and Intel adapter policy. The fixed timings are 120 seconds warmup, 600 seconds observe, a 5 second cohort tail, and 60 seconds teardown grace. The observe population is fixed before reading results; the tail may close only starts admitted before observe end.

The reference lineage is host `e25f32806957b1a44c75a598cbdb7ab53afc5383`, client `abb811bd0afa1acd99319ccd5bc36bfb241080f9`; the candidate lineage is host `ca56e14371d1edb8c09df1276638ce196d502e36`, client `fd956c91bf09e059359c8e182a33583e2c626cd3`. The controller accepts the eventual binary hash only through an explicit root-supplied frozen receipt and manifest. It rejects missing, placeholder, wrong-role, wrong-source, wrong-client, wrong-binary, and receipt/manifest-mismatched provenance; source aggregate counts/hashes are never copied from an older control.

Clean performance environment removes `BOT_DEBUG`, `BOT_CPU`, owner census, memory policy/workload/output, and related diagnostic overrides. Responsiveness, fine responsiveness, rendering, GPU completion, scheduling, and failure-capture profiles remain explicit and identical.

## Input boundary

`invoke-panel-input-stimulus.ps1` is copied byte-for-byte from the reviewed helper and is not modified (SHA256 `04822c0e9f5ade2d555e408cc707722c234bc1e7b442676975a16e7efcaebbd1`). The prelaunch plan fixes one 120-pulse alternating Left/Right sequence, 1000ms cadence, 80ms holds, slot 0, no retry, and a 60–90 second window after observe-start publication. After launch root verifies scene2, capture enabled, slot-zero focus, actual PID/start/session, and the observed physical window rectangle/Game Image point; the controller never guesses coordinates or reuses identity. The pair controller accepts a completed post-run receipt only after the run, with helper/target identities, trigger timing, cadence, UI bindings, and available helper resource accounting. SendInput completion is not host acknowledgement or latency evidence.

If root cannot safely schedule the helper inside observe, it must supply a separately root-managed receipt or report the `{slot 0}` input gate unavailable. SendInput success alone is never host acknowledgement or latency evidence.

## Controls and no-launch proof

- `run-cohort-pair.py`: receipt-bound controller, explicit argv/spec, clean environment, server/cache/catalog/nav/conditions sidecars, and managed-cell invocation.
- `preflight-cohort.ps1`: privileged native preflight with pair timings and role stages.
- `prepare-cohort.ps1`, `stage-contract.ps1`, `run-contractcheck.ps1`: staged no-launch contract flow.
- `check-cohort-contract.py`, `test_cohort_pair_controls.py`, `validate-cohort-ast.ps1`: actual local parser/schema assertions and PowerShell AST target.
- `launch-cohort.ps1`, `poll-cohort.ps1`, `archive-cohort.ps1`: root-only scheduled execution, polling, and sidecar/raw-artifact archival.

The no-launch contract check runs a pure validation phase with `COHORT_NO_LAUNCH_VALIDATION=1`: it requires build/manifest/plan provenance and emits a contract receipt without requiring or claiming a prepare receipt. `prepare-cohort.ps1` consumes that completed contract receipt and emits the exact per-cell prepare receipt; `launch-cohort.ps1` remains the only path that requires and consumes preparation before scheduling. The staged launcher passes hashed role stages, host root, preflight directory, and all provenance paths into its fresh scheduled-task process. It does not start a client or server. The Python tests exercise the actual local argv parser plus post-run stimulus receipt validation, including wrong-run, stale, cadence, and incomplete-accounting failures, and the chronological contract → prepare → launch gate.

## Verification boundary

The Python contract tests run on macOS only and do not establish native behavior. Native PowerShell parsing, privileged preflight, actual builds, screenshot inspection, helper scheduling, managed execution, cohort-reader recomputation, and any performance/non-regression statement remain root-owned. The post-run envelope must include raw helper and target PID/start identity plus `resourceAccounting` sampled by the separate root-owned `windows_process_sample` accounting task (`status=available`, helper/target CPU seconds and RSS bytes, and non-empty sample records); this controller does not invent those samples. Old tile-boxed controls and all existing helper/report files are untouched. Missing or late helper evidence, or the accounting-task prerequisite, remains explicitly incomplete; it cannot qualify the pair.
