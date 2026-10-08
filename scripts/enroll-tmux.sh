#!/bin/sh
# Install only an absent user tool, under the same lock as helper maintenance.
set -eu
[ "$(id -u)" -ne 0 ] || exit 1
PATH="$HOME/.local/bin:$PATH"
export PATH
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
for path in "$HOME/.local/state/cx/maintenance.lock" "$HOME/.local/bin/tmux"; do
    [ ! -L "$path" ] || exit 1
    if [ -e "$path" ]; then
        if [ ! -f "$path" ] || [ "$(stat -c %u "$path")" != "$(id -u)" ] || [ "$(stat -c %h "$path")" != 1 ]; then exit 1; fi
    fi
done
exec 9>>"$HOME/.local/state/cx/maintenance.lock"
flock -n 9 || exit 75

# Recheck after locking: another enrollment may have installed it meanwhile.
if command -v tmux >/dev/null 2>&1; then
    timeout 10 tmux -V >/dev/null || exit 1
    exit 0
fi
# Preserve even a broken existing tool. The user must decide how to repair it.
[ ! -e "$HOME/.local/bin/tmux" ] || exit 1
tmp=$(mktemp "$HOME/.local/bin/.tmux-install.XXXXXX")
trap 'rm -f "$tmp"' EXIT
trap 'exit 130' INT
trap 'exit 143' HUP TERM
cat > "$tmp"
chmod 700 "$tmp"
[ "$(timeout 10 "$tmp" -V)" = 'tmux 3.7c' ] || exit 1
mv "$tmp" "$HOME/.local/bin/tmux"
