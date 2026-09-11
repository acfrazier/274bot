import pathlib,subprocess,json,sys
root=pathlib.Path(__file__).resolve().parents[2];rev=sys.argv[1];assert rev in ('274','289');gate=json.loads((root/'docs/compat/evidence/catalog-headed/review-gates-9cc6adb1.json').read_text());assert gate['all_required_reviews_approved'] and gate['root_fixture_checks_passed'];failed=[]
for case in ['mule_crafter']:
 for cat in ['100adccc037d9f6898080e1cad58fcfc43364775','8e7d965be2071d6ec65c3265e12af797082d720a']:
  code=subprocess.call(['python3',str(root/'.superpowers/catalog-headed/run_headless_28.py'),rev,case,'9cc6adb1',cat],cwd=root)
  if code:failed.append((case,cat,code));break
print(json.dumps(dict(revision=rev,failed=failed)),flush=True);raise SystemExit(bool(failed))
