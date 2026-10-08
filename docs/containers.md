# Containers and Dev Containers

CX treats containers as execution scopes beneath their enrolled device. They are not new SSH devices. Open **Ctrl+P → Containers**, choose a device or **All devices**, and expand its tree with Right/l. Names use the Docker container name, with **Devcontainer** or **Docker**, state and access status beside it. Enter opens actions; `n` opens a compact terminal chooser in a running accessible container. Devcontainers are labeled **Devcontainer** in the chooser and **devcontainer** in the session list; ordinary Docker and unclassified containers have distinct labels. The underlying terminal remains a shell.

## Attach and browse

Devcontainers are recognized by modern/legacy Dev Containers labels, or a verified devcontainer configuration at a bind-mounted workspace. A name such as `roboboat_dev` alone does not grant access. Running entries offer Shell, Files and Stop. Container menus do not offer Claude/Codex sessions or agent permissions. Existing sessions remain attachable. No agents, credentials or packages are automatically installed.

Containers use the device's CX-managed tmux, so inner tmux is unnecessary. Native attachment retains terminal colors/default background, Tab completion, clipboard forwarding, paste and scrolling. Ctrl+] returns without stopping work. `d` in Sessions confirms stopping the exact managed shell or agent, including its owned process tree inside the container. An ordinary login shell inside a container loads its existing initialization; CX also loads an existing selected `/opt/ros/$ROS_DISTRO/setup.sh` when `ros2` is otherwise unavailable. It does not select a different ROS distribution.

Files reuse CX's browser and previews, with device → container → folders in the Selected panel. Use `Space`/`v` to select, `y` to copy, `x` to cut and `p` to paste into the displayed folder. `t` chooses a destination device, then its host files or an accessible running container; Enter confirms the destination folder. `T` shows durable progress and retry/cancel. Mkdir, rename and guarded delete run as the selected container user. `n` starts a container terminal in that folder; commands run through that terminal. Equal host/container paths remain separate locations. Markdown relative links stay inside the container; PDF previews require Poppler there. Ctrl+P → Files returns to host files.

Transfers stream bounded chunks through CX helpers without an intermediate full host copy. Partial files are staged at the destination, with checksum-verified completion and resume. Container-to-host, host-to-container and container-to-container copies support trees, symlinks and opaque filenames. Scoped cuts verify the complete destination before source cleanup; changed/skipped destinations retain source files, and tree cleanup removes only empty directories. A failed cleanup may leave a partially moved tree; durable receipts distinguish removed entries on retry. Same-container moves use guarded atomic rename and report cross-filesystem failure without deleting the source. Cancellation retains destination partials for explicit resume. Both remote endpoint helpers need `container-transfers-v1`; update older CX installations first. A stopped/restarted/replaced container must be refreshed and reselected; CX never redirects a stale transfer into its replacement.

## Ordinary Docker services

Ordinary Docker containers are inspection-only. **Enable access** opts in only that exact container on that device/account and engine; it then offers terminals, scoped files/transfers and individual start/stop. Shell-less images can still be inspected and individually managed after opt-in, but terminal/files requests report missing tooling. Disable access revokes future access; it does not terminate existing work. There are no bulk stop/remove/rebuild actions. Ordinary containers have no Rebuild action.

**Start** resumes the existing exact container with Docker, retaining its image, mounts, ports and network configuration. **Stop** ends work inside that container after confirmation. Neither runs a workspace rebuild.

## Rebuild

For a devcontainer with an associated host workspace/configuration, choose **Rebuild** in its actions and confirm, or run `cx devcontainer-rebuild FULL_ID --device ALIAS --yes`. This runs the Node Dev Containers CLI with the selected container's unique workspace labels and `--remove-existing-container`; ambiguous matches are refused. It replaces the writable container layer, ends sessions/transfers using that container, and may run workspace hooks or restart Compose services. Back up container-only files first. Mounts follow the existing workspace configuration; CX does not request volume removal. Cached image layers may be reused.

The container row displays `rebuilding…` until the result arrives, with up to ten minutes allowed. Success and failure refresh discovery; failures are shown as Error notifications without automatic rebuild retries. Invalid configuration is refused before replacement. For failures after replacement starts, inspect the refreshed state before retrying. Old sessions, clipboard entries and transfer scopes are never silently rebound to the new container ID.

## Start a configured workspace

Browse the workspace's host folder in Files, then choose **Ctrl+P → Start devcontainer workspace here**. Review its `.devcontainer/devcontainer.json` or `.devcontainer.json` first. Confirmation runs `devcontainer up` on that device, which can build images, start declared Compose services and run lifecycle hooks. Startup has a ten-minute limit; on timeout, refresh before retrying because already-started containers/hooks may continue.

Workspace startup requires the [Node Dev Containers CLI](https://code.visualstudio.com/docs/devcontainers/devcontainer-cli) installed on the execution device. CX finds it on PATH or in existing nvm Node installations. CLI 0.89.0 (Node >=20) is used in integration fixtures. CX does not globally install Node/npm/CLI. When a CLI and configuration are available, attachment uses `devcontainer exec` to honor environment probing and `remoteEnv`. Without a CLI, existing containers use Docker exec and literal configuration `remoteEnv`; unresolved `${…}` substitutions are omitted. For full configuration semantics, install the CLI. See the [configuration reference](https://containers.dev/implementors/json_reference/).

```sh
cx containers --device laptop
# Use the full ID from discovery in the following commands.
cx container-new FULL_ID --device laptop
cx container-files FULL_ID --device laptop --path /workspace
cx container-start FULL_ID --device laptop
cx container-stop FULL_ID --device laptop
cx container-access FULL_ID --device laptop     # ordinary container opt-in
cx container-access FULL_ID --device laptop --disable
cx devcontainer-up /path/to/workspace --device laptop --yes
cx devcontainer-rebuild FULL_ID --device laptop --yes
cx copy /tmp/data ./data --source-device laptop --source-container FULL_ID
cx copy ./data /workspace --destination-device laptop --destination-container FULL_ID
```

## Platform and network scope

Graphical applications such as RViz2 need a separate display transport; native terminal attachment does not provide one. For the choices between a local ROS viewer and streaming the robot's graphical application, see [RViz2 on tranquility](../research/rviz-remote-display.md). No display forwarding or ROS network changes are activated automatically.

The execution device needs a usable local Docker socket and permission to access it. Docker contexts pointing at remote TCP/SSH daemons are rejected: enroll that host as a CX SSH device instead. Containers need native Linux x86_64 or ARM64 matching the host helper, `/bin/sh`, basic POSIX tools, and writable executable `/tmp`. Read-only/noexec images and different architectures fail with an explanation. Alpine and older glibc images use the static release helper. Docker is the supported engine; Podman and Windows containers are not supported.

Discovery is read-only and never executes in containers or starts them. The helper is copied only when opening files/a session, into a versioned private `/tmp/cx-tools-UID` directory as the selected container user. A future CX version supplies its matching helper without an OS package update. CX performs no automatic port forwarding, host network changes, ROS/GPU reconfiguration or SSH trust changes. Configuration-driven workspace startup uses the network settings declared by that configuration.

## Implementation and verification

Execution/file scopes carry device, Docker-engine fingerprint, full container ID, start timestamp and selected user. Restarted containers or changed engines require refresh. File caches, replies, remembered folders and viewer restoration distinguish host/container scopes. Scoped file requests use the existing bounded CX1 protocol without a TTY. Terminal processes have private PID/start-time ownership records; stopping the Docker client alone would leave the container shell alive, so termination checks exact inner process identities too. Transfer specifications, receive checkpoints and retry paths carry the container scope throughout.

Local fixtures use owned `--network none` containers and private CX/tmux state. They cover evidence-based classification, ordinary opt-in/revocation, read-only image refusal, scoped files/Markdown, engine/restart rejection, exact resume, native paste/Tab/OSC52/RGB/default background/wheel/detach/termios, scoped shell/agent termination, synthetic Codex/Claude permissions, and real Node CLI startup/hooks/remoteEnv. UI fixtures cover 48×24, 80×24 and 120×40 layouts; these are rendered fixtures, not physical terminal screenshots. Actual robot ROS/GPU/mount checks are recorded separately in [validation](../VALIDATION.md).

[Back to README](../README.md)
