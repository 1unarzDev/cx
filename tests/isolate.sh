#!/bin/sh
# Run a command in private user, pid, mount and cgroup namespaces: /proc lists only the
# command's own processes, so adoption discovery in a fixture viewer or helper can
# never see a real terminal on this machine, and the user bus under /run/user is
# hidden, so fixture tmux servers never become units of the caller's systemd.
# The uid and gid stay the caller's (files keep their owner). When the command
# ends, or this script is killed, every process inside goes with it.
#
#   tests/isolate.sh COMMAND [ARG...]
#
# The tty group is left out of /etc/group inside: its gid is not mapped there, and a
# fixture sshd would fail to chown the pseudo-terminal of `ssh -t` to it.
# Fails closed: without unprivileged user namespaces the command does not run.
# Already inside (CX_TEST_NS=1): runs the command directly.
[ "${CX_TEST_NS:-}" = 1 ] && exec "$@"
export CX_TEST_NS=1
# Outer namespace as root to mount; inner one back at the caller's ids. The inner
# shell stays pid 1 (no exec) and reaps orphaned fixture processes.
exec unshare --user --map-root-user --mount --pid --cgroup --fork --kill-child --mount-proc \
    sh -c 'uid=$1 gid=$2; shift 2
        mount -t tmpfs -o mode=1777 tmpfs /dev/shm || exit 125
        grep -v "^tty:" /etc/group >/dev/shm/group && mount --bind /dev/shm/group /etc/group || exit 125
        if [ -d "/run/user/$uid" ]; then mount -t tmpfs -o mode=700 tmpfs "/run/user/$uid" || exit 125; fi
        unshare --user --map-user="$uid" --map-group="$gid" -- "$@"
        status=$?; exit $status' \
    sh "$(id -u)" "$(id -g)" "$@"
