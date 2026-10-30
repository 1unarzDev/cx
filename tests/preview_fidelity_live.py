#!/usr/bin/env python3
"""Synthetic native/preview colour and deep-history acceptance; never reads personal sessions."""
import sys,json,os,fcntl,termios,struct,signal
sys.dont_write_bytecode=True
from session_adversarial import Fixture,require
import session_adversarial_ui as ui

def run(binary):
 f=Fixture(binary,live=True);viewer=None
 try:
  f.start_managed();s=f.create('fidelity',name='Synthetic fidelity')
  f.tmux('set-option','-p','-t',s['id'],'pane-colours[8]','#303030')
  f.send(s['id'], "printf '\\033[2J\\033[H\\033[48;5;8mFIRST-PROMPT\\033[K\\nSECOND-PROMPT\\033[K\\033[0m\\n'")
  f.until(lambda:b'SECOND-PROMPT' in f.tmux('capture-pane','-p','-t',s['id']).stdout,'synthetic ready')
  (f.state/'devices.json').write_text(json.dumps([dict(id='local',name='fixture',target=None,account='fixture',host='fixture',status='unknown',observed_at=0)]))
  original=ui.restart_state
  def restart():
   state=original();state.update(device_ids=['local'],device=1);return state
  ui.restart_state=restart
  viewer=ui.Viewer(f)
  viewer.wait(lambda:'SECOND-PROMPT' in viewer.text(),'preview missing')
  for width in [80,120]:
   viewer.screen.resize(lines=30,columns=width)
   fcntl.ioctl(viewer.slave,termios.TIOCSWINSZ,struct.pack('HHHH',30,width,0,0))
   os.kill(viewer.process.pid,signal.SIGWINCH)
   viewer.pump(0.3)
   for marker in ['FIRST-PROMPT','SECOND-PROMPT']:
    row=next(i for i,line in enumerate(viewer.screen.display) if marker in line)
    col=viewer.screen.display[row].index(marker)
    require(viewer.screen.buffer[row][col].bg=='303030','prompt background lost or accent-coloured')
    require(viewer.screen.buffer[row][col+len(marker)+4].bg=='303030','prompt background only paints behind letters')
  f.send(s['id'], "for i in $(seq 1 1200); do printf 'DEEP-ROW-%04d\\n' $i; done")
  f.until(lambda:b'DEEP-ROW-1200' in f.tmux('capture-pane','-p','-t',s['id']).stdout,'history ready')
  page=f.request('session_preview',dict(session=s,scrollback=True,history_start=0))['result']
  require(any('DEEP-ROW-0001' in line for line in page['styled_lines']),'cannot reach history beyond recent 500 rows')
  viewer.wait(lambda:'DEEP-ROW-1200' in viewer.text(),'latest history not visible')
  for _ in range(430):
   os.write(viewer.master,b'\x1b[<64;100;15M');viewer.pump(0.006)
  viewer.wait(lambda:'DEEP-ROW-0001' in viewer.text(),'wheel paging cannot reach beginning')
  for _ in range(430):
   os.write(viewer.master,b'\x1b[<65;100;15M');viewer.pump(0.006)
  viewer.wait(lambda:'DEEP-ROW-1200' in viewer.text(),'wheel paging cannot return live')
  app=f.root/'scroll-app.py'
  app.write_text("import os,tty,sys\ntty.setraw(0)\nsys.stdout.write('\\x1b[?1049h\\x1b[?1000h\\x1b[?1006h\\x1b[2J\\x1b[HAPP-LATEST');sys.stdout.flush()\nwhile True:\n data=os.read(0,4096)\n text='APP-OLDER' if b'[<64;' in data else 'APP-LATEST'\n sys.stdout.write('\\x1b[2J\\x1b[H'+text);sys.stdout.flush()\n")
  f.send(s['id'], '/usr/bin/python3 '+str(app))
  viewer.wait(lambda:'APP-LATEST' in viewer.text(),'alternate application missing')
  viewer.send(b'\x1b[<64;100;15M')
  viewer.wait(lambda:'APP-OLDER' in viewer.text(),'wheel did not reach application-owned history')
  viewer.send(b'\x1b[<65;100;15M')
  viewer.wait(lambda:'APP-LATEST' in viewer.text(),'application scroll down failed')
  viewer.quit();viewer=None
  print(json.dumps(dict(status='PASS' ,checks=['full-width muted prompts','multiline colour continuity','history beyond 500 rows', 'wheel pages to oldest and returns live', 'alternate application wheel scrolling'])))
 finally:
  if viewer:viewer.close()
  f.close()
if __name__=='__main__':run(sys.argv[1])
