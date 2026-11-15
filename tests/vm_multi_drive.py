#!/usr/bin/env python3
"""VM-only multi-drive integration checks using the two saved backup disks."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

assert os.geteuid() == 0
assert subprocess.check_output(['hostname'], text=True).strip() == 'cachy-vm', 'VM-only test'
BIN = os.environ.get('SNAP_BACK_TEST_BIN', '/backup/snap-back/.build/debug/snap-back')
CONFIG = Path('/etc/snap-back/config.json')
EVIDENCE = Path('/backup/snap-back/.validation/multi-drive')
EVIDENCE.mkdir(parents=True, exist_ok=True)
original = CONFIG.read_bytes()
config = json.loads(original)
targets = config['targets']
assert len(targets) == 2, 'This check expects the two existing VM backup disks'
ids = [target['uuid'] for target in targets]
active = config['backup_uuid']
checks = []
completed = False
markers_owned = False
markers = [Path('/etc/snap-back-multidrive-check'), Path('/home/aneesh/snap-back-multidrive-check')]
recovered = []


def run(*args, ok=True, env=None):
    result = subprocess.run([BIN, *args], text=True, capture_output=True, env=env)
    if ok:
        assert result.returncode == 0, result.stderr
        return json.loads(result.stdout)
    assert result.returncode != 0, f'Unexpected success: {args}'
    return result.stderr


def check(name, condition=True):
    assert condition, name
    checks.append(name)
    print('PASS:', name, flush=True)


def points(target):
    directory = Path(target['mount']) / 'points'
    return sorted((json.loads(path.read_text()) for path in directory.glob('*/manifest.json')
                   if not path.parent.name.startswith('.')), key=lambda point: point['created_at'], reverse=True)


def latest():
    return {target['uuid']: points(target)[0]['id'] for target in targets}


def write_markers(value):
    for marker in markers:
        marker.write_text(value)
    os.chown(markers[1], 1000, 1000)


try:
    assert not any(marker.exists() for marker in markers), 'Refusing to overwrite existing markers'
    markers_owned = True
    before = latest()
    rejected = run('backup', '--drive', 'unknown-test-target', '--save-targets', ok=False)
    check('Unknown targets are refused before backups or configuration change',
          bool(rejected) and latest() == before and CONFIG.read_bytes() == original)
    unprivileged = subprocess.run(['runuser', '-u', 'aneesh', '--', BIN, 'status'], text=True, capture_output=True)
    check('Unprivileged status exposes all saved backup drives',
          unprivileged.returncode == 0 and {drive['uuid'] for drive in json.loads(unprivileged.stdout)['backup_drives']} == set(ids))

    arguments = [argument for uuid in ids for argument in ('--drive', uuid)]
    run('set-backup-drives', *arguments)
    check('Both destinations can be saved for automatic backups', set(run('status')['backup_targets']) == set(ids))
    for target in targets:
        run('use-drive', target['uuid'])
        check(f'Browsing {target["label"]} preserves backup selection', set(run('status')['backup_targets']) == set(ids))
    run('use-drive', active)

    write_markers('multi-drive first checkpoint\n')
    first_run = subprocess.Popen([BIN, 'backup', *arguments, '--save-targets', '--label', 'Multi-drive validation'],
                                 text=True, stdout=open(EVIDENCE / 'first-backup.json', 'w'),
                                 stderr=open(EVIDENCE / 'first-backup.stderr', 'w'))
    for _ in range(100):
        if run('status')['busy']:
            break
        assert first_run.poll() is None, 'Backup ended before observing the operation lock'
        time.sleep(.1)
    check('Combined backup holds one global operation lock', bool(run('backup', '--drive', ids[-1], ok=False)))
    assert first_run.wait() == 0, (EVIDENCE / 'first-backup.stderr').read_text()
    first = latest()
    for target in targets:
        point = points(target)[0]
        root = Path(target['mount']) / 'points' / point['id']
        check(f'{target["label"]}: all nine subvolumes and boot files completed', len(point['subvolumes']) == 9 and point['boot'])
        check(f'{target["label"]}: root and home marker contents verified',
              (root / 'root/snapshot/etc/snap-back-multidrive-check').read_text() == markers[0].read_text()
              and (root / 'home/snapshot/aneesh/snap-back-multidrive-check').read_text() == markers[1].read_text())

    write_markers('multi-drive second checkpoint\n')
    second_status = run('backup', '--scheduled')
    (EVIDENCE / 'second-backup.json').write_text(json.dumps(second_status, indent=2))
    second = latest()
    for target in targets:
        all_points = points(target)
        point = all_points[0]
        check(f'{target["label"]}: scheduled backup uses an independent incremental parent',
              point['id'] != first[target['uuid']] and point['kind'] == 'incremental')
        same_hour = [point for point in all_points if point['created_at'][:13] == all_points[0]['created_at'][:13]]
        check(f'{target["label"]}: retention coalesces its own hourly bucket', len(same_hour) == 1)
        run('use-drive', target['uuid'])
        listed = run('browse', point['id'], 'home', 'aneesh')
        check(f'{target["label"]}: completed point is browsable', any(entry['name'] == markers[1].name for entry in listed))
        destination = Path('/home/aneesh') / ('.snap-back-multidrive-restored-' + target['uuid'][:8])
        assert not destination.exists()
        run('restore-file', point['id'], 'home', 'aneesh/' + markers[1].name, '--to', str(destination),
            env={**os.environ, 'PKEXEC_UID': '1000'})
        recovered.append(destination)
        check(f'{target["label"]}: file recovery returns the latest content and user ownership',
              (destination / markers[1].name).read_text() == markers[1].read_text()
              and (destination / markers[1].name).stat().st_uid == 1000)
    run('use-drive', active)

    saved = CONFIG.read_bytes()
    missing = {'uuid': 'ffffffff-ffff-ffff-ffff-ffffffffffff', 'label': 'Missing QA drive',
               'mount': '/backup/snap-back-drives/missing-multidrive-qa'}
    temporary = json.loads(saved)
    temporary['targets'].append(missing)
    CONFIG.write_text(json.dumps(temporary, indent=2))
    before_partial = latest()
    try:
        failure = run('backup', '--drive', missing['uuid'], '--drive', active, '--label', 'Partial-failure validation', ok=False)
        (EVIDENCE / 'partial-failure.txt').write_text(failure)
        after_partial = latest()
        check('A missing destination is reported while a healthy destination completes',
              missing['label'] in failure and after_partial[active] != before_partial[active])
        other = next(uuid for uuid in ids if uuid != active)
        check('A one-run selection leaves unselected drives untouched', after_partial[other] == before_partial[other])
        placeholder = Path(missing['mount'])
        check('No backup is written into an unmounted destination directory', not (placeholder / 'points').exists())
    finally:
        CONFIG.write_bytes(saved)

    for marker in markers:
        marker.unlink()
    for directory in recovered:
        shutil.rmtree(directory)
    recovered.clear()
    final = run('backup', '--label', 'Multi-drive verification complete')
    check('Final backup clears the error and preserves both targets and the restore drive',
          final['last_error'] is None and set(final['backup_targets']) == set(ids)
          and json.loads(CONFIG.read_bytes())['backup_uuid'] == active)
    completed = True
    (EVIDENCE / 'results.json').write_text(json.dumps({'checks': checks, 'targets': ids,
        'restore_drive': active, 'first_points': first, 'second_points': second}, indent=2))
finally:
    if not completed:
        CONFIG.write_bytes(original)
    if markers_owned:
        for marker in markers:
            marker.unlink(missing_ok=True)
    for directory in recovered:
        shutil.rmtree(directory)
