#!/usr/bin/env python3
"""Exercise actual private-host observations against native headed page state.

Only a run-owned Xvfb, profile and fixture.invalid proxy are used. The page reports
its real visibility/focus/RAF over a versioned HTTP handshake; no CDP visibility
override, foreground restoration, model API or fake visibility getter is used.
"""
import ast
import base64
import http.server
import json
import os
from pathlib import Path
import re
import secrets
import select
import shlex
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
import uuid
import zlib
from fixture_grant import persistent_fixture_grant

binary, chrome, evidence = map(Path, sys.argv[1:4])
evidence.mkdir(parents=True, exist_ok=True)
free=shutil.disk_usage(Path.cwd()).free
admission={'bothLocksHeldByCaller':True,'freeBytes':free,'minimumBytes':20*1024**3,'admitted':free>=20*1024**3}
(evidence/'admission.json').write_text(json.dumps(admission,indent=2)+'\n')
if not admission['admitted']:raise SystemExit(78)
# Reuse the real host smoke's PNG decoder without executing its harness.
smoke = Path(__file__).with_name('private-host-smoke.py')
definitions = [node for node in ast.parse(smoke.read_text()).body
               if isinstance(node, ast.FunctionDef) and node.name in ('decoded_png', 'masked_pixels', 'region_counts')]
exec(compile(ast.Module(body=definitions, type_ignores=[]), str(smoke), 'exec'))

lock = threading.Lock()
versions = {'first': 0, 'second': 0}
states = {}
state_trace = []
page = '''<!doctype html><title>Visibility host fixture</title>
<h1>Public visibility context</h1><input aria-label=Query value=SYNTHETIC_VISIBILITY_PRIVATE style="position:fixed;left:50px;top:180px;PAINT">
<button id=button>Counter control</button><output id=status></output>
<script>
const parameters=new URL(location.href).searchParams;
const who=parameters.get('tab'),generation=parameters.get('generation');
let frames=0,clicks=0,version=-1;
function report(){const state={tab:who,generation,version,visibility:document.visibilityState,focus:document.hasFocus(),frames,clicks,clock:performance.now(),date:Date.now()};document.getElementById('status').textContent=state.visibility+' focus:'+state.focus+' frames:'+frames+' clicks:'+clicks;fetch('http://fixture.invalid/_visibility-report?'+new URLSearchParams(state));}
function tick(){frames++;requestAnimationFrame(tick)}requestAnimationFrame(tick);
document.addEventListener('visibilitychange',report);window.addEventListener('focus',report);window.addEventListener('blur',report);
setInterval(()=>fetch('http://fixture.invalid/_visibility-poll?tab='+who).then(r=>r.json()).then(r=>{if(version!==r.version){version=r.version;report()}}),50);
document.getElementById('button').onclick=()=>{clicks++;report()};report();
</script>'''

class Proxy(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        target = urllib.parse.urlsplit(self.path)
        if target.hostname != 'fixture.invalid':
            self.send_error(403); return
        query = urllib.parse.parse_qs(target.query)
        if target.path == '/_visibility-poll':
            with lock: body = json.dumps({'version': versions[query['tab'][0]]})
        elif target.path == '/_visibility-report':
            state = {key: value[0] for key, value in query.items()}
            with lock:
                previous=states.get(state['tab'],{})
                same=previous.get('generation')==state.get('generation')
                current_generation=state.get('generation')==generations.get(state['tab'])
                monotonic=not same or all(int(state.get(key,-1))>=int(previous.get(key,-1)) for key in ('version','frames','clicks'))
                if current_generation and monotonic:
                    state['receivedAt']=time.monotonic()
                    states[state['tab']]=state
            body = 'ok'
        elif target.path == '/visibility':
            body = page.replace('PAINT', 'text-shadow:120px 0 black' if query.get('paint') else '')
        else:
            self.send_error(404); return
        encoded = body.encode()
        self.send_response(200); self.send_header('Content-Type', 'text/html')
        self.send_header('Content-Length', str(len(encoded))); self.end_headers()
        try: self.wfile.write(encoded)
        except (BrokenPipeError, ConnectionResetError): pass
    def do_CONNECT(self): self.send_error(403)
    def log_message(self, *args): pass

proxy = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Proxy)
threading.Thread(target=proxy.serve_forever, daemon=True).start()
profile = Path(tempfile.mkdtemp(prefix='clovy-host-visibility-'))
grant, profile_lease = persistent_fixture_grant(profile / 'browser')
runtime = profile / 'runtime'; runtime.mkdir(mode=0o700)
display = ':' + str(6000 + os.getpid() % 10000)
env = {'HOME': str(profile), 'PATH': '/usr/bin:/bin', 'DISPLAY': display,
       'XDG_RUNTIME_DIR': str(runtime),
       'DBUS_SESSION_BUS_ADDRESS': 'unix:abstract=clovy-visibility-unavailable-' + str(os.getpid())}
proc = None; xvfb = None; receipts = []; checks = []; clock_checks = []; stderr = None
generations = {}

def state(tab, visible, focus, advance=False, clicks=None):
    with lock:
        before = dict(states.get(tab, {})); versions[tab] += 1; version = versions[tab]
        if before.get('generation')!=generations[tab]:before={}
    deadline = time.monotonic() + 4
    while time.monotonic() < deadline:
        with lock: current = dict(states.get(tab, {}))
        if (current.get('generation') == generations[tab]
            and int(current.get('version', -1)) >= version
            and current.get('visibility') == visible
            and current.get('focus') == str(focus).lower()
            and (not advance or int(current.get('frames', 0)) > int(before.get('frames', 0)))
            and (clicks is None or int(current.get('clicks', -1)) == clicks)):
            state_trace.append(current)
            (evidence / 'native-states.json').write_text(json.dumps(state_trace, indent=2))
            return current
        time.sleep(.02)
    raise AssertionError({'expectedTab': tab, 'visibility': visible, 'focus': focus,
                          'version': version, 'generation': generations[tab], 'observed': current})

try:
    xvfb = subprocess.Popen(['Xvfb', display, '-screen', '0', '1280x900x24', '-nolisten', 'tcp'],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(.3)
    if xvfb.poll() is not None: raise RuntimeError('Owned Xvfb failed')
    fault = profile / 'native-error'
    relay_config = profile / 'relay.json'
    relay_config.write_text(json.dumps({'chrome': str(chrome.resolve()), 'proxyPort': proxy.server_port,
        'evidence': str(evidence.resolve()), 'armed': str(profile / 'unarmed-race'), 'fault': str(fault)}))
    relay = Path(__file__).resolve().parent / 'private-host/cdp_fixture.py'
    wrapper = profile / 'fixture-chrome'
    wrapper.write_text('#!/bin/sh\nexec /usr/bin/python3 ' + shlex.quote(str(relay)) + ' ' + shlex.quote(str(relay_config)) + ' "$@"\n')
    wrapper.chmod(0o700)
    stderr = (evidence / 'host-stderr.txt').open('w')
    proc = subprocess.Popen([str(binary.resolve())], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=stderr, text=True, bufsize=1, env=env, start_new_session=True)
    def wire(request):
        proc.stdin.write(json.dumps(request) + '\n'); proc.stdin.flush()
        if not select.select([proc.stdout], [], [], 30)[0]: raise TimeoutError('Host deadline')
        line = proc.stdout.readline()
        if not line: raise RuntimeError('Host exited')
        return json.loads(line)
    nonce = secrets.token_hex(32)
    assert wire({'protocol': 1, 'token': nonce, 'grant': grant}) == {'protocol': 1, 'token': nonce}
    def call(action, ok=True, **arguments):
        receipt = wire({'id': len(receipts) + 1, 'action': action, **arguments}); receipts.append(receipt)
        (evidence / 'receipts.json').write_text(json.dumps(receipts, indent=2))
        if ok and 'error' in receipt: raise AssertionError((action, receipt))
        return receipt.get('data', {}) if ok else receipt
    call('start', chromePath=str(wrapper), profilePath=str(profile / 'browser'),
         proxyPort=proxy.server_port, persistent=True, headed=False)
    takeover = call('takeover'); assert takeover['suspended'] and takeover['mode'] == 'visible'
    call('resume'); first = takeover['tabId']
    def url(tab, paint=False):
        generations[tab] = uuid.uuid4().hex
        return 'http://fixture.invalid/visibility?' + urllib.parse.urlencode({'tab': tab,
            'generation': generations[tab], **({'paint': '1'} if paint else {})})
    call('navigate', tab_id=first, url=url('first'))
    second = call('tab_open', url=url('second'))['tabId']
    deadline = time.monotonic() + 5; window = None
    while time.monotonic() < deadline:
        match = subprocess.run(['xdotool', 'search', '--onlyvisible', '--name', '^Visibility host fixture( - .*)?$'],
                               env=env, capture_output=True, text=True)
        if match.returncode == 0: window = match.stdout.splitlines()[0]; break
        time.sleep(.05)
    assert window
    # Physical setup only: select the first owned tab before measuring. No
    # operation restores focus/visibility by bringing a page to the foreground.
    subprocess.run(['xdotool', 'windowfocus', '--sync', window], env=env, check=True)
    subprocess.run(['xdotool', 'key', '--clearmodifiers', 'ctrl+shift+Tab'], env=env, check=True)
    state('first', 'visible', True, True)

    def exercise(tab_id, who, visible, focused):
        def preserved():
            observed=state(who,visible,focused,visible=='visible')
            # Compare the page's actual native clocks with the independent
            # fixture clock across a measurement interval. This is not a
            # readiness delay or an emulated visibility/clock observation.
            time.sleep(.4)
            after=state(who,visible,focused,visible=='visible')
            elapsed=(after['receivedAt']-observed['receivedAt'])*1000
            deltas={key:float(after[key])-float(observed[key]) for key in ('clock','date')}
            measurement={'tab':who,'visibility':visible,'elapsedMs':elapsed,'nativeDeltasMs':deltas}
            clock_checks.append(measurement)
            (evidence/'native-clock-pacing.json').write_text(json.dumps(clock_checks,indent=2)+'\n')
            assert elapsed>=350 and all(abs(delta-elapsed)<=max(150,elapsed*.25) for delta in deltas.values()),measurement
            if not focused:state('second','visible',True,True)
            return after
        before = state(who, visible, focused, visible == 'visible')
        snap = call('snapshot', tab_id=tab_id)['snapshot']
        assert 'SYNTHETIC_VISIBILITY_PRIVATE' not in snap
        preserved(); checks.append(who + ':snapshot')
        reference = re.search(r'"Counter control"[^\n]*ref=([a-f0-9]{32})', snap).group(1)
        expected = call('inspect_reference', tab_id=tab_id, ref=reference)
        assert expected['interactionClass'] == 'approval_required'
        preserved(); checks.append(who + ':inspect')
        if visible == 'visible':
            image = call('screenshot', tab_id=tab_id); png = base64.b64decode(image['base64'])
            assert masked_pixels(png) > 1000
            public=region_counts(decoded_png(png),(0,0,600,90));assert public['black']>30 and public['masked']<public['pixels']*.05
            (evidence / (who + '-' + visible + '-actual-host.png')).write_bytes(png)
            checks.append(who + ':visible-screenshot-png')
        else:
            refused = call('screenshot', ok=False, tab_id=tab_id)
            assert refused.get('delivery') == 'not_attempted' and refused.get('error', {}).get('code') == 'screenshot_hidden' and 'data' not in refused
            checks.append(who + ':hidden-screenshot-unavailable-no-image')
        preserved()
        call('guarded_action', tab_id=tab_id, ref=reference, operation='click', expected=expected)
        state(who, visible, focused, visible == 'visible', int(before['clicks']) + 1)
        preserved()
        checks.append(who + ':actual-button-effect')
        call('navigate', tab_id=tab_id, url=url(who, paint=True))
        state(who, visible, focused, visible == 'visible')
        refused = call('screenshot', ok=False, tab_id=tab_id)
        assert refused.get('delivery') == 'not_attempted' and refused.get('error', {}).get('code') == ('redaction_failed' if visible == 'visible' else 'screenshot_hidden') and 'data' not in refused
        preserved(); checks.append(who + (':paint-refusal' if visible == 'visible' else ':hidden-paint-context-unavailable'))
        call('navigate', tab_id=tab_id, url=url(who))
        state(who, visible, focused, visible == 'visible')
        fault.touch()
        failed = call('snapshot', ok=False, tab_id=tab_id)
        assert failed.get('delivery') == 'not_attempted' and failed.get('error') and 'data' not in failed
        preserved(); checks.append(who + ':native-CDP-error')
        assert fault.with_name(fault.name + '.consumed').exists()
    exercise(first, 'first', 'visible', True)
    # The actual second tab becomes foreground through normal owned tab creation;
    # use the keyboard once to establish the original hidden-control condition.
    subprocess.run(['xdotool', 'key', '--clearmodifiers', 'ctrl+Tab'], env=env, check=True)
    state('second', 'visible', True, True); state('first', 'hidden', False)
    exercise(first, 'first', 'hidden', False)
    state('second', 'visible', True, True)
    call('shutdown'); proc.wait(timeout=5); assert proc.returncode == 0
    method_logs=list(evidence.glob('fixture-cdp-methods-*.json'))
    assert method_logs
    assert all('Emulation.setVirtualTimePolicy' not in json.loads(path.read_text()) for path in method_logs)
    result = {'passed': True, 'receipts': len(receipts), 'checks': checks,
              'realHost': True, 'visibilityEmulation': False, 'foregroundRestoration': False,
              'virtualTimeCommands':0,'nativeClockMeasurements':len(clock_checks),'nativeHiddenCaptureUnavailable':True}
    (evidence / 'result.json').write_text(json.dumps(result, indent=2) + '\n'); print(json.dumps(result))
finally:
    if proc:
        try: os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError: pass
        proc.wait()
    if stderr: stderr.close()
    if xvfb: xvfb.terminate(); xvfb.wait(timeout=5)
    proxy.shutdown(); profile_lease.close(); shutil.rmtree(profile)
