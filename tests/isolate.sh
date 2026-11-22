#!/bin/sh
# Run a command in private user, pid, mount and cgroup namespaces: /proc lists only the
# command's own processes, so adoption discovery in a fixture viewer or helper can
# never see a real terminal on this machine, and the user bus under /run/user is
# hidden, so fixture tmux servers never become units of the caller's systemd.
# The private /dev/shm holds the cx-isolated marker: only there does a debug build
# discover terminals, and pty_lib start a viewer (tests/isolation.py).
# The uid and gid stay the caller's (files keep their owner). When the command
# ends, or this script is killed, every process inside goes with it.
#
#   tests/isolate.sh COMMAND [ARG...]
#
# The host's /etc/ssh/ssh_config is an empty file inside: shown as nobody's there, ssh
# refuses it ("Bad owner"), and fixtures must not depend on it anyway.
# The tty group is left out of /etc/group inside: its gid is not mapped there, and a
# fixture sshd would fail to chown the pseudo-terminal of `ssh -t` to it.
# Fails closed: without unprivileged user namespaces the command does not run.
# Always new namespaces, also when started inside some: nothing inherited turns it off.
# Outer namespace as root to mount; inner one back at the caller's ids. The inner
# shell stays pid 1 (no exec) and reaps orphaned fixture processes.
exec unshare --user --map-root-user --mount --pid --cgroup --fork --kill-child --mount-proc \
    sh -c 'uid=$1 gid=$2; shift 2
        mount -t tmpfs -o mode=1777 tmpfs /dev/shm || exit 125
        : >/dev/shm/cx-isolated || exit 125
        grep -v "^tty:" /etc/group >/dev/shm/group && mount --bind /dev/shm/group /etc/group || exit 125
        if [ -f /etc/ssh/ssh_config ]; then : >/dev/shm/ssh_config && mount --bind /dev/shm/ssh_config /etc/ssh/ssh_config || exit 125; fi
        if [ -d "/run/user/$uid" ]; then mount -t tmpfs -o mode=700 tmpfs "/run/user/$uid" || exit 125; fi
        unshare --user --map-user="$uid" --map-group="$gid" -- "$@"
        status=$?; exit $status' \
    sh "$(id -u)" "$(id -g)" "$@"
