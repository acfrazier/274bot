# Incremental integration: multi-revision foundation

The operator requested continuous staging, review, merge and publication of
verified work on 2026-09-11. This first batch preserves campaign history through
8385eb85a, then backports only the host profile permission and its test changes
from87084cbc0. The candidate pins client aef3952d1cd7bb3b93d39c497f0f476b68021c59.
The original main base is b2bd5023489ab2e0b6ba690f228217e8984ac91b.

Scope: reconciled274/289 client, immutable process-wide server/resource profile,
profile selection in panel/TUI, revision-aware host writes and session reset,
selected-world navigation manifests and controlled boundary fixtures. The client
also includes the later independently checked inventory/scenery publication fixes
and a mechanical revision-based diagnostic-label change. The complete candidate
requires the independent whole-branch Grok4.6 review before integration.

The later catalog shim capability changes are outside this batch. In particular,
this candidate does not carry the new cake-restocking helper or named bank-opening
JS orchestration now being audited for the thin-shim ownership contract. Existing
catalog behavior and incomplete compatibility are not described as complete.
No Alpha2 release, package, announcement or tag is implied by this integration.

Existing scoped evidence is in01-session-profile.md,02-host-boundary.md,
02a-host-outbound.md,02b-host-snapshot-reset.md and03-world-capabilities.md.
Fresh staging checks and the final review receipt will be recorded here before
publication. Client must be fetchable from FR-client-bothost/r274-bh-modular before
the host gitlink is published; then verify a fresh recursive checkout.

Status: staged; validation and whole-branch review pending. No merge/push yet.
