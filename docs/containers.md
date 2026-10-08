# Containers and Dev Containers

CX treats containers as execution scopes beneath their enrolled device. They are not new SSH devices. Open **Ctrl+P → Containers**, choose a device or **All devices**, and expand its tree with Right/l. Names use the Docker container name, with **Devcontainer** or **Docker**, state and access status beside it. Enter opens actions; `n` opens a compact Shell/Codex/Claude chooser in a running accessible container. Sessions retain the container name and provider.

## Attach and browse

Devcontainers are recognized by modern/legacy Dev Containers labels, or a verified devcontainer configuration at a bind-mounted workspace. A name such as `roboboat_dev` alone does not grant access. Running entries offer Shell, Agent session, Files and Stop. Codex/Claude must be installed inside the container; Default/YOLO permissions apply to that launch only. No agents, credentials or packages are automatically installed.

Containers use the device's CX-managed tmux, so inner tmux is unnecessary. Native attachment retains terminal colors/default background, Tab completion, clipboard forwarding, paste and scrolling. Ctrl+] returns without stopping work. `d` in Sessions confirms stopping the exact managed shell or agent, including its owned process tree inside the container. An ordinary login shell inside a container loads its existing initialization; CX also loads an existing selected `/opt/ros/$ROS_DISTRO/setup.sh` when `ros2` is otherwise unavailable. It does not select a different ROS distribution.

Files reuse CX's browser and previews, with device → container → folders in the Selected panel. Container files are **read-only** in this release. `n` there chooses a container session in the displayed folder for editing. Host copy/cut/transfer/delete/rename/command actions are disabled: equal host/container paths are different locations. Markdown relative links stay in the container. PDF previews require Poppler inside the container; previewing files does not install it. Choose Ctrl+P → Files to return to host files.

## Ordinary Docker services

Ordinary Docker containers are inspection-only. **Enable access** opts in only that exact container on that device/account and engine; it then offers terminals, read-only files and individual start/stop. Shell-less images can still be inspected and individually managed after opt-in, but terminal/files requests report missing tooling. Disable access revokes future access; it does not terminate existing work. There are no bulk stop/remove/rebuild actions.

**Start** resumes the existing exact container with Docker, retaining its image, mounts, ports and network configuration. **Stop** ends work inside that container after confirmation. Neither runs a workspace rebuild.

## Start a configured workspace

Browse the workspace's host folder in Files, then choose **Ctrl+P → Start devcontainer workspace here**. Review its `.devcontainer/devcontainer.json` or `.devcontainer.json` first. Confirmation runs `devcontainer up` on that device, which can build images, start declared Compose services and run lifecycle hooks. Startup has a ten-minute limit; on timeout, refresh before retrying because already-started containers/hooks may continue.

Workspace startup requires the [Node Dev Containers CLI](https://code.visualstudio.com/docs/devcontainers/devcontainer-cli) installed on the execution device. CX finds it on PATH or in existing nvm Node installations. CLI 0.89.0 (Node >=20) is used in integration fixtures. CX does not globally install Node/npm/CLI. When a CLI and configuration are available, attachment uses `devcontainer exec` to honor environment probing and `remoteEnv`. Without a CLI, existing containers use Docker exec and literal configuration `remoteEnv`; unresolved `${…}` substitutions are omitted. For full configuration semantics, install the CLI. See the [configuration reference](https://containers.dev/implementors/json_reference/).

```sh
cx containers --device laptop
# Use the full ID from discovery in the following commands.
cx container-new FULL_ID --device laptop
cx container-new FULL_ID --device laptop --provider codex --yolo
cx container-files FULL_ID --device laptop --path /workspace
cx container-start FULL_ID --device laptop
cx container-stop FULL_ID --device laptop
cx container-access FULL_ID --device laptop     # ordinary container opt-in
cx container-access FULL_ID --device laptop --disable
cx devcontainer-up /path/to/workspace --device laptop --yes
```

## Platform and network scope

The execution device needs a usable local Docker socket and permission to access it. Docker contexts pointing at remote TCP/SSH daemons are rejected: enroll that host as a CX SSH device instead. Containers need native Linux x86_64 or ARM64 matching the host helper, `/bin/sh`, basic POSIX tools, and writable executable `/tmp`. Read-only/noexec images and different architectures fail with an explanation. Alpine and older glibc images use the static release helper. Docker is the supported engine; Podman and Windows containers are not supported.

Discovery is read-only and never executes in containers or starts them. The helper is copied only when opening files/a session, into a versioned private `/tmp/cx-tools-UID` directory as the selected container user. A future CX version supplies its matching helper without an OS package update. CX performs no automatic port forwarding, host network changes, ROS/GPU reconfiguration or SSH trust changes. Configuration-driven workspace startup uses the network settings declared by that configuration.

## Implementation and verification

Execution/file scopes carry device, Docker-engine fingerprint, full container ID, start timestamp and selected user. Restarted containers or changed engines require refresh. File caches, replies, remembered folders and viewer restoration distinguish host/container scopes. Read-only helper requests use the existing bounded CX1 protocol without a TTY. Terminal processes have private PID/start-time ownership records; stopping the Docker client alone would leave the container shell alive, so termination checks exact inner process identities too. Transfers remain host-only until the endpoint/chunk protocol can carry container scope throughout.

Local fixtures use owned `--network none` containers and private CX/tmux state. They cover evidence-based classification, ordinary opt-in/revocation, read-only refusal, scoped files/Markdown, engine/restart rejection, exact resume, native paste/Tab/OSC52/RGB/default background/wheel/detach/termios, scoped shell/agent termination, synthetic Codex/Claude permissions, and real Node CLI startup/hooks/remoteEnv. UI fixtures cover 48×24, 80×24 and 120×40 layouts; these are rendered fixtures, not physical terminal screenshots. Actual robot ROS/GPU/mount checks are recorded separately in [validation](../VALIDATION.md).

[Back to README](../README.md)
