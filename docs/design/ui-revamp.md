# cx UI/UX revamp: design spec

Status: draft for review (pass 0). Scope: the viewer TUI only (`src/ui.rs`, `src/ui/*`),
the managed tmux config (`assets/tmux.conf`) where it affects detach, and the
small amount of new local state the Network and Secrets views need. Nothing here
changes the wire protocol. Additive data needs are listed in section 14.

Evidence: `tests/capture_matrix.py` captures the real UI. Pass 0 (the current
v0.1.95 UI) is in `local-evidence/ui-passes/pass0/`: 12 states at 40x20, 80x24,
120x40 and 200x60, plus `ascii-nocolor/` at 80x24. The fixture is two local
managed shells, one "online" fake remote (`lab-arm`, served by a fake `ssh` that
runs `cx helper` under a second temp HOME), one refused remote (`field-rover`) and
a small file tree. IPs, MACs, `*.local` names, the real user and hostname, and the
temp directory name are masked in every capture. A run of spaces after
`/tmp/tmp-xxxxxx` comes from the masking, not from cx.

---

## 1. What pass 0 shows (the problems this spec addresses)

Each item cites a capture or a code location. Numbers are referenced later as P1..P18.

| # | Observation | Evidence |
|---|---|---|
| P1 | Focus is hard to see. The focused panel gets a `▸` in its title and a bold border. Every panel title is cyan, and bold borders look the same as normal ones in most fonts. | `sessions-all-120x40.png`, `ui.rs` `block()` |
| P2 | `CX_ASCII` does not produce ASCII. `BorderType::Plain` is still `┌─┐`. | `ascii-nocolor/sessions-all-80x24.txt` |
| P3 | Sessions are hard to tell apart. Group headers are cut from the right, which keeps the shared prefix and drops the distinguishing tail: at 40 cols every header reads `/tmp/cx-ca`. An unnamed shell is labelled "New shell". Each session takes 3 rows (header, row, blank). The AGENT column disappears below 100 cols. | `sessions-all-40x20.txt`, `sessions-all-80x24.txt` |
| P4 | The preview spends its first row on a hint (`Enter · focus terminal · Wheel · scroll`). It shows no age and no live or stale distinction. | `sessions-next-200x60.png` |
| P5 | Opening a session is a chain: Enter focuses the inline pane, then Ctrl+O opens the native attach, then Ctrl+] returns. Ctrl+O also belongs to Claude Code (transcript), and Ctrl+] conflicts with Codex. | `session_preview.rs` `session_pane_key`, `assets/tmux.conf` |
| P6 | Device rows are cryptic and inconsistent. `*` means "directed access". Status dots have no legend. Mesh, plain-SSH, jump and LAN devices look the same. Names are cut at a fixed 21-col sidebar even at 200 cols (`field-rov…`). | all sizes |
| P7 | The Actions panel repeats view navigation (Files/Network/Containers) and its items change per view, so click targets move. | `sessions-*.txt` vs `files-*.txt` |
| P8 | The "Selected" panel draws a staircase tree of the path, which uses 6 rows and a stray highlight bar. | `files-preview-120x40.png` |
| P9 | The file browser looks rough. Directories are ANSI blue on a dark background (low contrast). There is a `/` type column, vim jargon (`NORMAL 0 selected · hidden off`), names cut from the left at narrow widths (`…go.toml`), and an earlier run showed the size column clipped (`203.9 Ki`). | `files-*-40x20.png`, `files-inside-80x24.png` |
| P10 | The error toast covers the bottom of the list at every size and most of the screen at 40x20. It repeats long text. | `startup-*.png` |
| P11 | Network is a list, not a graph. At 200 cols the link column sits 1200 px from its row and the middle is empty. `?` and `!` glyphs are unexplained. Details are pinned to the bottom, far from the selection. | `network-expanded-200x60.png` |
| P12 | Overlays (palette, device chooser) stack 2-3 separate boxes and let the background borders show through at their edges. | `palette-80x24.txt`, `files-chooser-*.txt` |
| P13 | Files and Network add a device-chooser step after the palette, although the sidebar already holds a device selection. | `files-chooser-*`, `network-*` |
| P14 | Three footer rows of key hints at every size duplicate the help. The status line `● 2/3 devices` is yellow, which reads as a warning. | all |
| P15 | Help lists `d` twice. | `help-120x40.txt` |
| P16 | Up/down jank, cause 1: hovering a session row selects it (`MouseEventKind::Moved` in the session hit test). With the mouse resting over the list, a keypress and a hover fight over the selection. | `ui.rs` ~5994 |
| P17 | Up/down jank, cause 2: moving in Devices calls `refresh()` on every step. It clears provider caches, re-requests every device and resets the session selection. Enter on a device also only refreshes. | `panels.rs` `Devices::move_by`, `ui.rs` `refresh` |
| P18 | Startup lands in Files at the launch directory. The tab strip then reads "Sessions / Files  Network", a label for a state that is not a tab. | `startup-*`, `rendering.rs` label match |

---

## 2. Design principles

1. **One screen, one focus, one obvious next step.** Exactly one panel is focused, and it is unmistakable (section 5). The footer shows only the 2-4 actions that make sense next, as tappable chips.
2. **Quiet chrome, loud data.** Borders, titles and hints use low-contrast tokens. Color is reserved for state, identity and focus. Metrics (latency, activity, sizes, throughput, ages) are drawn as small inline sparklines and bars, never as walls of text.
3. **Evidence, not assumption.** Every status carries its age or source (`12ms · 3s`, `seen 5m`, `? unknown`). Unknown values render as `·`, never `0` or green.
4. **Same anatomy everywhere.** A device looks the same in the sidebar, the graph, the chooser and a session row: same colored identity chip, same status glyph, same transport badge. A session looks the same in the list, the palette switcher and the preview header.
5. **Everything is a target.** Each visible noun (row, chip, tab, breadcrumb segment, graph node, graph edge) is clickable and tappable, and it maps to the same action as its key. Hover never changes state.
6. **Actions chain.** Each action leaves the selection where the next likely action starts: detach returns to the row you left, create lands attached, transfer completes into the destination folder, and enrol lands on the new device in the graph.
7. **Zero configuration.** The UI adapts to the terminal (section 9). The only settings are networking ones (mesh, routes, edge policy, secrets). There is no theme file. `NO_COLOR`, `CX_ASCII` and `CX_THEME=light|dark` remain as environment escape hatches, not config.
8. **Graceful down to 40x20 and ASCII.** Every view has a phone layout and an ASCII/NO_COLOR rendering that keeps the same information (glyph and attribute substitutes, section 4).

---

## 3. Information architecture

Four top-level views. Each has a stable letter, used both in the tab bar and in `g`-goto:

| View | Goto | Purpose |
|---|---|---|
| **Sessions** | `g s` | All shells and agents across devices. Inline live peek, attach. Containers fold in as a device sub-scope (`laptop › devcontainer cx`), not a separate view. |
| **Files** | `g f` | Single or dual pane browser with inline preview and transfers. |
| **Network** | `g n` | Topology graph hub: devices, mesh, jumps, LAN neighbours, access edges and policy. Enrolment starts here. |
| **Secrets** | `g k` (keys) | Masked credentials cx knows about. Cloudflare token onboarding. |

Persistent across views:

- **Device rail** (the sidebar at ≥100 cols, a chip strip at 60-99, the Devices tab at <60). Selecting a device scopes the current view. It never triggers a network refresh by itself (fixes P17).
- **Command palette** (`Ctrl+P` or `:`, or tap `⋯`) for actions plus a jump-to-anything switcher.
- **Status line** with global metrics and a transfer progress bar.

Removed: the Actions panel (P7; its contents move to contextual chips and the palette), the "Selected" panel (P8; it becomes the inspector at ≥160 cols, or the preview header), and the per-view device-chooser step (P13).

**Startup** (decision needed, Q1): open Sessions with the sessions in the launch directory sorted first and the cursor on the most recent one. Files is one key away (`g f`). If there are no sessions, open Files at the launch directory as today. This fixes P18 and makes "start cx, resume work" a 1-key task.

---

## 4. Theme tokens

Semantic tokens only. Widgets never use raw colors. `bg` stays the terminal default (`Color::Reset`), so transparency and user palettes keep working. Only selections and chips paint a background.

### 4.1 Token table (dark / light)

| Token | Use | Truecolor dark | Truecolor light | 256 dark / light | 16-color | NO_COLOR substitute |
|---|---|---|---|---|---|---|
| `fg` | body text | default fg | default fg | default | default | default |
| `fg.muted` | secondary text, ages | `#8b93a1` | `#5c6470` | 246 / 241 | `brightblack` | dim |
| `fg.faint` | borders, separators, hints | `#4a515c` | `#c3c8d0` | 239 / 251 | `brightblack` | dim |
| `accent` | focus border, focused title chip, cursor bar | `#6cc4d6` | `#0b7285` | 80 / 30 | `cyan` | bold + reverse title |
| `accent.bg` | selected row in focused panel | `#1d3a42` | `#d3eef3` | 23 / 195 | `reverse` | reverse |
| `select.dim.bg` | selected row in unfocused panel | `#262b33` | `#eceff3` | 236 / 255 | none (use `▏` marker) | `>` marker only |
| `ok` | reachable, running | `#8fcf9d` | `#2b8a3e` | 114 / 28 | `green` | `●` glyph |
| `warn` | stale, slow, password needed | `#e3c275` | `#a66a00` | 179 / 130 | `yellow` | `◐` / `~` |
| `err` | unreachable, failed | `#e8837f` | `#c92a2a` | 210 / 160 | `red` | `✕` / `x` + bold |
| `info` | checking, in-flight | `#8fb0e0` | `#1c64b8` | 110 / 25 | `blue` (bold) | `◌` / `.` |
| `mesh` | mesh membership, mesh edges, mesh rail | `#c49cf0` | `#7048c0` | 177 / 97 | `magenta` | `◆` / `*` badge |
| `lan` | LAN neighbours, LAN edges | `#7fc9b4` | `#0f7a62` | 79 / 29 | `cyan` | `~` badge |
| `dir` | directory names | `#8fb7f0` (bold) | `#1d4f91` (bold) | 111 / 25 | `blue`+bold → **`brightblue`** | bold + trailing `/` |
| `prov.claude` | provider badge | `#e0a37a` | `#a4532a` | 173 / 130 | `yellow` | `CL` text |
| `prov.codex` | provider badge | `#9ec5f8` | `#2659a8` | 111 / 25 | `blue` | `CX` text |
| `prov.shell` | provider badge | `#aab2bf` | `#4b5563` | 249 / 240 | default | `SH` text |
| `id.0..7` | per-device identity hue (hash of device id) | 8 hues at L≈72 C≈0.09 | same hues at L≈45 | nearest 256 | rotate `cyan, magenta, green, yellow, blue, red, brightcyan, brightmagenta` | device initials in brackets `[la]` |

Rules:

- In 16 colors, directory text uses `brightblue` + bold. Plain `blue` fails contrast on dark backgrounds (P9).
- Text tokens must reach WCAG 4.5:1 against both a `#000`-`#282c34` dark range and a `#fff`-`#f0f0f0` light range. The critic checks this from the PNGs (section 12).
- Identity hues are assigned by stable hash, so a device keeps its color across sessions, views and restarts. Collisions among up to 8 devices are resolved by first-come order within the inventory.
- **Light/dark detection**: `CX_THEME`, then an OSC 11 background query (100 ms timeout; skipped when stdin is not a TTY), then `COLORFGBG`, then dark. Under tmux, OSC 11 is answered by tmux ≥3.3 with the outer terminal's color. On older tmux the query times out and dark is used.

### 4.2 Glyph sets

| Purpose | Unicode | ASCII (`CX_ASCII`, `TERM=dumb`, non-UTF-8 locale) |
|---|---|---|
| Border unfocused | `╭─╮│╰╯` (rounded, `fg.faint`) | `+-+` `\|` |
| Border focused | `┏━┓┃┗┛` (heavy, `accent`) | `#=#` `#` (corners `#`, horizontal `=`, vertical `#`) |
| Status | `● ◐ ◌ ✕ ·` | `+ ~ . x -` |
| Selection bar | `▌` (accent) / `▏` (dim) | `>` / `:` |
| Access arrows | `⇄ → ← ⇢ ⊘` | `<> -> <- ~> x` |
| Transport badges | `◆ mesh` `↳ via` `~ lan` `⌂ local` | `* mesh` `^ via` `~ lan` `@ local` |
| Sparkline | `▁▂▃▄▅▆▇█` | `_.-=#` (5 levels) |
| Bars | `▏▎▍▌▋▊▉█` | `#` and `.` |
| Spinner | `◐◓◑◒` | `-\|/` (one cell) |
| Tree | `├ └ │` | `\|- \`- \|` |

Nerd Font icons are off by default. The detection heuristic (section 9) only flags `nerd_font_likely`. Icons appear only with `CX_ICONS=1` (an escape hatch, not a setting), because a wrong guess produces tofu boxes that are worse than no icons.

---

## 5. Focus treatment

The focused panel must be identifiable in under 200 ms from a thumbnail. Four cues stack, so at least two survive any capability level:

1. **Border**: focused panels use heavy `┏━┓` in `accent`. Unfocused panels use rounded `╭─╮` in `fg.faint`.
2. **Title chip**: the focused title renders as a reversed chip ` Sessions 3 ` (`accent` bg, terminal bg as fg). Unfocused titles are `fg.muted`, not cyan (fixes P1: today every title is cyan).
3. **Selection bar**: the focused panel's selected row gets `accent.bg` plus a left `▌`. Unfocused panels keep only a dim `▏` and `select.dim.bg`, so you can see where you will land when you return.
4. **Footer chips** name the focused panel's actions (`Enter attach · i type · z zoom`).

NO_COLOR: heavy border + reversed title + `>` bar. ASCII: `#=#` border + `[ Sessions ]` title in reverse + `>`.

Focus moves by Tab/Shift+Tab (cycle), Ctrl+arrows (directional), click/tap on any panel, and Esc (move up one level: preview → list → rail). Focus never moves by itself, except on explicit transitions (create lands attached, detach returns to the list row).

---

## 6. Size classes and layouts

Width classes: **XS** <60 (phones), **S** 60-99, **M** 100-159, **L** ≥160. Height: when rows <20, the footer collapses to one line and the top bar merges into the status line.

### 6.1 Sessions

**L (200x60)**: rail · list · live preview · inspector

```
 cx  ▸Sessions  Files  Network  Secrets        laptop ● 0ms   lab-arm ● 12ms ▁▂▁▃   field-rover ✕ 5m     ⇅ idle   ⋯
╭ Devices ───────────────╮┏━ Sessions 3 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓╭ build watcher · laptop ──────────────────────────╮╭ Inspector ───────────────╮
│ ▌● laptop     ⌂ local 2│┃ ▌● SH build watcher      ~/projects/cx-demo   [la] ▁▃▇▅ 4s┃│Compiling cx-demo v0.1.0                          ││ build watcher            │
│ │● lab-arm  ◆ ⇄ key  1 │┃  ● SH robot-logs         ~/projects/robot-logs[la] ▅▇▃▁ 9s┃│    Finished dev in 2.41s                         ││ shell · bash · pid 4182  │
│ │✕ field-rover ◆ → pwd │┃  ● SH ros core           ~/catkin_ws          [lb] ▁▁▂▁ 1m┃│[Running cargo check]                             ││ ~/projects/cx-demo       │
│   ↳ via lab-arm         │┃                                                          ┃│fixture:cx-demo$ █                                ││ laptop · local · ⌂       │
│ + Add device            │┃                                                          ┃│                                                  ││ up 2h 14m · idle 4s      │
│                         │┃                                                          ┃│                                                  ││ activity 60s ▁▃▇▅▂▁▁▃    │
│ Mesh team  ◆ 2/3 online │┃                                                          ┃│                                                  ││                          │
│ ▁▂▂▃ rtt p50 14ms       │┃                                                          ┃│                                                  ││ [⏎ Attach] [i Type]      │
╰─────────────────────────╯┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛╰ live · 4 Hz · 120x32 ─────────────── [⤢ zoom] ───╯╰──────────────────────────╯
 [⏎ Attach]  [i Type]  [z Zoom]  [w Watch]  [n New]  [d Stop]                                             3 sessions · 2/3 devices · mesh ◆ ok        ? help
```

Notes: the left mesh rail `│` in `mesh` color joins mesh members (P6). Preview identity and liveness move to the border (`live · 4 Hz · 120x32`, or `stale 12s` in `warn`) instead of using a content row (P4).

**M (120x40)**: rail · list · preview (no inspector; inspector facts go in the preview title)

```
 cx  ▸Sessions  Files  Network  Secrets               lab-arm ● 12ms  field-rover ✕ 5m   ⋯
╭ Devices ─────────╮┏━ Sessions 3 ━━━━━━━━━━━━━━━━━━━━━━━━━━━┓╭ build watcher · laptop · idle 4s ────╮
│ ▌● laptop      2 │┃ ▌● SH build watcher   cx-demo   [la] 4s┃│Compiling cx-demo v0.1.0              │
│ │● lab-arm  ◆ ⇄ 1│┃  ● SH robot-logs      robot-logs[la] 9s┃│    Finished dev in 2.41s             │
│ │✕ field-rov ◆ → │┃  ● SH ros core        catkin_ws [lb] 1m┃│fixture:cx-demo$ █                    │
│ + Add            │┃                                        ┃│                                      │
╰──────────────────╯┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛╰ live 4 Hz ─────────────── [⤢] ───────╯
 [⏎ Attach] [i Type] [z Zoom] [n New]                                    3 sessions · ⇅ idle  ? help
```

The rail widens to fit the longest device name up to 24 cols (P6: no `field-rov…` when space exists).

**S (80x24)**: device chip strip on top, list over preview

```
 cx ▸Sessions Files Network Secrets                             ⇅ idle   ⋯
 [All 3] [● laptop 2] [● lab-arm ◆ 1] [✕ field-rover ◆]
┏━ Sessions 3 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ ▌● SH build watcher        ~/projects/cx-demo        [la] ▁▃▇▅  4s      ┃
┃  ● SH robot-logs           ~/projects/robot-logs     [la] ▅▇▃▁  9s      ┃
┃  ● SH ros core             ~/catkin_ws               [lb] ▁▁▂▁  1m      ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
╭ build watcher · laptop · live ─────────────────────────────────── [⤢] ──╮
│Compiling cx-demo v0.1.0                                                 │
│    Finished dev in 2.41s                                                │
│fixture:cx-demo$ █                                                       │
╰─────────────────────────────────────────────────────────────────────────╯
 [⏎ Attach] [i Type] [z Zoom] [n New]                               ? help
```

**XS (40x20, phone)**: single column, bottom tab bar, contextual action bar

```
 cx · Sessions          ⇅  ⋯
 [All] [●laptop] [●lab-arm◆]
┏━ 3 sessions ━━━━━━━━━━━━━━━┓
┃▌● SH build watcher  la  4s ┃
┃  ~/projects/cx-demo        ┃
┃ ● SH robot-logs     la  9s ┃
┃  ~/projects/robot-logs     ┃
┃ ● SH ros core       lb  1m ┃
┃  ~/catkin_ws               ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
╭ peek · live ───────────────╮
│    Finished dev in 2.41s   │
│fixture:cx-demo$ █          │
╰────────────────────────────╯
 [ Attach ] [ Type ] [ New ]
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 ▸Sess │ Files │ Net │ Keys
```

At XS each session uses 2 rows (identity, then path tail) and the peek shrinks to the last 2-4 lines. Tab bar cells are ≥8 cols wide, and action chips ≥10 cols.

### 6.2 Files

**M/L**: rail · list · inline preview (L adds a second pane for transfers: `Tab` toggles panes, `y`/`p` copy across)

```
 cx  Sessions ▸Files  Network  Secrets                                             ⇅ 1 ▕████▍    ▏ 61% 4.2MB/s   ⋯
╭ Devices ─────────╮┏━ laptop › ~ › projects › cx-demo ━━━━━━━━━━━━━━━━━━━━━━━━┓╭ README.md · 69 B · markdown ─────────────╮
│ ▌● laptop        │┃   ▸ docs/                       1 item           2m      ┃│ cx demo                                  │
│ │● lab-arm  ◆ ⇄  │┃   ▸ src/                        1 item           2m      ┃│                                          │
│ │✕ field-rover ◆ │┃     Cargo.toml          toml    45 B  ▏          2m      ┃│ A fixture project for UI captures.       │
│                  │┃ ▌   README.md           md      69 B  ▏          2m      ┃│                                          │
│ Bookmarks        │┃     data.bin            bin   16 KiB  ▊          2m      ┃│ • item one                               │
│  ~/projects      │┃                                                          ┃│ • item two                               │
│  ~/Downloads     │┃                                                          ┃│                                          │
╰──────────────────╯┗━ 5 items · 16.2 KiB · hidden off ━━━━━━━━━━━━━━━━━━━━━━━━━┛╰──────────────────────── [⏎ open] [⤢] ─────╯
 [⏎ Open] [y Copy] [x Cut] [t Send to…] [r Rename] [Space Mark]                                    5 items · 1 marked  ? help
```

- The breadcrumb is the panel title. Each segment is clickable. When it is too long, middle segments collapse (`laptop › … › projects › cx-demo`), keeping the root device and the tail.
- Row: selection bar · dir chevron or space · name (cut in the middle, keeping the extension: `long-fi…name.toml`) · type tag (`fg.muted`, category hue) · size right-aligned with a relative micro-bar · relative mtime.
- Mode text (`NORMAL`, `VISUAL`) is replaced by state shown only when non-default, e.g. `3 marked`, `range`, `filter: *.rs` chips in the panel footer (P9).
- Preview is inline and automatic at ≥100 cols, with a 150 ms debounce after selection settles. Enter on a file opens a full-width preview. At <100 cols, Enter opens the preview in place of the list.

**S/XS**: list only. The preview replaces the list on Enter, and `Esc` returns. The breadcrumb collapses to `… › cx-demo`, with the device chip on the line above.

### 6.3 Devices (rail and Devices tab at XS)

```
 Devices                       sessions  rtt        seen
 ▌● laptop        ⌂ local         2      ·          now
 │● lab-arm       ◆ team ⇄ key    1      12ms ▁▂▁▃  3s
 │✕ field-rover   ◆ team → pwd    0      ·          5m
  ◌ jetson-01     ↳ lab-arm → key 0      checking…
  ~ 192.168.x.x   ~ lan (via lab-arm)    ssh open   40s
  + Add device
```

Every device row, mesh or not, has the same column order: status · name · transport badge (+ mesh name) · access arrow + auth · session count · rtt sparkline · seen age. A mesh member additionally gets the mesh rail in `mesh` color down the left margin, which groups members visually and makes membership feel connected (requirement: mesh should feel visual).

### 6.4 Network graph (hub)

See section 10 for rendering. **L (200x60)**:

```
 cx  Sessions  Files ▸Network  Secrets                                                        observed 3s ago · 2/3 reachable    ⋯
┏━ Topology ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓╭ Edge laptop → lab-arm ───────────╮
┃                                   ╭┄┄┄┄┄┄┄┄┄┄┄┄┄ mesh team ◆ ┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄╮                            ┃│ route   direct over mesh         │
┃  ╭──────────────╮   key ⇄        ┆  ╭──────────────╮        ╭──────────────╮   ┆                            ┃│ auth    key (auto)               │
┃  │ ● laptop     │━━━━━━━━━━━━━━━━━━━▶│ ● lab-arm    │━━━━━━━━▶│ ✕ field-rover│   ┆                            ┃│ reverse key (core) ⇄             │
┃  │ ⌂ you · 2 ses│◀━━━━━━━━━━━━━━━━━━━│ ◆ 12ms · 1 se│  pwd → │ ◆ seen 5m    │   ┆                            ┃│ rtt     12ms ▁▂▁▃▂ p95 31ms      │
┃  ╰──────┬───────╯                ┆  ╰──────┬───────╯        ╰──────────────╯   ┆                            ┃│ checked 3s ago (authenticated)   │
┃         │ lan enp5s0              ┆         │ jump ↳                             ┆                            ┃│                                  │
┃         ┊                         ╰┄┄┄┄┄┄┄┄┄│┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄╯                            ┃│ Policy                           │
┃  ╭──────┴────────╮                         ╰─────────▶╭──────────────╮                                       ┃│  (•) key auto      1             │
┃  │ ~ 10 LAN  +   │                            key →   │ ◌ jetson-01  │                                       ┃│  ( ) password req. 2             │
┃  │ 1 ssh-open    │                                    │ ↳ via lab-arm│                                       ┃│  ( ) one-way →     3             │
┃  ╰───────────────╯                                    ╰──────────────╯                                       ┃│  ( ) none          4             │
┃                                                                                                              ┃│                                  │
┃  legend  ━ key  ┄ password  ▶ direction  ⊘ none  ◆ mesh  ~ lan  ↳ jump                                       ┃│ [⏎ Apply] [s Sessions] [f Files] │
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛╰──────────────────────────────────╯
 [⏎ Open] [Tab Next edge] [1-4 Policy] [e Enrol neighbour] [+ Add device] [r Refresh]                                          ? help
```

- Nodes are 16x3 boxes: status + name / badge + one metric. The selected node gets a heavy accent border. A selected edge is redrawn in `accent` heavy, with its label shown as a chip.
- The LAN cluster under an observer collapses to a pill (`~ 10 LAN +`). Enter or click expands it into a leaf column, and selecting a leaf shows the neighbour inspector with `[e Enrol]`.
- The inspector (right) shows the selected node or edge. Edge policy is a radio list driven by keys `1-4` or by tap.

**M (120x40)**: the graph takes the full width, and the inspector becomes a 6-row drawer at the bottom. **S (80x24)**: the graph with compact 12x2 nodes (`● lab-arm` / `◆ 12ms`); the drawer is 4 rows. **XS (<60)** and graphs over 24 nodes fall back to the **outline view**, a tree with the same glyph semantics:

```
┏━ Network · outline ━━━━━━━━━━┓
┃▌● laptop  ⌂ you              ┃
┃ ├━▶ ● lab-arm ◆ ⇄ key 12ms   ┃
┃ │   ├━▶ ✕ field-rover ◆ → pwd┃
┃ │   └─▶ ◌ jetson-01 ↳ key    ┃
┃ └┄  ~ 10 LAN (enp5s0)  +     ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
 [ Open ] [ Policy ] [ Enrol ]
```

### 6.5 Secrets

```
┏━ Secrets ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃   NAME                     VALUE             SOURCE          AGE   USED BY    ┃
┃ ▌ Cloudflare API token     •••••••• (40)     file 0600       3d    mesh setup ┃
┃   Mesh worker token        managed remotely  cloudflare      3d    mesh       ┃
┃   Claude gateway lend      2 active          mesh endpoint   1h    claude lend┃
┃   SSH identity             ~/.ssh/id_ed25519 agent: no       ·     ssh routes ┃
┃                                                                               ┃
┃   Cloudflare token missing? [ Create token ↗ ]  [ Paste token ]               ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
 [⏎ Details] [v Reveal 10s] [c Copy] [p Replace] [d Remove]             ? help
```

- Values are always masked as dots with the length in parentheses. No partial characters, not even the last 4. `v` reveals for 10 s and only while the Secrets panel stays focused. The value is drawn in the alternate screen only, so it never reaches scrollback. Capture tests never invoke reveal.
- SSH identities show path and fingerprint only, never key material. Provider credentials (Claude/Codex) are listed as "present / absent" only and are never read (AGENTS.md: never alter provider credentials).
- **Cloudflare onboarding**: `[ Create token ↗ ]` opens `https://dash.cloudflare.com/profile/api-tokens` with a prefilled template (`?permissionGroupKeys=[…]&name=cx-mesh`), built from the permission list in `docs/mesh.md`. It uses the viewer's existing browser opener. With no `DISPLAY`/`WAYLAND_DISPLAY` (SSH, phones), it shows the URL as an OSC 8 hyperlink plus `[c Copy URL]` (OSC 52). `[ Paste token ]` opens a masked input, and Enter verifies it through the mesh util's existing token check, then saves it with mode 0600 where `cx mesh setup` already looks. The exact permission-group keys must be checked against Cloudflare's current list during implementation; see section 14.

### 6.6 Command palette

One overlay box, no stacked sub-boxes (P12). It is centered at ≥80 cols and full-width at the bottom on XS. The background is dimmed, not drawn through.

```
╭ ⌕ att█                                                         ╮
│ Sessions                                                       │
│ ▌● SH build watcher      laptop   ~/projects/cx-demo       ⏎   │
│  ● SH ros core           lab-arm  ~/catkin_ws              ⏎   │
│ Actions                                                        │
│   Attach selected session                                  ⏎   │
│   Watch selected session (read-only)                       w   │
│ Go to                                                          │
│   Network                                                 g n  │
╰ ↑↓ choose · ⏎ run · Tab section · Esc close ───────────────────╯
```

Sources: sessions on all devices (switcher), devices, folders recently used, actions valid for the current selection (with keys), and goto. The ranking is fuzzy score, then recency. `:` opens the same palette prefilled with `run: ` for the existing run-command action.

---

## 7. Row anatomy

Columns are fixed per size class, so rows align across refreshes. Every cell is either data or `·`.

### 7.1 Session row

```
▌ ● SH  build watcher        ~/projects/cx-demo     [la]  ▁▃▇▅  4s   working
│ │ │   │                    │                      │     │     │    └ state text (only if not idle; warn/err colored)
│ │ │   │                    │                      │     │     └ idle-for age (since last output), fg.muted
│ │ │   │                    │                      │     └ activity sparkline, 60 s, from successive session snapshots
│ │ │   │                    │                      └ device identity chip (2 initials, id hue); omitted when scoped to one device
│ │ │   │                    └ folder, ~-relative, TAIL-preserving ellipsis (…/cx-demo)
│ │ │   └ title: provider title, else user name, else "<folder> shell" (never "New shell")
│ │ └ provider badge: SH / CL / CX in provider hue (Nerd/emoji never by default)
│ └ status: ● running/attached-elsewhere (ok), ◐ stale (warn), ✕ ended (err), ◌ checking (info)
└ selection bar
```

Pins sort first and show `⚑` before the badge. Group-by-folder headers are removed: the folder is a column, and a `▁` tick appears when the folder changes between adjacent rows. That saves 2 rows per session (P3). Attached-elsewhere shows `⧉` after the status.

### 7.2 Device row

See 6.3. Transport badge precedence: `⌂ local` > `◆ <mesh>` > `↳ via <jump>` > `ssh` > `~ lan`. Access arrow: `⇄` both directions proven, `→` viewer→device only, `←` reverse only, `⊘` policy none, `?` unproven. Auth word: `key` / `pwd` / `·`. These replace the cryptic `*` (P6).

### 7.3 File row

```
▌ ▸ src/            dir     1 item  ▏      2m      (directory: dir token, bold)
▌   README.md       md       69 B   ▏      2m      (file: type tag colored by category: code, doc, data, archive, media)
▌ ✓ data.bin        bin    16 KiB   ▊      2m      (✓ marked, accent)
```

Size bars are scaled to the largest file in the listing. Symlinks show `→ target` in `fg.muted`. Unreadable entries show `✕` and the reason in the status line, not in the row.

---

## 8. Unified keymap

Mnemonic rules:

1. Lowercase letters are verbs on the selection. Shift is the "bigger or reverse" form (`n` new / `N` new with options, `t` transfer / `T` jobs).
2. `g`+letter goes somewhere (vim "go").
3. Ctrl is reserved for global, escape-like keys (`Ctrl+P` palette, `Ctrl+C` quit, `Ctrl+\` detach).
4. Digits remain counts in lists (`5j`). In the policy editor digits pick options, because counts do not apply there.

| Key | Action | Context | Mnemonic | Mouse / touch |
|---|---|---|---|---|
| `↑↓` / `j k` | move selection (counts: `5j`) | lists, graph (spatial) | vim | tap row; wheel scrolls viewport |
| `←→` / `h l` | up/into (files), fold/expand (tree), spatial (graph) | panel | vim | tap chevron / breadcrumb |
| `gg` / `G` / `Home` / `End` | first / last | lists | vim | — |
| `PgUp/PgDn` | page | lists, preview | — | wheel ×page with Shift |
| `Enter` | **primary**: attach session · open file/dir · open node · edit edge · run palette item | all | — | tap selected row again, or tap `[⏎ …]` chip |
| `Esc` | back / clear / focus up one level | all | — | tap `‹` in title or outside overlay |
| `Tab` / `Shift+Tab` | next / previous panel (Files L: other pane) | all | — | tap panel |
| `Ctrl+←↑↓→` | directional panel focus | all | — | — |
| `g s` `g f` `g n` `g k` | go to Sessions / Files / Network / Secrets (k = keys) | global | go | tap tab |
| `[` / `]` | previous / next view | global | bracket = cycle | swipe-free: tap tab |
| `Alt+1..4` | view accelerator where Alt reaches the app | global | — | — |
| `/` | search (incremental, Enter keeps filter) | lists | vim | tap `⌕` |
| `f` | filter list | lists | filter | tap filter chip |
| `Ctrl+P` / `:` | palette / palette in run mode | global | VS Code / vim | tap `⋯` |
| `?` / `F1` | contextual help | global | — | tap `? help` |
| `n` / `N` | new session here (provider chooser) / with device+folder options | global | new | `[n New]` |
| `i` | type into the inline preview (inline input); `Ctrl+\` leaves | Sessions | vim insert | tap preview |
| `z` | zoom the preview to the workspace (read-only, live) | Sessions, Files | tmux zoom | `[⤢]` |
| `w` | watch: native read-only attach | Sessions | watch | `[w Watch]` |
| `p` | pin/unpin session · paste (Files) | Sessions, Files | pin / paste | chip |
| `d` | stop session (confirm) · delete (confirm) | Sessions, Files | delete | chip |
| `s` | agent tasks (Sessions) · sessions in folder (Files) · sessions of node (Network) | — | sessions | chip |
| `y`/`c`, `x`, `t`, `T`, `r`, `M`, `.`, `Space`, `v`, `u`, `o` | unchanged Files verbs | Files | unchanged | chips + context menu |
| `e` | enrol selected LAN neighbour | Network | enrol | `[e Enrol]` |
| `1`..`4` | edge policy: key auto / password / one-way / none | Network edge | ordered strict→off | tap radio |
| `+` / `a` | add device by SSH address | Devices, Network | add | `+ Add device` |
| `r` | refresh selected device (explicit; moving never refreshes) | Devices, Network | refresh | `[r]` |
| `v` / `c` | reveal 10 s / copy (OSC 52) | Secrets | view / copy | chip |
| `Ctrl+C` | quit viewer (work keeps running) | global | — | — |
| **`Ctrl+\`** | **detach** from a native attach or leave inline input | inside sessions | "back-slash = back out" | tap `⏏ detach` chip in the tmux status line / preview border |

Removed: `Ctrl+]` (conflicts with Codex), `Ctrl+O` capture (Claude Code transcript), `Ctrl+h/j/k/l` focus (Ctrl+H is Backspace and Ctrl+J is LF in legacy encodings, so they are ambiguous), and hover-to-select.

**Detach chip**: the managed tmux status-left becomes `#[reverse] ⏏ detach #[noreverse] cx · <title>` with `bind -n MouseDown1StatusLeft detach-client`. `MouseDown1StatusLeft` works on tmux 2.x and later, and covers the bundled portable tmux. tmux ≥3.3 could use a `range=user|detach` region instead, which is not needed. The return key binding becomes `bind-key -n 'C-\' detach-client`. In the viewer, `session_pane_key` accepts `Ctrl+\`, the legacy `0x1c` (which crossterm reports as `Ctrl+4`), and kitty-protocol `Ctrl+\`.

### 8.1 Conflict audit

Scope A is cx's own UI (raw mode, nothing forwarded). Scope B is keys cx intercepts while the user is inside a session (native attach or inline `i`). Only scope B can break other programs, and it is now a single key.

| cx key | Scope | Codex CLI | Claude Code | tmux | bash/zsh/fish + readline | vim/nvim | Verdict |
|---|---|---|---|---|---|---|---|
| `Ctrl+\` | B | no binding known | no binding known | unbound by default | tty SIGQUIT to the foreground job (core dump); readline/zle/fish unbound | `Ctrl+\ Ctrl+N` leaves nvim `:terminal`/insert mode | **Accepted cost.** Loses SIGQUIT and the nvim terminal-escape inside managed sessions. A literal `Ctrl+\` can be sent from the palette ("Send Ctrl+\ to session"). External sessions keep their own bindings. Verify Codex and Claude bindings against installed versions before release (Q3). |
| `Ctrl+]` (removed) | B | conflict (reported by user) | — | — | telnet escape | tag jump | removed |
| `Ctrl+O` (removed) | B | — | transcript/verbose toggle | — | operate-and-get-next | jump back | removed; promote inline→native via `Ctrl+\` then `Enter`, or `[⤢]` |
| `Ctrl+P` | A | (not forwarded) | — | — | previous history | — | safe: scope A only |
| `Ctrl+C` | A | — | — | — | — | — | quits viewer only; inside a session it reaches the app |
| `Ctrl+S`/`Ctrl+Q` | A | — | — | — | XON/XOFF | — | crossterm raw mode clears IXON; not bound |
| `Ctrl+Z` | A | — | — | — | suspend | — | unbound; cx does not suspend (tmux sessions are separate anyway) |
| `Alt+digit` | A | — | — | — | readline digit-argument | — | scope A only; often eaten by macOS Option / phone keyboards, so it is an accelerator, never the only path |
| `g` prefix | A | — | — | — | — | — | `gg` kept; `g`+view letter is new; pending `g` times out after 1 s with no action |
| `[` `]` | A | — | — | — | — | — | free in cx today |
| `i`, `z`, `e`, `1-4` | A | — | — | — | — | — | free in their contexts today (`i` unused; `z` unused; `e` = edit device in Devices stays there; digits remain counts except in the policy editor) |
| Nested tmux (cx run inside the user's tmux) | B | — | — | outer prefix (`C-b`) reaches the outer tmux first | — | — | unchanged; `Ctrl+\` is unbound in default tmux so it passes through |

---

## 9. Terminal capability detection

Computed once at startup into a `Caps` struct. It is logged only as a single bounded line in debug builds (no environment dump). All fields have fallbacks, and the result never blocks startup for more than 100 ms.

| Signal | Detection | Effect |
|---|---|---|
| UTF-8 | `LC_ALL` > `LC_CTYPE` > `LANG` contains `UTF-8`/`utf8` (case-insensitive) | not UTF-8 → ASCII glyph set |
| ASCII forced | `CX_ASCII` set, or `TERM=dumb`/`linux` (the console font lacks many glyphs) | ASCII glyph set (borders too, fixing P2) |
| Truecolor | `COLORTERM` in {`truecolor`,`24bit`}; or `TERM` ends `-direct`; or terminfo `Tc`/`RGB` capability; or `TERM_PROGRAM` in {`iTerm.app`,`WezTerm`,`ghostty`,`vscode`}; `KITTY_WINDOW_ID`; `ALACRITTY_SOCKET`/`ALACRITTY_WINDOW_ID`; foot (`TERM=foot*`) | truecolor tokens |
| 256 colors | `TERM` contains `256color`, or terminfo `colors >= 256` | 256 tokens |
| 16 colors | otherwise | 16 tokens |
| No color | `NO_COLOR` set (any value, per no-color.org) | attributes only |
| tmux | `TMUX` set | Inside tmux, truecolor only if the outer terminal reports it via `COLORTERM`, or `tmux display -p '#{client_termfeatures}'` contains `RGB` (a bounded read-only query of the user's own tmux, 200 ms). Kitty graphics and OSC 52 need `allow-passthrough`/`set-clipboard`; cx does not change the user's tmux. |
| Termux | `TERMUX_VERSION` or `PREFIX` containing `com.termux` | phone defaults: XS layout even at 60-70 cols when rows > cols×0.5, larger chip padding, prefer the tap bar |
| Blink (iOS) / mosh | `TERM_PROGRAM=Blink` (when forwarded), `LC_TERMINAL`; mosh detected by parent `mosh-server` is not inspected | no OSC 11 query when unanswered within 100 ms; no reliance on Alt |
| Kitty keyboard protocol | crossterm `supports_keyboard_enhancement()` | enables unambiguous `Ctrl+\`/`Ctrl+I` and so on; off otherwise |
| Mouse | always request SGR mouse (1006). Touch terminals deliver taps as clicks | the same targets serve mouse and touch |
| Hyperlinks (OSC 8) | allowlist: kitty, foot, ghostty, WezTerm, iTerm2, VTE ≥0.50 (`VTE_VERSION`), Alacritty ≥0.11 | links in Secrets/preview; otherwise plain URL + copy chip |
| Nerd Font | **heuristic only, default off**: `TERM_PROGRAM`/font hints never prove font presence | `nerd_font_likely` is recorded but icons need `CX_ICONS=1` |
| Background | OSC 11 query (see 4.1) | light/dark tokens |
| Shell | irrelevant to the viewer; only `SHELL` for new sessions (bash/zsh/fish all launch via the existing quoted launcher) | none |

Detection is unit-tested as a pure function `Caps::from(env, terminfo_probe)` with table tests for foot, kitty, ghostty, alacritty, tmux-in-kitty, Termux, Blink, `TERM=dumb`, `NO_COLOR`, and `LANG=C`.

---

## 10. Network graph rendering

### 10.1 Model

- **Nodes**: the viewer, enrolled devices, jump hosts, and LAN clusters (one per observer + interface; they expand to neighbour leaves).
- **Edges**: directed `(from, to, kind)`, where kind ∈ {`ssh-direct`, `mesh`, `jump`, `lan-observed`}. Each edge carries `policy` (section 10.5), `evidence` (last authenticated check, rtt samples), and `direction` (which way access is proven or allowed).
- **Mesh** membership is a node attribute, rendered as an enclosure, not as N² edges.

### 10.2 Layout

A layered (Sugiyama-lite) layout. It is deterministic, cheap for fewer than 50 nodes, and stable between frames.

1. **Rank**: BFS hop distance from the viewer over route edges. LAN clusters take rank = observer rank + 1. Unreachable devices take rank 1 with a dashed edge from the viewer.
2. **Order within rank**: barycenter heuristic, 4 down/up sweeps, seeded with the **previous frame's order** and device id as tiebreak. Topology changes therefore move as few nodes as possible.
3. **Mesh grouping**: mesh members within a rank are made contiguous (stable partition). The mesh enclosure is the bounding box of member nodes plus 1 cell, drawn with dashed `┄┆` in `mesh`.
4. **Coordinates**: rank → column at L (left-to-right reads "from you outward"), rank → row band at S/M when the width is too small for the rank count. Node size 16x3 (L/M), 12x2 (S). Gaps are ≥6 cols between ranks and ≥1 row between nodes.
5. **Overflow**: if the layout does not fit, collapse LAN clusters first, then show ranks ≥3 as `+N more` pills. If it still does not fit, or width <60, use the outline view (6.4).

### 10.3 Edge routing and glyphs

- Orthogonal (Manhattan) routing on the cell grid: an A* search over free cells, with costs bend = 4, crossing an existing edge = 6, step = 1, and running alongside a node border = 2. Ports sit on node sides, spread when a node has several edges.
- Box-drawing glyphs are merged per cell from a 4-bit direction mask (`─│┌┐└┘├┤┬┴┼`). Heavy variants (`━┃┏…`) mark key-auto edges and the selected edge, dashed variants (`┄┆`) mark password edges, and `⊘` sits mid-edge for policy none. Arrowheads `▶◀▲▼` (ASCII `> < ^ v`) at the target show the direction of access. A two-way edge gets heads at both ends.
- Labels (`key ⇄`, `pwd →`, `12ms`) are placed at the longest straight segment if they fit, otherwise shown only when the edge is selected.
- **Braille** (`⣿`, 2x4 subcells) is used only for the optional rtt sparkline inside the inspector and the mesh-cloud fill at L in truecolor. It is never used for edges: braille lines cannot carry color per edge cleanly, cannot be hit-tested per cell reliably, and degrade badly in fonts without braille.

### 10.4 Caching, hit testing and performance

- `GraphLayout` is cached, keyed by `(topology hash, area size, size class, glyph set)`. Status, rtt and policy changes only restyle cached cells. Layout and routing rerun only on topology or size change, and the target is ≤2 ms for 30 nodes in debug builds (checked by perf-harness).
- At render time, a hit map `Vec<Option<HitId>>` (one per cell) records the node rect, edge cells (including a 1-cell halo for touch), label chips and the enclosure title. Clicking an edge cell selects the edge, clicking a node selects the node, and tapping the selected item opens it.
- Keyboard: arrows move to the nearest node in that direction (angle within ±60°, then distance). `Tab`/`Shift+Tab` cycle the selected node's edges. `Enter` opens the node (scopes Sessions) or edits the edge. `Esc` clears edge selection, then node selection.
- Observation refresh is explicit (`r`) or periodic at the current cadence. Moving the selection never triggers I/O (P17).

### 10.5 Edge policy

Per directed edge, a local enrolment decision like today's `AccessMode`:

| Policy | Meaning | Rendering |
|---|---|---|
| `key` (auto) | key auth allowed in both configured directions; background checks may authenticate | heavy `━`, `⇄`/`→` |
| `password` | interactive auth required; no background authentication; prompts are masked as today | dashed `┄` + `pwd` |
| `one-way` | only `from → to` may be used; reverse routes and core setup are refused | single arrowhead, reverse end `⊘` |
| `none` | no route over this edge; enrolment and jumps refuse it | dotted `┈` faint + `⊘` |

Changing a policy shows a one-line consequence preview before Enter applies it ("lab-arm will no longer reach laptop; 1 jump route via laptop will be refused"). The change is serialized through the root's existing store functions, with no networking change applied by the UI.

---

## 11. Interaction flows: keystrokes before vs after

Counts are keystrokes (or taps) from the stated start, excluding typed names. "Before" was measured against v0.1.95 behavior in code and captures.

| # | Task | Before (v0.1.95) | n | After | n |
|---|---|---|---|---|---|
| 1 | Launch cx and resume the most recent session in this folder | `cx` opens Files → `Esc` → Sessions; `Enter` (focus inline) → `Ctrl+O` (native) | 3 | `cx` opens Sessions on the folder's latest → `Enter` | 1 |
| 2 | Leave a session and switch to the adjacent one | `Ctrl+]` → `j` → `Enter` → `Ctrl+O` | 4 | `Ctrl+\` (or tap ⏏) → `j` → `Enter` | 3 |
| 3 | Jump to a session by name on any device | `Esc` (focus devices) → `gg` `Enter` (All) → `/` text `Enter` → `Enter` → `Ctrl+O` | 7 | `Ctrl+P` text → `Enter` | 2 |
| 4 | Glance at a session's output without attaching | select → read (inline) | 1 | select → read (inline; `z` to zoom) | 1 |
| 5 | Type a quick command into a session without leaving cx | select → `Enter` → type → `Ctrl+]` | 3 | select → `i` → type → `Ctrl+\` | 3 |
| 6 | New shell in the current folder | `n` → `s` | 2 | `n` → `s` | 2 |
| 7 | Copy a file from laptop to a robot folder | select → `t` → choose device `j` `Enter` → navigate → `Enter` | 5+nav | select → `t` (opens on the last destination device and folder) → `Enter`; another place: select → `t` → tap device chip → navigate → `Enter` | 2 (repeat) / 4+nav |
| 8 | Switch from Files to Network | `Ctrl+P` "net" `Enter` → chooser `Enter` | 4 | `g n` (or tap tab) | 2 / 1 tap |
| 9 | Enrol a LAN neighbour seen by a robot | `Ctrl+P` "net" `Enter` → chooser `j` `Enter` → `l` (expand) → `j…` → `Enter` → account `Enter` | 8+ | `g n` → arrow to cluster → `Enter` (expand) → arrow → `e` → account `Enter` | 6 |
| 10 | Make laptop→lab-arm password-only | (not available; only `b` toggles core/directed on the device) | — | `g n` → select edge (`Tab`) → `2` → `Enter` | 4 |
| 11 | Set up the Cloudflare token for mesh | leave cx; `cx mesh setup`; find the token page and permissions in docs; paste | ∞ (out of app) | `g k` → `Enter` on Cloudflare → `Create token ↗` → paste → `Enter` | 4 |
| 12 | Stop a session | `d` → confirm | 2 | `d` → confirm | 2 |

---

## 12. Up/down smoothness (fixing "janky")

1. Selection moves are pure state changes. No I/O happens on move (P17). Device-scoped refresh is coalesced and debounced 250 ms after the selection settles. Provider-capability caches are not cleared by moves.
2. Hover never selects (P16). Hover may draw a faint underline at most.
3. Fixed row heights per size class (1 row S/M/L, 2 rows XS). No group-header rows, so each step moves the bar exactly one row.
4. Scrolloff 2: the list scrolls before the bar reaches the edge, and the bar never jumps to center.
5. The preview request is throttled: the latest selection wins, and in-flight replies for older selections are dropped by key (already keyed). The old frame stays visible, dimmed, until the new one arrives. It is never blanked.
6. Key repeat: bursts of more than 3 moves within 60 ms skip preview requests until 120 ms of idle.
7. Wheel scrolls the viewport by 3 rows. Selection follows only when it would leave the viewport.

---

## 13. Critic rubric for screenshot passes

Each pass runs:

```
cargo build && uv run --with pyte --with pillow python tests/capture_matrix.py target/debug/cx \
  local-evidence/ui-passes/passN --network
uv run --with pyte --with pillow python tests/capture_matrix.py target/debug/cx \
  local-evidence/ui-passes/passN/ascii-nocolor --sizes 80x24 --env CX_ASCII=1 --env NO_COLOR=1
```

A reviewer (human or agent) opens every PNG and scores each criterion 0-3 per size. A pass ships only when no criterion scores 0 and the mean is ≥2.5.

| # | Criterion | 3 (good) | 0 (fail) |
|---|---|---|---|
| C1 | Focus obvious | focused panel identifiable from a 25% thumbnail; ≥2 cues in ASCII/NO_COLOR | two panels could plausibly be focused |
| C2 | Identity distinct | any two sessions/devices differ in the first 20 visible cells | identical visible prefixes (P3) |
| C3 | Truncation | tails kept for paths, extensions kept, names never cut while space exists elsewhere | meaningful part elided while columns are empty |
| C4 | Consistent anatomy | same columns/glyphs for the same noun in every view and size | mesh/non-mesh or view-specific variants |
| C5 | Color semantics | one meaning per token; text contrast ≥4.5:1 (sample PNG pixels) | decorative color, or blue-on-dark dirs |
| C6 | Evidence | every status has an age/source; unknown renders as `·` | green/ready without evidence |
| C7 | Density | ≤35% empty interior at L/M, no row wasted on hints | large dead zones (P11) |
| C8 | Overlays | single box, dimmed background, nothing bleeds through, toasts never cover the selection | stacked boxes, or a toast over the list (P10, P12) |
| C9 | Affordance | every actionable thing looks tappable (chip/underline/chevron) and has ≥3-col targets on XS | actions only discoverable via help |
| C10 | Size-class fit | layout matches section 6 for its class; nothing overlaps; XS usable one-thumb | clipped columns, overlap, unreadable XS |
| C11 | ASCII/NO_COLOR parity | the same information is present, glyph substitutes applied, no Unicode in ASCII captures (`grep -P '[^\x00-\x7f]'` is empty) | info lost or Unicode leaks (P2) |
| C12 | Metrics | at least one relevant live metric per view at M/L (rtt, activity, throughput, sizes) | text-only |
| C13 | Next step | the footer offers the 2-4 most likely next actions for the focused selection | generic key list (P14) |

Pass 0 baseline scores (current UI, mean over sizes): C1 1, C2 0, C3 1, C4 1, C5 1, C6 2, C7 1, C8 0, C9 1, C10 1, C11 0, C12 0, C13 1.

---

## 14. Integration notes, contracts and open questions

**Root-owned or shared changes this design needs** (coordinate before implementing):

- `assets/tmux.conf`: `C-\` detach, the status-left detach chip, `MouseDown1StatusLeft`. Existing managed servers pick this up on the next server start. The `cx reset` path already reloads the owned config.
- **Session snapshot fields** for activity sparklines and idle age: the tmux `#{session_activity}` / `#{window_activity}` timestamps. This is additive and optional (`#[serde(default)]`), and old helpers render `·`.
- **Device rtt**: measured in the viewer from existing request round-trips. No protocol change.
- **Edge policy store**: a new local file (`edges.json`, mode 0600, under the device lock), separate from `Device`, like `DevicePolicy`. The existing `AccessMode::Core/Directed` maps to `key ⇄` / `key →`. This is a costly-to-reverse contract: it needs the root's sign-off (Q2).
- **Secrets inventory**: read-only presence checks of known paths/env names. Writes only for the Cloudflare token, through the mesh util's existing save path. Provider credentials are never read.
- Cloudflare token template URL: verify the `permissionGroupKeys` names against Cloudflare's current permission-group list during implementation. This is a bounded technical check, not a user question.

**Suggested implementation order** (each step is independently shippable): (1) theme tokens + caps + focus treatment + ASCII borders; (2) jank fixes (P16, P17) + fixed row anatomy; (3) Enter-attaches, `i`/`z`, `Ctrl+\` + chip; (4) palette switcher + `g` goto + tab bar + XS layout; (5) Files row/breadcrumb/preview polish; (6) Network graph (layout module with pure unit tests, then render); (7) Secrets.

**Open questions for the user** (batched, at most 4):

- **Q1 (HIGH VALUE)**: Should a plain `cx` open **Sessions** (latest session in this folder pre-selected; Files one key away), instead of Files at the launch directory as today?
- **Q2 (BLOCKING for step 6)**: Edge policies replace the per-device `b` core/directed toggle with per-edge `key / password / one-way / none`. Is a per-edge, per-direction policy the model you want, or should policy stay per device?
- **Q3 (HIGH VALUE)**: `Ctrl+\` inside sessions gives up SIGQUIT and nvim's `Ctrl+\ Ctrl+N`. Is that acceptable, given the tappable ⏏ chip and the palette "send Ctrl+\" fallback?
- **Q4 (POLISH)**: Mesh membership in lists is shown as a colored left rail plus a `◆` badge. Is that visual enough, or do you want mesh members grouped under a "mesh" header?
