# CX status

## Hover previews, availability and loader alignment — 0.1.89

Fixed a capability ingestion bug: Info replies discarded both live-preview capabilities, disabling previews as soon as availability returned. Added render-derived session-row mouse hit boxes, including group spacers and table scroll offset. Hover selects without attaching. Preview moves below the list in narrower layouts; outside-tmux rows explain that Enter moves the terminal before screen access.

External-terminal open intent now survives its capability check and resumes automatically; navigation/Escape, endpoint changes, failures and a 12-second pending deadline cancel it. Capability probes have a separate scheduler/transport lane from inventory. Relay -v subprocesses are bounded to 250ms/128 bytes with owned process cleanup. Loader text is centered vertically inside its panel.

Local installed relay was observed as user-local without file capabilities; its version returned immediately. A read-only actual local Info check returned in113ms with Claude/Codex and password-requiring adoption available. Disposable actual adoption and fake-sudo/askpass fixtures passed; no personal shells or privileges were changed. Real user terminal/emulator specifics remain unvalidated. The earlier hypothetical capability-relay dumpability issue was not reproduced and no privileged helper policy was changed.

## Integrated session terminals and unified creation — 0.1.88

Deployed locally from `6fc6411` to `~/.local/bin/cx` under the maintenance lock. Installed version and build digest verified; rollback `~/.local/bin/cx.rollback-0.1.87-live-pane`. Existing sessions remain running.

The user rejected clipped text snapshots. The session pane now displays the full styled current tmux screen with blank rows, cursor and alternate-screen content; its viewport follows the cursor. Enter focuses the existing pane within CX, Ctrl+] returns navigation, Ctrl+O opens native attachment. Input binds to device/server/session/pane, not process identity or title, and a same-queue generation guard prevents stale input delivery. Focus survives a shell starting an agent. Shell is now a choice in New session, with duplicate sidebar/palette creation entries removed.

Limits: original terminal geometry is preserved; oversized content is viewport-clipped. No mouse forwarding/scrollback/terminal emulation; native attach remains available. Pasted input is bounded to 4096 bytes including mode markers. Remote helpers need `live-session-pane-v1`; no remote deployment is included. Physical acceptance beyond synthetic PTYs and real provider-specific keybindings remains separate.

## Session labels and read-only previews — 0.1.87

Implemented in `worktrees/session-previews-20261024` and installed locally from `0dd91e0` as 0.1.87. Installed SHA-256 matches the optimized build; previous binary saved at `~/.local/bin/cx.rollback-0.1.86-session-previews`. No remote rollout or session restart. Session titles retain exact provider metadata; generic labels show folder/provider/compact runtime identity. Wide session views show wrapped context and a read-only current-screen snapshot. Narrow layouts keep context below the list. Snapshot fetching is selected-only, bounded, asynchronous, separated from remote inventory traffic and discarded on selection/runtime change. No snapshot text is persisted. Mouse interaction over snapshots cannot activate or scroll session rows.

Limits: no interactive embedded terminal or scrollback; screen text is clipped and plain, up to 60 current-screen lines/24 KiB, with a two-second refresh interval. Unsupported helpers and bare terminals remain metadata-only. Container tmux screens work through their managed host runtime; inner-container/custom sockets remain future work. Remote helpers require `session-preview-v1`. Provider history selection remains native; this does not introduce a transcript index or guessed conversation matching.

## Local deployment integration — 0.1.86

Integrated container parity (`5863f5a`) and session workspace (`970f9f8`) with the installed release baseline `v0.1.85` (`ff7ebdc`) on `deploy/session-workspace-20261024`. This preserves external terminal adoption, task/activity metadata, command panel and startup/handoff improvements. A failed session refresh now retains stale navigation rows, clears transient activity, and requires a new successful check before attachment.

Deployed locally from integration commit `bef1def`: `/home/lunarz/.local/bin/cx` now reports 0.1.86 and matches the optimized build SHA-256. Atomic replacement used the maintenance lock; rollback is `/home/lunarz/.local/bin/cx.rollback-0.1.85-session-deploy`. No public release, push, remote-helper rollout, terminal restart or personal network/trust changes. Earlier milestone deployment notes below are historical. Remote endpoints need a matching helper for container agent parity. Native provider authentication/history and physical terminal acceptance remain unvalidated; custom/inner-container tmux remains future work.

## Container agent launch/history parity — 2026-10-24

Implemented on `feat/container-agent-parity-20261024` in `worktrees/container-agent-parity-20261024`, based on switcher commit `970f9f8`. Container session choices now expose installed Claude/Codex profiles scoped to the exact device/container/user/folder. Existing runtimes, fresh work and the provider's native history picker remain separate choices. Default/YOLO intent survives both launch hops; container history uses `codex resume` / `claude --resume`, never latest. Provider checks run no agent and install nothing. Login-startup directory changes cannot redirect agent launches from the selected folder.

Version bumped to 0.1.60 so container-private helpers receive the matching decoder. New `container-agents-v1` capability and versioned agent-create operation prevent old helpers from silently dropping history intent. Runtime tests verify actual arguments/cwd with synthetic agents in a disposable network-none container. [Validation](VALIDATION.md) records evidence and limits.

Integration/acceptance remaining:

- Changes are isolated and committed; no installed binary replacement, publication or remote deployment. The root checkout's pre-existing conflicts remain untouched. Update execution helpers when integrating.
- Real provider authentication/history selection, physical terminal acceptance, actual Node Dev Containers CLI environment behavior and remote robot acceptance remain separate from this synthetic Docker validation.
- Custom/inner-container tmux and nested-client switching remain future work. Exact provider conversation IDs are still selected by the provider picker; CX does not merge separate conversations by folder/title.


## Unified session workspace — 2026-10-23

Implemented in `worktrees/session-workspace-20261023`, branch `feat/session-workspace-20261023`: named runtime selection, all sessions with pins first, persistent unavailable-work bookmarks, folder agent presence, cached navigation, obsolete-read coalescing, alias deduplication, dead-pane lifecycle handling and checked native attachment. Pins reopen saved folders deliberately; no guessed-idle hiding or automatic agent restart. The previous latest/fresh/past launch prompt is removed.

Full Rust suite, isolated runtime lifecycle cases, managed/external attach/watch PTYs delayed-host browser navigation and actual pin/cache viewer-restart checks passed. [VALIDATION.md](VALIDATION.md) records exact evidence and limits. [Feature priorities](docs/astra-improvement-proposals.md) includes the dedicated QoL ideation.

Current unresolved integration/acceptance:

- The original root checkout is detached with pre-existing conflicts. This isolated branch is committed separately; no release, installed binary replacement or remote deployment is included. Updated endpoint helpers provide stable terminal generations; legacy observations retain legacy attachment behavior.
- Container agent launch and resume parity is implemented in the next milestone above. Custom sockets, inner-container tmux, nested-client handoff and exact provider conversation history remain gaps. No global terminal hook was installed.
- Physical emulator/provider/robot acceptance remains separate from synthetic providers, fake SSH/Docker scope checks and disposable tmux PTYs. Tests cover an explicit matrix, not every possible configuration.


## Session workflow exploration 2026 10 23

The user prioritizes agentic coding across machines and native-feeling management of all shells/agents, including sessions created outside CX. Three Astra specialists reviewed committed snapshot `c60e1a2`; findings and ranked features are in [session management and feature priorities](docs/astra-improvement-proposals.md). This isolated branch contains proposals only. `cargo check --locked --offline` and all 175 selected UI tests passed; no live sessions, shell configuration or networking were changed.

Before implementation: resolve the root checkout's existing integration conflicts; define attachment endpoint versus lifecycle ownership; demonstrate nested-client switching and physical terminal behavior before offering optional tmux-backed terminal integration. No further workflow-priority answer is needed.

## Device markers and session choices (v0.1.56)
## Automatic core mesh — v0.1.76

Enrolling/promoting a core device, opening CX, or a background retry reconciles
reciprocal SSH access with every other core device. Directed devices are excluded.
Each host retains its own dedicated private key. Managed host pins stay in CX state;
personal SSH config/known_hosts are unchanged. New directories use mode 700 even
under umask 0002; safe empty legacy roots can recover without rotating keys.

LIVE: Innovation, Creation and Tranquility each completed real session-list requests
to both other hosts: all six directions passed. Ordinary startup formed the mesh
after Tranquility woke up; repeated sync reports all three pairs ready with no
pending edges. Tranquility's legacy Innovation entry was migrated from Directed to
Core, and Creation's empty v0.1.75 managed directories were repaired. All original
session identities and personal SSH configuration hashes are preserved.
Signed v0.1.76 is installed on all three, and all six session-list requests
passed again after deployment. Release validation passed on retry. No provider/tmux restart or live networking change occurred.

## Command output panel (Unreleased)

`:` commands run in a managed pane shown in a floating panel (live styled capture, input including Ctrl+C, Esc to close, Ctrl+O to expand, exit status). Panel traffic uses its own worker lane so captures stay responsive behind slow refreshes. Covered by a helper fixture and a real-PTY fixture on bash and fish (both in CI).

## Session adoption (Unreleased, `feat/session-adoption`)

Outside-CX terminals are listed and can be moved into managed tmux through the guarded cx-reptyr relay. Adoption live fixtures run locally (ptrace sandbox), not CI. Synthetic busy multithreaded agents kept their identities, saw no hangup or EIO, and output paused at most about 14 ms over repeated randomized moves. Target exits, relay kills and forged identities were injected and left nothing behind. Pending:
- Real Claude/Codex mid-turn on a real terminal and over SSH.
- A real move through the sudo password panel, and the privileged `cx device ptrace enable`, on a real host.
- A signed cx-reptyr release asset.
- Fallback resume for hosts without ptrace.
- In-container (shim-held) terminals.

## Session behavior already released

Startup-folder navigation, background cache reconciliation, dead-shell filtering,
container lifecycle checks and rediscovery of tmux-backed Docker exec terminals are
implemented. Native agent names/directories/tasks refresh with identity checks;
supported Codex launches use separate processes. Sessions show observed activity and
offer task details with `s`. Terminal startup shows an animated loader. Codex falls
back to a neutral provider label; Claude preserves terminal transparency.

## Remaining integration and acceptance

- Blastoise and Squirtle remain Directed and were unreachable during earlier bounded
  checks. Physical reconnect/attach acceptance needs them online and updated helpers.
- Silent model/network waits and silent inner-container commands have unknown
  activity; unsupported or ambiguous native metadata remains unavailable.

Removal/demotion prevents future enrollment but does not revoke existing SSH keys.
Automatic setup requires supported helpers and direct port-22 connectivity; offline
peers remain pending and retry. Changed identity/key pins require verification.
Work was integrated through an isolated worktree; the original conflicted checkout
was left untouched. Test evidence is in [VALIDATION.md](VALIDATION.md); prior design
findings are in [the audit](research/session-reliability-audit.md).
