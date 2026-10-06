#!/bin/sh
set -eu
# No public release exists yet. Supply the real repository when published.
CX_REPO=${CX_REPO:-}
CX_VERSION=${CX_VERSION:-}
CX_PREFIX=${CX_PREFIX:-"$HOME/.local"}
fail() { printf '%s\n' "cx install: $*" >&2; exit 1; }
[ "$(id -u)" -ne 0 ] || fail 'run as your ordinary user'
[ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ] || fail 'this release supports Linux x86_64'
[ -n "$CX_REPO" ] && [ -n "$CX_VERSION" ] || fail 'no public release is configured; use a verified local build until publication'
case "$CX_REPO" in *[!a-zA-Z0-9_./-]*|../*|/*) fail 'invalid repository';; esac
case "$CX_VERSION" in v[0-9]* ) ;; *) fail 'version must be a release tag';; esac
command -v curl >/dev/null || fail 'curl is required'
command -v gh >/dev/null || fail 'GitHub CLI is required to verify release provenance against the repository trust root'
command -v ssh >/dev/null || fail 'OpenSSH is required; install using your OS package manager'
CX_TMP=$(mktemp -d)
trap 'rm -rf "$CX_TMP"' EXIT HUP INT TERM
CX_ASSET=cx-linux-x86_64.tar.gz
curl -fsSL "https://github.com/$CX_REPO/releases/download/$CX_VERSION/$CX_ASSET" -o "$CX_TMP/$CX_ASSET"
gh attestation verify "$CX_TMP/$CX_ASSET" --repo "$CX_REPO" --signer-workflow "$CX_REPO/.github/workflows/release.yml" >/dev/null || fail 'release provenance verification failed'
tar -tzf "$CX_TMP/$CX_ASSET" > "$CX_TMP/members"
[ "$(cat "$CX_TMP/members")" = cx ] || fail 'unexpected archive members'
tar -xzf "$CX_TMP/$CX_ASSET" -C "$CX_TMP" cx
mkdir -p "$CX_PREFIX/bin"
install -m 700 "$CX_TMP/cx" "$CX_PREFIX/bin/cx.installing"
"$CX_PREFIX/bin/cx.installing" --version
mv "$CX_PREFIX/bin/cx.installing" "$CX_PREFIX/bin/cx"
printf '%s\n' "Installed $CX_PREFIX/bin/cx. Add this directory to your shell PATH if needed."
# stdin may be a pipe; use the controlling terminal for the application.
if [ -r /dev/tty ]; then exec "$CX_PREFIX/bin/cx" </dev/tty; fi
