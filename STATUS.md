# CX status

## Container agent launch/history parity — 2026-10-24

Implemented on `feat/container-agent-parity-20261024` in `worktrees/container-agent-parity-20261024`, based on switcher commit `970f9f8`. Container session choices now expose installed Claude/Codex profiles scoped to the exact device/container/user/folder. Existing runtimes, fresh work and the provider's native history picker remain separate choices. Default/YOLO intent survives both launch hops; container history uses `codex resume` / `claude --resume`, never latest. Provider checks run no agent and install nothing. Login-startup directory changes cannot redirect agent launches from the selected folder.

Version bumped to 0.1.60 so container-private helpers receive the matching decoder. New `container-agents-v1` capability and versioned agent-create operation prevent old helpers from silently dropping history intent. Runtime tests verify actual arguments/cwd with synthetic agents in a disposable network-none container. [Validation](VALIDATION.md) records evidence and limits.

Integration/acceptance remaining:

- Changes are isolated and committed; no installed binary replacement, publication or remote deployment. The root checkout's pre-existing conflicts remain untouched. Update execution helpers when integrating.
- Real provider authentication/history selection, physical terminal acceptance, actual Node Dev Containers CLI environment behavior and remote robot acceptance remain separate from this synthetic Docker validation.
- Custom/inner-container tmux and nested-client switching remain future work. Exact provider conversation IDs are still selected by the provider picker; CX does not merge separate conversations by folder/title.


## Unified session workspace — 2026-10-23

Implemented in `worktrees/session-workspace-20261023`, branch `feat/session-workspace-20261023`: named runtime selection, all sessions with pins first, persistent unavailable-work bookmarks, folder agent presence, cached navigation, obsolete-read coalescing, alias deduplication, dead-pane lifecycle handling and checked native attachment. Pins reopen saved folders deliberately; no guessed-idle hiding or automatic agent restart. The previous latest/fresh/past launch prompt is removed.

Full Rust suite, isolated runtime lifecycle cases, managed/external attach/watch PTYs delayed-host browser navigation and actual pin/cache viewer-restart checks passed. [VALIDATION.md](VALIDATION.md) records exact evidence and limits. [Feature priorities](docs/astra-improvement-proposals.md) includes the dedicated QoL ideation.

Current unresolved integration/acceptance:

- The original root checkout is detached with pre-existing conflicts. This isolated branch is committed separately; no release, installed binary replacement or remote deployment is included. Updated endpoint helpers provide stable terminal generations; legacy observations retain legacy attachment behavior.
- Container agent launch and resume parity is implemented in the next milestone above. Custom sockets, inner-container tmux, nested-client handoff and exact provider conversation history remain gaps. No global terminal hook was installed.
- Physical emulator/provider/robot acceptance remains separate from synthetic providers, fake SSH/Docker scope checks and disposable tmux PTYs. Tests cover an explicit matrix, not every possible configuration.


## Session workflow exploration 2026 10 23

The user prioritizes agentic coding across machines and native-feeling management of all shells/agents, including sessions created outside CX. Three Astra specialists reviewed committed snapshot `c60e1a2`; findings and ranked features are in [session management and feature priorities](docs/astra-improvement-proposals.md). This isolated branch contains proposals only. `cargo check --locked --offline` and all 175 selected UI tests passed; no live sessions, shell configuration or networking were changed.

Before implementation: resolve the root checkout's existing integration conflicts; define attachment endpoint versus lifecycle ownership; demonstrate nested-client switching and physical terminal behavior before offering optional tmux-backed terminal integration. No further workflow-priority answer is needed.

## Device markers and session choices (v0.1.56)

Only single-direction devices have an access marker: a leading `*`, kept visible before long names are truncated. The edit menu includes the access toggle. New-session buttons, movement and shortcuts use the selected device’s fresh capability evidence; unknown or unavailable agents are hidden.

## Device editing and robot silence (v0.1.53 candidate)

The Devices panel `e` menu edits an enrolled SSH target without changing identity or posture and routes removal through confirmation. `creation` now uses its mesh SSH alias instead of the timed-out public address. Devices explicitly marked down, including Squirtle, suppress all connection-failure notifications until brought up with `u`.

## Device state marker and posture controls (v0.1.52 candidate)

The down marker is the empty circle (`○`, `o` in ASCII), while access uses compact `↔` / `→` markers. `b` toggles remote bidirectional/viewer-only access; `d` and `u` control background availability. Passive connection failures remain quiet and are represented by device state; the local device is guarded from all posture changes.

## Device marker and footer polish (v0.1.51 candidate)

The footer no longer displays an unavailable error count; errors use the existing timed notification panel instead.

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
