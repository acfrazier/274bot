#!/usr/bin/env python3
import tarfile
from pathlib import Path

conc_arc = Path(
    "diagnostics/nav-stage-a-native-preparation/scheduler-fixture-correction-native-01/coordinate-concord-qualified-03.tar.gz"
)
with tarfile.open(conc_arc, "r:gz") as t:
    names = [m.name for m in t.getmembers() if m.isfile()]
gq = sorted({n.split("/GQ/")[1].split("/")[0] for n in names if "/GQ/" in n})
print("GQ", gq)
comps = [
    n
    for n in names
    if "comparison" in n.lower()
    or "linux-ref" in n.lower()
    or "linux_ref" in n.lower()
    or "linux-reference" in n.lower()
]
print("comp count", len(comps))
for c in comps[:60]:
    print(c)
print("total", len(names))
# also list unique parent dirs under comparisons
dirs = sorted({"/".join(n.split("/")[:-1]) for n in names if "compar" in n.lower()})
print("compar dirs", dirs[:40])
