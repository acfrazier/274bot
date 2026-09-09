# Direct-owner admission correction

Task: `t_5c2087e1`

## Outcome

The direct-owner controller no longer treats non-empty JSON admission files as evidence. Direct mode now requires one explicit root release contract plus five typed receipts, validates every object against an exact schema, binds the claims to current build/source/cache/server state, and fails before frontend `Popen` when a claim cannot be proved.

The ordinary and Heaptrack modes remain unchanged. `--preflight-only` still does not reserve output or create a managed launch.

## Root invocation input

A direct-owner invocation must add:

- `--release-contract <path>`
- `--conflict-receipt <path>`
- `--account-admission <path>`
- `--population-admission <path>`
- `--cache-admission <path>`
- `--server-health-receipt <path>`

All six paths must already name regular, non-symlink JSON files no larger than 1 MiB. The release contract binds the canonical paths and SHA-256 values of all five receipts. No default release duration or implicit receipt path is supplied.

## Shared context

The release contract and every receipt contain the same exact `context` object:

- `fixture_binding_sha256`: root-selected opaque SHA-256 identity for the admitted active fixture; it is repeated in account, population, and cache evidence without exposing an account name, password, vault record, or cache payload.
- `host_commit`, `client_commit`
- `host_sources_sha256`, `client_sources_sha256`
- `build_manifest_sha256`, `binary_sha256`
- `nav_pack_sha256`, `nav_flags_sha256`, `catalog_sha256`
- `cache_content_identity_sha256`, `cache_snapshot_version`
- `server_pid`, `server_start_identity`, `server_executable_basename`

The controller computes or samples every context value except the opaque fixture binding and requires exact equality. Commits are lowercase 40- or 64-hex identities; SHA-256 fields are lowercase 64-hex; cache snapshot version is lowercase 16-hex. Server PID is a positive JSON integer, start identity is non-empty, and executable identity is a basename rather than a path, argv, or environment value.

## Root release contract schema

The top-level object has exactly these fields:

- `schema`: `direct-owner-root-release-v1`
- `mode`: `direct-owner-v1`
- `n`: integer `1`
- `workload`: `active`
- `frontend`: `tui`
- `issued_unix_s`: finite nonnegative number
- `observation_window`: exact object with `not_before_unix_s` and `not_after_unix_s`
- `expires_unix_s`: finite nonnegative number selected by root
- `boot_id`: current non-empty boot identity
- `context`: the exact shared context above
- `spec_binding`: exact canonical path object below
- `receipt_bindings`: exact binding object below

Time ordering must be:

`not_before_unix_s < not_after_unix_s <= issued_unix_s <= current_time < expires_unix_s`

`spec_binding` has exactly:

- `result_path`, `cell_dir`, `run_dir`, `frontend_handoff_path`
- `build_manifest_path`, `binary_path`
- `nav_pack_path`, `nav_flags_path`, `catalog_path`
- `cache_dir`, `unpack_root`

Every value must equal the current canonical spec/file path.

`receipt_bindings` has exactly the keys `conflict`, `account`, `population`, `cache`, and `server_health`. Each value is exactly `{ "path": <canonical path>, "sha256": <lowercase 64-hex> }` and must match the corresponding explicit capture-contract path and actual bytes.

## Common receipt schema

Each receipt has exactly:

- `schema`, `kind`
- `mode`: `direct-owner-v1`
- `n`: integer `1`
- `workload`: `active`
- `frontend`: `tui`
- `admitted`: JSON boolean `true`
- `observed_start_unix_s`, `observed_end_unix_s`
- `boot_id`
- `context`
- `evidence`

`schema` is `direct-owner-<kind>-admission-v1`, with underscores in the kind replaced by hyphens. `kind` must match the capture-contract slot, preventing receipt swaps. The observation values are finite nonnegative numbers and must satisfy:

`release.not_before_unix_s <= observed_start_unix_s < observed_end_unix_s <= release.not_after_unix_s`

Boot and shared context must equal current measured state and the release contract. Unknown or missing fields, wrong JSON types, false admission, stale times, and identity drift are rejected.

## Per-kind evidence

### `conflict`

Exact fields:

- `checked_classes`: a four-element list containing exactly `build`, `test`, `profiler`, and `tui-panel-frontend`, with no omission, duplicate, or unknown class.
- `matches`: a list. Each entry has exactly `class`, positive integer `pid`, non-empty `start_identity`, and basename-only `executable_basename`.

A non-empty `matches` list fails admission. The controller records only a zero match count and checked classes in public summaries; it neither signals a matched process nor copies argv/environment.

### `account`

Exact fields:

- `fixture_binding_sha256`
- `slot_count`: integer `1`
- `active_slot_count`: integer `1`

### `population`

Exact fields:

- `fixture_binding_sha256`
- `requested_n`: integer `1`
- `admitted_n`: integer `1`
- `workload`: `active`

### `cache`

Exact fields:

- `fixture_binding_sha256`
- `content_identity_sha256`
- `snapshot_version`

The cache identities must equal a fresh controller capture of the current cache/unpack roots.

### `server_health`

Exact fields:

- `pid`, `start_identity`, `executable_basename`
- `probe`: exact object `{ "host": "127.0.0.1", "port": 43594, "succeeded": true }`

The process identity must match the declared sidecar and controller samples. The controller independently performs a loopback TCP probe immediately before admission validation and records its exact start/end interval. The independent probe must begin no earlier than the root release issue time and finish no later than the controller's current-time sample. The server process and executable basename are sampled again after the bounded preflight interval; PID reuse, executable replacement, missing state, or a zombie fails admission.

## Root preparation sequence

1. Select a finite observation window and expiry policy; record the current boot identity.
2. Obtain the already-required verified build/source manifest, binary, nav/flags/catalog files, and current cache snapshot identity.
3. Select an opaque SHA-256 fixture binding without placing private fixture data in any public file.
4. During the release observation window, observe all four conflict classes and record every match only as class/PID/start identity/executable basename. Admission requires no matches.
5. During that same window, produce account, population, cache, and server-health observations using the schemas above. Account and population must prove one active N1 fixture; cache must use current controller-verifiable identities; server health must use the declared current server and loopback 43594.
6. Hash the final five receipt byte streams and put their canonical paths/hashes into `receipt_bindings`.
7. Issue the release contract after the observation window with root's explicit finite expiry and all current context/spec bindings.
8. Invoke the controller with all six explicit paths before expiry. Any rewrite requires a new hash and release contract.

## TOCTOU and durable receipt behavior

The controller rechecks release/receipt hashes after the preflight interval, after `launch.json` creation but before frontend `Popen`, and after the run. It also rechecks build/source/provenance bindings immediately before `Popen` and after the run. `launch.json` accepts exactly six immutable admission bindings: the release contract plus the five typed receipts. Completion independently re-hashes those six files; mutation makes the durable managed receipt `failed_or_unavailable`.

## Generated verification

Evidence is under `diagnostics/direct-owner-managed-extension/admission-correction/`.

- Direct generated tests reject the four root-reproduced invalid families: arbitrary object, expired object, explicit rejection, and wrong server/conflict object. They assert no `Popen` call.
- Mutation coverage rejects common and per-kind missing/unknown/type/value errors, omitted/unknown conflict classes, malformed and admitted conflict matches, swapped receipts, altered hashes/paths/identities, stale receipt/release/current-probe times, missing raw swap/OOM counters, and receipt/provenance mutation before launch.
- Supporting calibration, build-provenance, and managed-receipt regressions pass.
- The full managed-cell module has one pre-existing macOS order-dependent control failure comparing parent and child `time.monotonic()` epochs. The same test fails at unmodified HEAD and passes alone; both baseline and current logs are preserved. This task does not alter that default-path test or its timing policy.

No native or live admission was run. Root owns the post-review Linux/native and live qualification.
