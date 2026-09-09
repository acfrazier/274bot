"""Root-only F1 admission preparation; does not launch a probe."""
import argparse,hashlib,json,pathlib,sys,time
p=argparse.ArgumentParser();p.add_argument('--root',type=pathlib.Path,required=True);p.add_argument('--input',type=pathlib.Path,required=True);p.add_argument('--routes',type=pathlib.Path,required=True);p.add_argument('--review-sha256',required=True);p.add_argument('--qualification-name');p.add_argument('--source-binding',type=pathlib.Path);p.add_argument('--source-binding-sha256');p.add_argument('--child-decision-sha256');a=p.parse_args()
assert len(a.review_sha256)==64 and all(c in '0123456789abcdef' for c in a.review_sha256)
coordinate=any((a.source_binding,a.source_binding_sha256,a.child_decision_sha256))
assert coordinate==all((a.source_binding,a.source_binding_sha256,a.child_decision_sha256))
qualification=a.qualification_name or ('coordinate-ebf0f30-qualification-04' if coordinate else 'singleton-qualification-04')
prefix='coordinate-ebf0f30-qualification-' if coordinate else 'singleton-qualification-'
assert qualification.startswith(prefix) and qualification.removeprefix(prefix).isdigit()
root=a.root.resolve();tool=root/'host/docs/memory/nav-tiled-stage-a';run=tool/('coordinate-concord-run-01' if coordinate else 'concord-run-01')
sys.path.insert(0,str(tool));import sharded as sh
stage=sh.stage
if coordinate:
 stage.configure_source_binding(a.source_binding,a.source_binding_sha256)
 sh.coordinate_budget(a.child_decision_sha256,sh.PRIOR_F1_LEDGER)
qpath=run/qualification/'result.json';guard=tool/(('coordinate-concord-native-guards-01' if coordinate else 'concord-native-guards-01')+'/result.json')
release=root/('root-coordinate-cf1-admission' if coordinate else 'root-singleton-f1-admission');release.mkdir()
# Qualify executable/source/guards before reading the real corpus.
pre=dict(released=True,mode='real',limits=stage.LIMITS,tools=stage.tools(),platform=sh.platform.platform(),hardware=stage.hardware(),order=json.loads((tool/'proposed-manifest.json').read_text())['order'],manifest_sha256=stage.sha(tool/'proposed-manifest.json'),correctness_review_released=True,guard_path=str(guard),guard_sha256=stage.sha(guard))
if coordinate:pre.update(schema='stage-a-coordinate-release-v1',candidate_id=sh.protocol()['candidate_id'],
 source_binding=sh.protocol()['source_binding'],phase='tooling-release')
for variant in ('clean','counting'):
 pre[variant+'_qualification_sha256']=stage.sha(run/f'qualification-{variant}/result.json')
 for arm in stage.ARMS:pre[f'{arm}-{variant}-admission_sha256']=stage.sha(run/f'{arm}-{variant}-admission.json')
stage.check_release(run,pre)
sh.check_scheduler_qualification(dict(scheduler_qualification=sh.reference(qpath)))
expected_pack='2f393138c905aaf1b2db4f77442426db01ff2dfad5ee575d77454012d27a4a30'
expected_routes='49e348ea78806c8278d720d27b171c54a0a209930fa8191ffcd38d2e9bbbc125'
stage.admit(a.input,expected_pack,73438581);assert a.input.stat().st_size==73438581
stage.admit(a.routes,expected_routes,65536);rows=sh.normalized_rows(a.routes)
pre.update(input_path=str(a.input.absolute()),input_sha256=expected_pack,input_bytes=73438581,routes_path=str(a.routes.absolute()),routes_sha256=expected_routes)
def counters():return list(map(int,pathlib.Path('/proc/stat').read_text().splitlines()[0].split()[1:9]))
before=counters();time.sleep(sh.NATIVE_IDLE_SECONDS);after=counters();elapsed=sum(after)-sum(before)
assert elapsed>0
idle=100*(after[3]+after[4]-before[3]-before[4])/elapsed;steal=100*(after[7]-before[7])/elapsed
memory={r.split(':')[0]:int(r.split()[1])*1024 for r in pathlib.Path('/proc/meminfo').read_text().splitlines()}
commands=stage.subprocess.check_output(['ps','-axo','comm='],text=True).splitlines()
conflicts=[s for s in commands if pathlib.Path(s.strip()).name in {'stage-a-probe','cargo','rustc','heaptrack','tui-play','panel-play'}]
assert not conflicts and idle>=90 and steal==0 and memory['MemAvailable']>=512*1024**2 and memory['SwapTotal']==memory['SwapFree']
preflight=dict(timestamp=time.time(),hardware=stage.hardware(),boot_id=pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),idle_percent=idle,steal_percent=steal,no_competing_work=True,memory_available=memory['MemAvailable'],sample_seconds=sh.NATIVE_IDLE_SECONDS,before=before,after=after)
pp=release/'preflight.json';sh.write_json(pp,preflight)
phase='CF1' if coordinate else 'F1'
auth=dict(sh.identity(phase),released=True,mode='real',run=str(run),
 destination=str(run/('coordinate-ebf0f30-CF1-release-01' if coordinate else 'root-singleton-release-01')),
 prerequisites=pre,contract=sh.contract(rows),preflight=sh.reference(pp),scheduler_qualification=sh.reference(qpath),
 root_reference_review_sha256=a.review_sha256,scope=phase+' feasibility only; no continuation or acceptance release')
if coordinate:auth.update(sh.coordinate_ledger_identity(),campaign_budget=sh.coordinate_budget(
 a.child_decision_sha256,sh.PRIOR_F1_LEDGER))
ap=release/(phase+'-authorization.json');sh.write_json(ap,auth)
print(json.dumps(dict(authorization=str(ap),sha256=stage.sha(ap),run=str(run),scope=auth['scope']),indent=2))
