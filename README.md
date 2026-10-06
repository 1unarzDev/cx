# cx

A terminal workspace for device-local shells, Claude/Codex sessions, files and transfers. Development is in progress; see [STATUS.md](STATUS.md) and [VALIDATION.md](VALIDATION.md) for verified behavior and gaps.

On the current enrolled hosts:

```sh
~/.local/bin/cx
~/.local/bin/cx add tranquility
```

`add` reuses SSH configuration and verified host keys and installs this binary's helper. Current enrollment supports Linux x86_64. It does not install AI agents or copy provider credentials.

- Ctrl+P opens actions. In Work, New session chooses device, available provider, then folder; Start here launches and enters it. In Files, New session chooses a provider and immediately starts/enters it in that browser’s host and current folder. Returning restores the browser. Availability uses bounded version checks through that device's selected shell profile, independently of provider authentication.
- `/` searches live. Enter opens a session, directory or file preview. Files uses Left/Right or h/l for parent/child navigation.
- Tab/Shift+Tab cycle focus; Ctrl+arrows or Ctrl+h/j/k/l select panels directionally. The bottom Focus label and marked heading identify the active panel.
- In managed native terminals, Ctrl+] returns to cx. Native application input, including Ctrl+C, remains its own. External tmux sessions keep their existing bindings.
- Files: `.` toggles hidden files; Space selects and advances; `v` selects a range; `u` clears selection. Selected rows have markers and the heading shows their count. `/` searches live; `f` filters names. Escape ends visual mode, clears selection/filter, then returns to Work.
- `r` renames without overwriting; `d` opens a permanent, recursive delete confirmation with Cancel selected. There is no undo. Enter still enters directories or previews files.
- `c` (or `y`) copies selected files, or the hovered file if nothing is selected; `x` cuts. Navigate to the destination and press `p` to paste into the **focused folder**. COPY/CUT markers identify the clipboard; `Y` clears it. Switching devices while Files is open retargets only the focused location and remembers its previous folder; the clipboard and other pane retain their own hosts.
- `t` opens **Transfer to…**: choose a device, browse its destination folder, then press `p` to Paste here. With no clipboard it captures the selected/hovered files for copy; an existing clipboard keeps its copy/cut intent. The second location uses the same browser.
- Incoming conflicts rename by default; actions offer explicit skip/overwrite. `T` opens transfer progress/results; the compact drawer stays alongside browsing. Details show both endpoints and actual resulting names. In Transfers, `r` retries a saved job, `c` cancels, Tab focuses details, and PgUp/PgDn scroll long paths/errors. Detached jobs survive closing the viewer; retry works from a new viewer.
- Work → Watch · read-only opens a terminal without sending input. Opening a session normally gives it input. This is a usability control, not isolation between processes sharing an account.
- Same-host cuts use atomic no-overwrite moves, including directories and symlinks. Cross-host cuts support regular files only: source removal follows destination content/identity verification. Cross-host directory/symlink cuts are refused before copying; copy is supported. Cross-filesystem moves preserve the source on failure. Keep sources/destinations stable; transfers are not snapshots of growing files.
- Network shows execution-host connectivity observations. ICMP replies establish limited IPv4 reachability, not DNS/HTTPS/provider health. Transactional sharing is under development.

New managed Fish sessions restore the viewer's palette after normal Fish initialization. Tmux uses terminal-default foreground/background. Applications that explicitly paint backgrounds or RGB colors retain those styles; cx cannot supply missing fonts or make all application panels transparent.

For a local source build, use `cargo build --release`; run `./target/release/cx`. There is no published release or working public curl installation command yet. The bootstrap and release workflow are prepared, but require a real published release before public installation can be verified.

cx checks stable tagged releases from `1unarzDev/cx` asynchronously at startup and hourly. It verifies GitHub artifact provenance against that repository's release workflow before replacing the installed binary, then restores the workspace when idle. This currently supports Linux x86_64 and requires system curl and GitHub CLI for verification; it does not pull or build arbitrary new commits. Sessions and detached jobs continue independently of the viewer.

Without internet, automatic checks stay quiet and keep the current version. Local work and reachable SSH devices remain available; SSH connectivity and model-route availability are separate. Use Ctrl+P → Update cx or `cx update --check` to retry explicitly after reconnecting. Failed checks are cached for five minutes across viewer starts; an explicit check bypasses that backoff. A failed bootstrap download also leaves an existing installation unchanged. Updates cannot yet be exercised against a real signed release because none is published.

To stop the viewer, press Ctrl+C in cx. This preserves managed sessions and detached transfer jobs. Removing `~/.local/bin/cx` removes the installed binary; do this only when no transfer job needs to invoke the helper. Preserve `~/.local/state/cx` while sessions/jobs exist. Full owned-resource uninstall is not implemented. Ordinary shell configuration is unchanged by launch; private backups from removal of the temporary owned Fish guard are retained under each Arch account's cx state directory.
