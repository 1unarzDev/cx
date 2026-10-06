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

User physically confirmed both Starship block-background fix and single Ctrl+] return work. Independent validation /tmp/cx-independent-tmux-df5e84f.json additionally proves bracketed paste containing Ctrl+] stays application input, and detach preserves the pane. Reviewer MEDIUM stalled infocmp finding fixed with 200ms per-candidate deadline; regression asserts timeout under 1s and process reaping. Root integration recheck pending next build.

## Capability-aware launch and workspace feedback (2026-10-06)

Code commit `9263cb1`, main, innovation Linux x86_64; deployed identical optimized binary on innovation, tranquility and verybeautifulserver (SHA-256 identities recorded privately). Runtime versions: innovation Fish Claude 2.1.290/Codex 0.160.1, tranquility Fish Codex 0.160.1, server Bash without AI agents. Tranquility's Claude launcher exists but exits unsuccessfully: availability correctly follows launchability, not file presence. No provider prompts or paid inference performed by these checks.

| Result | Actual command/scenario | Evidence |
|---|---|---|
| PASS | `cargo test --quiet`: 106 passed, one existing live test ignored; includes failed/timed-out wrappers, early-exit descendant cleanup, monitor mode, stale selection, job reorder/cancel and Network stale completion | `/tmp/cx-provider-tests.log` |
| PASS | `cargo build --release`; atomic local install and serialized `cx add tranquility`, `cx add verybeautifulserver`; all installed SHA-256 values match | `local-evidence/provider-install-identities.json`, `/tmp/cx-provider-build.log` |
| PASS | Real helper Info through each host's selected profile: innovation Claude+Codex, tranquility Codex, server neither | `local-evidence/provider-capabilities.json` |
| PASS | `uv run --with pyte python tests/provider_choices_pty.py target/release/cx`: actual integrated New session picker on all three hosts gives Shell/Claude/Codex, Shell/Codex, Shell respectively; zero agents started | `local-evidence/provider-picker/results.json`, sanitized text captures alongside |
| PASS | `CX_TEST_TERM=foot python3 tests/shell_return_pty.py`: isolated native shell, Unicode in ASCII SSH locale, emitted italics rather than reverse, single-key return, same selection/process lifetime and restored termios | assertion-only tool output; fixture cleans its own socket |
| PASS | Actual Fish post-init output on both Arch hosts: normal palette initialization precedes three managed resets, which precede execution; old exact owned guard removed with private backup | `local-evidence/viewer-theme-post-init.json` |
| PASS | Independent capability checks, expired-selection and monitor-mode regression rechecks | `/tmp/cx-independent-provider-hosts.json`, `/tmp/cx-independent-expiry-fixed-test.log`, `/tmp/cx-independent-monitor-fixed-test.log` |
| PASS | User physically tried the installed new tranquility Shell and confirmed “Viewer colors and background look right” | user response 2026-10-06; emulator unspecified, not proof of every emulator/theme combination |

Review corrections: replaced optional/racy Fish configuration rewriting with fixed init argv; bounded terminfo lookup; preserved selected job identity through progress reorder; released stale Network loading; invalidated obsolete browser/preview generations; refused expired-agent-to-shell substitution; disabled probe monitor mode and observed exits with `waitid(WNOWAIT)` before group cleanup/reaping. Early-exit child cleanup has successful/failed wrapper regressions. Runtime availability does not prove provider authentication or route health.

Remaining known transfer limitations: retry after viewer restart lacks durable spec routing; failed detached-worker launch can retain queued metadata. Sharing worker `df75d04` remains separate and unintegrated: namespace policy fixtures passed, NM end-to-end fixture blocked by missing dnsmasq/dhclient; privileged packaging and physical acceptance unresolved. Public release/bootstrap, full uninstall, outer-tmux handoff and enrollment trust/revocation remain incomplete.
