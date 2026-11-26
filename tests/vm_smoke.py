#!/usr/bin/env python3
"""Run ONLY inside cachy-vm, as root, against the installed prototype.

Creates marker files and exercises real Btrfs transfers. Does not reboot;
full recovery is staged separately after these checks.
"""
import json, os, pathlib, subprocess, sys
assert os.geteuid() == 0
assert subprocess.check_output(['hostname'], text=True).strip() == 'cachy-vm', 'VM-only test'
BIN='/usr/local/bin/snap-back'
LOG=pathlib.Path('/backup/snap-back/.validation')
LOG.mkdir(exist_ok=True)
results=[]
def run(*args, ok=True, env=None):
    p=subprocess.run([BIN,*args],text=True,capture_output=True,env=env)
    if ok: assert p.returncode == 0, p.stderr
    else: assert p.returncode != 0, f'Unexpected success: {args}'
    return json.loads(p.stdout) if ok else p.stderr

def check(name, condition=True):
    assert condition, name
    results.append(name)
    print('PASS:', name, flush=True)

markers={'/etc/snap-back-test-marker':'root-before-recovery\n','/home/aneesh/snap-back-test-marker':'home-before-recovery\n','/srv/snap-back-test-marker':'srv-before-recovery\n','/var/lib/machines/snap-back-test-marker':'nested-before-recovery\n'}
for p,content in markers.items(): pathlib.Path(p).write_text(content)
os.chown('/home/aneesh/snap-back-test-marker',1000,1000)
s=run('backup','--label','Verified root and home')
point=s['points'][0]['id']
check('Full backup creates all configured subvolumes', len(s['points'][0]['subvolumes']) == 9)
for name,path in [('root','etc/snap-back-test-marker'),('home','aneesh/snap-back-test-marker'),('srv','snap-back-test-marker'),('machines','snap-back-test-marker')]:
    check(f'{name} marker stored on backup disk', (LOG.parent/'points'/point/name/'snapshot'/path).exists())
browse=run('browse',point,'home','aneesh')
check('File browser lists backed-up home',any(e['name']=='snap-back-test-marker' for e in browse))
dest=f'/home/aneesh/SnapBack-Test-Recovered-{point}'
run('restore-file',point,'home','aneesh/snap-back-test-marker','--to',dest,env={**os.environ,'PKEXEC_UID':'1000'})
check('File recovery preserves content', pathlib.Path(dest,'snap-back-test-marker').read_text() == markers['/home/aneesh/snap-back-test-marker'])
check('Recovered data belongs to invoking user', pathlib.Path(dest,'snap-back-test-marker').stat().st_uid == 1000)
check('Existing recovery destination refused', bool(run('restore-file',point,'home','aneesh/snap-back-test-marker','--to',dest,ok=False)))
check('Path traversal refused', bool(run('browse',point,'home','../../etc',ok=False)))
pathlib.Path('/etc/snap-back-incremental-marker').write_text('incremental-only\n')
s=run('backup','--label','Recovery test checkpoint')
point=s['points'][0]['id']
check('Second backup uses incremental Btrfs send',s['points'][0]['kind']=='incremental')
check('Hourly retention coalesces same-hour points',len(s['points'])==1)
check('Incremental content exists on backup disk',(LOG.parent/'points'/point/'root/snapshot/etc/snap-back-incremental-marker').read_text()=='incremental-only\n')
run('schedule','on')
check('Schedule can be enabled',run('status')['schedule_enabled'])
run('schedule','off')
check('Schedule can be paused',not run('status')['schedule_enabled'])
run('recover-offline',ok=False)
check('Offline restore refuses live system')
subprocess.run(['btrfs','subvolume','create','/home/aneesh/snap-back-unconfigured-test'],check=True,capture_output=True)
try:
    s=run('backup','--label','Recovery test checkpoint')
    point=s['points'][0]['id']
    check('Backup reports skipped nested subvolume and keeps all configured sources',
          '@home/aneesh/snap-back-unconfigured-test' in s['skipped_subvolumes']
          and len(s['points'][0]['subvolumes']) == 9)
finally: subprocess.run(['btrfs','subvolume','delete','/home/aneesh/snap-back-unconfigured-test'],check=True,capture_output=True)
(LOG/'smoke-results.json').write_text(json.dumps(dict(point=point,checks=results),indent=2))
(LOG/'recovery-point').write_text(point)
print('RECOVERY_POINT='+point,flush=True)
