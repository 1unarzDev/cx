# cx

CX is a lightweight terminal workspace for SSH devices, persistent shells, Claude/Codex sessions, files and network discovery. It runs on **Linux x86_64 and ARM64** and works with existing SSH connections, including devices on a mesh network. Coding agents run on the execution device; viewing devices do not need an AI runtime.

## Get started

```sh
curl -fsSL https://raw.githubusercontent.com/1unarzDev/cx/main/install.sh | sh
cx
```

Running `cx` opens the hub, the network map around this device; `f` on a device's screen opens its files (this device's Files start in the folder you launched from). Update restarts restore your previous workspace.

Install as your ordinary user. The installer verifies signed releases and offers to install missing dependencies. See [installation and requirements](docs/installation.md) for supported platforms, optional tools and minimal robot setup.

Add a device using an existing SSH alias or account/address:

```sh
cx add user@host
cx add robot@192.168.0.2 --via laptop
```

A device's screen says `single-direction` for directed (viewer-to-device) access, and the hub draws such a link with one arrowhead. Core devices automatically establish reciprocal access with every other core device when CX starts, after enrollment or promotion, and on background retries. Directed devices such as robots are excluded. On a device's screen press `b` to edit the viewer's link to it in the edge editor (key, password, one-way or none; `cx device promote`/`demote` set the core posture), `E` to edit its endpoint, or `X` to remove it. Reachability reports observed contact; core posture alone does not prove that setup has completed.

Core setup uses an existing authenticated connection to verify machine/account identities and exchange public keys. Each device generates its own dedicated private key; CX never copies private keys or edits personal SSH config/known_hosts. Managed trust is pinned in CX state, and restricted public authorizations are added to `~/.ssh/authorized_keys`. Verified portable endpoints replace stale aliases when necessary. Use `cx device sync` to retry immediately; ordinary CX startup does this automatically. Helpers need v0.1.76 or newer, Linux/OpenSSH, direct port-22 connectivity and a reachable address observed on the authenticated connection. Custom proxy/jump routes stay pending rather than being guessed. Changed pinned identities/keys fail closed. Removal or demotion stops future enrollment; existing SSH authorizations remain and are not automatically revoked. Explicit `cx device pair NAME --return-target user@host` remains available for already configured SSH connections.


You can also find a discovered LAN neighbor in the palette (`/`, after a Refresh on the hub observed the network) and press Enter to connect. **`/` → Add by SSH address** handles aliases and devices discovery cannot reach. Enrollment checks compatibility and installs the helper and missing portable tmux. SSH passwords are masked and not stored.

## Everyday use

`cx` opens on the **hub**: the network around this device, drawn as a map (this device `◉` in the middle, its peers beside it, devices reached through a relay further out), the selected device's vitals, a row of live session tiles (what needs you first) and a one-line ticker that pairs each problem with the keys that fix it. Every device has a letter, underlined in its name. One grammar works everywhere: **a device letter, then a session slot or a verb**.

| Key | Action |
|---|---|
| device letter (`p`, `b`, …) | Open that device: its sessions, ready to attach or start |
| `a s d g` on a device | Attach that session slot at once |
| `c` / `x` / `n` on a device | New Claude / Codex / shell in its home, attached at once |
| Enter / Space | Attach (or open) the selection / peek a session's screen |
| Backspace, Backspace | Stop the selected CX-managed session (any other key keeps it) |
| `f` on a device | Its files |
| `e` then a letter | Edit the link to that device: `a p o` / `A P O` set key, password or off per direction, `u` undoes |
| `` ` `` (or Esc Esc) | Back to the hub from any screen; Esc goes back one level |
| `-` | Re-attach the last session you detached from |
| `/` | Find anything: sessions, devices, actions, folders, LAN neighbours |
| `K`, `+`, `?` | Keys and secrets, invite a device, help |
| Tab on the hub | Switch between the network map and the session tiles |
| Ctrl+\ (or tap `⏏ detach`) in a managed terminal | Return to CX; the session keeps running. Ctrl+] reaches the application |
| `w`, `p`, `t` on a device | Watch read-only, pin/unpin, inspect agent tasks |
| Ctrl+C in CX | Exit the viewer; sessions keep running |

Every node, tile, chip and row is also a tap target that does what its key does; a right-click or long press shows the selection's keys as a menu. The palette's **New session** (and `n` in Files) opens the full chooser: Shell, Devcontainer and the Claude or Codex profiles available on the device, existing sessions in that folder by name and observed state, then **Default** or **YOLO** permissions for that launch (YOLO bypasses the agent's normal approval protections). CX does not choose a conversation by recency. Detaching lands on the hub with the tile you left selected, so Enter or `-` re-attaches.

In **Files**, Enter previews a file; from 100 columns the highlighted file previews inline. To send files, select with Space, press `s` and then the destination's device letter (or `t` to choose a device), browse to the folder, then press Enter to confirm. `S` lists the sessions in the selected folder. Incoming conflicts rename by default; overwrite is explicit. See the [user guide](docs/user-guide.md) for file operations, previews, clipboard, scrolling and all controls.

Refresh on the hub (palette **Refresh**) observes each device's network; its LAN neighbours then appear in the palette. Devices reachable through another host use saved SSH jump routes. Internet sharing and full reverse-access enforcement are still in development; see [robot mesh sharing](docs/robot-mesh-sharing.md).

Devices you own pair over SSH with a short code: `+` on the hub (or `cx mesh invite --backend ssh-overlay`) shows it, and the new device runs `cx join <code> <host>`. `cx edge set FROM TO key|password|one-way|none` sets access per link and direction, and `cx secrets` (or `K` on the hub) keeps tokens in the OS keyring or a 0600 file.

`cx mesh` also sets up a private WARP mesh on your own free Cloudflare account: internet over MASQUE, LANs untouched, the mesh reachable. Devices join with one line (`curl -sL <endpoint>/<code>|sh`) and need no cx. See [mesh](docs/mesh.md). `cx claude lend <name>` lends your Claude Code gateway the same way: one line (`curl -sL <endpoint>/claude/<code>|sh`) configures Claude Code; see [claude](docs/claude.md).

**Containers** groups Docker containers and devcontainers beneath each device. Join existing work, start a persistent shell or installed Claude/Codex agent, choose past conversations in the provider’s native picker, browse files, or explicitly start a configured workspace. See the [container guide](docs/containers.md).

CX checks signed updates in the background. You can also run `cx update --check` or `cx update --device NAME`; existing sessions keep running.

## Development

```sh
cargo build --release --locked
cargo test --locked
```

See [development](docs/development.md) for contribution and release notes, [validation](VALIDATION.md) for tested behavior and limitations, and [CHANGELOG](CHANGELOG.md) for release changes.
