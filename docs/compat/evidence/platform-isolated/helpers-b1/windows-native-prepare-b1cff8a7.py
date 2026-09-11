import hashlib,json,pathlib,shutil,subprocess
base=pathlib.Path(r'C:\Users\Austen\274bot-campaign\multirevision-20260910');source=base/'platform-isolated-b1cff8a7';engine=base/'server-289';dest=pathlib.Path(r'C:\ProgramData\274bot-Test\multirevision-20260910-native-b1cff8a7');assert not dest.exists();dest.mkdir()
manifest=json.loads((source/'source-manifest.json').read_text());build=json.loads((source/'platform-build.json').read_text());job=next(j for j in build['jobs'] if j['name']=='catalog_watch');assert job['exit_code']==0
assert build.get('isolated_build') and build.get('target_started_empty')
binary=pathlib.Path(job['binary']);assert hashlib.sha256(binary.read_bytes()).hexdigest()==job['binary_sha256'];shutil.copyfile(binary,dest/'catalog-watch.exe')
selected=[p for p in manifest['files'] if p.startswith('inputs/') or p.startswith('nav/289/')]
for name in selected:
 p=source/name;assert hashlib.sha256(p.read_bytes()).hexdigest()==manifest['files'][name];out=dest/name;out.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(p,out)
for name in ('title','config','interface','media','versionlist','textures','wordenc','sounds'):
 src=engine/'data/pack/client'/name;out=dest/'cache'/name;out.parent.mkdir(exist_ok=True);shutil.copyfile(src,out)
node=pathlib.Path(r'C:\ProgramData\274bot-Test\tools\node-v24.19.0-win-x64\node.exe')
subprocess.run([str(node),str(base/'windows-public-login.mjs'),str(engine/'data/config/private.pem'),str(dest/'login-public.json')],check=True)
for name in ['windows-native-watch-b1cff8a7.py']:
 shutil.copyfile(base/name,dest/name)
files={str(p.relative_to(dest)).replace('\\','/'):hashlib.sha256(p.read_bytes()).hexdigest() for p in dest.rglob('*') if p.is_file()}
receipt=dict(isolated_build=True,cargo_target_dir=build['target_dir'],host_commit=manifest['host_commit'],client_commit=manifest['client_commit'],binary_sha256=job['binary_sha256'],engine_commit='cc359656b4acd216ca452495874b6beba9a0ac75',revision=289,files=files,scope='Native panel diagnostic inputs; contains only public login key components')
(dest/'native-manifest.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(dict(root=str(dest),files=len(files),binary_sha256=job['binary_sha256'])),flush=True)
