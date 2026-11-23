#!/usr/bin/env python3
"""Build actual portable product modules against the upstream committed lock.

This does not build the desktop crate or substitute its native dependencies.
Run under the shared build locks, after browser:prepare on the Linux box.
"""
import os
import hashlib
import json
import pathlib
import shutil
import shlex
import subprocess
import sys
import time
from process_survivors import descendants, invalidate_reused_groups, same_image, snapshot, survivors

root = pathlib.Path(__file__).resolve().parents[2]
def require_space():
    if shutil.disk_usage(root).free < 20 * 1024**3:
        raise RuntimeError('The shared build queue requires at least 20 GiB free before compiling this proof.')
require_space()
evidence = pathlib.Path(sys.argv[1]).resolve()
evidence.mkdir(parents=True, exist_ok=True)
source = root / '.tauri-agent-browser/upstream.git'
cargo = evidence / 'adapter-cargo'
(cargo / 'src').mkdir(parents=True, exist_ok=True)
pin = '5868f7c05020e047c00e68e4abf9253cce864f41'
for filename in ['Cargo.toml', 'Cargo.lock']:
    content = subprocess.check_output(['git', '--git-dir', str(source), 'show', f'{pin}:cli/{filename}'])
    (cargo / filename).write_bytes(content)
def module(name, path):
    return f'#[path={__import__("json").dumps(str(root / path))}] pub mod {name};\n'
wrapper = module('policy', 'src-tauri/src/browser/policy.rs') + module('proxy', 'src-tauri/src/browser/proxy.rs')
wrapper += 'pub mod browser { pub use crate::policy; pub type BoxFuture<\'a,T> = std::pin::Pin<Box<dyn std::future::Future<Output=T>+Send+\'a>>; }\n'
wrapper += 'pub use browser::BoxFuture;\n'
wrapper += module('engine', 'src-tauri/src/browser/engine.rs')
wrapper += (root / 'scripts/agent-browser/adapter-smoke.rs').read_text()
(cargo / 'src/main.rs').write_text(wrapper)
env = os.environ.copy()
resources = root / 'resources/agent-browser'
if '--fixture-network-inprocess' in sys.argv[2:] and '--fixture-no-sandbox' not in sys.argv[2:]:
    raise RuntimeError('The network-service workaround is restricted to the isolated Linux sandbox fixture.')
if '--fixture-no-sandbox' in sys.argv[2:]:
    # Only for this locked-down Linux box, after its exact pinned executable
    # has failed with "No usable sandbox". Never modify the shipped resource
    # tree or add a launch bypass to the product.
    original = resources / 'linux64'
    provenance = json.loads((original / 'provenance.json').read_text())
    resources = evidence / 'fixture-resources'
    fixture = resources / 'linux64'
    chrome = fixture / 'chrome/chrome-linux64/chrome'
    chrome.parent.mkdir(parents=True, exist_ok=True)
    fixture_flags = ['--no-sandbox']
    if '--fixture-network-inprocess' in sys.argv[2:]:
        baseline = root / '.tmp/verify/T5B/private-host/chrome-baseline/stderr.txt'
        if not baseline.exists() or 'FD ownership violation' not in baseline.read_text():
            raise RuntimeError('Record the exact no-CDP Chrome FD-ownership failure before using this fixture workaround.')
        fixture_flags.append('--enable-features=NetworkServiceInProcess2')
    chrome.write_text('#!/bin/sh\nexec ' + shlex.quote(str((original / 'chrome/chrome-linux64/chrome').resolve())) + ' ' + shlex.join(fixture_flags) + ' "$@" 2>>' + shlex.quote(str(evidence / 'chrome-stderr.txt')) + '\n')
    chrome.chmod(0o700)
    engine = fixture / 'clovy-agent-browser'
    if engine.exists() or engine.is_symlink():
        engine.unlink()
    engine.symlink_to((original / 'clovy-agent-browser').resolve())
    provenance['fixtureOnly'] = 'Synthetic loopback-only sandbox workaround; not a prepared product artifact'
    provenance['fixtureChromeFlags'] = fixture_flags
    provenance['files']['chrome/chrome-linux64/chrome']['sha256'] = hashlib.sha256(chrome.read_bytes()).hexdigest()
    (fixture / 'provenance.json').write_text(json.dumps(provenance, indent=2))
    # The Linux module proof must not connect to the box owner's SecretService
    # or desktop bus. Retain normal HOME, but point this fixture's native bus
    # environment at a run-owned, nonexistent endpoint. This is not a native
    # credential-store qualification and changes no product crypto flags.
    runtime = evidence / 'fixture-runtime'
    runtime.mkdir(mode=0o700, exist_ok=True)
    env['XDG_RUNTIME_DIR'] = str(runtime)
    env['DBUS_SESSION_BUS_ADDRESS'] = 'unix:abstract=clovy-proof-unavailable-' + str(os.getpid())
    (evidence / 'fixture-config.json').write_text(json.dumps({'chromeFlags':fixture_flags,'originalResources':str(original),'fixtureResources':str(fixture),'networkResolver':'fixture.invalid only; every external hostname refused','nativeEnvironment':'run-owned unavailable bus; owner SecretService/desktop bus never inherited; native encryption remains unproven'},indent=2))
    env['CLOVY_PROOF_SANDBOX_FIXTURE'] = '1'
env.update(CARGO_INCREMENTAL='0', CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0',
           CARGO_TARGET_DIR='/home/agent/projects/os-clovy-melhorias-20261101/.tmp/engine-target',
           CLOVY_PROOF_RESOURCES=str(resources), CLOVY_PROOF_EVIDENCE=str(evidence))
display = None
try:
    if not env.get('DISPLAY'):
        if not shutil.which('Xvfb'):
            raise RuntimeError('A private Xvfb display is required to prove human takeover.')
        display_name = f':{3000 + os.getpid() % 10000}'
        display = subprocess.Popen(['Xvfb', display_name, '-screen', '0', '1280x900x24', '-nolisten', 'tcp'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        env['DISPLAY'] = display_name
        time.sleep(.4)
    require_space()
    engine_stat = (resources / 'linux64/clovy-agent-browser').stat()
    engine_image = (engine_stat.st_dev, engine_stat.st_ino)
    known, groups = {}, {}
    observed_engine = False
    adapter = subprocess.Popen(['cargo', 'run', '--manifest-path', str(cargo / 'Cargo.toml'), '--locked'], env=env, start_new_session=True)
    while adapter.poll() is None:
        processes = snapshot()
        invalidate_reused_groups(processes, groups)
        for pid in descendants(processes, adapter.pid, known):
            info = processes.get(pid)
            if info is None:
                continue
            known[pid] = info['birth']
            if same_image(pid, engine_image):
                observed_engine = True
                groups[info['group']] = min(groups.get(info['group'], info['birth']), info['birth'])
        time.sleep(.05)
    if adapter.returncode != 0:
        raise subprocess.CalledProcessError(adapter.returncode, adapter.args)
    if not observed_engine:
        raise RuntimeError('The proof did not observe an engine process identity.')
    processes = snapshot()
    invalidate_reused_groups(processes, groups)
    remaining = survivors(processes, known, groups)
    (evidence / 'process-survivors.json').write_text(json.dumps({'identities': known, 'groups': groups, 'survivors': remaining}))
    if remaining:
        raise RuntimeError(f'The run left browser processes alive: {remaining}')
    # Also execute the actual policy/proxy tests on this portable source set.
    require_space()
    subprocess.run(['cargo', 'test', '--manifest-path', str(cargo / 'Cargo.toml'), '--locked', '--', '--test-threads=1'], env=env, check=True)
    require_space()
    subprocess.run(['cargo', 'clippy', '--manifest-path', str(cargo / 'Cargo.toml'), '--locked', '--all-targets', '--', '-D', 'warnings'], env=env, check=True)
finally:
    if display:
        display.terminate()
        display.wait(timeout=5)
