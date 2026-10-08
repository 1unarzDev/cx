# Installation and requirements

Install CX as your ordinary user on Linux x86_64 or ARM64. No Rust toolchain is needed for released binaries.

## Install and update safely

```sh
curl -fsSL https://raw.githubusercontent.com/1unarzDev/cx/main/install.sh | sh
cx
cx add user@host                 # existing SSH alias also works
cx add robot@192.168.0.2 --via laptop
```

Install as your ordinary user. Missing dependencies require an explicit package-manager confirmation through your terminal; no Rust toolchain is needed. The installer verifies downloads using the pinned [release public key](../release-key.pem), stages privately and replaces the binary atomically. Re-running is safe; failed/offline downloads preserve an existing installation. After updating with the installer, reopen an already-running viewer to use the new binary; sessions keep running. The script itself is trusted from GitHub over HTTPS. Builds also carry GitHub provenance.

## Dependencies and platform support

**Requirements:** Linux; guarded filesystem mutations need kernel 5.6+ and fail safely when unavailable. The installer offers OpenSSH, tmux, curl, OpenSSL, Poppler (PDF previews), iproute2, ping and ncurses/terminfo utilities through apt, dnf/yum or pacman. Image decoding, Markdown and tar/tar.gz previews are bundled; no Yazi or graphics protocol is required. NetworkManager and AI runtimes are optional and are not installed by cx. Transfers use supported detached processes; a working systemd user manager is needed when launched from a service cgroup. Fish, Bash and Zsh work; terminal colors/backgrounds remain yours. Agents and their credentials stay on the execution host. Enrollment checks platform compatibility and required utilities before replacing the helper, and supplies signed portable tmux when it is absent. The installer can supply the broader set of optional preview and networking utilities on a fresh execution host. Cross-architecture enrollment downloads the matching signed release on the viewer, then transfers it over SSH; the robot need not have internet access. Minimal host utilities must already be available on that execution host; see [robot execution and updates](robot-tools.md) for portable tmux enrollment.

## Minimal robot enrollment

Robot enrollment can use `cx add user@host --via enrolled-device --minimal` on an immutable Linux host; it installs the helper and an absent portable tmux without OS package or profile changes. Directed access review and existing-LAN sharing previews are described in [robot mesh sharing](robot-mesh-sharing.md). Sharing previews do not activate networking or establish reverse SSH enforcement.

## Removal

Full uninstall is not release-ready. Remove only `~/.local/bin/cx` when no active job needs its helper; preserve state and sessions.

[Back to README](../README.md)
