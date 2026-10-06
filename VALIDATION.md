# Validation record

## Offline updater and workspace restoration (2026-10-06)

PASS — integrated `cargo test --locked --quiet`: 78 unit, 20 files/network, 16 session, 15 transfer and 15 update fixture tests; one session test ignored. Log `/tmp/cx-update-integrated-tests.log`. DNS/timeout fixtures preserve the binary, suppress repeated checks within backoff, and permit explicit recovery. Provenance in fixtures is synthetic, not live verification.

PASS — `python3 tests/install_fixture.py`: unavailable network preserves binary; trust-root mismatch and symlink target refused; lock inode preserved; concurrent installers serialized. PASS — `uv run --with pyte python tests/update_viewer_pty.py target/release/cx`: offline update action preserves navigation and termios; private restart snapshot restores host/folder/search/focus. Evidence `local-evidence/update-viewer-pty.json`.

PASS — optimized `HTTPS_PROXY=http://127.0.0.1:9 target/release/cx update --check`: exit 0, current version retained. Independent review found no critical/high issues; fixed dropped manual-result request during startup check. Historical BLOCKED — at that earlier milestone no release or publication authorization existed; superseded by the signed-release results below. Tests simulate unavailable update transport without disconnecting production WARP; physical outage behavior of every provider remains unverified.

Each result distinguishes implemented code, fixture tests, PTY tests and physical hardware. No acceptance gate is complete merely because its code compiles. Sensitive local evidence is outside tracked files.

| Gate | Status | Evidence / limit |
|---|---|---|
| A1 | PASS | 2026-10-05 bounded BatchMode SSH, 6-second connection timeout: innovation→tranquility peace, innovation→server lunarz; tranquility→innovation lunarz, tranquility→server lunarz; server→innovation lunarz, server→tranquility peace. Server peace alias also resolves tranquility. Innovation self-alias fails host-key verification; not one of six directed fleet edges. |
| A2 | BLOCKED | Public bootstrap/releases now verified (see latest record); full owned-resource uninstall and the complete enrollment/failure acceptance remain unfinished. |
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

## Files device switching and direct-transfer feedback

Reproduction on 5cd85f7: `cargo test --bin cx file_device_selection_retargets_only_the_focused_location --quiet` FAIL (browser.device0 after selecting device1). `uv run --with pyte python tests/browser_device_loading_pty.py target/release/cx` FAIL (local directory still Loading after2.5 seconds while synthetic SSH sleeps4 seconds). No live SSH settings changed by the failure fixture.

Fix: retarget the focused Files location on device selection, preserve per-host folders and independent clipboard/panes; synchronize sidebar context on pane focus; bound concurrent metadata workers to4, serialize per SSH target, separate local file reads from potentially remote-blocking job/provider coordination; prioritize restored Files before background probes. Clearing a selection with Escape no longer invalidates an in-flight directory list.

Independent review `/tmp/cx-files-device-queue-review.md` identified three P2 gaps: x→t lost cut intent, pane switches retained the wrong sidebar context, and local job coordination could still block Files on remote owners. All corrected; cut intent and focused-host behavior have regression checks. No shared host services/networking changed.

Focused independent rereview `/tmp/cx-file-ui-final-rereview.md` confirmed those corrections and found opening remote Files from All devices retained stale sidebar context. Root synchronized context in open_browser; regression `opening_remote_files_from_all_devices_keeps_execution_context_on_enter` and full164-test suite PASS after correction. T is also available for jobs outside Files; input fields retain printable text ownership.

| Result | Command/scenario | Evidence |
|---|---|---|
| PASS | Full locked test suite: 164 passed, one existing live session test ignored; optimized build | `/tmp/cx-file-device-tests.log`, `/tmp/cx-file-device-build.log` |
| PASS | `uv run --with pyte python tests/browser_device_loading_pty.py target/release/cx`: local listing ~102ms with4-second slow SSH fixture, sidebar retarget/return and clipboard preservation, restored termios | `local-evidence/browser-device-loading/results.json` |
| PASS | `uv run --with pyte python tests/browser_devices_live_pty.py target/release/cx`: actual read-only innovation Files→verybeautifulserver home listing→original local folder, clipboard retained; private viewer state, no remote writes/sessions started | `local-evidence/browser-devices-live/results.json` |
| PASS | Browser ecosystem at120x40/80x24/48x24, including t destination picker, plain Rename title and colored/bold Help keys; actual copy/cut/delete/termios retained | `local-evidence/browser-ecosystem/results.json`, rename-panel/help-colored-keys/transfer-picker captures |
| PASS | New-viewer durable retry and actual Files folder shell attach/return | `local-evidence/browser-retry/results.json`, `local-evidence/browser-live-launch-device-fix/results.json` |
| PASS | Existing workspace flow at3 sizes | `local-evidence/browser-workspace-device-fix/results.json` |

Interaction: t chooses transfer device/folder (preserves existing copy/cut clipboard; captures selection for copy if empty), p submits, T opens aligned Status/File/Progress jobs and endpoint details. Observe is now Watch · read-only. No physical-emulator success is inferred from these allocated PTYs. Tranquility availability and broader acceptance limits remain as previously recorded.

Deployment PASS: installed final build on innovation and verybeautifulserver under maintenance locks; matching SHA-256 5795afe7cc360742966e34826dc73ef814dc204a775da926ee837d6ac1859af3. Tranquility BLOCKED: fresh SSH still reports No route to host. Physical user confirmation requested for repaired switching and t picker; pending, not a pass.

## Signed releases, compact dialogs and shell-launched agents (2026-10-06)

Release source: main/a1c4a28, tag [v0.1.2](https://github.com/1unarzDev/cx/releases/tag/v0.1.2). Final publication [workflow37459990195](https://github.com/1unarzDev/cx/actions/runs/37459990195) PASS. Earlier v0.1.1 publication workflow37457071172 PASS. Both architectures build natively on GitHub-hosted Ubuntu24 workers; static MUSL executables have no ELF interpreter. Nine distro cases: Ubuntu22/24 and Rocky8/9 on x86_64/aarch64 plus Arch x86_64. Official Arch ARM container unavailable; licensed RHEL and older stock kernels were not tested. Containers share the runner kernel; guarded filesystem writes require openat2/kernel5.6+ and fail safely otherwise.

| Result | Actual command/scenario | Evidence |
|---|---|---|
| PASS | `cargo test --locked --quiet`:167 passed, one existing live-session case ignored; optimized native/static builds | `/tmp/cx-release-session-tests-final.log`, `/tmp/cx-manual-agent-build.log`, `/tmp/cx-tmux27-color-build.log`, published CI |
| PASS | `python3 tests/install_fixture.py`:31 ordinary-user cases, real synthetic RSA4096 signatures, archive/lock/ownership/offline/signal/concurrency failures, apt/dnf/yum/pacman controlling-TTY consent and compact/pretty JSON | `/tmp/cx-install-compact-results.json`; CI repeats current fixture |
| PASS | Nine actual distro/architecture installer+shell+detached-copy+rename and attached-PTY manual-agent cases | final publication workflow above; synthetic signing root in distro tests |
| PASS | Exact README public bootstrap command:fresh nonroot Ubuntu22/24, Rocky8/9, Arch x86_64 install/rerun/offline preservation against real public v0.1.1 assets and production key | `/tmp/cx-public-bootstrap-results.log`; no mocked transport/key in these tests |
| PASS | Same exact public piped command starts real v0.1.2 viewer, handles PATH prompt through controlling terminal, exits cleanly and restores termios in80x24 PTY | `/tmp/cx-public-bootstrap-pty-v012.json`; allocated PTY, not physical emulator proof |
| PASS | Public v0.1.2 signature verification and automatic viewer exec after idle, same shell id/PID/start/boot and restored terminal; unavailable update service preserves executable | `/tmp/cx-public-update-proof-v012.json`; isolated current signed updater compiled as0.1.0, real production v0.1.2 asset; explicit check-only then fresh automatic-check interval |
| PASS | 120x40,80x24,48x24 actual browser PTYs: Rename three rows, plain Filter title, horizontal delete Cancel/Delete with y/n, gg/G/count navigation and input ownership | `/tmp/cx-packaging-browser-final.log`, `local-evidence/browser-ecosystem/results.json`;48 ASCII/NO_COLOR |
| PASS | Foreground Claude/Codex recognized in shell; exit restores Shell, background excluded, argv0 spoof excluded; unchanged terminal id/PID/start/boot/socket | `tests/manual_agents_live.py`, `/tmp/cx-manual-agent-fixture.json`, `/tmp/cx-tmux27-attached-fixture.json`; synthetic Bash executables, real attached tmux PTY |
| PASS | Actual installed Claude2.1.290 and Codex0.160.1 manually launched in disposable innovation Fish shells; provider recognized without creating/replacing terminal | `/tmp/cx-real-manual-agents.json`, `/tmp/cx-real-manual-agents-installed.json` (repeated with published static binary); no prompt sent/inference requested, not full agent-task proof |
| PASS | Native shell attach/single Ctrl+] return, Unicode, italic output and terminal restoration | `/tmp/cx-final-shell-return.json`; disposable owned server |
| PASS | Custom tmux version wrapper sleeps10sec; setup completes0.275sec and uses default colors | `/tmp/cx-stalled-tmux-version.json`; scope is capability-probe bound, not UI latency budget |
| PASS | Public signed v0.1.2 installer on innovation and verybeautifulserver, matching SHA256, server Claude/Codex lookup remains absent | actual tool output; `/tmp/cx-release-deployment.json` |
| BLOCKED | Tranquility update: `ssh -o BatchMode=yes -o ConnectTimeout=6 tranquility 'id -un'` reports No route to host | no trust/mesh/network mutation; prior installed version retained |

Review findings fixed: independent review caught POSIX quoting parsed through Fish (HIGH), fixed owned server parser=/bin/sh while explicitly execing selected user shell. Actual selected Fish wrapper and quoted directory were independently retested. Public API compact JSON exposed installer parser failure, corrected and public command rerun on all five x86_64 distributions. Older tmux rejected terminal colors and opened an input-consuming configuration-error pager; compatible default-color fallback plus real attached-PTY test fixed it. tmux2.7 crashes on detached `send-keys -l`; the fixture now uses genuine attached-terminal input, which is cx's native path. Rocky8 sleep is a coreutils wrapper, so fixtures use actual ELF Bash copies; no production heuristic was weakened to accept argv0 spoofing. A hosted ARM timing-sensitive Bash test now avoids reading host startup files. Node recognition reads only the script argument; capability probe is bounded with existing process-group cleanup. Independent reports: `/tmp/cx-install-integrated-review.md`, `/tmp/cx-tmux27-followup-review.md`, `/tmp/cx-manual-agents-independent-review.md`, `/tmp/cx-manual-agents-final-refinement-review.md`. No demonstrated unresolved critical/high finding in these changes.

Earlier workflows37455584726/37455888619/37458115920 FAIL (UID collision, newer tmux option, old-tmux config/test issues);37456871486/37459611057 CANCELLED before publication for corrections. Unpublished candidate tags were updated; published release assets remain immutable.

Authenticity root: RSA4096/SHA256 public key in release-key.pem, embedded in installer/updater. DER public-key SHA256:`9ee4c08c680f982d66fcf10e0b59f806547da86a3880844c1baa522b40536b77`. Bootstrap script itself is trusted from GitHub HTTPS; downloaded checksums alone are not authenticity. Signing private material remains outside tracked files and in the isolated GitHub signing secret. Provenance is additional evidence.

Deployment: innovation Arch/lunarz and verybeautifulserver Ubuntu24/lunarz both run public0.1.2 at `~/.local/bin/cx`, SHA256`fa55a31f42b7bd477ff4e2080aeb0b4ffe77e706c86ee2744c4c333fc6925fe5`. Installer serializes owned maintenance, replaces only its binary/PATH integration, and leaves sessions/jobs intact. No AI runtime installed on server. Tranquility/peace remains pending offline; no reboot, global network/firewall, SSH-trust or protected tmux change. No new chezmoi changes required by this packaging milestone. Reopen cx for the new viewer; existing work remains running.

Limits: detection is observational metadata, not provider authentication. Custom/renamed binaries, interpreter flags before script, inaccessible /proc, inactive panes and traversal beyond64 processes/depth6 may remain Shell/unknown. Background agents do not replace the active shell badge; original names persist. Non-tmux running terminals cannot be retrofitted reliably. Older tmux default-color fallback and foreground/control PTYs tested; not every advanced tmux option/Watch mode or physical emulator/theme. Full sharing, enrollment trust/revocation, owned uninstall, broader failure/hardware and performance acceptance remain incomplete. Prior performance observations remain in their earlier records;0.275sec above measures a deliberately stalled setup probe only.

Removal/rollback: when no active transfer needs the helper, remove only `~/.local/bin/cx` to stop new launches; leave state/user work/tmux sessions alone. Full owned-resource uninstall is not implemented. Reinstall a pinned reviewed release with `CX_VERSION=v0.1.2 CX_NO_LAUNCH=1 sh install.sh`; the same signature/locking checks apply. Do not delete state, stop jobs or restore whole firewall/configuration as an uninstall shortcut.

### Actions audit and foreground folder commands — 0.1.3 actions/commands

Host: innovation, Linux/Arch; native Bash and tmux, allocated PTYs (not a physical emulator confirmation). Source: 7fba202; optimized static MUSL binary.

| Result | Scenario | Evidence |
|---|---|---|
| PASS | Full Rust suite: 177 passed, one existing ignored; workspace-only actions, n folder/provider context, pinned : input, cancellation/unsupported helper, long-command editing/scroll | /tmp/cx-context-final-tests.log |
| PASS | Native command: exact quoted/Unicode folder/write, wrapper, exit7, Ctrl+C, stdin, ordinary/exec Ctrl+Z resume/cancel, no suffix replay, same-command completion, termios/foreground restoration; optimized MUSL Bash/Fish | /tmp/cx-context-native-final.json |
| PASS | Integrated browser/navigation/clipboard/destination/conflict/jobs at 80x24,120x40,48x24 | /tmp/cx-context-final-flows/results.json and terminal captures |
| PASS | Actual detached copy/integrity/job drawer, folder shell creation and single-key return | /tmp/cx-context-jobs/results.json and terminal captures |
| BLOCKED | tranquility deployment/live remote command | SSH BatchMode/ConnectTimeout5: No route to host; no network/trust changes |

Independent Actions review: /tmp/cx-actions-command-review.md. File operations and duplicate shell controls were removed from workspace actions. Long command visibility/editing and misleading missing-capability remedy were fixed, with regression tests. Foreground commands deliberately do not provide managed-session lifetime through disconnect.

PASS — Zsh native-command scenarios above in an isolated nonroot Arch container with initialized synthetic .zshrc: /tmp/cx-context-zsh-fixture.log. New-user Zsh first-run wizard is preserved in production; fixture supplies an ordinary startup file. Initial fixture exposed Zsh treating command-prefixed builtins as external commands; corrected to explicit builtin invocation and Bash/Fish/Zsh scenarios rerun. Release checks now gate all three shells.

PASS — integrated current-folder n/: workflows, real foreground commands and outer-viewer survival with Bash/Fish: /tmp/cx-context-integrated-final.json. Independent review additionally exposed ordinary child-job suspension beyond direct exec; shell adapters now keep these commands in their owned group and offer explicit Resume/Cancel. Scope is one-shot command execution; personal and managed tmux servers are unchanged.

PASS — independent post-fix native Bash/Fish PTY acceptance: /tmp/cx-independent-command-bash.json and /tmp/cx-independent-command-fish.json; final independent review /tmp/cx-actions-command-review.md reports no demonstrated remaining high blocker for this slice. Physical emulator confirmation remains pending; Zsh fixture success was orchestrator-run.
