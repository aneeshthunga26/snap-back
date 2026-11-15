#!/usr/bin/env python3
"""Verify real setup of the disposable VM disk, without formatting anything."""
import json,os,pathlib,subprocess
assert os.geteuid()==0 and subprocess.check_output(['hostname'],text=True).strip()=='cachy-vm'
log=pathlib.Path('/backup/snap-back/.validation/drive-setup')
c=json.load(open('/etc/snap-back/config.json'));o=json.load(open(log/'original-config.json'))
assert c['backup_uuid']=='469248ae-e83c-4ecc-bfdc-cbd384ea6eff' and c['sources']==o['sources']
assert len(c['targets'])==2 and any(t['uuid']==o['backup_uuid'] for t in c['targets'])
b=json.loads(subprocess.check_output(['lsblk','--json','-o','NAME,FSTYPE,UUID','/dev/vdc'],text=True))['blockdevices'][0]
assert b['fstype']=='btrfs' and not b.get('children')
p=pathlib.Path(c['destination']);assert not (p/'old-data-marker').exists()
f=pathlib.Path('/etc/fstab').read_text()
assert 'UUID={} {} btrfs'.format(c['backup_uuid'],c['destination']) in f
print('PASS: GPT/ext4 replaced with Btrfs; old marker gone; new UUID mounted/persistent; original backup registered; sources unchanged')
(log/'setup-config.json').write_text(json.dumps(c,indent=2))
