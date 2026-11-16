#!/usr/bin/env python3
"""VM-only first-launch QA. Configuration is hidden in a private mount namespace.

Uses the real CLI for status/inventory. Mutating helper commands are blocked.
Requires a debug workspace build in /backup/snap-back/.build.
"""
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
p = Path('/tmp/snap-back-first-launch-qa')
p.mkdir(exist_ok=True)
empty = p / 'config'
empty.mkdir(exist_ok=True)
(empty / 'config.json').unlink(missing_ok=True)
runtime = p / 'runtime'
runtime.mkdir(exist_ok=True)
runtime.chmod(0o700)
calls = p / 'calls'
calls.write_text('')
real = p / 'real-helper'
real.touch()
helper = p / 'readonly-helper'
helper.write_text('''#!/usr/bin/python
import os, pathlib, sys, time
p=pathlib.Path('/tmp/snap-back-first-launch-qa')
args=sys.argv[1:]
if args not in (['status'], ['drives']):
 sys.exit('Read-only QA: mutations disabled')
with (p/'calls').open('a') as f: f.write(args[0]+'\\n')
if args==['status']: time.sleep(.7)
os.execv(str(p/'real-helper'),[str(p/'real-helper'),*args])
''')
helper.chmod(0o755)
start = p / 'start.sh'
start.write_text('''#!/bin/bash
set -e
mount --bind /backup/snap-back/.build/debug/snap-back /tmp/snap-back-first-launch-qa/real-helper
mount --bind /tmp/snap-back-first-launch-qa/readonly-helper /usr/local/bin/snap-back
mount --bind /tmp/snap-back-first-launch-qa/config /etc/snap-back
export DISPLAY=:95 ICED_BACKEND=tiny-skia HOME=/home/aneesh XDG_CURRENT_DESKTOP=KDE XDG_RUNTIME_DIR=/tmp/snap-back-first-launch-qa/runtime
unset WAYLAND_DISPLAY
exec /backup/snap-back/.build/debug/snap-back-gui
''')
start.chmod(0o755)
env = {**os.environ, 'DISPLAY': ':95'}
checks = []


def cmd(*args, **kwargs):
    return subprocess.run(args, env=env, check=True, **kwargs)


def capture(name):
    path = p / (name + '.png')
    cmd('import', '-window', wid, str(path))
    return path


def words(path, psm='11'):
    output = subprocess.check_output(['tesseract', str(path), 'stdout', '--psm', psm, 'tsv'], stderr=subprocess.DEVNULL, text=True)
    return list(csv.DictReader(io.StringIO(output), delimiter='\t'))


def wait_text(expected, name):
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        assert app.poll() is None, (p / 'gui.log').read_text()
        path = capture(name)
        found = words(path)
        content = ' '.join(w['text'] for w in found)
        if expected in content:
            return path, found
        time.sleep(.3)
    raise AssertionError((expected, content))


def click_word(found, label):
    w = next(w for w in found if w['text'] == label)
    cmd('xdotool', 'mousemove', '--window', wid,
        str(int(w['left']) + int(w['width']) // 2),
        str(int(w['top']) + int(w['height']) // 2))
    time.sleep(.3)
    cmd('xdotool', 'click', '1')
    time.sleep(.5)
    cmd('xdotool', 'mousemove', '1175', '795')


def difference(left, right):
    result = subprocess.run(['magick', 'compare', '-metric', 'AE', str(left), str(right), 'null:'], text=True, capture_output=True)
    assert result.returncode in (0, 1), result.stderr
    return float(result.stderr.split()[0])


def check(label, condition):
    assert condition, label
    checks.append(label)
    print('PASS:', label, flush=True)


xvfb = subprocess.Popen(['Xvfb', ':95', '-screen', '0', '1180x800x24', '-ac'], stdout=open(p/'xvfb.log', 'w'), stderr=subprocess.STDOUT, start_new_session=True)
time.sleep(.3)
app = subprocess.Popen(['dbus-run-session', '--', 'unshare', '--mount', '--propagation', 'private', '--', str(start)], stdout=open(p/'gui.log', 'w'), stderr=subprocess.STDOUT, start_new_session=True)
try:
    for _ in range(100):
        result = subprocess.run(['xdotool', 'search', '--onlyvisible', '--name', '^Snap Back$'], env=env, text=True, capture_output=True)
        if result.returncode == 0:
            break
        assert app.poll() is None
        time.sleep(.1)
    wid = result.stdout.splitlines()[-1]
    cmd('xdotool', 'mousemove', '1175', '795')
    baseline, found = wait_text('Set up your first backup', 'first-launch')
    content = ' '.join(w['text'] for w in found)
    check('First launch offers setup without an error or retry panel',
          not any(term in content.lower() for term in ['unavailable', 'error', 'installer', 'retry', 'try again']))
    button_crop = p/'setup-button.png'
    cmd('magick', str(baseline), '-crop', '300x60+400+290', '+repage', '-resize', '300%', str(button_crop))
    button_text = subprocess.check_output(['tesseract', str(button_crop), 'stdout', '--psm', '6'], text=True, stderr=subprocess.DEVNULL)
    check('Primary drive-selection button is visible', 'Choose backup drive' in button_text)
    # Capture both idle frames and the deliberately delayed requests across two polls.
    diffs = []
    for i in range(24):
        time.sleep(.4)
        diffs.append(difference(baseline, capture('poll-' + str(i))))
    check('All 24 frames stay pixel-identical across background refreshes', all(value == 0 for value in diffs))
    check('Real helper was polled at least three times', calls.read_text().splitlines().count('status') >= 3)
    # The primary button is at a fixed location in this 1180x800 fixture.
    cmd('xdotool', 'mousemove', '500', '320')
    time.sleep(.3)
    cmd('xdotool', 'click', '1')
    time.sleep(.5)
    cmd('xdotool', 'mousemove', '1175', '795')
    _, picker = wait_text('Saved backup drives', 'drive-picker')
    check('Choose backup drive opens real drive inventory', 'drives' in calls.read_text().splitlines())
    check('Drive picker keeps refresh and back in the same row',
          abs(int([w for w in picker if w['text'] == 'Back' and int(w['left']) > 238][-1]['top']) -
              int([w for w in picker if w['text'] == 'Refresh'][-1]['top'])) < 10)
    click_word([w for w in picker if int(w['left']) > 238 and int(w['top']) > 280], 'Back')
    wait_text('Set up your first backup', 'back-to-setup')
    # A malformed config is a genuine failure, distinct from a fresh install.
    (empty/'config.json').write_text('{broken')
    failure_path, failure = wait_text('check backups', 'configuration-error')
    content = ' '.join(w['text'] for w in failure)
    check('Malformed config displays one error card and no duplicate setup card',
          content.count('check backups') == 1 and 'Set up your first backup' not in content)
    retry_crop = p/'retry-button.png'
    cmd('magick', str(failure_path), '-crop', '107x40+292+286', '+repage', '-resize', '300%', str(retry_crop))
    retry_words = words(retry_crop, '7')
    again = next(w for w in retry_words if w['text'] == 'again')
    choose = next(w for w in failure if w['text'] == 'Choose' and int(w['left']) > 238)
    check('Error recovery buttons share a row', abs(286 + int(again['top']) / 3 - int(choose['top'])) < 10)
    (empty/'config.json').unlink()
    wait_text('Set up your first backup', 'recovered-setup')
    check('Successful refresh returns from a real error to setup', True)
    # Copy the valid VM configuration into the isolated directory to exercise
    # the same status transition as successful setup, without formatting a disk.
    (empty/'config.json').write_bytes(Path('/etc/snap-back/config.json').read_bytes())
    _, ready = wait_text('AUTOMATIC BACKUPS', 'configured-overview')
    content = ' '.join(w['text'] for w in ready)
    check('A configured status replaces setup with the actual backup overview',
          'Set up your first backup' not in content and 'Every hour' in content)
    (p/'results.json').write_text(json.dumps({'checks': checks, 'frame_differences': diffs,
        'environment': 'cachy-vm, private mount namespace, Xvfb :95, tiny-skia',
        'real_configuration_changed': False, 'backup_or_format_started': False}, indent=2))
finally:
    for proc in (app, xvfb):
        try:
            os.killpg(proc.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
