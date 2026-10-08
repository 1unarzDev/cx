#!/usr/bin/env python3
"""Real routed packets, source NAT and directional TCP in disposable namespaces."""
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time

binary = str(pathlib.Path(sys.argv[1]).resolve())
if os.environ.get('CX_LAN_PACKETS') != '1':
    env = dict(os.environ, CX_LAN_PACKETS='1')
    os.execvpe('unshare', ['unshare', '-Urn', sys.executable, __file__, binary], env)

def run(*args, text=None):
    return subprocess.run(args, input=text, text=True, capture_output=True, timeout=10, check=True).stdout

def in_ns(pid, *args):
    return run('nsenter', '-t', str(pid), '-n', *args)

server_code = '''import socket,sys
s=socket.socket();s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);s.bind(('0.0.0.0',int(sys.argv[1])));s.listen()
while True:
 c,a=s.accept()
 with c:c.sendall(a[0].encode())
'''
probe_code = '''import socket,sys
try:
 with socket.create_connection((sys.argv[1],int(sys.argv[2])),timeout=1) as c:print(c.recv(100).decode())
except OSError:print('DENIED')
'''
children = []
try:
    for _ in range(2):
        children.append(subprocess.Popen(['unshare', '-n', 'sleep', '60']))
    time.sleep(.1)
    robot, external = [p.pid for p in children]
    for interface, pid, subnet, guest in [('robotlan', robot, '192.168.0', '147'), ('uplink', external, '198.18.0', '1')]:
        peer = interface + 'peer'
        run('ip', 'link', 'add', interface, 'type', 'veth', 'peer', 'name', peer)
        run('ip', 'link', 'set', peer, 'netns', str(pid))
        run('ip', 'addr', 'add', subnet + '.2/24', 'dev', interface)
        run('ip', 'link', 'set', interface, 'up')
        in_ns(pid, 'ip', 'link', 'set', 'lo', 'up')
        in_ns(pid, 'ip', 'addr', 'add', subnet + '.' + guest + '/24', 'dev', peer)
        in_ns(pid, 'ip', 'link', 'set', peer, 'up')
    in_ns(robot, 'ip', 'route', 'add', 'default', 'via', '192.168.0.2')
    in_ns(external, 'ip', 'addr', 'add', '100.96.0.12/32', 'dev', 'lo')
    in_ns(external, 'ip', 'route', 'add', '192.168.0.0/24', 'via', '198.18.0.2')
    run('ip', 'route', 'add', 'default', 'via', '198.18.0.1')
    run('ip', 'route', 'add', '100.96.0.12/32', 'via', '198.18.0.1')
    run('sysctl', '-w', 'net.ipv4.ip_forward=1')
    for pid, port in [(robot, 22), (external, 22), (external, 8080), (os.getpid(), 22)]:
        children.append(subprocess.Popen(['nsenter', '-t', str(pid), '-n', sys.executable, '-c', server_code, str(port)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
    def probe(pid, address, port):
        return in_ns(pid, sys.executable, '-c', probe_code, address, str(port)).strip()
    time.sleep(.1)
    assert probe(robot, '100.96.0.12', 22) == '192.168.0.147'
    assert probe(robot, '192.168.0.2', 22) == '192.168.0.147'
    before = run('ip', '-j', 'address'), run('ip', '-j', 'route')
    with tempfile.TemporaryDirectory() as tmp:
        path = pathlib.Path(tmp) / 'observation.json'
        path.write_text(json.dumps(dict(observed_at=int(time.time()), downstream='robotlan', upstream='uplink', gateway='192.168.0.2', forwarding_enabled=True, mesh_addresses=['100.96.0.12'], recipients=[dict(id='robot', address='192.168.0.147', gateway='192.168.0.2', dns=['1.1.1.1'])])))
        plan = json.loads(run(binary, 'share-plan', 'packet_test', '--observation', str(path)))
    run('nft', '-f', '-', text=plan['nft_rules'])
    assert probe(robot, '100.96.0.12', 22) == 'DENIED'
    assert probe(robot, '192.168.0.2', 22) == 'DENIED'
    assert probe(robot, '198.18.0.1', 8080) == '198.18.0.2', 'source NAT missing'
    assert probe(external, '192.168.0.147', 22) == '198.18.0.1', 'mesh initiation or reply blocked'
    assert before == (run('ip', '-j', 'address'), run('ip', '-j', 'route'))
    run('nft', 'delete', 'table', 'ip', 'cx_packet_test_nat')
    run('nft', 'delete', 'table', 'inet', 'cx_packet_test')
    assert probe(robot, '100.96.0.12', 22) == '192.168.0.147', 'rollback did not restore baseline'
    assert probe(robot, '198.18.0.1', 8080) == '192.168.0.147'
    print(json.dumps(dict(result='PASS', real_packets=True, source_nat=True, ipv4_reverse_tcp22_denied=True, mesh_initiation_and_return=True, addresses_routes_preserved=True, rollback=True, live_robot_dns_https_ipv6='NOT TESTED')))
finally:
    for p in reversed(children):
        p.terminate()
    for p in children:
        p.wait(timeout=3)
