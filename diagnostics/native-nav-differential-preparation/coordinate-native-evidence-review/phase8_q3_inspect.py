#!/usr/bin/env python3
import json
import tarfile
from pathlib import Path

out = Path("diagnostics/native-nav-differential-preparation/coordinate-native-evidence-review")
conc_arc = Path(
    "diagnostics/nav-stage-a-native-preparation/scheduler-fixture-correction-native-01/coordinate-concord-qualified-03.tar.gz"
)

with tarfile.open(conc_arc, "r:gz") as tf:
    q3 = json.loads(
        tf.extractfile(
            "host/docs/memory/nav-tiled-stage-a/coordinate-concord-run-01/coordinate-ebf0f30-qualification-03/result.json"
        ).read()
    )
    print("qualified", q3.get("qualified"))
    print("native_hard_as", q3.get("native_hard_as_qualified"))
    print("generated_smoke_executed", q3.get("generated_smoke_executed"))
    print("scope", q3.get("scope"))
    print("schema", q3.get("schema"))
    print("guards keys", q3.get("guards"))
    print("smoke", json.dumps(q3.get("smoke"), indent=2)[:2000])
    print("admissions", json.dumps(q3.get("admissions"), indent=2)[:2000])
    print("helper_spans", q3.get("helper_spans"))
    print("source_binding", q3.get("source_binding"))
    comps = q3["comparisons"]
    print("N comps", len(comps))
    for c in comps:
        print(
            f"{c.get('variant')}/{c.get('fixture')}/{c.get('arm')} ref={c.get('reference_arm')} "
            f"raw={c.get('raw_calls')} new==old={c.get('new_sha256')==c.get('old_sha256')} "
            f"new={c.get('new_sha256')[:12]} old={c.get('old_sha256')[:12]}"
        )

    # look for comparison detail files
    names = [m.name for m in tf.getmembers() if m.isfile()]
    detail = [n for n in names if "compar" in n.lower() or "linux" in n.lower()]
    print("detail files", len(detail))
    for n in detail[:30]:
        print(n)

    # scheduler-guards.out tail for 29 tests
    for path in [
        "host/docs/memory/nav-tiled-stage-a/coordinate-concord-run-01/coordinate-ebf0f30-qualification-03/scheduler-guards.out",
        "host/docs/memory/nav-tiled-stage-a/coordinate-concord-run-01/coordinate-ebf0f30-qualification-03/scheduler-guards.err",
    ]:
        try:
            data = tf.extractfile(path).read().decode("utf-8", "replace")
            print("====", path, "len", len(data))
            print(data[-2500:])
        except Exception as e:
            print("missing", path, e)

    # GQ structure - files are flat under GQ/ not GQ/id/
    gq_files = [n for n in names if "/GQ/" in n]
    print("gq file count", len(gq_files))
    # unique child prefixes
    prefixes = sorted(
        {
            n.split("/GQ/")[1].rsplit(".", 1)[0]
            for n in gq_files
            if n.split("/GQ/")[1][0].isdigit()
        }
    )
    # better: 000-1-1-dense as stem before .err/.out
    import re

    stems = sorted(
        {
            re.match(r"(\d{3}-\d+-\d+-(?:dense|refined))", n.split("/GQ/")[1]).group(1)
            for n in gq_files
            if re.match(r"(\d{3}-\d+-\d+-(?:dense|refined))", n.split("/GQ/")[1] or "")
        }
    )
    print("gq stems", stems)

(out / "phase8-q3-dump.json").write_text(
    json.dumps(
        {
            "qualified": q3.get("qualified"),
            "comparisons": comps,
            "smoke": q3.get("smoke"),
            "guards": q3.get("guards"),
            "admissions": q3.get("admissions"),
            "native_hard_as_qualified": q3.get("native_hard_as_qualified"),
            "generated_smoke_executed": q3.get("generated_smoke_executed"),
            "helper_spans": q3.get("helper_spans"),
            "source_binding": q3.get("source_binding"),
            "gq_stems": stems,
        },
        indent=2,
    )
    + "\n"
)
