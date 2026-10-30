#!/usr/bin/env python3
"""Read-only preview acceptance using exact disposable tmux sockets only."""
import copy
import json
import shlex
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
            lines = result['result']['styled_lines']
            require(any('bottom-screen-proof' in line for line in lines), 'tall screen bottom missing')
            require(len(lines) == 100 and sum(len(s.encode()) for s in lines) <= 262144, 'preview not bounded')
            after = f.tmux('display-message', '-p', '-t', target, fmt, default=default).stdout
            require(before == after, 'preview mutated geometry, attachment or process')
            history = f.request('session_preview', dict(session=row, scrollback=True))['result']
            require(len(history['styled_lines']) == history['rows'] + history['history_rows'], 'history coordinates do not match capture')
            require(history['history_rows'] <= 500, 'history exceeds bound')
            pane = result['result']['pane_id']
            require(result['result']['live'] and result['result']['rows'] == 100, 'live dimensions missing')
            # Send only synthetic fixture input; never print terminal content.
            proof = f.root / ('input-default' if default else 'input-managed')
            command = "printf proof > " + str(proof) + "\n"
            sent = f.request('session_pane_input', dict(session=row, pane_id=pane, hex=command.encode().hex()))
            require(not sent.get('error') and sent['result']['delivered'], 'focused input failed')
            f.until(lambda: proof.exists(), 'input not delivered to actual shell')
            require(proof.read_text() == 'proof', 'input changed')
            # A raw alternate-screen application proves styled content and control keys.
            tui = f.root / 'synthetic-tui.py'
            ready = f.root / 'tui-ready'
            keys = f.root / 'tui-keys'
            ready.unlink(missing_ok=True)
            keys.unlink(missing_ok=True)
            tui.write_text("import os,sys,termios,tty\nfrom pathlib import Path\nfd=sys.stdin.fileno(); old=termios.tcgetattr(fd)\ntry:\n tty.setraw(fd)\n sys.stdout.write('\\x1b[?1049h\\x1b[2J\\x1b[3;4H\\x1b[31mTUI-PROOF\\x1b[0m'); sys.stdout.flush()\n Path(sys.argv[1]).touch()\n data=b''\n while len(data)<4: data+=os.read(fd,4-len(data))\n Path(sys.argv[2]).write_bytes(data)\nfinally:\n termios.tcsetattr(fd,termios.TCSANOW,old)\n sys.stdout.write('\\x1b[?1049l'); sys.stdout.flush()\n")
            f.send(target, '/usr/bin/python3 ' + ' '.join(map(shlex.quote,map(str,(tui,ready,keys)))), default=default)
            f.until(ready.exists, 'raw TUI did not start')
            rendered = f.request('session_preview', dict(session=row))['result']
            require(any('TUI-PROOF' in line and '\x1b[' in line for line in rendered['styled_lines']), 'styled alternate screen missing')
            require(rendered['cursor_y'] == 2, 'actual cursor position missing')
            alternate = f.request('session_preview', dict(session=row, scrollback=True))['result']
            require(len(alternate['styled_lines']) == alternate['rows'] + alternate['history_rows'], 'alternate-screen history coordinates incorrect')
            typed = b'\x1b[A\x03'
            require(not f.request('session_pane_input', dict(session=row,pane_id=pane,hex=typed.hex())).get('error'), 'raw keys refused')
            f.until(keys.exists, 'raw application did not receive keys')
            require(keys.read_bytes() == typed, 'control keys changed')

            require(bool(f.request('session_pane_input', dict(session=row, pane_id='%999999', hex='61')).get('error')), 'wrong pane input accepted')
            require(bool(f.request('session_pane_input', dict(session=row, pane_id=pane, hex='zz')).get('error')), 'invalid input accepted')
            for field in ('server_start', 'created', 'session_id'):
                stale = copy.deepcopy(row)
                stale['terminal'][field] = '$999999' if field == 'session_id' else '0'
                require(bool(f.request('session_preview', dict(session=stale)).get('error')), 'stale identity accepted')
                require(bool(f.request('session_pane_input', dict(session=stale, pane_id=pane, hex='61')).get('error')), 'stale input accepted')
            f.tmux('kill-session', '-t', target, default=default)
            require(bool(f.request('session_preview', dict(session=row)).get('error')), 'ended preview accepted')
            require(bool(f.request('session_pane_input', dict(session=row, pane_id=pane, hex='61')).get('error')), 'ended input accepted')
        print(json.dumps(dict(status='PASS', checks=['managed and external screen snapshots', 'no resize/attach/process mutation', 'full styled100-row screen and cursor metadata', 'input reaches actual selected shell', 'stale/wrong-pane/ended input refusal'])))
    finally:
        f.close()

if __name__ == '__main__':
    run(sys.argv[1])
