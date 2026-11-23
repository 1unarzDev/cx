# CX user guide

Detailed controls and behavior for sessions, files, terminals, network discovery and updates. For installation and dependencies, see [installation](installation.md).

Plain `cx` opens the **hub**, the network around this device; this device's Files open in the folder `cx` was started from. An update restart restores the existing workspace instead. The design is described in [concept-mission](design/concept-mission.md).

## Controls

One grammar on every screen: **a device letter, then a session slot or a verb**. Letters are underlined in device names on the map and printed on every tile and palette row; they stay the same while a device stays enrolled (`e`, `c`, `x`, `a`, `o` and `n` are never devices: `e` edits a link and `c x a o n` launch on this device; `g h j k l` are used only past 15 devices, so `gg` is the first row on the hub; 20 letters in all, and devices past them are opened from the map or `/`).

| Keys | Action |
|---|---|
| device letter | Open that device's screen (from the hub, peek or palette) |
| `s d g q …` (device screen) | Attach the session whose tile shows that key, at once. Keys go out home row first (`s d g`, then `q u i y z v m`, then `1`-`9`; `a` and `o` start agents); a session keeps its key until it ends, and a new one takes the first free key. Past 19 sessions on a device, `/ +N more` finds the rest |
| `s d g …` (device with nothing running) | The launch panel in place of the tiles: `s` resumes the newest session that ended there while cx watched (the agent's latest conversation in its folder, or a shell there), then `d g …` open the launch chooser in recent folders (ended and pinned sessions' folders, the last Files folder). `c x a o n` start new agents as everywhere; only the agents the device's helper reported are offered |
| `c` / `x` / `a` / `o` / `n` (hub) | New Claude / Codex / Antigravity (`agy`) / opencode / shell on this device in the folder `cx` was started in, attached at once; an agent in the mode (Default / YOLO) it was last started in on this device. The SESSIONS rule says where (`here ~/proj/foo`, cut from the left on narrow screens) |
| `C` / `X` / `A` / `O` (hub) | Resume the last Claude / Codex / Antigravity / opencode conversation in that folder, in that mode |
| `c x a o n`, `C X A O` (device screen) | The same on that device: this device in the folder `cx` was started in, another device in the last folder Files showed there (in this viewer), else its home. The launch row says the folder when it is not the home |
| Enter (device screen, nothing selected) | The full chooser in that folder: agent, then Default / YOLO with the last mode preselected |
| Enter, Space | Attach / open the selection; peek a session's screen (Tab steps through peeks) |
| Backspace, Backspace | Stop the selected CX-managed session; any other key keeps it, never `y` |
| `e` + letter | The link to that device: `a p o` (outbound) / `A P O` (the way back) set key, password or off at once, `j k` move the lane bar, `u` undoes |
| `r` (device screen) | Retry / refresh the device |
| `f` (device screen) | Its files |
| `H` (Shift+H), Esc Esc | The hub, from any screen, Files included (a second Esc within 400 ms). v0.2.1's `` ` `` still works, unlisted, and goes away in a later release |
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
| Files: `!` then `c` / `x` / `a` / `o` / `n`, `!` Enter, `e`, Alt+← / Alt+→ | New Claude / Codex / Antigravity / opencode (last mode) / shell in the folder, the full chooser there, send to a device, yazi's history back / forward (`H` is the hub) |
| Files: `s` + letter, `t`, `T` | Send to a device, choose a destination device/folder, transfer results |
| Files: `r`, `d`, `M`, `o`, `S`, `n` | Rename, delete (confirm), new folder, conflict policy, sessions here, new session here |
| Device screen: `w`, `p`, `t`, `b`, `E`, `D` | Watch, pin, agent tasks, edit the viewer's link (edge policy), edit endpoint, remove |
| Invite (`+`): Esc, `R` | Close the invite, or replace it with a new code |
| Keys (`K`): `s d g`, `h` | Set or rotate that secret, or add a new one by name |

`+` opens a single-use ssh-overlay invite for 30 minutes (`cx invite` from a shell): the code, the `cx join <code> <host>` line to run on the new device, the other addresses it listens on and a QR code of that line. `y` copies the code and `Y` the line (or tap them), through the terminal's clipboard escape. It listens on this host's mesh and LAN addresses until one device joins, it expires, or you press Esc; leaving the screen closes it. On the keys screen a slot key opens a masked entry: type or paste the value (it is shown only as dots and a count) and Enter stores it through `cx secrets` (keyring, else the 0600 file); Esc discards it.

Every node, tile, chip, file row and palette row is a tap target running its key's command. A right-click or long press on a target selects it and opens a menu of the selection's keys. Hover never selects. On phones (under 70 columns) the hub is an indented device tree and footers wrap; keys read `ret` and `bksp`.

Counts work with Up/Down, j/k and PageUp/PageDown in lists, dialogs and transfers; in transfer details, Tab changes focus so the same keys scroll details. File search and preview matches support counted n/N. Search, filter, command, password and palette inputs own ordinary typing.

## Sessions and permissions

A device's screen keeps all observed shells and agents visible, with pins first. Press `p` to pin/unpin the selected session. A pin promotes its existing tile; it does not create a duplicate. Pins persist after the terminal and viewer close. An unavailable pin opens a choice to browse its saved folder or refresh its device; it never restarts a terminal automatically. `not seen` means absent from the last successful inventory, while `? cached`/`? stale` means current state is unconfirmed. A terminal that no longer exists is never offered: if one ends just before you press Enter, its tile is removed with a quiet `‹name› ended · removed` and you stay where you were (only a detach returns to the hub). A device's screen re-checks its `outside cx` terminals every 3 s (the hub every 30 s), and ones remembered from the last run show only once their device lists them again. A device cx cannot reach keeps its tiles with their last screen and the reason; Enter on one only repeats the reason, it never tries to attach.

Opening a provider in a folder with existing sessions shows every matching runtime by name, provider, short identity and observed state. Choose the intended session or explicit New. New agents proceed directly to permissions, without an unidentified “latest” prompt. Distinct conversations in the same folder remain separate; CX does not infer conversation identity from a title or folder.

Cached session/folder navigation is saved locally and displayed before refresh. Failed refreshes retain rows. Saved file identities are stripped and refreshed before mutation. In Files, `!` Enter offers the sessions already in the folder beside New. Session tiles show each session's last screen from memory (never written to disk) in its own colours, so two agents look different because their screens do.

On the hub `c`, `x`, `a`, `o` and `n` start Claude, Codex, Antigravity, opencode or a shell on this device in the folder `cx` was started in and attach at once; on a device's screen they start there (this device: that folder; another: the last folder Files showed there, else its home). The full chooser (`!` Enter in Files, Enter on a device with nothing selected, palette **New session**) uses the current folder from Files or where the device's keys start. The same `c`, `x`, `a`, `o` and `n` pick Claude, Codex, Antigravity, opencode or Shell there (only the agents the device's helper found are listed), and `d` picks Devcontainer; h/l and Left/Right move between choices. Missing runtimes stay disabled, and selecting an agent proceeds to Default/YOLO permissions. The palette's Files and New session actions ask for a device; New session then offers provider/folder setup. File and command actions show their current device/folder scope. New sessions use the chosen execution host and folder. Agent choices reflect installed launchable runtimes. `w` on a device's selected session opens a read-only terminal; ordinary opening gives input. Starting Claude/Codex manually inside a shell updates its badge while the agent owns the terminal; returning to the shell restores it. The original shell remains attachable after its agent exits. When a standalone agent process exits, its dead single-pane runtime is omitted; a session with other panes remains visible. Closing a viewer detaches without ending work. CX follows process and pane evidence rather than treating Ctrl+C as proof of exit. In managed terminals, the wheel reviews retained scrollback unless the application requests native mouse input. Antigravity and opencode scroll their own transcript, like Codex: cx starts agy full screen for that session (`--settings '{"altScreenMode":"always"}'`, never saved to your agy settings) and opencode with mouse capture on (an inline `OPENCODE_CLI_CONFIG_CONTENT='{"mouse":true}'` layer merged over your `cli.json`, as opencode documents; it replaces any value of that variable you set yourself), so both request mouse reporting and the wheel goes to them, never tmux copy-mode. Fullscreen Codex with a stale mouse policy receives genuine wheel events through a verified foreground-process fallback; agy and opencode have no such fallback (if either turns its mouse off, the wheel does nothing over an empty screen and opens scrollback only over real history; never arrow keys). Empty terminal history does not open a `0/0` buffer (the Codex fallback requires tmux 3.4+); Escape returns to the live terminal. Up/Down remain application keys. Because these agents own the mouse, select text with Shift+drag (the terminal's own selection, when your emulator supports it). Existing personal tmux sessions retain their bindings.

In a container, `n` probes installed agents for its selected user and folder. Choose Claude/Codex to join an existing runtime, start New, or choose a past conversation in the provider’s own picker. Both new and history launches offer Default/YOLO. Sign-in and history stay inside that container; host availability does not enable container agents. See [containers](containers.md).

Press `:` to run a command in the current folder. It opens in a floating panel instead of leaving CX: output streams live with color, typing goes to the command (including Ctrl+C), Esc closes the panel while Ctrl+O opens it full screen, and the exit status shows when it finishes. CX is never suspended.

Launching, counted in keys from a shell in `~/proj` (this device's letter is `t`):

| Action | Keys | Count |
|---|---|---|
| New Claude / Codex / Antigravity / opencode / shell in `~/proj`, last mode | `cx` ⏎ `c` / `x` / `a` / `o` / `n` (then, from the hub, the same key) | 2 (1) |
| Resume the last conversation there, last mode | `C` / `X` / `A` / `O` on the hub | 1 |
| The same on another device (`p`), in its last Files folder | `p c` … `p n`, `p C` … `p O` | 2 |
| New agent in the folder Files shows | `!` then `c` / `x` / `a` / `o` / `n` | 2 |
| A Codex in YOLO through the chooser, nothing running yet | `t` ⏎ `x` (⏎ New, when past conversations exist) ↓ ⏎ | 4–5 |

`c`, `x` and `n` sit on the bottom row under the left hand's fingers, `a` on the home row and `o` under the right ring finger; Shift is the only chord, for resume. cx remembers the last mode per agent per device in `launch-modes.json` in its state directory, shared by every viewer (and the `cx claude` / `cx codex` CLI) and changed under a lock, so one viewer's choice never erases another's. A remembered YOLO on a remote device whose helper capabilities are stale is checked first (`Checking <device> for YOLO · starts when it answers`); an old helper, no answer, another session starting meanwhile, or moving to another screen before the answer starts nothing and says so. With a container session selected on a device's screen, its keys open that container's chooser instead. `C` / `X` / `A` / `O` run `claude --continue` / `codex resume --last` / `agy -c` / `opencode -c` in that folder. agy and opencode have no picker cx can start: choose an older conversation inside them (`/resume` in agy, `/sessions` or Ctrl+X L in opencode).

New agent sessions offer **Default** (existing host permissions) or **YOLO** before launch; the chooser preselects the mode last used for that agent on that device. YOLO applies only to that session: Codex bypasses approvals and sandboxing; Claude and Antigravity bypass permission prompts (`--dangerously-skip-permissions`); opencode auto-approves (`--auto`). From a shell: `cx claude --yolo` / `cx codex --normal` (see below). Existing sessions keep their launch permissions. Update remote helpers before selecting YOLO.

Antigravity (`agy`) and opencode are offered on a device once its helper (v0.5.0 or newer) finds them on the launch shell's PATH (`agy --version`, `opencode --version`). opencode starts with `--standalone`, its own server inside the session, so Backspace Backspace or `cx kill` stops both. A tile cannot say **needs you** for either: opencode keeps permission prompts only on its server's event stream, and agy's waiting state is not confirmed yet; their tiles show working or idle from the terminal (titles and state from their own databases where cx can read them). Give them API keys through `cx secrets set gemini.api_key` (then **expose to sessions**, `e` on its card), never through the viewer's environment; `GOOGLE_CLOUD_PROJECT` and `GOOGLE_GENAI_USE_VERTEXAI` from the environment cx was started in reach new sessions. Neither is offered inside containers yet.

Backspace stops the selected session after an inline question on its tile (on a device's screen, in the hub's sessions zone and in peek, naming the session): a second Backspace stops it, any other key (Enter and `y` included) keeps it. The runtime identity captured when it asked is the one stopped. It stops CX-managed shells, Claude and Codex sessions, split ones included (every pane goes, and the notice counts them), together with anything they started that left the terminal or ignores hang-ups (asked to exit, killed after 3 s). It also stops a Claude or Codex agent running in a terminal outside tmux (`outside cx` tiles) once its identity is re-checked on its device: the agent's process group gets a hang-up and is killed after 3 s; the window is left to its terminal app. External tmux sessions and plain shells outside tmux are protected. On a remote device the question comes at once; if cx's knowledge of that device is over a minute old it re-checks in the background and refuses only if the device's cx turns out too old.

Moving a terminal into CX (Enter on an `outside cx` tile) opens the moved session as soon as the move is seen. The old window, now empty, is closed (killed: terminals such as foot ignore a polite hang-up, and closing the frozen window later would end the moved session); terminal servers serving other windows are never touched.

### From a plain terminal

`cx claude`, `cx codex`, `cx agy` (also `cx antigravity`), `cx opencode` and `cx shell` start a CX-managed session and attach to it, without the UI: in this folder by default, or in `[DIR]` (relative to this folder, `~/…` allowed). `-d DEVICE` starts on an enrolled device instead (its home by default; give `~/…` or an absolute folder). An agent starts in the mode last used for it on that device (the same memory the hub's `c` and `x` use); `--yolo` or `--normal` choose and remember it. `-r` resumes the latest conversation in that folder, `-p` opens the agent's own picker (agy and opencode refuse `-p` and say where their picker is: `/resume`, `/sessions`). Ctrl+\ detaches as in the UI; a dropped connection to a remote device reattaches by itself (Ctrl+C gives up). `cx claude lend|revoke|list` remain the [gateway lending](claude.md) commands, so a folder with one of those names is written `./list`.

`cx kill ID|NAME [-d DEVICE] [-y]` stops a session after asking: a CX-managed or moved session ends with everything it started, and a Claude or Codex terminal outside tmux (its id from the hub, or `cx adopt`'s list) is hung up after its identity is re-checked. `cx adopt [PID|TTY] [-d DEVICE]` moves an agent terminal started outside CX into CX (by its shell's pid or its `/dev/pts/N`; without one it lists them to choose) and attaches; a sudo password, when the move needs one, is asked with echo off. `cx new` and `cx sessions` stay as scripting commands that print JSON.

### When a device cannot be reached

A device CX cannot reach says why on its hub node, its strip and every one of its tiles, within the connect timeout: its name does not resolve (DNS), the connection was refused, no route or the connect timed out, its host key could not be verified, or SSH authentication failed, with when it was last seen. No tile waits blank.

When a device's own name does not resolve or connect, CX tries addresses it has seen that device answer on, in order: your ssh config's `HostName` for the device's name or host (`ssh -G`, read-only), its last overlay address (WARP or Tailscale, from its own interfaces), its last LAN address, then `<host>.local` (mDNS). Each must present the host key already pinned for the original name (`HostKeyAlias`, strict checking; a different key is refused and never learned). The first that answers is remembered in `~/.local/state/cx/addresses.json` and shown quietly on the device (`via 100.96.0.12`); when it stops answering CX forgets it and searches again. CX never changes networking or DNS configuration.

### Moving terminals started outside CX

A device's screen also shows your shells and Claude/Codex sessions that CX did not start. These include sessions in an ordinary terminal window, an SSH login, another tmux or screen server, a default-server tmux pane, or a `docker exec` client. Rows outside tmux end with where they live, such as `· terminal` or `· ssh`.

Remote devices list them too: agent terminals outside CX's tmux appear as their own tiles marked `outside cx · ⏎ adopt`, with the agent, folder and age but no screen (CX cannot read a plain terminal's screen), and are not counted as managed sessions. When a device cannot move terminals yet, its tiles say exactly why; Enter then installs the signed `cx-reptyr` there with `cap_sys_ptrace` (one `sudo` password on that device) and the move continues by itself, with no prompt on later moves. A remote device whose relay only works through `sudo` (installed with `install` below) is offered this setup too: a password panel cannot reach its helper over SSH.

Press Enter on such a session to move it into CX. The terminal's processes keep running: the same PIDs, threads, child tools and in-progress work, with only where the terminal is shown changing. A move takes well under a second, and output continues during it. If the terminal was a single-window emulator (foot, kitty, alacritty, ghostty, wezterm, xterm, urxvt, st, konsole, tilix, xfce4-terminal, terminator), cx opens a new window of the same terminal app, in the same folder, attached to the moved session, and closes the frozen one, so your work stays where it was on screen (your window manager may place the new window elsewhere; both it and the viewer show the session, sized to whichever you used last). This also happens on the terminal's own machine when the move was asked for from another device. If the new window cannot be opened (no display, or the app is gone), the frozen window is just closed, and the notice after the move says which happened. Other single-window emulators are closed without a replacement; a multiplexer (tmux, screen) or a server emulator that drives other windows (gnome-terminal, konsole) is left alone, and you can close that window yourself. The moved row then appears once, as a managed CX session. Stopping it with Backspace, Backspace on its tile, exiting its shell, or `cx reset` ends it as it would end any CX session. Programs that ignore SIGTERM are not force-killed.

Moving uses a guarded copy of [reptyr](https://github.com/nelhage/reptyr). Install it once per device:

```sh
cx device ptrace local install    # or a device name; no admin rights needed
cx device ptrace local status
cx device ptrace local uninstall
```

`install` places `cx-reptyr` in `~/.local/bin`. Each move from a viewer on that device then runs it through `sudo`, and CX asks for your password in a panel (once per move; the move's own time starts after you answer), so no privileged state persists between moves. The relay accepts only one exact form: it refuses any process not fully owned by you, re-checks every process identity CX observed, and drops back to your user (and all capabilities) once the terminal has moved.

To skip the per-move password on a device you trust, `cx device ptrace <device> enable` installs a root-owned `/usr/local/libexec/cx/cx-reptyr` with `cap_sys_ptrace` (one `sudo` at setup; undo with `disable`). Any program running as you on that device could then use it to take over your other terminals, so enable it only where you want that. Hosts with Yama `ptrace_scope=3` cannot move terminals. Without setup, these rows stay visible and default-server tmux rows still attach as before.

Automatically named shells show the running executable, such as `running cargo`; new managed Fish shells remember short commands as `last: git`, while older/Bash/Zsh shells show the most recently observed command as `recent: git`. Arguments and shell history are excluded. Claude native names and Codex names use exact process metadata when available; daemon-backed Codex sessions without a reliable identity retain their folder label. Custom names stay unchanged and label refreshes preserve row order.

A new terminal opens at once and paints as its shell and program start. Whether Claude or Codex can start on a device comes from that device's last capability check (cx rechecks it in the background; the launch chooser never waits for it). If a launch fails anyway, for example because the provider was uninstalled since, its pane stays for 30 seconds saying `cx: the launch failed (exit status 127: command not found)`; detach to go back. A terminal that ends before cx could open it is reported in a notice with its exit status.

Session rows refresh while Files and other panels are open. Failed device checks discard unattachable cached rows; successful checks recover existing managed/default tmux terminals, including recognized Docker exec terminals in running devcontainers. Stopped, restarted or missing container runtimes disappear from the device's screen after authoritative checks. The Containers inventory retains actual stopped containers for an explicit Start; cache cleanup never deletes Docker objects or reattaches arbitrary orphaned PTYs.

The Sessions state column animates for observed foreground commands, known native agent work, or recent terminal output. `output` means the terminal emitted output; it does not prove a model is reasoning or a silent command is running. Stale observations expire. On a device's screen, `t` inspects the selected session's provider-reported tasks/subagents and task text. Open/closed child records are task states, not busy indicators. The dialog follows refreshed metadata and closes when the session disappears.

Exact native names and project directories follow provider metadata without changing terminal identity. Custom CX names stay unchanged. New host Codex launches use a separate process (`--no-daemon`) when supported, improving current-thread tracking at the cost of additional memory; older versions fall back. Existing sessions are not restarted. New supported host launches also set Codex’s native thread-name terminal title per launch to identify idle thread switches. This requires explicit, unique names of at most 48 characters among owned threads; otherwise CX shows “Directory unavailable” and avoids using the launch folder for contextual commands. Saved provider configuration is unchanged. Codex database metadata additionally needs the optional `sqlite3` executable; unsupported identities omit optional metadata. No transcripts are read. Antigravity (`agy`) and opencode tiles show the agent's own conversation title and working / idle, read from its local database (the newest top-level, live conversation whose folder is exactly the agent's folder; `sqlite3` needed). They never show "needs you": neither agent records that it is waiting for you. opencode's older file-based storage shows no title.

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
A dropped connection to a remote device does not leave the session: its last screen stays
up with `connection lost · reconnecting (2s) · esc to leave` on the bottom row, and cx
reattaches the same session by itself (at once, then after 0.5, 1, 2, 4 and every 5 s),
over the device's warm SSH connection or a new one, on an address seen working when its
name stops resolving. Esc gives up and returns to its tile; a session that ended
meanwhile is reported. Terminal connections ride out about a minute of silence (ssh
keepalives every 10 s, six missed) before they count as dropped.
Attaching from a tile grows it to the full screen over at most 45 ms (it costs an attach at most about 40 ms) (ease-out, a thin
edge outside it, the screen behind it dimmed). It shows the screen you are about to see,
starting from the tail the tile showed, and lands on it cell for cell. The attach is
prepared while it grows and hands off as it lands; on a remote device it starts a round
trip early. Detaching shrinks it back into its tile; any key skips
either. It is off with `CX_REDUCED_MOTION=1`, on phones and over slow terminal links.

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
