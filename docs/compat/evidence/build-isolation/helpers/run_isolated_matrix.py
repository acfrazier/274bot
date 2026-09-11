import json,pathlib,subprocess,sys
root=pathlib.Path(__file__).resolve().parents[2]; rev=sys.argv[1]; assert rev in ['274','289']; source='a4157243'; catalogs=['100adccc037d9f6898080e1cad58fcfc43364775','8e7d965be2071d6ec65c3265e12af797082d720a']; cases=['bank_fletcher_string','bank_fletcher_cut_string','dart_fletcher','dart_fletcher_iron','herb_cleaner','herb_cleaner_named','gem_cutter','gem_cutter_named']; out=root/'docs/compat/evidence/catalog-harness/live'
for case in cases:
 for catalog in catalogs:
  if case=="bank_fletcher_cut_string" and catalog==catalogs[0]:
   print("Not applicable: old catalog has no cut-and-string mode; retained preflight refusal is a root matrix invocation error, no actor started.",flush=True); continue
  old=out/f'r{rev}-{case.replace("_","-")}-{catalog[:8]}-{source}.json'
  if old.exists():
   data=json.loads(old.read_text()); assert data['exit_code']==0,'Preserved earlier failed cell requires explicit diagnosis'; print('Existing completed cell: '+old.name,flush=True); continue
  command=['python3',str(root/'.superpowers/catalog-headed/run_headless_cell.py'),rev,case,source,catalog]; code=subprocess.call(command,cwd=root)
  if code: raise SystemExit(code)
