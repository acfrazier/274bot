from pathlib import Path
import subprocess,json,sys
root=Path(__file__).resolve().parents[2];rev,source=sys.argv[1:];build=json.loads((root/f'docs/compat/evidence/catalog-headed/binary-headless-{source}.json').read_text())
for area in ['autofighter-mage-fixture','bounded-native-reachability']:
 gate=json.loads((root/f'docs/compat/evidence/{area}/root-model-verification.json').read_text())
 assert gate['actual_model_verified'] and gate['model']=='grok-4.5' and gate['review_outcome']=='approved'
 subprocess.run(['git','merge-base','--is-ancestor',gate['implementation_commit'],build['host_commit']],cwd=root,check=True)
rows=[]
for case,helper in [('auto_fighter_mage','mage-root'),('flax_aio_pick','resource125-root')]:
 for catalog in ['100adccc037d9f6898080e1cad58fcfc43364775','8e7d965be2071d6ec65c3265e12af797082d720a']:
  code=subprocess.call(['python3',str(root/f'.superpowers/{helper}/run_cell.py'),rev,case,source,catalog],cwd=root)
  rows.append(dict(case=case,catalog=catalog,exit_code=code))
  if code:break
print(json.dumps(rows),flush=True)
