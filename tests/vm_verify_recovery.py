#!/usr/bin/env python3
"""Verify the destructive recovery test, only inside cachy-vm after reboot."""
import json,os,pathlib,subprocess
assert os.geteuid()==0
assert subprocess.check_output(['hostname'],text=True).strip()=='cachy-vm'
log=pathlib.Path('/backup/snap-back/.validation')
point=(log/'recovery-point').read_text().strip()
checks=[]
def check(name,value):
 assert value,name
 checks.append(name)
 print('PASS:',name,flush=True)
expected={'/etc/snap-back-test-marker':'root-before-recovery\n','/home/aneesh/snap-back-test-marker':'home-before-recovery\n','/srv/snap-back-test-marker':'srv-before-recovery\n','/var/lib/machines/snap-back-test-marker':'nested-before-recovery\n'}
for path,content in expected.items():check('Restored '+path,pathlib.Path(path).read_text()==content)
for path in ['/etc/snap-back-post-checkpoint','/home/aneesh/snap-back-post-checkpoint']:check('Removed post-checkpoint file '+path,not pathlib.Path(path).exists())
check('VM rebooted',pathlib.Path('/proc/sys/kernel/random/boot_id').read_text()!=(log/'pre-recovery-boot-id').read_text())
journal=json.loads(pathlib.Path('/boot/snap-back/last-recovery.json').read_text())
check('Recovery journal committed',journal['phase']=='completed' and journal['point_id']==point)
check('Recovery is disarmed',not pathlib.Path('/boot/snap-back/pending.json').exists() and 'Snap Back Recovery' not in pathlib.Path('/boot/limine.conf').read_text())
check('Booted kernel has matching modules',pathlib.Path('/usr/lib/modules',os.uname().release,'vmlinuz').is_file())
check('Home marker owner preserved',pathlib.Path('/home/aneesh/snap-back-test-marker').stat().st_uid==1000)
check('Root Snapper history preserved',pathlib.Path('/.snapshots/29/snapshot').is_dir())
check('Home Snapper history preserved',pathlib.Path('/home/.snapshots/3/snapshot').is_dir())
top=pathlib.Path('/run/snap-back-verify')
top.mkdir(exist_ok=True)
subprocess.run(['mount','-t','btrfs','-o','subvolid=5','UUID=1967a6a9-c92b-4d71-a361-94dd80a7856e',str(top)],check=True)
try:
 before=top/('.snap-back-restore-'+point)/'before'
 check('Previous root retained',(before/'root/etc/snap-back-test-marker').read_text()=='CHANGED AFTER BACKUP\n')
 check('Previous home retained',(before/'home/aneesh/snap-back-test-marker').read_text()=='CHANGED AFTER BACKUP\n')
 check('Previous auxiliary data retained',(before/'srv/snap-back-test-marker').read_text()=='CHANGED AFTER BACKUP\n')
finally:subprocess.run(['umount',str(top)],check=True)
(log/'recovery-results.json').write_text(json.dumps({'point':point,'checks':checks,'journal':journal},indent=2))
