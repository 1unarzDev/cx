# Development

Work from an isolated branch/worktree as described in [AGENTS.md](../AGENTS.md).

## Build and test

```sh
cargo build --release --locked
cargo test --locked
tests/run_regressions.sh [--with-ssh] [--with-docker]
```

`tests/run_regressions.sh` builds cx and runs the offline fixture set (the one list CI and release run); see [tests/REGRESSIONS.md](../tests/REGRESSIONS.md). `tests/mesh_worker.mjs` needs Node >= 22.13 for `node:sqlite`, and is a SKIP without it.

GitHub workers test builds and installer fixtures across distributions. Rocky Linux supplies the RHEL-compatible CI fixture; RHEL itself requires access to a licensed environment. See [validation](../VALIDATION.md) for actual test results and remaining limits, and [status](../STATUS.md) for outstanding work.

## Releases

Record changes in [CHANGELOG.md](../CHANGELOG.md). Before tagging a stable version, move its reviewed changes from Unreleased into an exact `vX.Y.Z` section; publication requires these notes.

## UI maintenance

`src/ui.rs` owns the shared types (`App`, `Browser`, `Dialog`, `Action`, ...), `App::new`, device bookkeeping and refresh. Behavior lives in private modules under `src/ui/`, mostly as `impl App` blocks, so changes stay local:

| Module | Responsibility |
|---|---|
| `runtime.rs` | Event loop, `Screen` terminal ownership, input batching, task workers, auth prompts, self-restart state and update checks |
| `actions.rs` | `execute()`, the command palette and action availability |
| `keys.rs` | Workspace key and mouse dispatch: resolves keys through `keymap`, runs `Cmd`s, focus movement, command-panel keys |
| `keymap.rs` | The single keymap table (`BINDINGS`): key dispatch, help overlay, footer chips, context menu and palette key column all read it |
| `hitmap.rs` | Pointer targets registered during render (tabs, rows, chips, scrollbars, palette/menu rows); tap / long-press / drag resolution; context menu |
| `input.rs` | Text-input fields (filter, search, rename, commands); they own every printable key |
| `dialogs.rs` | Modal dialog keys, Enter handling per dialog family, confirmation button layout |
| `apply/` | Task replies: `mod.rs` shared bookkeeping and dispatch; `sessions.rs`, `files.rs`, `network.rs`, `containers.rs` per operation group |
| `launch.rs` | Session launch flows: device/provider choice, permission prompts, shells, adoption |
| `files_view.rs` | File browser state, transfers, `render_browser` (windowed rows, breadcrumb, inline peek, tap targets) and the prefetch/peek debounce (`files_tick`) |
| `file_index.rs` | `Entries` (Arc-shared listing with revisions) and `ViewIndex` (visible rows, lowercase names, search matches), extended incrementally per page |
| `preview.rs` | File preview state and rendering: text, markdown layout/links, raster fallback, in-preview find; `DocCache` (retained wrapped layout, slice rendering) and chunked streaming of large text previews |
| `network_view.rs` | Network/neighbor state, probes and the network render |
| `screens.rs` | The screens as a controller: the hub (start view), a device's sessions, peek, the edge policy editor, keys and the send-to picker; one grammar of a device letter, then a verb or a session slot |
| `hotkeys.rs` | Device letters for the hub and the session slots `a s d g` ([concept-mission](design/concept-mission.md) section 6) |
| `viewmodel.rs` | `ViewModel::from(&App)`: App state mapped into the view layer's `LabState`, plus the screen, focus and selection to draw; pure, never dispatches a request |
| `views/` | The production view layer (`views/mission/`: hub, orbit, tiles) drawn from the view-model into a ratatui buffer; no App, IO, tmux or network |
| `activity.rs` | Per-viewer session activity history (from listings, in memory), the trend a session's peek shows |
| `lab/` | Test-only render lab: captures `views` from fixtures for `tests/lab.sh` ([lab](design/lab.md)) |
| `containers_view.rs` | Container panel state, tree rows and container sessions |
| `caps.rs` | Terminal capability detection (spec section 9): color depth, UTF-8/ASCII, light/dark (`CX_THEME`, then a 100 ms OSC 11 query with a DA1 sentinel before the TUI starts, then `COLORFGBG`), tmux, Termux/phone, hyperlinks. Pure `Caps::detect(env, probe)`, computed once at `ui::run` start; probes (`infocmp`, `tmux display`) run in their own process group, time-bounded end to end, group-killed. A timed-out OSC 11 arms `swallow_stray_reply` so a late reply never becomes keystrokes. Tests override with `with_caps` |
| `theme.rs` | Semantic tokens (`Tok`, `tok()`, `accent`/`muted`/`faint`/`ok`/`warn`/`err`/`info`/`dir`/`provider`/`swatch`) downgraded per depth, glyph sets (`glyphs()`), selection styles, `block()` focus treatment (heavy accent + title chip vs rounded faint; ASCII `#=#` vs `+-+`) and the ASCII glyph pass. The pass converts UI glyphs in the whole frame except regions a content renderer registered with `protect_content` (file preview body, live terminal preview), so documents and remote screens keep their own quotes, dashes and box drawing; content renderers emit their own ASCII decorations. Under `NO_COLOR` the selection bar is `>`. Widgets never use raw `Color::` values |
| `navigation.rs` | Counted key motions and the `Target` interface; no requests or rendering |
| `panels.rs` | List, device and file adapters, including refresh/selection effects |
| `filters.rs` | Scoped filter state, device identity mapping and fuzzy matching |
| `menus.rs` | Session choices shared by labels/shortcuts/dispatch; container display labels |
| `notifications.rs` | Typed floating panels, bounded layout and timed animation |
| `rendering.rs` | Layout and rendering; no request dispatch |
| `session_preview.rs`, `workspaces.rs`, `container_agents.rs` | Session pane preview (inline input, helper watch with polling fallback, remembered frames, parsed-screen cache), workspace session identity and attach, container agent profiles |
| `tests.rs` | Existing UI and cross-panel regression fixtures |

Reuse a navigation target when adding a panel; keep panel-specific side effects in its adapter. Change session choices in `menus.rs` rather than maintaining parallel label/key/provider arrays. Rendering never changes the session provider: devcontainer identity is a display scope, while shell/agent behavior stays in the session runtime. Preserve typed-input ownership and the existing host/container scope guards. Verify UI changes with the capture matrices and native PTY fixtures in `tests/`.

Emit notifications through `App::set_notice` (Info) or `App::set_notice_as` with an explicit type: it renews a five-second monotonic deadline with 260ms eased entry/exit transitions, and the event loop clears expired text and redraws while idle. Floating panels anchor inside the bottom-right workspace, above the footer and transfer drawer, with type-colored borders/titles and the default background. Modals stay above notifications. Native bitmaps temporarily use the existing raster fallback so terminal graphics cannot cover the panel. Positioning and timer renewal follow noice.nvim's [mini view configuration](https://github.com/folke/noice.nvim/blob/7bfd942445fb63089b59f97ca487d605e715f155/lua/noice/config/views.lua) and [mini backend](https://github.com/folke/noice.nvim/blob/7bfd942445fb63089b59f97ca487d605e715f155/lua/noice/view/backend/mini.lua). CX keeps its single floating notice rather than adding a notification stack. Device connection failures use typed transport evidence and a per-device notification latch; current-generation successful helper replies clear it. Ordinary operation errors remain visible.

## Architecture and current limits

Network observation and SSH jump routes work. Internet sharing, full trust/revocation and full uninstall are not release-ready. No public hostnames or mesh provider are required. See [robot mesh sharing](robot-mesh-sharing.md) for access policy, sharing previews and their acceptance requirements, and [robot execution and updates](robot-tools.md) for platform-specific tooling.

For session naming research, see [Codex session identity](../research/codex-session-names.md).

Sessions are pushed, not polled. The event loop blocks on one channel fed by the input thread (parked for every terminal handoff), task replies and helper events. Each capable device (`watch-v1`) holds one `Watch` on its interactive helper channel: the reply carries the selected screen, then `evt:<n>` frames push the session list and that screen. In the helper, `src/sessions/control.rs` runs one `tmux -C` client per managed server (tmux >= 3.2, `ignore-size`, parked in the hidden `cxctl` session). `%output` only wakes a capture, which runs through the client at most 60 times a second, and keystrokes go through it as `send-keys -H`. Captures are never written to disk. tmux < 3.2, a refused watch or `CX_TMUX_CONTROL=0` uses the polling path. Helper channels are interactive, capabilities, metadata and bulk, all over one SSH master, and the task workers use the same lanes. `tests/perf_harness.py` (`instant_*`) measures the latency targets, and `tests/session_watch_live.py` covers control-client loss, attach cycles and old/new protocol mixes.

Session adoption (`src/adoption.rs`) discovers outside-CX terminals by reading `/proc`, pairing each session leader with the process holding its PTY master. `sessions::adopt` moves one by running the guarded `cx-reptyr -T` relay in a managed pane. The relay is built by `scripts/build-portable-reptyr.sh` from pinned reptyr 0.10.0 plus `scripts/reptyr/` (uid/start-time guard, SEIZE-based attach, capability drop). Adopted records keep `origin`, and inspection follows the original leader instead of pane descendants. This supersedes the earlier "never automate migration" note; see [the adoption research](../research/session-adoption.md). Live fixtures run under `unshare -Ur`, or with `CX_ADOPT_CAPABLE=1` and a `setcap` relay in CI. Debug builds accept a fixture relay through `CX_ADOPT_REPTYR`.

Container transfers use optional source/destination `ContainerScope` fields on `TransferSpec`, dispatched with `scoped_transfer` whenever either field is present. The transfer adapter routes file I/O through `container_file_action`; ordinary `transfer` rejects scopes. Container I/O reuses private framed helper channels, with engine/user/lifecycle validation before and after each request. Container state/checkpoints stay under the selected user's private tools directory; queued UI mutations capture their scope at enqueue time. Rebuild uses the Node CLI with unique selected workspace labels and requires `devcontainer-rebuild-v1` on remote hosts.

The session-adoption live fixtures (`tests/adopt_terminal_live.py`, `tests/adopt_sudo_live.py`, `tests/adopt_stress_live.py`) need real ptrace privilege and run locally, not in CI: they self-sandbox in an unprivileged user namespace (needs `newuidmap`/subuid), or set `CX_ADOPT_CAPABLE=1` with a `setcap cap_sys_ptrace` relay. This mirrors the `#[ignore]` live tmux test. Discovery, the guard's capability parsing and the SGR parser are covered by unit tests in CI.

Command panels run a `:` command in a transient managed pane (`sessions::command_panel`, kept with `remain-on-exit`) and poll `capture_pane`/`pane_input`/`pane_close` over a dedicated worker lane. `src/ui/rendering.rs` parses the styled capture (`ansi_line`) into the floating panel.

[Back to README](../README.md)
