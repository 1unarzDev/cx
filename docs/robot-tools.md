# Robot execution and updates

The deployed robot accounts are `roboboat` on blastoise-odroid (Fedora CoreOS 43 ARM64) and squirtle-jetson (Ubuntu 20.04 ARM64). Their existing network profiles, gateway, addresses and router remain unchanged. Squirtle-jetson is not squirtle-odroid.

CX uses signed Linux ARM64 musl releases, avoiding a dependency on the robots' different glibc versions. Minimal enrollment transfers a viewer-verified helper through the saved SSH route without package installation. Files, ordinary SSH login and network observation work through that helper. Persistent sessions additionally need tmux.

## User-space tmux

Tmux 3.7c was built on the Jetson from the official upstream release, with a private static ncurses 6.5 library. Source downloads used HTTPS with redirects restricted to HTTPS. Recorded source SHA256:

- tmux: `7c60cae9a0e25288e2e24750aafc9e8800fc7fd4555e447e1b29ee4201cfb3bf`
- ncurses: `136d91bc269a9a5785e5f9e980bc76ab57428f604ce3e5a5a90cebc767971cc6`

The build uses GCC 9.4 and the Jetson's existing libevent development library. Ncurses was configured without shared libraries, programs, tests, C++ or Ada; its headers and static library were installed only in `~/.local/state/cx/tool-build/prefix`. Tmux includes both that prefix's `include` and `include/ncursesw` directories and links its static ncurses library (`libncurses.a` aliases `libncursesw.a` inside the private build prefix).

Both hosts passed `tmux -V` and dynamic-library resolution checks before atomic installation to `~/.local/bin/tmux`, under CX's maintenance lock. Binary SHA256: `37764f22859495cd4dd43cb186ac0439abfa510333d6f5c75ebcc5befa7ec356`.

This binary still uses each OS's existing glibc, libutil, libresolv, libm and `libevent_core-2.1.so.7`; it is not a universal static binary or part of CX's signed release archive. These dependencies were checked on both actual devices. No rpm-ostree layering, reboot, global library replacement or shell-profile change is needed. Updating the CX helper preserves this independently installed tmux.

Roboboat user-service lingering is enabled to keep the owned tmux user service alive after the last login ends. This is an account-specific lifetime setting, not a network change. Rollback is `sudo loginctl disable-linger roboboat` after stopping intended persistent work. Remove only the user-space tmux binary when no intended sessions depend on it; do not stop unrelated tmux servers.

## Terminal compatibility

A vendor terminal type absent from the execution OS (reproduced with `xterm-ghostty`) caused immediate attachment failure while leaving the shell alive. Native attachment now preserves a recognized terminal type or selects an installed standard terminal description, preferring xterm-256color. It does not install or replace the OS terminfo database. The fallback can omit vendor-specific extensions while retaining ordinary shell interaction and mouse/color behavior supported by the standard type.

## Automatic helper updates

Accessed hosts are checked in the background by a running CX viewer, coalesced per host, with hourly checks after success and five-minute backoff after failure. When the remote updater cannot download a release, the viewer independently authenticates Info, checks Linux architecture, downloads the exact signed architecture-specific artifact, and transfers it over the existing SSH route. No credentials or agent forwarding are needed. An unattended disconnected robot is not running its own scheduled updater.

Transfer is bounded. Signature verification uses CX's pinned release key before transfer; archive limits and extraction checks are unchanged. The remote installer locks maintenance, refuses an equal/newer disk version and checks the staged executable's reported version before atomic replacement. A fresh authenticated Info must confirm machine/account and expected installed version afterward. Existing terminals/jobs remain independent of helper metadata reconnection.

`cx update --device blastoise-odroid` or `cx update --device squirtle-jetson` explicitly runs the same path. A current-version check does not repeatedly download an archive.

## Access and interface

Verybeautifulserver attempted GSSAPI before its working robot key; this took about19 seconds and exceeded the15-second helper response deadline. Prioritizing public-key authentication reduced the measured SSH connection to about1.2 seconds. CX keeps GSSAPI, hostbased, keyboard-interactive and password methods available after public keys; personal SSH config is preserved. The new background policy uses a separate CX-owned versioned file, keeping the previous file for older viewers.

Network discovery plus Enter on a neighbor is the primary connection path. Ctrl+P retains **Add by SSH address** for uncached targets, aliases and discovery gaps, and asks which gateway to use. Files, Network, Sessions, SSH terminal and session creation in Ctrl+P offer explicit device choices. Network/Sessions also offer All devices. Contextual file/command actions show the active device/folder. `n` creates a new shell in the selected device's home; from Files it uses the current directory.

## Remaining limits

Robot internet sharing and full reverse-access enforcement remain inactive. The CoreOS helper supports files and raster/text previews; PDF previews require optional Poppler converters, which are not installed there. Coding agents likewise require their own installed runtimes. No claim is made that minimal helper enrollment supplies every optional third-party program.

## Portable enrollment from v0.1.37

New supported Linux x86_64/ARM64 devices receive an absent tmux from a separate signed `cx-tmux-vVERSION-linux-ARCH.tar.gz` asset containing exactly one regular `tmux` member. The archive shares the pinned CX release signer; it never executes on the viewer. The native musl build statically links tmux3.7c, ncurses6.5 and libevent2.1.12 with fixed upstream source hashes in `scripts/build-portable-tmux.sh`. Both architectures are built and checked in portable CI; distributions exercise this binary through its own disposable tmux socket.

Enrollment checks architecture before installing, then probes the tool on the execution OS under the helper maintenance lock. A usable existing system/user tmux is retained; broken binaries and unsafe paths fail closed. Absent tools install atomically at `~/.local/bin/tmux`; neither packages, profiles nor live networking change. Helper installation may precede a tool download failure; retry enrollment safely to complete it. A device is saved as ready only after helper and tmux checks succeed. Existing robot tmux described above remains untouched.

The compact password prompt is masked by default. F2 or Show/Hide explicitly exposes/hides its ephemeral value. Passwords are cleared on drop and are not persisted. Showing a value exposes it in the terminal by the user's choice.
