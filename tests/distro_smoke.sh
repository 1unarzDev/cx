#!/bin/sh
# Disposable containers only; no privileged mode, host services, or host mounts writable.
set -eu
CX_BINARY=$(realpath "${1:?provide a freshly built static cx binary}")
CX_SOURCE=$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)
CX_ARCH=$(uname -m)
for CX_IMAGE in ubuntu:22.04 ubuntu:24.04 rockylinux:8 rockylinux:9 archlinux:latest; do
    # The official Arch image is x86_64 only; do not pretend ARM was tested.
    if [ "$CX_ARCH" != x86_64 ] && [ "$CX_IMAGE" = archlinux:latest ]; then
        printf '%s\n' 'Arch aarch64: unavailable official image; skipped'
        continue
    fi
    printf 'Testing %s (%s)\n' "$CX_IMAGE" "$CX_ARCH"
    docker run --rm --security-opt=no-new-privileges \
        --mount "type=bind,src=$CX_SOURCE,dst=/repo,readonly" \
        --mount "type=bind,src=$CX_BINARY,dst=/binary/cx,readonly" \
        "$CX_IMAGE" sh -eu -c '
        if command -v apt-get >/dev/null; then
            apt-get update -qq
            DEBIAN_FRONTEND=noninteractive apt-get install -y -qq python3 openssl curl openssh-client tmux util-linux passwd
        elif command -v dnf >/dev/null; then
            dnf install -y python3 openssl curl openssh-clients tmux util-linux shadow-utils
        else
            pacman -Syu --noconfirm --needed python openssl curl openssh tmux util-linux shadow
        fi
        useradd -m -u 1000 cx-test
        # Root installer refusal is tested separately from ordinary-user installation.
        if CX_NO_LAUNCH=1 sh /repo/install.sh >/dev/null 2>&1; then
            echo "root installer unexpectedly accepted" >&2; exit 1
        fi
        exec runuser -u cx-test -- env HOME=/home/cx-test python3 /repo/tests/distro_install_fixture.py /binary/cx
        '
done
