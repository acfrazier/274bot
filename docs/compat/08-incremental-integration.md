# Incremental integration: multi-revision foundation

The operator requested continuous staging, review, merge and publication of
verified work on 2026-09-11. This first batch preserves campaign history through
8385eb85a, backports only the profile permission/test changes from 87084cbc0,
and includes the historical fixture corrections f52565f02, 6b925583e and
43a57c368. Original main base: b2bd5023489ab2e0b6ba690f228217e8984ac91b.
Final code candidate: 940531c3f7a4fe392f8ee5688702fe1f617d28b0.
Client: aef3952d1cd7bb3b93d39c497f0f476b68021c59.

The batch includes the reconciled 274/289 client, immutable process-wide server
and resource profile, panel/TUI profile selection, revision-aware host writes,
session reset and selected-world navigation foundation. The client also contains
later reviewed inventory/scenery publication fixes and a mechanical revision-based
diagnostic label change. Historical scoped reports are 01-session-profile.md,
02-host-boundary.md,02a-host-outbound.md,02b-host-snapshot-reset.md and
03-world-capabilities.md; their old next-actions are evidence snapshots.

Later catalog shim capabilities are outside this batch, including the new cake
helper and bank opening/autocast sequencing being corrected to native ownership.
Existing incomplete catalog behavior is not described as complete. This is not
Alpha 2/0.1.7 release acceptance, platform qualification or a performance result.

## Fresh verification

Formatting and strict Clippy passed. Workspace checks with memory-profile passed
2603 tests after explicitly excluding two unchanged external-catalog failures.
The separate client integration invocation passed 1007 tests (two ignored).
Both failing external-catalog tests were reproduced on exact main b2bd5023 and
client 9b41e6e0 against the same operator catalog 42011fb1:

- catalog_start: missing STAFF_RUNES in Alcher, ClimbingBoots and Superheater.
- declared_abi: the committed ABI fixture differs from that newer local catalog.

All original failures, baseline assertion comparisons, commands and logs remain
in evidence/stage-1. No test was changed or silently disabled. The passing
workspace/client checks used source 65938065/client aef3952d. The only subsequent
code changes are the three fixture corrections in world_boundary_live.rs;
focused formatting and strict Clippy passed again.

Eight controlled live cells passed: session boundary, cross-square navigation,
door traversal and Guardian lamp on each of 274/289. They show real walking,
NPC dialogue, world change, logout clearing; door opening and inside arrival;
and lamp consumption/XP followed by resumed walking. The first 274 door run
failed because the early batch omitted the later Catherby preparation fix.
That failed run is retained. Corrected door/lamp cells passed on both revisions;
289 boundary/navigation also ran after the correction. Unaffected 274 boundary
and navigation results are retained from 65938065. The exact source/proof mapping
and verified log hashes are in evidence/stage-1/validation-ready.json.

## Integration gate

Whole-branch review approved this exact code/client batch in t_fd3a8b47,
actual Grok 4.6 / xai-oauth run 1346 (30 verified API calls). Report:
08a-stage1-whole-branch-review.md. No material findings. Root is proceeding
with client/staging-branch publication and fresh recursive clone verification
before merging main. The final full-campaign review remains separate.

Publish the client to acfrazier/FR-client-bothost r274-bh-modular
before publishing the host gitlink. Verify a fresh recursive checkout, merge
ordinary history into host main and push acfrazier/274bot. Preserve unrelated
primary checkout files and active campaign work. No tag/package/announcement.

The client and staging branch are published. A fresh recursive GitHub clone
of cb9d14e83 fetched clientaef3952d with no local object alternates;
`cargo check --workspace --all-targets --features memory-profile --locked`
passed. Receipt: evidence/stage-1/fresh-remote-check.json. Only documentation
and this evidence have changed since that clone. Main integration follows.
