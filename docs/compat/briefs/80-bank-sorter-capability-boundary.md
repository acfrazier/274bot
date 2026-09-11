# Bound the BankSorter capability and ancillary quest reporting

Use grok46 defaults. Bounded read-only design under plan sections6-7 and the
current imported-script/ancillary-stub policy. Own only
docs/compat/04l-bank-sorter-capability-boundary.md and unique
evidence/bank-sorter-capability-boundary/. No code, LIVE, ledgers, remote changes
or subagents. Root owns implementation decisions and acceptance.

BankSorter is one of the frozen45 enabled cards. Its core explicitly requests
sortBank and then stops; current bank_sort.js throws. The imported card also
defaults reportQuestJunk=true and dropQuestJunk=false. bankQuestJunk.findQuestJunk
and bankSortRules.categoryOf are currently unsupported. Inspect both frozen
card/logic sources, the relevant call contracts and native Rust bank/item/menu
operations, selected revision protocol/interface facts and available item data.
Do not recreate or import the foreign sorting rules, quest catalog or planner.

Specify the smallest independently owned Rust bank-reorder capability that can
fulfil the core card with observed server bank ordering changes. Determine
whether existing native APIs already expose item moves/swap/insert mode and
fresh bank order, or which exact public facts/operations are missing. No raw
invented packet, JS sorting policy, deep world copies or mutation before an
explicit script request. Preserve existing bank operations, item identity,
wire ordering, session/Pause/hold/Stop cancellation and deadlines. Explain any
new native policy choice rather than presenting foreign policy as inherited
requirements. No product sorting occurs in this audit.

Separate core sorting from ancillary quest-junk reporting/drop. User clarified
that features needing large foreign scripts or half their world stay honest
stubs until separate native projects supply them; an ancillary stub alone must
not dim an otherwise working card. Determine what can be represented honestly
with the existing caller result shape and what requires a visible unsupported
option/refusal before Start. Returning an empty list as if a junk scan succeeded
or claiming zero successful moves is not a valid substitute for unavailable
work. Destructive drop remains opt-in and must never be enabled by this audit.
Do not confuse a disabled optional report with proof of the default report.

Recommend one coherent bounded implementation task (if in scope) with exact
owned files, source facts, operation/result contracts, meaningful negative
tests and a controlled local LIVE fixture. Use a deliberately unsorted private
bank; seed-only contents or an already-sorted zero-move bank cannot prove the
core. Preserve original inventory/history and report unsupported boundaries
plainly. Flag genuinely broader native feature scope before implementation,
with a concrete explanation of why it exceeds the authorized core capability.
