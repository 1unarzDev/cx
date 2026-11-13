# CX status

## v0.3.0 integration (branch `revamp/base` on v0.2.2 5484a4d; not tagged, pushed or deployed)
- Merged: revamp/realdata (e350afc: real hub data on the nebula map), revamp/nebula (c68dd69: group letter colours, calm identity hues, narrow-list hues, welcome button; realdata's ev_w `.max(0)` kept), revamp/files-yazi (dd4e14f: Files as a yazi clone, image protocols, instant listings, `'` device switch, cross-device y/p).
- Conflict decisions: v0.2.2's device LINKS zone keeps `orbit::Sel::Node` (the nebula had dropped it); map arrows take the nebula's planned moves (±45°, ±80° only when empty) plus v0.2.2's memory of the last move, so the opposite arrow undoes it, also after a visit to the tiles (bf45024); the phone tree letter takes the group hue on a transparent row; the startup probe keeps realdata's terminal rtt with yazi's DA1 counting; Files `H` is the hub (an intended shadow), yazi's history is Alt+←/Alt+→ (`L` still also goes forward, as yazi), `!` is a new session; an arrow never closes the which-key panel.
- Fixes: a lost helper channel reports one stable reason (the device status alternated between `helper closed before response` and `Broken pipe` depending on which side of the pipe noticed first; 4d3a0a6); no Error toast for a device the ticker already names, and Files says `Device unreachable over SSH` in its empty column instead of `No items` (0023bbd); `allow-passthrough` is set only when cx starts a managed server, never sourced into a running one (the yazi branch had put it in the re-sourced config: compat_021 caught it changing a live v0.2.2 server's options; 51cc873).
- grid_keys_pty root cause: the tiles rank by attention (state, then latest activity), so four new shells going idle reshuffled the grid right after Tab focused the first tile (focus follows the session, not the slot). The failing branches predated v0.2.2's wait for the order to settle; with it the tile part passed 13/13 runs. Its Files part, chooser_arrows_pty and session_chooser_pty now drive yazi's Files (`/` find, `!`); map_grid_pty checks the nebula's cone.
- Gates (2026-11-13): `cargo fmt --check` clean; clippy --all-targets 117 warning lines, per-crate counts identical to 5484a4d, none on a line changed since; `cargo test --locked` 740+55+4+149+114+25 = 1087 passed, 0 failed, 5 ignored; `cargo build --release --locked`; both ignored all-size fuzz tests (release, overflow checks and debug assertions, scratch target deleted) passed in 263 s; tests/run_regressions.sh --no-rust --with-ssh 75/75 (hub_matrix, yazi_files_pty and @ssh, grid_keys_pty, hub_motion_pty, map_grid_pty included), and after the passthrough fix the session/attach/yazi subset 10/10; hub_motion_pty 6/6 standalone.
- Compatibility (tests/compat_021.py, loopback sshd): v0.2.1 <-> this build PASS both ways (server and pane pids, options and keys unchanged); v0.2.2 <-> this build PASS both ways with a scratch copy that skips the version-string check (Cargo.toml still says 0.2.2).
- A/B vs v0.2.2 (release 5484a4d in ~/.cache/cx-targets/v022, interleaved, ~/.cache/cx-030-ab/full): files_bench 3 local pairs (runs 2) and 2 remote pairs (rtt 20), perf_harness 3 matched pairs (3-5; pairs 1-2 of the new side failed on the old harness's Files row finder, fixed in 50be513). Medians old -> new: Files local deep Enter hop 17.6 -> 1.6 ms, sibling 100k first rows 52.1 -> 3.1, j after first rows 11 -> 1.8, viewer ready 23.4 -> 12.3, startup 1k rows 16.3 -> 6.0; remote startup 1k rows 256 -> 34, sibling 100k 77 -> 5.2; cold first rows 3.5 -> 4.1 (local 1k), parent (h) local 2.5 -> 4.0/7.2/2.9/3.0 (100/1k/20k/100k) and remote 3.5 -> 4.7; previews within ±1 ms. perf: echo 1.6/1.6, Enter -> attached 18.6/18.4, detach 19.3/18.8, markdown open 24.9 -> 15.3, idle bytes hub 0 / Files 0 on both; instant remote select -> screen 6.2 -> 9.6, instant remote Enter -> attached 41.9 -> 50.9, instant local Enter 18.7 -> 22.7, source j 4.4 -> 8.0 (bimodal 4/8 ms on both sides; old pairs 4-5 ran in the same slow phase).
- Captures (real app, release build): ~/.cache/cx-030-int/{dark,ascii}/{startup,device,files,help,welcome}-{40x20,95x50,120x40,190x50}.png/.txt; both capture_matrix runs PASS, 0 unmasked, no leftovers. Hub and welcome match the nebula reference renders (dotted rings, bowed dotted links, sparklines and vitals strip, tile states, the welcome orbits and button).
- Unresolved: (1) the parent (`h`) frame and instant remote Enter/select are slower than v0.2.2 in this A/B (yazi draws three columns; the 2 s vitals ask shares the interactive channel with attach) and need a focused recheck on a quiet host; (2) Cargo.toml is still 0.2.2 and CHANGELOG has no v0.3.0 section; (3) running managed servers get passthrough (kitty/iTerm2 images inside tmux) only after they next start; (4) the reconnect-text flip was fixed at its source and unit-tested, but its ~3 s cadence was not reproduced in a fixture.

## v0.2.2 release candidate (branch `revamp/base`; not tagged, pushed or deployed)
- Finalization round (2026-11-13, on 75823cc): the hub key is `H` (Shift+H) everywhere, Files and the preview included (keymap, footers, help, palette, README, user guide, concept doc; the backtick stays a hidden alias for v0.2.2 only; Files had no H/L binding to move, so nothing went to Alt+Left/Right); arrows never close a popup (the session chooser's Left/Right move along its row, the agent step's Left returns to the device chooser or device menu that opened it, only Esc closes; red on v0.2.1 in chooser_arrows_pty); the find palette and the link editor already draw only `╭╮╰╯` (the critic's square corners were link junctions, fixed in db90c01; now a test at 40x20..190x50); the full-size fuzz note names both ignored tests.
- Live-safety review of attach and update (required before deploying to machines with running sessions):
  - What touches a running managed server: `configure_managed` (`source-file` of the regenerated `tmux.conf`, then a marker naming the server pid, start time and cx version) from `ensure_server` (every create/reset), local `sessions::attach`, `native-attach-checked` (only when the marker is not this version's) and the viewer's attach ticket (`prepare_attach`); per attach `refresh_managed_status_as` (session options `status-right`, `@cx_title`, `@cx_ascii`, the window's `window-style`/`window-active-style` reset to the global value, Codex's `pane-colours[8]` only on a Codex pane) and `exit_copy_mode` (cancels copy mode on the attached pane, focus attaches only); `refresh_provider_environment` (`set-environment -g`, new panes only); the control-mode client (`attach -f ignore-size` into the hidden `cxctl` session; no resize). `kill-server` only in `cx reset`, `kill-session` only for an explicit stop.
  - v0.2.2 changes none of these: `assets/tmux.conf`, `managed_config`, `src/sessions/control.rs`, `src/model.rs`, `src/transport.rs`, `src/update.rs` and `.github` are byte-identical to v0.2.1; `src/main.rs` only adds `self_exe` (below); `src/sessions.rs` only adds the probe's `setsid` and the `self_exe` uses (below), the attach-guard unlink after its verdict, the startup sweep of `attach-guard-*`/`attach-ticket-<hex>` older than 1 h that no process holds open, and `env` in the command-label skip list. So when v0.2.2 first attaches to a 0.2.1 server (version marker differs) it re-sources the same config: every `set -g`/`bind-key` repeats its current value, and `history-limit`, `default-terminal` and `default-shell` only affect new panes. No new setting reaches a live server, so nothing needed gating to server start.
  - Verified (tests/compat_021.py): after the other version's create and attach re-sourced its config into the running server, `show-options -g`, `-s`, `-gw` and `list-keys` are identical to the server as the first version left it, and the server pid and every pane pid are unchanged.
  - The update path (`cx update`, the fleet swap) only replaces the binary; it never contacts tmux.
  - Fixed (release blocker, 2026-11-13): a viewer left open across the binary swap could not attach this device's own sessions, and a device's helper channel started before its swap refused typed input to a session (`terminal changed; input refused`). Root cause: `std::env::current_exe()` reads /proc/self/exe, which is `…/cx (deleted)` once the file is renamed over; cx passed that path to tmux's `if-shell` attach predicate (`native-attach-check`), which could not run it. Fix: `self_exe()` in src/main.rs: when the path ends in ` (deleted)` and does not exist, use the stripped path if a regular executable owned by this user or root is there (the new binary at the same install path), else the original. Used wherever a cx path reaches another process or is re-executed: the attach and pane-input predicates, the container launcher, `native-command`, SSH/sudo askpass, transfer jobs, container tooling install, enrollment upload and the `cx update` relaunch. The adoption exclusion now lists both `<path>` and `<path> (deleted)`, so viewers on either side of a swap are never offered as adoptable terminals. Identity checks are unchanged: attach verification is the tmux server's pid, start ticks, session id and creation time plus the 0600 guard file, none of which names the binary; askpass is uid plus nonce. `src/update.rs` keeps `current_exe()`: its inode check (installed target == running binary) makes a stale process refuse to install over an already updated binary, which is the intent. Guards: rust `a_replaced_binary_resolves_to_the_new_file_at_its_path`; tests/swap_attach_pty.py (local viewer from `$HOME/.local/bin/cx`: create, swap by `cx.new` + rename, Tab, Enter attaches, Ctrl+\ detaches to the hub) and `--ssh` (a `cx helper` over a loopback sshd, swap, SessionPaneInput from the same helper runs the typed command). Both red on f011523 (local: no client within 8 s; ssh: `terminal changed; input refused`) and green after; both green on the x86_64 musl build.
  - Fixed: started from a terminal, the local agent check never offered Claude/Codex to bash/sh users. Root cause: `bounded_provider_check` ran `SHELL -l -i -c 'claude --version'` in a new process group of cx's controlling terminal; interactive bash/sh initialises job control on that terminal from a background group and is stopped (SIGTTOU) before `set +m` runs, so each probe hit its 3 s timeout. Fix: the probe runs under `setsid` (its own session, so still its own group for the kill, and no controlling terminal). Guard: tests/provider_probe_tty.py (`cx helper` on a private PTY, stand-in `claude`/`codex`, bash, sh, fish): red on f011523 (bash: none offered, 6.0 s), green after (0.02 s each). tests/session_chooser_pty.py relied on the old timeout (Info arrived after it looked, so neither the agents nor Devcontainer were offered); it now shadows `claude`/`codex` with failing stand-ins and no longer asserts Devcontainer's absence (local `containers-v1` is unconditional). tests/enrollment_ssh.py's harness and the session_core/transfers test crates define a `self_exe` (as for their other root items).
- Release gates after the swap and probe fixes (2026-11-13, on f011523 plus this commit): `cargo fmt --check` clean; `cargo clippy --all-targets --locked` 117 warning lines, per-crate and per-file counts identical to the 46bdbd0 run (no new warnings); `cargo test --locked` 686+55+4+148+112+25 = 1030 passed, 0 failed, 5 ignored; `cargo build --release --locked`; tests/run_regressions.sh --no-rust --with-ssh 71/71 (swap_attach_pty, swap_attach_pty@ssh and provider_probe_tty new); tests/compat_021.py (v0.2.1 ↔ this release build) PASS both ways, server and pane pids unchanged, options and keys unchanged. Static musl rebuilt (~/.cache/cx-targets/musl022, aarch64 linked with rust-lld): x86_64 16041464 bytes sha256 ed4b52769f03c3e77232e5c9df1944195a20a164c5f587c112fbba95e52a2efb (static-pie; swap_attach_pty, its `--ssh` variant and provider_probe_tty pass on it); aarch64 15541208 bytes sha256 845e17d64750b2791d107883ca289cc4e9cfa1e7e3d4907ba74c6adf5f860015 (static). These supersede the musl hashes in the next bullet. Not tagged, pushed or deployed.
- Release gates (2026-11-13, at 46bdbd0): `cargo fmt --check` clean; `cargo clippy --all-targets --locked` no new warnings vs 295020d (warning kinds and files identical: bin 21, bin test 29, session_core 49, files_network 24, transfers 12, update_backend 11, regressions 1; one `bool::then` warning this round was fixed); `cargo test --locked` 685+55+4+148+112+25 = 1029 passed, 0 failed, 5 ignored; `cargo build --release --locked`; tests/run_regressions.sh --no-rust --with-ssh 68/68 (chooser_arrows_pty new). ubuntu:24.04 reproduction of release.yml/ci.yml (cx-ci24:local, steps in ~/.cache/cx-release-022/ci24, logs ci24*.log): tag/version, release notes, fmt + test (same 1029) + build, apt, shellcheck (now including tests/run_regressions.sh, as the workflows do) all pass; the fixture step ran 74 fixtures: the first run failed only the 5 docker fixtures because the inner dockerd's /var/lib/docker sat on the container's overlay root (`failed to mount ... overlay`; an environment change since v0.2.1), so it was rerun with /var/lib/docker on a volume: 73/74, session_chooser_pty failing only its temp-dir cleanup (`Directory not empty`, it passed in the first run), which passed alone in a third run with the variants step (zsh native command, fish folder command, devcontainer CLI): ALL STEPS PASSED. Static musl (~/.cache/cx-targets/musl022, aarch64 linked with rust-lld): x86_64 16045248 bytes sha256 c5e4f97a5469d9ce232b33245c066f5d6b72a45002344f8f7c6a0afcab6abb5b (static-pie); aarch64 15538448 bytes sha256 565621a6967958fd646c987d820e7bc04d42267a6c1b21c9eabb930eb8acd19b (static).
- Compatibility (tests/compat_021.py, loopback sshd, private HOMEs; v0.2.1 built from tag v0.2.1 into ~/.cache/cx-targets/v021): both directions PASS on the release build. Each scenario on a fresh device server: list (CLI), preview (the viewer's own UI drew each pane's marker), `cx attach --device` and Ctrl+\ for each session (exit 0), ids and pids before → after:
  - A, 0.2.1 viewer + 0.2.2 helper (server and `a1` by 0.2.1, then the helper swapped by rename and `a2` by 0.2.2): server 2766476 → 2766476; `cx-f55ff16f6…` (a1) pane 2766490 → 2766490; `cx-2c3a4249d…` (a2) pane 2766557 → 2766557; config marker 0.2.1 → 0.2.2, options and keys unchanged.
  - B, 0.2.2 viewer + 0.2.1 helper (server and `b1` by 0.2.2, then rolled back and `b2` by 0.2.1): server 2767226 → 2767226; `cx-7dc96f776…` (b1) pane 2767247 → 2767247; `cx-4814d9209…` (b2) pane 2767313 → 2767313; marker 0.2.2 → 0.2.1, options and keys unchanged.
- Fixes from daily use of v0.2.1 (CHANGELOG v0.2.2): half terminals (`hub.rs:43` clamp min > max; every view now renders at every size 20x5..250x80), blank hub thumbnails (only the selected session was captured), `✕ 20770d` for never-seen devices, leaked attach guards (removed after the check; startup sweep of stale unowned ones), transparent-friendly chrome (no cell backgrounds), curved links, bigger previews.
- User feedback round 2 (all done): `␣ ⏎ ⌫` key labels (`spc ret bksp` in ASCII); yazi-style rounded key pills only where the terminal draws U+E0B4/E0B6 itself (kitty, ghostty, wezterm; foot needs `CX_NERD_FONT=1`), else bold keys with nothing behind; no `NETWORK lan` strip; the palette's arrows/Ctrl+N/P move the selection it draws (root cause: it drew the hub's `caps.selected`) with a text cursor in every find field; the hub and device grids and the map walk in 2D with arrows and `h j k l` (Tab still switches zones; a tap focuses a tile, a second tap attaches).
- Gates (2026-11-13, at the round-4 head 7600e92 plus docs; host load average 14-18): `cargo fmt --check`; clippy --all-targets 117 warning lines, the same summary counts and kinds as 295020d, none on a line added since 295020d (warning locations intersected with the diff); `cargo test --locked --no-fail-fast` (default threads) 683+55+4+148+112+25 = 1027 passed, 0 failed, 5 ignored; the ignored full matrices `every_view_renders_at_every_size` and `every_overlay_renders_at_every_size` (release, overflow checks and debug assertions, scratch target in ~/.cache, deleted) passed in 443 s; release build; tests/run_regressions.sh --no-rust --with-ssh 67/67 (hub_thumbnails_pty, grid_keys_pty, map_grid_pty included); the half-terminal repro against a copy of the real device state at 80x48, 95x50, 60x40, 190x25, 40x20, 20x5, 19x30, 100x24, 250x80, 120x40, 190x50: alive at each; capture_matrix dark, light, 16-colour (TERM=xterm), ASCII at 40x20, 80x48, 95x50, 120x40, 190x50 (4 x 65) and pills (`CX_NERD_FONT=1`, 95x50 and 120x40) in ~/.cache/cx-cap-023/{dark,light,c16,ascii,pills}; lab frames of the 7- and 25-device fixtures (hub, device, edge; dark, 16, ASCII) in ~/.cache/cx-cap-023/lab/curves023; all 5 capture runs PASS, 0 unmasked, no leftovers; orphans: `cx orphans` 12 cx processes, 0 stray, 0 stale handoffs (the viewers are the user's, started before this round); two cx-perf tmux units (started 23:35 and 00:03, before this round) and /tmp/cxreal-tmux-29tqugyx (02:56) belong to earlier runs and were left alone; this round's scratch state, fixtures (/var/tmp/cxab-probe) and fuzz target were removed.
- transfer_enter_pty@container (docker; outside the --with-ssh gate) fails here on v0.2.1 and on v0.2.2 alike.
- A/B vs v0.2.1 (release builds, interleaved, ~/.cache/cx-ab-023): perf_harness 3 runs a side and files_bench 3 x 2 runs (full2), then focused interleaved reruns of every flagged files metric: dirs only 3 x 2 (ab2), previews src_50m/src_5m/md_2m 3 x 3 (pv), and a 16-pair cold-Enter probe of a 20k folder. Medians (v0.2.1 → v0.2.2):

  | metric | v0.2.1 | v0.2.2 |
  |---|---|---|
  | select → screen (peek), local / SSH | 8.2 / 10.1 ms | 5.1 / 5.3 ms |
  | keystroke echo, local / SSH | 1.6 / 1.9 ms | 1.5 / 1.8 ms |
  | Enter → attached, local / SSH | 13.4 / 37.2 ms | 12.9 / 32.7 ms |
  | detach → cx, local / SSH | 22.6 / 22.1 ms | 22.2 / 22.6 ms |
  | 20k listing first rows (perf_harness) | 3.9 ms | 1.1 ms |
  | first rows cold 100 / 1k / 20k / 100k (dirs rerun) | 4.0 / 4.2 / 3.8 / 4.1 ms | 5.1 / 3.7 / 5.8 / 3.8 ms |
  | cold Enter, 20k folder (16-pair probe) | 3.7 ms | 3.8 ms |
  | parent (h) 100 / 1k / 20k / 100k (dirs rerun) | 2.4 / 2.4 / 2.4 / 2.4 ms | 2.3 / 2.3 / 2.4 / 2.4 ms |
  | warm first rows 100 / 1k / 20k / 100k | 3.6 / 3.6 / 3.6 / 4.1 ms | 3.7 / 3.7 / 3.7 / 3.6 ms |
  | preview first paint src 50 MB / 5 MB / md 2 MB (rerun) | 4.3 / 4.4 / 4.2 ms | 4.5 / 4.3 / 4.5 ms |
  | preview scroll j, md 2 MB / src 5 MB | 1.8 / 4.1 ms | 1.8 / 4.1 ms |
  | markdown / source open (perf_harness) | 24.5 / 7.8 ms | 27.8 / 8.3 ms (previous full run, pre-lazy head: 24.2 → 23.2 / 4.5 → 4.3) |
  | attach-path echo / Enter (perf_harness sessions_attach) | 1.4 / 13.1 ms | 1.5 / 13.3 ms (previous full run: 1.4 → 1.6 / 13.0 → 11.8) |
  | viewer start → ready | 22.5 ms | 22.7 ms |

  The folder column was the cold-rows cost: shown at once from the listing in hand, it made the parent frame on `h` twice as large (2.4 → 4.5 ms in every round). It is now lazy: it shows once the highlight rests (the prefetch's 150 ms), so a folder's or parent's first rows paint alone (parent back to 2.4 ms). Cold first rows are bimodal on both sides (about 3.7 ms in one PTY read, 6-10 ms when the frame lands in two); the dirs rerun's 100 and 20k medians caught more two-read samples on v0.2.2, the 1k and 100k medians are lower, and the 16-pair probe (3.7 vs 3.8 ms, first output byte 3.67 vs 3.77 ms) puts the difference at about 0.1 ms. The Enter frame clears the column it leaves (about 1 KB more output); that is the column itself, which the user asked for. Not a clean sweep: other small rises (SSH detach +0.5 ms, viewer ready +0.2 ms, src 50 MB first paint +0.2 ms, markdown and source open, attach-path echo and Enter) change direction between the two full runs on this shared host, and nothing this round touched those paths (only Files, the hub's arrows, tiles and marks), and the full run's flagged src_50m (6.1 → 9.4 ms) did not reproduce in the focused 9-sample rerun (4.3 → 4.5 ms).
- Answered for the user: secrets reach agents only when asked for. cx injects nothing into claude/codex/shell sessions; `cx secrets get NAME --exec -- cmd` puts values into that one child's environment (`--fd N` writes one to a descriptor), values live in the OS keyring or the 0600 file, and cx itself reads only `cloudflare.api_token` (for mesh.py).
- Review round 4 (verifier and visual critic on ad017c5), all fixed: tiles blanked on a fleet past 32 previewable sessions (each 2 s round asked for every session on every device, and the frame map, capped at 32, was cleared whenever a new key arrived): only the tiles the last frame drew are captured, and at the cap one frame whose tile is not drawn is forgotten (`drawn_tiles_keep_their_screens_past_the_frame_cap`, red with the old clear); map arrows were not reversible and could move 90° off (Down then Up from innovation reached blastoise-jetson; Left from blastoise-jetson went straight down): the pick is a ±45° cone (±63° if empty), the opposite arrow retraces the moves made (a stack, so Down Down Up Up returns), and an arrow with nothing that way stays put instead of jumping along the order (lab property test over 1-25 devices: every node reachable, no move outside ±63°; `the_opposite_arrow_undoes_each_map_move` over 4-8 nodes, red without the stack; map_grid_pty now walks each arrow once and twice and back; checked on a copy of the real state at 120x40); Up from an empty tile zone returns to the map; the folder column is lazy (above); tile screens start at the top like the peek and the terminal; `t a s d jump`; the device header lines up under its key letter; one `▌` bar for zone labels and tiles; the link editor marks the chosen policy by colour and bold only; link junctions draw their trunk's line, never a square tee (test: no ┌┐└┘├┤┬┴ on the hub, a device or the edge editor for the 7- and 25-device fixtures); pills shown in the `pills` captures.
- Review round 3 (verifier and visual critic on 9ebb0f1), all fixed: the command panel's inverted clamp under 20 columns (`rendering.rs:807`; choosers and the context menu also drew past tiny frames), found by a new sweep of every App overlay from 1x1 (`every_overlay_renders_across_sizes`, ignored full matrix `every_overlay_renders_at_every_size`); the dead background code (`has_surface`, `wash`, `surface`, `sel_bg`, `tile_bg`, AccentBg/SelectDimBg/Surface2) deleted; the help legend draws the map's strokes (braille, `▶`, `⊘`); a device's LINKS are a zone (Tab, Down from the bottom tiles; Enter edits the link; Up from the top tile row stays); the hub always has one large tile (the first while the map has focus; Tab focuses it); new PTY test map_grid_pty (map by place, device grid in 2D, LINKS, tap focus; red on v0.2.1); shell_labels_live's flake was real: a pane starts as `exec env … shell`, so a listing in that window labelled it `recent: env` (now excluded; the test also waits for the prompt); grid_keys_pty waits for the attention order to settle; Esc, a key, Esc is no double Esc; no toast for background failures the ticker already reports; folder previews in Files; rounded code frames and bright-blue syntax; `h` for a new secret in the keys body; the crumb names the hub's focused zone; footers keep `? help`; `←↑↓→ move` hints; tighter chips; help rows that fit at 95 columns; pty_lib decodes UTF-8 incrementally.
- Kept (with reason): `<letter> then  a attach` keeps the word `then` (the user locked the nebula mockup's hint wording, .decisions.md 2026-11-12). A device's LINKS star draws two links as straight lines when both sit on the root's row: nothing turns, so there is no corner to round.
- Flaky under host load (load average 30+ on 20 cores), green when rerun alone: files::tests::twenty_thousand_entry_listing_is_fast_sorted_and_matches_old_pages (1 s budget), syntax_preview fence colours (500 ms highlight budget), mesh a_cancelled_invite_closes_its_listener_promptly.

## Files as a yazi clone (branch `revamp/files-yazi`; not integrated)
- Round 9 (critique fixes). Keys now match yazi. Esc with nothing left to clear does nothing, and Ctrl+S only cancels a running search; q leaves at once (with or without a selection). `T` is cx's transfers in Files as everywhere else (yazi leaves it unbound), and a new session here is `!`. yazi defaults that cx does not do give way to cx's global on the same key, so `-` re-attaches the last session. `;` closes its panel when the command succeeds; `:` keeps it open, like yazi's blocking shell. A test lists every cx global the yazi layer shadows and why: Tab, Enter, Esc, :, ?, F1, Ctrl+C, /, K.
- Linemode: under 48 cells the size bar goes and the size and day stay; under 24 cells it falls back to yazi's `none`. Example at 60x30: `config.toml    22B 11/12`. The built-in which-key separator is now yazi's U+EA9C (the user's theme already set it).
- Instant: an unlisted folder shows the header, rails and status at once with an empty column. "Loading..." appears only after 1 s, so a remote connect never flashes it. The bench now flags Loading frames: there were none for cx and several for yazi at 100k. A big folder persists its first 256 entries plus its size (before, nothing over 2048 entries was kept), so a remote 100k folder reopens in 34-40 ms instead of 314. The hovered child folder is also prefetched while its folder is still loading.
- Spawn -> first frame (files_bench `firstframe`, 120x40, rtt 20, load ~10 on 20 CPUs). Chrome means the header and status line:

  | | yazi chrome / rows | cx chrome / rows |
  |---|---|---|
  | local 1k cold | 54 / 57 ms | 14 / 23 ms |
  | local 100k cold | 30 / 455 ms | 19 / 126 ms |
  | remote 1k cold (`ssh -tt host yazi`) | 245 / 249 ms | 14 / 257 ms |
  | remote 1k warm | 233 / 236 ms | 28 / 33 ms |
  | remote 100k cold / warm | 239 / 674, 235 / 585 ms | 14 / 327, 34 / 40 ms |

  Rows over a cold remote connect take the SSH handshake plus one List round trip; the fixture's Python ssh wrapper adds 12.6 ms that only cx pays. Enter right after startup reaches first rows in 31.7 ms (target RTT+10 = 30). Where that time goes: the hovered child's prefetch leaves with the first frame and takes RTT + relay + helper ~1 ms, and the bench adds ~3 ms frame detection. Two other approaches made it worse and were reverted: listing the restore-selection child before the folder arrives went from 32 to 36 ms on the Files lane (the lane is one request at a time) and to 66 ms as a warm read on the hover channel (it is still connecting at startup).
- Images: kitty sends PNG (`f=100`, RGB when opaque) in place of raw RGBA. A re-hover reuses the image id the terminal still holds (16 kept, the oldest freed). The startup probe asks for `CSI 16 t`, which supplies the cell pixels when the pty reports none. Inside a tmux with passthrough, the probe sends the kitty query, XTVERSION and DA1 wrapped, so the outer terminal answers. This was verified against an isolated tmux 3.8, which forwards those answers. The environment (stale in tmux) is only a fallback. When tmux's own DA1 lists 4, tmux draws sixel itself, unwrapped. The helper fits images with a filtered resize (thumbnail to 2x, then Triangle), and uses PNG rather than JPEG for small boxes. That removed the moiré and chroma fringes in the half-block capture (r9/04). The half-block fallback averages pixels instead of sampling them. PTY checks cover kitty, kitty via probe, kitty via 16t, sixel via probe, iTerm2, and kitty, kitty-probe and sixel inside tmux. Each one sent a 440x330 bitmap, which is exactly the 44x38-cell box at 10x20 px from a 640x480 photo. Leaving the image erases it.
- A/B against revamp/base (`--remote --rtt 20`, 2 runs each, ~/.cache/cx-files-yazi/bench/r9-compare.md). Sequential runs flagged local first-rows and fs floors, but yazi's own numbers and the pure-Python fs floor moved by the same amount (machine load ~10). Interleaved local runs show no regression: 1k cold first rows base 3.0/4.8 vs new 6.0/4.8 ms (bimodal), parent 4.7/3.9 vs 4.7/4.1 ms, 100k sibling 534/482 vs 554/455 ms. Hover image, interleaved with r8: 19.5/23.0 vs 17.3/17.3 ms. Remote startup Enter -> rows: 255 -> 32 ms.
- Conscious deviations: the size bar and day stay the default linemode (the user's request; `m n` gives yazi's look). `q` closes Files and `Q` quits cx; yazi's `q` quits the process. This is kept because Files is one view of cx, and it is in help.
- yazi bindings cx does not do (dimmed in help and which-key; each shows a notice): Ctrl+Z suspend, `_` and Ctrl+- (relative symlink and hardlink), `A` bulk create, `m b` and `m o` (btime and owner linemodes), `,b` and `,B` (btime sort). `-` (symlink) gives way to cx's re-attach. Changed: `H`/`L` history moved to Alt+Left/Alt+Right (`H` is the hub), and `w` opens cx's transfers. Every other row of yazi's 116 `[mgr]` bindings runs (test `yazi_default_keymap_parity`).
- Captures: ~/.cache/cx-files-yazi/r9 (120x40, 80x24, 60x30).
- Unresolved:
  - No real-terminal screenshots of images. Only foot is installed, it needs a Wayland compositor, and nothing headless is available. The streams are verified byte-for-byte in PTY tests instead. To check: in kitty, ghostty, foot or wezterm, open Files, hover an image, then press j/k.
  - `grid_keys_pty` fails on the hub tiles (pre-existing, not Files code).
  - /tmp is at its user quota (other sessions' files), so the 20k-entry listing test needs TMPDIR elsewhere.

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
  - ~~The hub view-model carries jump routes but no mesh membership, LAN evidence, link rtt or device metrics~~ Closed in v0.3.0 (revamp/realdata): metrics-v1 helper op, 2 s vitals with cpu and rtt, link loss and jitter, mesh membership and LAN evidence observed off the UI thread; py:hub_matrix compares the real viewer and helpers with the lab.
  - The device screen's ` · single-direction` marker and LabDevice.core still come from the legacy posture, so they can disagree with recorded edges; whether `cx access --graph` should validate against edges::Snapshot is a design call.
  - Inside a device `g` is slot 4 (spec), so `gg` is not a motion there; `G` is.
  - Files `Y` stays "clear the transfer clipboard" (the spec defines no alternate copy in Files).
  - The invite's `R` and `+`-after-Esc paths have no unit test (they open a real listener); Esc is tested.
  - machine_id() is read in three places (MESH-8 part): sharing it would add indirection for a 5-line read. Invite expiry stays Instant-based (MESH-13 drift part, rejected).
- Open: ~~the hub still shows an Error toast for an unreachable device beside the ticker that already names it~~ (closed in v0.3.0: no toast once the ticker names the device; Files says why in its empty column); capture_matrix flags the keys screen's "Cloudflare…" as a cut interface name (false positive).

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
