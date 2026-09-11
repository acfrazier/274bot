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
diagnostic label change. Historical scoped reports are01-session-profile.md,
02-host-boundary.md,02a-host-outbound.md,02b-host-snapshot-reset.md and
03-world-capabilities.md; their old next-actions are evidence snapshots.

Later catalog shim capabilities are outside this batch, including the new cake
helper and bank opening/autocast sequencing being corrected to native ownership.
Existing incomplete catalog behavior is not described as complete. This is not
Alpha2/0.1.7 release acceptance, platform qualification or a performance result.

## Fresh verification

Formatting and strict Clippy passed. Workspace checks with memory-profile passed
2603 tests after explicitly excluding two unchanged external-catalog failures.
The separate client integration invocation passed1007 tests (two ignored).
Both failing external-catalog tests were reproduced on exact mainb2bd5023 and
client9b41e6e0 against the same operator catalog42011fb1:

- catalog_start: missing STAFF_RUNES in Alcher, ClimbingBoots and Superheater.
- declared_abi: the committed ABI fixture differs from that newer local catalog.

All original failures, baseline assertion comparisons, commands and logs remain
in evidence/stage-1. No test was changed or silently disabled. The passing
workspace/client checks used source65938065/clientaef3952d. The only subsequent
code changes are the three fixture corrections in world_boundary_live.rs;
focused formatting and strict Clippy passed again.

Eight controlled live cells passed: session boundary, cross-square navigation,
door traversal and Guardian lamp on each of274/289. They show real walking,
NPC dialogue, world change, logout clearing; door opening and inside arrival;
and lamp consumption/XP followed by resumed walking. The first274 door run
failed because the early batch omitted the later Catherby preparation fix.
That failed run is retained. Corrected door/lamp cells passed on both revisions;
289 boundary/navigation also ran after the correction. Unaffected274 boundary
and navigation results are retained from65938065. The exact source/proof mapping
and verified log hashes are in evidence/stage-1/validation-ready.json.

## Integration gate

Whole-branch review is running as t_fd3a8b47, profile branchreviewer/Grok4.6,
against exact code940531c3f and clientaef3952d. Approval is pending; creation or
an assignee label is not a verdict. Root must verify the actual model and final
findings before publication. No merge or push yet.

After approval, publish the client to acfrazier/FR-client-bothost r274-bh-modular
before publishing the host gitlink. Verify a fresh recursive checkout, merge
ordinary history into host main and push acfrazier/274bot. Preserve unrelated
primary checkout files and active campaign work. No tag/package/announcement.
