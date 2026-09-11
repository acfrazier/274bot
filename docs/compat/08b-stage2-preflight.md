# Second integration preflight

Frozen host `b18f28383e4ae462e21fe0b48e167bb46d87595c` and unchanged client
`aef3952d1cd7bb3b93d39c497f0f476b68021c59` underwent workspace checks against
explicit frozen catalog `100adccc`. The full run passed 3,066 tests and failed
three; 42 existing tests remained ignored. Every original log is retained.

The three failures were the isolated export missing relative frozen test inputs,
a catalog test recognizing only the old static dim set, and a persisted-root
fixture unintentionally inheriting the explicit RS2B0T environment. Root supplied
both frozen catalogs, updated the existing dim expectation, and used the existing
thread-local IsolatedEnv for that fixture. No failed test was skipped.

Root commit `fadba2b7ec910fc59bcc102239966f0a87f1706e` also resolves frontend
Clippy type-complexity findings with transparent aliases and formats the panel
rail window. These are mechanical changes. On that exact committed export,
workspace formatting and strict all-targets Clippy passed; all 25 tests in the
two affected catalog suites passed, and the frozen-source check passed. This
combines the original full run with targeted correction checks; it is not a claim
of one fresh all-green workspace run. No new mirrored tests were added.

The isolated source manifests, commands, environment selection, compiler-cache
reuse and logs are under `evidence/stage-2-preflight-b18/`. No client code changed
since the published foundation's separate 1,007-test client qualification.

Reviewed combat observer / fixture correction136 and quest transport137 are in
this source. Root verified Grok4.5 / xai-oauth runs1372 and1373, respectively.
Live binaries for b18 were frozen before the mechanical checks; their host
scenario, API, script runtime and host-play source remain identical to fadba.
The nine-case combat/CoalTrucks batch on b18 was stopped after three cells
exposed an evidence serializer panic; see06an. Its fresh qualification, later
capability work and the required whole-branch review remain separate. Main is
still published through76b2016b7; this preflight is not a merge, complete catalog
acceptance, Alpha2 release or performance result.

## Current candidate checks

Exact324d7b594, code-equivalent to original candidate9b3dc71c0, passed workspace
formatting, strict all-target Clippy, full script517PASS/3existing ignored and
catalog49PASS/1LIVE ignored.5030 source files were reverified. Evidence is in
stage-2-preflight-324. Both native140 and mage132 actualG4.5 reviews approved.
FlaxAIO pick now passes all four cells; both initial mage cells failed pre-Start
on Repeat-wear and remain FAIL. Rootdf2 changes only two existing gear steps
to Perform, preserving waits and generic runner semantics.105 scenario tests
pass on exactdf2; fresh mage/Green qualification is running. Whole-branch
review146 owns source/evidence approval before root integrates the candidate.
This is an incremental milestone, not complete card/option or release acceptance.
