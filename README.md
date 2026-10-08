# cx

CX is a lightweight terminal workspace for SSH devices, persistent shells, Claude/Codex sessions, files and network discovery. It runs on **Linux x86_64 and ARM64** and works with existing SSH connections, including devices on a mesh network. Coding agents run on the execution device; viewing devices do not need an AI runtime.

## Get started

```sh
curl -fsSL https://raw.githubusercontent.com/1unarzDev/cx/main/install.sh | sh
cx
```

Install as your ordinary user. The installer verifies signed releases and offers to install missing dependencies. See [installation and requirements](docs/installation.md) for supported platforms, optional tools and minimal robot setup.

Add a device using an existing SSH alias or account/address:

```sh
cx add user@host
cx add robot@192.168.0.2 --via laptop
```

You can also select a discovered neighbor in **Network** and press Enter to connect. **Ctrl+P → Add by SSH address** handles aliases and devices discovery cannot reach. Enrollment checks compatibility and installs the helper and missing portable tmux. SSH passwords are masked and not stored.

## Everyday use

| Key | Action |
|---|---|
| Arrows / `h j k l`, Enter, Escape | Navigate, open, go back |
| Ctrl+P | Open actions and choose a device |
| `/`, `?` | Search, help |
| `n` | Start a shell in the device’s home, or the current folder in Files |
| Tab / Shift+Tab | Change panel focus |
| Ctrl+] in a managed terminal | Return to CX; the session keeps running |
| `d` in Sessions | Confirm stopping a CX-managed shell or agent |
| Ctrl+C in CX | Exit the viewer; sessions keep running |

Use **Ctrl+P → New session** to choose a device and start a shell, Codex or Claude. Agents must be installed on that device. Agent sessions offer **Default** permissions or **YOLO** for that launch; YOLO bypasses the agent’s normal approval protections.

In **Files**, Enter previews a file. To transfer files, select with Space, press `t`, choose the destination device/folder, then press `p`. Incoming conflicts rename by default; overwrite is explicit. See the [user guide](docs/user-guide.md) for file operations, previews, clipboard, scrolling and all controls.

**Network → All devices** groups discoveries beneath each observer. Selecting one device shows only its interfaces, routes and neighbors. Devices reachable through another host use saved SSH jump routes. Internet sharing and full reverse-access enforcement are still in development; see [robot mesh sharing](docs/robot-mesh-sharing.md).

CX checks signed updates in the background. You can also run `cx update --check` or `cx update --device NAME`; existing sessions keep running.

## Development

```sh
cargo build --release --locked
cargo test --locked
```

See [development](docs/development.md) for contribution and release notes, [validation](VALIDATION.md) for tested behavior and limitations, and [CHANGELOG](CHANGELOG.md) for release changes.
