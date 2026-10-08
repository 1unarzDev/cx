#!/bin/sh
# Fixed remote stdin installer; called over authenticated SSH, not a root setup.
set -eu
[ "$(id -u)" -ne 0 ] || { echo 'cx: enroll an ordinary account' >&2; exit 1; }
# Honor existing user-owned launchers (including tmux wrappers) on SSH's minimal PATH.
PATH="$HOME/.local/bin:$PATH"
export PATH
missing=
tools='curl openssl ssh tmux pdftoppm pdfinfo ip ping infocmp flock tar gzip stat timeout install mktemp head sed grep'
if [ "${CX_ENROLL_MINIMAL:-0}" = 1 ]; then
    # The signed helper is downloaded/verified by the viewer, never by the robot.
    # Missing optional runtimes remain unavailable; no packages/profile changes.
    tools='id mkdir ssh ip flock stat timeout mktemp cat chmod mv rm'
fi
for tool in $tools; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        if [ "$tool" != ip ] || { [ ! -x /usr/sbin/ip ] && [ ! -x /sbin/ip ]; }; then
            missing="$missing $tool"
        fi
    fi
done
if [ -n "$missing" ]; then
    printf 'cx: missing host utilities:%s\nRun the cx installer on this host, then retry cx add.\n' "$missing" >&2
    exit 1
fi
umask 077
check_dir() {
    path=$1
    while [ "$path" != / ]; do
        [ ! -L "$path" ] || exit 1
        if [ -e "$path" ]; then
            [ -d "$path" ] || exit 1
            owner=$(stat -c %u "$path")
            [ "$owner" = "$(id -u)" ] || [ "$owner" = 0 ] || exit 1
            mode=$(stat -c %a "$path")
            [ "$((0$mode & 0022))" = 0 ] || exit 1
        fi
        path=${path%/*}; [ -n "$path" ] || path=/
    done
}
case "$HOME" in /*) ;; *) exit 1;; esac
check_dir "$HOME/.local/bin"
check_dir "$HOME/.local/state/cx"
mkdir -p "$HOME/.local/bin" "$HOME/.local/state/cx"
for path in "$HOME/.local/state/cx/maintenance.lock" "$HOME/.local/bin/cx"; do
    [ ! -L "$path" ] || exit 1
    if [ -e "$path" ]; then
        if [ ! -f "$path" ] || [ "$(stat -c %u "$path")" != "$(id -u)" ] || [ "$(stat -c %h "$path")" != 1 ]; then exit 1; fi
    fi
done
exec 9>>"$HOME/.local/state/cx/maintenance.lock"
flock -n 9 || exit 75
tmp=$(mktemp "$HOME/.local/bin/.cx-install.XXXXXX")
trap 'rm -f "$tmp"' EXIT
trap 'exit 130' INT
trap 'exit 143' HUP TERM
cat > "$tmp"
chmod 700 "$tmp"
timeout 10 "$tmp" --version >/dev/null
mv "$tmp" "$HOME/.local/bin/cx"
