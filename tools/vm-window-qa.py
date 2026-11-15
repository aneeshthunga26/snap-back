#!/usr/bin/env python3
"""Isolated KWin window QA. Execute only inside cachy-vm as root."""
import argparse,json,os,pathlib,signal,subprocess,time
assert os.geteuid()==0 and subprocess.check_output(['hostname'],text=True).strip()=='cachy-vm', 'VM-only QA'
p=pathlib.Path('/tmp/snap-back-window-qa');p.mkdir(exist_ok=True)
parser=argparse.ArgumentParser();parser.add_argument('action',choices=['start','launch','probe','restore','stop']);parser.add_argument('--backend',default='x11',choices=['x11','wayland']);a=parser.parse_args()
def env():return {**os.environ,**json.loads((p/'session.json').read_text())}
def script(code):
    path=p/'probe.js';path.write_text(code)
    name='snap-back-qa-'+str(time.time_ns())
    sid=subprocess.check_output(['qdbus6','org.kde.KWin','/Scripting','org.kde.kwin.Scripting.loadScript',str(path),name],env=env(),text=True).strip()
    subprocess.run(['qdbus6','org.kde.KWin','/Scripting/Script'+sid,'org.kde.kwin.Script.run'],env=env(),check=True,capture_output=True)
    time.sleep(.15)
    subprocess.run(['qdbus6','org.kde.KWin','/Scripting','org.kde.kwin.Scripting.unloadScript',name],env=env(),capture_output=True)
if a.action=='start':
    runtime=p/'runtime';runtime.mkdir(exist_ok=True);runtime.chmod(0o700)
    (p/'session.json').unlink(missing_ok=True)
    (p/'session.sh').write_text('''#!/bin/bash
python - <<'CODE'
import os,json,pathlib
keys=['DISPLAY','WAYLAND_DISPLAY','XDG_RUNTIME_DIR','DBUS_SESSION_BUS_ADDRESS','XAUTHORITY']
pathlib.Path('/tmp/snap-back-window-qa/session.json').write_text(json.dumps({k:os.environ.get(k,'') for k in keys}))
CODE
exec sleep 7200
''');(p/'session.sh').chmod(0o755)
    (p/'start.sh').write_text('''#!/bin/bash
export QT_QPA_PLATFORM=offscreen
export XDG_RUNTIME_DIR=/tmp/snap-back-window-qa/runtime
export KWIN_COMPOSE=O2
export XWAYLAND_NO_GLAMOR=1
export QT_LOGGING_RULES=kwin_scripting.debug=true
exec kwin_wayland --virtual --xwayland --no-lockscreen --no-global-shortcuts --no-kactivities --width 1400 --height 1000 --socket snap-back-qa --exit-with-session /tmp/snap-back-window-qa/session.sh
''');(p/'start.sh').chmod(0o755)
    proc=subprocess.Popen(['dbus-run-session','--',str(p/'start.sh')],stdout=open(p/'compositor.log','w'),stderr=subprocess.STDOUT,start_new_session=True)
    (p/'compositor.pid').write_text(str(proc.pid))
    for _ in range(50):
        if (p/'session.json').exists():break
        assert proc.poll() is None,(p/'compositor.log').read_text()
        time.sleep(.2)
    print((p/'session.json').read_text())
elif a.action=='launch':
    e=env();e.update(HOME='/home/aneesh',XDG_CONFIG_HOME='/home/aneesh/.config',XDG_CURRENT_DESKTOP='KDE',WINIT_UNIX_BACKEND=a.backend,ICED_BACKEND='tiny-skia',SNAP_BACK_DEBUG_CHROME='1')
    e['WAYLAND_DEBUG']='client'
    e.pop('WAYLAND_DISPLAY' if a.backend=='x11' else 'DISPLAY',None)
    proc=subprocess.Popen(['/backup/snap-back/.build/debug/snap-back-gui'],env=e,stdout=open(p/f'gui-{a.backend}.log','w'),stderr=subprocess.STDOUT,start_new_session=True)
    (p/f'gui-{a.backend}.pid').write_text(str(proc.pid));time.sleep(2);print('GUI',a.backend,proc.pid,'exit',proc.poll())
elif a.action=='probe':
    import gi
    from gi.repository import Gio,GLib
    connection=Gio.DBusConnection.new_for_address_sync(env()['DBUS_SESSION_BUS_ADDRESS'],Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT|Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,None,None)
    connection.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','RequestName',GLib.Variant('(su)',('org.snapback.WindowQA',0)),None,Gio.DBusCallFlags.NONE,3000,None)
    interface=Gio.DBusNodeInfo.new_for_xml('<node><interface name="org.snapback.WindowQA"><method name="Report"><arg type="s" direction="in"/></method></interface></node>').interfaces[0]
    loop=GLib.MainLoop();rows=[]
    def receive(conn,sender,path,iface,method,params,invocation):
        rows.extend(json.loads(params.unpack()[0]));invocation.return_value(GLib.Variant('()',()));loop.quit()
    registration=connection.register_object('/org/snapback/WindowQA',interface,receive,None,None)
    code="const result=[];for(const w of workspace.windowList()){if(w.caption.indexOf('Snap Back')>=0){const g=w.frameGeometry;result.push({cursorX:workspace.cursorPos.x,cursorY:workspace.cursorPos.y,pid:w.pid,id:String(w.internalId),caption:w.caption,noBorder:w.noBorder,minimized:w.minimized,resizeable:w.resizeable,moveable:w.moveable,x:g.x,y:g.y,width:g.width,height:g.height});}}callDBus('org.snapback.WindowQA','/org/snapback/WindowQA','org.snapback.WindowQA','Report',JSON.stringify(result));"
    script(code)
    GLib.timeout_add(3000,lambda:loop.quit() or False);loop.run()
    connection.unregister_object(registration)
    print(json.dumps(rows))
elif a.action=='restore':
    pid=int((p/f'gui-{a.backend}.pid').read_text());script('for(const w of workspace.windowList()){if(w.pid==%s){w.minimized=false;workspace.activeWindow=w;}}'%pid)
elif a.action=='stop':
    for file in p.glob('gui-*.pid'):
        try:os.kill(int(file.read_text()),signal.SIGTERM)
        except ProcessLookupError:pass
    try:os.killpg(int((p/'compositor.pid').read_text()),signal.SIGTERM)
    except (ProcessLookupError,FileNotFoundError):pass
