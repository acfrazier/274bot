from pathlib import Path
import subprocess,os,difflib
root=Path.cwd();area=root/'.superpowers/gear-once-root';p='crates/scenario/src/lib.rs';base=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip();assert not subprocess.check_output(['git','diff','9b3dc71c0',base,'--','crates','vendor'])
old=subprocess.check_output(['git','show',base+':'+p],text=True);new=old
for name in ['wear and acknowledge Dragonfire shield before hostile-field teleport','wield and acknowledge Staff of fire before hostile-field teleport']:
 anchor=f'name: "{name}",\n            kind: StepKind::Repeat {{' if name.startswith('wear ') else f'name: "{name}",\n        kind: StepKind::Repeat {{'
 assert new.count(anchor)==1,name;new=new.replace(anchor,anchor.replace('StepKind::Repeat','StepKind::Perform'))
(area/'after.rs').write_text(new);patch=''.join(difflib.unified_diff(old.splitlines(True),new.splitlines(True),fromfile='a/'+p,tofile='b/'+p));(area/'fix.patch').write_text(patch)
subprocess.run(['git','apply','--check',str(area/'fix.patch')],check=True);subprocess.run(['git','apply',str(area/'fix.patch')],check=True)
env=dict(os.environ,GIT_INDEX_FILE=str(area/'index'));subprocess.run(['git','read-tree',base],env=env,check=True);subprocess.run(['git','apply','--cached',str(area/'fix.patch')],env=env,check=True);tree=subprocess.check_output(['git','write-tree'],env=env,text=True).strip();commit=subprocess.check_output(['git','commit-tree',tree,'-p',base],input='fix(scenario): send pre-start gear wear once before acknowledgment\n',text=True).strip();subprocess.run(['git','update-ref','HEAD',commit,base],check=True);subprocess.run(['git','apply','--cached',str(area/'fix.patch')],check=True)
print(commit);print(patch)
