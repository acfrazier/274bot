import json,pathlib,sys,time
root=pathlib.Path('/home/acfrazier/274bot-campaign/nav-coordinate-113ce05-concord-01')
tool=root/'host/docs/memory/nav-tiled-stage-a';sys.path.insert(0,str(tool));import sharded as sh
stage=sh.stage
stage.configure_source_binding(tool/'coordinate-source-binding.json','7b101c30bfd64c302815f9844fe27d3a52302feedaec58737726ab6a062491bb')
rootref={'path':str(root/'root-coordinate-cf1-admission/CF1-authorization.json'),'sha256':'62835390bdcbee419029f6c0fcf38f1126a502cde926be643de0c64acd9977ec'}
a=sh.read_ref(rootref);run=pathlib.Path(a['run']);dest=pathlib.Path(a['destination'])
stage.check_release(run,a['prerequisites']);sh.check_scheduler_qualification(a)
parent=sh.reference(dest/'CF1/result.json');r=sh.read_ref(parent)
assert r['status']=='complete' and r['completed']==4 and r['budget']['children']==8
sh.bind_entries(dest/'CF1',r['entries'],'CF1');sh.collect(dest/'CF1',r['entries'],'CF1')
def counters():return list(map(int,pathlib.Path('/proc/stat').read_text().splitlines()[0].split()[1:9]))
before=counters();time.sleep(1);after=counters();n=sum(after)-sum(before);assert n>0
idle=100*(after[3]+after[4]-before[3]-before[4])/n;steal=100*(after[7]-before[7])/n
mem={r.split(':')[0]:int(r.split()[1])*1024 for r in pathlib.Path('/proc/meminfo').read_text().splitlines()}
commands=stage.subprocess.check_output(['ps','-axo','comm='],text=True).splitlines();conflicts=[s for s in commands if pathlib.Path(s.strip()).name in {'stage-a-probe','cargo','rustc','heaptrack','tui-play','panel-play'}]
assert not conflicts and idle>=90 and steal==0 and mem['MemAvailable']>=512*1024**2 and mem['SwapTotal']==mem['SwapFree']
p=root/'root-coordinate-cf2-admission';p.mkdir(exist_ok=False)
pre=dict(timestamp=time.time(),hardware=stage.hardware(),boot_id=pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),idle_percent=idle,steal_percent=steal,no_competing_work=True,memory_available=mem['MemAvailable'],sample_seconds=1,before=before,after=after)
sh.write_json(p/'preflight.json',pre)
auth=dict(sh.identity('CF2'),**sh.coordinate_ledger_identity(),campaign_budget=a['campaign_budget'],released=True,mode='real',root=rootref,parent=parent,review_approved=True,review_sha256='f08e73dfdcc1909ea15e20313a4b58b08e1ba96a43ecac4acea1ec538946305a',preflight=sh.reference(p/'preflight.json'),scope='CF2 remaining114 feasibility only; no CA or performance acceptance')
sh.write_json(p/'CF2-authorization.json',auth);print(json.dumps(dict(authorization=sh.reference(p/'CF2-authorization.json'),preflight=pre,parent=parent),indent=2))
