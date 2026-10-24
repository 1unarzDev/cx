#!/usr/bin/env python3
"""New Codex launches select supported --no-daemon; older CLI fallback stays valid."""
import json,os,pathlib,subprocess,sys,tempfile,time
binary=str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-codex-mode-') as t:
 root=pathlib.Path(t);(root/'bin').mkdir()
 (root/'fixture').mkdir();launcher=root/'fixture/bash'
 launcher.write_text('#!/bin/sh\nexport PATH='+str(root/'bin')+':/usr/bin:/bin\nshift\nexec /bin/bash --noprofile --norc \"$@\"\n');launcher.chmod(0o700)
 (root/'state/cx').mkdir(parents=True)
 (root/'state/cx/launcher.json').write_text(json.dumps(str(launcher)))
 env=dict(os.environ,HOME=t,XDG_STATE_HOME=t+'/state',SHELL='/bin/bash',TERM='xterm-256color')
 env.pop('TMUX',None);env.pop('TMUX_PANE',None)
 socket=t+'/state/cx/managed.sock'
 try:
  for supported in [True,False]:
   output=root/('supported.args' if supported else 'legacy.args')
   script=root/'bin/codex'
   help_text='  --no-daemon' if supported else '  --model'
   script.write_text('#!/bin/sh\ncase "$1" in --version) echo codex-fixture; exit 0;; --help) echo "'+help_text+'"; exit 0;; esac\nprintf "%s\\n" "$@" > "'+str(output)+'"\nexec sleep 30\n');script.chmod(0o700)
   row=json.loads(subprocess.check_output([binary,'new','--provider','codex','--directory',t,'--key',str(supported)],env=env))
   end=time.monotonic()+5
   while not output.exists():
    assert time.monotonic()<end,'provider launcher never ran'
    time.sleep(.04)
   assert ('--no-daemon' in output.read_text().splitlines())==supported, 'incorrect no-daemon capability fallback'
  print(json.dumps({'result':'PASS','checks':['supported new Codex uses separate process','legacy Codex launches without unsupported flag','provider config untouched']}))
 finally:
  subprocess.run(['tmux','-S',socket,'kill-server'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
