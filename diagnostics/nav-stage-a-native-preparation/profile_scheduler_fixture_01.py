"""Generated fixture cost diagnosis only; never native qualification."""
from pathlib import Path
import sys,json,pstats
root=Path('/home/builder/nav-coordinate-a44a930-native-01');tool=root/'host/docs/memory/nav-tiled-stage-a'
sys.path.insert(0,str(tool));import frozen_support as support
out=root/'root-scheduler-profile-01';out.mkdir(exist_ok=False)
command=[sys.executable,'-m','cProfile','-o',str(out/'legacy.prof'),'-m','unittest','-v','test_sharded.Metrics.test_full_generated_protocol']
r=support.bounded(command,out,'profile',cwd=tool,wall=100,cpu=90,rss=512*1024**2,address=4*1024**3,output=1024**2)
if (out/'legacy.prof').exists():
 with (out/'profile-summary.txt').open('x') as f:pstats.Stats(str(out/'legacy.prof'),stream=f).strip_dirs().sort_stats('cumtime').print_stats(35)
print(json.dumps(r,indent=2))
