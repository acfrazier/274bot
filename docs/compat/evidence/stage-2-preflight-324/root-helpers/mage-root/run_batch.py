import pathlib,sys,subprocess,json
root=pathlib.Path(__file__).resolve().parents[2];rev,source=sys.argv[1:]
for area in ['autofighter-mage-fixture','bounded-native-reachability']:
 gate=json.loads((root/f'docs/compat/evidence/{area}/root-model-verification.json').read_text())
 assert gate['actual_model_verified'] and gate['model']=='grok-4.5'
for cat in ['100adccc037d9f6898080e1cad58fcfc43364775','8e7d965be2071d6ec65c3265e12af797082d720a']:
 code=subprocess.call(['python3',str(root/'.superpowers/mage-root/run_cell.py'),rev,'auto_fighter_mage',source,cat],cwd=root)
 if code:break
