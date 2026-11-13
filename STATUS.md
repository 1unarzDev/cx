# CX status

## v0.2.2 release candidate (branch `revamp/base`; not tagged, pushed or deployed)
- Fixes from daily use of v0.2.1 (CHANGELOG v0.2.2): half terminals (`hub.rs:43` clamp min > max; every view now renders at every size 20x5..250x80), blank hub thumbnails (only the selected session was captured), `✕ 20770d` for never-seen devices, leaked attach guards (removed after the check; startup sweep of stale unowned ones), transparent-friendly chrome (no cell backgrounds), curved links, bigger previews.
- User feedback round 2 (all done): `␣ ⏎ ⌫` key labels (`spc ret bksp` in ASCII); yazi-style rounded key pills only where the terminal draws U+E0B4/E0B6 itself (kitty, ghostty, wezterm; foot needs `CX_NERD_FONT=1`), else bold keys with nothing behind; no `NETWORK lan` strip; the palette's arrows/Ctrl+N/P move the selection it draws (root cause: it drew the hub's `caps.selected`) with a text cursor in every find field; the hub and device grids and the map walk in 2D with arrows and `h j k l` (Tab still switches zones; a tap focuses a tile, a second tap attaches).
- Gates (2026-11-13, at the round-3 head; host load average 17-36 on 20 cores from other workers throughout): `cargo fmt --check`; clippy --all-targets with the same 82 summary lines and the same warning kinds and per-target counts as 295020d (two redundant closures this round added were fixed); `cargo test --locked -- --test-threads=4` 679+55+4+148+112+25 = 1023 passed, 0 failed, 5 ignored (at the default thread count the timing tests listed below fail under this load and pass alone); the ignored full matrices `every_view_renders_at_every_size` and `every_overlay_renders_at_every_size` (release with overflow checks and debug assertions) passed in 811 s; release build without warnings; tests/run_regressions.sh --no-rust --with-ssh 67/67 (an earlier run failed 3: the crumb change broke tests that found the hub by `cx › network`, fixed; peer_mesh_cli timed out under load and passed twice alone); the half-terminal repro against real device state at 12 sizes (80x48, 95x50, 60x40, 190x25, 40x20, 120x40, 190x50, 20x5, 19x20, 10x5, 100x24, 250x80) alive at each; the verifier's command-panel repro (`/`, `run: echo hi`, Enter) alive at 12, 16, 19 and 30 columns (v0.2.1 panics at 19); capture_matrix dark, light, 16-colour (TERM=xterm) and ASCII at 40x20, 80x48, 95x50, 120x40, 190x50: 4 x 65 PASS, 0 unmasked, in ~/.cache/cx-cap-022/{dark,light,c16,ascii}; orphans: one cx-perf tmux unit left by the first suite run's timed-out session_watch_live was stopped and its dir removed; an older cx-perf unit (00:03) and the cx-fb / cx-perf runs still active belong to another worker and were left alone; `cx orphans` reports no stray process.
- transfer_enter_pty@container (docker; outside the --with-ssh gate) fails here on v0.2.1 and on v0.2.2 alike.
- A/B vs v0.2.1 (release builds, interleaved, ~/.cache/cx-ab-022): perf_harness 6 runs a side over two rounds (the second with v0.2.2 first), files_bench 3 x 2 runs, then focused reruns (4 x 2) of every flagged files metric. The folder column first indexed the whole highlighted folder (100k warm first rows 11 → 27 ms); it now reads the listing's first rows, and a build without it confirmed the attribution. Medians:

  | metric | v0.2.1 | v0.2.2 |
  |---|---|---|
  | select → screen (peek), local / SSH | 18.0 / 18.9 ms | 10.6 / 12.8 ms |
  | keystroke echo, local / SSH | 3.6 / 3.9 ms | 3.8 / 4.1 ms |
  | Enter → attached, local / SSH | 21.8 / 59.4 ms | 21.2 / 60.9 ms |
  | detach → cx, local / SSH | 48.4 / 60.4 ms | 45.1 / 49.1 ms |
  | 20k listing first rows (perf_harness) | 10.9 ms | 2.1 ms |
  | first rows cold 100 / 1k / 20k / 100k | 9.4 / 8.9 / 9.9 / 9.9 ms | 9.9 / 10.7 / 13.8 / 11.5 ms |
  | sibling folder 20k / 100k | 34.4 / 97.9 ms | 8.1 / 13.8 ms |
  | warm 100k | 10.9 ms | 10.8 ms |
  | preview first paint src 5 MB / md 2 MB | 17.1 / 10.3 ms | 14.0 / 9.4 ms |
  | preview scroll j, md 2 MB / src 5 MB | 4.4 / 11.0 ms | 4.7 / 11.4 ms |
  | markdown open | 31.7 ms | 32.5 ms (round 1: 39.9 → 33.8, round 2: 30.5 → 32.3) |
  | viewer start → ready | 33.6 ms | 32.8 ms |

  Not a clean sweep: echo (+0.2 ms), scroll j (+0.3/0.4 ms), SSH Enter (+1.5 ms) and cold first rows (+0.5 to 3.9 ms) have higher medians. These are within the spread between rounds on this loaded host (markdown open changed direction between rounds), except cold first rows: a build without the folder column measured lower there (10.1 / 9.1 / 9.6 / 8.2 ms), so the column costs a few ms on the frame that enters a folder (clearing its rows). It stays far inside the 50 ms target, and the sibling case is 7x faster.
- Answered for the user: secrets reach agents only when asked for. cx injects nothing into claude/codex/shell sessions; `cx secrets get NAME --exec -- cmd` puts values into that one child's environment (`--fd N` writes one to a descriptor), values live in the OS keyring or the 0600 file, and cx itself reads only `cloudflare.api_token` (for mesh.py).
- Review round 3 (verifier and visual critic on 9ebb0f1), all fixed: the command panel's inverted clamp under 20 columns (`rendering.rs:807`; choosers and the context menu also drew past tiny frames), found by a new sweep of every App overlay from 1x1 (`every_overlay_renders_across_sizes`, ignored full matrix `every_overlay_renders_at_every_size`); the dead background code (`has_surface`, `wash`, `surface`, `sel_bg`, `tile_bg`, AccentBg/SelectDimBg/Surface2) deleted; the help legend draws the map's strokes (braille, `▶`, `⊘`); a device's LINKS are a zone (Tab, Down from the bottom tiles; Enter edits the link; Up from the top tile row stays); the hub always has one large tile (the first while the map has focus; Tab focuses it); new PTY test map_grid_pty (map by place, device grid in 2D, LINKS, tap focus; red on v0.2.1); shell_labels_live's flake was real: a pane starts as `exec env … shell`, so a listing in that window labelled it `recent: env` (now excluded; the test also waits for the prompt); grid_keys_pty waits for the attention order to settle; Esc, a key, Esc is no double Esc; no toast for background failures the ticker already reports; folder previews in Files; rounded code frames and bright-blue syntax; `h` for a new secret in the keys body; the crumb names the hub's focused zone; footers keep `? help`; `←↑↓→ move` hints; tighter chips; help rows that fit at 95 columns; pty_lib decodes UTF-8 incrementally.
- Kept (with reason): `<letter> then  a attach` keeps the word `then` (the user locked the nebula mockup's hint wording, .decisions.md 2026-11-12). A device's LINKS star draws two links as straight lines when both sit on the root's row: nothing turns, so there is no corner to round. Up from a lower map corner reaches the node straight above it, not home, so Down then Up is not always a round trip; Left/Right are (map_grid_pty checks both).
- Flaky under host load (load average 30+ on 20 cores), green when rerun alone: files::tests::twenty_thousand_entry_listing_is_fast_sorted_and_matches_old_pages (1 s budget), syntax_preview fence colours (500 ms highlight budget), mesh a_cancelled_invite_closes_its_listener_promptly.

## v0.2.1 release candidate (branch `revamp/base`; not tagged, pushed or deployed)
- Integrated the strict v0.2.0 review (159 confirmed issues) from fix/021-{tests,transport,mesh,sessions,files,views}, then fixed every recheck still broken: S8 (watch ids in replies and frames), V2 (`g` is a late device letter), F2 (markdown list rows skip the wrap pass), F15 (the bench, not cx: its screen readers rebuilt pyte's display per line), T3 (duplicate `portable` job; tests/workflow_keys.py), T9 (container chooser `[n]`).
- Decisions applied: inline typing kept and escapable (Ctrl+\, Esc Esc, any tap outside the screen; concept-mission section 6); startup measured with `CX_THEME` set by the bench (OSC 11 probe unanswered in the bench PTY); palette Send Ctrl+\ works from the hub (selects the last session first if its screen is not live).
- Gates (2026-11-12, at 76b44f5 and after): fmt; clippy with no new warning kinds vs 0e4086d (117 warnings, down from 164); `cargo test --locked` twice, 1010 passed / 0 failed / 3 ignored each; release build with no warnings; tests/run_regressions.sh --no-rust --with-ssh 64/64; session_watch_live against v0.1.95 (control-kill, 100 cycles, old helper, old viewer) and against v0.2.0 in both directions (echo works; both sides watch); ubuntu:24.04 reproduction of release.yml checks (Node 22.23.5) all steps green, regressions 70/70 with SSH and docker; static musl x86_64 15.97 MB, aarch64 15.47 MB; capture_matrix dark (52, only the known keys-screen false positive) and ASCII (52, PASS); orphan check: one fixture sshd this run leaked (session_watch_live with a mistyped `--old` path) was stopped and the fixture now cleans up a failed start; a `cx-tmux-*` user unit for a deleted `/tmp/cx-perf-*` HOME, started 02:21 by the review run, is left for root.
- A/B vs v0.2.0 (0e4086d release build), same harness for both (it now sets CX_THEME, and its screen readers no longer add ~40 ms per check, so first-rows numbers are not comparable with the v0.2.0 table below); medians, perf_harness 3 interleaved runs, files_bench 3 x 2 runs:

  | metric | v0.2.0 | v0.2.1 |
  |---|---|---|
  | select → screen (peek), local / SSH | 6.5 / 7.3 ms | 6.6 / 7.4 ms |
  | keystroke echo, local / SSH | 1.8 / 2.0 ms | 1.6 / 2.0 ms |
  | Enter → attached, local / SSH | 12.9 / 31.3 ms | 12.6 / 29.5 ms |
  | detach → cx, local / SSH | 22.2 / 21.3 ms | 21.1 / 21.4 ms |
  | first rows 1k / 20k / 100k (cold) | 3.7 / 3.6 / 3.7 ms | 3.6 / 3.7 / 3.7 ms |
  | 100k sibling folder | 47.7 ms | 45.4 ms |
  | preview first paint src 5 MB / md 2 MB | 4.3 / 7.0 ms | 5.0 / 4.1 ms |
  | G on md 2 MB right after opening | 101 ms | 5.2 ms |
  | markdown open (perf_harness) | 46.9 ms | 23.3 ms |
  | preview scroll j, src 5 MB / md 2 MB | 4.5 / 3.5 ms | 4.5 / 3.5 ms |
  | viewer start → Files ready / first rows 1k | 21.4 / 16.3 ms | 21.9 / 16.3 ms |
- Small preview first paints (src_2k, src_5m, unicode) are bimodal at about 4 and 7 ms in both builds; an 8-run re-check per side put src_5m 4.7 → 4.0, src_2k 4.4 → 5.7, unicode 4.0 → 4.9, txt_200k end 5.5 → 5.4, which is the mix of those two modes rather than a shift.
- Deferred (decided, not bugs to chase now):
  - Old v0.2.0 `attach-ticket-<hex>` files (0 bytes) in the state dir are not swept.
  - The hub view-model carries jump routes but no mesh membership, LAN evidence, link rtt or device metrics; production draws links as direct ssh or a jump, the mesh render paths run only in the lab.
  - The device screen's ` · single-direction` marker and LabDevice.core still come from the legacy posture, so they can disagree with recorded edges; whether `cx access --graph` should validate against edges::Snapshot is a design call.
  - Inside a device `g` is slot 4 (spec), so `gg` is not a motion there; `G` is.
  - Files `Y` stays "clear the transfer clipboard" (the spec defines no alternate copy in Files).
  - The invite's `R` and `+`-after-Esc paths have no unit test (they open a real listener); Esc is tested.
  - machine_id() is read in three places (MESH-8 part): sharing it would add indirection for a 5-line read. Invite expiry stays Instant-based (MESH-13 drift part, rejected).
- Open: the hub still shows an Error toast for an unreachable device beside the ticker that already names it (capture startup-120x40; present in v0.2.0); capture_matrix flags the keys screen's "Cloudflare…" as a cut interface name (false positive).

## v0.2.0 release candidate (branch `revamp/base`; not tagged, pushed or deployed)
- Integrated: the final Mission design (revamp/design-final), instant sessions (revamp/instant: helper Watch, one tmux control client per server, one wake channel, warm masters), hub tiles from in-memory frames and watch events, `+` opening a real ssh-overlay invite (code, `cx mesh pair` line, QR; closed on use, expiry, Esc or `R`), a masked set/rotate entry on the keys screen, `cx join`.
- Fixed for the release: the invite listener's reset race (drain before close), relay probe ETXTBSY under parallel forks, markdown first paint (one layout, tagged layout on first click), the 100k sibling-folder double read (Enter waits on the prefetch), update fixtures past 0.2.0, Ubuntu 24.04 fixture cleanup race, session_watch_live and perf_harness instant_* on the hub.
- Gates (2026-11-12): fmt, clippy (no new warning kinds vs b4174c4), `cargo test --locked` twice green, release build, regression ledger with SSH 60/61 then session_watch_live green once ported to the hub, ubuntu:24.04 CI-checks reproduction, static musl x86_64/aarch64, A/B vs v0.1.95 (CHANGELOG v0.2.0), captures dark and ASCII at 4 sizes.
- A/B vs the v0.1.95 release binary (medians; perf_harness 3 interleaved runs, files_bench 3 x 2 runs; v0.1.95 measured with its own-era harness 01c08b4 for its keys):

  | metric | v0.1.95 | v0.2.0 |
  |---|---|---|
  | select → screen (peek), local / over SSH | n/a | 5.8 / 6.5 ms |
  | keystroke echo, peeked session (SSH) | 101 ms | 1.5 ms (2.1) |
  | Enter → attached (SSH) | 83 ms | 12 ms (34) |
  | detach → cx | 270 ms | 17 ms |
  | first rows 1k / 20k / 100k (cold) | 124 / 124 / 124 ms | 45 / 46 / 46 ms |
  | 100k sibling folder | 107 ms | 43 ms |
  | preview first paint src 5 MB / md 2 MB | 105 / 111 ms | 5.7 / 6.5 ms |
  | preview scroll j, src 5 MB / md 2 MB | 5.3 / 3.5 ms | 4.5 / 3.5 ms |
  | viewer start → Files ready | 47 ms | 147 ms (46 with the bench fix below) |
- Startup attribution (v0.2.1 review): the 47 → 147 ms row was the bench, not cx. The OSC 11 background probe (caps.rs, 100 ms bound) waits for a DA1 reply that the bench PTY never sends; with the probe skipped (`CX_THEME=dark`, now set in perf_harness Env) startup is 46 ms (median of 4 runs, release build, review A/B). Real terminals answer DA1 within milliseconds.
- Resolved in v0.2.1: the 100k sibling outlier (bench) and G on a 2 MB Markdown preview.

## Claude gateway lends (Unreleased)
- `cx claude lend | revoke | list` (mesh/claude.py, docs/claude.md): a better-ccflare key, an Access service token and policy, and a single-use code; the teammate runs `curl -sL <endpoint>/claude/<code>|sh` or `irm …|iex`. `cx mesh deploy` redeploys only the Worker.
- Tested offline (fake Cloudflare, fake gateway, claude.sh against a local stand-in) and live: a throwaway lend over the Access route and the mesh route, Claude Code answering with the written settings, then revoke (the key refused, token and policy gone). claude.ps1 is untested on Windows.

## Mesh one-line joins (Unreleased, `mesh-oneliner`)
- `cx mesh setup | invite | devices | audit | revoke | teardown` and the standalone `mesh/mesh.py` (docs/mesh.md). Devices join with `curl -sL <endpoint>/<code>|sh` or `irm …|iex`, with no cx.
- Tested offline: the profile, setup against a fake API (idempotent, dry run, teardown) and the Worker against an in-memory D1. Live: two throwaway Linux devices joined, registered, got mesh addresses and left cleanly on a real account. macOS and Windows joins are untested on hardware.

## Preview fidelity and paged history — 0.1.92

Installed locally from `f7c90f3`; version **0.1.92**, SHA-256 `e8424f77ae80e42ed9cc0867e480c5113344d3773ee013bebf1e8947c828a954` matches the optimized candidate. Atomic maintenance-locked replacement; rollback `/home/lunarz/.local/bin/cx.rollback-0.1.91-preview-fidelity`. Restart only the CX viewer. No remote rollout or personal-session restart.

Reproduced the remaining prompt highlight as missing trailing painted cells and lost SGR state between captured rows. Preview rendering now preserves both, including explicit color resets. Synthetic actual-viewer tests verify muted backgrounds behind letters and adjacent blanks at 80/120 columns. Native contrast behavior remains verified.

History requests now page through all retained tmux rows with absolute row coordinates rather than stopping at the latest 500. Each page stays bounded and in memory; reading pauses polling. Full-screen applications with mouse tracking receive identity-checked wheel events to navigate application-owned history. Keyboard/paste focus behavior is unchanged. Protocol v1 adds defaulted `history_start` and additive history/mouse metadata; older helpers retain earlier behavior.

Remaining acceptance: user's actual terminal appearance and provider-specific scrolling. Retention remains controlled by tmux; evictions can shift older page positions, and discarded history cannot be recovered. Applications without terminal history or enabled mouse scrolling require their own navigation controls. Remote helpers need updating; no fleet rollout.


## Terminal preview scrolling — 0.1.91

Installed locally from `39a49ba`; version **0.1.91**, SHA-256 `1e3179641bbbaaf54c2fa5c1fa903e163cdd8c44eecc06abe9e875630187a02c` matches the release candidate. Atomic maintenance-locked replacement; rollback `/home/lunarz/.local/bin/cx.rollback-0.1.90-preview-scroll`. Restart the CX viewer to use scrolling; terminal sessions keep running. No remote rollout.

Mouse wheel over either preview layout or the focused pane scrolls recent terminal output. Up to 500 history rows load on demand, then polling pauses to keep the reading position fixed. Scrolling to the live position, focusing or typing resumes live output. Selection changes discard history. History remains memory-only; original pane geometry and process input are unchanged. Remote helpers need this version for history; older helpers retain current-screen scrolling. Physical user acceptance remains pending.


## Codex terminal contrast — 0.1.90

Installed locally from `0b2b928`: `~/.local/bin/cx` reports **0.1.90**, SHA-256 `2398c8bc29ca489972668a75ef8249dec2b6de4de15bd6f8a10277df4a109fdc` matches the optimized candidate. Atomic replacement held the maintenance lock; rollback `/home/lunarz/.local/bin/cx.rollback-0.1.89-codex-contrast`. Final optimized native contrast and managed/external attach fixtures passed after the optional-styling compatibility adjustment. No remote rollout or personal-session restart. Reopen CX and detach/reopen native Codex to apply; physical visual acceptance remains pending.

Native Codex prompt backgrounds were observed using ANSI index8 while actual clients already advertised RGB. Private synthetic native-attach acceptance reproduces dependence on that palette slot and verifies outgoing muted RGB after correction. On native attach, managed Codex panes receive a pane-local neutral index8 fallback (#303030 for dark, #e4e4e4 for detected light); explicit pane palette entries take precedence. Returning to shell removes only CX's own fallback. No global terminal palette/provider config changes. Existing sessions take the change on next native attachment.

Embedded previews receive the pane's effective index8 RGB override. SGR handling now honors DIM and attribute-reset codes, including 27 ending reverse video; tests reproduced accent foreground remaining inverted before the fix. Remaining acceptance: confirm the exact user's appearance after reattachment; synthetic color output is verified, but no real provider conversations were emitted in logs.

## Hover previews, availability and loader alignment — 0.1.89

Installed locally from `f91242a`; executable reports0.1.89 and matches build digest. Atomic maintenance-locked install, rollback `~/.local/bin/cx.rollback-0.1.88-hover-fix`. No remote rollout or personal session restart.

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
- Edge policy root contract (accepted at integration): `Operation::PeerAuthorize`,
  `PeerConfigure.authorize` (serde default true, omitted when true, so old wire is
  unchanged), capability `peer-authorize-v1`, `edges.json`, `cx edge`, and the edge
  guard in `transport::request` and `edges::interactive_ssh` (attach, terminal, run).

Removal/demotion prevents future enrollment but does not revoke existing SSH keys.
Automatic setup requires supported helpers and direct port-22 connectivity; offline
peers remain pending and retry. Changed identity/key pins require verification.
Work was integrated through an isolated worktree; the original conflicted checkout
was left untouched. Test evidence is in [VALIDATION.md](VALIDATION.md); prior design
findings are in [the audit](research/session-reliability-audit.md).
