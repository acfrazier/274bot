from pathlib import Path
import subprocess,os,json,time,datetime,hashlib
root=Path.cwd();ev=root/'docs/compat/evidence/stage-1';head=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip();client=subprocess.check_output(['git','rev-parse','HEAD'],cwd=root/'vendor/fr-client-rust',text=True).strip();target=Path('/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision/.superpowers/review-exports/t_a74e9684-combat-target');env={**os.environ,'CARGO_TARGET_DIR':str(target),'GIT_COMMIT':head,'GIT_DIRTY':'0'}
jobs=[('fmt',['cargo','fmt','--all','--','--check'],root),('clippy',['cargo','clippy','-p','api','-p','host','-p','host-play','-p','nav','-p','script','-p','panel','-p','tui','--all-targets','--features','memory-profile','--','-D','warnings'],root),('host-tests',['cargo','test','--workspace','--features','memory-profile'],root),('client-tests',['cargo','test','-p','client','--tests','--features','audio'],root/'vendor/fr-client-rust')]
rows=[]
for name,cmd,cwd in jobs:
 p=ev/(name+'.log');assert not p.exists();start=time.monotonic()
 with p.open('x') as f:code=subprocess.call(cmd,cwd=cwd,env=env,stdout=f,stderr=subprocess.STDOUT)
 row=dict(name=name,command=cmd,cwd=str(cwd),exit_code=code,seconds=round(time.monotonic()-start,3),log=str(p.relative_to(root)),sha256=hashlib.sha256(p.read_bytes()).hexdigest());rows.append(row);print(json.dumps(row),flush=True)
 (ev/'checks.json').write_text(json.dumps(dict(host_commit=head,client_commit=client,source_root=str(root),target=str(target),target_started_empty=False,cache_ownership='Exclusively root-owned completed cache transferred sequentially to isolated stage1 checkout',checks=rows,all_passed=len(rows)==len(jobs) and all(r['exit_code']==0 for r in rows)),indent=2)+'\n')
 if code:raise SystemExit(code)
