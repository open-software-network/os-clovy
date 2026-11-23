#!/usr/bin/python3
"""Source-only Chrome pipe relay for the deterministic private-host regression.

This file is never copied into a browser bundle. It forwards the owned fixture's
CDP bytes without injecting commands. An optional run-owned fault latch changes
one DOMSnapshot parameter so real Chrome rejects the request for restoration QA. A test HTTP image latch changes the page
between the host's field inventory and its first AX request.
"""
import http.client
import json
import os
from pathlib import Path
import select
import signal
import sys
import threading
import time

config = json.loads(Path(sys.argv[1]).read_text())
out = Path(config['evidence'])
read_child, write_parent = os.pipe2(os.O_CLOEXEC)
read_parent, write_child = os.pipe2(os.O_CLOEXEC)
actions = [(os.POSIX_SPAWN_DUP2, read_child, 3), (os.POSIX_SPAWN_DUP2, write_child, 4)]
for fd in (read_child, write_parent, read_parent, write_child):
    if fd not in (3, 4):
        actions.append((os.POSIX_SPAWN_CLOSE, fd))
# Preserve the parent's owned process group; never create another group/Job.
child = os.posix_spawn(config['chrome'], [config['chrome'], *sys.argv[2:]], os.environ, file_actions=actions)
os.close(read_child)
os.close(write_child)
closed = threading.Event()
state_lock=threading.Lock();pending_breakpoints={};owned_breakpoints=set();owned_pause=False
fixture_faults=[]

def write_all(fd, data):
    data = memoryview(data)
    while data:
        count = os.write(fd, data)
        if not count:
            raise BrokenPipeError('Owned fixture pipe closed')
        data = data[count:]

def output():
    global owned_pause
    buffered=b''
    try:
        while True:
            data=os.read(read_parent,65536)
            if not data:break
            buffered+=data
            if len(buffered)>16*1024*1024:raise RuntimeError('Fixture output wire limit')
            while b'\0' in buffered:
                packet,buffered=buffered.split(b'\0',1);message=json.loads(packet)
                with state_lock:
                    marker=pending_breakpoints.pop(message.get('id'),None)
                    if marker:
                        expected='br:1:0:0:'+marker
                        if message.get('result',{}).get('breakpointId')==expected:owned_breakpoints.add(expected)
                    if message.get('method')=='Debugger.paused':
                        hits=message.get('params',{}).get('hitBreakpoints',[])
                        owned_pause=isinstance(hits,list) and len(hits)==1 and hits[0] in owned_breakpoints
                    elif message.get('method')=='Debugger.resumed':owned_pause=False
                # State is recorded before the exact unchanged notification reaches host.
                # No frame/locals/URL or arbitrary breakpoint ID is retained.
                write_all(4,packet+b'\0')
    except (BrokenPipeError,OSError):
        pass
    finally:
        closed.set()

thread = threading.Thread(target=output, daemon=True)
thread.start()
buffer = b''
latched = False
frozen = False
methods = []
graceful_close_requested = False

def fixture_get(path):
    connection = http.client.HTTPConnection('127.0.0.1', config['proxyPort'], timeout=1)
    try:
        connection.request('GET', 'http://fixture.invalid/' + path)
        response = connection.getresponse()
        if response.status != 200:
            raise RuntimeError('Owned fixture latch rejected')
        return response.read(64).decode('ascii')
    finally:
        connection.close()

try:
    while not closed.is_set():
        if not select.select([3], [], [], .01)[0]:
            continue
        data = os.read(3, 65536)
        if not data:
            break
        buffer += data
        if len(buffer) > 16 * 1024 * 1024:
            raise RuntimeError('Fixture CDP packet exceeded wire limit')
        while b'\0' in buffer:
            packet, buffer = buffer.split(b'\0', 1)
            message = json.loads(packet)
            method = message.get('method', '')
            methods.append(method)
            if method == 'Browser.close':graceful_close_requested = True
            if method=='Debugger.setBreakpointByUrl':
                params=message.get('params',{});marker=params.get('url','');prefix='clovy-private-observation:'
                nonce=marker[len(prefix):] if marker.startswith(prefix) else ''
                if len(nonce)==32 and all(c in '0123456789abcdef' for c in nonce) and params.get('lineNumber')==0 and params.get('columnNumber',0)==0:
                    with state_lock:pending_breakpoints[message['id']]=marker
            elif method=='Debugger.disable':
                with state_lock:owned_pause=False;owned_breakpoints.clear()
            fault=config.get('fault')
            if method=='DOMSnapshot.captureSnapshot' and fault and Path(fault).exists():
                Path(fault).rename(str(fault)+'.consumed')
                message.setdefault('params',{})['computedStyles']=17
                packet=json.dumps(message).encode()
                fixture_faults.append({'method':method,'nativeInvalidParameter':'computedStyles integer'})
            if method == 'Page.setWebLifecycleState':
                frozen = message.get('params', {}).get('state') == 'frozen'
            if method == 'Accessibility.getFullAXTree' and not latched and Path(config['armed']).exists():
                latched = True
                fixture_get('_fixture/release')
                deadline = time.monotonic() + 1
                done = False
                while time.monotonic() < deadline:
                    if fixture_get('_fixture/status') == 'done':
                        done = True
                        break
                    time.sleep(.01)
                (out / 'race-latch.json').write_text(json.dumps({
                    'lifecycleFrozenAtAxDispatch': frozen,
                    'ownedDebuggerPauseAtAxDispatch': owned_pause,
                    'mutationDoneBeforeAx': done,
                    'injectedCdpCommands': 0,
                }, indent=2) + '\n')
            write_all(write_parent, packet + b'\0')
finally:
    (out / ('fixture-faults-'+str(os.getpid())+'.json')).write_text(json.dumps(fixture_faults,indent=2)+'\n')
    (out / ('fixture-cdp-methods-'+str(os.getpid())+'.json')).write_text(json.dumps(methods, indent=2) + '\n')
    os.close(write_parent)
    if not closed.is_set() and not graceful_close_requested:
        try:
            os.kill(child, signal.SIGTERM)
        except ProcessLookupError:
            pass
    os.waitpid(child, 0)
    os.close(read_parent)
