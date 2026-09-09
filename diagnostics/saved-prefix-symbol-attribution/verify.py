"""Execute the bounded audit/tests and retain real output and repeatability proof."""
import hashlib
import json
import subprocess
import sys
from pathlib import Path

OUT = Path(__file__).resolve().parent
ROOT = OUT.parents[1]
commands = ['test_audit.py', 'audit.py', 'summarize.py']
runs = []
for name in commands:
    argv = [sys.executable, str(OUT/name)]
    p = subprocess.run(argv, cwd=ROOT, text=True, capture_output=True, timeout=60)
    runs.append(dict(argv=argv, exit_code=p.returncode, stdout=p.stdout, stderr=p.stderr))
    assert p.returncode == 0, runs[-1]
outputs = ['audit.json', 'mapping.json', 'populations.json', 'leading-chains.json', 'source-identities.json']
def hashes():
    return {n: hashlib.sha256((OUT/n).read_bytes()).hexdigest() for n in outputs}
first = hashes()
for name in commands[1:]:
    p = subprocess.run([sys.executable, str(OUT/name)], cwd=ROOT, text=True, capture_output=True, timeout=60)
    assert p.returncode == 0, p.stderr
assert first == hashes()
a = json.loads((OUT/'audit.json').read_text())
r = dict(status='PASS', commands=runs, repeated_outputs_identical=True, outputs_sha256=first,
         prior_attempts=['execute_code unavailable in headless approval mode; no code ran',
                         'python -c inspection blocked by approval mode; no code ran',
                         'first audit failed JSON parsing because TSV JSON quoting was treated as CSV quoting; corrected to QUOTE_NONE before successful audit'],
         arithmetic=dict(nav_decode_peak_minus_last=a['cutoffs']['peak']['groups'][0]['bytes']-a['cutoffs']['last_mark']['groups'][0]['bytes'],
                         eof_minus_last_bytes=a['cutoffs']['eof']['bytes']-a['cutoffs']['last_mark']['bytes'],
                         eof_minus_last_count=a['cutoffs']['eof']['count']-a['cutoffs']['last_mark']['count'],
                         trace_3865_plus_3867_bytes=65142784+8142848),
         capture_complete=False, acceptance=False)
(OUT/'verification.json').write_text(json.dumps(r, indent=2)+'\n')
print(json.dumps({k:v for k,v in r.items() if k not in ('commands','prior_attempts')}, indent=2))
