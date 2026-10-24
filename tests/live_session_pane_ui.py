#!/usr/bin/env python3
"""Interactive live-pane acceptance on a disposable tmux server and viewer PTY."""
import json
import sys
sys.dont_write_bytecode = True
from session_adversarial import Fixture, require
import session_adversarial_ui as ui


def run(binary):
    f = Fixture(binary, live=True)
    viewer = None
    try:
        f.env.update(HTTPS_PROXY='http://127.0.0.1:9', HTTP_PROXY='http://127.0.0.1:9', ALL_PROXY='http://127.0.0.1:9')
        f.start_managed()
        f.create('live-ui', name='Synthetic interactive terminal')
        second = f.create('hover-ui', name='Hover terminal proof')
        f.send(second['id'], "printf '\\033[2J\\033[HHOVER-SCREEN-PROOF\\n'")
        (f.state / 'devices.json').write_text(json.dumps([dict(id='local',name='local-fixture',target=None,account='fixture',host='fixture-host',status='unknown',observed_at=0)]))
        original = ui.restart_state
        def local_restart():
            state = original()
            state.update(device_ids=['local'],device=1)
            return state
        ui.restart_state = local_restart
        viewer = ui.Viewer(f)
        viewer.wait(lambda: 'focus terminal' in viewer.text(), 'live terminal pane not shown')
        viewer.pump(2) # Allow actual Info reply to arrive; it must not disable previews.
        require('New shell' not in viewer.text(), 'duplicate shell button remains')
        target_row = next(i for i,line in enumerate(viewer.screen.display) if 'Hover terminal proof' in line[22:65])
        viewer.send(f'\x1b[<35;31;{target_row+1}M'.encode())
        viewer.wait(lambda: 'HOVER-SCREEN-PROOF' in viewer.text(), 'mouse hover did not display actual selected terminal after Info')
        viewer.send(b'\r')
        viewer.wait(lambda: 'back to sessions' in viewer.text(), 'Enter did not focus pane')
        proof = f.root / 'focused-proof'
        # Real bracketed paste into the viewer, then explicit Enter to submit.
        command = 'printf integrated > ' + str(proof)
        viewer.send(b'\x1b[200~' + command.encode() + b'\x1b[201~')
        viewer.send(b'\r')
        viewer.wait(proof.exists, 'focused paste/Enter did not reach running shell')
        require(proof.read_text() == 'integrated', 'shell input corrupted')
        viewer.send(b'\x1d')
        viewer.wait(lambda: 'back to sessions' not in viewer.text(), 'Ctrl+] did not return navigation')
        viewer.send(b'n')
        viewer.wait(lambda: 'Shell' in viewer.text() and 'Session' in viewer.text(), 'unified New session chooser missing Shell')
        viewer.send(b'\x1b')
        viewer.quit()
        require(len(f.rows()) == 2, 'focus created or killed a session')
        print(json.dumps(dict(status='PASS',checks=['actual live pane in viewer PTY','Enter focuses same running shell','bracketed paste and Enter delivered','Ctrl+] returns navigation','unified New session chooser','termios restored; runtime preserved'])))
    finally:
        if viewer: viewer.close()
        f.close()

if __name__ == '__main__':
    run(sys.argv[1])
