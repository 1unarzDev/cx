#!/usr/bin/env python3
"""Read-only preview acceptance using exact disposable tmux sockets only."""
import copy
import json
import sys
sys.dont_write_bytecode = True
from session_adversarial import Fixture, require

def run(binary):
    f = Fixture(binary, live=True)
    try:
        f.start_managed()
        managed = f.create('preview', name='Synthetic preview')
        f.tmux('new-session', '-d', '-s', 'external-preview', '/bin/sh', default=True)
        rows = f.rows()
        require(len(rows) == 2, 'fixture inventory mismatch')
        for row in rows:
            default = row['external']
            target = row['terminal']['session_id']
            f.send(target, "printf '\\033[2J\\033[Hpreview-proof-123\\n'", default=default)
            f.until(lambda: b'preview-proof-123' in f.tmux('capture-pane', '-p', '-t', target, default=default).stdout, 'synthetic screen not ready')
            f.tmux('resize-window', '-t', target, '-x', '100', '-y', '100', default=default)
            f.send(target, "printf '\\033[100;1Hbottom-screen-proof\\n'", default=default)
            fmt = '#{window_width}:#{window_height}:#{session_attached}:#{pane_id}:#{pane_current_command}'
            before = f.tmux('display-message', '-p', '-t', target, fmt, default=default).stdout
            result = f.request('session_preview', dict(session=row))
            require(not result.get('error'), 'preview failed')
            lines = result['result']['lines']
            require(any('bottom-screen-proof' in line for line in lines), 'tall screen bottom missing')
            require(len(lines) <= 60 and sum(len(s.encode()) for s in lines) <= 24576, 'preview not bounded')
            after = f.tmux('display-message', '-p', '-t', target, fmt, default=default).stdout
            require(before == after, 'preview mutated geometry, attachment or process')
            for field in ('server_start', 'created', 'session_id'):
                stale = copy.deepcopy(row)
                stale['terminal'][field] = '$999999' if field == 'session_id' else '0'
                require(bool(f.request('session_preview', dict(session=stale)).get('error')), 'stale identity accepted')
            f.tmux('kill-session', '-t', target, default=default)
            require(bool(f.request('session_preview', dict(session=row)).get('error')), 'ended preview accepted')
        print(json.dumps(dict(status='PASS', checks=['managed and external screen snapshots', 'no resize/attach/process mutation', 'bounded screen', 'stale generation and ended runtime refusal'])))
    finally:
        f.close()

if __name__ == '__main__':
    run(sys.argv[1])
