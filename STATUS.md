# cx status

Implementation is in progress, not release ready. Local project has no GitHub remote; public publication is not authorized. Baseline f30bfa2; three native workers use isolated build branches under worktrees.

Working inventory: innovation Arch/tmux 3.7c/Codex 0.160.1/Claude 2.1.290; tranquility Arch with system Codex 0.160.1 but Fish selects user Codex 0.157.1, npm installed, user-owned signature-verified tmux added; server Ubuntu 24.04/tmux 3.4, no AI runtimes. Existing rideshare_planning untouched. Five inter-device SSH directions work; innovation self-alias trust fails and is not modified. Linger is disabled on both Arch accounts. These observations are local deployment facts, not product defaults.

Root owns shared model, transport, persistence, manifests, integration and deployment. Workers own sessions; UI; files/read-only network. Live network changes and trust changes remain serialized and gated by actual trust/privilege requirements. The legacy robotics shared_internet.sh replaces the host default route and appends unowned firewall rules; it is inspected, not executed or changed.

## User feedback loop
Milestones: build → integrated test → concrete result → focused high-value questions → incorporate. Routine engineering remains autonomous. At each milestone assess workflow assumptions, UI alternatives, physical behavior, costly reversals, and tradeoffs. Internally classify Blocking / High value / Polish. Only Blocking interrupts dependent work; other work continues. Answers become product direction and go to relevant workers.

Current user direction (validated first live milestone):
- Native attach/return initially worked on the user's terminal; return shortcut was not discoverable and Ctrl+C did not mean return. Preserve native Ctrl+C; permanently show Ctrl+] then Space in managed status and provide prefix-help overlay.
- Search must be fuzzy, live on every edit, and inline in a bottom textbox. Implemented in Work and Files; Enter opens selected match, arrows select, printable navigation characters remain input text.
- Keep sidebar for now but use its empty space better. Slimmed it, added keyboard-accessible device actions and selected execution/workspace context.
- Visuals need more intention, aligned rows and restrained colors/status indicators. Integrated pass uses fixed table columns, project separation, ANSI accents and explicit indicators with default terminal background.

Pending: POLISH — review the updated visual/search/return-hint build after showing an actual metadata-only PTY capture. No repeat of answered sidebar/search questions.

- Bottom hints now use a two-row, three-column grid with a shared six-column key slot and transparent background. Enter/Ctrl+P and Search/Help align; 80/120-column real PTY captures verified in local-evidence/ui-{80,120}.png. Local installed binary updated; reopen viewers to load it.

- Folder navigation: Left/h parent, Right/l enter selected directory, Enter directory/preview. Shift+Tab now reverses the device/action/workspace focus cycle and exits search to the preceding focus. Input text retains printable h/l. Regression tests reproduce old failures and now pass.
- Shell return: user confirms it works. Isolated managed-shell PTY test also passes Ctrl+] Space, same selection, shell survival and terminal restoration. Native shell keys remain untouched.

- Unicode attachment: tranquility noninteractive SSH reports ASCII locale (LANG/LC_CTYPE unset). Managed/local/remote tmux clients and owned server now use documented -u mode. Synthetic Unicode regression failed before and passes after under LANG=C/LC_ALL=C; no locale or personal tmux configuration changed. Font coverage remains a viewer-terminal responsibility, and Codex output is not rewritten.
