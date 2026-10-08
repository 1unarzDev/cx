#!/bin/sh
# Build a native Linux tmux artifact with no dynamic library dependency.
set -eu
[ $# = 1 ] || { echo 'usage: build-portable-tmux.sh OUTPUT' >&2; exit 2; }
case $(uname -sm) in 'Linux x86_64'|'Linux aarch64') ;; *) echo 'native Linux x86_64/ARM64 build required' >&2; exit 1;; esac
output=$1
case "$output" in /*) ;; *) output="$PWD/$output";; esac
build=$(mktemp -d)
trap 'rm -rf "$build"' EXIT HUP INT TERM
fetch() {
    name=$1 url=$2 expected=$3
    if [ -n "${CX_TOOL_SOURCE_CACHE:-}" ] && [ -f "$CX_TOOL_SOURCE_CACHE/$name" ]; then
        cp "$CX_TOOL_SOURCE_CACHE/$name" "$build/$name"
    else
        curl --disable --fail --silent --show-error --proto '=https' --proto-redir '=https' --location --connect-timeout 5 --max-time 90 "$url" --output "$build/$name"
    fi
    printf '%s  %s\n' "$expected" "$build/$name" | sha256sum --check --status
}
fetch tmux.tar.gz https://github.com/tmux/tmux/releases/download/3.7c/tmux-3.7c.tar.gz 7c60cae9a0e25288e2e24750aafc9e8800fc7fd4555e447e1b29ee4201cfb3bf
fetch ncurses.tar.gz https://ftp.gnu.org/gnu/ncurses/ncurses-6.5.tar.gz 136d91bc269a9a5785e5f9e980bc76ab57428f604ce3e5a5a90cebc767971cc6
fetch libevent.tar.gz https://github.com/libevent/libevent/releases/download/release-2.1.12-stable/libevent-2.1.12-stable.tar.gz 92e6de1be9ec176428fd2367677e61ceffc2ee1cb119035037a27d346b0403bb
cd "$build"
for archive in ncurses.tar.gz libevent.tar.gz tmux.tar.gz; do tar xzf "$archive"; done
prefix="$build/prefix"
export CC=musl-gcc
cd "$build/libevent-2.1.12-stable"
./configure --prefix="$prefix" --disable-shared --enable-static --disable-openssl --disable-libevent-regress --disable-samples
make -j2
make install
cd "$build/ncurses-6.5"
./configure --prefix="$prefix" --without-shared --with-normal --without-debug --without-ada --without-cxx --without-cxx-binding --without-progs --without-tests --disable-db-install --with-default-terminfo-dir=/usr/share/terminfo --with-terminfo-dirs=/etc/terminfo:/lib/terminfo:/usr/share/terminfo
make -j2
make install.libs install.includes
ln -s libncursesw.a "$prefix/lib/libncurses.a"
cd "$build/tmux-3.7c"
export PKG_CONFIG_LIBDIR="$prefix/lib/pkgconfig"
CFLAGS="-O2 -I$prefix/include -I$prefix/include/ncursesw" LDFLAGS="-static -L$prefix/lib" ./configure
make -j2
./tmux -V
if readelf -l ./tmux | grep -q INTERP; then echo 'dynamic tmux refused' >&2; exit 1; fi
mkdir -p "$(dirname "$output")"
install -m 755 ./tmux "$output"
sha256sum "$output"
