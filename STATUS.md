# CX status

## Device editing and robot silence (v0.1.53 candidate)

The Devices panel `e` menu edits an enrolled SSH target without changing identity or posture and routes removal through confirmation. `creation` now uses its mesh SSH alias instead of the timed-out public address. Devices explicitly marked down, including Squirtle, suppress all connection-failure notifications until brought up with `u`.

## Device state marker and posture controls (v0.1.52 candidate)

The down marker is the empty circle (`○`, `o` in ASCII), while bidirectional access uses a spaced `←→` marker. `b` toggles remote bidirectional/viewer-only access; `d` and `u` control background availability. Passive connection failures remain quiet and are represented by device state; the local device is guarded from all posture changes.

## Device marker and footer polish (v0.1.51 candidate)

Sidebar posture markers now use spaced long arrows (`⟷` / `⟶`, ASCII fallbacks `<-->` / `-->`). The footer no longer displays an unavailable error count; errors use the existing timed notification panel instead.

## Device lifecycle and availability (v0.1.50 candidate)

Remote devices now support confirmed removal from the local registry, explicit up/down availability hints, muted down markers, and background probe suppression for devices marked down. Enrollment records bidirectional access by default; local posture can still be demoted. The requested `verybeautifulserver` enrollment was removed from this viewer's state; no remote host configuration was changed.

## Native agent resume (v0.1.49 candidate)

Implemented and locally verified in `worktrees/access-deploy`: Codex and Claude launch dialogs distinguish fresh sessions from native resume. Codex uses `codex resume --last`; Claude uses `claude --continue`; YOLO variants retain their provider-specific bypass flags. Existing CX session attach remains a separate choice. Full release and signed fleet deployment follow the locked test gate below.

The v0.1.48 device access posture UI passed the full locked Rust suite and policy CLI checks in `worktrees/access-deploy`, branch `feat/access-deploy`. Signed publication is pending.

## Current behavior

- Native persistent sessions support attach, single Ctrl+] detach, read-only watch and guarded shell/agent stop. Host agent launches offer Default/YOLO; container menus are shell-only.
- Devices, Sessions, Files, Network, Containers and Transfers share counted navigation. Typed search/filter/password/command inputs retain ownership of text.
- Device/network observations and saved SSH jump routes work. Containers remain execution scopes beneath enrolled devices. Ordinary Docker access requires explicit opt-in; devcontainers require discovery evidence. Scoped files and resumable transfers support host/container locations; devcontainer rebuilds require an explicit confirmation and a unique workspace match.
- Signed releases support Linux x86_64/ARM64 and old-glibc hosts. Containers receive a matching private helper on access; Node Dev Containers CLI supports configuration-driven startup/exec, with Docker fallback for existing containers.

## Remaining integration and acceptance

- Publish/update v0.1.48 after its release checks. This work performs no remote host deployment; computers were powered down at the user's request.
- Validate the signed container workflow on actual CoreOS Blastoise and Ubuntu20.04 Jetson: ROS help/import, terminal input/detach and preservation of mounts/network/runtime/start times. Local fixtures do not establish ROS messaging, actuator or GPU compute acceptance.
- Jetson configuration-driven workspace startup/full environment substitutions require the Node Dev Containers CLI. Fresh-hardware enrollment and physical UI/input acceptance remain the user's planned checks.
- Internet sharing, full reverse SSH/trust/revocation enforcement and full uninstall remain incomplete. No live networking, SSH trust or provider configuration is changed by this UI work. Podman, remote Docker contexts and cross-architecture emulation remain unsupported.

Actual tests and release evidence are in [VALIDATION.md](VALIDATION.md). Historical milestones are preserved in [the status archive](docs/archive/status-2026-10-08.md). The short module map is in [development](docs/development.md).
