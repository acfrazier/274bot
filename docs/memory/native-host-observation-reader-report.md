Native host observation reader report

Scope

This is a read-only validation repair for native Windows host-condition sidecars. No raw artifact, receipt, live workload, or production Rust source was changed.

Finding

The reader previously applied _deep_missing to the complete host-condition document. That treated provider-reported optional nulls (for example dxdiag monitorName/currentMode/hybridGraphicsGPU and WMI process CommandLine) and an observed empty ProcessLasso process list as missing evidence. The original native rejection remains preserved in its existing artifact and is not rewritten.

Repair

matched_evidence_adapter.py now recognizes one bounded native Windows observation shape and fails closed whenever its native marker is present. It requires non-empty platform/user identity and purpose, explicit boolean control fields, native preflight adapter/time/session, VM state, quiet-service records, driver identity records, session text, process record types, ProcessLasso consistency and file hashes, display card/driver identity, server identity, and explicit performance-acceptance booleans. It permits only the documented optional null display fields and process CommandLine values, and permits empty lists only where the frozen observation legitimately reports them. Required null, missing, empty, wrong-type, inconsistent, or malformed identity/control records remain invalid. Legacy host-condition documents retain the recursive strict validator.

The same bounded rule is used when checking the host_conditions match key. All other match keys retain the recursive strict validator. Sidecar path binding, recorded SHA-256 verification, post-read rechecks, raw hashes, manifest/build provenance, and pair eligibility logic are unchanged. A bound side is not a pair-eligible or performance-accepted result.

Verification

Mac targeted suite:

    python3 -m unittest docs.memory.test_matched_evidence_adapter -q

Result: 42 tests passed.

Coverage added for the representative native shape with running=false and empty process lists, nullable optional display/process fields, missing/null machine identity and controls, and read-only host-sidecar hash/provenance preservation. Existing tests continue to cover null required match keys, nested required settings, sidecar hash tampering, manifest/path binding, and unchanged required match-key behavior.

Native replay instructions for root

Do not modify the preserved raw directory or receipt. Run as root only after same-card Grok4.5 approval. Use the exact source/dependency files from this checkout:

    docs/memory/matched_evidence_adapter.py
    docs/memory/test_matched_evidence_adapter.py
    docs/memory/qualify_control.py
    docs/memory/reference_metrics.py
    docs/memory/run_diagnostic.py
    docs/memory/build_provenance.py
    docs/memory/managed_resource_binding.py
    docs/memory/diagnostics/windows-explicit-adapter-20260907/nvidia/managed-panel-36825a9-console-nvidia/

Replay the preserved NVIDIA console receipt with the reader's existing CLI/API, supplying its recorded manifest and server-identity sidecar paths exactly as recorded. Verify the returned host-condition result is bound without changing the raw sidecar or its SHA-256. Repeat independently for sibling20260907T212252Z_panel_n1_active when its preserved path and dependency files are available. Record the original rejection as historical evidence; do not convert binding into pair eligibility, quiet-host proof, or a performance claim. Do not SSH, rebuild Windows, or run the application as part of this reader task.

Provenance

The preserved NVIDIA archive SHA-256 is d1aaba9be74ed85b6126dbfe802d1bc2dafa7cb0f95150e8ed60fc91429eb611. This report does not alter that archive or any receipt.
