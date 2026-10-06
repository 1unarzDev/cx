# Validation record

Each result distinguishes implemented code, fixture tests, PTY tests and physical hardware. No acceptance gate is complete merely because its code compiles. Sensitive local evidence is outside tracked files.

| Gate | Status | Evidence / limit |
|---|---|---|
| A1 | PASS | 2026-10-05 bounded BatchMode SSH, 6-second connection timeout: innovation→tranquility peace, innovation→server lunarz; tranquility→innovation lunarz, tranquility→server lunarz; server→innovation lunarz, server→tranquility peace. Server peace alias also resolves tranquility. Innovation self-alias fails host-key verification; not one of six directed fleet edges. |
| A2 | BLOCKED | No authorized repository/release; bootstrap pipeline must be prepared and tested as a fixture, not advertised as public installation. |
| A3–A16 | BLOCKED | Not yet exercised; implementation and tests in progress. |

Baseline inventory at f30bfa2/8619484: innovation tmux 3.7c, Codex 0.160.1, Claude 2.1.290; tranquility tmux initially absent/npm installed; server Ubuntu 24.04 tmux 3.4, codex/claude absent. Tranquility dependency extracted as user from tmux 3.7_c-1 and libutempter 1.2.3-1 packages, both verified by pacman-key with fully trusted Arch packaging signatures. No system package or privilege mutation.

Provider observation: configured Codex provider codex-lb uses HTTPS /backend-api/codex; codex-lb Docker container runs on host network. Unauthenticated /health gives HTTP 200 (transport evidence only). Claude /health returns HTTP 302; this does not establish provider readiness. No inference health requests made.

## Folder navigation and managed shell return (2026-10-05)

Working tree based on 42315b2, innovation, tmux 3.7c. `cargo test --locked ui::tests::` first reproduced Right/l not entering a folder and Shift+Tab not reversing focus (11 pass, 2 fail). Both regressions pass after fixing the input dispatch. `cargo test --locked` passes. `python3 tests/shell_return_pty.py` uses isolated XDG state and an owned disposable tmux server; it verifies native shell attachment, complete return hint, Ctrl+] Space restoring Work and selection, continued shell lifetime, and restored termios. PASS for allocated PTY; user subsequently confirmed shell return works. Private assertion-only evidence: local-evidence/shell-return-navigation.json.

## SSH Unicode regression (2026-10-05)

Innovation tmux 3.7c; tranquility noninteractive `locale charmap` reports ANSI_X3.4-1968. `python3 tests/shell_return_pty.py` now runs with LANG=C/LC_ALL=C and injects only synthetic symbols and Japanese text into a disposable shell. UTF-8 output assertion failed before adding tmux -u and passed afterward; attach/return, shell survival, and termios restoration also pass. This verifies transport output, not physical font glyph coverage. No global locale, SSH configuration, or external tmux server configuration changed.

Deployment: updated innovation and tranquility binaries using atomic local install and serialized `cx add tranquility`. Native PTY attach/return to an owned disposable remote shell passes (peace@tranquility visible); fixture session removed afterward under the remote maintenance lock. Attempted prior cx-fish-codex session was absent, so Codex-specific visual rendering was not retested. Foot configured Nerd Font resolves to installed JetBrainsMonoNerdFont-Regular.ttf; font config unchanged.

## Prompt italics, one-key return, internet checks (2026-10-06)

Working tree after df5e84f. Found upstream tmux 3.7 tty_set_italics chooses standout instead of sitm when default-terminal starts screen. Native synthetic-italic output test first failed (pane metadata italic but emitted SGR 7), then passed with available tmux-256color terminfo. `python3 tests/shell_return_pty.py` additionally verifies single Ctrl+] returns, restored selection/termios, and live shell survival. Deployed on innovation and tranquility, reloaded only owned tmux config through disposable managed session creation, removed those sessions; unrelated sessions unchanged. Actual prompt screenshots after fix await user confirmation.

`cx network` and `cx network --device tranquility` produced positive replies for 1.1.1.1 and 8.8.8.8. Private evidence local-evidence/internet-{innovation,tranquility}.json. Evidence covers public IPv4 ICMP via existing routing/VPN policy, not DNS, HTTPS, provider availability or downstream devices. Backend fixture tests cover cache freshness, concurrent checks and unavailable/failed probes. UI integration remains in progress.
