# Artifact-bound matched evidence reader

The reader binds explicit receipts to immutable saved binaries and their raw
run artifacts before invoking the existing qualifier and metric analyzer.
It does not establish performance acceptance. Helper overhead stays unavailable
until separate process_evidence integration; pair eligibility still ends at
`overhead_unavailable` when binding and native match keys succeed.

## Required artifact chain

- Receipt id/index/kind, typed frontend exit, exact run directory and binary,
  explicit UTC launch/completion envelope, and the recorded effective CLI.
- Persisted manifest path/SHA256, raw metadata/sample/qualification hashes,
  server-identity path/hash, and host-conditions path/hash when external.
- The launcher actual parser validates both enabled and disabled flags, panel
  render mode, headless terminal selection, stack mode, timing and canonical
  binary. Duplicate options follow real argparse last-wins behavior.
- The reviewed `build_provenance.verify_build` verifies canonical named binary
  and navpack/navflags/catalog paths against real file hashes, successful build
  exit, typed features/allocator, stable source digests and build/client commits.
  Metadata must declare non-empty actual paths for all three runtime fixtures
  (`nav_pack`, `nav_flags`, `catalog_path`) before verify; digest-only claims
  (hashes without paths) are rejected and never fall back to manifest paths.
  Metadata must carry matching nested `build_provenance` with completion_status
  unchanged. Missing runtime hash/configuration fields are never filled in.
- Top-level legacy source/client labels describe the checkout. The actual saved
  binary client/source identity comes from the separately recorded and verified
  build object. Reference/candidate host sources and binary hashes may differ;
  built client source identity remains an equality-checked configuration key.
- Qualification and analysis are recomputed independently. Raw files, manifest,
  binary/assets, receipt and external identity/host sidecars are rechecked after
  consumption. Malformed/unreadable artifacts return unavailable rather than a
  false binding or unhandled type/parse exception.

Explicit launcher/sampler failure, receipt binding_errors, missing/changed
artifacts and inconsistent time/CLI/configuration are rejected. A binding is
not eligibility: missing runtime match keys, qualification failure, overlapping
observation windows, missing ordinal mapping, native match-key mismatch,
endpoint mismatch or unknown helper overhead still prevent a pair from being
eligible. Configuration comparison retains every non-side key in the metric
adapter. Server identity includes PID plus process start identity; its
non-sensitive configuration is an explicit separate required match key.

## Native qualification (ordinals / settings / wall brackets)

`consume_native_qualification` reads hash-bound `samples.qualification.jsonl`
and validates:

- Exactly one ordered `observe-start` then `observe-end`.
- Typed complete N slots with ordinals `0..N-1` exactly once, array order
  matching ordinals, unique names and responsiveness/cadence slot IDs per
  boundary.
- Within-run stable name and instrumentation IDs across start→end at each
  ordinal. Cross-run identity is **ordinal only** — generated names and
  run-local slot IDs are exposed in `ordinal_mapping` but are never equality
  match keys across runs.
- Actual per-slot `runtime_settings` (lowmem/midi/wave bool+int + draw).
  `loop_cycle` is freshness-only (typed presence + meaning required; not
  compared across runs). Null/missing settings cannot fill matching defaults;
  observed `false` is distinct from null.
- Global qualification settings (host/port/lowmem_requested/mainland/frontend/n/
  workload/render_policy/single_renderer/diagnostics/failure_capture/profile
  enabled flags + env_flags_requested + cache canonical availability). Renderer
  rows optional when render profile is off (explicit disabled marker); required
  when enabled with stable backend/presence/draw/full_rate/ended (no timestamps,
  generation, or run-local renderer slot ids in match keys).
- Start/end runtime config must be equal; transition →
  `runtime_config_changed_between_observe_start_and_end`.
- Physical audio output remains unobserved (`physical_output_available` must
  not be true). `cache_content_hash` stays null at boundary (fingerprint
  integration pending separately).
- Prefer native `elapsed_wall_bracket` envelope (start.before → end.after) for
  `observation_wall_span` when brackets validate (finite, ordered, within
  launcher envelope, elapsed consistent within tolerance). Source labeled
  `native_elapsed_wall_bracket`. Legacy rows without brackets keep analysis
  observation-window fallback; malformed native brackets fail closed.

When native qualification is available on either side of a pair, both sides
must be available and their native match-key dicts must equal
(`native_match_key_mismatch` otherwise). N>1 without validated ordinals →
`missing_stable_slot_ordinals`.

## Sampler duration_mode (schema-2)

Match keys include `sampler_duration_mode` (`fixed` | `stop_controlled`).

- Legacy receipts omit `duration_mode`; a present positive
  `duration_s_requested` is treated as `fixed`.
- `fixed` still requires finite positive `duration_s_requested`.
- `stop_controlled` requires the key present with explicit JSON null (not
  omitted). Non-null under stop_controlled is rejected.
- Duration is never invented; continuous sampling coverage is not faked.

## Validation

`python3 -m unittest test_matched_evidence_adapter
 test_reference_metrics.ResourceAdapterTests -q` passed **42** tests. Fixtures
use real binary/asset bytes and persisted hashes, UTC envelopes matching their
metadata timestamps, completed nested build provenance and unchanged sidecars.
The positive fixture proves binding and qualification only, then reaches
`overhead_unavailable`. Multi-slot CLI-legal n is `{1,16,32,128}`.

Native cases cover: N16 ordinal match despite different generated names;
duplicate/missing/reordered/mismatched IDs; None vs false runtime settings;
differing runtime scalars/backend/scheduling_profile_enabled; stop_controlled
null duration vs fixed null / stop_controlled non-null; start→end settings
transition unavailable; legacy N1 without native rows still binds with
`slot_ordinals_present=False` and default `sampler_duration_mode=fixed`.

Earlier production-path negatives (UTC, profile flags, manifests, digests,
sidecars, build completion, launcher/sampler failure, binding errors,
digest-only fixtures, mutation probes) remain green. No live run or binary
build was performed for this Python integration.

## Remaining work / honest gaps

- Helper overhead stays unavailable until process_evidence is integrated;
  snapshots and receipt sampler overhead labels are not enough.
- Cache content fingerprint integration is separate (root optional cache
  receipt path); boundary hash stays null and is not a match key.
- Physical audio output remains unobserved by design.
- The approved OFF/ON/ON/OFF overhead sequence and matched repeated
  performance/lifecycle matrix have not been accepted. No caller
  overhead/qualified label unlocks a pass, and no confidence or latency
  margin is invented.
- Legacy N1 fixtures without native rows remain bindable; ordinals absent is
  OK only for n=1. N>1 requires validated native ordinals.
