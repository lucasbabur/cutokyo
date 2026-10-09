import base64,datetime,hashlib,importlib.util,json,os,re,shutil,socket,subprocess,tempfile,time,traceback,urllib.request
from pathlib import Path
REPO=Path('/home/lucas/Developer/personal/cutokyo-community')
EVIDENCE=REPO/'.claude/fleets/20260919-cutokyo-v01/evidence/product-audit-20261004/native-keyboard-trusted-probe'
EVIDENCE.mkdir(exist_ok=True)
spec=importlib.util.spec_from_file_location('native_runner',REPO/'ui/scripts/native-e2e.py'); helper=importlib.util.module_from_spec(spec);spec.loader.exec_module(helper)
source=REPO/'target/native-evidence/attempt-smrhxu6w'
original=json.loads((source/'package-launch.json').read_text())
root=Path(tempfile.mkdtemp(prefix='cutokyo-native-test-wdio-keyboard-'))
application=root/'package/usr/bin/cutokyo-desktop'
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
commands=[]; results=[]; process=None; sid=None; port=None; teardown={}; manifest={}; error=None
logfile=(EVIDENCE/'native-application.log').open('wb')
def command(args):
    entry={'command':args,'at':datetime.datetime.now(datetime.timezone.utc).isoformat()}; commands.append(entry)
    r=subprocess.run(args,check=True,capture_output=True,text=True);entry['stdout']=r.stdout;entry['stderr']=r.stderr;return r.stdout

def api(path,body=None,method=None):
    data=json.dumps(body).encode() if body is not None else None
    req=urllib.request.Request(f'http://127.0.0.1:{port}'+path,data=data,headers={'Content-Type':'application/json'},method=method or ('POST' if data is not None else 'GET'))
    with urllib.request.urlopen(req,timeout=15) as response: out=json.load(response)
    if isinstance(out.get('value'),dict) and out['value'].get('error'): raise RuntimeError(json.dumps(out))
    return out['value']
def execute(script): return api('/session/'+sid+'/execute/sync',{'script':script,'args':[]})
def active():
    obj=json.loads(subprocess.check_output(['hyprctl','activewindow','-j'],text=True))
    return {k:obj.get(k) for k in ['address','pid','xwayland','mapped']}
def verified_client():
    assert process and process.poll() is None,'Owned packaged app exited'
    assert Path(os.readlink(f'/proc/{process.pid}/exe'))==application,'PID executable identity mismatch'
    obj=json.loads(subprocess.check_output(['hyprctl','clients','-j'],text=True))
    matches=[c for c in obj if c.get('pid')==process.pid]
    assert len(matches)==1,'Expected exactly one owned app window'
    client=matches[0]
    assert re.fullmatch(r'0x[0-9a-fA-F]+',client['address'])
    assert client.get('mapped'), 'Target window not mapped'
    return client

def send(key):
    client=verified_client();before=active()
    assert not (before.get('xwayland') and before.get('pid')!=process.pid),'Refuse targeted keys while a foreign XWayland window is active'
    result=command(['hyprctl','dispatch','sendshortcut',f', {key}, address:{client["address"]}'])
    assert result.strip()=='ok','Hyprland dispatcher did not acknowledge'
    after=active()
    assert after['address']==before['address'],'Dispatch changed compositor active window; stop further keys'
    return {'key':key,'pid':process.pid,'address':client['address'],'active_before':before,'active_after':after,'dispatcher_result':result.strip()}

snapshot="return {id:document.activeElement?.id,tag:document.activeElement?.tagName,className:document.activeElement?.className,url:location.href,events:window.__cutokyoNativeKeyTrace,viewport:[innerWidth,innerHeight],hasFocus:document.hasFocus()};"
arm="""
if(!window.__cutokyoNativeKeyRecorder){
window.__cutokyoNativeKeyRecorder=e=>window.__cutokyoNativeKeyTrace.push({type:e.type,key:e.key??null,isTrusted:e.isTrusted,id:e.target.id,tag:e.target.tagName,className:e.target.className,time:performance.now()});
for(const name of ['keydown','keyup','click','focusin','focusout'])document.addEventListener(name,window.__cutokyoNativeKeyRecorder,true);}
window.__cutokyoNativeKeyTrace=[];
return true;
"""
try:
    package=source/'Cutokyo_0.1.0_amd64.deb'
    assert sha(package)==original['package_sha256'],'Retained package digest mismatch'
    command(['dpkg-deb','--extract',str(package),str(root/'package')])
    assert sha(application)==original['application_sha256'],'Extracted binary differs from retained proof'
    (root/'bin').mkdir(mode=0o700)
    old_root=Path(original['root'])
    for name in ['cutokyo','claude']:
        shutil.copy2(old_root/'bin'/name,root/'bin'/name)
    assert sha(root/'bin/cutokyo')==original['receiver_sha256']
    assert sha(root/'bin/claude')==original['synthetic_harness_sha256']
    reservation=helper.reserve_port();port=reservation.getsockname()[1]
    env=helper.runtime_environment(root,EVIDENCE,application,port)
    manifest={'revision':original['revision'],'package':str(package),'package_sha256':sha(package),'application':str(application),'application_sha256':sha(application),'receiver':str(root/'bin/cutokyo'),'receiver_sha256':sha(root/'bin/cutokyo'),'fake_harness':str(root/'bin/claude'),'fake_harness_sha256':sha(root/'bin/claude'),'root':str(root),'port':port,'backend':env.get('GDK_BACKEND'),'fixture':'search-resume','launch_origin':'fresh extraction of retained Debian package','one_app_only':True,'home':env['HOME'],'config_home':env['XDG_CONFIG_HOME'],'data_home':env['XDG_DATA_HOME'],'provider_environment_not_forwarded':True}
    reservation.close()
    process=subprocess.Popen([str(application)],cwd=root,env=env,stdout=logfile,stderr=subprocess.STDOUT,start_new_session=True)
    manifest.update({'pid':process.pid,'owned_group':process.pid})
    (EVIDENCE/'package-launch.json').write_text(json.dumps(manifest,indent=2)+'\n')
    deadline=time.monotonic()+60
    while time.monotonic()<deadline:
        assert process.poll() is None,'Private package process exited during startup'
        try:
            api('/status'); client=verified_client();break
        except (OSError,urllib.error.URLError,AssertionError):time.sleep(.1)
    else:raise RuntimeError('Private app/driver startup timed out')
    # Prove this allocated listener belongs to the owned application, not another server.
    inodes=set()
    for line in Path('/proc/net/tcp').read_text().splitlines()[1:]:
        fields=line.split()
        if int(fields[1].split(':')[1],16)==port and fields[3]=='0A':inodes.add(fields[9])
    sockets={os.readlink(f) for f in Path(f'/proc/{process.pid}/fd').iterdir() if f.exists()}
    assert any('socket:['+inode+']' in sockets for inode in inodes),'Driver listener not owned by private app PID'
    selector='address:'+client['address']
    if not client.get('floating'):command(['hyprctl','dispatch','togglefloating',selector])
    command(['hyprctl','dispatch','resizewindowpixel','exact 1280 800,'+selector])
    manifest['window']={k:client.get(k) for k in ['address','pid','xwayland','mapped']}
    created=api('/session',{'capabilities':{'alwaysMatch':{'browserName':'tauri','wdio:tauriServiceOptions':{'windowLabel':'main'}}}})
    sid=created['sessionId']; manifest['session']=sid;manifest['capabilities']=created['capabilities']
    deadline=time.monotonic()+30
    while time.monotonic()<deadline:
        if execute("return document.querySelector('a.skip-link')!==null && document.querySelector('main h1')!==null;"):break
        time.sleep(.1)
    else:raise RuntimeError('Native UI did not render')
    # Setup only. The acceptance activation below uses real scoped keys, never a click.
    execute("location.hash='#/sessions';return true;")
    deadline=time.monotonic()+15
    while time.monotonic()<deadline:
        if execute("return document.querySelector('main h1')?.textContent==='Sessions';"):break
        time.sleep(.1)
    else:raise RuntimeError('Native Sessions route not ready')
    execute("return new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>r(true))));")
    execute(arm)
    execute("document.querySelector('a.skip-link').focus();return true;")
    before=execute(snapshot);assert before['className']=='skip-link'
    api('/session/'+sid+'/actions',{'actions':[{'id':'diagnostic-key','type':'key','actions':[{'type':'keyDown','value':''},{'type':'pause','duration':10},{'type':'keyUp','value':''}]}]})
    after=execute(snapshot)
    results.append({'mode':'embedded-baseline','expected':'main-content','before':before,'after':after,'verdict':'PASS' if after['id']=='main-content' else 'FAIL'})
    assert after['className']=='skip-link' and after['id']=='','Expected reproduction of exact native embedded failure'
    assert not any(e['type']=='click' for e in after['events'])
    # Repeat trusted traversal without directly focusing either skip or main.
    for round in range(3):
        execute(arm)
        execute("const previous=document.body.getAttribute('tabindex');document.body.setAttribute('tabindex','-1');document.body.focus();if(previous===null)document.body.removeAttribute('tabindex');else document.body.setAttribute('tabindex',previous);return true;")
        before=execute(snapshot);assert before['tag']=='BODY', 'Initial native focus must be body'
        url=before['url']
        tab_dispatch=send('Tab')
        deadline=time.monotonic()+3
        while time.monotonic()<deadline:
            tab=execute(snapshot)
            if tab['className']=='skip-link':break
            time.sleep(.03)
        assert tab['className']=='skip-link','Trusted Tab did not traverse body to skip link'
        assert any(e['type']=='keydown' and e['key']=='Tab' and e['isTrusted'] for e in tab['events']),'No trusted native Tab event'
        if round==0:(EVIDENCE/'native-trusted-tab-1280x800.png').write_bytes(base64.b64decode(api('/session/'+sid+'/screenshot')))
        enter_dispatch=send('Return')
        deadline=time.monotonic()+3
        while time.monotonic()<deadline:
            after=execute(snapshot)
            if after['id']=='main-content':break
            time.sleep(.03)
        eventproof=all(any(e['type']==kind and (key is None or e['key']==key) and e['isTrusted'] for e in after['events']) for kind,key in [('keydown','Tab'),('keydown','Enter'),('click',None)])
        results.append({'mode':'trusted-scoped-hyprland','round':round,'before':before,'after_tab':tab,'after_enter':after,'tab_dispatch':tab_dispatch,'enter_dispatch':enter_dispatch,'trusted_event_proof':eventproof,'expected':'body -> skip-link via trusted Tab; Enter -> main-content; URL unchanged','verdict':'PASS' if after['id']=='main-content' and after['url']==url and eventproof else 'FAIL'})
        assert after['id']=='main-content','Trusted Enter failed to focus main-content'
        assert after['url']==url,'Skip keyboard activation changed route'
        assert eventproof,'Missing trusted native event proof'
        if round==0:(EVIDENCE/'native-trusted-enter-1280x800.png').write_bytes(base64.b64decode(api('/session/'+sid+'/screenshot')))
    execute("for(const type of ['keydown','keyup','click','focusin','focusout'])document.removeEventListener(type,window.__cutokyoNativeKeyRecorder,true);delete window.__cutokyoNativeKeyRecorder;delete window.__cutokyoNativeKeyTrace;return true;")
except BaseException as exc:
    error={'type':type(exc).__name__,'message':str(exc),'traceback':traceback.format_exc()}
finally:
    if sid:
        try:api('/session/'+sid,method='DELETE')
        except BaseException as exc:teardown['session_cleanup_error']=str(exc)
    if process:
        helper.stop_group(process.pid);process.wait(timeout=10)
        teardown.update({'owned_group':process.pid,'process_exit':process.returncode,'group_alive':helper.live_group(process.pid),'application_pid_remaining':helper.isolated_app_pid(application)})
    logfile.close()
    (EVIDENCE/'package-launch.json').write_text(json.dumps(manifest,indent=2)+'\n')
    record={'recorded_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'manifest':manifest,'commands':commands,'results':results,'error':error,'teardown':teardown,'retained_root':str(root),'no_product_edits':True,'no_global_key_dispatch':True,'no_explicit_focuswindow_dispatch':True}
    (EVIDENCE/'native-keyboard-results.json').write_text(json.dumps(record,indent=2)+'\n')
    print(json.dumps({'evidence':str(EVIDENCE),'root':str(root),'results':[{'mode':r['mode'],'round':r.get('round'),'verdict':r['verdict']} for r in results],'error':error,'teardown':teardown},indent=2),flush=True)
if error:raise SystemExit(1)
