"""Summarize retained receipts only; no probes or production input."""
import json
from pathlib import Path
here=Path(__file__).resolve().parent
print('Real two-pass cell output upper bound:',2*70_000_000*15)
print('Initial build failure suffix:',(here/'run-01/dense-build.err').read_text()[-400:])
a=json.loads((here/'run-03/dense-admission.json').read_text())
print('Frozen toolchain:',json.dumps(a['toolchain'],indent=2))
for name in ('dense-build-0.receipt.json','tiled-build-0.receipt.json'):
    r=json.loads((here/'run-03'/name).read_text());print(name,json.dumps(r))
