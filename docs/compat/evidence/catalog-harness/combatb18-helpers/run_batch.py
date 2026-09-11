import pathlib,sys,subprocess,json
root=pathlib.Path(__file__).resolve().parents[2];rev,source=sys.argv[1:];rows=[]
for area in ['combat-coal-qualification','native-quest-status']:
 gate=json.loads((root/f'docs/compat/evidence/{area}/root-model-verification.json').read_text());assert gate['actual_model_verified'] and gate['model']=='grok-4.5'
for case in ['chaos_druid', 'moss_giant', 'hill_giant', 'auto_fighter', 'rock_crab', 'green_dragon', 'fire_giant', 'ardy_fighter', 'coal_trucks']:
 for cat in ['100adccc037d9f6898080e1cad58fcfc43364775','8e7d965be2071d6ec65c3265e12af797082d720a']:
  code=subprocess.call(['python3',str(root/'.superpowers/combat136-root/run_cell.py'),rev,case,source,cat],cwd=root)
  rows.append({'revision':rev,'case':case,'catalog':cat,'exit_code':code})
  if code:break
print(json.dumps(rows),flush=True)
