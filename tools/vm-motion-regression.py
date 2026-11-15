#!/usr/bin/env python3
"""VM-only vector-animation regression using a read-only private helper mount."""
import json
import os
from pathlib import Path
import signal
import subprocess
import time

assert os.geteuid() == 0
assert subprocess.check_output(['hostname'], text=True).strip() == 'cachy-vm'
p = Path('/tmp/snap-back-motion-regression')
p.mkdir(exist_ok=True)
runtime = p / 'runtime'
runtime.mkdir(exist_ok=True)
runtime.chmod(0o700)
flag = p / 'busy'
flag.unlink(missing_ok=True)
last = p / 'last-status.json'
last.unlink(missing_ok=True)
helper = p / 'readonly-helper'
helper.write_text('''#!/usr/bin/python
import json, pathlib, subprocess, sys
p=pathlib.Path('/tmp/snap-back-motion-regression')
real='/usr/local/bin/snap-back'
# The real helper is available through this separately bound path.
real='/tmp/snap-back-motion-regression/real-helper'
if sys.argv[1:]==['status']:
 s=json.loads(subprocess.check_output([real,'status'],text=True))
 s['busy']=(p/'busy').exists(); s['progress']='Checking backup animation' if s['busy'] else None
 t=p/'last-status.tmp';t.write_text(json.dumps({'busy':s['busy'],'time':__import__('time').time()}));t.replace(p/'last-status.json')
 print(json.dumps(s))
elif sys.argv[1:2]==['browse']:
 sys.exit(subprocess.run([real,*sys.argv[1:]]).returncode)
else:
 print('Read-only animation QA: mutation disabled',file=sys.stderr);sys.exit(1)
''')
helper.chmod(0o755)
# Bind the original helper into a second name inside our namespace; never replace
# the installed executable globally and never perform a backup during this test.
real_helper = p / 'real-helper'
real_helper.touch()
start = p / 'start.sh'
start.write_text('''#!/bin/bash
set -e
mount --bind /usr/local/bin/snap-back /tmp/snap-back-motion-regression/real-helper
mount --bind /tmp/snap-back-motion-regression/readonly-helper /usr/local/bin/snap-back
export DISPLAY=:99 ICED_BACKEND=tiny-skia HOME=/home/aneesh XDG_CURRENT_DESKTOP=KDE XDG_RUNTIME_DIR=/tmp/snap-back-motion-regression/runtime
unset WAYLAND_DISPLAY
exec /backup/snap-back/.build/debug/snap-back-gui
''')
start.chmod(0o755)
log = open(p / 'xvfb.log', 'w')
xvfb = subprocess.Popen(['Xvfb', ':99', '-screen', '0', '1180x800x24', '-ac'], stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
time.sleep(.3)
app = subprocess.Popen(['dbus-run-session', '--', 'unshare', '--mount', '--propagation', 'private', '--', str(start)], stdout=open(p/'gui.log','w'), stderr=subprocess.STDOUT, start_new_session=True)
env = {**os.environ, 'DISPLAY': ':99'}
checks = []
metrics = {'inner_differences': [], 'outside_hero_differences': [], 'brand_differences': [], 'busy_annulus_differences': []}

def cmd(*args, **kwargs):
    return subprocess.run(args, env=env, check=True, **kwargs)

def wait_status(busy):
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        assert app.poll() is None, (p/'gui.log').read_text()
        try:
            if json.loads(last.read_text())['busy'] == busy:
                time.sleep(.7)
                return
        except (FileNotFoundError, json.JSONDecodeError):
            pass
        time.sleep(.1)
    raise AssertionError('GUI did not read the expected status')

def capture(name):
    path = p / (name + '.png')
    cmd('import', '-window', wid, str(path))
    return path

def crop(path, region, suffix):
    out = path.with_name(path.stem + '-' + suffix + '.png')
    cmd('magick', str(path), '-crop', region, '+repage', str(out))
    return out

def difference(left, right):
    result = subprocess.run(['magick','compare','-metric','AE',str(left),str(right),'null:'], env=env, text=True, capture_output=True)
    assert result.returncode in (0,1), result.stderr
    return float(result.stderr.split()[0])

def check(name, ok):
    assert ok, (name, metrics)
    checks.append(name)
    print('PASS:', name, flush=True)

try:
    for _ in range(60):
        result = subprocess.run(['xdotool', 'search', '--name', '^Snap Back$'], env=env, text=True, capture_output=True)
        if result.returncode == 0:
            break
        assert app.poll() is None, (p/'gui.log').read_text()
        time.sleep(.1)
    wid = result.stdout.splitlines()[-1]
    wait_status(False)
    baseline = capture('idle-before')
    inner = crop(baseline, '56x56+324+224', 'inner')
    outside = crop(baseline, '168x65+100+80', 'outside')
    brand = crop(baseline, '238x136+0+0', 'brand')
    flag.touch()
    wait_status(True)
    annuli = []
    for i in range(5):
        frame = capture('busy-' + str(i))
        metrics['inner_differences'].append(difference(inner, crop(frame, '56x56+324+224', 'inner')))
        metrics['outside_hero_differences'].append(difference(outside, crop(frame, '168x65+100+80', 'outside')))
        metrics['brand_differences'].append(difference(brand, crop(frame, '238x136+0+0', 'brand')))
        annuli.append(crop(frame, '112x112+296+196', 'annulus'))
        time.sleep(.22)
    metrics['busy_annulus_differences'] = [difference(annuli[0], frame) for frame in annuli[1:]]
    check('Inner logo stays pixel-identical in all five busy frames', all(value == 0 for value in metrics['inner_differences']))
    check('Outside-hero triangle has no SVG trails in busy frames', all(value == 0 for value in metrics['outside_hero_differences']))
    check('Sidebar brand region is unchanged during animation', all(value == 0 for value in metrics['brand_differences']))
    check('The outer annulus animates across busy frames', all(value > 0 for value in metrics['busy_annulus_differences']))
    flag.unlink()
    wait_status(False)
    time.sleep(.8)
    after = capture('idle-after')
    check('No SVG trails remain after the animation settles', difference(outside, crop(after, '168x65+100+80', 'outside')) == 0)
    cmd('xdotool', 'mousemove', '115', '244', 'click', '1')
    time.sleep(1.3)
    restore = capture('restore-after')
    check('Navigating to Restore leaves the outside-hero region intact', difference(outside, crop(restore, '168x65+100+80', 'outside')) == 0)
    result = {'checks': checks, 'metrics': metrics, 'environment':'cachy-vm only, isolated Xvfb :99, tiny-skia', 'read_only_fixture': True, 'actual_backup_started': False}
    (p/'results.json').write_text(json.dumps(result, indent=2))
finally:
    flag.unlink(missing_ok=True)
    for proc in (app, xvfb):
        try:
            os.killpg(proc.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
