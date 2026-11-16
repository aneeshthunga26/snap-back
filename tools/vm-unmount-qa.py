import subprocess,json,fcntl,os
from pathlib import Path
assert subprocess.check_output(['hostname'],text=True).strip()=='cachy-vm'
b='/backup/snap-back/.build/debug/snap-back'
u='469248ae-e83c-4ecc-bfdc-cbd384ea6eff'
m=Path('/backup/snap-back-drives')/u
c=Path('/etc/snap-back/config.json');before=c.read_bytes();checks=[]
def run(*args,ok=True):
 r=subprocess.run([b,*args],text=True,capture_output=True)
 assert (r.returncode==0)==ok,(args,r.stdout,r.stderr)
 return r
try:
 run('unmount-drive','not-a-drive',ok=False);checks.append('Unknown drive refused')
 with open('/run/snap-back/operation.lock','r') as f:
  fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)
  run('unmount-drive',u,ok=False)
 checks.append('Active operation lock refuses unmount')
 proc=subprocess.Popen(['sleep','30'],cwd=m)
 try:run('unmount-drive',u,ok=False)
 finally:proc.terminate();proc.wait()
 assert not Path('/run/snap-back/unmounted',u).exists()
 checks.append('Kernel busy mount refusal leaves mounted drive enabled')
 run('unmount-drive',u)
 s=json.loads(run('status').stdout);d=next(d for d in s['backup_drives'] if d['uuid']==u)
 assert d['connected'] and d['unmounted'] and not d['mounted']
 assert c.read_bytes()==before
 checks.append('Unmount succeeds, exposes physical connection and suppression, keeps config')
 run('unmount-drive',u,ok=False)
 run('backup','--scheduled','--drive',u)
 assert subprocess.run(['findmnt','-M',str(m)],capture_output=True).returncode!=0
 checks.append('Hourly backup skips unmounted target without remounting')
 run('use-drive',u)
 s=json.loads(run('status').stdout);d=next(d for d in s['backup_drives'] if d['uuid']==u)
 assert d['mounted'] and not d['unmounted']
 assert c.read_bytes()==before
 checks.append('Use drive remounts and clears suppression without changing selections')
finally:
 if Path('/run/snap-back/unmounted',u).exists():run('use-drive',u)
 assert c.read_bytes()==before
Path('/tmp/snap-back-unmount-results.json').write_text(json.dumps({'checks':checks,'vm':'cachy-vm','tested_drive':u,'configuration_preserved':True},indent=2))
print('\n'.join(checks))
