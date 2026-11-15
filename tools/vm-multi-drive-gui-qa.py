#!/usr/bin/env python3
"""VM-only split-button checks in a separate Xvfb display."""
import csv
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import time

assert os.geteuid() == 0
assert subprocess.check_output(['hostname'], text=True).strip() == 'cachy-vm'
out = Path('/tmp/snap-back-multi-drive-gui')
out.mkdir(exist_ok=True)
xvfb = subprocess.Popen(['Xvfb', ':98', '-screen', '0', '1180x800x24', '-ac'], stdout=open(out/'xvfb.log','w'), stderr=subprocess.STDOUT, start_new_session=True)
time.sleep(1.2)
env = {**os.environ, 'DISPLAY': ':98', 'HOME': '/home/aneesh', 'XDG_CURRENT_DESKTOP': 'KDE', 'ICED_BACKEND': 'tiny-skia'}
env.pop('WAYLAND_DISPLAY', None)
app = subprocess.Popen(['dbus-run-session', '--', '/backup/snap-back/.build/debug/snap-back-gui'], env=env, stdout=open(out/'gui.log','w'), stderr=subprocess.STDOUT, start_new_session=True)
for _ in range(50):
    result = subprocess.run(['xdotool', 'search', '--name', '^Snap Back$'],env=env,text=True,capture_output=True)
    if result.returncode==0:break
    assert app.poll() is None
    time.sleep(.1)
wid = result.stdout.splitlines()[-1]
pid = int(subprocess.check_output(['xdotool','getwindowpid',wid],env=env,text=True))
time.sleep(1)
checks = []


def click(x, y):
    subprocess.run(['xdotool', 'mousemove', str(x), str(y), 'click', '1'], env=env, check=True)
    time.sleep(1.2)


def capture(name):
    path = out / (name + '.png')
    subprocess.run(['import', '-window', wid, str(path)], env=env, check=True)
    return path


def text(path):
    return ' '.join(subprocess.check_output(['tesseract', str(path), 'stdout', '--psm', '11'], stderr=subprocess.DEVNULL, text=True).split())


def check(name, condition):
    assert condition, name
    checks.append(name)
    print('PASS:', name, flush=True)


def check_text(name, screenshot, expected, present=True):
    # The software renderer can finish after the input event. Wait for its
    # visible state instead of relying on a fixed screenshot delay.
    deadline = time.monotonic() + 10
    while True:
        actual = text(capture(screenshot))
        if (expected in actual) == present:
            check(name, True)
            return
        assert time.monotonic() < deadline, (name, actual)
        time.sleep(.2)


def latest():
    config = json.loads(Path('/etc/snap-back/config.json').read_text())
    return {target['uuid']: max(json.loads(path.read_text())['id'] for path in (Path(target['mount']) / 'points').glob('*/manifest.json') if not path.parent.name.startswith('.')) for target in config['targets']}


try:
    before = latest()
    click(1105, 240)
    check_text('Dropdown shows both remembered checkbox destinations', 'menu-both', '2 drives selected')
    click(965, 350)
    check_text('Clicking a drive label toggles it without closing the dropdown', 'menu-one', '1 drive selected')
    click(965, 430)
    check_text('All checkboxes can be cleared without resetting the draft', 'menu-none', 'Select at least one drive')
    click(700, 130)
    check_text('Clicking outside closes the dropdown', 'outside-dismissed', 'Back up to', present=False)
    click(1030, 240)
    time.sleep(1.2)
    status = json.loads(subprocess.check_output(['/usr/local/bin/snap-back', 'status'], text=True))
    check('Backup is disabled when no drives are selected', not status['busy'] and latest() == before)
    click(1105, 240)
    click(881, 353)
    check_text('Clicking the checkbox toggles once and keeps the dropdown open', 'checkbox-one', '1 drive selected')
    click(881, 441)
    check_text('Both drives can be reselected', 'menu-final', '2 drives selected')
    subprocess.run(['xdotool', 'windowfocus', '--sync', wid, 'key', 'Escape'], env=env, check=True)
    time.sleep(.4)
    check_text('Escape dismisses the dropdown', 'escape-dismissed', 'Back up to', present=False)
    click(1030, 240)
    observed_busy = False
    for _ in range(180):
        status = json.loads(subprocess.check_output(['/usr/local/bin/snap-back', 'status'], text=True))
        observed_busy = observed_busy or status['busy']
        if observed_busy and not status['busy']:
            break
        time.sleep(.5)
    check('One GUI backup click completes on both selected disks', observed_busy and not status['busy'] and all(value != before[uuid] for uuid, value in latest().items()))
    check('The GUI selection is remembered for hourly backups', len(status['backup_targets']) == 2 and status['last_error'] is None)
    time.sleep(.5)
    capture('backup-success')
    click(115, 244)
    time.sleep(1)
    toolbar = capture('restore-toolbar')
    rows = csv.DictReader(io.StringIO(subprocess.check_output(['tesseract', str(toolbar), 'stdout', '--psm', '11', 'tsv'], stderr=subprocess.DEVNULL, text=True)), delimiter='\t')
    found = {}
    for row in rows:
        word = ''.join(c for c in row['text'].lower() if c.isalpha())
        if word in ('home', 'root', 'up') and int(row['left']) >= 500:
            found.setdefault(word, int(row['top']) + int(row['height']) / 2)
    check('Home, Root, and Up share one toolbar row', len(found) == 3 and max(found.values()) - min(found.values()) < 6)
    (out / 'results.json').write_text(json.dumps({'checks': checks, 'before': before, 'after': latest(), 'environment': 'cachy-vm only, isolated Xvfb/X11'}, indent=2))
finally:
    for process in (app, xvfb):
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
