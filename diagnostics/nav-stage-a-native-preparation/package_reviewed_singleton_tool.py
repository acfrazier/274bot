import argparse,hashlib,importlib.util,json,subprocess,tarfile
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--commit',required=True)
p.add_argument('--reference-qualification',type=Path,required=True)
p.add_argument('--label',default='')
p.add_argument('--native-reference-manifest',type=Path)
p.add_argument('--reference-run',type=Path)
p.add_argument('--source-binding',type=Path);p.add_argument('--source-binding-sha256')
a=p.parse_args()
assert not a.label or (a.label.replace('-','').isalnum() and len(a.label)<40)
suffix=('-'+a.label) if a.label else ''
assert bool(a.native_reference_manifest)==bool(a.reference_run)
assert bool(a.source_binding)==bool(a.source_binding_sha256)
base=Path(__file__).resolve().parent;root=base.parents[1]
commit=subprocess.check_output(['git','-C',str(root),'rev-parse',a.commit+'^{commit}']).decode().strip()
kind='coordinate' if a.source_binding else 'singleton'
dest=base/(kind+'-stage-'+commit[:8]+suffix);dest.mkdir(exist_ok=False)
tool=dest/'host/docs/memory/nav-tiled-stage-a';tool.mkdir(parents=True)
names=['frozen_support.py','frozen_probe.rs','frozen_generate.py','source_binding.py','coordinate-source-binding.json','stage_a.py','stage_probe.rs','allocator.rs','test_stage_a.py','proposed-manifest.json','README.md','sharded.py','test_sharded.py','test_sharded_guards.py','qualify_sharded.py','README-sharded.md','sharded-proposed-manifest.json']
for name in names:
 data=subprocess.check_output(['git','-C',str(root),'show',commit+':docs/memory/nav-tiled-stage-a/'+name]);(tool/name).write_bytes(data)
binding=None;module=None
if a.source_binding:
 assert a.source_binding.name=='coordinate-source-binding.json'
 assert hashlib.sha256(a.source_binding.read_bytes()).hexdigest()==a.source_binding_sha256
 spec=importlib.util.spec_from_file_location('source_binding',tool/'source_binding.py')
 assert spec is not None and spec.loader is not None
 module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
 binding=module.load(a.source_binding,a.source_binding_sha256)
 assert (tool/'coordinate-source-binding.json').read_bytes()==a.source_binding.read_bytes()
qualification=json.loads(a.reference_qualification.read_text())
assert qualification['qualified'] is True
if binding:
 assert module is not None
 assert (qualification['schema'],qualification['phase'],qualification['candidate_id'],
  qualification['source_binding'])==('stage-a-coordinate-generated-v1','GQ',
  binding['candidate_id'],module.reference(a.source_binding,a.source_binding_sha256,binding))
# Historical comparisons bind compiled probe/stage inputs. Scheduler tests may
# change in the separately reviewed commit without changing these readbacks.
for name,digest in qualification['tools']['stage'].items():
 assert hashlib.sha256((tool/name).read_bytes()).hexdigest()==digest
references={(e['variant'],e.get('reference_arm',e['arm']),e['fixture']):e['old_sha256'] for e in qualification['comparisons']}
assert len(references)==12
if a.native_reference_manifest:
 native=json.loads(a.native_reference_manifest.read_text())
 assert native['original_admissions_verified'] is True and native['platform'].startswith('Linux')
 assert native['original_tool_commit']=='f24de7cbcaaa3f419fc5485b77c4a117543a6afc'
 for name,digest in native['original_tools'].items():
  original=subprocess.check_output(['git','-C',str(root),'show',native['original_tool_commit']+':docs/memory/nav-tiled-stage-a/'+name])
  assert hashlib.sha256(original).hexdigest()==digest
 references={(e['variant'],e['arm'],e['fixture']):e['sha256'] for e in native['files']}
 assert len(references)==12
 (dest/'native-reference-manifest.json').write_bytes(a.native_reference_manifest.read_bytes())
objects=base/'frozen-git-objects';m=json.loads((objects/'manifest.json').read_text());(dest/'git-objects').mkdir()
for e in m['packs']:
 data=(objects/(e['name']+'.pack')).read_bytes();assert hashlib.sha256(data).hexdigest()==e['sha256'];(dest/'git-objects'/(e['name']+'.pack')).write_bytes(data)
(dest/'git-objects/manifest.json').write_bytes((objects/'manifest.json').read_bytes())
(dest/'git-objects/root-verification.json').write_bytes((objects/'root-verification.json').read_bytes())
if binding:
 oid=binding['overlays'][0]['new_git_blob']
 payload=subprocess.check_output(['git','-C',str(root),'pack-objects','--stdout'],input=(oid+'\n').encode())
 (dest/'git-objects/coordinate-overlay.pack').write_bytes(payload)
# The qualifier compares generated outputs and source spans with the old probe.
# Keep this additional evidence separate from the immutable production packs.
ancestor='f24de7cbcaaa3f419fc5485b77c4a117543a6afc'
oids={ancestor,subprocess.check_output(['git','-C',str(root),'rev-parse',ancestor+'^{tree}']).decode().strip()}
for line in subprocess.check_output(['git','-C',str(root),'ls-tree','-r','-t',ancestor]).decode().splitlines():
 meta,path=line.split('\t');mode,kind,oid=meta.split()
 if kind=='tree' or path=='docs/memory/nav-tiled-stage-a/stage_probe.rs':oids.add(oid)
payload=subprocess.check_output(['git','-C',str(root),'pack-objects','--stdout'],input=('\n'.join(sorted(oids))+'\n').encode())
(dest/'git-objects/qualifier-ancestor.pack').write_bytes(payload)
for variant in ('clean','counting'):
 for arm in ('dense','tiled'):
  for fixture in ('all-uniform','all-dense','gated'):
   rel=Path('run-05')/('qualification-'+variant)/(arm+'-'+fixture+'.out')
   original=(a.reference_run/Path(*rel.parts[1:])) if a.reference_run else root/'docs/memory/nav-tiled-stage-a'/rel
   assert original.is_file() and not original.is_symlink()
   data=original.read_bytes();assert hashlib.sha256(data).hexdigest()==references[(variant,arm,fixture)]
   output=tool/rel;output.parent.mkdir(parents=True,exist_ok=True);output.write_bytes(data)
(dest/'reference-qualification.json').write_bytes(a.reference_qualification.read_bytes())
files=[]
for f in sorted(dest.rglob('*')):
 if f.is_file():files.append(dict(path=str(f.relative_to(dest)),bytes=f.stat().st_size,sha256=hashlib.sha256(f.read_bytes()).hexdigest()))
manifest=dict(tool_commit=commit,scope=kind+' tooling: source-only native build and generated qualification payload; no real measurement release',files=files)
if binding:manifest.update(candidate_id=binding['candidate_id'],source_binding_sha256=a.source_binding_sha256,
 effective_source_policy=binding['effective_tree_policy'],overlay_git_blob=binding['overlays'][0]['new_git_blob'])
(dest/'payload-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
archive=base/('native-'+kind+'-source-'+commit[:8]+suffix+'.tar.gz')
with tarfile.open(archive,'x:gz') as t:
 for f in sorted(dest.rglob('*')):
  if f.is_file():t.add(f,arcname=str(f.relative_to(dest)),recursive=False)
receipt=dict(archive=archive.name,bytes=archive.stat().st_size,sha256=hashlib.sha256(archive.read_bytes()).hexdigest(),tool_commit=commit,files=len(files)+1)
(base/(archive.name+'.json')).write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt,indent=2))
