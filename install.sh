#!/bin/sh
set -eu
# Verified stable releases only; offline failure preserves an existing installation.
CX_REPO=${CX_REPO:-1unarzDev/cx}
CX_VERSION=${CX_VERSION:-}
CX_PREFIX=${CX_PREFIX:-"$HOME/.local"}
[ "$CX_REPO" = 1unarzDev/cx ] || { printf '%s\n' 'cx install: this build updates only from 1unarzDev/cx; alternate repositories are unsupported' >&2; exit 1; }
fail() { printf '%s\n' "cx install: $*" >&2; exit 1; }
[ "$(id -u)" -ne 0 ] || fail 'run as your ordinary user'
[ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ] || fail 'this release supports Linux x86_64'
case "$CX_REPO" in *[!a-zA-Z0-9_./-]*|../*|/*) fail 'invalid repository';; esac
command -v curl >/dev/null || fail 'curl is required'
command -v gh >/dev/null || fail 'GitHub CLI is required to verify release provenance against the repository trust root'
command -v ssh >/dev/null || fail 'OpenSSH is required; install using your OS package manager'
CX_TMP=$(mktemp -d)
trap 'rm -rf "$CX_TMP"' EXIT HUP INT TERM
if [ -z "$CX_VERSION" ]; then
    curl -fsSL --connect-timeout 5 --max-time 15 --max-filesize 262144 \
        "https://api.github.com/repos/$CX_REPO/releases/latest" -o "$CX_TMP/release.json" \
        || fail 'release service unavailable; existing cx unchanged'
    # GitHub's public API emits this fixed numeric tag; any other form fails closed.
    CX_VERSION=$(sed -nE 's/^[[:space:]]*"tag_name": "(v[0-9]+\.[0-9]+\.[0-9]+)",?[[:space:]]*$/\1/p' "$CX_TMP/release.json")
fi
case "$CX_VERSION" in v[0-9]* ) ;; *) fail 'version must be a stable release tag';; esac
CX_NUMERIC=${CX_VERSION#v}
case "$CX_NUMERIC" in *[!0-9.]* ) fail 'version must use vMAJOR.MINOR.PATCH';; esac
CX_SAVED_IFS=$IFS
IFS=.
set -- $CX_NUMERIC
IFS=$CX_SAVED_IFS
[ "$#" = 3 ] || fail 'version must use vMAJOR.MINOR.PATCH'
for CX_PART in "$@"; do [ -n "$CX_PART" ] || fail 'empty version component'; done
CX_ASSET=cx-$CX_VERSION-linux-x86_64.tar.gz
curl -fsSL --connect-timeout 5 --max-time 60 --max-filesize 33554432 "https://github.com/$CX_REPO/releases/download/$CX_VERSION/$CX_ASSET" -o "$CX_TMP/$CX_ASSET" \
    || fail 'release download unavailable; existing cx unchanged. Retry when connected'
curl -fsSL --connect-timeout 5 --max-time 60 --max-filesize 1048576 "https://github.com/$CX_REPO/releases/download/$CX_VERSION/$CX_ASSET.intoto.jsonl" -o "$CX_TMP/provenance.jsonl" \
    || fail 'release verification download unavailable; existing cx unchanged. Retry when connected'
GH_HOST=github.com GH_PROMPT_DISABLED=1 GH_DEBUG= GH_FORCE_TTY= timeout 60 gh attestation verify "$CX_TMP/$CX_ASSET" --bundle "$CX_TMP/provenance.jsonl" --repo "$CX_REPO" --signer-workflow "$CX_REPO/.github/workflows/release.yml" --source-ref "refs/tags/$CX_VERSION" --deny-self-hosted-runners >/dev/null 2>&1 || fail 'release provenance verification failed'
tar -tzf "$CX_TMP/$CX_ASSET" > "$CX_TMP/members"
[ "$(cat "$CX_TMP/members")" = cx ] || fail 'unexpected archive members'
# Read the one allowlisted member to stdout; archive paths/links cannot write elsewhere.
tar -xOzf "$CX_TMP/$CX_ASSET" cx > "$CX_TMP/cx"
mkdir -p "$CX_PREFIX/bin"
# Installer and updater serialize through the same per-user lock; never replace its inode.
CX_STATE=${XDG_STATE_HOME:-"$HOME/.local/state"}/cx
[ ! -L "$CX_STATE" ] || fail 'unsafe cx state directory'
mkdir -p "$CX_STATE"
[ "$(stat -c %u "$CX_STATE")" = "$(id -u)" ] || fail 'foreign cx state directory'
chmod 700 "$CX_STATE"
[ ! -L "$CX_STATE/maintenance.lock" ] || fail 'unsafe maintenance lock'
(umask 077; : >> "$CX_STATE/maintenance.lock")
[ "$(stat -c %u "$CX_STATE/maintenance.lock")" = "$(id -u)" ] || fail 'foreign maintenance lock'
[ "$(stat -c %h "$CX_STATE/maintenance.lock")" = 1 ] || fail 'unsafe maintenance lock links'
exec 9>>"$CX_STATE/maintenance.lock"
flock -n 9 || fail 'another cx maintenance operation is active; current cx unchanged'
[ ! -L "$CX_PREFIX/bin/cx" ] && [ ! -d "$CX_PREFIX/bin/cx" ] || fail 'unsafe cx installation target'
if [ -e "$CX_PREFIX/bin/cx" ]; then
    [ "$(stat -c %u "$CX_PREFIX/bin/cx")" = "$(id -u)" ] || fail 'foreign cx installation target'
    [ "$(stat -c %h "$CX_PREFIX/bin/cx")" = 1 ] || fail 'unsafe cx installation links'
fi
CX_INSTALLING=$(mktemp "$CX_PREFIX/bin/.cx-install.XXXXXX")
trap 'rm -f "$CX_INSTALLING"; rm -rf "$CX_TMP"' EXIT HUP INT TERM
install -m 700 "$CX_TMP/cx" "$CX_INSTALLING"
"$CX_INSTALLING" --version
mv "$CX_INSTALLING" "$CX_PREFIX/bin/cx"
flock -u 9
exec 9>&-
printf '%s\n' "Installed $CX_PREFIX/bin/cx. Add this directory to your shell PATH if needed."
# stdin may be a pipe; use the controlling terminal for the application.
if ( : </dev/tty ) 2>/dev/null; then exec "$CX_PREFIX/bin/cx" </dev/tty; fi
