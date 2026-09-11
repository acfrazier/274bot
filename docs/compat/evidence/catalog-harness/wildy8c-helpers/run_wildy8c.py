import pathlib,sys,subprocess,json
root=pathlib.Path(__file__).resolve().parents[2];rev=sys.argv[1];rows=[]
gate=json.loads((root/'docs/compat/evidence/wildy-log-plane/root-model-verification.json').read_text());assert gate['actual_model_verified'] and gate['model']=='grok-4.5'
for cat in ['100adccc037d9f6898080e1cad58fcfc43364775','8e7d965be2071d6ec65c3265e12af797082d720a']:
 code=subprocess.call(['python3',str(root/'.superpowers/agility-root/run_cell.py'),rev,'wildy_agility','wildy8c',cat],cwd=root)
 rows.append({'revision':rev,'catalog':cat,'exit_code':code})
print(json.dumps(rows),flush=True)
