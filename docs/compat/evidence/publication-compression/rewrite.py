import subprocess as sp, gzip, hashlib, json
from pathlib import Path
from functools import lru_cache

repo = '/Users/acfrazier/experiments/274bot'
out = Path(__file__).parent
def git(*args, data=None):
    return sp.check_output(['git', '-C', repo, *args], input=data)
base = git('rev-parse', 'origin/main').decode().strip()
old = git('rev-parse', 'main').decode().strip()
assert base == '76b2016b7dafe9aae7b0c33dd591d8a9907380cb'
assert old == '64e917a5441d32b4eddaacef5567b906e70a0fd5'
git('update-ref', 'refs/heads/codex/stage2-before-compression', old)
replacements = {}
records = []
for oid in ['a487ff9ba54845282d17b35f13bdee678b737da3', 'e5989ae398ebd82f243c2e49e77513dabff8e4df']:
    raw = git('cat-file', 'blob', oid)
    packed = gzip.compress(raw, compresslevel=9, mtime=0)
    assert gzip.decompress(packed) == raw
    new = git('hash-object', '-w', '--stdin', data=packed).decode().strip()
    replacements[oid] = new
    records.append(dict(original_blob=oid, compressed_blob=new, original_bytes=len(raw), compressed_bytes=len(packed), original_sha256=hashlib.sha256(raw).hexdigest(), compressed_sha256=hashlib.sha256(packed).hexdigest()))

@lru_cache(None)
def tree(oid):
    entries = git('ls-tree', '-z', oid).split(b'\0')
    changed = False
    result = []
    for entry in entries:
        if not entry:
            continue
        metadata, name = entry.split(b'\t', 1)
        mode, kind, child = metadata.split(b' ')
        child = child.decode()
        updated = tree(child) if kind == b'tree' else replacements.get(child, child)
        if updated != child:
            changed = True
            if kind == b'blob':
                assert name == b'samples.diagnostics.jsonl'
                name += b'.gz'
        result.append(mode + b' ' + kind + b' ' + updated.encode() + b'\t' + name + b'\0')
    return git('mktree', '-z', data=b''.join(result)).decode().strip() if changed else oid

mapping = {}
for commit in git('rev-list', '--reverse', '--topo-order', f'{base}..{old}').decode().splitlines():
    raw = git('cat-file', 'commit', commit)
    headers, body = raw.split(b'\n\n', 1)
    lines = []
    for line in headers.split(b'\n'):
        if line.startswith(b'tree '):
            line = b'tree ' + tree(line[5:].decode()).encode()
        elif line.startswith(b'parent '):
            parent = line[7:].decode()
            line = b'parent ' + mapping.get(parent, parent).encode()
        assert not line.startswith(b'gpgsig ')
        lines.append(line)
    updated = b'\n'.join(lines) + b'\n\n' + body
    mapping[commit] = git('hash-object', '-t', 'commit', '-w', '--stdin', data=updated).decode().strip()
new = mapping[old]
git('merge-base', '--is-ancestor', base, new)
git('update-ref', 'refs/heads/codex/stage2-compressed', new)
(out / 'receipt.json').write_text(json.dumps(dict(base=base, old=old, new=new, blobs=records, commit_mapping=mapping, preserved_author_committer_message_and_topology=True), indent=2)+'\n')
print(json.dumps(dict(old=old, new=new, commits=len(mapping), blobs=records), indent=2))
