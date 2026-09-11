import hashlib,json,pathlib,shutil
root=pathlib.Path(__file__).resolve().parents[2]
base=root/'.superpowers/catalog-headed/source-d2372fe0-56d8027'
manifest=json.loads((root/'docs/compat/evidence/catalog-headed/source-d2372fe0.json').read_text())
proof=root/'.superpowers/navigation-proof';out=root/'docs/compat/evidence/navigation-load'
for variant in ['external','bundled']:
 source=proof/f'source-d2372fe0-{variant}';assert not source.exists();shutil.copytree(base,source)
 extras={}
 for dest,origin in [('crates/host-play/examples/nav_origin_probe.rs',proof/'nav_origin_probe.rs'),('crates/panel/examples/startup_watch.rs',root/'.superpowers/browse-ux/source-1a2f2dcf/crates/panel/examples/startup_watch.rs')]:
  p=source/dest;p.parent.mkdir(exist_ok=True);shutil.copy2(origin,p);extras[dest]=hashlib.sha256(p.read_bytes()).hexdigest()
 table=[]
 if variant=='bundled':
  for rev in [274,289]:
   nav=root/f'.superpowers/world-capabilities/{rev}/274bot.navpack';m=json.loads(nav.with_suffix('.navpack.json').read_text());assert hashlib.sha256(nav.read_bytes()).hexdigest()==m['nav_sha256'];table.append(dict(m,format='274V8',relative_path=f'nav/{rev}/274bot.navpack'))
  p=source/'crates/host-play/src/bundled-nav-identities.json';p.write_text(json.dumps(table,indent=2)+'\n');extras[str(p.relative_to(source))]=hashlib.sha256(p.read_bytes()).hexdigest()
 for name,h in manifest['files'].items():
  if name not in extras:assert hashlib.sha256((source/name).read_bytes()).hexdigest()==h,name
 record=dict(host_commit=manifest['host_commit'],client_commit=manifest['client_commit'],source_root=str(source),variant=variant,compiled_identity_table=table,private_file_overrides=extras,unchanged_source_manifest='docs/compat/evidence/catalog-headed/source-d2372fe0.json',purpose='Local native and profile diagnostic fixture; not a shipped or signed package.')
 (out/f'private-source-{variant}.json').write_text(json.dumps(record,indent=2)+'\n');print(json.dumps(record),flush=True)
