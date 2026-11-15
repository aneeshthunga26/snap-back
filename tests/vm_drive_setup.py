#!/usr/bin/env python3
"""VM-only non-destructive refusal checks before the disposable disk GUI test."""
import fcntl,json,os,pathlib,subprocess
assert os.geteuid()==0 and subprocess.check_output(['hostname'],text=True).strip()=='cachy-vm'
BIN='/usr/local/bin/snap-back'; log=pathlib.Path('/backup/snap-back/.validation/drive-setup'); log.mkdir(exist_ok=True)
checks=[]
def run(*args,ok=True):
 p=subprocess.run([BIN,*args],capture_output=True,text=True)
 assert (p.returncode==0)==ok, (args,p.stderr,p.stdout)
 return json.loads(p.stdout) if ok else p.stderr

def check(name,test=True):
 assert test,name
 checks.append(name);print('PASS:',name,flush=True)

d=run('drives'); by={x['device']:x for x in d['candidates']}
check('System drive blocked',not by['/dev/vda']['eligible'])
check('Current backup disk blocked',not by['/dev/vdb']['eligible'])
check('Disposable whole drive eligible',by['/dev/vdc']['eligible'])
for path in ['/dev/vda','/dev/vdb','/dev/vda2','/dev/null','/tmp/made-up-disk']:
 run('plan-drive','--device',path,ok=False)
check('System, current, partition, file, and arbitrary paths refused')
subprocess.run(['mount','/dev/vdc1','/run/snap-back-erase-test'],check=True)
try:
 check('Old filesystem marker present',pathlib.Path('/run/snap-back-erase-test/old-data-marker').is_file())
 run('plan-drive','--device','/dev/vdc',ok=False)
 check('Mounted child partition blocks whole-disk erasure')
finally:subprocess.run(['umount','/run/snap-back-erase-test'],check=True)
plan=run('plan-drive','--device','/dev/vdc')
check('Plan shows exact physical identity and device phrase',plan['serial']=='snap-back-setup-test' and plan['confirmation']=='ERASE /dev/vdc')
run('setup-drive','--token',plan['token'],'--confirm','ERASE /dev/vdb',ok=False)
check('Wrong-device confirmation refused')
run('setup-drive','--token','../../etc/passwd','--confirm','ERASE /dev/vdc',ok=False)
check('Invalid plan token refused')
file=pathlib.Path('/run/snap-back/drive-plans',plan['token']+'.json')
p=json.loads(file.read_text());p['expires_at']='2020-01-01T00:00:00Z';file.write_text(json.dumps(p))
run('setup-drive','--token',plan['token'],'--confirm',plan['confirmation'],ok=False)
check('Expired plan refused')
with open('/run/snap-back/operation.lock','r+') as lock:
 fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
 run('plan-drive','--device','/dev/vdc',ok=False)
 check('Backup/restore operation lock respected')
marker=pathlib.Path('/var/lib/snap-back/recovery-point');assert not marker.exists()
try:
 marker.write_text('test-pending-drive-guard')
 run('plan-drive','--device','/dev/vdc',ok=False)
 check('Pending recovery blocks drive setup')
finally:marker.unlink()
subprocess.run(['mount','/dev/vdc1','/run/snap-back-erase-test'],check=True)
try:check('All refusal cases left existing data intact',pathlib.Path('/run/snap-back-erase-test/old-data-marker').read_text().startswith('This marker'))
finally:subprocess.run(['umount','/run/snap-back-erase-test'],check=True)
plan=run('plan-drive','--device','/dev/vdc');(log/'hotplug-plan.json').write_text(json.dumps(plan))
(log/'refusal-results.json').write_text(json.dumps(checks,indent=2))
