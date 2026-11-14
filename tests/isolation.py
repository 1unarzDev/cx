"""Whether this process runs inside tests/isolate.sh, and a way to get there.

Inside means both: a pid namespace that is not the machine's first one (inode
0xEFFFFFFC), and the marker isolate.sh writes into its private /dev/shm. A container
with a pid namespace of its own has no marker, so its processes count as real ones.
Checked, never inherited. The rust side is `sessions::external_terminals`.
"""
import os
import pathlib
import sys

MARKER = '/dev/shm/cx-isolated'


def isolated():
    try:
        return os.readlink('/proc/self/ns/pid') != 'pid:[4026531836]' and os.path.exists(MARKER)
    except OSError:
        return False


def isolate():
    """Re-run this script under tests/isolate.sh (private user, pid and mount namespaces), so
    the viewers it starts can never list a real terminal of this machine. Call it first;
    a no-op once inside."""
    if not isolated():
        script = str(pathlib.Path(__file__).resolve().parent / 'isolate.sh')
        os.execv(script, [script, sys.executable, *sys.argv])
