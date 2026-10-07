#!/usr/bin/env python3
"""Worker-thread editor retains wheel ownership, including reading /dev/tty.
No provider, transcripts, user config, or inference. Usage: script BINARY.
"""
import fcntl, json, os, pathlib, pty, select, shlex, struct, subprocess, sys, shutil, tempfile, termios, time
binary = str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-wheel-') as temporary:
    root = pathlib.Path(temporary)
    (root/'.local/bin').mkdir(parents=True)
    (root/'.local/bin/cx').symlink_to(binary)
    env = dict(os.environ, HOME=str(root), XDG_STATE_HOME=str(root/'state'), SHELL='/bin/bash', TERM='xterm-256color')
    env.pop('TMUX', None); env.pop('TMUX_PANE', None)
    socket = str(root/'state/cx/managed.sock')
    session = json.loads(subprocess.check_output([binary, 'new', '--provider', 'shell', '--directory', str(root), '--key', 'scroll-fixture'], env=env))
    def tmux(*args):
        return subprocess.check_output(['tmux', '-S', socket, *args], env=env).decode().strip()
    tmux('set-environment', '-g', 'HOME', str(root))
    tmux('set-environment', '-g', 'XDG_STATE_HOME', str(root/'state'))
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
    def tty():
        os.setsid(); fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
    client = subprocess.Popen([binary, 'native-attach', 'attach-session', '-t', session['id']], stdin=slave, stdout=slave, stderr=slave, env=env, preexec_fn=tty)
    screen = bytearray()
    def drain():
        while select.select([master], [], [], 0)[0]:
            screen.extend(os.read(master, 65536))
    def wait(predicate, message):
        deadline = time.monotonic()+6
        while not predicate():
            drain()
            assert client.poll() is None, 'client unexpectedly exited'
            assert time.monotonic() < deadline, message
            time.sleep(.02)
    pane = session['id'] + ':0.0'
    def state(format): return tmux('display-message', '-p', '-t', pane, format)
    log=root/'editor-input'
    code=root/'thread-editor.c'
    code.write_text(r"""#include <pthread.h>
#include <unistd.h>
#include <stdio.h>
#include <termios.h>
#include <fcntl.h>
#include <sys/wait.h>
char *path;
void *worker(void *unused) {
 pid_t child=fork();
 if(!child) {
  int nil=open("/dev/null",O_RDONLY);dup2(nil,0);char output[4096];
  snprintf(output,sizeof(output),"of=%s",path);
  execl("/bin/dd","dd","if=/dev/tty",output,"bs=1",(char*)0);_exit(1);
 }
 waitpid(child,0,0);return 0;
}
int main(int argc,char **argv) {
 path=argv[1];struct termios t;tcgetattr(0,&t);cfmakeraw(&t);tcsetattr(0,TCSANOW,&t);
 printf("\033[?1049hSYNTHETIC_EDITOR\r\n");fflush(stdout);
 pthread_t thread;pthread_create(&thread,0,worker,0);pthread_join(thread,0);return 0;
}
""")
    executable=root/'codex'
    subprocess.run(['cc','-pthread',str(code),'-o',str(executable)],check=True)
    try:
        wait(lambda: 'native-wheel' in tmux('list-keys','-T','root'),'binding not ready')
        command='exec '+shlex.quote(str(executable))+' '+shlex.quote(str(log))
        tmux('send-keys','-t',pane,'-l',command);tmux('send-keys','-t',pane,'Enter')
        wait(log.exists,'worker-thread editor not ready')
        tmux('clear-history','-t',pane)
        pid=state('#{pane_pid}')
        os.write(master,b'\x1b[<64;10;10M')
        time.sleep(.3);drain()
        assert log.read_bytes()==b'', 'worker-thread editor received fabricated wheel input'
        assert state('#{pane_in_mode}')=='0'
        assert state('#{pane_pid}')==pid
        os.write(master,b'\x1d');client.wait(timeout=5)
        print(json.dumps(dict(result='PASS',worker_thread_editor_no_input=True,redirected_stdin=True,same_process=True)))
    finally:
        if client.poll() is None: client.terminate(); client.wait(timeout=5)
        os.close(master); os.close(slave)
        subprocess.run(['tmux','-S',socket,'kill-server'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
