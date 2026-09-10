# Coordinate CF2 feasibility — failed native admission

CF2 exited1 after57/114 children. No successful CF2 result or acceptance release
exists. The next pre-child bind failed native_preflight with
ValueError('native idle/steal admission'). The guard does not retain that specific
/proc/stat delta, so the evidence cannot distinguish idle below90% from nonzero
steal (or an invalid counter interval). Do not invent a precise host cause.
No child probe failed at this boundary:57 completed/validated children precede
failure while checking admission for the next child. No rerun is authorized by
this failed result; original CF1 and all spent work remain charged.

Raw evidence in diagnostics/nav-stage-a-native-preparation/coordinate-cf2-result-01/:
root-coordinate-cf2-evidence.tar.gz SHA256
45bed027638f73fc9c24c44310a212941174cbb159417669a9e7ce7b9156cc14,
887779bytes/316files, includes immutable prior CF1 and partial CF2, original pack,
all59selectors, both authorizations, failure/claim/checkpoint/logs and all outputs.
Archive is retained locally untracked because it contains the private input pack.

Root streamed all316member hashes and rechecked originalpack/all59selectors,
all61fresh records (CF1four+CF2fifty-seven),24calls per child, exact phase order,
per-row available aggregates, zero narrow allocations, native receipt/caps,
claim/checkpoint/authorization bindings. root-failure-audit.json explicitly has
qualified=false. Unexecuted audit_cf2.py remains the complete-result audit;
audit_cf2_failure.py separately examines partial failed evidence.

CF2 spent397.1195441669988s wall and303.456548s CPU. Cumulative including original
F1 and CF1:478.864614687991s wall/364.395005s CPU/65children. No pending reservation
or unknown charge at failure; stopped=false is a budget property, NOT a successful
continuation token. The57CF2children cover rows3-30 both arms and row31dense only.
Together with CF1,30rows have pairs and row31 is unmatched; no complete59-row
feasibility projection or performance acceptance is justified. CPU component
sampling delta25microseconds retained; waited surplus1.142900032s remains charged.

The process inventory after failure found no stage-a-probe/build/frontend/profiler
processes. It does not reconstruct the missing failure-time CPU sample. No CF2
restart, remaining-slot resume, cap change, CA authorization or performance claim.
Independent failure evidence review should determine whether park/review is the
only currently admitted action; any future method change requires its own review.
