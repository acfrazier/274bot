# Root review: unresolved inline symbol disclosure

Decision: approve correcting the Python reference and using the corrected
whole-stack symbol disclosure in the native implementation on task `t_98f58617`.
This is an explicit orchestrator review of the defect reported at `698cc6a`.
It implements the already approved whole-original-stack rule; allocation
accounting, canonical normalization and first-useful-frame family selection
must remain unchanged. Final same-card Grok 4.5 review must cover this correction
and its native differential tests together with the complete implementation.

## Independent evidence

Root inspected `classify` in the unchanged Python reference: its initial scan
checks only each IP's primary function, while the later inline scan can be
bypassed by the first useful frame's return. This misses unresolved inline
functions in either the selected owner IP or a caller IP.

Root ran `heaptrack-owner-native/tests/test_reference_contract.py -v` unchanged.
Three cases ran: the primary-caller control passed; the two inline cases produced
four expected failures, in classification and receipt disclosure. Their ten-byte
population, canonical comparison, domain and ownership assertions passed.

Root then ran those same three tests with an in-memory diagnostic wrapper that
computed unknown-symbol coverage over every original IP and each function group,
using `not tables.string(ip[index])` for indices 2, 5, 8, ... and treating missing
primary functions or an empty stack as unknown. All three cases passed. No
production/reference source was edited by this experiment.

## Authorized scope and continuation

Compute full symbol coverage before returning the selected family. Cover absent
function ID zero and empty function strings, including inline groups and caller
IPs. Keep the first useful frame, source anchors and all allocation totals and
canonical stack costs unchanged. Retain the three new regressions; add empty
string and fully resolved controls and exercise them through both engines.
Do not omit unknown-symbol fields from differential comparisons.

This decision resolves the reference-defect pause. Resume the full native
implementation on the same task; the partial investigation is not task completion.
Keep original failed captures/replays and their receipts unchanged. No production
retry, new capture, native deployment or limit increase is released here. The
final code review and Linux qualification gates still apply.
