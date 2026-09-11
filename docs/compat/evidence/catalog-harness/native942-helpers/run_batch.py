import pathlib,sys,subprocess,json
root=pathlib.Path(__file__).resolve().parents[2];rev=sys.argv[1];rows=[]
for area in ['native-sequencing-observation-projection','brimhaven-qualified-actor']:
 gate=json.loads((root/f'docs/compat/evidence/{area}/root-model-verification.json').read_text());assert gate['actual_model_verified'] and gate['model']=='grok-4.5'
for case in ['brimhaven_agility','ardy_cakes','bone_burier','alcher_defaults','bank_fletcher_shafts','bank_fletcher_headless']:
 for cat in ['100adccc037d9f6898080e1cad58fcfc43364775','8e7d965be2071d6ec65c3265e12af797082d720a']:
  code=subprocess.call(['python3',str(root/'.superpowers/native942-root/run_cell.py'),rev,case,'native942',cat],cwd=root)
  rows.append({'revision':rev,'case':case,'catalog':cat,'exit_code':code})
  if code:break
print(json.dumps(rows),flush=True)
