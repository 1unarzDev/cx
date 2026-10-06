# cx

A terminal workspace for SSH devices, persistent shells, Claude/Codex sessions and files. Runs on **Linux x86_64 and ARM64**; Ubuntu, RHEL-compatible and Arch distributions share static binaries. No AI runtime is required on viewing devices.

```sh
curl -fsSL https://raw.githubusercontent.com/1unarzDev/cx/main/install.sh | sh
cx
cx add user@host                 # existing SSH alias also works
```

Install as your ordinary user. Missing dependencies require an explicit package-manager confirmation through your terminal; no Rust toolchain is needed. The installer verifies downloads using the pinned [release public key](release-key.pem), stages privately and replaces the binary atomically. Re-running is safe; failed/offline downloads preserve an existing installation. After updating with the installer, reopen an already-running viewer to use the new binary; sessions keep running. The script itself is trusted from GitHub over HTTPS. Builds also carry GitHub provenance.

**Requirements:** Linux; guarded filesystem mutations need kernel 5.6+ and fail safely when unavailable. The installer offers OpenSSH, tmux, curl, OpenSSL, Poppler (PDF previews), iproute2, ping and ncurses/terminfo utilities through apt, dnf/yum or pacman. Image decoding, Markdown and tar/tar.gz previews are bundled; no Yazi or graphics protocol is required. NetworkManager and AI runtimes are optional and are not installed by cx. Transfers use supported detached processes; a working systemd user manager is needed when launched from a service cgroup. Fish, Bash and Zsh work; terminal colors/backgrounds remain yours. Agents and their credentials stay on the execution host. Run the installer on a fresh execution host before `cx add`; enrollment checks its utilities before replacing the helper. Enrollment currently requires matching viewer/host architectures; install separately on a different architecture.

| Keys | Action |
|---|---|
| Arrows / `h j k l`, Enter, Escape | Navigate, open, back |
| `/`, Ctrl+P, `?` | Search, workspace actions, help |
| `n`, `:` | New session / run command on current device and folder |
| Tab / Shift+Tab, Ctrl+arrows | Change panel focus |
| Ctrl+] inside managed terminals | Return; session keeps running |
| Ctrl+C in cx | Exit viewer; work keeps running |
| Files: `.`, Space, `v`, `f` | Hidden files, select/advance, range, filter |
| Files: `gg`, `G`, `5j` / `5k` | First, last, counted movement |
| Files: `y`/`c`, `x`, `p` | Copy, cut, paste into focused folder |
| Files: `t`, `T` | Choose destination device/folder, transfer results |
| Files: `r`, `d` | Rename, confirm permanent deletion (`y` / `n`) |
| Files: `M`, `o` | New folder, cycle copy conflict policy |

Select files → `t` → choose device → browse folder → `p`. Or copy/cut, switch devices, and paste. Incoming name conflicts rename by default; overwrite is explicit. Cross-host cuts support regular files; copy supports directories. Delete has no undo. Transfers are not snapshots of growing files.

Sessions lists persistent work. New session is the single launch action; file operations live on browser keys. Press `n` in Files to start in that folder. `:` opens a command prompt pinned to the execution host/folder and runs it in the native terminal; Ctrl+C interrupts the command; Ctrl+Z offers Resume/Cancel; Enter returns after completion. Fish/Bash/Zsh load interactive wrappers; sh/dash commands use login startup. Shell startup themes should respect `CX_VIEWER_THEME=1` during native commands; cx does not reset the viewing terminal’s palette. Commands are foreground work; cx does not save or retry their text, and do not survive disconnect as managed sessions do.

New sessions use the chosen execution host and folder. Agent choices reflect installed launchable runtimes. Watch opens a read-only terminal; ordinary opening gives input. Starting Claude/Codex manually inside a shell updates its badge while the agent owns the terminal; returning to the shell restores it. The original terminal remains attachable. In managed terminals, the wheel reviews retained scrollback unless the application requests native mouse input; Escape returns to the live terminal. Up/Down remain application keys. Hold Shift for terminal-native text selection when your emulator supports it. Existing personal tmux sessions retain their bindings.

Files: Enter previews text, Markdown/code, images and the first PDF page; tar/tar.gz archives list entries without extraction. Images/PDFs use a larger workspace and optional native graphics in detected compatible terminals (Foot Sixel tested; Kitty/Ghostty physical testing remains unavailable). Unknown terminals and nested tmux use colored-cell fallback; monochrome mode uses text. Translucent images fall back on Sixel to preserve the default background. Source files and Markdown code fences use bundled language grammars and your ANSI palette, without running code or installing interpreters. `j`/`k` scroll previews; Escape restores the browser selection. `d` deletes files after confirmation; in Work it stops only a cx-managed shell, with confirmation. Agent and external sessions are protected.

Updates check signed stable releases in the background, retain the running version when offline, and restore the viewer when idle. Run `cx update --check` to check explicitly. Sessions/jobs keep running independently.

Development is ongoing: Network observation works; sharing, full trust/revocation and full uninstall are not release-ready. See [validation](VALIDATION.md) for tested behavior and limits. Remove only `~/.local/bin/cx` when no active job needs its helper; preserve state and sessions. No public hostnames or mesh provider are required.

Build: `cargo build --release --locked`. Check: `cargo test --locked`. GitHub workers test builds and installer fixtures across distributions; RHEL itself requires access to a licensed environment, so Rocky Linux supplies the compatible CI fixture.

Release changes: [CHANGELOG.md](CHANGELOG.md). Before tagging a stable version, move its reviewed changes from Unreleased into an exact `vX.Y.Z` section; publication requires these notes.
