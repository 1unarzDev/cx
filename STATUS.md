# cx status

Implementation is in progress; the full product acceptance gates remain incomplete. Origin is https://github.com/1unarzDev/cx.git; the user authorized commits, pushes, releases and GitHub build workers on 2026-10-06. Integrated baseline 81fce86; bounded workers use isolated branches under worktrees. Sharing worker returned df75d04 with scoped controller/panel/fixtures; it is not integrated. NM DHCP/DNS, secure privileged packaging, endpoint verification and physical hardware acceptance remain unresolved.

Working inventory: innovation Arch/tmux 3.7c/Codex 0.160.1/Claude 2.1.290; tranquility Arch with system Codex 0.160.1 but Fish version probe returns Codex 0.160.1, npm installed, user-owned signature-verified tmux added; server Ubuntu 24.04/tmux 3.4, no AI runtimes. Existing rideshare_planning untouched. All six directed inter-device SSH edges were previously verified; innovation self-alias trust fails and is not modified. Linger is enabled on both Arch accounts (rechecked 2026-10-06). These observations are local deployment facts, not product defaults.

Root owns shared model, transport, persistence, manifests, integration and deployment. Workers own sessions; UI; files/read-only network. Live network changes and trust changes remain serialized and gated by actual trust/privilege requirements. The legacy robotics shared_internet.sh replaces the host default route and appends unowned firewall rules; it is inspected, not executed or changed.

## User feedback loop
Milestones: build → integrated test → concrete result → focused high-value questions → incorporate. Routine engineering remains autonomous. At each milestone assess workflow assumptions, UI alternatives, physical behavior, costly reversals, and tradeoffs. Internally classify Blocking / High value / Polish. Only Blocking interrupts dependent work; other work continues. Answers become product direction and go to relevant workers.

Current user direction (validated first live milestone):
- Native attach/return initially worked on the user's terminal; return shortcut was not discoverable and Ctrl+C did not mean return. Preserve native Ctrl+C; managed sessions now use single Ctrl+] to return; keep the native application’s Ctrl+C.
- Search must be fuzzy, live on every edit, and inline in a bottom textbox. Implemented in Work and Files; Enter opens selected match, arrows select, printable navigation characters remain input text.
- Keep sidebar for now but use its empty space better. Slimmed it, added keyboard-accessible device actions and selected execution/workspace context.
- Visuals need more intention, aligned rows and restrained colors/status indicators. Integrated pass uses fixed table columns, project separation, ANSI accents and explicit indicators with default terminal background.

Pending user feedback: physical confirmation of repaired device switching and the t destination picker is outstanding; do not repeat it. Earlier transfer-confusion feedback has been incorporated into direct t and cross-device clipboard navigation. User confirms the new tranquility Shell preserves viewer colors and default-background transparency. Prior prompt/return behavior is confirmed; do not re-ask.

- Bottom hints now use a two-row, three-column grid with a shared six-column key slot and transparent background. Enter/Ctrl+P and Search/Help align; 80/120-column real PTY captures verified in local-evidence/ui-{80,120}.png. Local installed binary updated; reopen viewers to load it.

- Folder navigation: Left/h parent, Right/l enter selected directory, Enter directory/preview. Shift+Tab now reverses the device/action/workspace focus cycle and exits search to the preceding focus. Input text retains printable h/l. Regression tests reproduce old failures and now pass.
- Shell return: user confirms it works. Isolated managed-shell PTY test also passes single Ctrl+], same selection, shell survival and terminal restoration. Native shell keys remain untouched.

- Unicode attachment: tranquility noninteractive SSH reports ASCII locale (LANG/LC_CTYPE unset). Managed/local/remote tmux clients and owned server now use documented -u mode. Synthetic Unicode regression failed before and passes after under LANG=C/LC_ALL=C; no locale or personal tmux configuration changed. Font coverage remains a viewer-terminal responsibility, and Codex output is not rewritten.

- 2026-10-06: user prefers one Ctrl+] to return. Managed root-table binding now detaches directly with prefix None; external servers untouched. Modern tmux terminfo selected by capability (tmux-256color then tmux, screen only fallback), eliminating tmux’s screen compatibility substitution of italic with reverse video. Installed and managed config refreshed nondestructively on innovation/tranquility; temporary refresh sessions removed. Native output regression catches prior reverse SGR and passes now.
- Cached ICMP backend integrated df5e84f. innovation and tranquility both replied to two bounded public probes; unknown on failure, 60-second cross-helper cache. New-session/unified-files/dual-location-transfer/directional-focus UI integrated in 81fce86 with subsequent root refinements.

- Physical user feedback: prompt reverse-background artifacts gone and one Ctrl+] returns cleanly (both working). Preference confirmed. User additionally chose viewer colors by default for cx-managed sessions. Independent review found unbounded terminfo lookup; bounded each probe to 200ms, kill/reap timeout regression added.

## Current integrated feedback milestone (2026-10-06)
- Launch profiles are checked on the execution host using its actual selected login/interactive shell and bounded, output-discarding --version calls. Innovation offers Shell/Claude/Codex; tranquility Shell/Codex (Claude launcher exists but fails); server Shell only. Authentication and model-route health are separate from runtime availability. No agent installed on server.
- New-session picker and folder Start here actions share capability filtering; unknown/stale data hides agents, checks refresh/coalesce, creation rechecks before starting. Expiry cannot substitute Shell for a selected agent.
- Explicit Focus indicator and active heading marker; actions header omits Tab; Search label simplified. Default incoming conflicts rename the copy. Actual detached UI copy preserves existing destination, verifies content and reports completion.
- Viewer colors: new Fish sessions reset their pane palette/defaults after normal shell config with fixed --init-command; no ongoing config edits. Temporary owned guard removed from both Arch Fish configs, with private backups. Default tmux window colors use terminal defaults; explicit application RGB/backgrounds and missing font glyphs remain application/viewer limitations. User physically confirmed the new tranquility Shell’s viewer colors and default background look right.
- Independent review fixes: bounded terminfo, optional-theme config mutation removed, stable selected job across reordered progress, stale Network loading released, refresh/dismiss invalidate old browser/preview responses, provider expiry guarded and probe job control disabled. Durable retry across viewer restart and failed transfer-worker launch status remain known gaps; sharing/auth/enrollment/installer/outer-tmux/release acceptance remains incomplete.
- Deployment: latest optimized local build installed atomically on innovation, tranquility and verybeautifulserver with serialized maintenance locks. Public bootstrap/release and chezmoi integration are still not deployed.
- New session from Files now chooses a provider, then starts and enters it using that browser's execution device/current folder; browser/search/focus remain available on return. Work keeps explicit folder selection. Matching sessions still offer reuse instead of silently duplicating work.
- Existing terminal recovery investigation: Codex 0.160.1 already uses a shared daemon on both Arch hosts; observed innovation non-tmux TUI connections prove daemon-backed engine recovery is a viable optional path. Native reconnect/task continuity has not yet been exercised. Claude /bg starts a replacement process and can stop/restart work; never automate migration. Opt-in tmux from future terminal creation remains the general same-PTY/process/screen path. Evidence /tmp/cx-session-recovery-review.txt.
- User supplied https://github.com/1unarzDev/cx.git; added origin after verifying the repository exists (PUBLIC, empty, no releases). Automatic verified tagged-release updates are now being implemented; publication authorization and actual release validation remain pending.
- Verified-release updater integration is implemented: asynchronous startup/hourly checks, private staging, pinned workflow provenance verification, atomic replacement and idle workspace restoration. Offline checks quietly preserve the current viewer; explicit checks report unreachable service and bypass five-minute cross-start failure caching. Installer download errors preserve an existing binary. No real signed release exists, so publication/bootstrap acceptance remains pending. Server still lacks the GitHub CLI verifier; tranquility deployment is pending while SSH reports no route to host.

## Unified browser/transfer milestone (2026-10-06)
- Yazi-inspired hidden toggle, selection/advance, visual ranges, rename, confirmed permanent delete, filter and clipboard keys integrated. Paste targets the focused folder without interrupting navigation; optional two-location mode shares the browser.
- Aligned markers/sizes, selection counts, COPY/CUT state, contextual hints, compact progress drawer and scrollable error/path details replace the forced destination workflow.
- Backend mutations guard identities and avoid symlink/mount traversal. Cut clipboard entries are consumed only after accepted submission. Detached-launch failures are durable; retries load saved specs/owner after viewer restart. This resolves the earlier retry/launch-status gaps. Skips do not authorize source deletion. Cross-host file cuts verify content/identity; cross-host directory/symlink cuts remain unsupported.
- Independent transfer/security and browser UX reviews fixed clipped errors/warnings, late failures, clipboard loss, misleading queued notices, narrow hints and delete Escape behavior. Large-tree stress and job-registry retention beyond the bounded scan remain unresolved. No undo or distributed snapshot guarantee.
- Final optimized build is installed atomically under maintenance locks on innovation and verybeautifulserver; SHA-256 de16d78818e9efa7a5d89b61fc5c4fda6b10bd011073a0fca3a952d0701008e0 matches both. Tranquility is currently unreachable (No route to host); deployment pending without trust/network changes. Server remains agent-free. Full release acceptance remains incomplete.

## Device browser responsiveness and direct transfer feedback
- Reproduced Files staying on the old host after sidebar selection and local loading blocked by slow remote requests. Device selection now retargets the focused location, restores its folder and keeps clipboard/other location independent. Pane focus updates device context.
- Bounded four-worker scheduling preserves FIFO per remote target; local filesystem reads use an independent lane from provider/job coordination. Sleeping workers use condition variables. Restored Files loads before fleet background probes; navigation remains responsive to offline hosts.
- t opens Transfer to device/folder selection; y/c and x persist across sidebar device switching, p pastes into focused folder. Existing cut intent survives t. T opens jobs/results. Observe renamed Watch · read-only. Rename has a plain title and no duplicate panel instructions; Help keys use accent color and bold monochrome fallback.
- Independent review caught local job-coordinator blocking, stale pane device context and x-to-t converting cut to copy; all three were corrected. Validation and final deployment identities are recorded below in VALIDATION.md.
- Installed final build on innovation and verybeautifulserver under maintenance locks; matching SHA-256 5795afe7cc360742966e34826dc73ef814dc204a775da926ee837d6ac1859af3. Tranquility remains unreachable; no network/trust settings changed.

## Signed distribution and shell-launched agents (2026-10-06)

- The user authorized public releases and GitHub-hosted build workers. v0.1.1 is published with native x86_64/aarch64 static builds, RSA4096/SHA256 signatures and GitHub provenance. v0.1.2 is published at a1c4a28 after all nine distro/architecture cases passed. It is installed from signed public assets on innovation and verybeautifulserver with identical hashes; tranquility is unreachable. Publication/deployment evidence is in VALIDATION.md.
- The public bootstrap was tested from the exact README URL on clean nonroot Ubuntu22/24, Rocky8/9 and Arch x86_64 containers: install/rerun/offline preservation PASS. Compact GitHub JSON exposed a real parser defect; corrected in8059e96 and all five public tests rerun successfully. Piped first launch, controlling-terminal PATH consent and terminal restoration PASS in allocated PTY.
- Work discovers foreground Claude/Codex inside existing shells without replacing their terminal, keeps original session names and identities, and returns the badge to Shell after exit. Background jobs are excluded. Actual local Claude/Codex launch detection PASS with no prompts sent. Recognition is observational: unusual custom/renamed/interpreter launchers and inaccessible/deep process trees can remain unknown.
- Rename is three rows including borders. Filter/Rename titles are plain, delete choices horizontal with y/n and Cancel default; gg/G/count navigation preserves input ownership.
- Older tmux now uses default colors instead of unsupported terminal-color syntax; managed command parsing is POSIX while explicitly preserving the user's selected shell. Fish palette/wrapper quoting and real tmux2.7 PTY agent recognition pass. Capability probing is bounded.
- Pending preference: whether manual agents should change the session name. Current direction preserves names and changes only provider badges. Earlier physical Files/t-picker feedback remains pending; do not repeat answered color/return questions.
- Full sharing, trust/revocation, owned uninstall and broader product acceptance remain incomplete.

## Actions and contextual commands (2026-10-06)

- Actions sidebar now contains workspace navigation, one New session action, transfers and enrollment; Ctrl+P excludes file-level mutations and duplicate shell launch actions. Visible Work is Sessions; persisted view identifiers remain compatible.
- n starts in the focused browser folder with only available providers. : pins that host/folder, opens an editable command prompt, hands the terminal to its native login/interactive shell and returns on Enter after completion. Commands are foreground, not persistent/replayed. Unsupported helpers refuse execution; stale capabilities request a check rather than suggesting a false upgrade.
- File commands remain available on their keys/help. M creates a folder and o cycles the existing copy-conflict policy (rename default).
- Review fixed hidden long commands with cursor editing/scroll and distinguished missing capability evidence from unsupported helpers. Unit and allocated-PTY evidence is recorded in VALIDATION.md. Physical user confirmation and tranquility deployment remain pending while that host is unreachable.

- Live server acceptance caught the viewer discarding native-command-v1 from real Info responses.0.1.4 retains the allowlisted capability, with a reply-to-dialog regression; actual remote command/folder/return checks now pass against the agent-free server.0.1.4 is published and installed from signed public assets on innovation and verybeautifulserver, superseding0.1.3 for remote commands; tranquility remains unreachable. Final signed native and remote PTY checks pass, with evidence in VALIDATION.md.
- Optional physical feedback pending: updated Files → n/Shell/return and :/pwd/return, checking execution location and the persistent-session/foreground-command distinction. Continue independent work; do not repeat this question.


## Native command colors / Actions cleanup
- Fixed native Fish command launch resetting the viewer palette/default foreground/background; managed tmux pane resets remain scoped to tmux.
- Removed Command from Ctrl+P/Actions; `:` and its help entry remain.
- The known Caelestia Fish startup loader needs a cx-scoped guard for direct native commands; targeted chezmoi integration preserves ordinary terminal themes and a private pre-change backup. This supersedes the earlier guard removal for this specific native-launch path.
- Optimized native PTYs, integrated folder command workflows, Rust suite and independent review pass. Signed v0.1.5/source `10e4a10` published (workflow37480567781 PASS) and installed via public bootstrap on all three hosts; real native commands emit no palette changes on each. Server remains agent-free. ChezMoi `4b6f8f0` pushed and narrowly applied on both Arch hosts. Actual remote server folder-command test passes; physical color confirmation remains optional.


## Managed terminal wheel scrolling
- Managed mouse reporting prevents viewing terminals converting wheel motion to prompt-history Up/Down keys. Explicit wheel routing forwards only to applications requesting mouse input; normal/alternate screens otherwise enter tmux scrollback. Escape exits both emacs/vi copy modes; status strip identifies scrollback. Native arrows and Ctrl+] remain intact.
- Real managed PTYs pass on tmux3.7c and container tmux2.7; alternate screens, app mouse forwarding, original process identity, detach/reattach and mode return covered. Independent review caught alternate-screen default-binding and vi Escape pitfalls; both fixed and rechecked. Only cx-owned live servers received scoped hotfix bindings.
- tmux history remains bounded at 2000 lines. Unrendered application history and older discarded lines cannot be recovered by tmux; native provider transcript controls remain relevant. Physical wheel behavior on the user's existing Codex chat awaits confirmation.

## Preview + installer dependency milestone
Candidate v0.1.7 integrates bounded image/PDF/Markdown/archive previews, clean guarded deletion and `d` managed-shell stop. Root integrated tests pass (234 Rust tests plus browser/helper/stop/mouse PTYs). Installer provides Poppler, network and terminfo tools as well as release prerequisites; enrollment prechecks before replacing a helper. Independent utility review fixes included gzip and RHEL sbin lookup. Distribution/release checks and signed deployment remain the next gate; see VALIDATION.md for evidence and honest limits.

v0.1.7 signed release installed on all three hosts; public clean-HOME bootstrap and live preview checks pass. Followup v0.1.8 narrows tmux absent-server detection after remote validation exposed HOME-relative wrapper errors. Optional user feedback pending: physical preview fit/return and Delete button clarity. No other new user decision is required.

## Preview polish and release descriptions
- User direction: prioritize larger image/PDF previews, keep j/k scrolling, use separately boxed confirmation choices. These decisions are resolved.
- Candidate v0.1.9 bundles language highlighting, native preview support and larger workspace. Real Foot cleanup defect fixed and independently inspected. Unknown terminals retain fallback. Signed v0.1.9/source1626cb7 published after workflow37494951757 PASS and installed on innovation/verybeautifulserver. Tranquility remains v0.1.8 while unreachable. Server PDF rendering is currently unavailable because Poppler reports a library I/O error; cx degrades explicitly.
- All published version descriptions now explain features/fixes; future publication requires exact-version CHANGELOG notes. Resolved user preference: release descriptions stay focused on Features and Fixes, without Try it workflows.

Optional physical feedback pending: v0.1.9 sharpness at normal Foot size and Escape restoring selection. Previous larger-preview/j-k/boxed-choice preferences remain resolved.

Missing-preview report investigated: live innovation viewer was still0.1.0. Exact old executable reproduces no raster; fresh installed0.1.9 Foot PNG/PDF workflow passes. User asked to reopen viewer and confirm same files; keep this focused confirmation pending rather than repeating general preview questions. Older orphaned viewers with closed terminals cleaned without stopping persistent sessions. Manual-bootstrap docs now explain reopening the viewer.

## Accessed-device updates and PDF pages
User clarified viewer is tranquility and previews started working after innovation was updated. Innovation's stale local viewer was a separate real finding, not proof of this remote workflow's cause. User requests automatic updates cover accessed devices; new background host checks use signed existing update backend and verify fresh helper version/identity before metadata replacement. No mutation replay or terminal/job restart. User confirmed one wheel step per PDF page; page counter and j/k/arrows/PgUpDn implemented. Independent review found no critical/high/medium issues; real PTY and Foot pages passed. Candidate0.1.10 pending signed release/live upgrade acceptance; currently all hosts have0.1.9 installed, with tranquility helper/image/PDF checks freshly PASS. Server Poppler library I/O error remains. Previous missing-preview clarification questions are resolved; do not repeat them.

## Signed v0.1.10 deployment and live endpoint updates

- Published source `1213ed3`, workflow37502598744 PASS: both static architectures, all nine distro/architecture cases. Innovation installed via exact signed public bootstrap; tranquility and verybeautifulserver upgraded automatically from0.1.9 by a real innovation viewer's authenticated Info checks. Both showed verified update notifications. All three now have0.1.10 installed; already-open older viewers may need reopening.
- Disposable shells on both remote hosts preserved session ID, PID, start ticks and boot identity across upgrade. Root stopped only those fixtures afterward using guarded StopSession; protected/user sessions unchanged. Server remains agent-free.
- Installed multi-page helper checks PASS on innovation/tranquility; actual page-color/keyboard/wheel PTY regression and clean-HOME public bootstrap PASS. Server PDF rendering remains unavailable due its existing Poppler library I/O error.
- Resolved preference: each PDF wheel step changes one page; retain j/k. Optional physical confirmation of the installed multi-page interaction is the only new feedback point. Full sharing and broader acceptance remain incomplete.

## Viewer clipboard integration

- Source `2acff83` adds OSC52 writes on cx's isolated tmux server, preserving built-in terminfo overrides without accumulation. Space/arrows/Enter or y copies keyboard selections; drag copies on release. Personal tmux remains untouched. Signed0.1.11 published, workflow37504474815 PASS: both architectures/all nine distro cases. Installed on innovation/verybeautifulserver; their existing cx-owned tmux configurations refreshed through disposable managed shells and cleaned. Tranquility still offline with0.1.10 installed; binary update pending reachability, while scoped Neovim configuration is already synced.
- ChezMoi `e3bf9a1` pushed and narrowly applied on both Arch hosts: Neovim selects its tmux clipboard provider only when TMUX identifies a cx-managed socket. Ordinary Neovim keeps normal display clipboard selection.
- Actual local Foot/Wayland Neovim yank/paste and actual Foot→SSH→server application copy/explicit clipboard query PASS. Local and server PTY bindings, return and configuration reload PASS. Tranquility went offline after config sync; initial remote physical test failed before Neovim provider evidence, and subsequent attempts BLOCKED No route to host. No trust/network changes.
- Inherited Neovim tmux-provider 50ms query wait can return a stale buffer on slow links; normal terminal paste is the reliable viewing-clipboard path. No clipboard polling/display forwarding. Optional drag-to-copy vs explicit-copy preference pending; current direction is drag copies on release.

## Stale innovation shells and metadata reconnect (2026-10-06)

- User clarified affected rows belong to innovation. Tranquility cache contained four ended innovation shells (`shell · scaling-law` plus three generated shell names); verified innovation tmux currently has only four live Codex sessions. Replaced tranquility's innovation cache with fresh authenticated live metadata; backup private on that host. No shell/provider processes stopped.
- Initial tranquility outage was a separate finding: three local cached tranquility shells are still running with original PIDs8046/9436/9917. Initially hid their offline snapshots locally, then restored them after verified live discovery. All three read-only/no-resize attachments and Ctrl+] return PASS, same process/start/boot identities. All four innovation live Codex read-only attachments likewise PASS, terminal content discarded.
- Tranquility open viewer PID18469 is0.1.4 while installed is0.1.11. Preserve it; user must Ctrl+C and reopen ~/.local/bin/cx to clear its in-memory stale rows. Stopped26 same-UID cx orphan viewers with PPID1/deleted stdin after exact executable/mode/start identity checks; no tmux/jobs targeted.
- Source536853a/tag0.1.12 adds one retry for closed Info/Sessions metadata channels only and canned SSH connectivity/auth/trust/helper classification with bounded stderr retention. Selected cached rows show failure plus explicit Refresh remedy. Added narrow rendering and inherited stderr cleanup regression in9238073. Integrated306 Rust tests PASS/one ignored; real framed CLI recovery/lost-mutation/offline/auth/trust tests PASS.
- Release37516433154 first ARM attempt FAIL unrelated updater synthetic replacement exec ETXTBSY; fixture isolated20 runs PASS. Fresh ARM rerun PASS; distro checks/publication/deployment pending. Do not claim the initial failure passed.

Signed0.1.12/source536853a published after workflow37516433154 attempt2 PASS (both architectures, nine distro/architecture cases). Signed bootstrap installed on all three hosts; tranquility's first noninteractive attempt safely refused missing tmux PATH, then succeeded with existing ~/.local/bin wrapper on PATH. Fresh installed session lists and all seven live session read-only/no-resize/return attachments PASS; original PID/start/boot identities preserved. Actual installed framed reconnect/no-mutation-replay fixture PASS. Saved stale cache is corrected; old open0.1.4 viewer still requires user reopening.

## Workspace/fleet browser milestone · v0.1.13 candidate
- Unified panel padding, transparent accent selection/focus, aligned colored footer and compact selected session/file/transfer context integrated. No-session states offer New session; local-only setup offers Add device.
- Network All aggregates bounded passive observations from enrolled hosts; owner/interface stays attached to neighbor actions. Explicit port checks distinguish TCP from SSH authentication/internet. Known targets can enroll via up to four SSH jumps; all transport/native/files paths reuse the route. Config-defined jumps also receive fail-closed background policy.
- Cross-architecture enrollment uses a signed foreign helper, never executes it on the viewer, and streams it to the host. Physical robot unavailable; existing host utilities remain prerequisite. Conflicting routes are refused rather than silently retargeting work. IPv6 link-local enrollment unavailable.
- Independent security/usability review findings corrected and re-reviewed. Integrated/live/fixture evidence and measured performance in VALIDATION.md. Candidate publishing and signed deployment are next.
- New optional questions pending: selected-context versus Actions sidebar space; whether robot workflow usually starts from neighbor browsing or known hostname/IP. Continue engineering while awaiting; do not repeat. Prior preferences remain in effect.

Signed v0.1.13 published from f4fd1db; workflow37525922390 passed both native builds and all nine distro cases. Installed/verified on all three hosts with identical signed hash; network observations/candidates verified live on each. New viewers use the improved UI; existing viewers/processes are preserved. Bootstrap followup fixes user-tmux detection outside SSH PATH; mouse test followup fixes an acknowledged-input timing race, without changing terminal behavior. Optional sidebar/robot-flow feedback remains pending; no blocking user decision.

## Network peers and enrollment prompts
Integrated enrolled peers independent of LAN cache, slim Add device panel and masked private OpenSSH askpass enrollment. Reviewed security findings corrected; transient authentication masters expire after ten idle minutes and contain no stored password. Cloudflare/LAN read-only evidence does not justify policy mutation. v0.1.14 installed all three hosts. Candidate0.1.15 awaits native multiplex fixture, signed pipeline and serialized host deployment. No new blocking user decision; physical password/MFA behavior remains explicitly unverified.

User clarified that SSH checks should be automatic. Candidate0.1.16 checks fresh observed LAN candidates quietly in background, bounded/cached, with direct Enter→account→masked authentication; manual Check SSH removed. Directory header spacing fixed and covered by two-project rendering. User's initial Network confirmation was premature on0.1.14; do not repeat the question before signed deployment.0.1.15 release pipeline37530479286 PASS, signed deployment/live PTY underway.0.1.16 integration/release next.
Signed0.1.15 installed on all three hosts; actual tranquility Network PTY now shows both enrolled peers (PASS). User may need to reopen an older viewer; no forced detach/restart.0.1.16 automatic-probe integration passes370 Rust tests (one protected live case ignored); final fixtures/release validation pending.


## Network routes and update clarity — candidate v0.1.17
- User clarified that LAN addresses are distinct devices, not duplicate server identities. Observer groups start collapsed; expansion persists across views within cx (resolved preference).
- Route information belongs in Selected: concise discovery path/interface in the sidebar, fuller saved SSH hops/status beneath the Network list. J/K scroll long details. No Evidence column or popup required.
- Update actions identify the running version; notices use consistent inset, transparent background and short ASCII-compatible wording.
- Integrated381 Rust tests pass (one protected live case ignored),98 UI tests in default/ASCII/NO_COLOR, synthetic auth/native encrypted-key/master/jump fixtures and three-size browser PTYs pass. Candidate innovation live Network/Update PTY passes. Independent placement/clipping findings fixed; final re-review and signed publication/deployment pending.

Signed v0.1.17/source `bd49e03` published; workflow37534796628 PASS (native checks, both static architectures, nine distro/architecture fixtures). Exact public bootstrap in clean private HOME PASS. Innovation and verybeautifulserver have verified0.1.17 installed, matching SHA256967f6c659132ec3fb0f2c5b11643d95fd036980fd9a9250d1d6ea7a7f7539f29. Server update discovery first returned offline preserving0.1.16; pinned signed bootstrap then succeeded. Tranquility last verified0.1.16; update/post-install testing BLOCKED by No route to host from both innovation and server. No network/trust/session changes were made to bypass this. Reopening an older viewer uses the installed update.

Final installed innovation/server Network+Update PTYs PASS. Server lacked a saved innovation registry entry: enrolled its existing authenticated alias with `cx add innovation`, then confirmed both peers visible; server remains agent-free. Tranquility's final check is still unreachable. Full sharing/controller mutation, complete trust revocation/uninstall and physical robot egress remain unfinished; this release addresses Network browsing/routes/update clarity only.


## File cursor cohesion — candidate v0.1.18
- User prefers a selector over underline but rejects the detached left rail and reversed white filename highlight. New cursor is a chevron immediately beside the filename, with foreground-only ANSI cyan/bold styling; inactive cursor is dim, marks remain separate and metadata brightens modestly on focus. Default backgrounds remain untouched.
- Compared chevron, compact brackets and whole-row color through captured design alternatives. Independent reviewer chose chevron: brackets cost narrow filename space, full-row accent competes with marks. No blocking issues; default/ASCII/NO_COLOR captures and actual Foot disposable Files movement/mark+advance/exit pass. Physical light-theme and Kitty/Ghostty contrast unverified.
- Integrated381 Rust tests pass (one intentionally ignored live/protected case),98 UI tests and120/80/48-column browser PTYs pass. Signed release/deployment next.

Signed v0.1.18/source `aba331d` published; workflow37536873542 PASS (native checks, both static architecture builds and nine distro/architecture fixtures). Clean-HOME exact public bootstrap PASS. Innovation/server signed auto-update PASS; both verify0.1.18/SHA256970a75eca93ddbfea6dad31f880ebd5e5ef25fbbe693469c7664b595208df6a0. Tranquility update BLOCKED by No route to host, still last verified0.1.16. Reopen an older viewer to use the installed selector; no terminal session restarted. Optional visual question pending: whether attached cyan/bold chevron is strong enough without the white background; do not repeat it.


## Session readability — implementation milestone
- Shell command-only labels, new Fish preexec observation, exact native metadata names and stable identity ordering integrated. Custom names remain unchanged; omitted CLI/UI names resolve on the execution helper after canonicalization.
- Independent review removed unsafe inherited-pane-title fallback and partial FD identity scans; read-only historical rollout handles excluded, fdinfo reads bounded. No provider configuration or personal shell/tmux files modified.
- Claude2.1.290 live native-name comparison PASS on innovation. Synthetic Codex writer/rename/ambiguity fixture PASS. Real Codex0.160.1 local /rename works but daemon-owned thread association remains BLOCKED; do not claim native naming of those running terminals. Installed versions unchanged pending release. Tranquility remains unreachable.
- Optional shell-name/detail preference already asked; pending, do not repeat. Conservative executable-only default used. Codex0.160.1 supports additive SessionStart hooks but launch-specific handlers require fresh native /hooks trust review; omitted from the default workflow. No hooks installed or trust changed.


Signed v0.1.19/source90c9b10 published after workflow37559092847 PASS (native tests, both static architectures, nine distro/architecture cases). Exact public bootstrap clean-HOME PASS. Innovation/server signed auto-update PASS, installed SHA25665ad4ce4f6f629e302d36e51f6ba29c397857817fb08b97f9314d698b50261f5. Six original innovation terminal PID/start/boot/socket identities preserved; installed Fish and metadata-writer fixtures, live Claude name and server Bash running/recent labels PASS. Server stays agent-free. Tranquility remains No route to host, last verified0.1.16. New Fish hooks apply to new shells; older shells use observed-command fallback.

Optional Codex naming-hook choice is pending: keep seamless startup/folder fallback versus opt-in instrumented launches with repeated native /hooks trust review. Asked once with the concrete installed-version tradeoff; do not repeat. Conservative fallback remains current behavior. An accidental extra prompt was explicitly withdrawn and requires no answer. Prior shell-subcommand preference is also pending; executable-only labels remain current behavior.


## Text preview navigation and search — v0.1.20 candidate
- Fixed G blanking text previews by clamping to actual wrapped content and viewport. Inline / search matches preview content live, highlighting exact occurrences or fuzzy subsequences; n/N wrap between results. File-tree selection/search remain separate.
- Pending previews cannot search placeholders; refresh retains the query and resizing reveals the selected match. Images/PDFs do not advertise text search. Terminal-default backgrounds and syntax styles are preserved.
- Independent read-only review loading/resize findings fixed and re-reviewed. Integrated402 Rust tests PASS/one protected live case ignored; source/PDF/search real PTYs PASS. Final optimized checks and signed release/deployment recorded in VALIDATION.md. No new blocking user decision.

Signed0.1.20/source d8a484a published; workflow37560669843 PASS. Exact public bootstrap and installed innovation preview/search PTYs PASS. Innovation/server verify0.1.20 and identical signed SHA256071272225fc9a441ef0d6f7c9ce8bf042392ab5a71890e9dd23549235b836810; six innovation terminal identities preserved. Tranquility deployment BLOCKED by No route to host, last verified0.1.16. Optional physical G/search confirmation pending; do not repeat before an answer. Rollback/deployment evidence in VALIDATION.md.


## Codex scroll fallback — v0.1.21 candidate
- Identified0/0 as empty tmux history while fullscreen Codex retains mouse-off startup policy. Forward genuine wheel bytes only to an unambiguous foreground Codex owner; generic empty histories remain live, native mouse-aware apps retain their input.
- Read-only and detached viewers cannot dispatch fallback; same-group editors/ambiguous terminal children block it. Existing empty Codex copy mode can recover on wheel. Actual0.160.1 disposable transcript scroll/draft/process test and synthetic regression pass, no inference prompts.
- tmux2.7 fixture passes guarded ordinary scrolling; event-format fallback supported on3.4+. Reviewer editor/observer findings corrected; final re-review and publication pending. No new blocking decision.

Unpublished0.1.21 first workflow37562442737 cancelled for reviewer worker-thread child-ownership finding. Fixed with bounded all-thread child enumeration/deduplication; real multithreaded regression red before/green after, actual Codex scrolling still PASS. Final reviewer no remaining high/medium issues; replacement candidate and signed deployment pending.

Signed0.1.21 source9c01b9b/workflow37562851745 PASS and installed innovation/server; actual installed Codex/worker-thread regressions pass, six original identities retained. User confirms problem is wheel/trackpad; physical check after install pending. Unavailable row error actually names peace@tranquility, currently No route to host from both reachable devices: preserved, not removed. All six innovation terminals pass full cx-over-SSH read-only attachment.

User requested three agents investigate native Codex names and inaccessible terminals. All investigations completed read-only; shared daemon API has names but no terminal-thread identity and includes prompt preview; do not guess by folder/recency or ingest it. Fixed trusted SessionStart handler plus supported per-thread environment marker merits optional prototype, but hook trust and daemon forwarding require actual verification. No hooks/config changed. User additionally reports unnamed Codex labels do not use folders; identity worker auditing current generated/explicit label distinction now. Empty persistent tmux server can retain old bindings across upgrade; live server scoped reload done, source regression/fix under test for next release.


## Session labels and empty-server refresh — v0.1.22 candidate
- Completed three read-only investigations. Codex's shared-daemon API supplies names but lacks a reliable terminal-to-current-thread mapping; do not infer identity from folder/recency or ingest prompt previews. Exact writable identity remains supported; native SessionStart plus per-thread marker is an optional, unverified direction, no hooks installed.
- Blank/whitespace saved names now use the same native lookup and execution-folder fallback as generated labels. Historical non-default labels remain preserved because their origin is unknown. Four of five live Codex rows already follow their execution folder; no current live fallback mismatch reproduced.
- Empty persistent managed servers now reload owned configuration before starting work. tmux2.7 rejects the previous exit-empty readiness probe; use portable global-options probing. Independent review found no high/medium issues.
- Pending physical wheel/trackpad confirmation remains requested once; no repeat question. Tranquility's unavailable execution rows are preserved; no innovation sessions removed.

Signed0.1.22/source64c41ab/workflow37563927815 PASS; exact public bootstrap PASS. Innovation/server installed0.1.22 with identical signed SHA256590ac5d7e447aacf4e23951de86b961d1d4a6f888953a3cd294cdb6a60cbfafe. Installed names/mouse and host-owned reload checks PASS, six original innovation session identities preserved; server remains agent-free. Tranquility still No route to host, deployment BLOCKED/last verified0.1.16. Final independent review no substantive findings. Physical scrolling confirmation remains pending; no new user question required.


## File find — v0.1.23 candidate
- User requests Yazi-style find rather than filter. `/` keeps visible entries/order, uses text-preview matching/highlighting, Enter finishes input, n/N cycle files with wraparound. f remains a separate filter. Active search owns n; Ctrl+P still offers New session.
- Marks/clipboard paths remain intact. Search count is per matching file, including repeated occurrences; no-match keeps the listing and selection. Long matching names keep index-aligned highlights and whole-name emphasis for clipped matches.
- Independent review caught clipped-index and invisible suffix-match issues; both corrected. Tests/release acceptance in VALIDATION.md. No new preference question needed: requested behavior is explicit.

File-find and transfer ecosystem three-size PTYs PASS; preview search palette/monochrome PASS. Optional shortcut question pending: dedicated New session shortcut while n/N own active file search; current behavior follows explicit user request and Ctrl+P remains available.

Resolved: user confirms n/N cycle search matches. Final independent review no substantive findings after monochrome correction. Release retry addresses existing parallel FD/inode-reuse test race; production updater unchanged.

Signed v0.1.23/source4c3c3f0/workflow37614097212 PASS. Innovation/server installed0.1.23 with SHA256148945ac75ff67cda8aa75f85c5cccaf1cace695e91c2edf63a840da7e244f5b. Exact clean-HOME public bootstrap and installed three-size find PTYs PASS; three pre-update managed terminal identities preserved. Server remains agent-free. Tranquility SSH unreachable, update BLOCKED/last verified0.1.16. User shortcut decision resolved: n/N cycle active search. Physical GUI feedback optional, no repeated request.


## Markdown underscore emphasis — v0.1.24 candidate
- Report reproduced: _text_ remains plain because the small inline renderer recognizes only stars/backticks. Added underscore italic/bold with UTF-8 boundaries; identifiers, escapes and inline code stay literal.
- Real PTY also exposed monochrome bypass of Markdown formatting. Preserve Markdown modifiers with foreground/background stripped in NO_COLOR/ASCII. Code previews retain their existing monochrome behavior. No new user decision needed.

Signed v0.1.24/source5e02398/workflow37615556311 PASS, installed innovation/server with verified SHA256cc7421e484a5c7d5c7cef10ec337626daaa3e492f7178ddfb04808f89084d05c. Exact public bootstrap and installed Markdown italic/bold/default-background PTYs PASS in color+monochrome. Three protected managed session identities preserved; server remains agent-free. Tranquility SSH unavailable, update BLOCKED/last verified0.1.16. No pending user choice.


## Markdown tables and code blocks — v0.1.25 candidate
- Bounded inert table renderer integrated: Unicode display-width columns, header/separator alignment, escaped/code pipes, inline emphasis, and responsive labeled rows with no discarded cell data. Cache by source/revision/viewport; logical source-line count preserved.
- Fenced code has muted boundaries/gutter and language label. Bundled grammars handle common aliases; unknown languages stay plain, closing marker/length must match, no interpreter/process execution.
- Final integrated suite: 417 tests PASS, one protected live test ignored. Independent final review found no substantive unresolved findings.
- Optional preference pending: labeled rows versus horizontal grid scrolling in narrow panes. Labeled rows remain current default after reasonable opportunity to answer; no repeated question.

Both review P2 findings reproduced and corrected with regressions: table/fence boundary and stale search index after resize. Reviewed113 UI tests and palette/monochrome PTYs PASS. Final re-review found no substantive unresolved findings; not published yet.

Release fixture synchronization: two unpublished CI candidates failed partial-frame/input timing checks. Second failure reproduced locally with debug build. Explicit input/frame barriers now PASS debug and optimized actual PTYs, with all assertions retained. No production renderer change in this correction.

Signed v0.1.25/source2b71421/workflow37623525992 PASS; exact clean-HOME public bootstrap PASS. Innovation/server installed0.1.25, matching signed SHA25660c6a121e157cc5220646b0090e60b6650b5efad22428c4b43ba2c306da0d45d. Installed Markdown blocks PTYs PASS color+monochrome; three original managed identities preserved; server remains agent-free. Tranquility No route to host, update BLOCKED/last verified0.1.16. Optional narrow-table preference remains pending, labeled-row default retained.

## Markdown links, lists and math — v0.1.26 candidate
- Inert inline-link labels styled/underlined. Click/o opens an inspectable target picker; Enter explicitly sends HTTP/HTTPS URLs to the viewer graphical browser. Other links stay inspectable, never executed; remote helpers do not launch browsers.
- Hanging list body wrapping uses Ratatui's exact Unicode/word layout. Ordered/nested/lazy/indented continuation lines included, protected fences/tables preserved. Regression reproduced red before fix.
- Lightweight bundled Unicode math implemented without dependencies/interpreters. Delimiters inline/display dollar and brackets; unknown syntax and ASCII mode retain source. Seven parser tests include 32,768 malformed cases. This is not full TeX layout.
- Independent review caught search underline falsely clickable; corrected hit regions from unhighlighted presentation with regression. Actual debug PTY passes color+ASCII, mouse/keyboard links, hanging indent, math and terminal restoration. Heading-link bypass also corrected. Reviewed428 tests plus optimized/debug prose and existing table/code PTYs PASS; release validation ongoing.
- Resolved user preference: keep lightweight built-in math; no full equation-image tooling planned.

Signed v0.1.26/source1cf0e2d/workflow37649848069 PASS: native/PTY checks, both static architectures and nine distro installation cases. Exact public clean-HOME bootstrap PASS. Innovation/server installed0.1.26 with matching signed SHA2569ccb1eb80bd632578348dfb987b41cc66a1aee2b58027f7c7adb5a95183acbb3; installed prose and table/code PTYs PASS, three original managed identities unchanged, server remains agent-free. Binary increase58,496bytes (0.53%), no dependencies added. Tranquility No route to host, update BLOCKED/last verified0.1.16.

Server updater initially failed safely because an inode-derived rollback filename already existed. Preserved the conflicting owned backup with no overwrite/delete under maintenance lock, then normal signed updater retry PASS. Known updater limitation: reused inode rollback-name collision can require this recovery; durable naming improvement remains separate follow-up work. Markdown renderer final independent re-review has no unresolved findings. Optional physical viewer-browser check asked once and pending.

## Fresh shell labels — v0.1.27 candidate
- New unnamed shells display New shell until observed activity supplies running/last/recent labels. Older UI-generated shell-key names recognized only by matching creation-key hash; custom names preserved. Disposable real Fish regression red before, green after, same terminal and no command arguments exposed.
- Also resolve known updater rollback-name collision: exclusive fresh private backup allocation on existing filename, no overwrite/adoption of old backups. Synthetic collision regression reproduced red, then corrected. Release/review validation ongoing.
- Relative Markdown links now resolve from the document directory on its execution host, with parent segments preserved for correct symlink traversal. Percent-encoded paths and heading anchors supported; Escape restores bounded parent-preview scroll/search while folder navigation clears history.
- Ordinary click/o inspects exact destinations, including repeated labels and table links; Shift/Ctrl click opens. Foot normally reserves Shift for text selection, so Ctrl click is the direct fallback without changing personal bindings. No new dependencies.
- Independent review reproduced/fixed wrong symlink-parent resolution and stale cached preview history. Final tests and release validation underway; physical modifier behavior remains unverified.

Signed v0.1.27/sourcef6baeb8/[workflow37702878059](https://github.com/1unarzDev/cx/actions/runs/37702878059) PASS:434 local Rust tests, native/PTY CI, x86/ARM static builds and nine distro installation cases. Exact public bootstrap clean-HOME PASS. Innovation signed updater PASS; server old0.1.26 updater failed safely on known collision, migrated successfully through signed bootstrap without modifying old backups. Both installed0.1.27/SHA25658407258f0919942d750213f889bb7d8690c24da92a619aecd2e2f9c7d4f3092 and report current. Installed local palette/monochrome links/history/termios and fresh Fish labels PASS; installed real server SSH-relative links PASS. All five recorded innovation managed identities unchanged; server remains agent-free. Static binary+36,720bytes/0.33%, no added dependencies. Tranquility SSH No route to host; deployment BLOCKED/last verified0.1.16.

Optional physical relative-link/keyboard-back/Ctrl-click check asked once; awaiting user confirmation. Foot’s reserved Shift selection is preserved. Full product acceptance remains incomplete as listed above. Rollback: signed bootstrap CX_VERSION=v0.1.26 CX_NO_LAUNCH=1, preserving sessions/jobs.

## Inherited tmux discovery — v0.1.28 candidate
User corrected affected execution host to innovation. Exact shell-329185-179126… label is absent from its current live sessions; the available viewer cache associates that label with tranquility, so that specific row remains unverified. Innovation authenticated SSH from server succeeds. Local self-alias SSH fails host-key verification, unchanged; local cx uses direct local execution and does not need self-SSH.

Reproduced separate real innovation attachment defect: viewer inside managed tmux inherits TMUX; external discovery implicitly chooses that same socket and creates duplicate $N/None-socket records which remote default-server attachment cannot enter. Fixed explicit environment isolation for external discovery/inspection/local attachment and remote native external helper. Managed socket/path and personal bindings untouched. Disposable two-server regression red before/green after verifies identical inside/outside metadata, real default external retained, identities unchanged, actual native external PTY and termios restoration. Release validation underway; no claim that unlocated old shell is recovered.

User confirms old shell-329185-179126… row disappears after Refresh. No terminal deletion/replacement needed. Separate duplicate-session defect fixed and signed v0.1.28 published after workflow37706817950 PASS (native/PTY, both architectures and distro installation checks). Exact clean-HOME public installer PASS. Innovation and server installed0.1.28/SHA256c8508b5ce7f7ce3d45513d47f88ad7a040addbae301a2c64d758265bbe307033. Innovation now reports five real managed rows rather than ten mixed rows, all five identities preserved. Actual installed disposable external-discovery/native-attachment PTY PASS. Tranquility remains offline/pending. Independent final review84822d5 CLEAR. User's new list-scrolling skips are the next active investigation.


## Wheel navigation — v0.1.29
- User confirms skipping is wheel/trackpad only. Foot 1.28 alternate-screen fallback multiplies arrow bursts; cx now captures wheel events directly across lists, with one adjacent item per reported event and hovered-panel focus. Native attachment releases capture and return restores it; keyboard navigation unchanged.
- Integrated debug/optimized Files/Work PTYs PASS, including real native attachment, process identity preservation and terminal restoration. Full suite 438 PASS, one protected live test ignored. Independent final review CLEAR b58b71e.
- Review fixes: register fullscreen two-location PDF mouse target; confirmation wheel scrolls details without arming Delete/Stop/Exit. Release/deployment follows; physical wheel check pending until installation.
