#!/bin/sh
# Install or remove cx-reptyr with cap_sys_ptrace. Runs as root through sudo.
# The staged file stays user-writable, so it is copied to a root-owned path
# first and only that copy is hash-checked, given the capability and renamed.
set -eu
dir=/usr/local/libexec/cx
target=$dir/cx-reptyr
[ "$(id -u)" = 0 ] || { echo 'cx: run through sudo' >&2; exit 1; }
case "${1:-}" in
disable)
    rm -f "$target" "$target.new"
    rmdir "$dir" 2>/dev/null || true
    exit 0
    ;;
enable)
    [ $# = 3 ] || { echo 'usage: enable-ptrace.sh enable STAGE SHA256' >&2; exit 2; }
    ;;
*)
    echo 'usage: enable-ptrace.sh enable STAGE SHA256 | disable' >&2
    exit 2
    ;;
esac
stage=$2
expected=$3
user=${SUDO_UID:?cx: run through sudo}
if [ ! -f "$stage" ] || [ -L "$stage" ] || [ "$(stat -c %u "$stage")" != "$user" ]; then
    echo 'cx: staged cx-reptyr is missing or not owned by the requesting user' >&2
    exit 1
fi
command -v setcap >/dev/null 2>&1 || { echo 'cx: setcap is required (package libcap2-bin or libcap)' >&2; exit 1; }
install -d -o root -g root -m 0755 "$dir"
rm -f "$target.new"
install -o root -g root -m 0755 "$stage" "$target.new"
actual=$(sha256sum "$target.new" | cut -d' ' -f1)
if [ "$actual" != "$expected" ]; then
    rm -f "$target.new"
    echo 'cx: staged cx-reptyr changed before installation' >&2
    exit 1
fi
setcap cap_sys_ptrace+ep "$target.new"
mv -f "$target.new" "$target"
