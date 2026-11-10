# Concept: mission control

Code: `src/ui/lab/concepts/mission/` (`mod.rs` render context and shared helpers,
`map.rs` the network schematic and its line router, `tiles.rs` device tiles and session thumbnails,
`views.rs` one function per screen). Render: `tests/lab.sh mission`.

A calm, dense-but-airy dashboard: btop refined by a Swiss designer. The start screen is
an instrument panel with two zones. On top, the network: a star around this device, every
node a small card with its own cpu trend, temperature and session pips, every link
carrying its access policy and rtt. Below, the sessions as live thumbnails you recognise
by how they actually look. Each screen is one grammar: **a device letter, then
a verb or a session slot**.

## 1. Principles

1. **The schematic is the hub.** You do not pick from a list of devices; you look at the
   network and press the letter printed on the node. Every node, edge chip, tile and
   chip on screen is also a click/tap target that maps to the same action as its key.
2. **Recognise, don't read.** A session tile shows the real last informative lines of its
   screen (agent status, the approval prompt, the failing test) in the screen's own
   colours, in every colour tier. Two Claude sessions look different because their
   screens do.
3. **Evidence on the surface, said once.** Every device shows how its status is known:
   the rtt rides on its link chip, the node card carries cpu history, temperature and
   its sessions' slot letters in their state colours (`a s`: how many run there and the
   key that attaches each), a down node says `seen 3h`. Unknown is `?`, never green.
   Under the map a lens row names the selected device with its route from here and the
   rtt of every hop (`t → c → b  0.6 + 2.4 ms`), then the keys that act on it from the
   hub (`b a attach  b c new claude  b f files  e b link`). Phones show the same lens
   as a device row.
4. **Tints, not boxes.** Panels are surfaces (background tints). Each zone opens with a
   plain small-caps label followed by a faint rule (`▍NETWORK ──────`; no letter
   spacing, so it copies and reads aloud as a word). Boxes are used only for the dashed
   "new session" ghost tile. Without tints (16 colours, NO_COLOR) tiles are separated
   by a faint `│` gutter.
5. **Focus is a colour bar, in every tier.** The focused zone's label gets an accent bar
   `▍` (ASCII `> `); its selected item gets a left bar (`▍`/`▌`; ASCII `>` on its first
   row and `|` below) and a bold accent name, plus `accent.bg` where tints exist. 16
   colours and NO_COLOR use the same bar, never a reversed block. Column 0 is the focus
   bar only: what needs you says so by its warn tint and `◆`, never a second bar. Every
   zone always shows its selection: unfocused zones keep a muted bar (the hub's
   first-ranked tile has one at launch, so `Tab ⏎` is predictable).
6. **Every action flows into the next.** A letter lands you on the device with its
   sessions under the home row; a slot key attaches straight away (no preview chain).
   Detach (`Ctrl+\`) returns to the hub with the tile you left selected and the sessions
   zone focused, so `⏎` re-attaches; `-` re-attaches the last detached session from any
   screen, so `Ctrl+\` / `-` is a two-key round trip (vim `Ctrl-^`, tmux `prefix l`).
7. **One address per session.** A session is always shown as its device keycap plus its
   slot keycap (`i` `a`) on hub tiles, device tiles, phone rows, the peek filmstrip, the
   palette and help. That is exactly what you type from the hub. There are no hub digits.

## 2. Screens

| View | What it shows |
|---|---|
| Hub | NETWORK (header with the mesh swatches, which are the only coloured squares, and the policy line-style legend; the star schematic, as tall as its content plus a pad row above and below; at M+ the lens row) and SESSIONS (thumbnails ranked by attention, then the ghost tile "new on <selected device>" with its real keys `t c`, `t x`, `t n`, content-sized and top-aligned). The grid picks its column count for the fewest empty cells, then the least height left over: at 200x60 five sessions become 2 x 3 tiles nearly 1:1 with their screens. Tiles never grow past the rows their screen needs. Launch selection is the viewer pill. The footer follows the focused zone: network `⏎ open  e link  - last  tab sessions`, sessions `⏎ attach  spc peek  ⌫ stop  - last  tab network`, then `/ find  + invite  K keys  ? help`. Phone (< 70 columns): the header with the policy legend, the keycap schematic (its outermost column keeps short names), the lens row, every session ranked by attention as a two-row target (what needs you first, warn-tinted, "is asking · tap to answer"), and a footer that leads with `ret` and wraps into the spacer row instead of dropping verbs. |
| Device (`letter`) | Header with identity, evidence, mesh squares, cpu chart and gauges (four rows, one blank row before SESSIONS); session thumbnails keyed `a s d g h l`, only as tall as their screen tails; the ghost tile "new on <device>" (`c` claude, `x` codex, `n` shell), content-sized, in the same row; under them LINKS: the same star from this device, its letters drawn as plain coloured text (in a device, letters are slots and verbs, so no keycap there may do something else), then (at M+, when rows are left) one block per link, left-aligned with the zone, with its network, policy, an rtt area chart as tall as the rows allow (up to 6), rtt and loss. Footer `⏎ a attach` (Enter and the slot keys on one chip); phones lead with `ret attach  esc back`. |
| Session peek (`␣`) | A meta line (address, state, provider, folder, uptime, clients, then the activity trend), the screen 1:1 on a deeper surface as tall as its content, and under it at M+ the OTHERS filmstrip: one row of equal live tiles across the full width filling the rest of the height. Short panes are bottom-anchored like a live terminal (the newest lines). Hub letters work in peek (`t a` attaches t's slot a; no inline typing, so letters are free; the footer says `t a… jump`) and `Tab` steps through the other peeks. `⏎` attaches, `⌫` stops, Esc goes back one level, `` ` `` to the hub. Phones drop the folder/uptime detail and `tab next` (tiles are taps) and keep `ret attach  esc back  del stop  ` hub`. Attached (`CX_LAB_FOCUS=attached`): the pane owns the terminal; tmux's status row carries the address and a `⏏ detach  ctrl+\` chip. |
| Files (`letter f`) | Tree with a size micro-bar and age, git letters; under it, when rows are left, the selected entry's path, size, age and git state and its send targets as device keycaps. Inline syntax preview at ≥100 cols, its surface as tall as the file, where a line cut at the edge ends in an accent `›`. The footer teaches `j k move  l h fold  s send to`; phones `ret open  esc back  s send  spc preview  / find  ` hub` (wrapped). `s` (send to) dims the files and raises a bottom sheet with the schematic seen from this device: press or tap the destination's letter (capture: `CX_LAB_FOCUS=dialog tests/lab.sh mission files`). The transfer then shows on the hub: its link heavy and lit, the rate on its chip, name and percentage in the top bar. |
| Palette (`/`) | Jump to anything: devices, sessions and actions (new claude / codex / shell, files, link), each with the keys that reach it directly (so the palette teaches the shortcuts). `/` is the one search everywhere; inside files or a preview it opens pre-filtered to that scope as the first section. Sized to its results, at most 60 % of the height, with a `⏎ go  ↑↓ move  esc close` row; on phones a full sheet on a cleared backdrop, descriptions cut with an ellipsis before the key column. |
| Keys / secrets | Masked secret cards keyed `a s d g` (a slot key opens that card's editor, so adding the Cloudflare key is `K a`, paste, `⏎`); missing ones get a `⏎ add` or `⏎ connect` button. Crumb `cx › keys`. |
| Onboarding | Viewer pill alone on an empty orbit, the wrapped one-sentence explanation, a `⏎ Connect Cloudflare` button, three plain numbered steps (`1.`, not keycaps: they are not keys), a breathing row, then `j` join code and `esc` plain SSH. |
| Join | Half-block QR (dark on light, always), the short code as keycaps, the one-liner wrapped over up to three rows (readable to retype where clipboard escapes fail) and an expiry bar. |
| Edge editor | Crumb `cx › t ⇄ i link`. The schematic right under the crumb, as tall as its content, with the edited link drawn heavy (`━┃┏┓`) at full mesh hue and the rest faded (letters as plain text: here `k p o` are policy keys). The sheet below has one segmented control per direction: the opener's outbound lane first with `k p o`, the way back with `K P O`, so no arrow keys are needed. A key applies at once, `u` undoes; one line says so and how many sessions cross the link, then the rtt history as a full-width area chart and, where rows remain, the live thumbnails of the crossing sessions with the link's figures beside them (now, min / max, mean, jitter, loss, network). |
| Help | Crumb `cx › help`. Centred vertically. A drawn keyboard per layer (hub: device keys with status and rtt when the keycaps are wide enough, `e link`, `/ find`; in a device: `attach` on the slot keys, `down` / `up` on `j k`, verbs), keycap width chosen so the staggered rows fit, a legend of device names and a key legend (including `- last session`), both on fixed column grids. Below 28 rows the device layer is one line of chips (`a s d attach  c x n new  f files  e link  ⌫ stop`). Phone: a list. |

## 3. The schematic

A star around the root (this device on the hub, the focused device in the device view).
The root is a one-row tinted pill in the middle carrying its own evidence (`t tranquility
▂▅▇ 47° ▖▖`). Devices it reaches over a routed link sit on both sides of it; devices
reached through them sit further out on their parent's side (at most three hops). Jump
edges are not drawn (the route is visible through the hop).

- **Halves.** Each first-hop device and everything behind it is one subtree; subtrees go,
  largest first, to whichever half stays shorter (ties go right). The left half is laid
  out exactly like the right one and mirrored through the pill, so there is one router.
  Mirrored nodes read toward the pill (`innovation ▄ i ●`), so the dot always meets its
  line. The drawing is centred in its band, both ways.
- **Lines** are box-drawing strokes with rounded elbows (`─ │ ╭ ╮ ╰ ╯`, ASCII `- | +`).
  **Hue means mesh and nothing else**: a mesh link is drawn in its mesh's hue, the same
  hue as the mesh squares after the names and in the header legend; lan and direct ssh
  links are neutral (their transport is named in the device view and the edge editor).
  **Line style is the access policy**, readable without colour: solid `─` when key
  access is allowed, double `═ ║ ╔ ╗` when only a password is, dotted `┈ ┊` when nothing
  is (or the link is down). ASCII: `-`, `=` (a double vertical is `|`, the `=` runs carry
  it), `.` / `:`. A trunk shared by two links takes the brighter, more solid style. The
  legend (`─ key  ═ pass  ┈ off`) is in the NETWORK header at every size and tier.
- **Peer links** (both ends away from the root, in one column) appear only while one
  end is selected: lit, with a chip, arcing over the pill through the top pad row (or
  under it) when both are their side's outermost cards. At rest the map shows the routes
  only, so no unlabelled stroke ever reads as decoration; any other peer link is left to
  the device view, where a device's neighbours include their own links.
- **No crossings by construction.** Links leave the pill from its top edge (targets
  above) or bottom edge (targets below), farthest target first, so elbows nest; a target
  on the pill's row gets a straight line; children leave their parent on a short trunk.
- **One chip per link**, one grammar: `[direction] [policy word] [rtt or rate]`, for
  example `⇄ key 14ms`, `→ pass 1.2ms`, `⊘ off`, `✕ down`, and `⇄ key ↓38M/s` on a
  link carrying a transfer. It sits on the link's last straight run with a tinted space
  each side and at least one line cell between it and any corner or dot, and is the
  link's tap target. The rtt is dropped before anything else; every link at M+ gets at
  least `glyph word` (the least gaps are sized for it: 9 columns next to the pill, 15
  between outer columns). Below 100 columns only links where nothing is allowed carry a
  chip (`⊘`, `✕`; ASCII `off`, `x`): there the line style and the legend carry the
  policy, and each run is a three-row tap target.
- **Motion** only where something moves: a link with an active transfer is heavy and
  lit (`━`, bold, full hue) and its chip carries the rate; a live build pulses that
  stroke's brightness at 2-4 Hz (synchronized output) only while the transfer runs.
  The former packet glyph read as a stray arrowhead and is gone.
- **Selection** puts the bar and tint on the node and lights its links plus its route back
  to the root (`t → c → b`). With the sessions zone focused, the schematic marks the
  selected session's device, so the two zones point at each other.
- **Node cards**: status dot, identity keycap, bold name, mesh squares (membership is
  said once, by the squares and the link hue); with height a second row with the cpu
  sparkline (M+) or cpu % (S), temperature and the slot letters. Label richness is
  chosen per frame (two-row, name, keycap only); names shorten to 12, 10, 8 characters.
  The pill, the hero, keeps its full name (and its mesh squares at M+) until nothing
  else fits, and a keycap-only map still names its outermost column on the outer side
  (`inno… i`, `b blas…`), where names cost no gap.
- **Spacing**: cards sit 3-5 rows apart (3-7 at L) inside the band, centred on the
  tallest column, one pad row above and below; even one-row columns get an even spacing
  so the pill row is exactly between them.
- The edge editor keeps both ends of an edited peer link on one side and draws it heavy
  as a bracket outside the column.
- Overflow per column becomes a `+N more` (`+N` on phones) target that opens the palette.
- Touch layouts (< 70 columns): node, pill, link and `+N` targets are at least 3 rows
  by 7 columns.

## 4. Palette tokens

Semantic tokens come from `caps.rs` (spec section 4.1); the concept adds blends, never
raw colours. `bg0` is only a blending reference: the terminal background stays default.

| Token | Use | Dark | Light | 256 | 16 | NO_COLOR / ASCII |
|---|---|---|---|---|---|---|
| `bg0` | blend reference | `#121519` | `#fafaf8` | n/a | n/a | n/a |
| screen | thumbnail and peek "glass" | `#0c0e11` | `#ffffff` | nearest | none | none |
| `fg` | names, titles | `#d5d9e0` | `#24292f` | default | default | default |
| `muted` | evidence, captions | `#8b93a1` | `#5c6470` | 246/241 | brightblack | dim |
| `faint` | grid dots, ghost outline | `#4a515c` | `#c3c8d0` | 239/251 | brightblack | dim |
| `accent` | focus bar, labels, pill | `#6cc4d6` | `#0b7285` | 80/30 | cyan | bold, `>` |
| `accent.bg` | focused selection | `#1d3a42` | `#d3eef3` | 23/195 | none: reversed name / `▸` | reverse, `>` |
| pill tint | viewer capsule | accent 22 % | accent 16 % | nearest | rounded box in accent | `+--+` box |
| `surface` | tiles | `#1b1f25` | `#f3f4f6` | 234/255 | none | none |
| heat ramp | cpu, temp, disk | ok → warn (60 %) → err | same | nearest | green/yellow/red at 60/85 % | ASCII bar glyphs |
| mesh hues | mesh 1, mesh 2 | `#c49cf0`, `#7fd8e8` | `#7048c0`, `#0a8090` | 177, 116 | magenta, brightcyan | `#` |
| `id.0..7` | device keycaps | spec hues | spec hues | spec | rotated 8 | reversed bold letter |

Session screens are mostly authored for dark terminals. On a light theme, dark screen
backgrounds are stretched to full saturation and then mixed 16 % into white (a diff row
becomes about `#d9ffe4` / `#ffdcdc`, like GitHub light), and pale inks are deepened 42 %
toward black (`tiles::adapt`). In 16 colours, named screen colours stay themselves, rgb
inks map by hue (a nearest-distance match turned muted reds grey), and coloured
backgrounds become ink of their hue, so `+` rows stay green and `-` rows red without a
loud block (`tiles::degrade`).

Glyphs, with ASCII fallbacks, each substitute meaning one thing: selection and focus
`▍ ▌` / `>` (and nothing else is `>` in ASCII); status `◉ ● ◐ ✕ ◌` / `@ o ~ x ?`;
session state `◆ ● ○ ·` / `! o - .` (`!` means needs you and nothing else; `*` stays
the agents' own bullet); alerts `▲` / `^`; policy `⇄ → ← ⊘` / `<> -> <- off`; line
styles `─ ═ ┈` / `- = .`; mesh squares `▄` / the mesh's number (`1`, `2`, repeated in
the legend, so membership reads without colour); the separator `·` / `-`; a cut `…` /
`..` (also inside fixture text); crumbs `›` / `/`; folds `▾ ▸` / `- +`; a line cut at
the preview edge `›` / `$`; keycaps are coloured cells in every tier
(`REVERSED|BOLD` without colour); link lines become `- | +`; `⎿` is `L` and `✻` is `*`;
charts become `_ . - = #` (`#` is used by nothing else); the minimap is omitted in ASCII
and NO_COLOR; the QR is drawn with `▀▄█` even in NO_COLOR and dropped only in ASCII.
Fixture text is transliterated in ASCII (`asciify`), so the lint stays at 0 warnings.
Inside thumbnails an emphasised lone letter (`a Always` in an agent prompt) takes the
muted caption ink, one step below every cx keycap and name, so cx keys stay unique.

## 5. Layout per size

Width classes as in the spec: XS < 60, S 60-99, M 100-159, L ≥ 160. Touch layouts are
everything under 70 columns (40x20 and 60x30 phones).

| | touch: 40x20, 60x30 | S 80x24 | M 120x40 | L 200x60 |
|---|---|---|---|---|
| Top bar | `cx`, crumb (earlier device crumbs as keycaps), `● 6/7 ▲1` | + transfer % and `?` | + transfer name and rate | + the alert headline (`▲ squirtle-jetson unreachable`), placed before the transfer |
| Schematic | keycaps, outer column named, chips only on closed links | names (≤ 10), chips only on closed links | two-row node cards, `glyph word` chips, lens row | chips with rtt or rate, more air |
| Below it | device rows (two rows: name, status, rtt; cpu trend, %, temp, pips) and a two-row "needs you" strip | — | — | — |
| Sessions | (in device rows and the strip) | 3 per row, 2 rows | 3 per row, 2 rows + ghost | 4 per row, taller tiles |
| Device view footer | `ret attach`, `esc back`, `f files`, `del stop`, `e link`, `` ` hub`` (two rows) | 7 chips | 7 chips | same |
| Hub footer | `ret open` / `ret attach` first, then `/ find`, `+ invite`, `K keys`, `? help` (wraps into the spacer row) | by zone, up to 8 chips | by zone | same |

The map band's pad rows separate NETWORK's header from the map and the map from
SESSIONS; a blank row always sits above the footer. Session tiles share one width and
one 2-column gutter; rows are equal height. Thumbnails show the last non-blank lines of
the screen and drop shared indentation when narrower than the screen; the agent's own
reversed key hints (`y Yes`) are drawn in flat thumbnail ink (no reverse, underline or
bold), so they never look like cx keycaps. A cut never shows two ellipses. Under 20x6
the frame shows only `cx`. Phones (and ASCII) label keys with words where symbols may be
missing from the font: `ret`, `del` (ASCII `bksp`); `spc` everywhere (the `␣` symbol
renders as a bare underscore). A phone footer leads with its primary verb, then
`esc back`, and every chip has a word.

## 6. Interaction model

Hub keys are non-letters plus `e`, so every other letter can be a device (a real build
never assigns `e` as a hotkey). Inside a device, letters are verbs and slots; the home row
holds the slots (`a s d g h l`; `f` stays files, `j k` stay motion as in files and every
list, so a vim hand never attaches by accident). More sessions than slots are reached
through `/`. Any list inside a screen uses the
same home-row slots (secrets cards are `a s d g`). Peek has no inline typing, so hub
letters work there unchanged. Uppercase letters are free on the hub, so the two global
panels use them.

| Key | Hub | Device | Peek | Mouse / touch |
|---|---|---|---|---|
| device letter | open that device | — | as on the hub | tap node, row or chip |
| `a s d g h l` | — | attach that session | (after a letter) attach | tap thumbnail |
| `c` / `x` / `n` | — | new claude / codex / shell here | — | tap ghost-tile option |
| `e` + letter | edit the link from this device to that one | edit the link to that peer | — | tap the link chip |
| `f` | — | files | — | tap `f files` |
| `s` (in files) | — | send the file: the schematic becomes the picker, a device letter sends | — | tap the node |
| `⌫` | stop the selected session tile (sessions zone; inline `y` / `n`) | stop the selected session; a slot key retargets (`⌫ s y`) | stop this one | tap `⌫ stop` |
| `⏎` | open selection (device or session) | attach selection | attach | tap selection |
| `␣` | peek selected session | peek | — | tap `␣ peek` |
| `Tab` | next zone (network, sessions) | next zone | next peek | tap a zone |
| `↑↓←→` | move spatially | move | — | — |
| `/` | find anything | find | — | tap `/ find` |
| `+` / `K` / `?` | invite / keys / help | | | footer chips |
| `Esc` | — | back one level | back to the device | tap the crumb or `esc back` |
| `` ` `` | — | to the hub from any depth | to the hub | tap `cx` |
| `-` | re-attach the last detached session | same | same | tap `- last` |
| `Ctrl+\` | — | detach from an attached session | | `⏏ detach` in the tmux status |
| `k p o` / `K P O`, `u` | edge editor: outbound / inbound key, password, off, applied at once; undo | | | tap segment |
| `y` / `Y` | copy (code, path, secret) / copy the alternate (command) | | | tap the code or line |

`/` is the one search: the global palette, pre-filtered to the current scope (this
device's files in files, this file in a preview) as its first section, so one key reaches
anything from anywhere. Files move with `j k` (and arrows), `l`/`h`
expand and collapse. Copying uses OSC 52; inside tmux with `set-clipboard off` (checked
once with `tmux show -gv set-clipboard`) or on a terminal that cannot confirm it, the toast
says "copied, if your terminal allows clipboard access" instead of a plain "copied".

Decisions where reviewers disagreed (recorded, not asked):
- **Launch focus is the network with the viewer pill selected**, as the brief's flow says
  (move to a node and Enter, or press its letter), so `⏎` from launch predictably opens
  this device. The approval that needs you is `Tab ⏎` (its tile ranks first and is
  pre-selected, muted) or its address `t a`, then the agent's `y`: three keys either way.
  Attention verbs answering approvals from the hub were left out: hub letters are
  devices (`n` is nas).
- **`⏎ open` on a device, `⏎ attach` on a session.** The hub footer names what Enter does
  to the focused zone; every screen that enters a session says `attach`.
- **`⌫` is stop** behind an inline confirm, so a reflexive "back" costs nothing.
- In the device layer `j` and `k` are motion, never slots (six slots `a s d g h l`).
- `Ctrl+\` replaces SIGQUIT inside attached shells (spec 8.1); on ISO layouts it may need
  AltGr, which is why the tmux status chip exists. It collides with nvim's terminal-mode
  escape (`Ctrl-\ Ctrl-n`) and emacs `toggle-input-method`: a second `Ctrl+\` within
  400 ms sends one `Ctrl+\` through to the pane, so those still work (two presses). `` ` `` is a dead key on some ISO
  layouts, so a double `Esc` (two presses within 400 ms) also jumps to the hub from any
  depth, and `cx` is always tappable.
- **Chips at S**: below 100 columns the map keeps names before chips (only closed links
  get one); the reviewers who asked for one chip form everywhere and those who asked for
  fuller names at 80 columns could not both be served at that width. The lens row and
  the device view carry the rtt there.
- **Hub-level answers** (`y`/`n` on the focused needs-you tile) stay out: `y` and `n`
  can be device letters (`n` is nas). The approval is `Tab ⏎ y` or `t a y`.

### Keystrokes for the top tasks (from the hub)

| Task | Keys | Count | Path |
|---|---|---|---|
| Open a session on another device | `i` `a` | 2 | tap tile (1) |
| Answer the approval that needs you | `Tab` `⏎` `y` or `t` `a` `y` | 3 | tap the strip, then Yes (2) |
| Re-attach after detaching | `⏎` (tile still selected) or `-` (from anywhere) | 1 | tap the tile or `- last` |
| Start a new claude session on innovation | `i` `c` | 2 | tap `i c claude` on the ghost tile (1) |
| Stop a session | `Tab` `⌫` `y` on the hub, `i` `⌫` `y` (selected), `i` `⌫` `s` `y` (another slot) | 3-4 | tap `⌫ stop`, tap `y` |
| Send a file to creation | `i` `f` `j…` `s` `c` | 4 + moves | `s send to`, tap the node |
| Edit innovation → tranquility to off | `i` `e` `t` `o` (outbound lane first) | 4 | tap the link chip, tap `o off` (2) |
| Edit tranquility ⇄ innovation from the hub | `e` `i` then `k p o` / `K P O` | 3 | tap the chip (1) |
| Send a join code | `+` `y` | 2 | tap `+ invite`, tap the code |
| Add a Cloudflare key | `K` `a` paste `⏎` | 2 + paste | tap `⏎ connect` |
| Browse and preview a file | `i` `f` `/` `tr` `⏎` | 5 | tap node, `f files`, tap file |
| Back to the hub from a file preview | `` ` `` (or a double `Esc`) | 1 (2) | tap `cx` |

### Ergonomics table

| Action | Frequency | Keys | Count | Key positions / fingers | Rationale |
|---|---|---|---|---|---|
| Go to a device | high | its letter | 1 | wherever the letter is; letters are the device's initial | mnemonic beats position; printed on every node, row and chip |
| Attach a session | high | slot `a s d g h l` | 1 (2 from hub) | home row, strong fingers | most frequent action, best keys; one address everywhere; `j k` left to motion |
| Open / re-attach selection | high | `⏎` | 1 | right pinky, large key | universal |
| Detach | high | `Ctrl+\` | 1 chord | left pinky + right pinky | the only key cx steals inside sessions; a double press passes it through; also a tap chip |
| Re-attach last | high | `-` | 1 | right pinky, top row, unshifted | the round trip with detach; works from every screen outside a pane |
| Answer what needs you | high | `Tab ⏎` | 2 | left pinky, right pinky | the tile ranks first; no letter collides with devices |
| Peek | med | `␣` | 1 | thumb | cheapest key for a look without commitment |
| Next peek | med | `Tab` | 1 | left pinky | triage without leaving peek |
| New claude / codex / shell | med | `c` / `x` / `n` | 1 (2 from hub) | left middle/ring, right index | mnemonic; `c x` adjacent |
| Switch zone | med | `Tab` | 1 | left pinky | two zones on the hub, so one press toggles |
| Find anything | med | `/` | 1 | right pinky | one meaning everywhere: the palette, scoped first to what you see |
| Files of a device | med | `f` | 1 | left index, home row | mnemonic, home row |
| Back / hub | med | `Esc` / `` ` `` | 1 | left pinky | one level vs. all the way; crumbs are tappable |
| Stop a session | low | `⌫` (slot) then `y` | 2-3 | right pinky reach, home-row slot, right index | destructive, so a far key plus confirm; a slot retargets without arrows |
| Send a file | low | `s` then device letter | 2 | left ring (home row), then the letter | the network is the picker, so the target is the same letter as everywhere |
| Edit a link | low | `e` then peer letter | 2 | left middle | mnemonic "edges"; same grammar on hub and device |
| Policy | low | `k p o` / `K P O`, `u` | 1 | right hand home/top row; Shift for the way back | applied at once with undo; no arrow to pick a lane |
| Copy | low | `y` / `Y` | 1 | right index | vim "yank"; never collides with `c` claude |
| Invite | low | `+` | 1 chord | shift + right pinky | rare; also a footer chip |
| Keys and secrets | low | `K` | 1 chord | shift + right middle | rare; uppercase is free on the hub |
| Help | low | `?` | 1 chord | shift + right pinky | convention |

## 7. Degradation

- **256 colours**: identical layout; blends resolve to the nearest xterm index.
- **16 colours**: no tints. Tiles separate by a faint `│` gutter and zones by their rule;
  selection is the same left bar plus a bold accent name; the pill is a solid accent row
  shaped like a keycap; charts keep the heat ramp; screen inks keep their hue (section 4).
  Overlays clear the backdrop instead of dimming it, and the palette's selection is one
  reversed run across the row.
- **NO_COLOR**: attributes only. Keycaps are reversed bold letters; the pill is a reversed
  row; selection is the bar plus bold; line styles carry the policy; thumbnails keep
  dim/bold.
- **ASCII**: every glyph has a one-meaning substitute (section 4); lines are `- | +`; the QR
  is dropped (code and one-liner remain). The capture lint reports 0 non-ASCII cells.
- **Ambiguous-width terminals** (Blink with CJK fonts, `ambiwidth=double`): status and
  policy glyphs, eighth-block charts, `▍ ▄ ▖`, `•` masks, `°` and box drawing are all
  East-Asian-Ambiguous. A real build probes once at startup (print `●` and `─`, read the
  cursor with `CSI 6n`) and, when either is two columns wide, renders with the whole ASCII
  glyph set (`unicode = false`: charts `_ : = #`, masks `*`, `°` as `C`, bars `>`, lines
  `- | +`) while keeping colour. That is the existing ASCII tier, which the lint keeps at 0
  non-ASCII cells. The lab cannot emulate double-width ambiguous cells, so it is not
  captured; real tmux 3.4 / Termux / Blink captures remain an open integration item
  (the lab never runs a real terminal or tmux).
- **Optional enhancement**: kitty/sixel images could show a real downscaled screenshot in
  session tiles; the cell tail is the shipped fallback and is what the captures show.

Flow captures: `CX_LAB_FOCUS=dialog tests/lab.sh mission files` (send-to picker),
`CX_LAB_FOCUS=attached tests/lab.sh mission session` (attached pane with the detach
chip), `CX_LAB_FOCUS=sessions CX_LAB_SELECTED=1 tests/lab.sh mission hub` (where detach
lands: sessions zone, the tile still selected), `CX_LAB_SELECTED=3 tests/lab.sh mission
hub` (a selected outer node: lit route, peer arc with chip, lens).

Captures: `tests/lab.sh mission` (dark/light truecolor, dark-16, ascii) plus
`CX_LAB_VARIANTS=dark-256,dark-nocolor,light-16` and a fallback-font check with
`CX_LAB_FONT="DejaVu Sans Mono"` (status glyphs shrink, `✕` becomes `X`, still legible).

## 8. Integration notes

- Rendering never dispatches; every target is recorded as `LabTarget` with the action the
  real UI would run (`device:<id>`, `session:<dev>:<id>`, `edge:<id>`, `new:<dev>:<provider>`,
  `policy:<from>:<to>:<value>`, `palette:<scope>`).
- The schematic needs per-device cpu history and per-edge rtt history (already in the
  spec's additive data, section 14) and the session screen tail the preview already has.
- `Sel::Root` (device view) draws letters as plain text; `Sel::Targets` (send-to) keeps
  them as live keycaps. `map::route` gives the lens its hop path; `Cx::unclash` makes the
  map's tap targets disjoint after drawing.
- The transfer pulse is driven by a frame counter; a real build would redraw the
  schematic at a low rate (2-4 Hz) only while a transfer runs and the hub is visible.
- `map::draw(cx, buf, band, root, depth, sel)` serves the hub (viewer, 3 hops), the
  device view (that device, 1 hop) and the edge editor. The left half is the right half
  mirrored (`Plan::sx`), so there is one router and one node painter.
- The edge editor id names the opener first (`innovation-tranquility` from `i e t`); the
  real UI resolves either order to the stored edge.
- Fixture copy in `src/ui/lab/state.rs`: the Claude gateway key is "used by cx lend
  (shared claude access)" (was "claude lends", which read as a typo).
- Fixture fix in `src/ui/lab/state.rs`: the Claude composer's middle row was one cell
  wider than its border, which showed as a stray `│` in every concept's peek.
