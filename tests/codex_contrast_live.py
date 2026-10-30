#!/usr/bin/env python3
"""Native Codex fallback palette regression, using only disposable synthetic panes."""
import sys,shlex,json,select,os,time
sys.dont_write_bytecode=True
from session_adversarial import Fixture,require
from session_adversarial_attach import Client

def run(binary):
 f=Fixture(binary,live=True)
 try:
  f.env['COLORTERM']='truecolor'
  source=f.root/'swatch.c'; source.write_text('#include <stdio.h>\n#include <unistd.h>\nint main(){printf("\\033[2J\\033[H\\033[48;5;8mPROMPT-BG\\033[49m\\n");fflush(stdout);for(;;)pause();}\n')
  f.run(['cc',str(source),'-o',str(f.bin/'codex')])
  f.start_managed();s=f.create('codex-contrast')
  f.send(s['id'],shlex.quote(str(f.bin/'codex')))
  f.until(lambda:any(r['provider']=='codex' for r in f.rows()),'synthetic Codex detection')
  s=next(r for r in f.rows() if r['provider']=='codex')
  def attach(check_colour=False):
   c=Client(f,s)
   try:
    c.wait_attached(False,s['terminal']['session_id'],False)
    if check_colour:
     f.tmux('refresh-client','-t',c.tty)
     output=bytearray();end=time.monotonic()+0.3
     while time.monotonic()<end:
      if select.select([c.master],[],[],0.02)[0]:output.extend(os.read(c.master,65536))
     require(b'48;2;48;48;48' in output,'native terminal did not receive muted RGB background')
    c.detach(False)
   finally:c.close()
  attach(check_colour=True)
  palette=f.tmux('show-options','-p','-qv','-t',s['id'],'pane-colours[8]').stdout.strip()
  require(palette==b'#303030','native Codex prompt background still depends on accent palette8')
  require(f.request('session_preview',dict(session=s))['result'].get('palette8')=='#303030','embedded pane loses native fallback colour')
  # Existing explicit preferences win; CX does not rewrite a user override.
  f.tmux('set-option','-p','-t',s['id'],'pane-colours[8]','#45475a')
  attach()
  require(f.tmux('show-options','-p','-qv','-t',s['id'],'pane-colours[8]').stdout.strip()==b'#45475a','explicit pane palette overwritten')
  # Returning to shell clears only our fallback, not an explicit override.
  f.tmux('set-option','-p','-t',s['id'],'pane-colours[8]','#303030')
  f.tmux('send-keys','-t',s['id'],'C-c')
  f.until(lambda:all(r['provider']!='codex' for r in f.rows()),'shell return')
  s=f.rows()[0];attach()
  require(not f.tmux('show-options','-p','-qv','-t',s['id'],'pane-colours[8]').stdout.strip(),'Codex palette leaked into shell')
  print(json.dumps({'status':'PASS','checks':['native muted prompt fallback','embedded palette parity','explicit override preserved','fallback removed on shell return']}))
 finally:f.close()
if __name__=='__main__':run(sys.argv[1])
