"""Bounded read-only inspection of exported tables and frozen git source."""
import csv
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if len(sys.argv) > 1:
    repo, commit, path = sys.argv[1:4]
    text = subprocess.check_output(['git', '-C', str(ROOT / repo), 'show', f'{commit}:{path}'], text=True)
    start, end = map(int, sys.argv[4:6]) if len(sys.argv) > 4 else (1, 200)
    for n, line in enumerate(text.splitlines(), 1):
        if start <= n <= end:
            print(f'{n}|{line}')
else:
    for directory in ['root-native-production-a55972a/production-replay-1/result', 'root-native-symbols-a55972a']:
        for p in sorted((ROOT / 'diagnostics' / directory).glob('*')):
            if p.suffix not in ('.json', '.tsv'):
                continue
            print(p.name, p.stat().st_size)
            if p.suffix == '.json':
                obj = json.loads(p.read_text())
                print(str(obj)[:900])
            else:
                print(p.read_text().splitlines()[:3])
