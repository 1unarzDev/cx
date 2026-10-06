# cx

A terminal workspace for device-local shells, Claude/Codex sessions, files and transfers. Development is in progress; see [STATUS.md](STATUS.md) and [VALIDATION.md](VALIDATION.md) for verified behavior and gaps.

On the current enrolled hosts:

```sh
~/.local/bin/cx
~/.local/bin/cx add tranquility
```

`add` reuses SSH configuration and verified host keys and installs this binary's helper. Current enrollment supports Linux x86_64. It does not install AI agents or copy provider credentials.

- Ctrl+P opens actions. New session chooses device, available provider, then folder; Start here launches. Availability uses bounded version checks through that device's selected shell profile, independently of provider authentication.
- `/` searches live. Enter opens a session, directory or file preview. Files uses Left/Right or h/l for parent/child navigation.
- Tab/Shift+Tab cycle focus; Ctrl+arrows or Ctrl+h/j/k/l select panels directionally. The bottom Focus label and marked heading identify the active panel.
- In managed native terminals, Ctrl+] returns to cx. Native application input, including Ctrl+C, remains its own. External tmux sessions keep their existing bindings.
- Files → Copy to… chooses the destination device and folder; Paste here submits the copy. Existing incoming names are renamed by default. Transfers shows durable jobs and cancellation. Retry from a newly opened viewer remains incomplete.
- Network shows execution-host connectivity observations. ICMP replies establish limited IPv4 reachability, not DNS/HTTPS/provider health. Transactional sharing is under development.

New managed Fish sessions restore the viewer's palette after normal Fish initialization. Tmux uses terminal-default foreground/background. Applications that explicitly paint backgrounds or RGB colors retain those styles; cx cannot supply missing fonts or make all application panels transparent.

For a local source build, use `cargo build --release`; run `./target/release/cx`. There is no published release or working public curl installation command yet. `install.sh` deliberately refuses without a real repository/version; do not treat it as a verified release installer.

To stop the viewer, press Ctrl+C in cx. This preserves managed sessions and detached transfer jobs. Removing `~/.local/bin/cx` removes the installed binary; do this only when no transfer job needs to invoke the helper. Preserve `~/.local/state/cx` while sessions/jobs exist. Full owned-resource uninstall is not implemented. Ordinary shell configuration is unchanged by launch; private backups from removal of the temporary owned Fish guard are retained under each Arch account's cx state directory.
