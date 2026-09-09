# Direct-owner controller native qualification

Root accepts the same-card Grok 4.5 review of controller `07b5b29` (round 2,
actual xai-oauth session `20260909_180340_77a5ff`). The correction adds generated
proof of direct collection through Stop/C/launcher exit and default collection
ending after its unchanged pad. The initial failed macOS and container logs remain
preserved. Review approval does not turn those failures into passes.

Root independently verified all seven final reported log hashes and sizes, then
exported 69 committed Python source/dependency files from exactly `07b5b29`.
Native Linux x86_64 builder execution used Python 3.12.3 and Git 2.43.0 in
`/home/builder/direct-owner-controller-07b5b29-native-01`. No private fixtures,
live account, game server, runtime launch, or Rust build was used.

The complete five-module unittest command passed: 142 tests, zero failures/errors,
two native-macOS-only skips, no exclusions. All Linux signal-mask, SIGINT,
pending-parent-stop, collector-lifecycle, and real-Git tests executed. The existing
owner validator separately passed three tests. Both commands exited zero without
timeout; their owned process groups were absent. All 69 source hashes were checked
before and after. Root downloaded and independently checked both raw log hashes
and byte counts against the native receipt.

Evidence: `diagnostics/direct-owner-managed-extension/root-native-qualification-01/`
contains exact command/platform/result, source manifest, logs, launcher, and root
audit. The transfer archive remains local alongside this evidence.

This proves native generated controller behavior only. Exact runtime manifest and private admission, fresh live preflight, live owner
capture, measured comparisons, remaining campaign gates, and final whole-branch
Grok 4.6 review remain required. No memory saving or live release is claimed.

## Windows import and contract check

The same 69 files from reviewed `07b5b29` were hash-verified before and after on
Windows 11 x64, Python 3.14.6. All five production modules imported successfully.
Thirteen explicitly selected existing pure argv/environment/default-contract tests
passed with no skips or exclusions from that named selection. Root downloaded and
verified the 3,494-byte log, SHA256
`ce860ff240321b751070e445b16ab013a10fe609335c51ae8fcbc749b0ed8fd1`.
The launcher, exact selection, result and raw log are alongside the Linux evidence
as `run_windows_contract.py`, `windows-result.json` and `windows-contract.log`.
This was a bounded import/contract check, not the full Windows suite, UI execution,
process-lifecycle proof or live qualification. No application settings were changed.
