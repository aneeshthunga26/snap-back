#!/usr/bin/env python3
"""Run a command INSIDE cachy-vm via its existing QEMU guest agent."""
import base64,json,subprocess,sys,time

def qga(payload):
    p=subprocess.run(['virsh','-c','qemu:///session','qemu-agent-command','cachy-vm',json.dumps(payload)],capture_output=True,text=True,check=True)
    return json.loads(p.stdout)['return']
r=qga({'execute':'guest-exec','arguments':{'path':'/bin/bash','arg':['-lc',sys.argv[1]],'capture-output':True}})
pid=r['pid']
while True:
    r=qga({'execute':'guest-exec-status','arguments':{'pid':pid}})
    if r.get('exited'): break
    time.sleep(.25)
for key,stream in [('out-data',sys.stdout),('err-data',sys.stderr)]:
    if key in r: stream.buffer.write(base64.b64decode(r[key]))
sys.exit(r.get('exitcode',1))
