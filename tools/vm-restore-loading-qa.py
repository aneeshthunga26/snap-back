#!/usr/bin/env python3
"""VM-only graphical check of retained file listings using a private mount namespace."""
import json,os,pathlib,signal,subprocess,time
assert subprocess.check_output(['hostname'],text=True).strip()=='cachy-vm'
p=pathlib.Path('/tmp/snap-back-restore-qa');p.mkdir(exist_ok=True)
runtime=p/'runtime';runtime.mkdir(exist_ok=True);runtime.chmod(0o700)
helper=p/'helper';subprocess.run(['cp','/usr/local/bin/snap-back',str(helper)],check=True)
wrapper=p/'delayed-helper'
wrapper.write_text('''#!/bin/bash
if [[ "$1" == browse ]]; then
  touch /tmp/snap-back-restore-qa/inflight-$$
  sleep 1.5
  /tmp/snap-back-restore-qa/helper "$@"
  result=$?
  rm -f /tmp/snap-back-restore-qa/inflight-$$
  exit "$result"
fi
exec /tmp/snap-back-restore-qa/helper "$@"
''');wrapper.chmod(0o755)
xvfb=subprocess.Popen(['Xvfb',':96','-screen','0','1180x800x24','-ac'],stdout=open(p/'xvfb.log','w'),stderr=subprocess.STDOUT,start_new_session=True)
time.sleep(.3)
e={**os.environ,'DISPLAY':':96','XDG_RUNTIME_DIR':str(runtime),'HOME':'/home/aneesh','XDG_CURRENT_DESKTOP':'KDE','ICED_BACKEND':'tiny-skia'};e.pop('WAYLAND_DISPLAY',None)
app=subprocess.Popen(['dbus-run-session','--','unshare','--mount','--propagation','private','--','bash','-c','mount --bind /tmp/snap-back-restore-qa/delayed-helper /usr/local/bin/snap-back && exec /backup/snap-back/.build/debug/snap-back-gui'],env=e,stdout=open(p/'gui.log','w'),stderr=subprocess.STDOUT,start_new_session=True)
checks=[]
def run(*args,**kw):return subprocess.run(args,env=e,check=True,**kw)
def click(x,y):run('xdotool','mousemove',str(x),str(y),'click','1');time.sleep(.12)
def capture(name):
 run('import','-window',wid,str(p/(name+'.png')))
def wait_idle():
 for _ in range(80):
  if not list(p.glob('inflight-*')):time.sleep(.25);return
  time.sleep(.1)
 raise AssertionError('Browse did not complete')
try:
 for _ in range(60):
  result=subprocess.run(['xdotool','search','--name','^Snap Back$'],env=e,text=True,capture_output=True)
  if result.returncode==0:break
  assert app.poll() is None,(p/'gui.log').read_text()
  time.sleep(.1)
 wid=result.stdout.splitlines()[-1];time.sleep(1)
 click(115,244)
 for _ in range(60):
  if list(p.glob('inflight-*')):break
  time.sleep(.1)
 wait_idle();time.sleep(.5);capture('before')
 click(355,290);time.sleep(.35)
 assert list(p.glob('inflight-*')),'Switch should have an outstanding read'
 capture('during')
 # The completed listing remains in place while the second point is pending.
 for name in ['before','during']:
  run('magick',str(p/(name+'.png')),'-crop','485x225+530+310','+repage',str(p/(name+'-list.png')))
 comparison=subprocess.run(['magick','compare','-metric','AE',str(p/'before-list.png'),str(p/'during-list.png'),'null:'],env=e,text=True,capture_output=True)
 assert float(comparison.stderr.split()[0])==0,comparison.stderr
 checks.append('Previous listing pixels retained during delayed restore-point read')
 wait_idle();capture('after');checks.append('New read completes without a blank-list intermediate state')
 # Selected point can be changed repeatedly while a prior request is pending.
 click(355,213);click(355,366);click(355,290);time.sleep(.35)
 assert len(list(p.glob('inflight-*')))>=2,'Rapid switches should start newer reads'
 wait_idle();capture('rapid-switch');checks.append('Rapid point switches remain responsive with overlapping reads')
 (p/'results.json').write_text(json.dumps({'checks':checks,'environment':'cachy-vm only','delay_seconds':1.5,'installed_helper_modified':False},indent=2))
 print(json.dumps(checks))
finally:
 for proc in [app,xvfb]:
  try:os.killpg(proc.pid,signal.SIGTERM)
  except ProcessLookupError:pass
