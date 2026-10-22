# cx

CX is a lightweight terminal workspace for SSH devices, persistent shells, Claude/Codex sessions, files and network discovery. It runs on **Linux x86_64 and ARM64** and works with existing SSH connections, including devices on a mesh network. Coding agents run on the execution device; viewing devices do not need an AI runtime.

## Get started

```sh
curl -fsSL https://raw.githubusercontent.com/1unarzDev/cx/main/install.sh | sh
cx
```

Running `cx` opens Files in the folder you launched it from on that device. Update restarts restore your previous workspace.

Install as your ordinary user. The installer verifies signed releases and offers to install missing dependencies. See [installation and requirements](docs/installation.md) for supported platforms, optional tools and minimal robot setup.

Add a device using an existing SSH alias or account/address:

```sh
cx add user@host
cx add robot@192.168.0.2 --via laptop
```

On the device list, a leading `*` marks directed (viewer-to-device) access. Core devices automatically establish reciprocal access with every other core device when CX starts, after enrollment or promotion, and on background retries. Directed devices such as robots are excluded. Press `b` to change posture, `e` to edit an endpoint, or `x` to remove a device. Reachability reports observed contact; core posture alone does not prove that setup has completed.

Core setup uses an existing authenticated connection to verify machine/account identities and exchange public keys. Each device generates its own dedicated private key; CX never copies private keys or edits personal SSH config/known_hosts. Managed trust is pinned in CX state, and restricted public authorizations are added to `~/.ssh/authorized_keys`. Verified portable endpoints replace stale aliases when necessary. Use `cx device sync` to retry immediately; ordinary CX startup does this automatically. Helpers need v0.1.76 or newer, Linux/OpenSSH, direct port-22 connectivity and a reachable address observed on the authenticated connection. Custom proxy/jump routes stay pending rather than being guessed. Changed pinned identities/keys fail closed. Removal or demotion stops future enrollment; existing SSH authorizations remain and are not automatically revoked. Explicit `cx device pair NAME --return-target user@host` remains available for already configured SSH connections.


You can also select a discovered neighbor in **Network** and press Enter to connect. **Ctrl+P → Add by SSH address** handles aliases and devices discovery cannot reach. Enrollment checks compatibility and installs the helper and missing portable tmux. SSH passwords are masked and not stored.

## Everyday use

| Key | Action |
|---|---|
| Arrows / `h j k l`, Enter, Escape | Navigate, open, go back |
| Ctrl+P | Open actions and choose a device |
| `/`, `?` | Search, help |
| `n` | Choose Claude, Codex, Shell or Devcontainer for the highlighted device |
| Tab / Shift+Tab | Change panel focus |
| Ctrl+] in a managed terminal | Return to CX; the session keeps running |
| `w` in Sessions | Watch the highlighted session read-only |
| Enter on a `· terminal` / `· ssh` row | Move a shell or agent started outside CX into CX ([setup](docs/user-guide.md#moving-terminals-started-outside-cx)) |
| `s` in Sessions | Inspect the selected agent's tasks and subagents |
| `b` on a device | Toggle local posture: core or viewer-to-device |
| `d` in Sessions | Confirm stopping a CX-managed shell or agent |
| Ctrl+C in CX | Exit the viewer; sessions keep running |

Press **n** for a compact chooser. It shows Shell, Devcontainer, and only the Claude or Codex profiles currently available on the selected device; `h`/`l` or Left/Right move between options. It uses home from Devices and the current folder from Files. Use **Ctrl+P → New session** to choose a device and start a supported profile. Agent sessions offer **Fresh session**, **Resume latest session**, or **Resume · choose session**, followed by **Default** permissions or **YOLO** for that launch. Resume latest reopens the most recent session (`codex resume --last` / `claude --continue`); Resume · choose opens the provider’s own session picker (`codex resume` / `claude --resume`). YOLO bypasses the agent’s normal approval protections.

In **Files**, Enter previews a file. To transfer files, select with Space, press `t`, choose the destination device/folder, then press Enter to confirm. Incoming conflicts rename by default; overwrite is explicit. See the [user guide](docs/user-guide.md) for file operations, previews, clipboard, scrolling and all controls.

**Network → All devices** groups discoveries beneath each observer. Selecting one device shows only its interfaces, routes and neighbors. Devices reachable through another host use saved SSH jump routes. Internet sharing and full reverse-access enforcement are still in development; see [robot mesh sharing](docs/robot-mesh-sharing.md).

**Containers** groups Docker containers and devcontainers beneath each device. Start persistent container shells/agents, browse container files, or explicitly start a configured workspace. See the [container guide](docs/containers.md).

CX checks signed updates in the background. You can also run `cx update --check` or `cx update --device NAME`; existing sessions keep running.

## Development

```sh
cargo build --release --locked
cargo test --locked
```

See [development](docs/development.md) for contribution and release notes, [validation](VALIDATION.md) for tested behavior and limitations, and [CHANGELOG](CHANGELOG.md) for release changes.
