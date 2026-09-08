# Fixed-window cohort reader report

Implemented the offline reader in `docs/memory/cohort_reader.py` and wired additive `decode_cohort` and `input_cohort` gates into `reference_metrics.analyze_run` and the existing `--require` selector. Legacy gates and `final_acceptance_claim: false` remain unchanged; archives without the publisher sidecar report `cohort_schema_missing` while retaining legacy results.

Reader contract

- Reads only the run-bound `samples.cohort.jsonl`; malformed, truncated, header-only, unknown-schema, missing-terminal, provenance-mismatched, and failed-accounting inputs are unavailable.
- Requires schema 1, `responsiveness_process_mono`, exact immutable boundaries, `DEFAULT_TAIL_NS`, matching frontend, and exact derived sidecar path. Starts use `[start,end)`; completion may equal `end + tail` and must not precede its start.
- Keeps EventId sequence separate from append cursor, permits completion/start records in either order, rejects duplicate identities and cursor/counter regressions, and preserves loss/overflow/unavailable status.
- Enforces declared populations: decode requires all declared slot identities by count; panel focused-one requires its declared slot set; TUI is a distinct endpoint. Empty or partial populations cannot pass.
- Calculates durations in nanoseconds, then independently bins coarse `[5,10,20,25,40,50,100,250,500,1000]ms` and fine `1..100ms` boundaries. Exact 100ms is in-range; values above 100ms are overflow with no invented upper bound. The fine p99 bound determines the <=100ms cohort verdict.

CLI example:

`python3 docs/memory/reference_metrics.py RUN --require decode_cohort,input_cohort`

Verification

- `python3 -m unittest discover -s docs/memory -p 'test_cohort_reader.py' -v` — 3 tests passed.
- `python3 -m py_compile docs/memory/cohort_reader.py docs/memory/reference_metrics.py docs/memory/test_cohort_reader.py` — passed.
- Existing archived samples in this checkout do not contain publisher cohort sidecars; no native or live fixture was fabricated. Full legacy suite and matched-evidence/instrumentation commands remain to be run by the root campaign checkout as required by the parent card.
