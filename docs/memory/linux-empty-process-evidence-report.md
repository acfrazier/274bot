Linux empty frontend observation report

Scope

This is a narrow provenance-reader repair for an explicitly observed Linux host-condition field. Runtime code, benchmark inputs, raw evidence, and protocol behavior were not changed. The reader still binds only hash-matching sidecars and does not establish performance acceptance.

Finding

The legacy host-condition validator recursively classified every empty list as missing. That is too broad for the explicitly Linux-tagged preflight field `frontend_processes`: an empty list can be a valid observation that no conflicting frontend processes were found. Empty arbitrary fields remain invalid. The legacy path retains its existing missing-leaf checks; it does not define a required-key schema for otherwise absent metadata.

Repair

`matched_evidence_adapter.py` now recognizes only a top-level `frontend_processes` observation paired with an explicit platform string beginning with `Linux`. Its value must be a list; an empty list is accepted as an explicit zero-result observation. Non-empty entries must be non-empty objects with a positive, unique integer `pid` and a non-empty string `name`, and all nested floating-point values must be finite. Missing, null, empty, wrong-type, malformed, non-positive PID, duplicate PID, and non-finite process records are rejected. The field is removed only for the legacy recursive check of the remaining document, so arbitrary empty unknown fields and non-Linux empty observations remain rejected.

Windows native observations retain their existing recognized-schema validator, including optional null fields, process/PID typing, contradictory duplicate checks, path/hash checks, and fail-closed required records. Sidecar path binding, recorded SHA-256 verification, raw-file rechecks, manifest/build provenance, and pair eligibility are unchanged. The raw Linux artifact is not rewritten or normalized, and no performance conclusion is made.

Verification

Targeted RED test before implementation: the new positive empty-list and exact-sidecar tests failed with `host_conditions_invalid`; malformed negatives remained closed.

Targeted GREEN suite:

    python3 -m unittest docs.memory.test_matched_evidence_adapter.MatchedEvidenceAdapterTests.test_linux_host_conditions_allow_explicit_empty_frontend_observation docs.memory.test_matched_evidence_adapter.MatchedEvidenceAdapterTests.test_linux_frontend_observation_rejects_malformed_records docs.memory.test_matched_evidence_adapter.MatchedEvidenceAdapterTests.test_linux_empty_frontend_observation_binds_exact_sidecar -v

Result: 3 tests passed.

Full adapter suite:

    python3 -m unittest docs.memory.test_matched_evidence_adapter -q

Result: 49 tests passed, 1 skipped.

The added exact-sidecar test writes the observed JSON, records its SHA-256 in the receipt, binds it through the production reader, and verifies the returned match key and persisted bytes are unchanged. The broader combined adapter/reference-metrics command remains blocked by pre-existing missing diagnostic fixtures outside this task (`input-interior-conservation-mismatch.json` and `low-end-reference-screen-20260906T220129Z/tui_n1_active`).
