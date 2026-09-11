import subprocess as sp,json,gzip,hashlib
from pathlib import Path
from functools import lru_cache
out=Path(__file__).resolve().parent
repo=out.parents[1]
def git(*args,data=None):return sp.check_output(['git','-C',str(repo),*args],input=data)
r=json.loads((out/'receipt.json').read_text());mapping=r['commit_mapping'];replacements={v['original_blob']:v['compressed_blob'] for v in r['blobs']}
old=git('rev-parse','HEAD').decode().strip();assert git('branch','--show-current').decode().strip()=='codex/rs2b0t-multirevision'
git('update-ref','refs/heads/codex/campaign-before-compression',old)
@lru_cache(None)
def tree(oid):
 entries=[];changed=False
 for row in git('ls-tree','-z',oid).split(b'\0'):
  if not row:continue
  meta,name=row.split(b'\t',1);mode,kind,child=meta.split();child=child.decode();updated=tree(child) if kind==b'tree' else replacements.get(child,child)
  if updated!=child:
   changed=True
   if kind==b'blob':assert name==b'samples.diagnostics.jsonl';name+=b'.gz'
  entries.append(mode+b' '+kind+b' '+updated.encode()+b'\t'+name+b'\0')
 return git('mktree','-z',data=b''.join(entries)).decode().strip() if changed else oid
for commit in git('rev-list','--reverse','--topo-order',f"{r['base']}..{old}").decode().splitlines():
 if commit in mapping:continue
 raw=git('cat-file','commit',commit);headers,body=raw.split(b'\n\n',1);lines=[]
 for line in headers.split(b'\n'):
  if line.startswith(b'tree '):line=b'tree '+tree(line[5:].decode()).encode()
  elif line.startswith(b'parent '):parent=line[7:].decode();line=b'parent '+mapping.get(parent,parent).encode()
  assert not line.startswith(b'gpgsig ')
  lines.append(line)
 mapping[commit]=git('hash-object','-t','commit','-w','--stdin',data=b'\n'.join(lines)+b'\n\n'+body).decode().strip()
new=mapping[old]
paths=git('diff','--name-only',old,new).decode().splitlines()
assert len(paths)==4 and all('/fleet-prerequisite/n32-r' in p and p.endswith(('samples.diagnostics.jsonl','samples.diagnostics.jsonl.gz')) for p in paths)
assert not git('diff','--cached','--name-only')
for p in paths:
 if not p.endswith('.gz'):assert git('hash-object',p).decode().strip() in replacements
(out/'campaign-mapping.json').write_text(json.dumps(dict(old=old,new=new,commit_mapping=mapping),indent=2)+'\n')
# Compare-and-swap preserves any concurrent commit rather than overwriting it.
git('update-ref','refs/heads/codex/rs2b0t-multirevision',new,old)
for p in paths:
 if p.endswith('.gz'):
  blob=git('show',f'{new}:{p}');(repo/p).write_bytes(blob)
  oid=git('rev-parse',f'{new}:{p}').decode().strip();git('update-index','--add','--cacheinfo',f'100644,{oid},{p}')
 else:
  git('update-index','--force-remove',p);(repo/p).unlink()
print(json.dumps(dict(old=old,new=new,paths=paths)))
