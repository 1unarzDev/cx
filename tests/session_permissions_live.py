#!/usr/bin/env python3
"""Real private tmux launches/stops with synthetic agents; no inference or provider config."""
import json, os, pathlib, subprocess, sys, tempfile, time
binary = str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-permissions-') as tmp:
    root = pathlib.Path(tmp); bins = root/'bin'; bins.mkdir()
    source = root/'agent.c'
    source.write_text(r'''#include <stdio.h>
#include <string.h>
#include <unistd.h>
int main(int argc,char **argv) {
 if(argc>1 && !strcmp(argv[1],"--version")){puts("fixture 1.0");return 0;}
 FILE *f=fopen("args.jsonl","a");
 fprintf(f,"%s",argv[0]); for(int i=1;i<argc;i++)fprintf(f," %s",argv[i]);
 fprintf(f,"\n");fclose(f); for(;;)pause();
}''')
    for provider in ['codex','claude']:
        subprocess.check_call(['cc',str(source),'-o',str(bins/provider)])
    launch=root/'bash'; launch.write_text('#!/bin/sh\nexec /bin/bash --noprofile --norc "$@"\n');launch.chmod(0o700)
    env=dict(os.environ,HOME=tmp,XDG_STATE_HOME=str(root/'state'),SHELL=str(launch),TERM='xterm-256color',PATH=str(bins)+':'+os.environ['PATH'])
    env.pop('TMUX',None);env.pop('TMUX_PANE',None)
    socket=root/'state/cx/managed.sock'
    def cli(*args): return json.loads(subprocess.check_output([binary,*args],env=env,cwd=tmp))
    def request(op,args):
        payload=json.dumps(dict(version=1,id='permissions-proof',op=dict(op=op,args=args))).encode()
        wire=b'CX1 '+str(len(payload)).encode()+b'\n'+payload
        reply=subprocess.check_output([binary,'helper'],input=wire,env=env,cwd=tmp)
        return json.loads(reply.split(b'\n',1)[1])
    def wait_agent(created):
        deadline=time.monotonic()+5
        while time.monotonic()<deadline:
            live=next(s for s in cli('sessions') if s['id']==created['id'])
            if live['process']: return live
            time.sleep(.05)
        raise AssertionError('synthetic agent did not start')
    try:
        guard=cli('new','--directory',tmp,'--key','guard')
        for provider,flag in [('codex','--dangerously-bypass-approvals-and-sandbox'),('claude','--dangerously-skip-permissions')]:
            for yolo in [False, True]:
                key=f'{provider}-{yolo}'
                args=['new','--directory',tmp,'--provider',provider,'--key',key]
                created=cli(*args,*(['--yolo'] if yolo else [])); live=wait_agent(created)
                lines=(root/'args.jsonl').read_text().splitlines()
                assert lines[-1] == provider+(' '+flag if yolo else ''),lines
                fields={k:live[k] for k in ['id','pid','started','boot_id']}
                # Different mode must never reuse an already-running creation key.
                mismatch=request('create' if yolo else 'create_yolo',dict(key=key,directory=tmp,provider=provider,name=''))
                assert mismatch['error'],mismatch
                bad=request('stop_agent_session',dict(fields,provider='claude' if provider=='codex' else 'codex'))
                assert bad['error'],bad
                assert request('stop_session',fields)['error']
                result=request('stop_agent_session',dict(fields,provider=provider))
                assert result['result']=={'status':'stopped'},result
                assert any(s['id']==guard['id'] for s in cli('sessions'))
        assert request('create_yolo',dict(key='invalid',directory=tmp,provider='shell',name=''))['error']
        print(json.dumps(dict(result='PASS',checks='Default/YOLO argv for Codex and Claude; wrong-mode retry, wrong-provider and shell-stop refusal; agent stop and peer preservation',backend='private tmux + synthetic ELF agents; no inference')))
    finally:
        subprocess.run(['tmux','-S',str(socket),'kill-server'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
