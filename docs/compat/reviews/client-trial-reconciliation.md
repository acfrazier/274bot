# Client integration review trial reconciliation

2026-09-10. Both independent reviews completed before root changed the frozen
candidate. Client input was `8b1a80918f87089e4eb339f1d0e216c1b163348d`, host
input `391726831b3dc97d3614bb8ad8ddb48f75c1e85d`, with identical contract and
manifest SHA-256 `a975118db49f01361b3a085ec4708ff6073ea14bcc3b134b60ac769084b58724`.
The earlier same-card Grok 4.5 review approved this candidate. None of these
reviews is approval of subsequent corrections or the full campaign.

## Submitted results and root decision

| Reviewer | Actual session | Submitted result | Whole run elapsed |
|---|---|---|---:|
| `branchreviewer`, Grok 4.6 / xai-oauth | `20260910_100829_e357e9` | Approve; no blocking findings | 497 s |
| `orch`, GPT-6 Astra / openai-codex | `20260910_100829_8e54b7` | Changes required; one P1 login framing regression | 638 s |

Actual sessions and completed run receipts are `client-whole-grok46.json` and
`client-whole-astra.json`. These elapsed values include report/tool work and
differ from each report's narrower investigation interval. Both encountered
blocked Python tools and used permitted alternatives; Astra additionally had
debugger retries and context recovery. Timing is not a model ranking, and this
one trial does not change the configured reviewer assignments.

Root accepts Astra's P1. A completed short game packet leaves a frame bound on
the reusable input Packet. Login reads an eight-byte seed through that same
Packet without clearing the previous bound. Astra reproduced the panic using
debugger state injection after a passing unmodified control. Root then added
an actual socket regression that receives a one-byte run-energy packet before
cold relogin, reconnect, or socket adoption on each revision. It reproduces the
Packet read assertion before the fix. The correction clears the bound when
the input buffer transitions to the login seed, preserving game-frame checks.

The submitted reviews have no overlapping actionable findings. Grok 4.6's
approval missed this lifecycle defect; it does not override the concrete
failure. Root found no reason to reject Astra's finding. Both reviews agree
on the raw counts (774 baseline, 990 initial candidate, two explicit GPU tests,
441 host tests with seven later live tests ignored), the GPU reconciliation,
and the scoped host fixture correction.

Root additionally confirmed a P2 preservation defect during the full-base
audit: incoming report-abuse handlers ran on 274 as well as 289, completing
previously idle 274 controls. Neither submitted review listed that as a
finding. The current task requires preserving existing incomplete 274 behavior.
Root added revision guards and a regression covering mute and all twelve
reason controls: 274 stays idle, while 289 retains its ordered close/report
frames. The new test also fails on the frozen candidate before correction.

## Corrections and validation

Corrected product: `d14755da758c64d971c1103b2d7703f6fd8379fb`.
Only client login framing, report-control revision guards and their tests
changed. New raw evidence is in client
`docs/revision-289/evidence/bothost-corrections/`. The initial test-helper compile
error is retained separately from the meaningful runtime failures.

The seven affected client suites passed 127 tests, zero failures/ignores, after
the correction. Formatting and strict all-target Clippy pass. The final full
client suite passed 992 tests with zero failures and two ignored GPU tests;
both explicit GPU tests then passed on the same committed product. The reviewed
artifact candidate is `58120f28ee5208553ca07f41cb364f2cf98ea280`, whose last
commit adds only report/evidence.
The frozen trial reports and manifest remain evidence of the earlier candidate;
use their named commits when resolving a file changed by the correction.

## Corrective review closure

Same-card corrective review `t_706ce8a5`, run 1084, completed with approval
under actual Grok 4.5 / xai-oauth, session `20260910_103233_3506ce`.
Fresh whole-client follow-up `t_4bed943f`, run 1085, then completed with approval
under actual Grok 4.6 / xai-oauth, session `20260910_103533_55a2fa`.
Both reviewed candidate `58120f28` / product `d14755da`; neither edited product
code, and both verified the meaningful red/green evidence. Completed run/model
receipts are `client-corrections-grok45.json` and
`client-corrected-whole-grok46.json`. The final report is
`client-grok46-corrected.md`.

Root closes both confirmed defects for this client source/regression milestone.
The final host dependency check also passed 441 tests, zero failures, seven
live tests ignored against the corrected client. The corrective manifest names
host `80552d3c`, so its reconciliation hash resolves to that frozen version of
this document; this closure section was added after the review completed.

Review scope lists distinguish unqualified work from approved behavior. The
plan remains authoritative: full 377, mixed fleets, tutorial/audio parity and
public live qualification are outside this campaign, rather than new gates
introduced by a review's list of unapproved areas.

No live or platform gate was satisfied by these offline corrections. Host
profile binding and the remaining campaign work are still pending.
