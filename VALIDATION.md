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

Initial unpublished 0.1.3 release workflow37465255508 failed its Zsh fixture: runner-wide compinit opened an insecure-directory consent prompt. Synthetic .zshenv now disables global startup only inside the fixture (and clears inherited ZDOTDIR); the product continues to preserve user startup and trust prompts. No release was published from that failed candidate. Evidence: /tmp/cx-actions-checks.log. Updated isolated Zsh scenarios pass again; the candidate gate is rerun.

### Remote capability correction — 0.1.4

Live 0.1.3 verification FAIL: the viewer filtered helper capabilities down to agent names, dropping native-command-v1 and falsely suggesting a helper update. Corrected in357c9d4, with an Operation::Info → command-dialog regression. Full Rust suite:178 PASS, one existing ignored (/tmp/cx-context-final-tests.log).

PASS — actual innovation viewer candidate0.1.4 to installed0.1.3 helper on lunarz@verybeautifulserver: remote browser, n offers only Shell, : pins that device/folder, exact remote write, refreshed browser, Ctrl+C preserves viewer, tty modes/foreground restored. Command: python3 tests/remote_folder_command_pty.py target/x86_64-unknown-linux-musl/release/cx verybeautifulserver. Evidence /tmp/cx-context-remote-live.json. Agent inventory absent before/after; one temporary remote folder cleaned. These compatible helper versions use the same negotiated capability.

Public0.1.3 release workflow37465753315 PASS (all checks, both native builds, nine distro/architecture cases, signing/publication). Installed using the public pinned installer on innovation and verybeautifulserver; SHA256c5421f26b971751369f0c0b333883b65793de33148918a1e32d318474baf49b0. Actual installed signed Bash/Fish and isolated Zsh native-command PTYs PASS: /tmp/cx-context-installed-native-{bash,fish,zsh}.json. Remote : needs the0.1.4 viewer correction above. Tranquility retry remains No route to host. The published0.1.3 artifacts remain immutable; a successor release carries the fix.

PASS — sh/dash compatibility was checked before0.1.4 publication: Ubuntu24 dash and Arch sh-as-Bash, all native command scenarios above (/tmp/cx-context-sh-fixture.log, /tmp/cx-context-sh-bash-fixture.log). Interactive dash returned to its own prompt after Ctrl+C; its one-shot adapter now uses login startup and native batch signal behavior, preserving profile functions. Managed shell sessions are unchanged. Bash/Fish/Zsh suites and real server workflow were rerun successfully; full178 Rust tests PASS. Release checks now include /bin/sh. Unpublished workflow37467429303 was cancelled to include this compatibility correction; published0.1.3 remains immutable.

Final0.1.4 release workflow37469591250 PASS: four-shell native PTYs, full Rust/installer checks, x86_64/aarch64 builds, nine distro/architecture cases, signing/publication. Published source36cd089223427c063dcacb8063733397b404bb94: https://github.com/1unarzDev/cx/releases/tag/v0.1.4.

Installed0.1.4 from the public pinned bootstrap on innovation (Arch/lunarz) and verybeautifulserver (Ubuntu24/lunarz), serialized by installer maintenance locks. Both binary SHA256be9e7d83d004ce0121fa660e5b956242459c9dafa125f389e441d78dd84832a3. Actual signed native Bash/Fish/sh/Zsh scenarios PASS: /tmp/cx-context-installed-native-{bash,fish,sh,zsh}.json. Ubuntu's container test account UID differed from the owner-only installed binary; fixture was rerun as ordinary UID1000 without changing installation permissions. Bash/Fish were native host PTYs; sh/Zsh isolated container PTYs, not physical GUI confirmations.

PASS — final installed0.1.4 innovation → lunarz@verybeautifulserver live workflow: python3 tests/remote_folder_command_pty.py ~/.local/bin/cx verybeautifulserver; /tmp/cx-context-remote-installed.json. Agent inventory absent before/after; temporary folder cleaned. Both installed client and helper are0.1.4. Tranquility remains BLOCKED: latest BatchMode/ConnectTimeout5 retry returns No route to host. No new host packages, user services, provider configuration, SSH trust, mesh settings or personal tmux configuration were changed by this slice. Existing owned binary/PATH integration was updated.

Rollback: CX_VERSION=v0.1.2 CX_NO_LAUNCH=1 sh install.sh installs the reviewed prior signed build without deleting session/job state. Remove only ~/.local/bin/cx when no active job needs the helper; full owned-resource uninstall remains unfinished. Physical Files/n/: feedback is optional and pending; broader product limitations above remain applicable.


## Native command palette preservation (2026-10-06)

Source: `10e4a10110b2bd9d922237d28725ea7c95149470`, signed [v0.1.5](https://github.com/1unarzDev/cx/releases/tag/v0.1.5). A harmless Fish native command reproduced OSC104/110/111 resetting the viewer's dynamic palette/default colors in v0.1.4. Those resets are now confined to managed tmux panes. Native commands retain `CX_VIEWER_THEME=1`; targeted personal chezmoi integration guards the known Caelestia startup palette loader while preserving ordinary terminal behavior.

| Result | Scenario | Evidence |
|---|---|---|
| PASS | Original v0.1.4 regression fails on OSC104; optimized fixed Bash/Fish/sh native PTYs retain palette, wrappers, command/signal semantics and tty restoration | `/tmp/cx-palette-native-{bash,fish,sh}.json` |
| PASS | Integrated Files → `:` → command → browser restoration in Bash/Fish; Actions excludes Command while `:` remains available | `/tmp/cx-palette-folder-ui-final.json`, Rust UI regression |
| PASS | Full Rust suite: 178 passed, one existing live test ignored | `/tmp/cx-palette-tests-final.log` |
| PASS | Caelestia guard fixture preserves ordinary palette emission, suppresses it under CX_VIEWER_THEME, and reruns without extra backups | chezmoi `tests/check-cx-theme.py` (including stale atime regression) |
| PASS | Actual innovation Fish startup + native command emits no palette mutation codes | `/tmp/cx-palette-live-innovation.json` |

Physical emulator palette/transparency confirmation remains a user check. Programs and unrelated startup themes can explicitly change terminal colors; cx does not filter native terminal traffic or claim to override them.

Independent review: no high/blocking Rust finding; guard false-race on self-updated atime fixed and regression-tested before deployment. Evidence `/tmp/cx-independent-palette-{bash,fish}.json`, `/tmp/cx-theme-guard-atime-review.txt`.

Release workflow [37480567781](https://github.com/1unarzDev/cx/actions/runs/37480567781) PASS: four-shell native tests, integrated Bash/Fish commands, signed x86_64/aarch64 builds and nine distro smoke cases. Exact public bootstrap with `CX_VERSION=v0.1.5 CX_NO_LAUNCH=1` installed the signed release on innovation, tranquility and verybeautifulserver. Identical binary SHA256: `91ab9ec254e185d551a8b0e46b6c7eda76c81188b844f89e2b3c9fa2584022e8`. Installer maintenance locks serialize mutation. Tranquility's first noninteractive install attempt safely refused because PATH excluded its existing private tmux wrapper; retry with `$HOME/.local/bin` added succeeded without installing packages.

Actual installed native palette captures on all three hosts PASS: `/tmp/cx-palette-installed-{innovation,tranquility,server}.json`. Installed Bash/Fish folder UI including Actions exclusion PASS: `/tmp/cx-palette-installed-folder-ui.json`. Installed actual remote folder workflow on server PASS: `/tmp/cx-palette-remote-folder-installed.json`. Server has no Claude/Codex commands on PATH before/after; no agent installation occurred. Install evidence: `/tmp/cx-palette-install-{innovation,tranquility,server}.log`.

ChezMoi integration `second_dots/main/4b6f8f0` pushed and narrowly applied on both Arch hosts; ordinary theme and cx guard independently pass `/tmp/cx-independent-guard-final.log`. Only the recognized palette-load block changes, backed up under `~/.local/state/cx/fish-theme-before-*`. To reverse this integration, remove its guarded block and restore the original single `cat ~/.local/state/caelestia/sequences.txt 2> /dev/null` line; remove the integration script from source before a future apply. No unrelated tmux, trust, provider or network state changed. A fresh viewing terminal is needed if v0.1.4 already reset its palette. Physical GUI confirmation remains unverified.


## Managed terminal wheel scrollback (2026-10-06)

Source: next v0.1.6. Reproduced v0.1.5 managed mouse=off; terminals can synthesize Up/Down on wheel motion in alternate screens, invoking native prompt history. Independent actual alternate-screen/no-mouse reproduction showed tmux3.7c's default WheelUp also fails to enter scrollback when mouse is enabled. Explicit cx-owned routing now considers app mouse requests and copy mode, rather than alternate-screen state. Both copy-mode key tables explicitly cancel on Escape. No provider command, endpoint or credential changed.

| Result | Scenario | Evidence |
|---|---|---|
| PASS | Real managed/native-attach PTY on tmux3.7c: wheel enters/scrolls history without application input, normal+alternate screen, emacs/vi Escape, native Up key, requested native mouse events, Ctrl+] in copy/live modes, same pane PID after reattach | `tests/mouse_scrollback_pty.py`, `/tmp/cx-wheel-after-final.json` |
| PASS | Same managed-PTY scenarios on actual tmux2.7/Rocky8 container using MUSL build | `/tmp/cx-wheel-tmux27.json`, image `cx-tmux27-fixture:local` |
| PASS | Independent integrated tmux3.7c PTYs; reviewed alternate-screen and vi Escape defects fixed | `/tmp/cx-mouse-independent-final.json`, `/tmp/cx-mouse-final-review.md` |
| PASS | Full Rust suite178 passed, one existing live test ignored | `/tmp/cx-wheel-rust-tests.log` |

Physical Foot/trackpad/Codex chat scrolling is not established by synthetic PTYs. No transcript scraping/inference requests used. History remains bounded2000 lines and cannot restore text an application never rendered or tmux already discarded. External personal tmux sessions are unchanged.

## Preview, confirmation and installer utility integration (2026-10-06)

Candidate v0.1.7. Images (PNG/JPEG/GIF/WebP/BMP/ICO), inert Markdown, text/code, tar/tar.gz entry lists and WAV metadata are bundled. PDF page-one rendering uses bounded Poppler execution; other formats have an explicit fallback. Text CRLF/CR are normalized, control/bidi characters escaped, and hostile PAX names bounded before protocol framing. Stop is restricted to the original managed shell identity and rejects external sessions, extra panes and detected provider descendants. A process-spawn race cannot be eliminated entirely on a shared Unix account.

| Result | Scenario | Evidence |
|---|---|---|
| PASS | Full Rust suite: 234 passed, one existing ignored live test | `/tmp/cx-v017-tests.log` |
| PASS | Real framed helper: CRLF, terminal controls, Markdown, bundled PNG, corrupt image, actual Poppler PDF, 1.1M-character PAX filename | `tests/preview_helper_live.py`, `/tmp/cx-utils-preview-helper.json` |
| PASS | Browser PTYs at 120x40, 80x24, monochrome 48x24: selection/copy/cut/rename, delete default cancellation and confirmation, termios restoration | `/tmp/cx-utils-browser-pty.json` |
| PASS | Exact shell stop, runtime mismatch/reuse/boot guards, provider takeover and peer preservation on private tmux socket | `/tmp/cx-utils-stop-live.json` |
| PASS | Managed mouse/input/return PTY regressions, tmux3.7c | `/tmp/cx-utils-wheel.json` |
| PASS | 32 signed installer fixtures; apt/dnf/yum/pacman Poppler/ip/ping/terminfo/gzip/grep/sed mappings; enrollment refusal before helper replacement | `/tmp/cx-utils-install-fixture.json` |
| PASS | Three-size preview/delete/stop captures, fixture renderer only | `/tmp/cx-preview-captures-current`, `/tmp/cx-utils-captures.log` |

Independent utility review found missing gzip, grep/sed package declarations and enrollment prerequisite validation; fixed. Actual Rocky8 testing exposed ip in sbin outside ordinary PATH; installer/enrollment and network observation now use standard-path fallback. PDF utilities are installed on verybeautifulserver (user supplied). AI runtimes, NetworkManager, systemd and personal shells are not installed or replaced. A working existing systemd user manager is required for persistence when launched from a service cgroup; unsupported environments fail explicitly. RHEL CI uses Rocky compatibility fixtures, not licensed physical RHEL. Physical image/PDF terminal appearance still needs user feedback. Broader sharing/trust/uninstall acceptance remains incomplete.

Root distribution smoke PASS for optimized MUSL v0.1.7 on x86_64 Ubuntu22.04/24.04, Rocky8/9 and Arch: real package availability, tmux terminfo, signed-fixture install/rerun, shell creation, detached copy and guarded rename. Evidence `/tmp/cx-v017-distro-smoke.log`. This uses synthetic signing keys and agents; public release authenticity and actual installed provider behavior are separate gates. Independent re-review PASS: `/tmp/cx-utility-rereview.md`, 32 installer fixtures, missing-tool enrollment refusal and ShellCheck0.10. Static real-helper preview, stop and actual Codex0.160.1 workspace checks PASS: `/tmp/cx-v017-{static-preview,stop,directory}.json`.

Public v0.1.7 (source `fd52419`) release workflow37488367744 PASS: signed Linux x86_64/aarch64 and nine distro cases. Exact public bootstrap installed on innovation, tranquility and verybeautifulserver; identical SHA256 `0f9f6de58c81023b402881a1bb8ed78c984e29aec49e02dc732c118719c36d88`. Real helper previews PASS on all three (`/tmp/cx-v017-installed-preview-{innovation,tranquility,server}.json`). Installed private-socket stop checks PASS on innovation/server. Tranquility's initial stop fixture FAIL: isolated HOME broke its existing HOME-relative tmux wrapper. Preserving real HOME while isolating only XDG state passes `/tmp/cx-v017-installed-stop-tranquility.json`. Actual server network observation now reports interfaces/routes observed and public ICMP reachable (`/tmp/cx-v017-installed-network-server.json`); server remains agent-free. Exact unpinned README curl bootstrap starts viewer in a clean ordinary-user HOME and restores PTY modes (`/tmp/cx-v017-public-bootstrap-pty.json`).

Followup v0.1.8: red fixture proves tmux loader error containing “No such file” was incorrectly reconciled as already stopped in v0.1.7 (`/tmp/cx-loader-before.json`). Error matching is now restricted to tmux's server-connect messages; a broken wrapper/library is an error. Same synthetic failure preserves both shells and returns an error (`/tmp/cx-loader-after.json`); normal managed wheel/return regression PASS (`/tmp/cx-loader-wheel.json`). It is not a process-kill defect. Publication/deployment is pending its fresh gate.

## Language grammars, native previews and release descriptions (2026-10-06)

Candidate v0.1.9: bundled two-face/syntect grammars highlight source languages and Markdown fenced code without execution. Styling follows ANSI foreground colors with plain monochrome fallback; preparation is cached off the input thread. Higher-resolution bounded PNG payloads support optional native graphics and larger preview workspace.

- PASS innovation: 254 Rust tests (one existing ignored), optimized build, real helper PNG/PDF/CRLF/PAX, syntax ANSI/monochrome PTY with inert-source sentinel and terminal/selection restoration. Evidence `/tmp/cx-v019-tests.log`, `/tmp/cx-v019-build.log`, `/tmp/cx-v019-helper.json`, `/tmp/cx-v019-source-pty.json`.
- PASS scoped Foot1.28/Sixel on Hyprland: image/PDF, Help overlay/return, resize, Escape and SIGTERM cleanup. Initial residual fallback strip failed; clear-on-native-ready fix passed rerun. Eight sanitized captures independently inspected: `/tmp/cx-native-ui-foot-results.json`, `/tmp/cx-native-ui-foot-*.png`. Fixture-only PTY input relay; this is physical emulator rendering, not direct human keyboard confirmation.
- Independent review found no critical/high issues; medium stale-font-geometry completion and low ASCII outer-border issues fixed with regression coverage. Evidence `/tmp/cx-native-final-independent-review.md`, `/tmp/cx-native-review-fixes.log`.
- BLOCKED physical Kitty/Ghostty tests: emulators unavailable. Unknown/SSH/nested-tmux viewers retain fallback; translucent Sixel images also fall back. Native graphics are optional. Broader fleet performance acceptance remains incomplete; no new performance budget claim.
- PASS release descriptions: all eight published v0.1.1–v0.1.8 bodies updated from source history and read back exactly. Reviewed CHANGELOG sections now supply future release bodies; missing stable-version notes block publication. Assets and historical tags were preserved.

Post-review v0.1.9 source `1626cb7`: full Rust suite PASS (255 tests, one existing ignored) and optimized build PASS (`/tmp/cx-v019-final-tests.log`, `/tmp/cx-v019-final-build.log`). Independent delayed-font and monochrome confirmation regressions PASS (`/tmp/cx-independent-geometry-regression.log`, `/tmp/cx-independent-ascii-regression.log`). Optimized synthetic performance on innovation/i7-12700K/Linux7.2.8 x86_64: 100 cached sessions, 20 startup samples p50 7.65ms/p95 16.69ms; 40 Help samples p50 0.95ms/p95 3.10ms; 2s idle zero output/CPU ticks, viewer7976KiB/helper5084KiB. Evidence `/tmp/cx-v019-performance.json`; this excludes remote probes and does not measure loaded grammars/native-image peak RSS or full fleet performance.

PASS optimized browser ecosystem at120x40/80x24/48x24: hidden files, multiselect, copy/paste, rename, guarded delete, cut conflict renaming, visual range and termios (`/tmp/cx-v019-browser.json`). Read-only live predeploy check: verybeautifulserver remains agent-free with Poppler; tranquility SSH currently fails No route to host, so its update is BLOCKED pending reachability.

Signed v0.1.9/source1626cb7 published: workflow37494951757 PASS, both static architectures and all nine distro/architecture install cases. Release body exactly matches reviewed Features/Fixes notes. Serialized exact bootstrap installed v0.1.9 on innovation and verybeautifulserver; tranquility update BLOCKED (SSH No route to host), retaining0.1.8. Installer logs `/tmp/cx-v019-install-{innovation,server}.log`. Actual unpinned README piped install in clean HOME/PTY PASS, version0.1.9 and terminal restoration (`/tmp/cx-v019-public-bootstrap-pty.json`). Installed innovation source ANSI/monochrome and PNG/PDF/hostile archive helper PASS (`/tmp/cx-v019-installed-{source-pty,helper}.json`).

FAIL server real PDF-render acceptance: installed Poppler cannot load `/lib/x86_64-linux-gnu/liblcms2.so.2` (Input/output error), confirmed on two bounded `pdftoppm -v` calls. No library/disk repairs attempted. PASS separate degraded-state scenario: helper explicitly reports PDF preview unavailable; PNG high resolution, corrupt-image fallback, text normalization and hostile archive bounds pass (`/tmp/cx-v019-server-degraded-helper.json`). This is not a successful server PDF-render test. Server still agent-free after install. Foot native physical evidence remains from the integrated build; physical Kitty/Ghostty and full-fleet performance remain unverified.

## Missing preview report: old open viewer (2026-10-06)

Read-only process identity check found the open innovation viewer PID654507 mapped to cx0.1.0 while installed binary is0.1.9. Disposable clone of that exact old executable reproduces the report: Enter on PNG shows Binary file — preview unavailable, no raster within2s; repeated twice. Command `uv run --with pillow python /tmp/cx-preview-old-viewer-foot-test.py` FAIL as expected; private screenshot `/tmp/cx-preview-old-viewer-foot-missing-preview.png`. Fresh installed0.1.9 same scoped real Foot workflow PASS: PNG/Sixel within2s, PDF, overlay/back, resize and terminal cleanup (`/tmp/cx-preview-stall-foot-test.json`, `/tmp/cx-preview-stall-foot-results.json`, matching captures). No provider content, user files or credentials captured. This reproduces a stale-viewer version difference, not a0.1.9 image-loader stall. User confirmation after reopening remains pending.

Stopped19 orphaned older cx viewers with PPID1 and deleted terminals; SIGTERM did not stop them, so exact revalidated viewer PIDs were terminated. Open viewer preserved; no tmux sessions or helper jobs were targeted. Cleanup evidence `/tmp/cx-preview-orphan-cleanup.json`. Also stopped orphaned GUI repro fixtures using matching private fixture viewer.pid. Installed0.1.9 closed-PTY check exits within2s without a runaway process, though exit101 is not a graceful-success claim (`/tmp/cx-closed-pty.py`). Server PDF library I/O error remains separately unresolved. No preview code change or new release was needed for the reproduced stale-viewer case.

## Accessed-host signed updates and multi-page PDF candidate (2026-10-06)

Integrated source19e7aa9: 297 Rust tests PASS, one existing ignored (`/tmp/cx-v010-integrated-tests.log`) and optimized build PASS (`/tmp/cx-v010-integrated-build.log`). PDF navigation test red before implementation (`/tmp/cx-pdf-scroll-red.log`), green after (`/tmp/cx-pdf-ui-tests.log`). Actual helper renders3 distinct pages, reports count, preserves source, rejects0/4/10001/malformed/nonPDF, missing converter honest (`/tmp/cx-pdf-pages-integrated-helper.json`). Actual viewer PTY displays red/green/blue pages, counters, j/k/PgDown/SGR wheel and Escape/termios (`/tmp/cx-pdf-pages-integrated-pty.json`). Scoped Foot physical output shows pages2/3 and backward navigation, resize/overlay/return; root inspected private captures (`/tmp/cx-pdf-pages-foot-test.json`, `/tmp/cx-pdf-pages-foot-*.png`); fixture input relay, not physical human wheel interaction. Keyboard/wheel coalescing/cancel/error preservation and host update read-only refresh regressions pass.

Independent review19e7aa9 vs5d3ca1d found no critical/high/medium findings; transport, page UI and helper tests independently passed (`/tmp/cx-pages-independent-review.md`, `/tmp/cx-pages-independent-helper.json`). Signed host updates use fixed BatchMode SSH, bounded output/runtime, max2 checks, per-host cooldown and existing per-host maintenance lock; fresh strictly-newer stable version and identity proof required before connection invalidation. Pending requests finish first. Synthetic update subprocess timeout/flood/offline/stale identity/current-on-disk tests passed (`/tmp/cx-host-update-validation.json`). Root32 installer scenarios PASS (`/tmp/cx-pages-installer-fixtures.json`); syntax regression PASS (`/tmp/cx-pages-source-regression.json`). Full signed publication and live auto-upgrade/session-survival acceptance remain next.

Fresh live read-only tranquility now reachable: installed/viewer0.1.9; Poppler26.08.0; helper PNG/PDF/CRLF/archive checks PASS (`/tmp/cx-tranquility-preview-helper.json`). User confirmed preview worked after innovation update; implementing endpoint update/reconciliation addresses this remote workflow. Maximum1280x960 PNG viewer decode regression PASS; an allocation-limit hypothesis was falsified, and limits were not loosened. PDF input16MiB/page-count10,000 are honest limits; server PDF library I/O fault still prevents real PDF rendering there.

## Signed v0.1.10 live acceptance (2026-10-06)

Release source `1213ed3`: workflow37502598744 PASS (both static architectures and nine distro/architecture cases); published Features/Fixes descriptions match CHANGELOG. Final local suite297 PASS/one existing ignored, optimized build PASS (`/tmp/cx-v010-final-{tests,build}.log`). ShellCheck0.10 PASS for install.sh and scripts/enroll-helper.sh.

| Result | Actual scenario | Local evidence |
|---|---|---|
| PASS | Innovation exact signed public bootstrap installs0.1.10 | `/tmp/cx-v010-install-innovation.log` |
| PASS | Ordinary innovation viewer automatically upgrades tranquility0.1.9→0.1.10; verified UI notice, same disposable session ID/PID49757/start ticks/boot identity, termios restored | `/tmp/cx-v010-auto-update-live.json`, `/tmp/cx-v010-auto-update-live.py` |
| PASS | Same accessed-device update on agent-free verybeautifulserver; disposable PID1863169 and process/boot identity survive | `/tmp/cx-v010-server-auto-update-live.json`, `/tmp/cx-v010-server-auto-update-live.py` |
| PASS | Installed page helper on innovation and tranquility renders three distinct pages; bounds/malformed/source-integrity checks | `/tmp/cx-v010-installed-pages-{innovation,tranquility}.json` |
| PASS | Actual optimized viewer PTY shows three page colors/counters, j/k/PgDown/wheel and Escape/terminal restoration | `/tmp/cx-v010-final-pty.json` |
| PASS | Exact unpinned public curl command in clean ordinary-user HOME starts0.1.10; controlling-TTY prompt and restoration | `/tmp/cx-v010-public-bootstrap-pty.json` |

Root serialized live mutation tests; removed only the two disposable shells via identity-guarded StopSession after verification. No provider/runtime installation on server; command-v inventory before/after has neither Claude nor Codex. No trust, WARP, provider configuration or protected tmux mutations. Updates did not manually reinstall remote hosts: authenticated Info from the actual viewer triggered their own signed updater. This verifies terminal process survival, not a live transfer-upgrade scenario. Synthetic pending-request/mutation tests remain separate. All three installed binaries now0.1.10; existing open viewer executables can remain older until their idle update/relaunch or manual reopening.

FAIL server PDF rendering remains the previously evidenced Poppler/liblcms2 I/O error; no privileged repair attempted. Physical human PDF wheel confirmation pending; real Foot fixture rendering and actual PTY wheel tests passed. Broader fleet acceptance/performance, complete sharing and trust revocation are not declared complete.

## Managed terminal clipboard (2026-10-06)

Source `2acff83`, candidate0.1.11; ChezMoi `e3bf9a1`. Regression before fix: `python3 /tmp/cx-clipboard-test.py target/release/cx` FAIL because the complete application OSC52 copy never reaches viewer. Fixed PTY validates complete synthetic sequences (startup/echo excluded), both emacs/vi Space+Enter/y selection, actual separated SGR mouse drag events, repeated config identical overrides and single-key return. PASS innovation tmux3.7c `/tmp/cx-clipboard-pty.json`; PASS actual static CI binary/server `/tmp/cx-v011-server-clipboard-pty.json`. Full Rust suite297 PASS/one ignored (`/tmp/cx-clipboard-rust.log`), optimized build PASS (`/tmp/cx-clipboard-build.log`); wheel/app-input regression PASS (`/tmp/cx-clipboard-wheel.json`).

Physical real local Foot/Wayland+Neovim0.12.5: PASS yank into viewer clipboard and explicit Neovim read paste of distinct synthetic text (`/tmp/cx-clipboard-foot-local.json`). Actual real Foot→SSH→verybeautifulserver managed tmux: PASS application write into viewing clipboard and explicit tmux clipboard query (`/tmp/cx-clipboard-foot-server.json`). Original text/empty clipboard preserved in memory only and restored; no real clipboard content logged. Isolated candidate binaries/state/servers cleaned, no protected sessions or personal tmux configuration touched. These tests use synthetic text without provider interaction/inference.

FAIL first physical tranquility attempt: provider evidence file never created; no successful remote Neovim result claimed. Subsequent attempts BLOCKED SSH No route to host. Local Neovim and real remote terminal clipboard paths are separately verified; their combination on tranquility remains pending. Self-SSH innovation fallback BLOCKED host-key verification, which was respected without trust modification.

Independent review corrected idempotent terminal overrides and Enter bindings; strengthened sequence, mode-table and mouse tests. Follow-up identified installed upstream Neovim provider `refresh-client -l && sleep 0.05 && save-buffer -` can return stale paste data on delayed remote responses. README explicitly documents this limitation and recommends terminal paste. Local low-latency success is not proof of reliable `"+p` over arbitrary SSH latency. Clipboard queries occur only on explicit paste/read; cx does not poll or forward display sockets. Physical Kitty/Ghostty remain unverified.

Signed0.1.11/source2acff83 publication PASS (workflow37504474815: both architectures and nine distro/arch cases). Exact public signed bootstrap installed on innovation/verybeautifulserver (`/tmp/cx-v011-install-{innovation,server}.log`), followed by installed real clipboard PTY PASS (`/tmp/cx-v011-installed-clipboard-{innovation,server}.json`). Existing cx-owned servers refreshed through disposable managed-session creation and identity-guarded cleanup; no unrelated viewers detached or sessions renamed. Tranquility binary update BLOCKED No route to host, retaining0.1.10; scoped Neovim config was pushed/applied there before outage. Its ordinary reachable-device updater will offer the newer signed release when reachable; no future deployment claimed.

## Stale shell rows / reconnect acceptance (2026-10-06)

Actual user clarification: innovation rows. Tranquility cache had four innovation shell IDs absent from innovation's current owned tmux list; direct authenticated tranquility→innovation helper list returns only four Codex sessions. Replaced that cache entry with the live list and private0600 backup `/home/peace/.local/state/cx/ui-sessions.before-verified-cleanup.json`. No dead-shell process kill was needed; retained execution-host records for idempotency. User's remaining open tranquility viewer PID18469 runs0.1.4; existing installed0.1.11 separately verified. Reopening is required to discard this old in-memory state.

Separate initial outage: tranquility No route to host initially, later recovered. Its three shells still live; local temporary hiding of three offline snapshots (`/tmp/cx-offline-shell-cache-cleanup.json`) was reversed by restoring authenticated live metadata. PASS all three real read-only/no-resize attachments and Ctrl+] return, PID/start/boot preserved (`/tmp/cx-unreachable-session-attach.json`). PASS all four actual innovation Codex attachments same conditions (`/tmp/cx-live-innovation-attach.json`). Terminal content discarded, no prompts/inference/input sent other than cx return. Initial attachment fixture FAIL TERM=dumb; corrected fixture TERM=xterm-256color passed, not an emulator portability claim.

PASS scoped orphan cleanup on tranquility:26 exact revalidated same-UID cx viewer processes, PPID1/deleted stdin/start identity; SIGTERM followed by guarded SIGKILL when needed. Open viewer18469 preserved; no tmux or transfer jobs targeted. Evidence `/tmp/cx-tranquility-orphan-viewer-cleanup.json`.

Source536853a plus test-only9238073: integrated306 Rust PASS/one existing ignored (`/tmp/cx-reconnect-root-integrated-tests.log`), optimized build PASS (`/tmp/cx-session-reconnect-final-build.log`). Framed actual CLI fixture: closed session observation reconnects once; persistent failure bounded2 attempts; auth/trust/unreachable1 attempt; creation accepted before EOF never replayed1 attempt; synthetic secret excluded (`/tmp/cx-metadata-reconnect-cli.json`, `/tmp/cx-reconnect-root-framed.json`). Actual helper stderr100KiB bounded-classification fixture PASS; inherited stderr holder closed via owned process-group cleanup, no indefinite classification wait. Ratatui48/80/120 rendering keeps Ctrl+P→Refresh visible; fresh empty Sessions clears ended cached rows.

Independent review found medium clipped remedy; fixed by fitting error to one line and reserving remedy's row before tag. Added narrow-size regression and inherited-stderr regression; review replay/sensitive-output behavior passes. Release37516433154 first ARM attempt FAIL upstream updater synthetic installed-exec ETXTBSY (`/tmp/cx-v012-arm-job.log`); no publication from that attempt. Isolated fixture20 runs PASS (`/tmp/cx-updater-textbusy-repro.json`); concurrent inherited writable-descriptor explanation plausible, unproven. Failed ARM job rerun PASS; distro checks/publication pending; no production updater reliability claim made from isolated success.

Post-integration0.1.12/source536853a signed publication PASS, release37516433154 attempt2: both architectures/nine distro cases. Original ARM ETXTBSY failure remains recorded; fresh rerun passed. Exact signed bootstrap installed innovation/tranquility/verybeautifulserver (`/tmp/cx-v012-install-{innovation,tranquility,server}.log`). First tranquility attempt refused missing tmux on noninteractive PATH and retained0.1.11; retried with existing ~/.local/bin wrapper, installed0.1.12.

PASS actual installed framed CLI recovery/lost-mutation and failure-classification fixture (`/tmp/cx-v012-installed-framed.json`). PASS fresh session lists all three hosts (`/tmp/cx-v012-installed-{innovation,tranquility,server}-sessions.json`). PASS installed read-only/no-resize attachment and Ctrl+] return on four innovation Codex terminals and three tranquility shells, with original PID/start/boot/native process identity preserved (`/tmp/cx-v012-installed-{innovation,tranquility}-attach.json`). No user/provider content retained. Server empty live-session result separate from endpoint availability. User's open old0.1.4 viewer preserved; instructed to Ctrl+C from cx workspace and reopen installed ~/.local/bin/cx to discard its in-memory stale rows. No remote stopped-terminal resurrection or auto-replacement claimed.

## Workspace and fleet browser · candidate v0.1.13

Source `8f7df93` on main; integrated on innovation, 2026-10-06. Tests below use the optimized candidate where applicable; fixture success is not physical robot acceptance.

| Result | Scenario / backend | Actual evidence |
|---|---|---|
| PASS | Rust integrated suite: 340 passed, one existing ignored; formatting/diff checks | `/tmp/cx-postreview-final-tests.log` |
| PASS | Six fresh directed SSH edges between innovation, tranquility and verybeautifulserver; actual accounts lunarz/peace/lunarz | `/tmp/cx-v013-ssh-matrix.json`; fixed identity command uses `uname -n` because Arch lacks `hostname` |
| PASS | Actual innovation → tranquility → server helper: sessions, `/tmp` browse, network observation | `/tmp/cx-live-jump-evidence.json` |
| PASS | Actual chained disposable shell: native input, single Ctrl+] return, restored termios, unchanged PID/start/boot | `/tmp/cx-live-jump-terminal.json`; fixture stopped afterward, unrelated sessions untouched |
| PASS | Actual chained copy both directions, 237568 bytes with exact round-trip content/hash | `/tmp/cx-live-jump-copy.json`; only private temporary files removed |
| PASS | Rootless OpenSSH10.5p1 sshd: unknown gateway host key fails without askpass; pinned synthetic gateway offering password auth fails without askpass; config-defined ProxyJump covered; conflicting route preserves old mapping | `python3 tests/jump_routes_cli.py target/release/cx`, `/tmp/cx-jump-routes-cli.json`; no personal SSH/trust edits |
| PASS | Public signed ARM64 artifact obtained; actual CLI enrollment transfers ARM ELF, never executes it locally; synthetic remote installation/Info boundary | `/tmp/cx-arm-enroll-evidence.json`; actual release0.1.12 artifact, synthetic ARM endpoint |
| PASS | Default/ASCII/NO_COLOR UI fixtures, 48/80/120 columns ×24/40 rows: first-run, loading, unavailable, sessions, search, transfer destination and Network menus | `/tmp/cx-integrated-captures/`; TestBackend, not emulator screenshots |
| PASS | Physical Foot disposable viewer: Sessions, Help, focus change and viewer exit | `/tmp/cx-panels-foot-evidence.json`; monochrome inherited environment; captures containing compositor transparency were discarded and are not public evidence |
| PASS | Native command PTY, clipboard PTY, metadata reconnect/failure boundary | `/tmp/cx-network-native-command.json`, `/tmp/cx-network-clipboard.json`, `/tmp/cx-network-reconnect.json` |
| BLOCKED | Physical Jetson/robot enrollment/egress, Kitty/Ghostty, live IPv6 link-local enrollment | No authorized robot fixture available; link-local Connect is explicitly unavailable |

Independent review found and fixed: cross-architecture rejection, All scope limited to local neighbors, background jump authentication prompts (including personal SSH-config routes), overlapping route retargeting, opposite-pane sidebar identity, lexical address sorting, stale port evidence, misleading Network counts, and duplicate footer hints. Re-review reported no remaining HIGH findings in inspected changes. No private keys moved, no agent forwarding, no WARP/network mutations, and no agents installed on the server.

Optimized performance on innovation/i7-12700K/Linux x86_64: 100 cached synthetic local sessions, no remote hosts; 20 startups p50 3.93ms/p95 4.89ms to first rendered status; 40 Help first-output samples p50 0.33ms/p95 0.43ms; 2s idle zero output/CPU ticks; viewer8236KiB, waiting helper4960KiB. `/tmp/cx-v013-performance.json`. This measures cached/basic UI and first output, not complete physical paint, remote fleet latency, image decoding peaks, or laptop load.

Limits: discovery reads bounded existing neighbor caches; it does not guarantee every reachable but unseen host appears. Known IPs/SSH aliases can enroll directly through a gateway. An open TCP22 port is not authentication or internet access; neighbor internet stays unknown until enrolled/observed. Distinct aliases are required for identical addresses behind different gateways. ARM enrollment requires an available signed release and preinstalled remote utilities. Sharing mutation, trust/revocation and full owned uninstall remain outside release readiness.
