# N16 server provenance recovery

## Result

The future Windows preflight producer now serializes a checked `server` record in
`native_preflight`. The managed runner requires that record to be present for a
native preflight and checks PID, `node.exe`, and console session 2 against the
explicit server PID before launching a cell. Legacy non-native host condition
fixtures retain their prior behavior.

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

`matched_evidence_adapter.bind_side` accepts an optional
`derived_host_conditions_path`. It still requires and independently verifies the
receipt's original `host_conditions_path` and hash, then validates and uses the
separate derived artifact for the host match key. It rechecks both source files
after analysis and reports derived-artifact provenance in the returned match
keys. This does not amend the receipt or its original hash and does not turn a
reconstruction into original evidence. Callers that omit the optional argument
retain the original strict behavior.

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
