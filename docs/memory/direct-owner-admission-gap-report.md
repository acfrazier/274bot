# Root finding: admission contents are not validated

Root withholds live use of controller `07b5b29` despite its reviewed implementation
and passing native generated tests. In `run_managed_cell.preflight`, each of the
five receipts is checked only for regular-file/nonempty-object form and stable
hash. Its contents are never checked for admission, identity, freshness or conflicts.
`run_current_tui_calibration.preflight` adds only existence checks.

This contradicts approved design section 6 and its section 11 rejection cases.
The existing boundary test supplies only `fixture_kind`, so passing that test does
not establish semantic admission validation.

Root called the real preflight using generated fixture processes/files, mocking
only source-build verification, sampling and host counters to isolate the receipt
logic. Four cases replaced all five receipt bodies: arbitrary nonempty objects,
expired timestamps, explicitly rejected admission, and wrong server identity with
a conflicting process. All four were incorrectly accepted. No launcher or live
fixture was used. Raw result is in
`diagnostics/direct-owner-managed-extension/root-admission-gap-01/result.json`.

Corrective implementation task `t_5c2087e1` uses Sol defaults and the same-card
Grok 4.5 review flow. It must add explicit typed and root-bound receipt schemas,
freshness/identity/conflict validation and meaningful negative coverage without
changing defaults or weakening lifecycle limits. Root must then qualify the final
controller natively before preparing actual private admission or launching live.
Existing native test results and runtime build-file verification remain factual
but do not close this gap or authorize live work.
