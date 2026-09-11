# TUI settings editing on macOS and Concord

Reviewed e72cfb39e/client9d090ed was exported into an exact6779-file package
with the same four-line private IsolatedEnv entry used for prior TUI proof.
New empty Mac and Linux targets produced separate retained binaries. The
Linux binary ran on Concord; Concord did not compile. Both native140x40 PTYs
completed the existing Alcher smoke, exited0 and emitted PASS only after
leaving alternate screen. No runtime errors. Raw source/build/control evidence
is verified by evidence/platform-isolated/harvest_e72.py and summarized in
qualification-e72cfb39.json.

Mac274 numeric Alchs per trip27->3 saved visibly. That run did not Start again
with3, so this proves editing/saving only. Concord289 Items to alch string[]
changed from empty to custom and then rune_chainbody; bad numeric input showed
invalid number, Esc restored27, and closing/reopening kept rune_chainbody.
After closing the form, Start performed one fresh cast:23notes/23nature/
120000coins became22/22/150000, with a fresh2-second paint. Actual mouse/key
inputs were forwarded through the PTY; no product API replaced the controls.

The form clears its own background. Two captures occurred after process exit;
their raw ANSI remains unchanged and normal-screen PASS overlaps pyte's old
frame. Separately labeled before-exit screens decode only bytes before final
LeaveAlternateScreen. Those screens were read for the numeric-save and fresh
Start claims; this is not an active TUI corruption finding.

Metadata remains incomplete independently of the editor: Alcher's imported
ALCH_OPTIONS/DEFAULT_ALCH_ITEMS/CUSTOM_ALCH_KEY do not reach shared SettingDef
fully. Items has no choices and saving custom does not reveal Custom item.
Task t_10ea7213/brief88 audits the shared loader and native-panel implications.
Other setting types, full supported options and final integrated frontends
remain separate acceptance. Prior CAF controls cover Mac289/Concord274; no
claim that these two e72 runs repeat that matrix. Elapsed times are diagnostics,
not performance. Owned Concord289 engine was stopped and both ports refused;
the original Concord service stayed inactive.
