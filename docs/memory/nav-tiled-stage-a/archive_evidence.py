"""Inspect and archive owned synthetic qualification evidence, never run probes."""
import argparse
import hashlib
import json
from pathlib import Path
import tarfile

HERE=Path(__file__).resolve().parent

def main():
    p=argparse.ArgumentParser();p.add_argument('--run',required=True);p.add_argument('--guards',required=True);p.add_argument('--archive',action='store_true');a=p.parse_args()
    run=HERE/a.run;guards=HERE/a.guards
    if run.parent!=HERE or guards.parent!=HERE:raise ValueError('owned direct subdirectories only')
    summary={'run':a.run,'guards':json.loads((guards/'result.json').read_text()),'integration':json.loads((run/'integration-tests/result.json').read_text()),
        'compiler':json.loads((run/'prepared.json').read_text())['compiler'],
        'builds':{p.name:json.loads(p.read_text()) for p in sorted(run.glob('*-build.receipt.json'))},
        'qualification':{v:json.loads((run/f'qualification-{v}/result.json').read_text()) for v in ('clean','counting')},
        'standin_metrics':json.loads((run/'standin-release/result.json').read_text())['metrics'],
        'self_tests':{p.name:json.loads(p.read_text()) for p in sorted((run/'integration-tests').glob('*-self-test.out'))}}
    print((guards/'guards.err').read_text())
    print('integration:',summary['integration']['qualified'])
    for name,b in summary['builds'].items():print(name,b['failure'],b['wall_seconds'])
    print(summary['compiler'])
    for v,q in summary['qualification'].items():
        for row in q['results']:print(v,row['fixture'],row['summary'][0]['aggregate'],'layout',row['summary'][1]['layout'],'narrow',[r['narrow_allocations'] for r in row['summary']])
    print('standin:',summary['standin_metrics'])
    print('self-tests:',summary['self_tests'])
    if a.archive:
        dest=HERE/'evidence';dest.mkdir(exist_ok=True)
        archive=dest/(a.run+'.tar.gz')
        if archive.exists():raise FileExistsError(archive)
        files=[]
        for root in (run,guards):
            for f in sorted(root.rglob('*')):
                rel=f.relative_to(root)
                if f.is_file() and (rel.parts[0] not in ('dense','tiled') or (len(rel.parts)==2 and f.suffix=='.json')):files.append(f)
        inventory=[]
        with tarfile.open(archive,'w:gz') as tar:
            for f in files:
                data=f.read_bytes();rel=str(f.relative_to(HERE));tar.add(f,arcname=rel,recursive=False)
                inventory.append(dict(path=rel,bytes=len(data),sha256=hashlib.sha256(data).hexdigest()))
        with tarfile.open(archive) as tar:
            members=tar.getmembers();assert len(members)==len(inventory)
            for m,e in zip(members,inventory):
                f=tar.extractfile(m);assert f is not None
                assert hashlib.sha256(f.read()).hexdigest()==e['sha256']
        record=dict(archive=archive.name,sha256=hashlib.sha256(archive.read_bytes()).hexdigest(),bytes=archive.stat().st_size,files=inventory)
        (dest/(a.run+'-archive.json')).write_text(json.dumps(record,indent=2)+'\n')
        (dest/(a.run+'-summary.json')).write_text(json.dumps(summary,indent=2)+'\n')
        print('verified archive:',archive,record['sha256'],len(files),record['bytes'])
if __name__=='__main__':main()
