#!/usr/bin/env python3
"""Install only explicit mesh public keys on this ordinary downstream account."""
import base64
import fcntl
import json
import os
import pathlib
import re
import stat
import sys
import tempfile


def main():
    if os.getuid() == 0:
        raise SystemExit("Run as the destination SSH account, not root")
    raw = sys.stdin.buffer.read(65537)
    if len(raw) > 65536:
        raise SystemExit("Public-key enrollment exceeds limit")
    data = json.loads(raw)
    if data.get("version") != 1 or not isinstance(data.get("keys"), dict) or not 1 <= len(data["keys"]) <= 32:
        raise SystemExit("Invalid public-key enrollment")
    managed = {}
    for device, public in data["keys"].items():
        if not re.fullmatch(r"[A-Za-z0-9_.-]{1,64}", device):
            raise SystemExit("Invalid mesh identity")
        parts = public.split()
        if len(parts) != 2 or parts[0] != "ssh-ed25519":
            raise SystemExit("Only explicit Ed25519 public keys are supported")
        decoded = base64.b64decode(parts[1], validate=True)
        if len(decoded) != 51 or decoded[:19] != b"\x00\x00\x00\x0bssh-ed25519\x00\x00\x00\x20":
            raise SystemExit("Invalid Ed25519 public-key encoding")
        managed["cx-mesh:" + device] = "no-agent-forwarding,no-X11-forwarding " + public + " cx-mesh:" + device
    directory = pathlib.Path.home() / ".ssh"
    if not directory.exists():
        directory.mkdir(mode=0o700)
    meta = directory.lstat()
    if not stat.S_ISDIR(meta.st_mode) or meta.st_uid != os.getuid() or meta.st_mode & 0o022:
        raise SystemExit("Unsafe SSH directory")
    lock = os.open(directory / ".cx-enrollment.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        meta = os.fstat(lock)
        if not stat.S_ISREG(meta.st_mode) or meta.st_uid != os.getuid() or meta.st_nlink != 1 or meta.st_mode & 0o022:
            raise SystemExit("Unsafe enrollment lock")
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        target = directory / "authorized_keys"
        try:
            fd = os.open(target, os.O_RDONLY | os.O_NOFOLLOW)
        except FileNotFoundError:
            previous = ""
        else:
            with os.fdopen(fd, "rb") as file:
                meta = os.fstat(file.fileno())
                if not stat.S_ISREG(meta.st_mode) or meta.st_uid != os.getuid() or meta.st_nlink != 1 or meta.st_mode & 0o022:
                    raise SystemExit("Unsafe authorized_keys")
                previous = file.read(262145).decode("utf-8")
                if len(previous.encode()) > 262144:
                    raise SystemExit("authorized_keys exceeds limit")
        lines = previous.splitlines()
        # Remove only exactly CX-tagged records for the supplied mesh identities.
        kept = [line for line in lines if not line.split() or line.split()[-1] not in managed]
        merged = "\n".join(kept + list(managed.values())) + "\n"
        tmp_fd, temporary = tempfile.mkstemp(prefix=".cx-authorized-", dir=directory)
        try:
            with os.fdopen(tmp_fd, "w") as file:
                file.write(merged)
                file.flush()
                os.fsync(file.fileno())
            os.replace(temporary, target)
        finally:
            if os.path.exists(temporary):
                os.unlink(temporary)
        print(json.dumps({"installed_mesh_public_keys": sorted(data["keys"]),
                          "reverse_keys_installed": False, "unrelated_entries_preserved": True}))
    finally:
        os.close(lock)


if __name__ == "__main__":
    main()
