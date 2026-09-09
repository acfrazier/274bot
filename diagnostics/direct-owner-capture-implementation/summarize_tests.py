"""Summarize real log assertions, never synthesize suite output."""
import json
from pathlib import Path
import re
ROOT = Path(__file__).resolve().parent
rows = json.loads((ROOT/'regressions.json').read_text())
retries = {r['name']:r for r in json.loads((ROOT/'final-retries.json').read_text())}
for row in rows:
    if row['name'] in retries:
        row['initial_exit'] = row['exit']
        row.update(retries[row['name']])
    text = (ROOT/row.get('log', row['name']+'.log')).read_text()
    row['test_results'] = re.findall(r'test result: .*', text)
    row['failures'] = re.findall(r'^(?:error[^\n]*|test [^\n]*FAILED[^\n]*)', text, re.MULTILINE)
result = {'expected_suites':14, 'collected_suites':len(rows), 'all_passed':len(rows)==14 and all(r['exit']==0 for r in rows), 'suites':rows}
(ROOT/'test-summary.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({'expected':14,'collected':len(rows),'all_passed':result['all_passed'],'results':[(r['name'],r['exit'],r['failures']) for r in rows]},indent=2))
