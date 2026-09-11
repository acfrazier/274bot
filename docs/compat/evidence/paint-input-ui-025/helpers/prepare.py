import pathlib,subprocess,json,hashlib,sys
root=pathlib.Path(__file__).resolve().parents[2];commit=subprocess.check_output(['git','rev-parse',sys.argv[1]],cwd=root,text=True).strip();label=sys.argv[2];dest=root/'.superpowers/review-exports'/('paint-ui-'+label);dest.mkdir();files={}
client=subprocess.check_output(['git','rev-parse',commit+':vendor/fr-client-rust'],cwd=root,text=True).strip()
for repo,ref,prefix in [(root,commit,''),(root/'vendor/fr-client-rust',client,'vendor/fr-client-rust/')]:
 entries=[]
 for row in subprocess.check_output(['git','ls-tree','-rz',ref],cwd=repo).split(b'\0'):
  if not row:continue
  header,path=row.split(b'\t');mode,kind,oid=header.split();name=prefix+path.decode()
  if kind==b'commit':continue
  assert kind==b'blob' and mode in [b'100644',b'100755'];assert not pathlib.PurePosixPath(name).is_absolute() and '..' not in pathlib.PurePosixPath(name).parts
  entries.append((name,mode,oid))
 out=subprocess.check_output(['git','cat-file','--batch'],cwd=repo,input=b''.join(oid+b'\n' for _,_,oid in entries));at=0
 for name,mode,oid in entries:
  end=out.index(b'\n',at);header=out[at:end].split();assert header[0]==oid and header[1]==b'blob';n=int(header[2]);data=out[end+1:end+1+n];assert out[end+1+n:end+2+n]==b'\n';at=end+2+n
  p=dest/name;p.parent.mkdir(parents=True,exist_ok=True)
  with p.open('xb') as f:f.write(data)
  if mode==b'100755':p.chmod(0o755)
  files[name]=hashlib.sha256(data).hexdigest()
 assert at==len(out)
p=dest/'crates/scenario/src/lib.rs';s=p.read_text();anchor='        "alcher" => Some(alcher_scenario()),';assert s.count(anchor)==1;s=s.replace(anchor,anchor+'\n        "paint_buttons_ui" => Some(paint_buttons_ui_scenario()),')
old=root/'.superpowers/review-exports/paint-ui-cd49e147/crates/scenario/src/lib.rs';tail=old.read_text().split('// Private root UI diagnostic.',1)[1];tail=tail.replace('ScriptInjectValue::Str("Air")','ScriptInjectValue::Str("Air runes")');p.write_text(s+'\n// Private root UI diagnostic.'+tail)
p2=dest/'crates/tui/examples/platform_proof.rs';p2.parent.mkdir(parents=True,exist_ok=True);assert not p2.exists();p2.write_text('fn main() -> std::process::ExitCode { let _stores = script::IsolatedEnv::enter(&format!("paint-ui-{}", std::process::id())); tui::bin::main() }\n')
overlays={str(p.relative_to(dest)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [p,p2]};files.update(overlays)
m=dict(host_commit=commit,client_commit=client,source_root=str(dest),files=files,overlays=overlays,export_method='Exact regular Git blobs plus two explicit private UI fixture files; canonical Air runes; original Alcher clock and witness unchanged; no gameplay acceptance.')
(root/f'docs/compat/evidence/catalog-headed/source-{label}.json').write_text(json.dumps(m,indent=2)+'\n');print(commit,client,len(files),dest)
