from pathlib import Path
import json,re,hashlib,subprocess
p=Path('docs/compat/evidence/stage-1');names=['baseline-declared-abi','host-tests-remaining'];d={}
for name in names:
 b=(p/(name+'.log')).read_bytes();s=b.decode();m=re.search(r'(assertion `left == right` failed: js_declared_abi.json is stale;.*?)(?=note: run with)',s,re.S);assert m;d[name]={'sha256':hashlib.sha256(b).hexdigest(),'assertion_sha256':hashlib.sha256(m[1].encode()).hexdigest(),'exit_code':101}
assert d[names[0]]['assertion_sha256']==d[names[1]]['assertion_sha256'];diff=subprocess.check_output(['git','diff','b2bd5023489ab2e0b6ba690f228217e8984ac91b','HEAD','--','crates/script/tests/declared_abi.rs','crates/script/tests/fixtures/js_declared_abi.json'],text=True);assert not diff
d['conclusion']='Identical assertion mismatch against current operator external catalog on baseline and candidate; unchanged test and declared ABI fixture. Explicitly excluded only from remaining check invocation, no source changes.'
(p/'baseline-abi-comparison.json').write_text(json.dumps(d,indent=2)+'\n')
