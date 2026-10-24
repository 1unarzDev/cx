#!/usr/bin/env python3
"""Container provider/history parity; fake scope checks and root-owned argv proof.

--self-test never spawns a process. Root runs --fixtures or --run-owned-container.
Live mode requires a preinstalled image and uses --pull=never --network=none.
"""
import argparse
import copy
import io
import json
import os
from pathlib import Path
import shlex
import shutil
import sys
import time
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
from session_adversarial import Fixture, Failure, decode_frame, require


class ParityFixture(Fixture):
    def request(self, op, args):
        request = json.dumps(dict(version=1, id='adversarial', op=dict(op=op, args=args))).encode()
        wire = b'CX1 ' + str(len(request)).encode() + b'\n' + request
        return decode_frame(self.run([self.binary, 'helper'], data=wire,
                                     timeout=120 if self.live else 15,
                                     label='container-helper:' + op).stdout)


def scope_of(container):
    return {k: container[k] for k in ('engine', 'id', 'name', 'user', 'folder', 'started_at')}


def launch(scope, provider='codex', resume=False, yolo=False, key='parity'):
    return dict(scope=scope, request=dict(key=key, directory=scope['folder'], provider=provider,
                                        name='Synthetic parity fixture', resume=resume), yolo=yolo)


def expected_args(provider, resume, yolo):
    args = ['resume'] if provider == 'codex' and resume else ['--resume'] if resume else []
    if yolo:
        args.append('--dangerously-bypass-approvals-and-sandbox' if provider == 'codex'
                    else '--dangerously-skip-permissions')
    return args


def checked_reply(f, operation, args):
    reply = f.request(operation, args)
    require(not reply.get('error'), operation + ' returned error')
    return reply['result']


def providers_match(value, scope, expected):
    require(value.get('scope') == scope, 'provider evidence does not preserve exact requested scope')
    require(isinstance(value.get('observed_at'), int) and value['observed_at'] > 0,
            'provider evidence has no observation time')
    require(set(value.get('providers', [])) == set(expected), 'wrong container-specific provider set')


FAKE_DOCKER = r'''
import json,sys
from pathlib import Path
root=Path(__file__).resolve().parent.parent
state=root/'docker-fixture.json'
data=json.loads(state.read_text())
args=sys.argv[1:]
with (root/'docker-operations').open('a') as log: log.write(args[0]+'\n')
if args[0]=='context': print('unix:///fixture/docker.sock')
elif args[0]=='info': print(data['engine'])
elif args[0]=='inspect': print(json.dumps(data['container']))
elif args[0]=='exec' and any('CXPROVIDERS1 ' in arg for arg in args):
    requested_id=next((arg for arg in args if len(arg)==64 and all(c in '0123456789abcdef' for c in arg)),None)
    evidence={'id':requested_id,'user':args[args.index('--user')+1],
              'folder':args[args.index('--workdir')+1]}
    with (root/'probe-scopes.jsonl').open('a') as log: log.write(json.dumps(evidence)+'\n')
    print('synthetic startup noise')
    print('CXPROVIDERS1 '+' '.join(data.get('providers',['shell'])))
    if data.get('after_probe'):
        state.write_text(json.dumps(data['after_probe']))
else:
    # Scope rejection must happen before reaching an execution/install path.
    sys.exit(97)
'''


def fixture_checks(f):
    state = f.root / 'docker-fixture.json'
    log = f.root / 'docker-operations'
    original = dict(id='a' * 64, name='/parity', image='fixture', state='running',
                    started_at='start-one', user='root', folder='/same-workspace',
                    network='none', networks={}, ports={}, mounts=[], metadata='[]')
    baseline = dict(engine='fixture-engine', container=original, providers=['shell', 'codex'])
    state.write_text(json.dumps(baseline))
    f.write_tool('docker', '#!' + sys.executable + '\n' + FAKE_DOCKER)
    container = checked_reply(f, 'container_inspect', dict(id=original['id']))
    scope = scope_of(container)
    available = checked_reply(f, 'container_providers', dict(scope=scope))
    providers_match(available, scope, ['shell', 'codex'])
    probes = f.root / 'probe-scopes.jsonl'
    require(json.loads(probes.read_text().splitlines()[-1]) ==
            {k: scope[k] for k in ('id', 'user', 'folder')},
            'provider probe ran outside requested container/user/folder')
    sibling = dict(baseline, container=dict(original, id='b' * 64), providers=['shell', 'claude'])
    state.write_text(json.dumps(sibling))
    sibling_scope = dict(scope, id='b' * 64)
    providers_match(checked_reply(f, 'container_providers', dict(scope=sibling_scope)),
                    sibling_scope, ['shell', 'claude'])
    cases = [('engine', dict(scope, engine='wrong-engine'), baseline),
             ('start', scope, dict(baseline, container=dict(original, started_at='start-two'))),
             ('replacement', scope, dict(baseline, container=dict(original, id='b' * 64))),
             ('user', scope, dict(baseline, container=dict(original, user='other'))),
             ('stopped', scope, dict(baseline, container=dict(original, state='exited')))]
    for label, stale, observed in cases:
        for operation, args in [('container_providers', dict(scope=stale)),
                                ('container_agent_create', launch(stale, resume=True))]:
            state.write_text(json.dumps(observed))
            log.write_text('')
            response = f.request(operation, args)
            require(bool(response.get('error')), operation + ' accepted stale ' + label)
            # Unknown operations also reject, but are not evidence for a scope check.
            error = response['error'].lower()
            require('unknown variant' not in error and 'unknown operation' not in error,
                    'binary lacks the new container-agent protocol')
            require('inspect' in log.read_text().splitlines(), 'scope guard never checked actual container')
            require('exec' not in log.read_text().splitlines(), operation + ' executed stale ' + label)
    for label, changed in [('restart', dict(baseline, container=dict(original, started_at='start-two'))),
                           ('user', dict(baseline, container=dict(original, user='other'))),
                           ('engine', dict(baseline, engine='replacement-engine'))]:
        state.write_text(json.dumps(dict(baseline, after_probe=changed)))
        log.write_text('')
        reply = f.request('container_providers', dict(scope=scope))
        require(bool(reply.get('error')), 'post-probe ' + label + ' change accepted as capability evidence')
        require(log.read_text().splitlines().count('exec') == 1, 'post-probe fixture did not execute exactly one probe')
    state.write_text(json.dumps(baseline))
    unavailable = f.request('container_agent_create', launch(scope, provider='claude', resume=True))
    require(bool(unavailable.get('error')) and 'unavailable' in unavailable['error'].lower(),
            'missing container provider did not fail with availability evidence')
    # Same path alone never authorizes a different lifecycle/user/engine.
    state.write_text(json.dumps(baseline))
    invalid = launch(scope, resume=True)
    invalid['request']['directory'] = '/different-project'
    response = f.request('container_agent_create', invalid)
    require(bool(response.get('error')), 'scope/request directory mismatch accepted')
    return ['same-folder sibling containers return distinct exact provider evidence',
            'new provider/create operations reject old engine/start/user/replaced/stopped scope before exec',
            'restart/user/engine change after probe invalidates capability evidence',
            'missing container provider refuses history launch',
            'request directory cannot override container scope']


def legacy_refusal(f, binary):
    scope = dict(engine='fixture', id='a' * 64, name='fixture', user='root',
                 folder='/fixture', started_at='one')
    request = json.dumps(dict(version=1, id='adversarial',
        op=dict(op='container_agent_create', args=launch(scope, resume=True)))).encode()
    response = f.run([str(Path(binary).resolve()), 'helper'],
        data=b'CX1 ' + str(len(request)).encode() + b'\n' + request,
        check=False, label='legacy-container-helper')
    if response.returncode == 0:
        require(bool(decode_frame(response.stdout).get('error')),
                'old helper accepted versioned history launch')
    return 'specified old helper refuses new agent-create operation without fresh fallback'


AGENT_C = r'''
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <stdlib.h>
int main(int argc,char **argv) {
 if(argc>1 && !strcmp(argv[1],"--version")) { puts("synthetic parity provider"); return 0; }
 FILE *f=fopen("/proof/argv","wb"); if(!f) return 80;
 for(int i=0;i<argc;i++) fwrite(argv[i],1,strlen(argv[i])+1,f);
 if(fclose(f)) return 81;
 char cwd[4096]; if(!getcwd(cwd,sizeof(cwd))) return 82;
 f=fopen("/proof/cwd","wb"); if(!f) return 83;
 fwrite(cwd,1,strlen(cwd),f); fclose(f);
 for(;;) pause();
}
'''


def live_checks(f, image):
    docker = shutil.which('docker')
    cc = shutil.which('cc')
    require(bool(docker) and bool(cc), 'owned-container fixture requires docker and cc')
    f.write_tool('docker', '#!/bin/sh\nexec ' + shlex.quote(docker) + ' "$@"\n')
    proof = f.root / 'proof'
    proof.mkdir()
    source = f.root / 'synthetic-agent.c'
    source.write_text(AGENT_C)
    executable = f.root / 'synthetic-agent'
    f.run([cc, '-static', str(source), '-o', str(executable)], label='compile-synthetic-agent', timeout=30)

    def engine(*args, check=True, timeout=30):
        return f.run([docker, *args], check=check, timeout=timeout, label='owned-docker:' + args[0])

    engine('image', 'inspect', image)
    f.start_managed()  # No systemd/service path; exact owned server already exists.
    owned = None
    checks = []
    try:
        owned = engine('run', '--pull=never', '-d', '--network=none',
            '--name', 'cx-parity-' + f.root.name,
            '--label', 'devcontainer.metadata=[{"remoteUser":"root"}]',
            '--mount', 'type=bind,src=' + str(proof) + ',dst=/proof',
            image, 'sleep', 'infinity').stdout.decode().strip()
        require(len(owned) == 64 and all(c in '0123456789abcdef' for c in owned), 'invalid owned container ID')
        for provider in ('codex', 'claude'):
            engine('cp', str(executable), owned + ':/usr/local/bin/' + provider)
        folder = "/parity project ' \u2603"
        engine('exec', owned, 'mkdir', '-p', folder)
        container = checked_reply(f, 'container_inspect', dict(id=owned))
        scope = dict(scope_of(container), folder=folder)
        observed = checked_reply(f, 'container_providers', dict(scope=scope))
        providers_match(observed, scope, ['shell', 'codex', 'claude'])
        # Discovery must not install private tooling; none exists before the first launch.
        require(engine('exec', owned, 'test', '!', '-d', '/tmp/cx-tools-0', check=False).returncode == 0,
                'provider discovery installed private helper tooling')
        checks.append('container-specific executable discovery returns exact scope without helper installation')
        engine('exec', owned, 'chmod', '600', '/usr/local/bin/claude')
        providers_match(checked_reply(f, 'container_providers', dict(scope=scope)), scope, ['shell', 'codex'])
        engine('exec', owned, 'chmod', '700', '/usr/local/bin/claude')
        checks.append('nonexecutable provider is not advertised')

        for provider in ('codex', 'claude'):
            for resume in (False, True):
                for yolo in (False, True):
                    for path in (proof / 'argv', proof / 'cwd'):
                        path.unlink(missing_ok=True)
                    key = f'parity-{provider}-{int(resume)}-{int(yolo)}'
                    request = launch(scope, provider, resume, yolo, key)
                    session = checked_reply(f, 'container_agent_create', request)
                    f.until(lambda: (proof / 'cwd').exists(), 'synthetic agent did not record launch', seconds=12)
                    argv = (proof / 'argv').read_bytes().split(b'\0')[:-1]
                    require(bool(argv) and Path(argv[0].decode()).name == provider, 'wrong container provider launched')
                    require([v.decode() for v in argv[1:]] == expected_args(provider, resume, yolo),
                            'provider fresh/resume/permission argv changed through payload chain')
                    require((proof / 'cwd').read_text() == folder, 'launch lost quoted container working directory')
                    require(session['container'] == scope and session['directory'] == folder,
                            'created session metadata escaped selected container scope')
                    again = checked_reply(f, 'container_agent_create', request)
                    require(again['id'] == session['id'] and len(f.rows()) == 1, 'idempotent request created duplicate work')
                    if provider == 'codex' and not resume and not yolo:
                        for field, value in [('provider', 'claude'), ('resume', True), ('yolo', True)]:
                            changed = copy.deepcopy(request)
                            if field == 'yolo':
                                changed[field] = value
                            else:
                                changed['request'][field] = value
                            refused = f.request('container_agent_create', changed)
                            require(bool(refused.get('error')) and len(f.rows()) == 1,
                                    'creation key accepted changed ' + field + ' semantics')
                    if provider == 'codex' and resume and not yolo:
                        from session_adversarial_attach import Client
                        client = Client(f, session, observe=True)
                        before = (proof / 'argv').read_bytes()
                        try:
                            client.wait_attached(False, session['terminal']['session_id'], True)
                            client.detach(False)
                        finally:
                            client.close()
                        require(len(f.rows()) == 1 and (proof / 'argv').read_bytes() == before,
                                'joining existing container session launched another agent')
                    fields = {k: session[k] for k in ('id', 'pid', 'started', 'boot_id')}
                    checked_reply(f, 'stop_agent_session', dict(fields, provider=provider))
                    f.until(lambda: not f.rows(), 'owned agent session did not stop')
        checks.append('all eight provider/fresh-or-native-picker/default-or-YOLO argv combinations preserve cwd')
        checks.append('idempotent launch/watch avoid duplicates; changed provider/resume/permission key bindings refuse')

        # Dedicated resume op prevents an old handler silently accepting a fresh launch.
        legacy = launch(scope, 'codex', True, False, 'legacy-resume-refusal')
        result = f.request('container_create', legacy)
        require(bool(result.get('error')) and not f.rows(), 'legacy container-create silently accepted resume')
        invalid = launch(scope, 'shell', True, False, 'shell-resume-refusal')
        require(bool(f.request('container_agent_create', invalid).get('error')) and not f.rows(),
                'agent create accepted shell history resume')
        checks.append('legacy operation and shell profile refuse agent history resume')

        # Metadata guard at both boundaries: running container restart invalidates old scope.
        engine('restart', owned)
        require(bool(f.request('container_providers', dict(scope=scope)).get('error')),
                'restarted container advertised providers under old lifecycle')
        require(bool(f.request('container_agent_create', launch(scope, resume=True, key='stale-live')).get('error')),
                'restarted container accepted old-scope launch')
        require(not f.rows(), 'stale scope created a terminal')
        checks.append('real owned restart invalidates old capability and launch scope')
        return checks
    finally:
        if owned:
            engine('rm', '-f', owned, check=False)


class PureTests(unittest.TestCase):
    def test_expected_argv_matrix(self):
        self.assertEqual(expected_args('codex', True, False), ['resume'])
        self.assertEqual(expected_args('claude', True, False), ['--resume'])
        for provider in ('codex', 'claude'):
            for resume in (False, True):
                for yolo in (False, True):
                    self.assertNotIn('--last', expected_args(provider, resume, yolo))
                    self.assertNotIn('--continue', expected_args(provider, resume, yolo))

    def test_request_preserves_quoted_folder(self):
        scope = dict(folder="/work ' \u2603", id='a' * 64)
        self.assertEqual(launch(scope, resume=True)['request']['directory'], scope['folder'])
        self.assertTrue(launch(scope, resume=True)['request']['resume'])

    def test_fake_docker_compiles(self):
        compile(FAKE_DOCKER, '<fake-docker>', 'exec')

    def test_scope_evidence_is_exact(self):
        scope = dict(id='a' * 64, folder='/same', user='root', engine='one', started_at='now', name='test')
        providers_match(dict(scope=scope, providers=['codex'], observed_at=1), scope, ['codex'])
        with self.assertRaises(Failure):
            providers_match(dict(scope=dict(scope, user='other'), providers=['codex'], observed_at=1), scope, ['codex'])

    def test_fake_probe_changes_lifecycle_without_subprocess(self):
        f = Fixture('/does-not-exist')
        try:
            after = dict(engine='new', container={}, providers=['shell'])
            (f.root / 'docker-fixture.json').write_text(json.dumps(
                dict(engine='old', container={}, providers=['shell', 'codex'], after_probe=after)))
            output = io.StringIO()
            args = ['docker', 'exec', '-i', '--user', 'root', '--workdir', "/work ' x", 'a' * 64,
                    '/bin/sh', '-c', 'CXPROVIDERS1 ']
            with patch.object(sys, 'argv', args), patch.object(sys, 'stdout', output):
                exec(compile(FAKE_DOCKER, '<fake-docker>', 'exec'), {'__file__': str(f.bin / 'docker')})
            self.assertEqual(output.getvalue().splitlines()[-1], 'CXPROVIDERS1 shell codex')
            self.assertEqual(json.loads((f.root / 'docker-fixture.json').read_text()), after)
            self.assertEqual(json.loads((f.root / 'probe-scopes.jsonl').read_text())['folder'], "/work ' x")
        finally:
            f.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', nargs='?')
    modes = parser.add_mutually_exclusive_group(required=True)
    modes.add_argument('--self-test', action='store_true')
    modes.add_argument('--fixtures', action='store_true')
    modes.add_argument('--run-owned-container', action='store_true')
    parser.add_argument('--image', help='required preinstalled image; never pulled or built')
    parser.add_argument('--legacy-binary', help='optional old helper binary; fake mode only')
    args = parser.parse_args()
    if args.self_test:
        result = unittest.TextTestRunner().run(unittest.defaultTestLoader.loadTestsFromTestCase(PureTests))
        return int(not result.wasSuccessful())
    if not args.binary or (args.run_owned_container and not args.image):
        parser.error('binary required; live container also requires explicit --image')
    if args.legacy_binary and not args.fixtures:
        parser.error('--legacy-binary requires --fixtures')
    f = None
    report = dict(case='container-agent-parity', mode='fake' if args.fixtures else 'owned-container', status='PASS')
    try:
        f = ParityFixture(args.binary, live=args.run_owned_container)
        report['checks'] = fixture_checks(f) if args.fixtures else live_checks(f, args.image)
        if args.legacy_binary:
            report['checks'].append(legacy_refusal(f, args.legacy_binary))
    except Exception as error:
        report.update(status='FAIL', reason=str(error) if isinstance(error, Failure) else type(error).__name__)
    finally:
        if f:
            try:
                f.close()
            except Exception:
                report.update(status='FAIL', cleanup='owned fixture cleanup failed')
    print(json.dumps(report))
    return int(report['status'] != 'PASS')


if __name__ == '__main__':
    sys.exit(main())
