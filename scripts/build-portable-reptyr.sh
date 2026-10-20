#!/bin/sh
# Build cx-reptyr: pinned reptyr with the CX ptrace guard, no dynamic libraries.
# CX_REPTYR_DYNAMIC=1 builds with the system compiler for local fixtures only.
set -eu
[ $# = 1 ] || { echo 'usage: build-portable-reptyr.sh OUTPUT' >&2; exit 2; }
case $(uname -sm) in 'Linux x86_64'|'Linux aarch64') ;; *) echo 'native Linux x86_64/ARM64 build required' >&2; exit 1;; esac
output=$1
case "$output" in /*) ;; *) output="$PWD/$output";; esac
here=$(cd "$(dirname "$0")" && pwd)
build=$(mktemp -d)
trap 'rm -rf "$build"' EXIT HUP INT TERM
name=reptyr.tar.gz
expected=c6ffbc34a511ac00d072219bda30699e51f2f4eb483cbae9e32e981d49e8b380
cache_file=
if [ -n "${CX_TOOL_SOURCE_CACHE:-}" ]; then
    cache_file="$CX_TOOL_SOURCE_CACHE/$name"
    if [ -f "$cache_file" ]; then
        cp "$cache_file" "$build/$name"
    fi
fi
if ! printf '%s  %s\n' "$expected" "$build/$name" | sha256sum --check --status 2>/dev/null; then
    curl --disable --fail --silent --show-error --proto '=https' --proto-redir '=https' --location \
        --connect-timeout 10 --max-time 180 --retry 8 --retry-delay 5 --retry-max-time 240 \
        --retry-connrefused --retry-all-errors \
        https://github.com/nelhage/reptyr/archive/refs/tags/reptyr-0.10.0.tar.gz --output "$build/$name"
    printf '%s  %s\n' "$expected" "$build/$name" | sha256sum --check --status
    if [ -n "$cache_file" ]; then
        mkdir -p "$CX_TOOL_SOURCE_CACHE"
        cp "$build/$name" "$cache_file"
    fi
fi
cd "$build"
tar xzf "$name"
cd reptyr-reptyr-0.10.0
patch -p1 --quiet < "$here/reptyr/cx-reptyr.patch"
cp "$here/reptyr/cx_guard.c" "$here/reptyr/cx_guard.h" .
if [ "${CX_REPTYR_DYNAMIC:-}" = 1 ]; then
    make -s CFLAGS=-O2 reptyr
else
    make -s CC=musl-gcc CFLAGS=-O2 LDFLAGS=-static reptyr
    if readelf -l ./reptyr | grep -q INTERP; then echo 'dynamic cx-reptyr refused' >&2; exit 1; fi
fi
[ "$(./reptyr -v)" = 'cx-reptyr 1 (reptyr 0.10.0)' ]
mkdir -p "$(dirname "$output")"
install -m 755 ./reptyr "$output"
sha256sum "$output"
