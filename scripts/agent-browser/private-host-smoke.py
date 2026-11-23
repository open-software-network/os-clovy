#!/usr/bin/env python3
"""Exercise the shipped private host against a synthetic site, without external traffic.
Usage: python3 private-host-smoke.py HOST_BINARY PINNED_CHROME EVIDENCE_DIR
The private proxy answers fixture.invalid locally and rejects every other host.
"""
import base64, http.server, json, os, pathlib, re, secrets, select, shutil, signal, subprocess, sys, tempfile, threading, time, urllib.parse, struct, zlib
from fixture_grant import persistent_fixture_grant
binary,chrome,evidence=map(pathlib.Path,sys.argv[1:4])
regressions_only = sys.argv[4:] == ["--privacy-regressions-only"]
assert not sys.argv[4:] or regressions_only
evidence.mkdir(parents=True,exist_ok=True)
def decoded_png(png):
    offset=8;packed=b''
    while offset<len(png):
        length=struct.unpack('>I',png[offset:offset+4])[0];kind=png[offset+4:offset+8];data=png[offset+8:offset+8+length];offset+=length+12
        if kind==b'IHDR':width,height,depth,color,_,_,interlace=struct.unpack('>IIBBBBB',data);assert (depth,color,interlace)==(8,6,0)
        if kind==b'IDAT':packed+=data
    raw=zlib.decompress(packed);stride=width*4;prev=bytearray(stride);offset=0;rows=[]
    def paeth(a,b,c):
        p=a+b-c;pa,pb,pc=abs(p-a),abs(p-b),abs(p-c)
        return a if pa<=pb and pa<=pc else b if pb<=pc else c
    for _ in range(height):
        mode=raw[offset];offset+=1;row=bytearray(raw[offset:offset+stride]);offset+=stride
        for i in range(stride):
            a=row[i-4] if i>=4 else 0;b=prev[i];c=prev[i-4] if i>=4 else 0
            add=0 if mode==0 else a if mode==1 else b if mode==2 else (a+b)//2 if mode==3 else paeth(a,b,c)
            row[i]=(row[i]+add)&255
        rows.append(bytes(row));prev=row
    return width,height,rows
def masked_pixels(png):
    return sum(row[i:i+4]==b'\x18\x18\x18\xff' for row in decoded_png(png)[2] for i in range(0,len(row),4))
def region_counts(decoded,rect):
    width,height,rows=decoded;x,y,w,h=rect
    pixels=[rows[yy][xx*4:xx*4+4] for yy in range(y,min(y+h,height)) for xx in range(x,min(x+w,width))]
    return {'masked':sum(p==b'\x18\x18\x18\xff' for p in pixels),'black':sum(p==b'\0\0\0\xff' for p in pixels),'pixels':len(pixels)}
page='''<!doctype html><title>Private host fixture</title><h1>Fixture</h1>
<form id="fixture-form" action="http://fixture.invalid/submitted"><label for="query">Query</label><input id="query" name="query"><button type="submit">Submit</button></form><input form="fixture-form" aria-label="External form" name="associated">
<input aria-label="Readonly" readonly value="synthetic-readonly-secret" oninput="document.getElementById('result').textContent='readonly mutated'">
<label for="password">Password</label><input id="password" type="password" value="synthetic-password-secret">
<label for="otp">Code</label><input id="otp" autocomplete="one-time-code" value="synthetic-otp-secret">
<input aria-label="Card" autocomplete="cc-number" value="synthetic-card-secret"><div contenteditable="true">synthetic-editable-secret</div><input name="otp" aria-label="Alias"><input aria-label="KeySwap" id="keySwap" onkeydown="this.outerHTML='<input id=keySwap aria-label=KeySwap>'">
<button id="save" onclick="document.getElementById('result').textContent='saved'">Save</button>
<button onclick="document.getElementById('save').outerHTML='<button id=save>Save</button>'">Replace</button>
<div id="result"></div><span id="buy-label">Buy</span><button aria-label="Search" aria-labelledby="buy-label"><span aria-hidden="true">+</span></button><a href="http://fixture.invalid/two">Second page</a>'''
privacy_overflow="""<!doctype html><title>Overflow privacy fixture</title><style>
#editor{position:fixed;left:1050px;top:650px;width:20px;height:20px}#child{position:fixed;left:600px;top:300px;white-space:nowrap}#editor::after{content:'synthetic-pseudo-secret';position:fixed;left:600px;top:360px;white-space:nowrap}#context{position:fixed;left:600px;top:500px}
</style><div id=editor contenteditable=true><span id=child>synthetic-overflow-secret</span><span id=closed></span></div><p id=context>Public context remains readable</p><input aria-label='Normal control' value='normal-secret' style='position:fixed;left:100px;top:180px'>
<script>document.getElementById('closed').attachShadow({mode:'closed'}).innerHTML='<span style="position:fixed;left:600px;top:420px;white-space:nowrap">synthetic-closed-secret</span>'</script>"""
privacy_passwords="""<!doctype html><title>Password token fixture</title><output id=events>Events:0</output><script>function record(){const e=document.getElementById('events');e.textContent='Events:'+(+e.textContent.split(':')[1]+1)}</script>
<input aria-label='Shown one' value='password' autocomplete='section-account current-password' onclick='record()' onfocus='record()' oninput='record()' onkeydown='record()'><input aria-label='Shown two' autocomplete='section-user new-password' onclick='record()' onfocus='record()' oninput='record()' onkeydown='record()'><input aria-label='Neutral three' id='billing_expiry' onclick='record()' onfocus='record()' oninput='record()' onkeydown='record()'><input aria-label='Neutral four' role='authentication-code' onclick='record()' onfocus='record()' oninput='record()' onkeydown='record()'><button onclick='record()'>Ordinary action</button>"""
privacy_race="""<!doctype html><title>Race privacy fixture</title><h1>Race fixture</h1><input data-race id='RACE_SECRET_A' name='RACE_SECRET_A' aria-label='RACE_SECRET_A' value='RACE_SECRET_A' onkeydown="document.getElementById('edited').textContent='Edited:1'"><input type=hidden id=hidden value='HIDDEN_SECRET_A'><button id=copy aria-label='RACE_SECRET_A'></button><button id=hidden-copy aria-label='HIDDEN_SECRET_A'></button><span id=closed></span><input aria-label='Query stable' id=ordinary value='ordinary-private-value'><output id=edited>Edited:0</output><button onclick="const e=document.querySelector('[data-race]');e.value='RACE_SECRET_C';e.id=e.name='RACE_SECRET_C';e.setAttribute('aria-label','RACE_SECRET_C')">Mutate labels</button>
<img src='http://fixture.invalid/privacy-latch' onload="const e=document.querySelector('[data-race]');e.value='RACE_SECRET_B';e.id=e.name='RACE_SECRET_B';e.setAttribute('aria-label','RACE_SECRET_B');document.getElementById('copy').setAttribute('aria-label','RACE_SECRET_B');document.getElementById('hidden').value='HIDDEN_SECRET_B';document.getElementById('hidden-copy').setAttribute('aria-label','HIDDEN_SECRET_B');const n=document.createElement('input');n.value=n.name=n.id='NEW_SECRET_B';n.setAttribute('aria-label','NEW_SECRET_B');document.body.append(n);document.getElementById('closed').attachShadow({mode:'closed'}).innerHTML='<input value=NEW_CLOSED_SECRET_B aria-label=NEW_CLOSED_SECRET_B>';fetch('http://fixture.invalid/privacy-race-done')">"""
privacy_empty="""<!doctype html><title>Empty field fixture</title><h1>Empty input fixture</h1><form action='http://fixture.invalid/privacy-empty-done'><label>Empty input<input name=input id=empty-input oninput="document.getElementById('changed').textContent='Changed:'+(+document.getElementById('changed').textContent.split(':')[1]+1)"></label><label>Empty textarea<textarea name=textarea id=empty-textarea oninput="document.getElementById('changed').textContent='Changed:'+(+document.getElementById('changed').textContent.split(':')[1]+1)"></textarea></label><button>Submit empties</button></form><output id=changed>Changed:0</output><p>Public empty context useful</p>"""
long_value='LONG_PRIVATE_'+('x'*1300)
privacy_race=privacy_race.replace("<img src=", "<input aria-label='Long field' value='"+long_value+"'><button>"+long_value+"</button><input aria-label='Spaced field' value=' SPACE_PRIVATE  VALUE '><button aria-label=' SPACE_PRIVATE  VALUE '>Spaced copy</button><img src=",1)
race_release=threading.Event();race_done=threading.Event()
requests=[]
class Proxy(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        requests.append({'method':'GET','url':self.path})
        parsed=urllib.parse.urlparse(self.path)
        if parsed.hostname!='fixture.invalid':self.send_error(403);return
        if parsed.path=='/_fixture/release':race_release.set();text='released'
        elif parsed.path=='/_fixture/status':text='done' if race_done.is_set() else 'waiting'
        elif parsed.path=='/privacy-race-done':race_done.set();text='done'
        elif parsed.path=='/privacy-latch':
            if not race_release.wait(15):self.send_error(408);return
            data=base64.b64decode('R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7')
            self.send_response(200);self.send_header('Content-Type','image/gif');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data);return
        elif parsed.path=='/privacy-overflow':text=privacy_overflow
        elif parsed.path=='/privacy-passwords':text=privacy_passwords
        elif parsed.path=='/privacy-empty':text=privacy_empty
        elif parsed.path=='/privacy-race':text=privacy_race
        elif parsed.path=='/privacy-paint':text=privacy_overflow.replace('width:20px;height:20px','width:20px;height:20px;text-shadow:120px 0 black')
        else:text=None
        if text is not None:
            self.send_response(200);self.send_header('Content-Type','text/html');self.send_header('Content-Length',str(len(text.encode())));self.end_headers();self.wfile.write(text.encode());return
        if parsed.path=='/download':
            self.send_response(200);self.send_header('Content-Type','application/octet-stream');self.send_header('Content-Disposition','attachment; filename=clovy-should-not-save.txt');self.end_headers();self.wfile.write(b'synthetic denied download');return
        text=(page + ('<p>Persisted login</p>' if 'fixture_login=synthetic' in self.headers.get('Cookie','') else '')) if parsed.path=='/' else f'<title>Result</title><h1>{parsed.path}</h1><p>{parsed.query}</p>'
        self.send_response(200);self.send_header('Content-Type','text/html');self.send_header('Set-Cookie','fixture_login=synthetic; Max-Age=3600; Path=/; SameSite=Lax');self.end_headers();self.wfile.write(text.encode())
    def do_CONNECT(self):requests.append({'method':'CONNECT','url':self.path});self.send_error(403)
    def log_message(self,*args):pass
proxy=http.server.ThreadingHTTPServer(('127.0.0.1',0),Proxy);threading.Thread(target=proxy.serve_forever,daemon=True).start()
profile=pathlib.Path(tempfile.mkdtemp(prefix='clovy-private-fixture-'))
grant, profile_lease = persistent_fixture_grant(profile/'browser')
proc=None;xvfb=None;receipts=[];stderr_file=None
try:
    env={'HOME':str(profile),'PATH':'/usr/bin:/bin'}
    runtime=profile/'runtime';runtime.mkdir(mode=0o700)
    env['XDG_RUNTIME_DIR']=str(runtime)
    env['DBUS_SESSION_BUS_ADDRESS']='unix:abstract=clovy-owned-host-unavailable-'+str(os.getpid())
    if shutil.which('Xvfb'):
        display=f':{200+os.getpid()%2000}'
        xvfb=subprocess.Popen(['Xvfb',display,'-screen','0','1280x900x24','-nolisten','tcp'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        env['DISPLAY']=display;time.sleep(.25)
        if xvfb.poll() is not None:raise RuntimeError('Owned Xvfb failed; refusing another display')
    relay=pathlib.Path(__file__).resolve().parent/'private-host/cdp_fixture.py'
    relay_config=profile/'relay.json';armed=profile/'race-armed'
    relay_config.write_text(json.dumps({'chrome':str(chrome.resolve()),'proxyPort':proxy.server_port,'evidence':str(evidence.resolve()),'armed':str(armed)}))
    relay_chrome=profile/'fixture-chrome';relay_chrome.write_text('#!/bin/sh\nexec /usr/bin/python3 '+__import__('shlex').quote(str(relay))+' '+__import__('shlex').quote(str(relay_config))+' "$@"\n');relay_chrome.chmod(0o700)
    stderr_file=(evidence/'host-stderr.txt').open('w+')
    proc=subprocess.Popen([str(binary.resolve())],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=stderr_file,text=True,bufsize=1,env=env,start_new_session=True)
    def wire(v):
        proc.stdin.write(json.dumps(v)+'\n');proc.stdin.flush()
        if not select.select([proc.stdout],[],[],25)[0]:raise TimeoutError('Private host response deadline')
        line=proc.stdout.readline()
        if not line:
            stderr_file.flush();stderr_file.seek(0);raise RuntimeError('Host exited: '+stderr_file.read())
        return json.loads(line)
    token=secrets.token_hex(32)
    assert wire({'protocol':1,'token':token,'grant':grant})=={'protocol':1,'token':token}
    def call(action,ok=True,**args):
        receipt=wire({'id':len(receipts)+1,'action':action,**args});receipts.append(receipt)
        (evidence/'receipts.json').write_text(json.dumps(receipts,indent=2))
        if ok and 'error' in receipt:
            if action=='navigate':
                time.sleep(2)
                diagnostic=wire({'id':999,'action':'snapshot','tab_id':args['tab_id']})
                (evidence/'post-failure-observation.json').write_text(json.dumps(diagnostic,indent=2))
            raise AssertionError((action,receipt))
        return receipt.get('data',{}) if ok else receipt
    def privacy_checks(active):
        checks={}
        call('navigate',tab_id=active,url='http://fixture.invalid/privacy-overflow')
        shot=call('screenshot',tab_id=active);png=base64.b64decode(shot['base64']);(evidence/'overflow.png').write_bytes(png);decoded=decoded_png(png)
        regions={name:region_counts(decoded,rect) for name,rect in {'inherited':(595,295,210,30),'pseudo':(595,355,210,30),'closed':(595,415,210,30),'ordinary':(95,175,210,35),'context':(595,495,230,45)}.items()}
        (evidence/'overflow-regions.json').write_text(json.dumps(regions,indent=2))
        for name in ['inherited','pseudo','closed','ordinary']:checks['masked_'+name]=regions[name]['masked']>regions[name]['pixels']*.8
        checks['public_context_preserved']=regions['context']['masked']<regions['context']['pixels']*.05 and regions['context']['black']>20
        call('navigate',tab_id=active,url='http://fixture.invalid/privacy-paint')
        refused=call('screenshot',ok=False,tab_id=active)
        checks['unsupported_paint_refused']=refused.get('delivery')=='not_attempted' and refused.get('error',{}).get('code')=='redaction_failed' and 'data' not in refused
        call('navigate',tab_id=active,url='http://fixture.invalid/privacy-passwords')
        def named_reference(snapshot,name):
            found=re.search(r'"'+re.escape(name)+r'"[^\n]*ref=([a-f0-9]{32})',snapshot);assert found,(name,snapshot)
            return found.group(1)
        for name in ['Shown one','Shown two','Neutral three','Neutral four']:
            for operation,params in [('click',{}),('fill',{'text':'forbidden synthetic'}),('press',{'key':'Enter'})]:
                text=call('snapshot',tab_id=active)['snapshot'];reference=named_reference(text,name);expected=call('inspect_reference',tab_id=active,ref=reference)
                checks[name+'_original_host_class']=expected.get('interactionClass')=='human_only'
                if name=='Shown one':checks['redacted_autocomplete_routes_human']=expected.get('interactionClass')=='human_only' and 'password' not in json.dumps(expected['element'])
                denied=call('guarded_action',ok=False,tab_id=active,ref=reference,operation=operation,expected=expected,**params)
                checks[name+'_'+operation]=denied.get('delivery')=='not_attempted' and denied.get('error',{}).get('code')=='sensitive_field'
        current=call('snapshot',tab_id=active)['snapshot'];checks['sensitive_events_zero']='Events:0' in current
        before_count=int(re.search(r'Events:(\d+)',current).group(1))
        reference=named_reference(current,'Ordinary action');expected=call('inspect_reference',tab_id=active,ref=reference)
        control=call('guarded_action',ok=False,tab_id=active,ref=reference,operation='click',expected=expected)
        checks['ordinary_class_requires_approval']=expected.get('interactionClass')=='approval_required'
        checks['ordinary_click_delivered']=control.get('delivery')=='delivered' and ('Events:'+str(before_count+1)) in call('snapshot',tab_id=active)['snapshot']
        call('navigate',tab_id=active,url='http://fixture.invalid/privacy-empty')
        empty=call('snapshot',tab_id=active);checks['native_empty_fields_snapshot']='Public empty context useful' in empty['snapshot']
        empty_png=call('screenshot',tab_id=active);checks['native_empty_fields_png']=base64.b64decode(empty_png['base64']).startswith(b'\x89PNG\r\n\x1a\n')
        (evidence/'empty-fields.png').write_bytes(base64.b64decode(empty_png['base64']))
        for name in ['Empty input','Empty textarea']:
            current=call('snapshot',tab_id=active)['snapshot'];reference=named_reference(current,name);expected=call('inspect_reference',tab_id=active,ref=reference)
            call('guarded_action',tab_id=active,ref=reference,operation='fill',text='Z',expected=expected)
            current=call('snapshot',tab_id=active)['snapshot'];reference=named_reference(current,name);expected=call('inspect_reference',tab_id=active,ref=reference)
            call('guarded_action',tab_id=active,ref=reference,operation='press',key='Backspace',expected=expected)
            checks[name+'_deleted_reobserved']='Public empty context useful' in call('snapshot',tab_id=active)['snapshot']
        current=call('snapshot',tab_id=active)['snapshot'];checks['empty_fill_delete_events']='Changed:4' in current
        reference=named_reference(current,'Submit empties');expected=call('inspect_reference',tab_id=active,ref=reference)
        call('guarded_action',tab_id=active,ref=reference,operation='click',expected=expected)
        submitted=call('snapshot',tab_id=active)['url'];query=urllib.parse.parse_qs(urllib.parse.urlsplit(submitted).query,keep_blank_values=True)
        checks['empty_form_reobserved']=query.get('input')==[''] and query.get('textarea')==['']
        call('navigate',tab_id=active,url='http://fixture.invalid/privacy-race');armed.touch()
        raced=call('snapshot',tab_id=active);(evidence/'race-snapshot.json').write_text(json.dumps(raced,indent=2))
        secrets_in_race=['RACE_SECRET_A','RACE_SECRET_B','HIDDEN_SECRET_A','HIDDEN_SECRET_B','NEW_SECRET_B','NEW_CLOSED_SECRET_B','LONG_PRIVATE_','SPACE_PRIVATE']
        checks['latched_snapshot_no_secret']=all(secret not in json.dumps(raced) for secret in secrets_in_race)
        checks['latched_mutation_completed']=race_done.wait(5)
        latch=evidence/'race-latch.json';checks['latched_before_ax']=latch.exists()
        checks['owned_debugger_pause_observed']=latch.exists() and json.loads(latch.read_text()).get('ownedDebuggerPauseAtAxDispatch') is True
        after=call('snapshot',tab_id=active);(evidence/'race-after.json').write_text(json.dumps(after,indent=2));checks['fresh_snapshot_no_secret']=all(secret not in json.dumps(after) for secret in secrets_in_race)
        inspections=[];target=None
        for reference in re.findall(r'ref=([a-f0-9]{32})',after['snapshot']):
            observed=call('inspect_reference',ok=False,tab_id=active,ref=reference)
            if 'data' in observed:
                inspected=observed['data'];inspections.append(inspected)
                if target is None and inspected['element']['tag']=='input' and inspected['element']['id']=='[redacted]':target=(reference,inspected)
        (evidence/'race-inspections.json').write_text(json.dumps(inspections,indent=2))
        checks['metadata_no_secret']=all(secret not in json.dumps(inspections) for secret in secrets_in_race)
        checks['ordinary_metadata_preserved']=any(x['element']['label']=='Query stable' and x['element']['id']=='ordinary' for x in inspections)
        ordinary=named_reference(after['snapshot'],'Query stable')
        first_ordinary=next(x for x in inspections if x['element']['label']=='Query stable' and x['element']['id']=='ordinary')
        second_ordinary=call('inspect_reference',tab_id=active,ref=ordinary)
        checks['unchanged_metadata_revision_stable']=bool(first_ordinary.get('metadataId')) and first_ordinary.get('metadataId')==second_ordinary.get('metadataId')
        ordinary_fill=call('guarded_action',ok=False,tab_id=active,ref=ordinary,operation='fill',text='allowed-control',expected=first_ordinary)
        checks['original_expected_after_reinspect_delivered']=ordinary_fill.get('delivery')=='delivered'
        mutate=named_reference(after['snapshot'],'Mutate labels');expected=call('inspect_reference',tab_id=active,ref=mutate)
        call('guarded_action',tab_id=active,ref=mutate,operation='click',expected=expected)
        if target:
            reference,expected=target
            overwritten=call('inspect_reference',tab_id=active,ref=reference)
            checks['changed_metadata_revision_changes']=bool(expected.get('metadataId')) and overwritten.get('metadataId')!=expected.get('metadataId')
            checks['redacted_metadata_collision_exercised']=overwritten['element']==expected['element']
            stale=call('guarded_action',ok=False,tab_id=active,ref=reference,operation='press',key='Enter',expected=expected)
            checks['redacted_metadata_change_stales']=stale.get('delivery')=='not_attempted' and stale.get('error',{}).get('code')=='stale_reference'
        else:
            checks['changed_metadata_revision_changes']=False;checks['redacted_metadata_collision_exercised']=False;checks['redacted_metadata_change_stales']=False
        checks['stale_input_no_event']='Edited:0' in call('snapshot',tab_id=active)['snapshot']
        (evidence/'privacy-results.json').write_text(json.dumps(checks,indent=2))
        return checks
    invalid=call('eval',ok=False,script='document.body.textContent')
    assert invalid['error']['code']=='invalid_request' and invalid['delivery']=='not_attempted'
    start=call('start',chromePath=str(relay_chrome),profilePath=str(profile/'browser'),proxyPort=proxy.server_port,persistent=True,headed=False)
    tab=start['tabId'];assert isinstance(tab,int)
    if regressions_only:
        checks=privacy_checks(tab);call('shutdown');proc.wait(timeout=5)
        print(json.dumps({'passed':all(checks.values()),'privacy':checks,'receipts':len(receipts)}))
        raise SystemExit(0 if all(checks.values()) else 1)
    initial_url='http://fixture.invalid/#native-url-fragment'
    assert call('navigate',tab_id=tab,url=initial_url)['url']==initial_url
    first_snapshot=call('snapshot',tab_id=tab);assert first_snapshot['url']==initial_url
    snap=first_snapshot['snapshot']
    assert all(secret not in snap for secret in ['synthetic-password-secret','synthetic-otp-secret','synthetic-card-secret','synthetic-editable-secret','synthetic-readonly-secret'])
    (evidence/'snapshot.txt').write_text(snap)
    def ref(name,snapshot=None):
        if snapshot is None:snapshot=snap
        match=re.search(r'"'+re.escape(name)+r'"[^\n]*ref=([a-f0-9]{32})',snapshot)
        assert match,(name,snapshot)
        return match.group(1)
    def act(reference,operation,**params):
        expected=call('inspect_reference',tab_id=tab,ref=reference)
        assert expected['backendNodeId'] and expected['documentId'] and expected['origin']=='http://fixture.invalid'
        return call('guarded_action',tab_id=tab,ref=reference,operation=operation,expected=expected,**params)
    alias=ref('Alias');expected_alias=call('inspect_reference',tab_id=tab,ref=alias)
    assert expected_alias['element']['name']=='otp' and expected_alias['element']['label']=='Alias'
    denied=call('guarded_action',ok=False,tab_id=tab,ref=alias,operation='fill',text='forbidden synthetic',expected=expected_alias)
    assert denied['delivery']=='not_attempted' and denied['error']['code']=='sensitive_field'
    buy=re.search(r'- button "Buy"[^\n]*ref=([a-f0-9]{32})',snap);assert buy,snap
    labelled=call('inspect_reference',tab_id=tab,ref=buy.group(1))
    assert labelled['element']['tag']=='button' and labelled['element']['label']=='Buy' and labelled['element']['name']=='',labelled
    external=call('inspect_reference',tab_id=tab,ref=ref('External form'))
    assert external['element']['inForm'] is True and external['element']['name']=='associated',external
    readonly=ref('Readonly');readonly_expected=call('inspect_reference',tab_id=tab,ref=readonly)
    denied_edit=call('guarded_action',ok=False,tab_id=tab,ref=readonly,operation='press',key='Backspace',expected=readonly_expected)
    assert denied_edit['delivery']=='not_attempted' and denied_edit['error']['code']=='unsupported_operation',denied_edit
    assert 'readonly mutated' not in call('snapshot',tab_id=tab)['snapshot']
    snap=call('snapshot',tab_id=tab)['snapshot']
    query=ref('Query');act(query,'fill',text='synthetic query')
    snap2=call('snapshot',tab_id=tab)['snapshot'];assert 'synthetic query' not in snap2
    shot=call('screenshot',tab_id=tab);assert shot['url']==initial_url and shot['tabId']==tab;png=base64.b64decode(shot['base64']);assert png.startswith(b'\x89PNG\r\n\x1a\n');assert masked_pixels(png)>1000;(evidence/'screenshot.png').write_bytes(png)
    save=ref('Save',snap2);expected=call('inspect_reference',tab_id=tab,ref=save)
    act(ref('Replace',snap2),'click')
    stale=call('guarded_action',ok=False,tab_id=tab,ref=save,operation='click',expected=expected)
    assert stale['delivery']=='not_attempted' and stale['error']['code']=='stale_reference',stale
    snap3=call('snapshot',tab_id=tab)['snapshot']
    swap=ref('KeySwap',snap3);expected_swap=call('inspect_reference',tab_id=tab,ref=swap)
    uncertain=call('guarded_action',ok=False,tab_id=tab,ref=swap,operation='press',key='Enter',expected=expected_swap)
    assert uncertain['delivery']=='unknown' and uncertain['error']['code']=='effect_unknown',uncertain
    assert call('inspect_reference',ok=False,tab_id=tab,ref=ref('Query',snap3))['error']['code']=='stale_reference'
    snap3=call('snapshot',tab_id=tab)['snapshot'];act(ref('Query',snap3),'press',key='Enter')
    submitted=call('snapshot',tab_id=tab)['url'];assert 'submitted' in submitted and urllib.parse.parse_qs(urllib.parse.urlsplit(submitted).query)['query']==['synthetic query']
    call('back',tab_id=tab);assert call('snapshot',tab_id=tab)['url']==initial_url
    tab2=call('tab_open',url='http://fixture.invalid/two')['tabId'];assert tab2!=tab
    assert len(call('tab_list')['tabs'])==2
    call('tab_switch',tab_id=tab);call('tab_close',tab_id=tab2)
    closed=call('snapshot',ok=False,tab_id=tab2);assert closed['error']['code']=='tab_gone' and closed['delivery']=='not_attempted'
    old=call('snapshot',tab_id=tab)['snapshot'];oldref=ref('Query',old)
    call('suspend');assert call('snapshot',ok=False,tab_id=tab)['error']['code']=='human_takeover'
    call('resume');assert call('inspect_reference',ok=False,tab_id=tab,ref=oldref)['error']['code']=='stale_reference'
    denied_download=call('navigate',ok=False,tab_id=tab,url='http://fixture.invalid/download')
    assert denied_download.get('error') and denied_download['delivery']=='unknown',denied_download
    call('snapshot',tab_id=tab)
    assert not list(profile.rglob('clovy-should-not-save*')) and not list(profile.rglob('*.crdownload'))
    if xvfb:
        takeover=call('takeover');assert takeover['mode']=='visible' and takeover['suspended'] and takeover['tabId']>tab2
        assert call('snapshot',ok=False,tab_id=takeover['tabId'])['error']['code']=='human_takeover'
        call('resume');call('navigate',tab_id=takeover['tabId'],url='http://fixture.invalid/')
        assert 'Persisted login' in call('snapshot',tab_id=takeover['tabId'])['snapshot']
        if shutil.which('xdotool'):
            # Simulate the human only on this run's private X server. No product
            # CDP/eval API exists for the model to navigate a local file.
            sentinel=profile/'human-file.html';sentinel.write_text('<title>Clovy file sentinel</title><h1>Synthetic file must remain private</h1>')
            def xdotool(*arguments):
                return subprocess.check_output(['xdotool',*arguments],env=env,text=True,timeout=5).strip()
            deadline=time.monotonic()+5
            while time.monotonic()<deadline:
                # This DISPLAY belongs to our Xvfb process. Match the actual
                # fixture page title rather than a browser-distribution class.
                result=subprocess.run(['xdotool','search','--onlyvisible','--name','^Private host fixture( - .*)?$'],env=env,text=True,capture_output=True,timeout=5)
                if result.returncode==0 and result.stdout.strip():
                    window=result.stdout.strip().splitlines()[0]
                    (evidence/'human-window.json').write_text(json.dumps({'display':env['DISPLAY'],'window':window,'title':xdotool('getwindowname',window)},indent=2))
                    break
                if result.returncode not in (0,1):result.check_returncode()
                time.sleep(.05)
            else:raise TimeoutError('Owned fixture window did not become visible')
            def human_url(url,title):
                xdotool('windowfocus','--sync',window)
                xdotool('key','--clearmodifiers','ctrl+l')
                xdotool('type','--clearmodifiers','--delay','0','--',url)
                xdotool('key','Return')
                deadline=time.monotonic()+5
                while time.monotonic()<deadline:
                    if title in xdotool('getwindowname',window):return
                    time.sleep(.05)
                raise TimeoutError('Human fixture navigation did not become visible')
            call('suspend');human_url(sentinel.as_uri(),'Clovy file sentinel');call('resume')
            file_capture=call('screenshot',ok=False,tab_id=takeover['tabId'])
            assert file_capture['delivery']=='not_attempted' and file_capture['error']['code']=='invalid_url' and 'data' not in file_capture,file_capture
            call('suspend');human_url('http://fixture.invalid/','Private host fixture');call('resume')
            # A title can become visible before the new document is ready.
            # Observe readiness through bounded reads, without replaying input.
            deadline=time.monotonic()+5
            while time.monotonic()<deadline:
                observation=call('snapshot',ok=False,tab_id=takeover['tabId'])
                observed=observation.get('data',{}).get('snapshot','')
                if 'heading "Fixture"' in observed and 'textbox "Query"' in observed:
                    (evidence/'human-return-readiness.json').write_text(json.dumps(observation,indent=2))
                    break
                if observation.get('delivery')=='unknown':raise AssertionError(observation)
                time.sleep(.05)
            else:raise TimeoutError('Human fixture document did not become ready')
            assert call('screenshot',tab_id=takeover['tabId'])['url']=='http://fixture.invalid/'
    privacy=privacy_checks(takeover['tabId'] if xvfb else tab)
    assert all(privacy.values()),privacy
    # Neither the host nor Chrome has a command socket / TCP CDP argument.
    parents={}
    for entry in pathlib.Path('/proc').iterdir():
        if not entry.name.isdigit():continue
        try:
            fields=(entry/'stat').read_text().rsplit(')',1)[1].split();parents[int(entry.name)]=int(fields[1])
        except (FileNotFoundError,ProcessLookupError,PermissionError):pass
    descendants={proc.pid}
    while True:
        found={pid for pid,ppid in parents.items() if ppid in descendants};before=len(descendants);descendants.update(found)
        if before==len(descendants):break
    for pid in descendants:
        try:
            assert os.getpgid(pid)==proc.pid,('escaped process group',pid)
            args=pathlib.Path('/proc',str(pid),'cmdline').read_bytes().replace(b'\0',b' ').decode(errors='replace')
            assert '--remote-debugging-port' not in args,args
            if '--user-data-dir' in args:assert '--password-store=basic' not in args and '--use-mock-keychain' not in args
        except (FileNotFoundError,ProcessLookupError):pass
    assert len(descendants)>1
    call('shutdown');proc.wait(timeout=5);assert proc.returncode==0
    (evidence/'receipts.json').write_text(json.dumps(receipts,indent=2))
    try:os.killpg(proc.pid,signal.SIGKILL)
    except ProcessLookupError:pass
    print(json.dumps({'passed':True,'receipts':len(receipts),'takeover':bool(xvfb),'owned_processes':len(descendants)}))
finally:
    if proc:
        try:os.killpg(proc.pid,signal.SIGKILL)
        except ProcessLookupError:pass
        proc.wait()
        stderr_file.flush();stderr_file.close()
    if xvfb:xvfb.terminate();xvfb.wait()
    proxy.shutdown();(evidence/'proxy-requests.json').write_text(json.dumps(requests,indent=2));profile_lease.close();shutil.rmtree(profile)
