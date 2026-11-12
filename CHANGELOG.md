# Release notes

Each stable release describes user-visible changes here before tagging. GitHub release descriptions are generated from the matching version section; missing notes block publication. Unreleased work is not included in published descriptions.

## v0.2.1

The strict review of v0.2.0, issue by issue: 159 confirmed findings, fixed or recorded in STATUS.md.

### Fixes
- **Sessions.** An attached session no longer streams its screen to the viewer underneath: the watch ends at every terminal handoff and resumes on return, so detach stays instant after a long attach. A replaced watch's late frame never hides the new first screen. Pushed session lists refresh every 3 s again (titles, running commands, agent state); the watched screen recovers after a failed capture instead of staying "unavailable"; an oversized control-mode capture fails whole instead of dropping rows. `cx orphans` sees cx's tmux control clients, and the attach ticket is one file.
- **Typing into a peeked session** (a tap on its screen) now has clear exits: Ctrl+\, Esc Esc (one Esc still reaches the agent), or a tap anywhere outside the screen, including the typing bar's `ctrl+\` chip and the crumb.
- **Palette "Send Ctrl+\"** works from the hub after a detach: it forwards one to the last session and re-attaches it (selecting it first if its screen is not live yet).
- **Keys.** `gg` is always the first row on the hub (`g` is now assigned to a device only past 20 devices, like `h j k l`). The new-session chooser picks Shell with `n`, as on a device's screen, and its tiles show that key. A pending `e` takes only a device letter; any other key does its usual job. `⌫ s ⌫` moves a stop question to slot `s`. In the link editor Ctrl chords are the keymap's (Ctrl+O no longer refuses a direction), `` ` `` `-` `+` `K` `?` `/` work over it and over the invite, and a second policy change waits for the first.
- **The link editor** draws the link its keys change (never the first stored link); `b` on a device's screen opens it on your link to that device.
- **The hub.** The ticker acts on the device its notice names and never offers another session's slot; a device that was not checked yet is no longer drawn as down, and a stale listing shows `◌` instead of a guessed state; help is the table of the screen you opened it from. Every drawn chip does what its key does. Onboarding's Connect Cloudflare opens the token's masked entry. Light-theme ok/warn colours reach 4.5:1 contrast.
- **Copying.** `y` copies a file's path in Files; on the invite `y` copies the code and `Y` the `cx join` line (or tap either), over the terminal's clipboard escape.
- **Invites.** `cx invite` is the CLI twin of `cx join`, and the line it prints is `cx join <code> <host>`. An invite lasts 30 minutes by default (at most 24 hours); a joiner whose request goes unanswered is told to cancel it. Both sides check that sshd answers on port 22 before pairing. The hub lists the other addresses an invite listens on, and Esc then `+` no longer finds the port taken. Port scans and silent connections no longer count as wrong codes; a refused join keeps a removed device out of automatic pairing.
- **Previews.** A large preview says when it shows only part of the file (`first 4.0 MiB of 6.2 MiB`, `stopped at …`, `partial`); a line longer than a chunk is never split or garbled; streamed code keeps its highlighting across chunks; `G`/End have no 100,000-row limit; Space pages down; `h`/Left in a preview step back like Esc. Markdown streams into one layout, and a list row that fits skips the wrap pass: `G` on a 2 MB Markdown file right after it opens 101 → 5 ms, and opening it 47 → 23 ms (medians of interleaved runs against v0.2.0).
- **Files.** A folder's later pages come from the same snapshot as its first page (no repeated or skipped rows when it changes while listing), or the list restarts keeping the selection.
- **SSH.** Warming a connection runs the edge policy first, so a `none` link opens no SSH master; the master's owner record is written before the master starts; pane capture and input ride the interactive channel.

### Changes
- The WARP Worker accepts only the token the mesh was deployed with for its owner endpoints: run `cx mesh deploy` once after updating, and after changing the token. `--backend cloudflare-warp` now covers only `status` and `ranges`; the rest is `cx mesh` without `--backend`, as before. `cx mesh backends --json` no longer lists capabilities.
- `CX_SECRETS_BACKEND=keyring` is gone (`auto` or `file`); `cx mesh invite --host-key` is gone (both sides use `/etc/ssh/ssh_host_ed25519_key.pub`); `cx edge list` drops its `observed` column.
- Helpers name each watch in its replies and frames; older viewers ignore it, and a newer viewer treats an older helper as before.

## v0.2.0

A new interface and a faster core. Everything below is new since v0.1.95. Numbers are medians of interleaved A/B runs against the v0.1.95 release binary on the same host (release builds, local fixtures, 120x40 PTY); see STATUS.md for the full table.

### Features
- **The hub.** `cx` opens on the network around this device: a calm radial map (this device in the middle, peers around it, devices behind a relay further out; colour is the network, the stroke the access policy), the selected device's vitals, live session tiles ranked by what needs you, and a one-line ticker that pairs each problem with the keys that fix it. One grammar everywhere: a device letter (underlined in its name), then a session slot `a s d g` or a verb (`c x n` new Claude/Codex/shell attached at once, `f` files, `r` retry, `e` + letter the link editor, Backspace twice stops). `` ` `` or Esc Esc returns to the hub, `-` re-attaches the last session, `/` finds anything, `?` shows the keys for the screen you are on. Phones get an indented device tree. The sidebar, Actions panel, view tabs and Network tree are gone.
- **Touch and mouse.** Every node, tile, chip and row is a tap target that does what its key does; a right-click or long press opens the selection's keys as a menu; a tap on a peeked session types into it. Hover never selects.
- **Instant sessions.** A helper `Watch` pushes the session list and the selected screen over one tmux control-mode client per server (polling remains for tmux older than 3.2), the viewer wakes on one channel, and SSH masters stay warm with interactive and bulk work on separate channels. Tiles keep each session's last screen in memory (never on disk). Keystroke echo in a peeked session 101 → 1.5 ms; Enter → attached 83 → 12 ms; detach back to cx 270 → 17 ms; peek (select → screen) 5.8 ms locally and 6.5 ms over SSH, attach over SSH 34 ms.
- **Ctrl+\ detaches** from any managed terminal (or a tap on the `⏏ detach` chip); Ctrl+] and Ctrl+O now reach the application, and the palette's "Send Ctrl+\" delivers a literal one. Running managed servers pick up the binding on the next attach.
- **Faster Files.** Lists are windowed, folders prefetch when the selection settles and Enter reuses that read, previews stream in chunks with a retained layout. First rows of a 1,000 / 20,000 / 100,000-entry folder 124 / 124 / 124 → 45 / 46 / 46 ms; preview first paint (5 MB source / 2 MB Markdown) 105 / 111 → 5.7 / 6.5 ms; preview scroll `j` on the 5 MB file 5.3 → 4.5 ms. A 20,000-entry folder now lists in full (v0.1.95 stopped at 10,000).
- **Mesh backends and short pairing codes.** `cx mesh backends` lists ssh-overlay (the default: built from cx's own SSH, no address range claimed), Tailscale and Cloudflare WARP. Press `+` on the hub (or `cx mesh invite --backend ssh-overlay`) for an 11-symbol single-use code, the line to run on the new device and a QR code of it; the new device runs `cx join <code> <host>` (short for `cx mesh pair`). The invite listens only on this host's mesh/LAN addresses, is rate-limited, checks HMAC-SHA256 both ways and closes on use, expiry or Esc.
- **Secrets.** `cx secrets list | set | get --fd/--exec | rm | migrate` keeps tokens in the OS keyring through `secret-tool` (values on stdin), else in a 0600 file in the 0700 state directory, stored the way an unencrypted SSH key is. `K` on the hub sets or rotates a secret in a masked entry that never draws or logs the value and wipes it on close. The Cloudflare mesh token is injected from the store.
- **Per-edge access policy.** `cx edge list` and `cx edge set FROM TO key|password|one-way|none` set access per link and direction, with a preview of the consequences before `--yes`; `e` + a device letter edits the same policy in the hub (`a p o` / `A P O`, `u` undoes). `none` refuses core sync, pairing and jump routes over that edge before any SSH.

### Fixes
- Enter on a session attaches directly when its device answered within the last minute; a refused attach refreshes the list and says so.
- Identical session captures no longer redraw; Markdown previews lay out once at the width they are drawn; a relay version probe retries a just-written binary that is briefly "text file busy".
- An ssh-overlay invite drains a refused request before closing, so the joiner always reads why it was refused.
- After a detach, the session you left always has a tile on the hub, even when it ranks below the row.

### Changes
- The static release binaries are 16.1 MB (x86_64) and 15.5 MB (ARM64), up from 13.6 MB for v0.1.95 x86_64. The OS keyring is reached through `secret-tool` rather than the keyring crate stack (which added about 5 MB and 84 packages); the QR code adds `qrcode`, which has no dependencies of its own.
- Helpers advertise `watch-v1` and `peer-authorize-v1`; older helpers keep polling and the earlier peer behaviour. Update remote devices to get pushed screens.

## v0.1.95

Includes every change from v0.1.86–v0.1.88 plus the work below, which was previously only deployed locally. (v0.1.93 and v0.1.94 were tagged but not published.)

### Features
- Show live, styled tmux screens beside the session list with cursor-following viewports (stacked on narrower screens), and hover over session rows to preview them. Enter focuses the existing terminal inside CX; Ctrl+] returns to navigation and Ctrl+O opens native attachment. Keys and bounded bracketed paste go only to the verified pane.
- Scroll terminal previews with the mouse wheel, including while focused, and page through all tmux-retained history; forward wheel scrolling to full-screen applications that enable mouse tracking.
- Show identifiable existing sessions with explicit new/history choices, persistent pins first, and saved-folder bookmarks after terminals end; restore bounded navigation caches immediately and mark observed agent presence beside folders.
- Launch installed Claude/Codex agents inside the exact container, user and folder with Default/YOLO permissions and native conversation pickers. Shell is part of the normal New session chooser, which offers only profiles available on the device.
- Preserve conversation titles and distinguish generic labels by folder, provider and runtime.

### Fixes
- Show live terminal previews with tmux older than 3.5 (such as Ubuntu 24.04's tmux 3.4), which lacks a format flag CX read; previews previously failed with "Terminal unavailable".
- Start the managed tmux server again when a stopped cx-tmux unit is still loaded because a pane outlived it, instead of failing with "persistent tmux user service did not start". The error now includes systemd-run's reason.
- Preserve prompt background blocks, multiline terminal colors, pane palettes, dim text and cleared SGR attributes in previews; keep Codex's fallback prompt backgrounds muted in CX-managed terminals.
- Collapse duplicate runtime observations, handle dead panes without hiding live peers, and verify terminal generations before native attachment; reject stale scopes, replies and mismatched creation keys.
- Keep live-preview support after capability refresh, continue external-terminal opening after availability returns, and bound stalled capability and relay-version checks.

## v0.1.88

### Fixes
- cx no longer spins at full CPU after its terminal disappears. When the pty hung up while cx ran in its own process group (so no SIGHUP arrived), the UI loop kept busy-polling the dead terminal and ignored SIGTERM. It now exits when the terminal is gone or it has been reparented to init without a tty, and a watchdog force-exits if a stop signal is not honoured within three seconds.

## v0.1.87

### Fixes
- Opening a session now lands you live and ready to type. If the pane was left in scrollback (tmux copy-mode, entered by scrolling the wheel), re-entering it used to show that read-only view until you pressed a key to dismiss it; a focus attach now cancels copy-mode so input works immediately. The read-only watch (`w`) still preserves the scrollback view.


## v0.1.86

### Changes
- Keep a robot's `.local` name as its enrolled target when adding it through a gateway, instead of pinning one of its DHCP addresses. The gateway resolves the name on each connect, so an address change needs no re-enrollment. Existing IP-pinned devices keep working; remove and re-add them to switch.
- Match the shared-LAN preview rules by robot subnet instead of interface names, and never forward or masquerade into `docker0`.

### Fixes
- Pressing `f` to filter, typing, then Esc now clears the filter instead of leaving the half-typed text applied, so the next `f` starts a fresh search. Refining an existing filter and then pressing Esc restores the previous filter rather than discarding it.
- Speed up container discovery and attachment. Discovery now inspects all containers in a single `docker` call instead of one per container — the slow part, because every Sessions refresh reconciles container scopes through it — and attaching no longer repeats the same container check and helper-setup round-trips several times. On a host with many containers this turns seconds of waiting into a fraction of a second.

## v0.1.85

### Fixes
- Stop the previous session's `Connection to … closed.` / `[detached …]` teardown text from flashing when you open a session. CX now clears the screen as it hands off to the attach, so a slow remote connection shows a clean screen while tmux repaints instead of the stale text, which looked like the session had detached rather than loaded.


## v0.1.84

### Changes
- Set up terminal moving from inside CX: when a device needs the ptrace relay, CX runs the one-time setup (with its admin-password prompt) for you and you retry the move, instead of printing a `cx device ptrace … enable` command to run yourself.

### Fixes
- Fix "unsupported artifact member" when installing the terminal-move relay: the signed `cx-reptyr` release asset is now an accepted download member.


## v0.1.83

### Fixes
- Stop external terminals from vanishing when you try to move them: the adoption record is written only once the move can actually proceed, a leftover record from an interrupted move no longer hides its terminal, and a move that needs a password prompt unavailable on that device (such as a remote host without a no-prompt relay) now fails with a clear message instead of making the terminal disappear.
- List only genuine interactive shells and detected Claude/Codex sessions as external terminals, so a short-lived process that briefly held a pseudo-terminal no longer shows up and then disappears.


## v0.1.82

### Features
- Agent resume offers "Resume · choose session" (`claude --resume` / `codex resume`) to pick from the provider's own session list, alongside "Resume latest".

### Fixes
- Move a terminal into CX asynchronously: the loader animates and the sudo password panel appears without the event loop freezing, instead of hanging with no feedback.
- Stop raising "SSH access unavailable" notices for background external-terminal discovery on unreachable devices.


## v0.1.81

### Fixes
- Always show a session's working directory — the agent's launch directory when CX can't pin its native thread — instead of "Directory unavailable".
- Relabel an agent session when its own control panel switches to another project, following the title the agent displays in its terminal.


## v0.1.80

### Changes
- Show the session loader animation while a terminal is being moved into CX.
- Size a command panel to its output and keep the final output after the command exits; a command that produces no output (e.g. `mkdir`, `echo`) shows only a brief status notice instead of an empty panel.

### Fixes
- Never offer the terminal that is running CX (or any ancestor terminal) as an adoptable session, and don't mislabel a CX-running terminal as a shell.


## v0.1.79

### Features
- Run a folder command (`:`) in a floating panel with live colored output and input instead of leaving CX; Esc closes it, Ctrl+O opens it full screen, and the exit status is shown.
- List shells and Claude/Codex sessions started outside CX (terminal windows, SSH logins, other tmux/screen servers and `docker exec` clients) in Sessions.
- Move such a terminal into CX with Enter, keeping its processes, PIDs and in-progress work running. The now-empty single-window source terminal closes itself; multiplexers and server emulators are left alone. Install the guarded relay with `cx device ptrace NAME install`; each move asks for a password through a panel, with no persistent privilege. `cx device ptrace NAME enable` optionally grants a no-prompt capability relay.

### Fixes
- Keep `cx sessions` managed-only; discover outside-CX terminals through a separate request merged by the UI, so scripts and the CLI are unchanged.
- Wait for tmux to report a command's exit status before the panel settles on "finished", so the real status is shown.
- Never list a CX viewer's own terminal as an adoptable session.
- Build the portable `cx-reptyr` as a static binary with the system compiler.

## v0.1.76

### Fixes

- Create managed SSH directories with explicit private permissions, including on hosts with group-writable umasks.
- Recover an empty directory left by v0.1.75 only when its parent is private and owned; reject nonempty or exposed directories.
- Run the three-helper mesh fixture under umask 0002 and verify bounded legacy-directory recovery.

## v0.1.75

### Features

- Automatically connect all core devices on enrollment, promotion, startup and background retry; directed devices remain excluded.
- Generate device-local dedicated keys, exchange authenticated public metadata and pin managed host trust without changing personal SSH configuration or transferring private keys.
- Verify reverse machine/account identities, repair stale aliases using verified portable endpoints, and reconcile newly discovered core devices into the open panel.
- Serialize inventory/policy mutations and retain removal tombstones so concurrent setup cannot undo local removal or demotion.
- Verify newly installed helpers without scheduling redundant background updates that can outlive enrollment.
- Keep staged key installation compatible with static musl builds through the Linux no-replace rename syscall.
- Add disposable three-helper and native OpenSSH regression fixtures. Old helpers, unsupported routes and offline peers remain pending; active sessions are not restarted.

## v0.1.74

### Fixes

- Unpublished candidate: validation found a redundant background helper update during enrollment. Superseded by v0.1.75.

## v0.1.73

### Fixes

- Unpublished candidate: static builds exposed an unavailable libc renameat2 symbol. Corrected in v0.1.74 using the Linux syscall directly.

## v0.1.72

### Features
- Add explicit reciprocal device pairing with portable account@host endpoints, reverse SSH verification and stable machine/account identity checks. Older helpers report that they need updating.

### Fixes
- Restore Claude terminal transparency and clear opaque window backgrounds left by older CX versions when attaching.
- Keep directory availability warnings out of Codex session titles. Preserve verified native/custom titles; show `codex` when the selected thread is ambiguous.
- Stop reporting local core posture as verified bidirectional connectivity. Preserve receiver-local routes and directed peer policies during pairing.

### Integration
- Pairing requires existing SSH trust/authentication and reachable endpoints; it does not install keys, copy SSH aliases, or modify networking. Each required connection must be paired explicitly.
- Installed binaries and active sessions are not restarted by this release. Updated helpers are needed for the pairing command and remote presentation fixes.

## v0.1.71

### Features
- Keep CX visible with an animated loader while a new terminal starts in the background. Hand off once the first screen has content, with a five-second fallback for intentionally silent programs.

### Fixes
- Report terminal startup exits in the panel instead of attempting to attach to an ended terminal.

## v0.1.70

### Validation
- Publication was blocked by a tmux 2.7 compatibility check. Superseded by v0.1.71, which keeps attachment styling out of shell creation.

## v0.1.69

### Features
- Open the invocation folder in Files on startup; restore prior workspace on update restart.
- Show observed command/agent activity and recent terminal output in Sessions. Press `s` for native task/subagent details.
- Track native project/name changes and launch new Codex terminals with `--no-daemon` and a native thread-name title option when supported. Ambiguous selection shows an unavailable directory; existing sessions remain running.

### Fixes
- Reconcile session/container caches in every view; discard unreachable cached shells and dead tmux panes, and revalidate saved container lifecycles on every listing.
- Recover existing tmux-backed Docker exec terminals from fresh devcontainer evidence without reparenting processes or changing containers.
- Report incomplete container discovery honestly, retain labeled workspace identity, and prefer explicit devcontainer configuration paths.
- Keep session selection stable during pruning and route in-flight replies by device identity across removal/reorder.
- Bound and isolate optional native metadata reads, including exact Codex parent/subagent writer trees.
- Improve Claude question-description contrast with solid default-cell backgrounds while preserving text colors and explicit highlights.

## v0.1.68

### Validation
- Publication was blocked by an older native-name fixture expecting a launch-folder fallback for ambiguous writer identities. Superseded by v0.1.69, which validates an unavailable directory instead.

## v0.1.67

### Validation
- Publication was blocked by an enrollment fixture that assumed Sessions startup. Superseded by v0.1.68, which validates enrollment through the global action palette from Files.

## v0.1.66

### Fixes
- Prevent verbose Dev Container CLI build progress from exhausting the result output limit and falsely reporting a failed rebuild. CLI exit status and machine-readable success checks remain enforced.

## v0.1.65

### Features
- Add fresh-evidence upstream selection for internet sharing, preferring measured throughput and using latency as a tie-breaker.

## v0.1.64

### Fixes
- Repair device-removal confirmation controls so arrow keys, h/l, Enter, and clear button labels work consistently.

## v0.1.63

### Features
- Add `cx reset` for the local CX-managed tmux server, and a confirmation action in the workspace. Reset ends managed sessions; external tmux servers remain untouched.

### Fixes
- Open the workspace after an interactive update finds the current version, with a temporary up-to-date notification.
- Allow Left Arrow to cancel the session/provider chooser.
- Refresh Anthropic environment variables from the execution process before creating managed sessions and after reset.

## v0.1.62

### Fixes
- Remove manual device down/up controls; reachability now retries automatically and recovers after successful contact.

## v0.1.61

### Changes
- Remove manual device down/up controls; reachability now retries automatically and recovers after successful contact.

## v0.1.60

### Fixes
- Make portable tmux source downloads retry transient DNS/network failures and reuse validated per-architecture CI cache entries.

## v0.1.59

### Fixes
- Keep the single-direction marker visible by placing it before long device names.

## v0.1.58

### Fixes
- Update the PTY session chooser fixture to follow capability-aware profile filtering, preventing stale expectations from failing push checks.

## v0.1.57

### Changes
- Replace device access arrows with a subtle `*` only on single-direction devices.
- Add the access toggle to the device edit menu with its current state and next action.

## v0.1.56

### Fixes
- Use subtle `↔` / `→` device access markers with ASCII fallbacks.
- Show only available session profiles in the new-session panel, with matching navigation and shortcuts.

## v0.1.55

### Fixes
- Keep SSH target validation local to the store so the full integration-test fixtures and portable builds compile correctly.

## v0.1.54

### Fixes
- Use the single-cell stacked opposing-arrow glyph `⇄` for bidirectional device posture, retaining `<->` in ASCII mode.

## v0.1.53

### Features
- Add an `e` device edit menu for changing an enrolled SSH alias/address or opening the existing removal confirmation. Address edits preserve machine identity and posture while clearing stale reachability state and old saved routes.
- Quiet all connection failures for devices explicitly marked down, including direct actions, so a powered-off robot cannot produce repeated error notices.

## v0.1.52

### Fixes
- Use the empty-circle down marker again and widen the bidirectional posture marker to `←→` so both states remain legible.
- Passive reachability failures now update the device state without producing an error notification; actionable operation failures still use notifications.
- Document the posture controls and keep the local device protected from access or availability changes.

## v0.1.51

### Fixes
- Widen and space device access arrows so bidirectional and viewer-only posture markers remain legible in the sidebar.
- Remove the persistent unavailable-device count from the footer; connection failures continue through the floating typed notification panel.

## v0.1.50

### Improvements
- Remote devices can be removed from the local CX registry with confirmation (`x` in the Devices panel or `cx device remove NAME`). Removal also clears the saved local policy and SSH jump route.
- Mark a device down with `d` or up with `u`. Down devices show a muted marker, skip background helper/network/provider checks, and do not generate repeated unreachable notices.
- Newly enrolled devices are recorded as bidirectional mesh peers; `b` still toggles an enrolled device back to viewer-only access.

## v0.1.49

### Improvements
- Codex and Claude session launchers now offer a fresh session or the provider-native latest-session resume flow. Resume keeps the selected Default/YOLO permission mode and invokes `codex resume --last` or `claude --continue` directly.
- Existing live CX sessions can still be attached separately, while choosing a fresh session always starts a new conversation.

## v0.1.48

### Features
- Device list rows show clear `↔` (bidirectional) and `→` (viewer-to-device) access markers. Press `b` while a remote device is selected to toggle its local access posture; the local device is protected.

### Fixes
- Integrated the bounded device policy store and CLI promotion/demotion commands into the current release line.


No changes queued.

## v0.1.47

### Fixes
- Keep routine file navigation quiet and shorten copy/cut confirmations. Keybind help stays in the existing hints and Help panel; notifications report outcomes, progress and errors.

## v0.1.46

### Features
- Copy, paste and cut between device files and devcontainers, including container-to-container transfers. Chunks stream through existing authenticated helpers; resumable staging stays at the destination.
- The transfer destination picker distinguishes host files from running containers. Container files support selection, mkdir, rename and guarded delete, with explicit device/container labels in progress.
- Rebuild a devcontainer through its workspace configuration using an explicit confirmation or `devcontainer-rebuild --yes`. Workspace identity must match exactly one container; errors refresh discovery without replaying the rebuild.

### Fixes
- Pin queued file actions to their original host/container scope across navigation. Older helpers reject scoped transfer requests rather than applying container paths to host files.
- Verify complete destination trees before cross-container cut cleanup. Cleanup removes only verified entries and empty directories, retaining new or changed source files.

## v0.1.45

### Improvements
- Floating notifications ease in and out with a short slide and fade; fast redraws run only during transitions.

### Fixes
- Unreachable-device errors notify once per device until a fresh successful helper response, while status and retry behavior remain available.
- Use lowercase `container` for unclassified container session labels, matching `devcontainer` and host session types.

## v0.1.44

### Improvements
- Notices appear in a floating bottom-right panel with colored Info, Success, Warning and Error types. Panels wrap text, preserve search/footer/transfer controls and disappear after five seconds.

### Fixes
- Use lowercase `devcontainer` session labels alongside `shell`, `claude` and `codex`.

## v0.1.43

### Fixes
- Footer notifications align to the bottom right and disappear after five seconds, including while idle. New messages restart the timeout.

## v0.1.42

### Features
- Shared navigation, panel adapters, scoped filters, session menus and rendering live in focused UI modules. Menu labels, shortcuts and dispatch use one choice definition.
- Devcontainer terminals have explicit labels in session rows, selected details and the shell chooser. Classification uses discovery evidence; ordinary Docker and unknown containers remain distinct.
- Keep current status concise and archive historical milestone notes; add a short developer map of the UI modules.

## v0.1.41

### Features
- Container menus offer only shells and read-only files, with no Claude/Codex or agent-permission choices.
- Devices, sessions, containers, network, transfers and dialogs share `gg`, `G`, numbered row jumps and counted movement. Device and transfer lists support focused search/filter without starting work.

### Fixes
- Session chooser buttons use rounded, separated borders and a bold selection marker like the confirmation dialogs. Underlines no longer run through all three button rows; full labels remain visible at narrow and standard widths.

## v0.1.40

v0.1.39 was left unpublished after a final host/container resume-scope correction.

### Features
- Host launches do not offer to resume a container session with the same path/provider.
- Enter in a transfer destination pane confirms sending into its displayed folder; h/l still navigates. Pending submissions are protected against duplicates, and the sidebar animates active transfer progress.
- `w` on a highlighted session opens a read-only watch attachment.
- Remove the redundant SSH terminal action from the sidebar and Ctrl+P; New shell remains the UI terminal entry point.
- `n` opens a compact session chooser on the highlighted device, with c/x/s/d shortcuts and h/l or Left/Right navigation. Device launches use home; Files launches retain the current folder.
- Containers tree for one device or All devices, with evidence-based devcontainer identification, clean Docker names and individual lifecycle actions. Ordinary service containers remain inspection-only until explicitly enabled.
- Persistent container shells, Codex and Claude sessions with Default/YOLO permissions, native terminal passthrough and scoped process termination. Host tmux means containers do not need inner tmux.
- Read-only container files reuse the browser and previews with explicit scope; host transfers and file mutations are disabled there.
- Explicit Dev Containers CLI workspace startup with configuration/hooks confirmation; existing CLI exec honors environment probing and remoteEnv, with Docker fallback for existing containers.
- Engine/start/user identity checks reject stale container scopes. Helpers use private versioned tooling without package/network changes.

### Documentation
- README focuses on getting started and everyday controls; installation, detailed user guidance, development and container details live in linked documents.

## v0.1.38

### Features
- New Codex and Claude sessions offer Default or YOLO permissions for that launch; CLI supports `cx new --yolo`. Codex YOLO bypasses approvals and sandboxing, while Claude bypasses permission prompts. Host provider settings remain unchanged.
- `d` in Sessions now confirms and stops CX-managed Codex and Claude sessions as well as shells, preserving ownership, runtime and single-pane checks. Remote agent-stop and YOLO require updated helpers.

## v0.1.37

### Features
- After authenticated connection, check Linux x86_64/ARM64 support and install the helper plus a signed, fully static portable tmux when absent; preserve existing tools without OS package or profile changes.
- Mask passwords in a compact prompt with F2 and clickable Show/Hide controls.

### Interface
- Stack parent folders, the highlighted current folder and selected item in the Selected side panel; remove duplicate workspace path headers and follow the active split pane.

### Fixes
- Include the robot shell attachment, public-key-first authentication, viewer-mediated updates and device-aware Ctrl+P improvements from the unpublished v0.1.34 candidate.
- Confirm completed command output through the owned tmux server before the single-key detach regression check, drain terminal cleanup output, and create the enrollment fixture directory on clean runners.

### Limitations
- Background robot updates require a connected viewer. Optional PDF converters and coding-agent runtimes remain separate. Internet sharing and full reverse SSH enforcement remain inactive.


## v0.1.34

### Features
- Update enrolled SSH hosts through a viewer-verified, architecture-specific release when the host cannot download it; add `cx update --device NAME` for an explicit check and installation.
- Make Ctrl+P a device chooser for Files, Network, Sessions, SSH terminal and session creation, with visible scope for contextual file/command actions.

### Fixes
- Start a shell directly with `n`, using home from Sessions/Network or the current folder from Files.
- Prefer working public keys before GSSAPI, fixing slow robot authentication that exceeded the helper deadline while retaining authentication fallbacks.
- Fall back to an installed standard terminal type when a robot lacks the viewer's vendor terminfo, preventing immediate shell-attachment failure.

### Interface
- Keep discovered-neighbor connection primary and retain manual **Add by SSH address** in Ctrl+P, with an explicit gateway choice.

### Limitations
- Background remote updates require a connected CX viewer; optional tmux/Poppler/agent runtimes are installed separately. Robot internet sharing remains inactive.

## v0.1.33

### Fixes
- Recover update checks from GitHub API rate limits through the official stable-release page, retaining pinned signature verification and downgrade protection.
- Focus each device's Network view on its own neighbors and interface/route details; All devices keeps the combined expandable hierarchy and refreshes all observers.

## v0.1.32

### Features
- Resolve bounded neighbor hostname hints on each observing host and display names beside addresses in Network.
- Add an ordinary SSH terminal from device actions and All devices, without requiring tmux; exit returns to CX.
- Support minimal immutable-host enrollment and directed access/sharing previews. Sharing previews do not activate networking.

### Fixes
- Preserve the selected gateway for manual Add device; match `.local` names against the observer's discovered addresses before authenticated enrollment.
- Label newly enrolled user@address devices with their authenticated hostname, and explain missing tmux when persistent sessions are unavailable.

### Limitations
- Discovered names are hints, not SSH identity evidence. Reverse lookup may be unavailable; IPv6 link-local name resolution/enrollment needs interface support.
- Robot internet sharing and complete reverse SSH enforcement remain unfinished and inactive.

## v0.1.31

### Fixes
- Normalize tightly duplicated physical wheel reports so a detent moves one row in Sessions and Files; preserve separate fast ticks and keyboard input.
- Keep wheel navigation adjacent while typing file search: visit unmatched rows instead of jumping or wrapping between matches. Preserve the query and keyboard match navigation.

## v0.1.30

### Features
- Click a file preview to focus it for keyboard scrolling. Text previews show a compact scroll percentage on the frame; wheel scrolling follows the panel under the pointer.

### Fixes
- Keep PDF previews in the full workspace while another page loads, preserving panel geometry and mouse targets. Page Up/Down continues to move one page during loading rather than jumping ten pages; rapid navigation coalesces without extra concurrent requests.

## v0.1.29

### Fixes
- Handle mouse-wheel navigation directly throughout the workspace, moving one item per reported step in session and file lists. Avoid terminal fallback that turns a wheel step into accelerated arrow-key bursts. Scroll the panel under the pointer, retain modal/input ownership, and release mouse handling when entering native shells and agents.

## v0.1.28

### Fixes
- Keep external tmux discovery and attachment on the default personal server, even when cx is started inside a managed terminal. Prevent duplicate managed rows with external identities and broken remote attachment paths. Preserve real personal sessions and running processes.

## v0.1.27

### Features
- Follow relative Markdown file links on the document’s device, including percent-encoded filenames and heading anchors. Escape returns through a bounded preview history without losing the file-tree selection. Links ending in / open the linked directory.
- Open links directly with Shift+click where the terminal forwards it, or Ctrl+click. Ordinary click and o inspect the exact destination, including repeated labels and table links. Foot reserves Shift for selection by default.

### Fixes
- Show fresh unnamed shells as New shell rather than a generated identifier or directory label. Switch to the observed running/last-command label as work starts. Recognize older generated shell names by their creation-key hash; preserve custom names and the same running terminal.
- Avoid updater failures when a reused filesystem inode collides with an older rollback filename. Allocate a fresh private backup name without overwriting existing backups, retaining atomic replacement and rollback safeguards.

## v0.1.26

### Features
- Format inline Markdown links with underlined labels. Click a link or press o to inspect destinations; Enter opens an HTTP/HTTPS link in the viewing machine's browser. Previewing never opens links or downloads content automatically.
- Add a bundled, lightweight math preview for common TeX expressions: Greek symbols, operators, scripts, fractions and square roots in dollar or bracket delimiters. Unsupported TeX stays visible as source; no interpreters, TeX packages or network resources are required. ASCII mode retains math source.

### Fixes
- Align soft-wrapped and explicit list continuation lines under their text, including ordered and nested lists. Preserve Unicode widths, code blocks, search and terminal-default backgrounds.
- Keep clickable link regions separate from search underlines so clicking an ordinary search match cannot open an unrelated link picker.

## v0.1.25

### Features
- Render Markdown tables with aligned columns, bold headers and left/right/center alignment. Preserve Unicode widths, escaped pipes and inline formatting; switch to labeled wrapping rows when the preview is too narrow.
- Give fenced code blocks clear boundaries and language labels. Use bundled language grammars for syntax highlighting—including common aliases such as python3, JavaScript and shell—without executing code or requiring an interpreter on the device.

### Fixes
- Match the correct fence marker and length, keeping table-like text inside code literal. Unknown languages and exhausted highlighting budgets degrade to plain code.
- Cache table layout by viewport/source revision and keep preview search highlights, n/N navigation and scrolling aligned after resizing. Preserve terminal-default backgrounds and monochrome formatting.

## v0.1.24

### Fixes
- Render underscore Markdown emphasis in previews: `_text_` uses italics and `__text__` uses bold. Keep underscores within identifiers, escaped delimiters and inline code literal. Preserve Markdown emphasis in monochrome previews without adding colors.

## v0.1.23

### Features
- Find files with `/` without hiding or reordering directory entries. Highlight every exact or fuzzy filename match, then use `n`/`N` to move between matching files with wraparound, just like text-preview search.
- Enter finishes the file search rather than opening a file; Escape clears it. Keep `f` as the separate name filter and show the current match count in the browser footer.

## v0.1.22

### Fixes
- Give blank saved session labels the same native-name lookup and execution-folder fallback as automatically named sessions. Preserve explicit names; Codex conversations without a verified terminal identity still use a folder fallback.
- Reload managed tmux bindings when starting work on an empty persistent server, so updates take effect even after the last session closes. Use a readiness check compatible with older tmux.

## v0.1.21

### Fixes
- Scroll fullscreen Codex’s native chat when its startup mouse policy is stale, instead of opening tmux’s empty `0/0` history buffer. Preserve the running process, draft and application-owned keyboard controls.
- Reject wheel fallback for read-only viewers and foreground editors, and keep empty history from opening copy mode. Older tmux retains guarded ordinary scrollback and requested native mouse input.

## v0.1.20

### Features
- Search text previews live with `/`, case-insensitive fuzzy matching and highlighted results. Use `n`/`N` for next/previous matches without losing the file-tree selection or search.

### Fixes
- Keep `G` within the preview’s actual wrapped content instead of scrolling into a blank screen. Preserve the selected search result after resizing or refreshing a preview.
- Keep loading placeholders out of content search and preserve terminal-default backgrounds in highlighted previews.

## v0.1.19

### Features
- Identify automatically named shells by the running executable or last observed command, without recording command arguments or history. New managed Fish shells capture short commands too.
- Display Claude’s native session name when its PID/start record matches. Read Codex’s native name only when a unique writable session handle establishes the conversation identity; unsupported daemon-backed sessions retain their folder label.

### Fixes
- Preserve explicitly named sessions and stable session ordering as labels change. Resolve default labels on the execution host after canonicalizing the selected folder.
- Remember shell commands with session-scoped metadata compatible with older tmux, including Rocky Linux 8.

## v0.1.18

### Fixes
- Join the file cursor to its filename with a compact chevron and restrained ANSI accent. Remove the detached rail and reversed white highlight, preserving the terminal’s default background.
- Keep inactive pane cursors dim, selection/copy/cut marks separate, and selected file sizes readable. The slimmer gutter gives long filenames more room.

## v0.1.17

### Features
- Organize Network beneath collapsible observing devices. Every LAN address remains a distinct entry; Right/l expands and Left/h folds or returns to its observer. Expansion is remembered within cx.
- Show the selected neighbor’s discovery path, observing interface and saved SSH jumps in selected-item details; J/K scroll longer paths. Known names require fresh, unique authenticated address/link metadata.
- Show the running cx version in the Update menu and update status messages.

### Fixes
- Remove Network’s Evidence column and distinguish an observer’s neighbors from its own addresses.
- Align notices with the footer and shorten update messages for narrow terminals, preserving transparent backgrounds and ASCII output.

## v0.1.16

### Features
- Automatically check SSH ports for current LAN neighbors in the background, with bounded concurrency and 90-second evidence reuse. No scans or authentication prompts run during observation.
- Open account entry directly when selecting a LAN neighbor, preserving its address and gateway for enrollment.

### Fixes
- Remove the manual Check SSH action and avoid per-neighbor checking notifications.
- Preserve port evidence across network refreshes and discard queued checks when their scope changes.

## v0.1.15

### Features
- Network keeps enrolled devices visible alongside separately scoped LAN neighbors, with recent viewer-authenticated access evidence and direct session/files actions.
- Enroll discovered neighbors through their observing device using compact account entry and masked OpenSSH password, key-unlock or verification prompts. New host keys require explicit review of a complete, scrollable fingerprint.
- Reuse a private authenticated SSH connection for ten idle minutes without saving passwords; background checks never open credential dialogs.

### Fixes
- Keep project directory headings directly above their sessions, with spacing before the next group.
- Slim the Add device panel and remove redundant confirmation/cancellation instructions.
- Show a refused neighbor SSH connection as port closed rather than unknown.
- Bound enrollment I/O and cancellation; keep passwords outside application caches, logs, process arguments and environment.

## v0.1.14

### Features
- Replace the file browser's row-wide underline with a solid cursor rail and padded filename highlight. The highlight follows the terminal's default color pair for light/dark contrast; inactive panes retain a quieter cursor, and file marks/copy/cut indicators keep their own colors.

### Fixes
- Recognize an existing user-installed tmux when a noninteractive SSH login omits its directory from PATH.

## v0.1.13

### Features
- Browse neighbors observed across enrolled devices in Network, with device/interface context, explicit SSH-port checks and honest unknown authentication/internet states.
- Connect through an enrolled device using OpenSSH jump routes, including sessions, commands, browsing and transfers. ARM64 enrollment obtains the matching signed helper on the viewer, so an offline robot does not need to download it.
- Give panels consistent padding and focus treatment, aligned colored footer keys, selected session/file context, and useful New session/Add device empty states.

### Fixes
- Keep the selected-file sidebar tied to the focused pane and show the transfer destination separately.
- Prevent background jump-host authentication prompts and reject enrollment that would silently retarget a device on an overlapping network.
- Preserve numeric address ordering and age stale SSH-port observations to unknown.

## v0.1.12

### Fixes
- Reconnect once after an unexpectedly closed session-metadata channel when listing sessions or checking a device. Mutations are never replayed.
- Distinguish SSH connectivity, authentication, host-key and missing-helper failures without displaying raw shell output. Cached sessions show the failure and a Refresh remedy; connectivity loss does not imply a stopped terminal.

## v0.1.11

### Features
- Copy remote application yanks and managed terminal selections into the viewing terminal's clipboard through OSC 52, without forwarding display sockets.
- Select scrollback with Space and arrows, then Enter/y to copy, or drag to select and copy. The managed status strip identifies copy controls.

### Fixes
- Preserve tmux's built-in terminal overrides and keep clipboard configuration idempotent on repeated attachment.

## v0.1.10

### Features
- Automatic signed updates now include accessed SSH devices. Updates run in the background; metadata reconnects after a verified helper upgrade while terminals and transfer jobs keep running.
- Browse multiple PDF pages with j/k, arrows, Page Up/Down or the mouse wheel. A page counter identifies the current page; each wheel step changes one page.

### Fixes
- Coalesce rapid PDF navigation into one pending render and retain the visible page when rendering fails. Escape rejects late responses and restores the browser.
- Show loading and invalid-bitmap states explicitly instead of leaving an empty preview.
- Check for pdfinfo alongside the PDF renderer during installation and enrollment; both come from Poppler.

## v0.1.9

### Features
- Larger image and first-page PDF previews with optional native terminal graphics and a colored-cell fallback.
- Bundled language grammars for source previews and Markdown code fences; highlighting follows the terminal palette and never executes source code.
- Separate outlined confirmation buttons for deleting files and stopping managed shells.

### Fixes
- Decode images and prepare syntax highlighting outside the input/render loop.
- Clear fallback image cells when native graphics finish encoding, and clean up graphics on resize, overlays and return.

## v0.1.8

### Fixes
- Distinguish an absent tmux session server from a failed tmux runtime. Missing libraries or wrapper failures now report an error rather than incorrectly claiming a session has already stopped.

## v0.1.7

### Features
- Preview images, the first page of PDFs, Markdown, source files, tar/tar.gz archive entries and WAV metadata directly in Files. Image/PDF previews use colored terminal cells in this version.
- Press `d` in Sessions to stop a cx-managed shell with confirmation. Agent and external sessions remain protected.
- Compact, consistent confirmation dialogs for file deletion and shell stopping.

### Fixes
- Normalize text line endings and escape terminal control characters in previews; bound image decoding, archive listing and PDF conversion.
- Install PDF, network and terminfo utilities through apt, dnf/yum or pacman. Enrollment checks required host utilities before replacing the helper.
- Resolve detected foreground-agent workspace directories independently of the original shell directory, refreshing generated session titles and attached location labels.

## v0.1.6

### Fixes
- Mouse-wheel scrolling in managed terminals reviews retained conversation scrollback rather than sending prompt-history arrow keys. Applications requesting native mouse input keep those events.
- Escape returns from tmux scrollback to the live terminal; the status strip identifies scrollback mode. Existing running applications may need to refresh their terminal mouse mode.

## v0.1.5

### Fixes
- Preserve viewer colors and default backgrounds when running native folder commands with Fish. Theme loading can respect the cx-only viewer-theme guard without changing ordinary terminals.
- Remove the redundant Run command action from the Actions menu; `:` remains the current-folder command shortcut.

## v0.1.4

### Fixes
- Retain the remote helper’s negotiated native-command capability so supported devices can execute `:` commands instead of incorrectly requesting an upgrade.
- Restore predictable interrupt and suspend behavior for POSIX shell command profiles.

## v0.1.3

### Features
- Simplify Actions around workspace navigation, New session, transfers and enrollment; file operations stay with the file browser.
- Press `n` to create and enter a session on the current device in the focused folder, using its available providers.
- Press `:` to run a command in that device’s current folder with native terminal input and execution-location context. Enter returns after completion; suspended commands offer Resume or Cancel.

### Fixes
- Preserve interactive Fish/Bash/Zsh wrappers and shell builtins while controlling foreground command process groups.
- Guard concurrent session creation, retain long-command editing and scrolling, and distinguish stale capability evidence from unsupported helpers.

## v0.1.2

### Features
- Recognize foreground Claude or Codex launched manually inside a persistent shell, updating its provider badge while retaining the same attachable terminal. Returning to the shell restores the Shell badge.

### Fixes
- Support older tmux color capabilities and bounded capability checks, including user wrappers.
- Parse compact GitHub release responses correctly during bootstrap installation.

## v0.1.1

### Features
- First published stable-channel build of the fleet workspace: persistent shells and Claude/Codex sessions over SSH, shared device context, Files and read-only network observations.
- Create and enter sessions from the selected host/folder; provider choices reflect detected host runtimes. A client-only device can view and enter work on another host.
- Unified file selection, rename, copy/cut/paste and destination-device browsing, with durable transfer jobs and incoming conflict renaming by default.
- Single `Ctrl+]` return from managed terminals while sessions continue running.
- Signed stable-release updates with background checks, idle relaunch and graceful offline preservation.
- User-level bootstrap with signature verification, static Linux x86_64/ARM64 binaries and Ubuntu, Rocky Linux (RHEL-compatible) and Arch installation checks.

### Limits
- Network sharing, full trust/revocation and owned-resource uninstall are not release-ready. Network observation is available; see VALIDATION.md for actual acceptance evidence.
