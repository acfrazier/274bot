import hashlib,json,pathlib,subprocess,sys,tarfile,time
root=pathlib.Path(__file__).resolve().parent
m=json.loads((root/'source-manifest.json').read_text());assert m['commit']=='6f2b41008f16414dc360b626f7283052888ec0fb'
assert hashlib.sha256((root/'build-manifest.json').read_bytes()).hexdigest()=='2869ec973bac0f9007be5f6e5e5c62c287da39ca26dfc5bfb4537bb6f75b919f'
assert hashlib.sha256((root/'source.tar.gz').read_bytes()).hexdigest()==m['archive_sha256']
src=root/'controller';src.mkdir(exist_ok=False)
with tarfile.open(root/'source.tar.gz') as t:
 ms=t.getmembers();assert len(ms)==len(m['files']) and {x.name for x in ms}=={x['path'] for x in m['files']}
 for x in ms:
  assert x.isfile() and not pathlib.PurePosixPath(x.name).is_absolute() and '..' not in pathlib.PurePosixPath(x.name).parts
  f=src/x.name;f.parent.mkdir(parents=True,exist_ok=True);f.write_bytes(t.extractfile(x).read())
for e in m['files']:
 b=(src/e['path']).read_bytes();assert len(b)==e['bytes'] and hashlib.sha256(b).hexdigest()==e['sha256']
sys.path.insert(0,str(src/'docs/memory'));import build_provenance as bp
r=json.loads((root/'build-manifest.json').read_text());v=bp.verify_direct_owner_build(root/'build-manifest.json','candidate','tui',r['binaries']['candidate_tui_play']['path'],r['nav']['nav_pack'],r['nav']['nav_flags'],r['catalog']['js_scripts_json'])
host=root.parent/'host';client=host/'vendor/fr-client-rust';checks=[]
for tree,commit,digest in [(host,bp.DIRECT_DIAGNOSTIC_HOST,bp.DIRECT_HOST_SOURCE_DIGEST),(client,bp.DIRECT_DIAGNOSTIC_CLIENT,bp.DIRECT_CLIENT_SOURCE_DIGEST)]:
 actual=subprocess.check_output(['git','-C',str(tree),'rev-parse','HEAD'],text=True).strip();clean=not subprocess.check_output(['git','-C',str(tree),'status','--porcelain','--untracked-files=all']);d=bp.source_digest(tree);assert actual==commit and clean and d==digest;checks.append({'path':str(tree),'commit':actual,'clean':clean,'source_digest':d})
out={'scope':'verified build/runtime file and source bindings only; no private-account admission, host-health admission or live launch','controller_commit':m['commit'],'controller_source_files_verified':len(m['files']),'verified_at_unix':time.time(),'runtime_build':v,'source_checks':checks,'live_released':False}
(root/'verification-result.json').write_text(json.dumps(out,indent=2)+'\n');print(json.dumps(out,indent=2))
