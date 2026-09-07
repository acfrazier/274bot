# N16 server provenance recovery

## Result

The future Windows preflight producer now serializes a checked `server` record in
`native_preflight`. This is a producer-side preservation fix; the existing managed
runner and evidence reader behavior were not broadened or otherwise weakened.
Legacy non-native host condition fixtures retain their prior behavior.

`n16_server_provenance_recovery.py` creates a new conditions artifact only. It
matches exactly one original `native_preflight.processes` row by:

- server-identity PID;
- validated process name (`node.exe` by default);
- validated session (`2` by default); and
- WMI `CreationDate` at millisecond precision against the Windows creation
  FILETIME identity.

WMI `/Date(milliseconds)/` cannot represent the identity's final 100-ns digits.
The utility therefore compares the millisecond portions and records that
precision limitation rather than pretending the row is an exact 100-ns match.
Missing, duplicate, wrong-name/session, malformed, or mismatched rows fail
closed. The output records `status: reconstructed`, `not_original_artifact:
true`, the match method, and SHA-256 hashes of the preflight, server identity,
and original host-conditions source artifacts. Existing artifacts are opened
read-only and the output is exclusive-create.

## Existing receipt binding

The recovered artifact is diagnostic-only and is not an alternate input to
`matched_evidence_adapter.bind_side`. The original strict reader still requires
the receipt's original `host_conditions_path` and hash, including the original
`server` observation. Because those receipts omit that original field, full
original receipt binding remains unavailable for the affected N16 cells. The
reader schema, receipt hashes, and failed bindings were not changed; no
derived-host acceptance path was added.

The original focused-one and focused-plus-background receipts, failed bindings,
qualification artifacts, and source archives remain unchanged. Raw metrics can
still be inspected diagnostically; no performance acceptance is asserted.

## Tests

`test_n16_server_provenance_recovery.py` covers FILETIME conversion, the
millisecond-vs-100-ns precision boundary, unique matching, absent/duplicate/
wrong-process rejection, source hash stability, non-replacement, and
reconstruction labeling.

The existing managed-cell and evidence-reader suites were also run. No native,
network, live, fixture, Rust, client, or 289 operation was performed here.

The named native qualifier protocol now includes the actual `--diagnostics`
flag; a default/no-sidecar run must not be described as profile-enabled
diagnostics. `performance_acceptance` remains false.
