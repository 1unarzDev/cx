# CX status

The v0.1.44 floating typed notifications and lowercase devcontainer labels have passed local checks in `worktrees/devcontainer-label-case`, branch `fix/devcontainer-label-case`. Signed publication is pending; v0.1.43 is published.

## Current behavior

- Native persistent sessions support attach, single Ctrl+] detach, read-only watch and guarded shell/agent stop. Host agent launches offer Default/YOLO; container menus are shell-only.
- Devices, Sessions, Files, Network, Containers and Transfers share counted navigation. Typed search/filter/password/command inputs retain ownership of text.
- Device/network observations and saved SSH jump routes work. Containers remain execution scopes beneath enrolled devices. Ordinary Docker access requires explicit opt-in; devcontainers require discovery evidence. Container files are read-only and host transfers cannot use their paths.
- Signed releases support Linux x86_64/ARM64 and old-glibc hosts. Containers receive a matching private helper on access; Node Dev Containers CLI supports configuration-driven startup/exec, with Docker fallback for existing containers.

## Remaining integration and acceptance

- Publish/update v0.1.44 after its release checks. This work performs no remote host deployment; computers were powered down at the user's request.
- Validate the signed container workflow on actual CoreOS Blastoise and Ubuntu20.04 Jetson: ROS help/import, terminal input/detach and preservation of mounts/network/runtime/start times. Local fixtures do not establish ROS messaging, actuator or GPU compute acceptance.
- Jetson configuration-driven workspace startup/full environment substitutions require the Node Dev Containers CLI. Fresh-hardware enrollment and physical UI/input acceptance remain the user's planned checks.
- Internet sharing, full reverse SSH/trust/revocation enforcement and full uninstall remain incomplete. No live networking, SSH trust or provider configuration is changed by this UI work. Container transfers, Podman, remote Docker contexts and cross-architecture emulation remain unsupported.

Actual tests and release evidence are in [VALIDATION.md](VALIDATION.md). Historical milestones are preserved in [the status archive](docs/archive/status-2026-10-08.md). The short module map is in [development](docs/development.md).
