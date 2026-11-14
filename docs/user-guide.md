# CX user guide

Detailed controls and behavior for sessions, files, terminals, network discovery and updates. For installation and dependencies, see [installation](installation.md).

Plain `cx` opens the **hub**, the network around this device; this device's Files open in the folder `cx` was started from. An update restart restores the existing workspace instead. The design is described in [concept-mission](design/concept-mission.md).

## Controls

One grammar on every screen: **a device letter, then a session slot or a verb**. Letters are underlined in device names on the map and printed on every tile and palette row; they stay the same while a device stays enrolled (`e` is never a device, and `g h j k l` are used only past 20 devices, so `gg` is the first row on the hub).

| Keys | Action |
|---|---|
| device letter | Open that device's screen (from the hub, peek or palette) |
| `a s d g` (device screen) | Attach that session slot at once; further sessions are a move or `/` away |
| `c` / `x` / `n` (device screen) | New Claude / Codex / shell in the device's home, attached at once |
| Enter, Space | Attach / open the selection; peek a session's screen (Tab steps through peeks) |
| Backspace, Backspace | Stop the selected CX-managed session; any other key keeps it, never `y` |
| `e` + letter | The link to that device: `a p o` (outbound) / `A P O` (the way back) set key, password or off at once, `j k` move the lane bar, `u` undoes |
| `r` (device screen) | Retry / refresh the device |
| `f` (device screen) | Its files |
| `H` (Shift+H), Esc Esc | The hub, from any screen, Files included (a second Esc within 400 ms). v0.2.1's `` ` `` still works in v0.2.2 and goes away after it |
| Esc | Back one level |
| `-` | Re-attach the last detached session |
| `/`, Ctrl+P | Find anything: sessions, devices, actions, folders, LAN neighbours; `run:` runs a command |
| `:` | Run a command in the current folder |
| `K`, `+`, `?` | Keys and secrets, invite, help |
| Tab / Shift+Tab | Hub: network map or session tiles; Files: a transfer's two locations |
| `j k`, arrows, `5j`, Home / End, PageUp / PageDown | Move (counts work); Left/Right move between tiles and nodes |
| Ctrl+\ (or the `⏏ detach` chip) inside managed terminals | Detach; session keeps running |
| Ctrl+C in cx | Exit viewer; work keeps running |
| Files: `.`, Space, `v`, `f`, `/` | Hidden files, select/advance, range, filter, find |
| Files: `y`, `x`, `p` | Copy, cut, paste into the folder (`'` + a device letter first: a transfer); yazi's keymap, `~` lists it |
| Files: `!`, `e`, Alt+← / Alt+→ | New session in the folder, send to a device, yazi's history back / forward (`H` is the hub) |
| Files: `s` + letter, `t`, `T` | Send to a device, choose a destination device/folder, transfer results |
| Files: `r`, `d`, `M`, `o`, `S`, `n` | Rename, delete (confirm), new folder, conflict policy, sessions here, new session here |
| Device screen: `w`, `p`, `t`, `b`, `E`, `X` | Watch, pin, agent tasks, edit the viewer's link (edge policy), edit endpoint, remove |
| Invite (`+`): Esc, `R` | Close the invite, or replace it with a new code |
| Keys (`K`): `a s d g`, `h` | Set or rotate that secret, or add a new one by name |

`+` opens a single-use ssh-overlay invite for 30 minutes (`cx invite` from a shell): the code, the `cx join <code> <host>` line to run on the new device, the other addresses it listens on and a QR code of that line. `y` copies the code and `Y` the line (or tap them), through the terminal's clipboard escape. It listens on this host's mesh and LAN addresses until one device joins, it expires, or you press Esc; leaving the screen closes it. On the keys screen a slot key opens a masked entry: type or paste the value (it is shown only as dots and a count) and Enter stores it through `cx secrets` (keyring, else the 0600 file); Esc discards it.

Every node, tile, chip, file row and palette row is a tap target running its key's command. A right-click or long press on a target selects it and opens a menu of the selection's keys. Hover never selects. On phones (under 70 columns) the hub is an indented device tree and footers wrap; keys read `ret` and `bksp`.

Counts work with Up/Down, j/k and PageUp/PageDown in lists, dialogs and transfers; in transfer details, Tab changes focus so the same keys scroll details. File search and preview matches support counted n/N. Search, filter, command, password and palette inputs own ordinary typing.

## Sessions and permissions

A device's screen keeps all observed shells and agents visible, with pins first. Press `p` to pin/unpin the selected session. A pin promotes its existing tile; it does not create a duplicate. Pins persist after the terminal and viewer close. An unavailable pin opens a choice to browse its saved folder or refresh its device; it never restarts a terminal automatically. `not seen` means absent from the last successful inventory, while `? cached`/`? stale` means current state is unconfirmed.

Opening a provider in a folder with existing sessions shows every matching runtime by name, provider, short identity and observed state. Choose the intended session or explicit New. New agents proceed directly to permissions, without an unidentified “latest” prompt. Distinct conversations in the same folder remain separate; CX does not infer conversation identity from a title or folder.

Cached session/folder navigation is saved locally and displayed before refresh. Failed refreshes retain rows. Saved file identities are stripped and refreshed before mutation. In Files, `!` offers the sessions already in the folder beside New. Session tiles show each session's last screen from memory (never written to disk) in its own colours, so two agents look different because their screens do.

On a device's screen `c`, `x` and `n` start Claude, Codex or a shell in its home and attach at once. The full chooser (`!` in Files, palette **New session**) uses the current folder from Files or the device's home. The same `c`, `x` and `n` pick Claude, Codex or Shell there (`!` `n` in Files is a new shell), and `d` picks Devcontainer; h/l and Left/Right move between choices. Missing runtimes stay disabled, and selecting an agent proceeds to Default/YOLO permissions. The palette's Files and New session actions ask for a device; New session then offers provider/folder setup. File and command actions show their current device/folder scope. New sessions use the chosen execution host and folder. Agent choices reflect installed launchable runtimes. `w` on a device's selected session opens a read-only terminal; ordinary opening gives input. Starting Claude/Codex manually inside a shell updates its badge while the agent owns the terminal; returning to the shell restores it. The original shell remains attachable after its agent exits. When a standalone agent process exits, its dead single-pane runtime is omitted; a session with other panes remains visible. Closing a viewer detaches without ending work. CX follows process and pane evidence rather than treating Ctrl+C as proof of exit. In managed terminals, the wheel reviews retained scrollback unless the application requests native mouse input. Fullscreen Codex with a stale mouse policy receives genuine wheel events through a verified foreground-process fallback; empty terminal history does not open a `0/0` buffer (the Codex fallback requires tmux 3.4+); Escape returns to the live terminal. Up/Down remain application keys. Hold Shift for terminal-native text selection when your emulator supports it. Existing personal tmux sessions retain their bindings.

In a container, `n` probes installed agents for its selected user and folder. Choose Claude/Codex to join an existing runtime, start New, or choose a past conversation in the provider’s own picker. Both new and history launches offer Default/YOLO. Sign-in and history stay inside that container; host availability does not enable container agents. See [containers](containers.md).

Press `:` to run a command in the current folder. It opens in a floating panel instead of leaving CX: output streams live with color, typing goes to the command (including Ctrl+C), Esc closes the panel while Ctrl+O opens it full screen, and the exit status shows when it finishes. CX is never suspended.

New Codex/Claude sessions offer **Default** (existing host permissions) or **YOLO** before launch. YOLO applies only to that session: Codex bypasses approvals and sandboxing; Claude bypasses permission prompts. CLI: `cx new --provider codex --directory ~/cx --yolo` (also supports `--provider claude`). Existing sessions keep their launch permissions. Update remote helpers before selecting YOLO.

Backspace stops the selected CX-managed shell, Claude or Codex session after an inline question on its tile: a second Backspace stops it, any other key (Enter and `y` included) keeps it. The runtime identity captured when it asked is the one stopped. External tmux sessions are protected. Expanded sessions must be stopped from their terminal.

### Moving terminals started outside CX

A device's screen also shows your shells and Claude/Codex sessions that CX did not start. These include sessions in an ordinary terminal window, an SSH login, another tmux or screen server, a default-server tmux pane, or a `docker exec` client. Rows outside tmux end with where they live, such as `· terminal` or `· ssh`.

Press Enter on such a session to move it into CX. The terminal's processes keep running: the same PIDs, threads, child tools and in-progress work, with only where the terminal is shown changing. A move takes well under a second, and output continues during it. If the terminal was a single-window emulator, its now-empty window closes automatically; a multiplexer (tmux, screen) or a server emulator that drives other windows (gnome-terminal, konsole) is left alone, and you can close that window yourself. The moved row then appears once, as a managed CX session. Stopping it with Backspace, Backspace on its tile, exiting its shell, or `cx reset` ends it as it would end any CX session. Programs that ignore SIGTERM are not force-killed.

Moving uses a guarded copy of [reptyr](https://github.com/nelhage/reptyr). Install it once per device:

```sh
cx device ptrace local install    # or a device name; no admin rights needed
cx device ptrace local status
cx device ptrace local uninstall
```

`install` places `cx-reptyr` in `~/.local/bin`. Each move then runs it through `sudo`, and CX asks for your password in a panel, so no privileged state persists between moves. The relay accepts only one exact form: it refuses any process not fully owned by you, re-checks every process identity CX observed, and drops back to your user (and all capabilities) once the terminal has moved.

To skip the per-move password on a device you trust, `cx device ptrace <device> enable` installs a root-owned `/usr/local/libexec/cx/cx-reptyr` with `cap_sys_ptrace` (one `sudo` at setup; undo with `disable`). Any program running as you on that device could then use it to take over your other terminals, so enable it only where you want that. Hosts with Yama `ptrace_scope=3` cannot move terminals. Without setup, these rows stay visible and default-server tmux rows still attach as before.

Automatically named shells show the running executable, such as `running cargo`; new managed Fish shells remember short commands as `last: git`, while older/Bash/Zsh shells show the most recently observed command as `recent: git`. Arguments and shell history are excluded. Claude native names and Codex names use exact process metadata when available; daemon-backed Codex sessions without a reliable identity retain their folder label. Custom names stay unchanged and label refreshes preserve row order.

New terminals prepare in the background while CX shows an animated loader. CX opens the terminal once its first screen has content; intentionally silent programs open after a bounded five-second wait. Startup failures return to the panel. Remote devices need an updated helper for this readiness check.

Session rows refresh while Files and other panels are open. Failed device checks discard unattachable cached rows; successful checks recover existing managed/default tmux terminals, including recognized Docker exec terminals in running devcontainers. Stopped, restarted or missing container runtimes disappear from the device's screen after authoritative checks. The Containers inventory retains actual stopped containers for an explicit Start; cache cleanup never deletes Docker objects or reattaches arbitrary orphaned PTYs.

The Sessions state column animates for observed foreground commands, known native agent work, or recent terminal output. `output` means the terminal emitted output; it does not prove a model is reasoning or a silent command is running. Stale observations expire. On a device's screen, `t` inspects the selected session's provider-reported tasks/subagents and task text. Open/closed child records are task states, not busy indicators. The dialog follows refreshed metadata and closes when the session disappears.

Exact native names and project directories follow provider metadata without changing terminal identity. Custom CX names stay unchanged. New host Codex launches use a separate process (`--no-daemon`) when supported, improving current-thread tracking at the cost of additional memory; older versions fall back. Existing sessions are not restarted. New supported host launches also set Codex’s native thread-name terminal title per launch to identify idle thread switches. This requires explicit, unique names of at most 48 characters among owned threads; otherwise CX shows “Directory unavailable” and avoids using the launch folder for contextual commands. Saved provider configuration is unchanged. Codex database metadata additionally needs the optional `sqlite3` executable; unsupported identities omit optional metadata. No transcripts are read.

Claude attachments use a solid light or dark background for default terminal cells, following the host Claude theme. This improves small question-description contrast in transparent terminals while preserving provider text colors and explicit highlights. Reattaching as a shell restores the terminal background. External tmux sessions and terminal/provider configuration are unchanged.

## Files and transfers

The crumb names the device and folder; under the list, when rows are left, the selected entry's path, size, age and the devices `s` can send it to (with their route) are listed. Switching a transfer's two locations (Tab) switches the crumb.

Select files → `s` and the destination's letter (or `t` → choose device) → browse folder with h/l → Enter confirms transfer into the displayed destination folder. A running transfer shows on the hub (its link lit, name and percentage in the top bar); `T` opens detailed progress. `p` also pastes into the focused folder. Or copy/cut, switch devices, and paste. Incoming name conflicts rename by default; overwrite is explicit. Cross-host cuts support regular files; copy supports directories. Delete has no undo. Transfers are not snapshots of growing files.

## Search and foreground commands

Leaving Files: `q` goes back, and so does Esc once there is no selection, visual range, search or filter left to clear (unlike yazi, where Esc then does nothing); `H` goes to the hub. The header's dim `‹ back` and the status line's `q back · H hub · ~ help` hint are tappable.

Each device's screen lists its persistent work; file operations live on browser keys. In Files, `/` highlights matching names without hiding or reordering files; Enter finishes typing, `n`/`N` cycle matches, and Escape clears the search. `f` filters names. With no active search, `n` chooses a session profile for that folder; New session remains available through the palette while searching. `:` opens a command prompt pinned to the execution host/folder and runs it in the native terminal; Ctrl+C interrupts the command; Ctrl+Z offers Resume/Cancel; Enter returns after completion. Fish/Bash/Zsh load interactive wrappers; sh/dash commands use login startup. Shell startup themes should respect `CX_VIEWER_THEME=1` during native commands; cx does not reset the viewing terminal’s palette. Commands are foreground work; cx does not save or retry their text, and do not survive disconnect as managed sessions do.

## Previews and Markdown

Files: Enter previews text, Markdown/code, images and PDF pages; tar/tar.gz archives list entries without extraction. PDFs show a page counter; j/k, arrows, Page Up/Down and each wheel step change pages. Rapid scrolling coalesces renders; Escape returns to the file. Images/PDFs use a larger workspace and optional native graphics in detected compatible terminals (Foot Sixel tested; Kitty/Ghostty physical testing remains unavailable). Unknown terminals and nested tmux use colored-cell fallback; monochrome mode uses text. Translucent images fall back on Sixel to preserve the default background. Source files and Markdown code fences use bundled language grammars and your ANSI palette, without running code or installing interpreters. `j`/`k` scroll text previews, Space pages down; `gg`/`G` go to the start/end. Large text streams in as you scroll, up to 4 MiB (Markdown 1 MiB); a preview that stops short says so in its header (`first 4.0 MiB of 6.2 MiB`, or `partial`), and a single line longer than 128 KiB ends it there. `/` searches text live with case-insensitive fuzzy matching and highlighted results; Enter finishes editing, `n`/`N` select the next/previous match. Escape (or `h`) clears a search, then returns to the list with the file selected. Image/PDF content search is unavailable. `d` deletes files after confirmation.

Markdown previews render aligned tables (labeled wrapping rows in narrow panes) and fenced code with language-aware, bundled syntax highlighting. Previewing code never runs it and does not require a language interpreter. Inline Markdown links use underlined labels; click one or press `o` to inspect targets, then Enter to open an HTTP/HTTPS link in the viewer’s graphical browser. Relative file links resolve from the document’s directory on its device; heading fragments jump within previews and Escape returns to the parent preview. Directory links ending in `/` open the browser. Shift+click opens a link directly where the terminal forwards it; Ctrl+click is the fallback (Foot normally reserves Shift for selection). Other URI schemes are rejected. List continuation rows align under their text. Common TeX math in `$…$`, `$$…$$`, `\(…\)` and `\[…\]` becomes readable Unicode (Greek, operators, scripts, fractions, roots), with no extra installation; unsupported TeX and ASCII-mode math retain their source. This is a bounded text preview, not full TeX typesetting. Unknown languages stay plain; `/`, `n`/`N`, `j`/`k`, and `gg`/`G` retain preview navigation.

## Clipboard and terminal interaction

Clipboard: managed sessions forward application copies and tmux selections to the viewing terminal using OSC 52; no Wayland/X11 socket forwarding is needed. Wheel up to scrollback, drag to select and copy, or press Space, extend with arrows, then Enter/y to copy; Escape returns. Shift+drag uses the terminal's own selection when supported, including visible Claude/Codex text. Paste with your terminal's normal paste shortcut (usually Ctrl+Shift+V); in Neovim enter Insert mode first. Terminal clipboard policy must permit OSC 52 writes. Neovim in the synced Arch configuration uses its tmux clipboard provider. Its `"+p` reads can return an older tmux buffer on slow SSH links because the upstream provider waits only 50 ms for a clipboard response. Use normal terminal paste for reliable viewer-to-session paste; terminal policy may also forbid clipboard reads. Personal tmux servers and external sessions retain their configuration.

## Devices and network discovery

The hub map shows enrolled devices with the links between them (edges.json and migrated postures); its colours are the network and its strokes the access policy (solid key, dashed password only, an arrowhead for one way, faint with `⊘` for closed). A device's status is the evidence of its last session listing. LAN neighbours appear in the palette after a Refresh, each with its observing device; the palette row's detail says what is known (SSH port, authentication, internet) and Refresh on a device's screen checks only that device. Fresh LAN neighbors receive bounded background SSH-port checks, cached for 90 seconds (up to 32 new checks per refresh). Enter begins connection through the observing device. An open port does not prove SSH authentication or internet access. Internet evidence belongs to the selected observing host and uses recent ICMP checks; neighbor internet access stays unknown until enrolled and observed. No subnet scan or interface changes run automatically. `cx add user@host --via enrolled-device` also works directly, without a prior robot session. Routes reuse OpenSSH host-key checks and existing identities, without copying keys or forwarding an agent. For identical private addresses behind different gateways, use distinct SSH aliases; conflicting enrollment is refused. Connecting a LAN neighbor asks for its SSH account, then uses masked OpenSSH password/key/MFA prompts when needed. New host keys require explicit fingerprint approval; mismatches fail closed. Passwords are not stored, logged or passed in arguments/environment. A private cx-owned SSH connection can remain authenticated for ten idle minutes, including after a failed enrollment; later access may require another unlock or your existing keys. Background checks never prompt. This protects against accidental disclosure, not malicious processes sharing your Unix account or root. IPv6 link-local enrollment is currently unavailable.

Refresh on the hub (palette **Refresh**) observes each device's network and, with a bounded port-22 check, its LAN neighbours; they then appear in the palette under **Neighbors**, by hostname hint or address with the observing device. Enter on a neighbour connects through its observer and asks for the SSH account; successful enrollment adds its authenticated hostname to the device list. **Add by SSH address** stays available in the palette for aliases, uncached hosts and networks discovery cannot reach. It uses the selected device as its gateway, shown in the prompt, and can match `.local` names against that observer's discovered addresses. After authentication, enrollment checks Linux x86_64/ARM64 compatibility and installs the helper plus signed portable tmux if absent, using only minimal host utilities. Existing system/user tmux is preserved; a broken one reports an error. Password prompts use dots with F2 or a clickable Show/Hide button. Unsupported platforms are reported before installation. `cx terminal --device NAME` remains available as a CLI-only ordinary SSH login without tmux; type `exit` to return. Use New shell in the UI for persistent terminals. Persistent sessions still require tmux.

## Updates and reconnection

Updates check signed stable releases in the background, retain the running version when offline, and restore the viewer when idle. The workspace also checks accessed SSH devices without authentication prompts; their own updater verifies the correct architecture and installs under its maintenance lock. If a remote device cannot download releases, the viewer downloads and verifies the signed artifact for that host architecture and transfers it over the existing SSH route. This supports ARM64 CoreOS and Jetson without replacing OS packages or changing their network. Remote checks run when a viewer accesses the host; a robot does not need its own running viewer or public egress. After a verified helper upgrade, metadata reconnects and previews reload without restarting terminals or jobs. Checks coalesce per host and back off when unavailable. Run `cx update --check` to check explicitly, or `cx update --device NAME` to update an enrolled SSH host. Sessions/jobs keep running independently. A lost metadata connection retries read-only session/device checks once; mutations are never replayed. Cached rows are unconfirmed until a successful Refresh; absent unpinned rows disappear, while pins remain saved references. Connection errors identify SSH access problems separately from terminal state.

A viewer exits when its terminal goes away, and an update's restart handoff is consumed whether or not it restores. Run `cx orphans` to list this user's stray cx processes (terminal gone, reparented to init, or spinning; a `cx mesh invite` listener left without its terminal is one) and stale restart handoffs; `cx orphans --clean` stops the strays and removes the handoffs. Detached transfer workers are listed but never stopped.

CX shares one SSH connection per device and route. The first contact opens a private OpenSSH master (its socket lives in CX's `0700` state directory, or the private runtime directory when that path would be too long); later helper requests and attaches reuse it, so only the first pays the handshake through every jump host. CX's own options set only the master socket and lifetime, overriding any `ControlMaster`/`ControlPath` in `~/.ssh/config` for CX connections; host names, users, ports, jumps and identities still come from your config, and agent forwarding stays off. The master is always a dedicated background connection that never prompts; helper channels and terminals only join it, so closing a pane or terminal never ends the shared connection. A terminal opened before the master is ready waits at most 1.5 seconds, then connects (and may prompt) on its own; a password-only host therefore keeps prompting per terminal and never leaves an authenticated socket behind (only `cx add` enrollment shares its one password login across its steps). A request likewise waits at most 1.5 seconds for a starting master before connecting directly. After a request times out, CX probes the master with a trivial channel and, if the link silently died, stops it from taking new clients so the next request reconnects at once. `cx prewarm <device>` opens (or checks and replaces) a device's master ahead of use. An idle master exits ten minutes after its last session ends, and removing or re-targeting a device closes its masters at once (each master records its exact target in a private `.owner` file before it connects). Any other idle master, such as one whose socket is gone, ends on that same ten-minute timer. Set `CX_SSH_MULTIPLEX=0` to connect directly every time.

[Back to README](../README.md)


### Integrated session terminals

Sessions preserve provider-authored titles. Generic labels include folder, provider,
and a compact runtime identity; separate conversations are never merged by label.
Hovering over a session never selects it. Each tile shows the session's last
screen in its own colours; Space peeks the selected one 1:1 (Tab steps through
the others). **Enter attaches** to the running shell or agent; `w` watches it
read-only. **A tap on the peeked screen types into it** inside CX (one Esc goes to
the session too); Ctrl+\, Esc Esc or a tap outside the screen (the typing bar's
`ctrl+\` chip, the crumb) stops typing.
**Ctrl+\ (or a tap on the `⏏ detach` chip) detaches** from an attached session;
Ctrl+], Ctrl+O, Escape and Ctrl+C go to the application. To send a literal Ctrl+\
(SIGQUIT, nvim Ctrl+\ Ctrl+N), use the palette (`/`) → **Send Ctrl+\ to the last
session**: it forwards one to the session you last detached from and re-attaches it.

The pane preserves colors, blank rows, alternate-screen content and cursor location.
It refreshes at up to 4 Hz while browsing and 10 Hz while focused. Its viewport
follows the cursor when the existing terminal is larger than the available space.
CX does not resize the original terminal. Screen text stays in memory, never in
navigation caches or pins. Remote interactive panes require `live-session-pane-v1`;
older helpers retain native attachment. Input verifies the exact runtime and pane
and is never retried automatically.

Use `c`, `x` or `n` on a device (or **New session** in the palette) for Claude, Codex
or a shell. Bracketed paste and application cursor modes are honored. Pasted input is
limited to 4096 bytes including bracket markers, with an explicit refusal for larger
pastes. Wheel over the peeked screen (including while typing) to read the terminal's
retained history. CX loads bounded pages on demand, so scrolling is no longer
limited to the most recent 500 lines. Reading holds the current page steady;
scroll down to return live. Focusing or typing also resumes live output. History
stays in memory. tmux's own retention limit still applies; discarded output cannot
be recovered by scrolling. While output is evicting tmux's oldest history, earlier
page positions may shift.

Full-screen applications that enable mouse tracking receive wheel reports in the
preview, letting the application navigate its own conversation history. Other
mouse input is not forwarded. Applications that do not expose history through
terminal scrollback or mouse scrolling still need their own history controls.
Older remote helpers keep their earlier preview behavior; update the helper for
paged history and application wheel support. Press Enter for native attachment.

Prompt backgrounds preserve trailing painted cells and SGR styling across rows,
so muted prompt blocks render consistently instead of highlighting only letters.


Codex native attachments keep fallback prompt/answer highlights neutral within
CX-managed panes. Explicit per-pane palettes win, and the fallback is removed when
reattaching after returning to a shell. To apply a contrast update to an existing
Codex session, return to CX and reopen the native terminal; the conversation need
not be restarted. Embedded panes honor the same palette and native dim/reset styling.
