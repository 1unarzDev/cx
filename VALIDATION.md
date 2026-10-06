# Validation record

## Offline updater and workspace restoration (2026-10-06)

PASS — integrated `cargo test --locked --quiet`: 78 unit, 20 files/network, 16 session, 15 transfer and 15 update fixture tests; one session test ignored. Log `/tmp/cx-update-integrated-tests.log`. DNS/timeout fixtures preserve the binary, suppress repeated checks within backoff, and permit explicit recovery. Provenance in fixtures is synthetic, not live verification.

PASS — `python3 tests/install_fixture.py`: unavailable network preserves binary; trust-root mismatch and symlink target refused; lock inode preserved; concurrent installers serialized. PASS — `uv run --with pyte python tests/update_viewer_pty.py target/release/cx`: offline update action preserves navigation and termios; private restart snapshot restores host/folder/search/focus. Evidence `local-evidence/update-viewer-pty.json`.

PASS — optimized `HTTPS_PROXY=http://127.0.0.1:9 target/release/cx update --check`: exit 0, current version retained. Independent review found no critical/high issues; fixed dropped manual-result request during startup check. BLOCKED — actual public signed artifact/bootstrap/update: no published release and first publication requires authorization. Tests simulate unavailable update transport without disconnecting production WARP; physical outage behavior of every provider remains unverified.

Each result distinguishes implemented code, fixture tests, PTY tests and physical hardware. No acceptance gate is complete merely because its code compiles. Sensitive local evidence is outside tracked files.

| Gate | Status | Evidence / limit |
|---|---|---|
| A1 | PASS | 2026-10-05 bounded BatchMode SSH, 6-second connection timeout: innovation→tranquility peace, innovation→server lunarz; tranquility→innovation lunarz, tranquility→server lunarz; server→innovation lunarz, server→tranquility peace. Server peace alias also resolves tranquility. Innovation self-alias fails host-key verification; not one of six directed fleet edges. |
| A2 | BLOCKED | Origin exists, but no published release; bootstrap pipeline fixture validation does not establish working public installation. |
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

## Current-folder new session (2026-10-06)

Root follow-up to `9263cb1`: `cargo test --quiet` passes, including launch on browser host/folder plus unconditional native attachment after Create response. Updated `tests/workspace_live_jobs_pty.py` PASS on innovation allocated 120x40 PTY: Files action New session → Shell immediately enters an actual detached shell; tmux pane_current_path equals chosen destination directory; single Ctrl+] restores Files. Actual copy/integrity/conflict preservation remains passing. Evidence `local-evidence/current-folder-live-jobs/results.json`. This is shell/PTY proof, not a paid agent interaction or every physical emulator.

Final prior milestone full browser-flow fixtures also PASS at 80x24, 120x40, 48x24 (`local-evidence/provider-workspace-flows-final/results.json`), and actual copy/session fixture PASS (`local-evidence/provider-live-jobs-final/results.json`).

## Yazi-inspired browser and transfer ecosystem (2026-10-06)

Integration based on fb656b0, main, innovation Linux x86_64/tmux 3.7c; remote backend verybeautifulserver Ubuntu24.04/tmux3.4. Backend commits 9255cdd and fb656b0 integrated before UI; subsequent integration commit contains final code.

| Result | Actual command/scenario | Evidence |
|---|---|---|
| PASS | `cargo test --locked --quiet`: 159 passed, one existing live session test ignored; guarded mutations, clipboard/input ownership, batch errors, durable retry, long paths and delete Cancel | `/tmp/cx-browser-tests-final.log` |
| PASS | Optimized `cargo build --release --locked` | `/tmp/cx-browser-build-final.log` |
| PASS | `uv run --with pyte python tests/browser_ecosystem_pty.py target/release/cx`: actual disposable-files hidden/selection/range/copy/paste/rename/delete/cut-conflict/termios at120x40,80x24,48x24;48 uses ASCII/NO_COLOR | `local-evidence/browser-ecosystem/results.json`, sanitized captures alongside |
| PASS | `uv run --with pyte python tests/transfer_retry_pty.py target/release/cx`: real detached permission failure, visible wrapped error, new-viewer r retry uses existing durable key, integrity/completion/termios at48x24 | `local-evidence/browser-retry/results.json` |
| PASS | Existing workspace flow PTYs at3 sizes and actual current-folder Shell attachment/copy/default conflict rename | `local-evidence/browser-live-launch/results.json` |
| PASS | `python3 tests/browser_remote_live.py target/release/cx verybeautifulserver`: local to SSH copy/conflicts/Unicode/punctuation/integrity, SSH to local verified regular-file cut, remote directory move/rename/delete; server agent inventory unchanged; fixtures cleaned | `local-evidence/browser-remote-live/results.json` |
| PASS | Disposable user-service cgroup without launcher tools: source preserved and durable failed record instead of phantom queued job | `local-evidence/browser-launch-refusal.json` |

Independent reports: `/tmp/cx-file-transfer-audit.md`, `/tmp/cx-cut-integrated-rereview.md`, `/tmp/cx-browser-ux-review.md`, `/tmp/cx-browser-ux-rereview.md`. Fixed skipped-copy deletion risk, stale receipts, key collisions, lost retry state, phantom queued launches, late error/clipboard loss, clipped paths/errors/warnings, stale notices, narrow footer ownership and misleading delete Escape. No demonstrated unresolved critical/high finding within supported cut behavior. Reviewer full-copy repeat was blocked by its unavailable user-service launcher; root actual-copy PTYs passed independently.

Final independent remediation check PASS: delete Details Escape cancels without submitting; modal counts are suppressed; ASCII footer uses j/k. Root reran all159 tests after these fixes. Final current-folder live PTY evidence: `local-evidence/browser-live-launch-final/results.json`.

Interaction reference inspected: installed `/usr/bin/yazi` keymap overrides and upstream `https://raw.githubusercontent.com/sxyazi/yazi/main/yazi-config/preset/keymap-default.toml` (bounded download, private copy `/tmp/cx-yazi-keymap-reference.toml`). Shared vocabulary: h/l parent/child, Space toggle+advance, visual selection, y/x/p clipboard, Y clear and layered Escape. cx retains requested c copy and d confirmed permanent deletion; Yazi's trash behavior is not promised. Yazi is not a runtime dependency.

Limitations: cross-host directory/symlink cut refused; same-host cross-filesystem cut refuses safely; stable source/destination required (no distributed snapshot); large-tree stress/job-registry retention beyond bounded4096 scan unresolved; permanent delete has no undo. Earlier retry/launch-status gaps are resolved. These captures prove allocated PTY behavior, not physical Foot/Kitty/Ghostty or all themes/hardware. Broader sharing/enrollment/release acceptance remains incomplete.

Deployment PASS: final optimized binary atomically installed under each per-user maintenance lock on innovation and verybeautifulserver. Build and both installed binaries match SHA-256 de16d78818e9efa7a5d89b61fc5c4fda6b10bd011073a0fca3a952d0701008e0. Server command lookup confirms Claude/Codex absent after deployment. Tranquility deployment BLOCKED: fresh `ssh -o BatchMode=yes -o ConnectTimeout=6 tranquility 'id -un'` reports No route to host. No mesh/trust/agent settings changed. Reopen cx to use this viewer build; managed sessions remain running.
