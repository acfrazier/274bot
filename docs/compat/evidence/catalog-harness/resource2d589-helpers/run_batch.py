import pathlib,sys,subprocess,json
root=pathlib.Path(__file__).resolve().parents[2];rev=sys.argv[1];rows=[]
gate=json.loads((root/'docs/compat/evidence/resource-fixture-corrections/root-model-verification.json').read_text());assert gate['actual_model_verified'] and gate['run']==1361
for case in ['gnome_chop','gnome_fletch_short','gnome_fletch_long','herblore_secondaries','flax_aio_pick']:
 for cat in ['100adccc037d9f6898080e1cad58fcfc43364775','8e7d965be2071d6ec65c3265e12af797082d720a']:
  code=subprocess.call(['python3',str(root/'.superpowers/resource125-root/run_cell.py'),rev,case,'resource2d589',cat],cwd=root)
  rows.append({'revision':rev,'case':case,'catalog':cat,'exit_code':code})
  if code:break
print(json.dumps(rows),flush=True)
