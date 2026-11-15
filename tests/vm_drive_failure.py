#!/usr/bin/env python3
"""VM-only destructive setup failure test on dedicated disposable vdd."""
import json,os,pathlib,subprocess
assert os.geteuid()==0 and subprocess.check_output(['hostname'],text=True).strip()=='cachy-vm'
b='/usr/local/bin/snap-back'
serial=subprocess.check_output(['lsblk','-dn','-o','SERIAL','/dev/vdd'],text=True).strip();assert serial=='snap-back-failure-te'
assert subprocess.check_output(['blockdev','--getsize64','/dev/vdd'],text=True).strip()=='2147483648'
cpath=pathlib.Path('/etc/snap-back/config.json');fpath=pathlib.Path('/etc/fstab');original=cpath.read_bytes();fstab=fpath.read_bytes()
p=json.loads(subprocess.check_output([b,'plan-drive','--device','/dev/vdd'],text=True))
subprocess.run(['mount','--bind',str(cpath),str(cpath)],check=True)
try:
 subprocess.run(['mount','-o','remount,bind,ro',str(cpath)],check=True)
 r=subprocess.run([b,'setup-drive','--token',p['token'],'--confirm',p['confirmation']],capture_output=True,text=True)
 assert r.returncode!=0 and 'previous backup configuration was restored' in r.stderr,r.stderr
 assert cpath.read_bytes()==original and fpath.read_bytes()==fstab
 uuid=subprocess.check_output(['blkid','-p','-s','UUID','-o','value','/dev/vdd'],text=True).strip()
 mounts=subprocess.check_output(['findmnt','-rn','-t','btrfs','-o','UUID'],text=True).splitlines();assert uuid not in mounts
 log=pathlib.Path('/backup/snap-back/.validation/drive-setup/save-failure.log');log.write_text(r.stderr)
 print('PASS: post-format save failure kept old config and fstab byte-identical and unmounted new disk')
finally:subprocess.run(['umount',str(cpath)],check=True)
