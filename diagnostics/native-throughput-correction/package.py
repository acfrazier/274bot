#!/usr/bin/env python3
"""Package complete small timing evidence, excluding executables/process listings."""
import hashlib
import json
from pathlib import Path
import subprocess
import tarfile
root = Path(__file__).resolve().parent
repo = root.parents[1]
archive = root/'raw-results.tar.gz'
paths = sorted(p for role in ('baseline','candidate') for p in (root/role).glob('*.json'))
with tarfile.open(archive, 'w:gz') as tar:
    for p in paths:
        tar.add(p, arcname=str(p.relative_to(root)))
with tarfile.open(archive, 'r:gz') as tar:
    for p in paths:
        assert tar.extractfile(str(p.relative_to(root))).read() == p.read_bytes()
files = ['docs/memory/heaptrack_owner_replay.py', 'docs/memory/heaptrack_owner_runner.py', 'docs/memory/test_heaptrack_owner_replay.py']
source = {}
for name in files:
    data = (repo/name).read_bytes()
    assert data == subprocess.check_output(['git','show','8b4d6f5:'+name], cwd=repo)
    source[name] = hashlib.sha256(data).hexdigest()
manifest = dict(archive_sha256=hashlib.sha256(archive.read_bytes()).hexdigest(),archive_bytes=archive.stat().st_size,
                members={str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in paths},
                unchanged_reference_supervisor_source=source)
(root/'archive-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
print(json.dumps(manifest,indent=2))
