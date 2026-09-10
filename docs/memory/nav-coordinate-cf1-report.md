# Coordinate CF1 native feasibility result

Four children completed on Concord with exit0 under authorization62835390.
This is feasibility evidence only; CF2/CA and performance acceptance remain closed.

Raw evidence: diagnostics/nav-stage-a-native-preparation/coordinate-cf1-result-01/.
Archive SHA2566b6314fe9e60235ab92feeb65f972921104c66bccbc5fabcf385e4e28f576cdf,
834734bytes,82files. Root audited every member/hash, original pack and all59
selectors, all4records/output receipts,24raw calls per child, per-row aggregate
agreement, zero narrow allocations and unchanged hard address-space guard/caps.

Fresh wall41.6546222899924s and CPU31.437117s. Cumulative ledger includes original
4children and fresh4: wall81.74507052099216s, CPU60.938457s,8children. No unknown
charge or pending reservations. Approved122-child/1800wall/1500CPU budget remains.
Peak process RSS observations: dense149553152B, refined153616384B on both rows;
not enough to claim a saving or evaluate the whole59-route workload.

Audit corrections are retained in audit-normalization-correction.txt: the first
selector comparison used tabs incorrectly; corrected to reviewed integer/space
normalization. Exact CPU component equality was inappropriate because snapshot
samples its clocks separately; actual difference is16microseconds. A 1ms audit
comparison tolerance changes no production budget. Waited-child total exceeds
sum of four per-probe child CPU records by0.120699825s; require independent review
of accounting before continuation, not an assertion that both gauges are equal.

Prerequisite source/native/correctness reviews are b62f173 and f13d682. Source
binding7b101c30; scheduler tool113ce05; original40+19selectors49e348ea. Root issued
only CF1 after fresh97.99%idle/zeroSteal/no swap/conflicts. No benchmark retry.
