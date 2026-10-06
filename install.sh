#!/bin/sh
# User-only verified release bootstrap. Existing sessions and offline installs are preserved.
set -eu
fail() { printf '%s\n' "cx install: $*" >&2; exit 1; }
[ "$(id -u)" -ne 0 ] || fail 'run as your ordinary user, not root'
[ "$(uname -s)" = Linux ] || fail 'supported platforms: Linux (Ubuntu, RHEL-compatible, Arch)'
case $(uname -m) in x86_64|amd64) CX_ARCH=x86_64;; aarch64|arm64) CX_ARCH=aarch64;; *) fail 'unsupported architecture';; esac
[ "${CX_REPO:-1unarzDev/cx}" = 1unarzDev/cx ] || fail 'alternate release repositories are unsupported'
CX_PREFIX=${CX_PREFIX:-"$HOME/.local"}
CX_STATE=${XDG_STATE_HOME:-"$HOME/.local/state"}/cx
case "$CX_PREFIX:$CX_STATE" in /*:/*) ;; *) fail 'installation and state paths must be absolute';; esac
# Install required tools only after an explicit controlling-terminal confirmation.
CX_MISSING=
for CX_TOOL in curl openssl ssh tmux flock tar stat timeout install mktemp head sed grep; do
    command -v "$CX_TOOL" >/dev/null 2>&1 || CX_MISSING="$CX_MISSING $CX_TOOL"
done
if [ -n "$CX_MISSING" ]; then
    [ "${CX_INSTALL_DEPS:-ask}" != never ] || fail "missing tools:$CX_MISSING; install them with your package manager"
    if command -v apt-get >/dev/null 2>&1; then
        set -- apt-get install -y curl openssl openssh-client tmux ca-certificates util-linux tar coreutils
    elif command -v dnf >/dev/null 2>&1; then
        set -- dnf install -y curl openssl openssh-clients tmux ca-certificates util-linux tar coreutils
    elif command -v yum >/dev/null 2>&1; then
        set -- yum install -y curl openssl openssh-clients tmux ca-certificates util-linux tar coreutils
    elif command -v pacman >/dev/null 2>&1; then
        set -- pacman -S --needed curl openssl openssh tmux ca-certificates util-linux tar coreutils
    else fail "missing tools:$CX_MISSING; no supported package manager"; fi
    command -v sudo >/dev/null 2>&1 || fail "missing tools:$CX_MISSING; install as administrator then retry"
    ( : </dev/tty ) 2>/dev/null || fail "missing tools:$CX_MISSING; interactive dependency setup requires a terminal"
    printf 'Install required tools using sudo %s? [y/N] ' "$*" >/dev/tty
    read -r CX_REPLY </dev/tty
    case "$CX_REPLY" in y|Y|yes|YES) sudo "$@";; *) fail 'dependency installation declined';; esac
    for CX_TOOL in curl openssl ssh tmux flock tar stat timeout install mktemp head sed grep; do
        command -v "$CX_TOOL" >/dev/null 2>&1 || fail "required tool still unavailable: $CX_TOOL"
    done
fi
# Refuse symlink/foreign/writeable parent chains before creating owned resources.
safe_dir() {
    CX_CHECK=$1
    case "$CX_CHECK" in /*) ;; *) fail 'relative resource path';; esac
    case "$CX_CHECK/" in *'/../'*|*'/./'*|*'//'*) fail 'noncanonical resource path';; esac
    while [ "$CX_CHECK" != / ]; do
        [ ! -L "$CX_CHECK" ] || fail "symlink directory: $CX_CHECK"
        if [ -e "$CX_CHECK" ]; then
            [ -d "$CX_CHECK" ] || fail "not a directory: $CX_CHECK"
            CX_OWNER=$(stat -c %u "$CX_CHECK")
            [ "$CX_OWNER" = "$(id -u)" ] || [ "$CX_OWNER" = 0 ] || fail "foreign directory: $CX_CHECK"
            CX_MODE=$(stat -c %a "$CX_CHECK")
            [ "$((0$CX_MODE & 0022))" = 0 ] || fail "writeable parent directory: $CX_CHECK"
        fi
        CX_CHECK=${CX_CHECK%/*}; [ -n "$CX_CHECK" ] || CX_CHECK=/
    done
}
safe_file() {
    [ ! -L "$1" ] || fail "symlink resource: $1"
    if [ -e "$1" ]; then
        [ -f "$1" ] && [ "$(stat -c %u "$1")" = "$(id -u)" ] && [ "$(stat -c %h "$1")" = 1 ] || fail "foreign or linked resource: $1"
    fi
}
safe_dir "$CX_PREFIX/bin"
safe_dir "$CX_STATE"
umask 077
mkdir -p "$CX_STATE" "$CX_PREFIX/bin"
[ "$(stat -c %u "$CX_STATE")" = "$(id -u)" ] || fail 'foreign state directory'
safe_file "$CX_STATE/maintenance.lock"
: >>"$CX_STATE/maintenance.lock"
exec 9>>"$CX_STATE/maintenance.lock"
flock -n 9 || fail 'another cx maintenance operation is active; existing cx unchanged'
safe_file "$CX_PREFIX/bin/cx"
CX_TMP=$(mktemp -d "$CX_STATE/install.XXXXXX")
CX_INSTALLING=
cleanup() { [ -z "$CX_INSTALLING" ] || rm -f "$CX_INSTALLING"; rm -rf "$CX_TMP"; }
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP
cat >"$CX_TMP/release-key.pem" <<'CX_PUBLIC_KEY'
-----BEGIN PUBLIC KEY-----
MIICIjANBgkqhkiG9w0BAQEFAAOCAg8AMIICCgKCAgEAyOj1t+p3uMcSx7D2EgJ1
Bjr/fruFB3Lj7vufzNHWHcvL3IVdCh+x69ChYe27G8o3GAEp7XNDjs8Q5Owd7GGu
kD1gK29TYGrPp6cHlua8xGZYf3VY/sMiSFuZC6eZiWWsInIzBL3MUNx84pmEF5Ud
2ZTZNeKNEQBOiCEXLHzz69K6uX/oceX1d4TkdryACJYxKU5GlW1wBhOoEcDYn3FM
OppAekdjJEFN6pG4lSfzE6+2dVs5U3XDJW1T+f9EK7W/g7IngRO+rD+pbcz5Z8Mm
myBXGi+bePTp9TG2IU5TfP+WF1pCXM8AwhOxGNRsk0zh//QdTyCDf1fJmUAFl3aN
bFkNrGWlUrUWlu1l+yGyk/JnTgns0rWSC969NEDACpkYB88SPAh7xC8PxXDeXg7f
ZkeZ8kDzIP7VO7ZWV3LtP3wElbpvNchkoIhx3a05x0pY4M2ELksJzYeBTpCBkqko
iT7cBuPFGBtD4g37LNsCymVaWxM5ecZ0Uw1tAMSm1cEkUZ8IeUXotGZjqL8aDqto
JMfsRMr3qI18Lwjs43/ipRlFnr6YEyIuKOvOHy00RYTYPHTbtx31ih4yBzpyDwBi
xg88O9Ft1ukclAlx+O0VZ3Jlyy2TjDuKEqaZi5RCB/FfNMbFaJjusxOJrqIOEeHo
UwBQK/sksgt8o1NmnQMGStkCAwEAAQ==
-----END PUBLIC KEY-----
CX_PUBLIC_KEY
download() {
    curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL --retry 1 --connect-timeout 5 --max-time 60 --max-filesize "$3" "$1" -o "$2" || fail 'release unavailable; existing cx unchanged. Retry when connected'
    [ "$(stat -c %s "$2")" -le "$3" ] || fail 'oversized release response'
}
CX_VERSION=${CX_VERSION:-}
if [ -z "$CX_VERSION" ]; then
    download https://api.github.com/repos/1unarzDev/cx/releases/latest "$CX_TMP/release.json" 262144
    CX_VERSION=$(sed -nE 's/^[[:space:]]*"tag_name":[[:space:]]*"(v[0-9]+\.[0-9]+\.[0-9]+)",?[[:space:]]*$/\1/p' "$CX_TMP/release.json")
fi
printf '%s\n' "$CX_VERSION" | LC_ALL=C grep -Eq '^v(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})$' || fail 'version must be a stable vMAJOR.MINOR.PATCH tag'
CX_ASSET=cx-$CX_VERSION-linux-$CX_ARCH.tar.gz
CX_URL=https://github.com/1unarzDev/cx/releases/download/$CX_VERSION/$CX_ASSET
download "$CX_URL" "$CX_TMP/artifact" 33554432
download "$CX_URL.sig" "$CX_TMP/signature" 512
[ "$(stat -c %s "$CX_TMP/signature")" = 512 ] || fail 'invalid signature size'
openssl dgst -sha256 -verify "$CX_TMP/release-key.pem" -signature "$CX_TMP/signature" "$CX_TMP/artifact" >/dev/null 2>&1 || fail 'release signature verification failed'
# A signed archive is still checked: one regular member named cx, no links/devices.
LC_ALL=C timeout 15 tar -tzf "$CX_TMP/artifact" >"$CX_TMP/members" || fail 'invalid archive'
[ "$(cat "$CX_TMP/members")" = cx ] || fail 'unexpected archive members'
LC_ALL=C timeout 15 tar -tvzf "$CX_TMP/artifact" >"$CX_TMP/types" || fail 'invalid archive'
case $(cat "$CX_TMP/types") in -*) ;; *) fail 'archive member must be a regular file';; esac
# Stream at most 64 MiB + one byte; never extract archive paths onto the filesystem.
timeout 15 tar -xOzf "$CX_TMP/artifact" cx | head -c 67108865 >"$CX_TMP/cx"
[ "$(stat -c %s "$CX_TMP/cx")" -le 67108864 ] && [ -s "$CX_TMP/cx" ] || fail 'invalid binary size'
chmod 700 "$CX_TMP/cx"
CX_ACTUAL=$(timeout 10 "$CX_TMP/cx" --version) || fail 'release binary cannot run on this system'
[ "$CX_ACTUAL" = "cx ${CX_VERSION#v}" ] || fail 'release binary version mismatch'
CX_CONFIG=${XDG_CONFIG_HOME:-"$HOME/.config"}
safe_dir "$CX_CONFIG/cx"
safe_dir "$CX_CONFIG/fish/conf.d"
safe_file "$CX_CONFIG/cx/env.sh"
safe_file "$CX_CONFIG/fish/conf.d/cx-path.fish"
CX_INSTALLING=$(mktemp "$CX_PREFIX/bin/.cx-install.XXXXXX")
install -m 700 "$CX_TMP/cx" "$CX_INSTALLING"
mv -f "$CX_INSTALLING" "$CX_PREFIX/bin/cx"
CX_INSTALLING=
# Fish loads additive conf.d snippets; Bash/Zsh use the owned sourceable env file.
case "$CX_PREFIX" in *[!a-zA-Z0-9_./-]*) printf '%s\n' 'Custom prefix: configure your shell PATH manually.';; *)
    mkdir -p "$CX_CONFIG/cx" "$CX_CONFIG/fish/conf.d"
    printf "case :\$PATH: in *:'%s':*) ;; *) export PATH='%s':\$PATH;; esac\n" "$CX_PREFIX/bin" "$CX_PREFIX/bin" >"$CX_CONFIG/cx/env.sh"
    printf 'if status is-interactive; and not contains -- %s/bin $PATH\n    set -gx PATH %s/bin $PATH\nend\n' "$CX_PREFIX" "$CX_PREFIX" >"$CX_CONFIG/fish/conf.d/cx-path.fish";; esac
# Offer one owned additive source line; never rewrite an existing shell configuration.
case "$CX_CONFIG" in *[!a-zA-Z0-9_./-]*) ;; *)
    CX_STARTUP=
    case "${SHELL:-}" in */bash) CX_STARTUP=$HOME/.bashrc;; */zsh) CX_STARTUP=${ZDOTDIR:-"$HOME"}/.zshrc;; esac
    if [ -n "$CX_STARTUP" ] && [ -f "$CX_CONFIG/cx/env.sh" ] && [ "${CX_NO_LAUNCH:-0}" != 1 ] && ( : </dev/tty ) 2>/dev/null; then
        CX_LINE="[ ! -r '$CX_CONFIG/cx/env.sh' ] || . '$CX_CONFIG/cx/env.sh' # cx PATH"
        if ! grep -Fqx -- "$CX_LINE" "$CX_STARTUP" 2>/dev/null; then
            printf 'Add cx PATH to %s? [y/N] ' "$CX_STARTUP" >/dev/tty
            read -r CX_REPLY </dev/tty
            case "$CX_REPLY" in y|Y|yes|YES)
                safe_dir "${CX_STARTUP%/*}"; safe_file "$CX_STARTUP"
                printf '\n%s\n' "$CX_LINE" >>"$CX_STARTUP";; esac
        fi
    fi;; esac
flock -u 9
exec 9>&-
cleanup
trap - EXIT INT TERM HUP
printf 'Installed %s/bin/cx (%s).\n' "$CX_PREFIX" "$CX_VERSION"
printf 'Bash/Zsh: source %s/cx/env.sh for PATH; existing shell config is preserved.\n' "$CX_CONFIG"
[ "${CX_NO_LAUNCH:-0}" = 1 ] && exit 0
if ( : </dev/tty ) 2>/dev/null; then exec "$CX_PREFIX/bin/cx" </dev/tty; fi
printf 'Run %s/bin/cx from a terminal.\n' "$CX_PREFIX"
