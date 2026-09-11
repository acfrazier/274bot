import datetime, hashlib, json, os, pathlib, subprocess, sys, time
root=pathlib.Path(__file__).resolve().parents[2]
revision,case,source,catalog=sys.argv[1:]
assert revision in ('274','289')
assert case in ('bone_burier','chicken_killer','thiever','alcher','bank_fletcher','alcher_custom', 'alcher_custom_alias', 'alcher_custom_name','alcher_ordered','alcher_large_batch','bank_fletcher_string','bank_fletcher_cut_string', 'dart_fletcher', 'dart_fletcher_iron', 'herb_cleaner', 'herb_cleaner_named', 'gem_cutter', 'gem_cutter_named', 'door_opener', 'door_opener_gate', 'gnome_course', 'gnome_course_radius', 'flax_picker')
assert catalog in ('100adccc037d9f6898080e1cad58fcfc43364775','8e7d965be2071d6ec65c3265e12af797082d720a')
identity=json.loads((root/f'docs/compat/evidence/catalog-headed/binary-headless-{source}.json').read_text())
if not identity.get('isolated_build', False):
    raise SystemExit('BLOCKED: rebuild this diagnostic binary in an isolated Cargo target before new LIVE cells')
binary=pathlib.Path(identity['binary']); assert hashlib.sha256(binary.read_bytes()).hexdigest()==identity['binary_sha256']
evidence=root/'docs/compat/evidence/catalog-harness/live';evidence.mkdir(exist_ok=True)
out=evidence/f'r{revision}-{case.replace("_","-")}-{catalog[:8]}-{source}'
assert not out.with_suffix('.log').exists()
nav=root/f'.superpowers/world-capabilities/{revision}/274bot.navpack'
engine=pathlib.Path('/Users/acfrazier/experiments')/('Server/engine' if revision=='274' else 'lostcity-289/engine')
env=os.environ.copy();env.pop('CATALOG_SETTINGS_JSON',None)
env.update(LIVE='1',BOT_CPU='1',BOT_DEBUG='1',RUST_BACKTRACE='1',CATALOG_REVISION=revision,CATALOG_SCENARIO=case,CATALOG_COMMIT=catalog,CATALOG_ROOT=str(root/f'.superpowers/inputs/rs2b0t-{catalog}'),CATALOG_NAV_PACK=str(nav),CATALOG_NAV_FLAGS=str(nav.with_suffix('.navflags')),CATALOG_ENGINE_DIR=str(engine))
command=[str(binary),'catalog_boundary_live','--ignored','--exact','--nocapture']
receipt=dict(command=command,started_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),host_commit=identity['host_commit'],client_commit=identity['client_commit'],binary_sha256=identity['binary_sha256'],catalog_commit=catalog,revision=revision,case=case,environment={k:v for k,v in env.items() if k.startswith('CATALOG_') or k in ('LIVE','BOT_CPU','BOT_DEBUG','RUST_BACKTRACE')},elapsed_is_not_performance_measurement=True)
before=time.monotonic()
with out.with_suffix('.log').open('w') as log:
 p=subprocess.Popen(command,cwd=identity['source_root'],env=env,stdout=log,stderr=subprocess.STDOUT)
 receipt['pid']=p.pid;out.with_suffix('.process.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt),flush=True)
 code=p.wait()
receipt.update(finished_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),elapsed_seconds=round(time.monotonic()-before,3),exit_code=code,log_sha256=hashlib.sha256(out.with_suffix('.log').read_bytes()).hexdigest(),nav_sha256=hashlib.sha256(nav.read_bytes()).hexdigest(),engine_commit=subprocess.check_output(['git','rev-parse','HEAD'],cwd=engine,text=True).strip())
out.with_suffix('.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt),flush=True)
sys.exit(code)
