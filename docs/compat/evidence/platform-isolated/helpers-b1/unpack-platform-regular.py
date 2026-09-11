"""Receive one hashed package using exclusive regular-file writes only."""
import datetime,hashlib,json,pathlib,sys,tarfile,os
archive=pathlib.Path(sys.argv[1]);dest=pathlib.Path(sys.argv[2]);expected=sys.argv[3];assert hashlib.sha256(archive.read_bytes()).hexdigest()==expected;assert not dest.exists()
with tarfile.open(archive,'r:gz') as tar:
 members=tar.getmembers();names=set()
 for member in members:
  path=pathlib.PurePosixPath(member.name);assert member.isfile() and not path.is_absolute() and all(p not in ('..','.git') for p in path.parts),member.name;assert member.name not in names;names.add(member.name)
 manifest=json.load(tar.extractfile('source-manifest.json'));assert set(manifest['files'])|{'source-manifest.json'}==names;dest.mkdir(parents=True)
 for member in members:
  data=tar.extractfile(member).read();assert len(data)==member.size
  if member.name!='source-manifest.json':assert hashlib.sha256(data).hexdigest()==manifest['files'][member.name],member.name
  path=dest/member.name;path.parent.mkdir(parents=True,exist_ok=True)
  with path.open('xb') as out:out.write(data)
  if os.name!='nt':path.chmod(0o755 if member.mode&0o111 else 0o644)
for name,h in manifest['files'].items():assert hashlib.sha256((dest/name).read_bytes()).hexdigest()==h,name
r=dict(time_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),root=str(dest),host_commit=manifest['host_commit'],client_commit=manifest['client_commit'],archive_sha256=expected,verified_files=len(manifest['files']),overlays=manifest.get('overlays',{}),method='Hash-verified unique regular members; exclusive new files; no archive extraction or overwrite');(dest/'verified-source.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r),flush=True)
