# Release notes

Each stable release describes user-visible changes here before tagging. GitHub release descriptions are generated from the matching version section; missing notes block publication. Unreleased work is not included in published descriptions.

## Unreleased

No changes queued.

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
