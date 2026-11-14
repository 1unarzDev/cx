#!/usr/bin/env python3
"""Capture cx's metadata UI into terminal text and a rendered PTY image; no attachments.

usage: capture_ui.py BINARY [COLS] [work]   (a fixture HOME, under tests/isolate.sh)
"""
import os,pty,subprocess,select,time,termios,fcntl,struct,sys,pathlib,tempfile,shutil
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parent))
import isolation
isolation.isolate()
import pyte
from PIL import Image,ImageDraw,ImageFont
binary=str(pathlib.Path(sys.argv[1]).resolve());cols=int(sys.argv[2]) if len(sys.argv)>2 else 120;rows=32
home=tempfile.mkdtemp(prefix='cx-capture-ui-')
master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',rows,cols,0,0))
env=dict(os.environ,TERM='xterm-256color',HOME=home,XDG_STATE_HOME=home+'/state',TMUX_TMPDIR=home);env.pop('TMUX',None);env.pop('TMUX_PANE',None);env.pop('NO_COLOR',None)
def control():os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
p=subprocess.Popen([binary],stdin=slave,stdout=slave,stderr=slave,preexec_fn=control,env=env,cwd=home)
screen=pyte.Screen(cols,rows);stream=pyte.Stream(screen)
def read(seconds):
 end=time.monotonic()+seconds
 while time.monotonic()<end:
  if select.select([master],[],[],.05)[0]:
   try:stream.feed(os.read(master,65536).decode(errors='replace'))
   except OSError:break
read(2)
if len(sys.argv)<4 or sys.argv[3]!='work':os.write(master,b'/cpc');read(.4)
root=pathlib.Path(__file__).resolve().parent.parent/'local-evidence';root.mkdir(exist_ok=True)
(root/f'ui-{cols}.txt').write_text('\n'.join(screen.display))
font=ImageFont.truetype(subprocess.check_output(['fc-match','-f','%{file}','monospace'],text=True),16);cellw=10;cellh=21
im=Image.new('RGB',(cols*cellw+24,rows*cellh+24),'#171a1e');draw=ImageDraw.Draw(im)
colors={'default':'#cbd0d7','black':'#171a1e','red':'#dc817d','green':'#91b49b','brown':'#c7b887','blue':'#8ca7cb','magenta':'#b29cc8','cyan':'#8cbabd','white':'#d6dce4'}
for y in range(rows):
 for x in range(cols):
  c=screen.buffer[y][x];fg=colors.get(c.fg,'#'+c.fg if len(c.fg)==6 else '#cbd0d7');bg='#171a1e' if c.bg=='default' else colors.get(c.bg,'#171a1e')
  if c.reverse:fg,bg=bg,fg
  if bg!='#171a1e':draw.rectangle((12+x*cellw,12+y*cellh,12+(x+1)*cellw,12+(y+1)*cellh),fill=bg)
  draw.text((12+x*cellw,12+y*cellh),c.data,font=font,fill=fg)
im.save(root/f'ui-{cols}.png');os.write(master,b'\x03');p.wait(timeout=3);os.close(master);os.close(slave);shutil.rmtree(home,ignore_errors=True)
print(root/f'ui-{cols}.png')
