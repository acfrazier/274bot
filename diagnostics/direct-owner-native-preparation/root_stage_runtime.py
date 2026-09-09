"""Materialize immutable owner runtime inputs; never launch the application."""
import argparse,hashlib,json,os,subprocess,time
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--root',type=Path,required=True);a=p.parse_args();root=a.root.resolve();source=root/'host';assert not source.exists()
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def git(tree,*args):return subprocess.check_output(['git','-C',str(tree),*args],stderr=subprocess.PIPE).decode().strip()
def write(path,data):
 temp=path.with_name(path.name+'.next')
 with temp.open('x') as f:json.dump(data,f,indent=2);f.write('\n');f.flush();os.fsync(f.fileno())
 os.replace(temp,path)
inputs=json.loads((root/'preparation.json').read_text());assert len(inputs['files'])==8
for row in inputs['files']:
 p=root/row['name'];assert p.parent==root and p.is_file() and not p.is_symlink();assert p.stat().st_size==row['bytes'] and sha(p)==row['sha256']
assert sha(root/'frozen-source-manifest.json')=='ca69e70352c5f6f8d1c9db026959af6ba8fd64886be533a0095c3ca94bb5732e'
assert sha(root/'tui-play')=='392dbecc7a86f2fcd7e3f6b515aedceb3f3de3b1d4e140b95463bf20438c0c95'
manifest=json.loads((root/'frozen-source-manifest.json').read_text());expected={row['path']:row for row in manifest['source_members']}
assert len(expected)==1124
record={'scope':'Standalone immutable runtime/source staging only; no application execution, fixture access, runtime admission or live launch','started_unix':time.time(),'stage_script_sha256':sha(Path(__file__)),'preparation_sha256':sha(root/'preparation.json'),'input_files':inputs['files'],'live_qualified':False,'runtime_manifest_issued':False}
write(root/'staging-launch.json',record)
subprocess.run(['git','clone',str(root/'host.bundle'),str(source)],check=True)
git(source,'switch','-c','codex/direct-owner-host-archive');client=source/'vendor/fr-client-rust'
subprocess.run(['git','clone',str(root/'client.bundle'),str(client)],check=True);git(client,'switch','-c','codex/direct-owner-client-archive')
actual={str(q.relative_to(source)) for q in source.rglob('*') if (q.is_file() or q.is_symlink()) and '.git' not in q.relative_to(source).parts};assert actual==set(expected)
for name,row in expected.items():
 q=source/name;data=os.readlink(q).encode() if q.is_symlink() else q.read_bytes();mode='120000' if q.is_symlink() else ('100755' if q.stat().st_mode&0o111 else '100644');assert (len(data),hashlib.sha256(data).hexdigest(),mode)==(row['bytes'],row['sha256'],row['mode']),name
assert git(source,'rev-parse','HEAD')=='dcdbeebf36665a1d07156402a5f64c823769e00d';assert git(client,'rev-parse','HEAD')=='b74dfb3c998b10371b263189774b055ecf880380'
assert not git(source,'status','--porcelain','--untracked-files=all');assert not git(client,'status','--porcelain','--untracked-files=all')
digests={}
for label,tree in [('host',source),('client',client)]:
 h=hashlib.sha256();names=subprocess.check_output(['git','ls-files','--cached','--others','--exclude-standard','-z'],cwd=tree).split(b'\0')
 for name in sorted(set(names)):
  if not name:continue
  rel=Path(os.fsdecode(name))
  if (rel.parts[0]=='crates' or rel.name in ('Cargo.toml','Cargo.lock')) and (tree/rel).is_file():h.update(name+b'\0'+(tree/rel).read_bytes()+b'\0')
 digests[label]=h.hexdigest()
assert digests=={'host':'b70608d10f203657cbe731c6fd5402d09c96c1053466ce6540a7ab0dacc62564','client':'169ba594a834fb5ea81f392ae77909251cc46889efa2324525fff71794a7ea30'}
commands=[]
for command in [['file',str(root/'tui-play')],['ldd',str(root/'tui-play')]]:
 r=subprocess.run(command,capture_output=True,text=True,timeout=20);commands.append({'command':command,'exit':r.returncode,'stdout':r.stdout,'stderr':r.stderr});assert r.returncode==0 and 'not found' not in r.stdout
record.update(staged=True,source_member_count=1124,all_source_bytes_modes_verified=True,source_digests=digests,host_commit=git(source,'rev-parse','HEAD'),host_branch=git(source,'branch','--show-current'),client_commit=git(client,'rev-parse','HEAD'),host_clean=True,client_clean=True,binary_sha256=sha(root/'tui-play'),binary_bytes=(root/'tui-play').stat().st_size,library_checks=commands,finished_unix=time.time());write(root/'staging-result.json',record);print('STAGED:1124 exact source members, clean diagnostic identities, binary hash and library checks; no application launch')
