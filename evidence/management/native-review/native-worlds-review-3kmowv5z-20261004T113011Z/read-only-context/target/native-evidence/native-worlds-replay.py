#!/usr/bin/env python3
"""Replay the maintained onboarding/capture worlds from a recorded Debian package."""
from pathlib import Path
import importlib.util
import json
import shutil
import subprocess
import sys
import tempfile
import time

repository = Path('/home/lucas/Developer/personal/cutokyo-community')
spec = importlib.util.spec_from_file_location('native_runner', repository / 'ui/scripts/native-e2e.py')
if spec is None or spec.loader is None:
    raise RuntimeError('Native runner module is unavailable')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
manifest_path = Path(sys.argv[1]).resolve(strict=True)
manifest = json.loads(manifest_path.read_text())
package = Path(manifest['retained_package'])
base = Path(manifest['root'])
if runner.sha256(package) != manifest['package_sha256']:
    raise RuntimeError('Recorded Debian package changed')
for key, name in [('receiver', 'cutokyo'), ('synthetic_harness', 'claude')]:
    if runner.sha256(base / 'bin' / name) != manifest[key + '_sha256']:
        raise RuntimeError('Recorded compiled test input changed')
evidence = Path(tempfile.mkdtemp(prefix='native-worlds-replay-', dir=repository / 'target/native-evidence'))
shutil.copy2(manifest_path, evidence / 'package-launch.json')
if runner.sha256(manifest_path) != runner.sha256(evidence / 'package-launch.json'):
    raise RuntimeError('Recorded package provenance changed during preservation')
record = {
    'source_manifest': str(manifest_path),
    'package_sha256': manifest['package_sha256'],
    'evidence': str(evidence),
    'purpose': 'Execute unchanged maintained browse/capture setup/capture history worlds with repaired phase orchestration. Not fresh aggregate acceptance.',
    'fresh_aggregate_acceptance': False,
}
start = time.monotonic()
code = 0
try:
    runner.run_onboarding_worlds(base, package, evidence, runner.build_environment(base))
except Exception as error:
    code = error.returncode if isinstance(error, subprocess.CalledProcessError) else 1
    record['error'] = str(error)
    raise
finally:
    record.update(exit_code=code, duration_seconds=round(time.monotonic() - start, 2))
    with (evidence / 'execution-record.json').open('x') as stream:
        json.dump(record, stream, indent=2)
    print(json.dumps(record, indent=2), flush=True)
