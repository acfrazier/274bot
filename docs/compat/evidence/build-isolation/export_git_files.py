"""Export verified regular Git blobs exclusively into a new source directory."""
import argparse,hashlib,json,pathlib,subprocess
p=argparse.ArgumentParser();p.add_argument('host');p.add_argument('client');p.add_argument('label');p.add_argument('--overlay',action='append',default=[]);a=p.parse_args()
root=pathlib.Path(subprocess.check_output(['git','rev-parse','--show-toplevel'],cwd=pathlib.Path(__file__).parent,text=True).strip());assert a.label and all(c.isalnum() or c in '-_' for c in a.label)
host=subprocess.check_output(['git','rev-parse',a.host],cwd=root,text=True).strip();client=subprocess.check_output(['git','rev-parse',a.client],cwd=root/'vendor/fr-client-rust',text=True).strip()
def entries(repo,ref,prefix=''):
 rows=[]
 for raw in subprocess.check_output(['git','ls-tree','-rz',ref],cwd=repo).split(b'\0'):
  if not raw:continue
  meta,name=raw.split(b'\t',1);mode,kind,oid=meta.decode().split();name=name.decode();parts=pathlib.PurePosixPath(name)
  assert not parts.is_absolute() and all(x not in ('..','.git') for x in parts.parts),name
  if kind=='commit':assert name=='vendor/fr-client-rust' and not prefix;continue
  assert kind=='blob' and mode in ('100644','100755'),(name,kind,mode)
  rows.append((prefix+name,mode,oid))
 return rows
specs=[(root,entries(root,host)),(root/'vendor/fr-client-rust',entries(root/'vendor/fr-client-rust',client,'vendor/fr-client-rust/'))]
overlay={}
for name in a.overlay:
 parts=pathlib.PurePosixPath(name);assert not parts.is_absolute() and all(x not in ('..','.git') for x in parts.parts)
 source=root/name;assert source.is_file() and not source.is_symlink();overlay[name]=source.read_bytes()
dest=root/'.superpowers/review-exports'/a.label;assert not dest.exists();dest.mkdir(parents=True)
files={};modes={}
for repo,rows in specs:
 proc=subprocess.Popen(['git','cat-file','--batch'],cwd=repo,stdin=subprocess.PIPE,stdout=subprocess.PIPE)
 for name,mode,oid in rows:
  proc.stdin.write((oid+'\n').encode());proc.stdin.flush();parts=proc.stdout.readline().decode().split();assert parts[0]==oid and parts[1]=='blob';size=int(parts[2]);buf=bytearray()
  while len(buf)<size:
   chunk=proc.stdout.read(size-len(buf));assert chunk;buf.extend(chunk)
  assert proc.stdout.read(1)==b'\n';data=bytes(buf);assert hashlib.sha1(b'blob '+str(size).encode()+b'\0'+data).hexdigest()==oid
  if name in overlay:data=overlay.pop(name)
  path=dest/name;path.parent.mkdir(parents=True,exist_ok=True)
  with path.open('xb') as out:out.write(data)
  path.chmod(0o755 if mode=='100755' else 0o644);files[name]=hashlib.sha256(data).hexdigest();modes[name]=mode
 proc.stdin.close();assert proc.wait()==0
for name,data in overlay.items():
 path=dest/name;path.parent.mkdir(parents=True,exist_ok=True)
 with path.open('xb') as out:out.write(data)
 files[name]=hashlib.sha256(data).hexdigest();modes[name]='100644'
manifest=dict(host_commit=host,client_commit=client,source_root=str(dest),files=files,overlays={name:files[name] for name in a.overlay},export_method='Exact Git regular blobs; reject symlinks and path traversal; exclusive new files. No archive extraction, overwrite or deletion.')
manifest_path=dest.with_name(dest.name+'-manifest.json');manifest_path.write_text(json.dumps(manifest,indent=2)+'\n');target=dest.with_name(dest.name+'-target');assert not target.exists()
print(json.dumps(dict(host=host,client=client,source=str(dest),target=str(target),target_exists=False,manifest=str(manifest_path),files=len(files))),flush=True)
