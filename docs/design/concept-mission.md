# Concept: mission control

Code: `src/ui/lab/concepts/mission/` (`mod.rs` render context and shared helpers,
`orbit.rs` the network diagram and its box-drawing lines, `hub.rs` the start screen (vitals
strip, ticker, phone tree), `tiles.rs` device rows and session thumbnails, `views.rs`
every other screen). Render: `tests/lab.sh mission`.

The judges' winner, with their agreed grafts (2026-11-10): the hub is the network around
this device (it sits in the middle as `◉` and its bold name, its peers symmetric above and
below its row on either side, on an arc, devices reached through a relay fanning outward
from it; thin continuous box-drawing
lines of one weight with rounded corners, whose hue is the network and whose stroke is
the policy: a quiet network diagram; user feedback 2026-11-10 replaced the earlier
braille orbit, which read as beaded dots), under it
the selected device's vitals strip, one row of live session thumbnails (what needs you
first) and a one-line alert ticker that pairs each problem with the keys that fix it.
Hotkey letters are underlined inside node names; a session is full screen under a
one-row breadcrumb with a tappable `ctrl+\ detach` chip; phones get the indented device
tree (no mini map). Each screen is one grammar: **a device letter, then a verb or a
session slot**.

## 1. Principles

1. **The orbit is the hub.** You do not pick from a list of devices; you look at the
   network and press the letter underlined in the node's name. Every node, link, tile and
   chip on screen is also a click/tap target that maps to the same action as its key.
2. **Recognise, don't read.** A session tile shows the real last informative lines of its
   screen (agent status, the approval prompt, the failing test) in the screen's own
   colours, in every colour tier. Two Claude sessions look different because their
   screens do.
3. **Evidence on the surface, said once.** Every device shows how its status is known:
   a node is its status dot (a down one its one red `✕`, unknown `◌`, never green) and
   its name; figures are said once, for the selection, under the map: the vitals strip
   names the selected device with its route from here and the
   rtt of every hop (`t → c → b  0.6 + 2.4 ms`), then the keys that act on it from the
   hub as two visible steps (`b then  a attach  c claude  f files`). Phones show the
   same evidence in the device tree.
4. **Tints, not boxes.** Panels are surfaces (background tints). Each zone opens with a
   plain small-caps label followed by a faint rule (`▍NETWORK ──────`; no letter
   spacing, so it copies and reads aloud as a word). Boxes are used only for the dashed
   "new session" ghost tile. Without tints (16 colours, NO_COLOR) tiles are separated
   by a faint `│` gutter; ASCII leaves the two-column gutter blank (`|` is the focus
   bar's continuation and means nothing else).
5. **Focus is a colour bar, in every tier.** The focused zone's label gets an accent bar
   `▍` (ASCII `> `); its selected item gets a left bar (`▍`/`▌`; ASCII `>` on its first
   row and `|` below; an unfocused zone's selection is `:` on every row in ASCII, so only
   the focused zone ever shows `>`) and a bold accent name, plus `accent.bg` where tints exist. 16
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
| Hub | `▍NETWORK` (accent while focused) with one quiet legend line on its rule (the meshes named in their hue with members up and `lan` in teal; the line grammar is in help) (the rule only when at least 12 cells remain); the orbit (section 3) under it with two blank rows either side (from 30 rows), and every row it leaves goes to the thumbnails (a second row of tiles with the rest of the sessions when each row still keeps ten; two rows share the tiles evenly, five sessions as 3+2 with the last row's tiles widened to fill it, and no ghost: the vitals strip offers `new`; with one row the ghost fills a free cell, its content centred); the map is centred in the rows above the vitals strip, counting the mesh wash's row above it; the selected device's vitals strip (no left bar: column 0 is the focus bar only; M+: two rows, name with route and per-hop rtt `t → c → b  0.6 + 2.4 ms`, os and account; cpu sparkline and %, mem gauge, temperature, uptime, meshes; its keys on the right as two steps, the device's keycap in its hue, a muted `then`, then neutral verb keycaps (`b then  a attach  c claude  f files`; `c claude` only when the session grid shows no `new on` tile, which already offers it), so the `c` after a letter never reads as the creation device; its text on the column every zone label starts on; S: one row); SESSIONS `needs you first`: one row of live thumbnails ranked by attention (the ghost "new on <selected>" tile fills a free cell; the full grid is in each device; `+2 more ›` opens the palette, shown only when tiles are hidden); the ticker (`▲ squirtle-jetson unreachable via ssh-overlay 3h  s r retry`, a tappable `+N` chip when more wait; the first-ranked session is the first tile right above it, so its `◆ … needs you  t a attach` item is not repeated there; addresses as on the vitals strip); the footer follows the focused zone. Phone (< 70 columns): NETWORK with the legend, the device tree (section 3; a keycap mini map only crowded at that size), up to two ticker rows (tinted, the whole row is the tap target) and a wrapping footer. |
| Device (`letter`) | Header with identity, evidence, mesh squares, cpu chart and gauges (four rows, one blank row before SESSIONS); session thumbnails keyed `a s d g` across the full width (three columns, two on S), as tall as their screens' text (up to the rows the LINKS block needs; a taller tile would only add blank rows); the ghost is always the one-row `new  c claude  x codex  n shell` under the tiles (a device with no session keeps the ghost tile), so no half-empty card floats beside a session; under them LINKS: the same star from this device with plain names (no letter stands out: in a device, letters are slots and verbs), and nothing else: each link's figures are one `e` away in the edge editor, so the star is not repeated as a table; the header's cpu chart stops at 72 columns and the metrics block hugs the right edge. LINKS follows right under the `new` row: the label, a blank row, then the star at the hub's spacing centred in every row left (both ways), a blank row above the footer; three links sit as one on the left and two right, symmetric about the root's row (no cross with an empty arm). Phones give the rows the entries, ghost row and links leave to the selected session as a thumbnail (up to its screen tail), so a prompt is recognisable. Phones (touch, under 70 columns) draw no star: a blank row after the `new` buttons when it costs no link, then one plain row per link on fixed columns sized to the longest name (name, policy, rtt or `last 41ms` when closed, since the policy column already says `off`; the loss goes first when rows are short). `⌫` asks inline on the selected tile's last row (`stop refactor transport?  ⌫ again to stop, any key keeps it`, warn-tinted; footer `⌫ stop it  esc keep`; capture `CX_LAB_FOCUS=stop tests/lab.sh mission device:innovation`): a second `⌫` stops, every other key keeps it. Footer `⏎ a attach` (Enter and the slot keys on one chip), `r retry` when the device is down or late, then `f files  e link  ⌫ stop  esc back  ` hub  - last  / find` (peek from 100 columns); phones lead with `ret attach  esc back`, `bksp stop` last. |
| Session (`⏎`, a slot key) | Full screen: the pane owns every row but the top one, a breadcrumb `cx › innovation › i a refactor transport  ● working` (the session's address, device keycap then slot keycap, before its title exactly as on its tile) whose segments are taps (hub, device) and whose right end is the `⏏ ctrl+\ detach` chip (ASCII `ctrl+\ detach`); the pane reflows to the screen's width (words wrap at spaces, an agent's drawn composer box is redrawn at that width with its right edge, before the ASCII substitution so ASCII keeps `+--+` at the pane width with blank sides; continuation rows hang at the line's content column, after its `⎿` / `+` / `●` marker, keeping the first row's tint), as the agent would draw it in a pane that size; at 120+ columns one muted live fact sits before it (`up 1h · 1 client`). On phones (touch, under 70 columns) the chip is ` ctrl+\ ` without `⏏` (phone fonts draw it as a wide emoji), so `cx › i a title ●` keeps its room (phones drop the device name). |
| Session peek (`␣`, `CX_LAB_FOCUS=peek`) | A meta line (address, state, provider, folder, uptime, clients, activity trend), the screen 1:1 on a deeper surface, and at M+ the OTHERS filmstrip. Hub letters work in peek (`t a` attaches t's slot a); `Tab` steps through the other peeks; `⏎` attaches, `⌫` stops, Esc back, `` ` `` hub. |
| Files (`letter f`) | Tree with a size micro-bar and age, git letters; under it, when rows are left, the selected entry's path, size, age and git state and its send targets as device letters (five rows: a SELECTED zone, one fact per row, and with room for them its send targets one per row with their route's rtt and relay, `b blastoise-odroid  via creation 17ms`; two to four rows: the path, then size, age and `s send to t c b` on one row; only devices a transfer can reach over links open in that direction are listed, those reached through a relay in their hue but not bold; tree guides are blank indentation in ASCII, since `|` and `:` are the selection bars; untracked is `U`; a tree cut by the height ends in a muted `+N more ↓`, and phones keep the spacer row above their wrapped footer). Inline syntax preview at ≥100 cols, its surface as tall as the file, where a line cut at the edge ends in an accent `›`. The footer teaches `j k move  l h fold  s send to` (80 columns: `⏎ open  j k move  l h fold  s send  spc preview  / find  esc back`); phones `ret open  esc back  s send  spc preview  / find  ` hub` (wrapped). `s` (send to) dims the files and raises a bottom sheet with the orbit seen from this device: press or tap the destination's letter (capture: `CX_LAB_FOCUS=dialog tests/lab.sh mission files`). The transfer then shows on the hub: its link heavy and lit, the rate on its chip, name and percentage in the top bar. |
| Palette (`/`) | Jump to anything: devices, sessions and actions (new claude / codex / shell, files, link), each with the keys that reach it directly, drawn as keycaps (the row's device letter in its hue), so the palette teaches the shortcuts by sight; action rows have no bullet, the key column is their mark. Rows cut by the height cap end in a muted `+N more ↓`, never silently. The selected row is the bar, a bold accent name and `accent.bg` where tints exist. The backdrop is dimmed to one level for every ink (names, strokes, tile text, bold or screen-authored colours). While it is open only the sheet is live: the backdrop's targets are dropped and a tap beside the sheet closes it. `/` is the one search everywhere; inside files or a preview it opens pre-filtered to that scope as the first section. Sized to its results, at most 75 % of the height (60 % would cut results with backdrop rows free), a breathing row between sections when every row still fits, a breathing row, then a row of `⏎ go  ↑↓ move  esc close` chips (`up/dn` on phones and in ASCII); on phones a full sheet on a cleared backdrop with those chips on the screen's last row like every phone footer (the sheet ends a row above them, so they never sit in its border), descriptions cut with an ellipsis before the key column. |
| Keys / secrets | Masked secret cards keyed `a s d g` (a slot key opens that card's editor, so adding the missing Tailscale key is `K g`, paste, `⏎`); missing ones get a solid accent button with a background-coloured label that names the key acting on it: `⏎ add` on the selected card, its slot key on the others (`g add`; in 16 colours a cyan cell with black text like the root capsule, never the gray keycap). Under the cards `h new secret` (the next free slot) adds any other secret by name; the footer says `h new`. Cards run the full content width like every zone. Names are cut against the room really left (the button only on its row). The status line under the cards is cut at a `·` boundary, never mid-word. The zone label and cards sit on the same column as every other screen's zones, with one breathing row under the label so its bar and the selected card's bar stay two marks. Crumb `cx › keys`. |
| Onboarding | The viewer alone on an empty orbit (`◉ tranquility` in bold, as the hub's root), the wrapped one-sentence explanation, a `⏎ Connect Cloudflare` button, three plain numbered steps (`1.`, not keycaps: they are not keys), a breathing row, then `+` join code (the global invite and join key; `j` stays motion) and `esc` plain SSH (the footer says `esc plain ssh`: chip labels are never cut to a first word, a phone footer wraps instead). Each step's note sits in a side column only when it fits whole; below about 90 columns it wraps on muted rows under its step, and the button's note moves under the button. |
| Join | Half-block QR (dark on light, always: in 16 colours on a dark theme colour 15 and 0; on a light theme, where bright white is often a gray, the terminal's own background and black ink with the NO_COLOR glyphs; with the standard four-module quiet zone wherever it fits, two only when the alternative is no QR) and the text column centred as one group (with no QR, as in ASCII and on phones, the text column centres on its own, as wide as the one-liner, with a faint `QR on a wider screen` in Unicode); at M+ the code is drawn in three-row half-block letters (a 3x5 pixel font of square half-block pixels) on a five-row tint, below that as spaced mono text on a tinted surface (`K X 7   4 Q M`; not keycaps: its letters are not cx keys), the one-liner broken after a space or `/`, never inside the code, over up to three rows (readable to retype where clipboard escapes fail) and an expiry bar. Footer `y code  Y command  esc done  R new code` (shifted: a new code revokes one that may already be shared). |
| Edge editor | Crumb `cx › t ⇄ i link`. The network diagram, then the sheet, as one centred column (the sheet at most 110 wide): the map at its spacing scale (one step up from the hub's; its mesh wash at half strength), two blank rows, the sheet (inside the screen's two-column gutter); the block sits in the middle of the rows between the crumb and the footer. The edited link keeps the one weight, drawn in the accent (ASCII `#`), everything else faded to one muted grey: other lines at 14 %, the other nodes' dots and names muted, plain, no underline (here `a p o` are policy keys). Phones draw no map: the link alone between its two keycaps in the accent (`t ── ⇄ key ── i`). The sheet has one segmented control per direction, labelled by its direction (`t→i`): the opener's outbound lane first with `a p o` (key, password, off), the way back with `A P O`; `j k` move the lane bar. Each press applies at once, `u` undoes; one line says so and how many sessions cross the link (only the count where the sentence does not fit), then `rtt`, a muted history (at most 40 columns; one sparkline row below 100 columns, a three-row chart on M, five on L, each growing to twice that with rows the map leaves, a row of air kept either side) and its figures as quiet text on its baseline (`32 samples · min 10ms · mean 14ms · max 17ms · jitter 1.1ms`, cut at a `·`); on L the first session that crosses the link follows as a thumbnail with plain letters (what a change would cut). Every segment has one shape, keycap, a blank cell, the word, the selected one on its policy tint (phones say `pass`). Footer `j k lane   u undo   esc done` (phones drop `j k`). |
| Help | Crumb `cx › help` (capture the other contexts with `CX_LAB_FOCUS=device|edge|files tests/lab.sh mission help`). No drawn keyboards (user feedback, 2026-11-10): a calm table for the view it was opened from (`ON THE HUB`, `IN A DEVICE`, `ON A LINK`, `IN FILES`; ASCII `[ON THE HUB]`): intent word muted in a left column as wide as its longest word, keys as plain bold text (a space between keys), what they do. On the hub the grammar is said as two steps so a device letter and a verb never look like one key (`c` creation vs `c` claude): `device` shows real device names with their letter underlined in the accent exactly as on the map (three from 100 columns, two below; `…`, ASCII `N more`) and `its letter opens it`; under it, indented as the second step and with no blank row between, `then`: `a s d g attach its 1st / 2nd / 3rd / 4th session` (`attach session 1-4` below 60 columns), `c x n new claude / codex / shell there`, `f its files`; then `move  j k  tab  enter  -` and `act  spc  e (then a letter: edit that link)  bksp`. In a device: session `a s d g / enter / spc / bksp`, new `c x n`, device `f / e / r`, move `j k / esc / -`. On a link: `way out a p o`, `way back A P O`, `move j k / u / esc`. In files: `j k / l h / enter`, `spc / s (then a letter) / y`, `esc`. Every context ends with `global`, one key per row like every block (`/ find`, `` ` hub``, `+ invite`, `K secrets`, `ctrl+\ detach`). The hub, device and link help add a quiet `marks` grid in the map's own glyphs where the screen holds it (`── key  ╌╌ password  ◀─ one way  ─╴ closed  ● online  ○ late  ✕ down`, on a link also `── this link` in the accent; ASCII `-- .. <- -`, `## this link`, `o !` and `[ ] selected`), every cell with the rows' key column. Keys are words: `enter` at every size, `bksp`. A blank row under the label and between blocks where the screen holds them; the block, at most 78 columns (68 below 100), sits a third of the way down. |

## 3. The network diagram

One renderer (`orbit::draw`) serves the hub (root = this device, three hops), the device
view's LINKS (root = that device, one hop, letters as plain text), the send-to picker
(letters live) and the edge editor (the edited link at full hue, everything else faded,
letters plain because `a p o` are policy keys there). It aims for a quiet network
diagram: continuous thin lines of one weight, crisp labels, air between everything,
nothing on the lines but the lines.

- **A radial orbit around the root** (elegance round 4, 2026-11-11). A spanning tree
  from the root over routed links (a device reached by jumping through a relay hangs
  from that relay, its line the jump link's), drawn as a **radial tree**: each subtree
  gets a share of the circle by its device count, its device sits in the middle of
  its share on ring one, the devices reached through it on ring two (1.75 times out),
  in the middle of their own shares. The heaviest relay sits **straight above the
  root** (its devices on ring two above-left and above-right); without a relay an odd
  count puts the first device there and an even count meets on the axis, so peers come
  in mirrored pairs. The rest go by weight to the lighter side, one-way links paired
  first, so the one left for the bottom of the axis reads both ways. Typical:
  `creation` above with `blastoise-odroid` and `squirtle-jetson` over its wings, `nas`
  and `laptop-guest` mirrored below the root's row, `innovation` straight below.
  A line leaves its parent along the parent's row and **turns once** with a rounded
  corner straight into the device's dot (a device level with its parent takes a straight
  run; a device on the axis a straight vertical); a device one row from its parent
  moves to two rows (or, on a small orbit, onto its row), so no line kinks. Positions
  are rounded the same way either side of the axis, so mirrored devices match. The
  ellipse is measured from the root label's ends. **Sized to its area**: ring radii
  grow with the band (rows up to 4 on S, 5 on M, 7 on L; 4/6/9 in the edge editor;
  a device's own star 4, 6 on L) and columns from twice to four times the rows, up to
  about 70 % of a wide band (all but four-column gutters on a narrow one); the drawing
  is centred in the band. Labels keep a cell either side for a selection's brackets,
  lines cross no label and links from different parents share no cell; where no orbit
  fits, or the network has more than eight devices, the column layout below takes over:
  peers in a column either side of the root, symmetric about its row on an arc, a
  relay's devices fanning outward, labels that would leave the band becoming
  `+N more` (opens the palette).
- **Mesh groups**: a mesh the root is not in (ssh-overlay: creation, blastoise,
  squirtle) is one quiet wash of its hue (7 % dark, 6 % light) on its members' own rows,
  two columns past the outer label and symmetric about the root's axis (the relay sits
  on it), only where no other label falls inside. The root's own meshes are its lines'
  hues. 16 colours and ASCII draw no wash (the legend and line hues carry it).
- **Lines** are box-drawing characters: a stub leaves the root (or relay) on its row, a
  trunk runs down the column gap with rounded corners `╭ ╮ ╰ ╯` and junctions
  `├ ┤ ┬ ┴ ┼`, and a branch enters each label on its row. Every line end keeps one clear
  cell from a label, one more where a selection's brackets may sit (a vertical line meets
  its labels directly). ASCII: `-` and `|`, corners `,` above and `'` below, a junction
  its trunk's `|` or `-` (`+` is the invite key). One weight everywhere; unlit lines at 55 %, the selection's route
  back to the root (and its own children) at 95 %. **Hue is network**: the mesh hue
  (violet, blue), teal for lan, neutral for direct ssh; a cell shared by links of
  different networks is neutral. **Stroke is policy, used sparingly**: solid where a key
  is allowed, dashed (`╌ ╎`, long dashes that stay visible; ASCII `.` and `:`) where
  only a password is, a faint neutral line where nothing crosses (30 %, even when lit;
  ASCII plain `-`, its marks say why); a link closed both ways **stops short**: its two
  cells nearest the device are blank (one where it turns into the dot, so its corner stays), no glyph on the stroke (a down device's `✕` says
  the rest). A one-way link
  has one solid arrowhead (`◀ ▶`, which meets its stroke where a font arrow floats;
  ASCII `<` `>`) at the end access reaches: the line's last cell beside the device (a
  line that turns: on its run along the parent's row, a cell before the corner), or
  beside the parent. Nothing else rides on a line. A cell
  shared by several links takes a lit link's ink. The edge editor draws its link in the
  accent and every other line in one muted grey (16 colours cannot fade a hue).
- **Fit before cutting**: rows come closer, the arc flattens and the column gap shrinks
  (down to 4) before any name is shortened; a dense network (more than eight devices)
  never takes rows more than two apart. On the hub the map keeps two blank rows either
  side (from 30 rows) and the session tiles take every other row, so no blank band opens
  around it.
- **No figures on the map.** rtt, transfer rates and session slots are said once, for the
  selection, in the vitals strip (and in the device and edge views). Each link keeps a
  3x1 tap target on its branch (desktop; phones tap the tree rows).
- **Nodes** have one treatment everywhere: the status dot on the side the line arrives
  (`● innovation`, left-hand `laptop-guest ○`; the one dot set `● ○ ✕ ◌`, a late device hollow in warn), then the name with its hotkey letter
  underlined in the accent (a keycap in front when the letter is not in the name). No
  background anywhere. A down device is its one red `✕` and a muted name.
  The viewer seen in another device's star is a plain peer (`●`); `◉` marks the map's
  root only. Richness is chosen per frame (name 16/12/9, keycap only).
- **The root** is `◉` and its bold name, no capsule (ASCII `@ tranquility`, selected
  `[@ tranquility]` on every map: brackets mean selection and nothing else, also on a
  keycap map, where the root is `◉` before its keycap; help's `marks` row says `[ ]
  selected` in ASCII). ASCII has no underline, so a live letter sits in parentheses:
  `(t)ranquility`, `la(p)top-guest` (map, vitals strip and help). The same mark is the viewer on onboarding.
  An online device has no ASCII mark (online is the normal case and `o` is a key there).
- **Selection**: a crisp label, not a button: the name in the accent (70 % without
  focus); in ASCII and NO_COLOR, `[ ]` one cell outside it. Its route back to the
  root is lit.
- **Phone tree** (the phone hub has no map): every device depth first, the root first,
  then each relay followed by the devices reached through it (`├─ c creation`,
  `│ ├─ b blastoise-odroid`; ASCII `|-`, `` `- `` and `|` guides, `|.` where a password is needed), the branch in the hue
  of the link to its parent, dashed (`├┄`) where only a password is allowed, faint where
  the link is down; the letter in its hue (a keycap on the selected row only), name,
  status, rtt or last sighting (no slot letters); with room
  a second row (cpu sparkline in one neutral muted tone, %, temperature; the figure turns warn from 85 %, nothing else takes a colour). Each row is the device's tap target.
  Rows the tree leaves go to a SESSIONS list (two-row session rows, what needs you
  first) above the ticker. The send-to picker keeps a keycap map on phones (its letters
  are the choice).
- **Lan in every tier**: the legend names lan in NO_COLOR and ASCII too, so every tier lists the same networks.
- **Light themes** deepen line hues like session inks (`tiles::adapt`), so every line
  keeps about 3:1 on white.

## 4. Palette tokens

Semantic tokens come from `caps.rs` (spec section 4.1); the concept adds blends, never
raw colours. `bg0` is only a blending reference: the terminal background stays default.

| Token | Use | Dark | Light | 256 | 16 | NO_COLOR / ASCII |
|---|---|---|---|---|---|---|
| `bg0` | blend reference | `#121519` | `#fafaf8` | n/a | n/a | n/a |
| screen | thumbnail and peek "glass" | `#0c0e11` | `#ffffff` | nearest | none | none |
| `fg` | names, titles | `#d5d9e0` | `#24292f` | default | default | default |
| `muted` | evidence, captions | `#8b93a1` | `#5c6470` | 246/241 | default + dim (brightblack is the background in Solarized Dark) | dim |
| `faint` | grid dots, ghost outline | `#4a515c` | `#c3c8d0` | 239/251 | brightblack | dim |
| `accent` | focus bar, labels, pill | `#6cc4d6` | `#0b7285` | 80/30 | cyan | bold, `>` |
| `accent.bg` | focused selection | `#1d3a42` | `#d3eef3` | 23/195 | none: the bar and a bold accent name | none: bar `>` / `|` and bold |
| `surface` | tiles | `#1b1f25` | `#f3f4f6` | 234/255 | none | none |
| heat ramp | cpu, temp, disk | ok → warn (60 %) → err | same; sparkline and chart fills below 60 % are a half tint toward `bg0`, so bars never outweigh names | nearest | green/yellow/red at 60/85 % | ASCII bar glyphs |
| mesh hues | mesh 1, mesh 2 | `#c49cf0`, `#7fd8e8` | `#7048c0`, `#0a8090` | 177, 116 | magenta, brightcyan | `#` |
| `id.0..7` | device keycaps | spec hues | spec hues | spec | rotated 8 | reversed bold letter |

Session screens are mostly authored for dark terminals. On a light theme, dark screen
backgrounds are stretched to full saturation and then mixed 16 % into white (a diff row
becomes about `#d9ffe4` / `#ffdcdc`, like GitHub light), and pale inks are deepened 42 %
toward black (`tiles::adapt`). In 16 colours, named screen colours stay themselves, rgb
inks map by hue (a nearest-distance match turned muted reds grey), and coloured
backgrounds become ink of their hue, so `+` rows stay green and `-` rows red without a
loud block (`tiles::degrade`).

Glyphs, with ASCII fallbacks, each substitute meaning one thing: a focused zone label
`▍NETWORK` / `[NETWORK]`; selection `▌` / `>` on the first row and `|` below in the focused zone, `|` on every row in an
unfocused one (`:` is a password line on the map) (and nothing else is `>`, `|` or `:` in ASCII: tile gutters, tree guides,
sheet and ghost-tile sides and an agent's own prompt bar are blank); status `◉ ● ○ ✕ ◌` / `@`, none, `o ! ?` (online is unmarked, the normal case; late a hollow `o`, since `~` starts every path; down `!`, since `x` is the codex key; the top bar says `6/7 up`);
session state `◆ ● ○ ·` / `^ ` none ` - .` (what needs you is an alert, `^`, as on the ticker; live work is unmarked like an online device, so `*` stays the agents' own bullet); alerts `▲` / `^`; a transfer `⇣` / `dl`; one-way arrowheads on the map `◀ ▶` / `<` `>`; policy `⇄ → ← ⊘` / `<> -> <- off`; line
styles `─ ╌` / `- .`, closed a line that stops short, the edge editor's edited link in the accent / `#` (nothing else is `#`); mesh squares `■` / none (the name says it); the separator `·` / `-`; a cut `…` /
`..` (also inside fixture text); crumbs `›` / `/` (the sessions overflow `+2 more ›` drops it in ASCII); a symlink `→` / `&`; an agent's own prompt bar inside a thumbnail `▌` / blank (so `|` and `:` stay cx's selection bars); folds `▾ ▸` / `v` and none (a collapsed folder is its trailing `/`); a line cut at
the preview edge `›` / `$`; keycaps are coloured cells in every tier
(`REVERSED|BOLD` without colour); link lines become `- | , '`; `⎿` is `L` and `✻` is `*`;
charts and sparklines become `_ =` (`.` and `-` are link strokes); sheet, palette and ghost frames are dashes with blank sides (`+` is the invite key); the minimap is omitted in ASCII
and NO_COLOR; the QR is drawn with `▀▄█` even in NO_COLOR and dropped in ASCII and where it cannot fit above the code (40x20: the code and one-liner carry it);
Fixture text is transliterated in ASCII (`asciify`) without rewriting what a screen says: a character with no ASCII stand-in of the same meaning (`↓`) becomes a neutral `?`, never a cx word; the lint stays at 0 warnings.
Inside thumbnails an emphasised lone letter (`a Always` in an agent prompt) takes the
muted caption ink, one step below every cx keycap and name, so cx keys stay unique.

## 5. Layout per size

Width classes as in the spec: XS < 60, S 60-99, M 100-159, L ≥ 160. Touch layouts are
everything under 70 columns (40x20 and 60x30 phones).

| | touch: 40x20, 60x30 | S 80x24 | M 120x40 | L 200x60 |
|---|---|---|---|---|
| Top bar | `cx`, crumb (earlier device crumbs as keycaps), `● 6/7 ▲1` | + transfer % and `?` | + transfer name and rate | same (the ticker carries the alert text) |
| Orbit | none (the device tree) | names (9-16) | two-row nodes (rtt, slots) | same, more air |
| Below it | device tree (one or two rows per device), up to two ticker rows | legend, one-row vitals | legend, two-row vitals | same |
| Sessions | (SESSIONS rows under the tree, needs-you in the ticker) | one row of 3 tiles, 5 tall | one row of 3, up to 9 tall | one row of 4, up to what the screens hold; two rows (the rest and the ghost) when the map leaves room for each to keep ten |
| Device view footer | `ret attach`, `esc back`, `f files`, `e link`, `` ` hub``, `bksp stop` (two rows, top first) | 7 chips | 7 chips | same |
| Hub footer | `ret open` / `ret attach` first, then `- last`, `/ find`, `+ invite`, `K secrets`, `? help` (two rows, top first) | by zone, up to 8 chips | by zone | same |

The orbit takes the rows the thumbnails do not need; the ticker sits right above the
footer (it is the last line you read before choosing a key). Session tiles share one width and
one 2-column gutter; rows are equal height. Thumbnails show the last non-blank lines of
the screen and drop shared indentation when narrower than the screen; the agent's own
reversed key hints (`y Yes`) are drawn in flat thumbnail ink (no reverse, underline or
bold), so they never look like cx keycaps. A cut never shows two ellipses. Under 20x6
the frame shows only `cx`. Phones (and ASCII) label keys with words where symbols may be
missing from the font: `ret`, `bksp`; `spc` everywhere (the `␣` symbol
renders as a bare underscore). A phone footer leads with its primary verb, then
`esc back`, and every chip has a word. When it wraps, rows fill in priority order from
the top (the spacer row above the footer takes the first chips) and a destructive verb
(`bksp stop`) is always last. Chips keep two columns between them at every width (a wrapped phone footer wraps one chip earlier rather than closing the gap).

## 6. Interaction model

Hub keys are non-letters plus `e`, `j` and `k`, so every other letter can be a device: a
real build never assigns `e` (link) and assigns `h` `j` `k` `l` last, only past 21
devices, so on the hub `j k` move through the sessions row and the phone tree as in every
list and a vim hand pressing `h` or `l` never opens a device (a device whose initial is
reserved takes the next letter of its name that is neither reserved nor a slot:
laptop-guest is `p`; the large fixture follows that order). Inside a device, letters are verbs and slots; the home row
holds the slots (`a s d g`; `f` stays files, `j k` stay motion and `h l` fold as in files
and every list, so a vim hand never attaches by accident). Sessions past the fourth are
reached by the arrows or `Tab` then `⏎`, or through `/`. Any list inside a screen uses the
same home-row slots (secrets cards are `a s d g`). Peek has no inline typing, so hub
letters work there unchanged. `K` (keys), `+` (invite) and `?` (help) are global outside text input and attached
panes: they are uppercase or symbols, so they never clash with slots, verbs or devices.

| Key | Hub | Device | Peek | Mouse / touch |
|---|---|---|---|---|
| device letter | open that device | — | as on the hub | tap node, row or chip |
| `a s d g` | — | attach that session | (after a letter) attach | tap thumbnail |
| `c` / `x` / `n` | — | new claude / codex / shell here, attached at once | — | tap ghost-tile option |
| `e` + letter | edit the link from this device to that one | edit the link to that peer | — | tap the link |
| `f` | — | files | — | tap `f files` |
| `s` (in files) | — | send the file: the orbit becomes the picker, a device letter sends | — | tap the node |
| `⌫` | stop the selected session tile (sessions zone; a second `⌫` confirms) | stop the selected session; a slot key retargets (`⌫ s ⌫`) | stop this one | tap `⌫ stop`, tap `⌫` |
| `⏎` | open selection (device or session) | attach selection | attach | tap selection |
| `␣` | peek selected session | peek | — | tap `␣ peek` |
| `Tab` | next zone (network, sessions) | next zone | next peek | tap a zone |
| `↑↓←→` | move spatially | move | — | — |
| `/` | find anything | find | — | tap `/ find` |
| `+` / `K` / `?` | invite / keys / help | same (global) | same | footer chips |
| `Esc` | — | back one level | back to the device | tap the crumb or `esc back` |
| `` ` `` | — | to the hub from any cx screen | to the hub | tap `cx` |
| `-` | re-attach the last detached session | same | same | tap `- last` |
| `r` | (after a letter: `s r`) retry the device the ticker names | reconnect / retry this device | — | tap the ticker's `s r retry` |
| `Ctrl+\` | — | detach from an attached session, at once | | the `⏏ ctrl+\ detach` chip at the breadcrumb's right end |
| `a p o` / `A P O`, `u`; `j k` | edge editor: outbound / inbound key, password, off, applied at once; undo; `j k` move between the lanes | | | tap segment |
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
- **`⌫` is stop** behind an inline confirm answered by a second `⌫`, never `y` (round 4):
  `Tab ⏎ y` approves the pre-selected needs-you tile, and `⏎` sits right under `⌫`, so a
  `y` confirm would let one slip stop the very session waiting for you. The default is
  no: every other key keeps the session.
- In the device layer `j k` are motion and `h l` fold keys, never slots (four slots `a s d g`;
  a fifth session is an arrow or `/` away, rarer than a vim hand pressing `l`).
- `Ctrl+\` replaces SIGQUIT inside attached shells (spec 8.1) and **detaches at once**
  (0 ms: no wait for a second press). On ISO layouts it may need AltGr, which is why the
  tap chip exists. Its collisions (nvim's `Ctrl-\ Ctrl-n`, emacs `toggle-input-method`)
  are accepted (user decision); a literal `Ctrl+\` is sent by the palette action "send
  Ctrl+\" (it re-attaches the last session and forwards one). Also accepted: in an
  outer tmux, vim-tmux-navigator binds `C-\` (`select-pane -l`) in the root table, so
  the key never reaches cx there. Integration: probe once at startup inside tmux
  (`tmux list-keys -T root 'C-\'`); when bound, the breadcrumb chip says `⏏ detach` (the
  tap still works) and a toast says so. Only then, and only if the user configures one,
  a fallback detach chord is active; there is no default, because every candidate takes
  a key from the pane (`Ctrl+]` is vim's tag jump and a Codex binding, and needs AltGr
  on DE/FR layouts just like `\`). The tap chip is the answer on ISO layouts and under
  vim-tmux-navigator; the help screen shows the configured chord when there is one.
  An outer tmux whose prefix is `` ` `` swallows the hub key: `Esc Esc` is the route there.
- `` ` `` is a dead key on some ISO layouts, so a double `Esc` (two presses within 400 ms)
  also jumps to the hub from any cx screen **outside an attached pane** (Claude Code and
  Codex own `Esc Esc` there: rewind and backtrack); the first `Esc` acts immediately (back
  one level), the second continues to the hub. `cx` is always tappable.
- **New sessions attach at once** (`i c` lands in the new claude session), consistent with
  principle 6, so every new-session task costs two keys.
- In the edge editor `j k` stay motion (they move the lane bar); the key policy is `a`
  (`A` for the way back), so a vim hand pressing `k` never grants managed-key access.
- **No chips on links** (user feedback, 2026-11-10: the graph should be calmer): the rtt
  is quiet text on the node's second row (M+), the vitals strip names the route's rtt
  per hop, and the device view and edge editor carry the full figures.
- **Hub-level answers** (`y`/`n` on the focused needs-you tile) stay out: `y` and `n`
  can be device letters (`n` is nas). The approval is `t a y`, the keys its tile prints (the phone ticker prints them too).
- **Numbered 1-9 attach from the hub** (one judge's graft) was left out: the ticker
  already prints the two-key address of what needs you, and digits would be a second
  address for the same session (one address per session, principle 7).
- **Session view**: the attached pane is the session view (`⏎` attaches directly); the
  breadcrumb sits on top so the detach chip is where the eye and a thumb look first.

### Keystrokes for the top tasks (from the hub)

| Task | Keys | Count | Path |
|---|---|---|---|
| Open a session on another device | `i` `a` | 2 | tap tile (1) |
| Answer the approval that needs you | `t` `a` `y` (its tile, first in the row, prints `t a`) or `Tab` `⏎` `y` | 3 | tap the ticker row, then Yes (2) |
| Retry an unreachable device | `s` `r` (printed on the ticker) | 2 | tap `s r retry` (1) |
| Re-attach after detaching | `⏎` (tile still selected) or `-` (from anywhere) | 1 | tap the tile or `- last` |
| Start a new claude session on innovation | `i` `c` | 2 | tap `i c claude` on the ghost tile (1) |
| Stop a session | `Tab` `⌫` `⌫` on the hub, `i` `⌫` `⌫` (selected), `i` `⌫` `s` `⌫` (another slot) | 3-4 | tap `⌫ stop`, tap `⌫` |
| Send a file to creation | `i` `f` `j…` `s` `c` | 4 + moves | `s send to`, tap the node |
| Edit innovation → tranquility to off | `e` `i` `O` (the inbound lane, from the hub) or `i` `e` `t` `o` (the outbound lane first) | 3 | tap the link, tap `O off` (2) |
| Make tranquility → innovation key-only | `e` `i` `a` | 3 | tap the link, tap `a key` (2) |
| Edit tranquility ⇄ innovation from the hub | `e` `i` then `a p o` / `A P O` | 3 | tap the link (1) |
| Send a join code | `+` `y` | 2 (3 presses with Shift) | tap `+ invite`, tap the code |
| Add the missing Tailscale key | `K` `g` paste `⏎` | 3 + paste (4 presses with Shift) | tap `⏎ add`, paste, `⏎` |
| Add any other secret | `K` `h`, name, paste, `⏎` | 2 + typing (3 presses with Shift) | tap `h new secret` |
| Browse and preview a file | `i` `f` `j…` (inline preview at ≥ 100 columns; `spc` below) or by name `i` `f` `/` `t` `r` `⏎` | 2 + moves, or 6 | tap node, `f files`, tap file |
| Back to the hub from a file preview | `` ` `` (or a double `Esc`) | 1 (2) | tap `cx` |

### Ergonomics table

| Action | Frequency | Keys | Count | Key positions / fingers | Rationale |
|---|---|---|---|---|---|
| Go to a device | high | its letter | 1 | wherever the letter is; letters are the device's initial | mnemonic beats position; printed on every node and row |
| Attach a session | high | slot `a s d g` | 1 (2 from hub) | home row, strong fingers | most frequent action, best keys; one address everywhere; `j k` left to motion |
| Open / re-attach selection | high | `⏎` | 1 | right pinky, large key | universal |
| Detach | high | `Ctrl+\` | 1 chord | left pinky + right pinky | the only key cx steals inside sessions; instant; a literal one via the palette; also a tap chip |
| Re-attach last | high | `-` | 1 | right pinky, top row, unshifted | the round trip with detach; works from every screen outside a pane |
| Answer what needs you | high | the `t a` on its tile (phones: the ticker) | 2 | its letter, then the home-row slot | the fix is printed next to the problem; `Tab ⏎` also works (the tile ranks first) |
| Retry a device | low | letter then `r` | 2 | its letter, left index top row | the ticker prints it; `r` is free in the device layer |
| Peek | med | `␣` | 1 | thumb | cheapest key for a look without commitment |
| Next peek | med | `Tab` | 1 | left pinky | triage without leaving peek |
| New claude / codex / shell | med | `c` / `x` / `n` | 1 (2 from hub) | left middle/ring, right index | mnemonic; `c x` adjacent |
| Switch zone | med | `Tab` | 1 | left pinky | two zones on the hub, so one press toggles |
| Find anything | med | `/` | 1 | right pinky | one meaning everywhere: the palette, scoped first to what you see |
| Files of a device | med | `f` | 1 | left index, home row | mnemonic, home row |
| Back / hub | med | `Esc` / `` ` `` | 1 | left pinky | one level vs. all the way; crumbs are tappable |
| Stop a session | low | `⌫` (slot) then `⌫` | 2-3 | right pinky reach twice, home-row slot | destructive, so a far key pressed twice; never `y`, which answers approvals; a slot retargets without arrows |
| Send a file | low | `s` then device letter | 2 | left ring (home row), then the letter | the network is the picker, so the target is the same letter as everywhere |
| Edit a link | low | `e` then peer letter | 2 | left middle | mnemonic "edges"; same grammar on hub and device |
| Policy | low | `a p o` / `A P O`, `u` | 1 | left pinky home, right pinky / ring top row; Shift for the way back | applied at once with undo; no arrow to pick a lane; `j k` stay motion |
| Copy | low | `y` / `Y` | 1 | right index | vim "yank"; never collides with `c` claude |
| Invite | low | `+` | 1 chord | shift + right pinky | rare; also a footer chip |
| Keys and secrets | low | `K` | 1 chord | shift + right middle | rare; global (uppercase never clashes) |

## 7. Degradation

- **256 colours** (also tmux with `tmux-256color` and no RGB): identical layout; blends
  resolve to the nearest xterm index. Keycaps sit a step above the sheet surface, so they
  stay cells after quantisation on panels too (captured: `CX_LAB_VARIANTS=dark-256,light-256`).
- **16 colours**: no tints; keycaps are a brightblack cell with white bold text on dark backgrounds and a white-gray (`Gray`) cell with black text on light ones, where white on brightblack is too faint (device keycaps keep their hue cell). Tiles separate by a faint `│` gutter and zones by their rule;
  selection is the same left bar plus a bold accent name; the pill is a solid accent row
  shaped like a keycap; charts keep the heat ramp; screen inks keep their hue (section 4).
  Overlays clear the backdrop instead of dimming it; every selection (files, palette,
  secret cards) is the bar plus a bold accent name, never a reversed run.
- **NO_COLOR**: attributes only. Keycaps are reversed bold letters; the pill is a reversed
  row; selection is the bar plus bold; line styles carry the policy; thumbnails keep
  dim/bold.
- **ASCII**: every glyph has a one-meaning substitute (section 4); lines are `- | , '`; the QR
  is dropped (code and one-liner remain). The capture lint reports 0 non-ASCII cells.
- **Ambiguous-width terminals** (Blink with CJK fonts, `ambiwidth=double`): status and
  policy glyphs, eighth-block charts, `▍ ▄ ▖`, `•` masks, `°` and box drawing are all
  East-Asian-Ambiguous. A real build probes once at startup (print `●` and `─`, read the
  cursor with `CSI 6n`) and, when either is two columns wide, renders with the whole ASCII
  glyph set (`unicode = false`: charts `_ : = #`, masks `*`, `°` as `C`, bars `>`, lines
  `- | , '`) while keeping colour. That is the existing ASCII tier, which the lint keeps at 0
  non-ASCII cells. The lab cannot emulate double-width ambiguous cells, so it is not
  captured; real tmux 3.4 / Termux / Blink captures remain an open integration item
  (the lab never runs a real terminal or tmux).
- **Optional enhancement**: kitty/sixel images could show a real downscaled screenshot in
  session tiles; the cell tail is the shipped fallback and is what the captures show.

Flow captures: `CX_LAB_FOCUS=dialog tests/lab.sh mission files` (send-to picker),
`tests/lab.sh mission session` (the full-screen session under its breadcrumb and
detach chip; `CX_LAB_FOCUS=peek` for the peek), `CX_LAB_FOCUS=sessions CX_LAB_SELECTED=1 tests/lab.sh mission hub` (where detach
lands: sessions zone, the tile still selected), `CX_LAB_SELECTED=3 tests/lab.sh mission
hub` (a selected outer node: lit route, its peer links, vitals strip).

Captures: `tests/lab.sh mission` (dark/light truecolor, dark-16, light-16 for yellow
and cyan on white, ascii), the 16-colour
proof `CX_LAB_PALETTE=solarized CX_LAB_VARIANTS=dark-16 tests/lab.sh mission hub,help`
(brightblack is the background there, so only faint strokes may use it), plus
`CX_LAB_VARIANTS=dark-256,dark-nocolor,light-16` and a fallback-font check with
`CX_LAB_FONT="DejaVu Sans Mono"` (status glyphs shrink, `✕` becomes `X`, still legible).

## 8. Integration notes

- Rendering never dispatches; every target is recorded as `LabTarget` with the action the
  real UI would run (`device:<id>`, `session:<dev>:<id>`, `edge:<id>`, `new:<dev>:<provider>`,
  `policy:<from>:<to>:<value>`, `palette:<scope>`).
- The orbit needs per-device cpu history and per-edge rtt history (already in the
  spec's additive data, section 14) and the session screen tail the preview already has.
- `Sel::Root` (device view) draws letters as plain text; `Sel::Targets` (send-to) keeps
  them as live keycaps. `orbit::route` gives the vitals strip its hop path; `Cx::unclash` makes
  the map's tap targets disjoint after drawing.
- The transfer pulse is driven by a frame counter; a real build would redraw the
  orbit at a low rate (2-4 Hz) only while a transfer runs and the hub is visible.
- `orbit::draw(cx, buf, band, root, depth, sel)` serves the hub (viewer, 3 hops), the
  send-to picker and the edge editor; `orbit::draw_with(.., keys_only)` lets the device
  view keep names on a phone. `orbit::drawn_h(cx, band, sel)` takes the selection
  (2026-11-11): the edge editor's map may spread rows further apart than the hub's. `orbit::tree_rows` gives the phone tree the same spanning
  tree, so the orbit and the tree never disagree about who is reached through whom.
- The ticker's retry action (`retry:<device>`) and the device-layer `r` verb are new
  integration items; notices name their device by a name prefix in the lab fixture, a
  real build carries the device id on the notice.
- The edge editor id names the opener first (`innovation-tranquility` from `i e t`); the
  real UI resolves either order to the stored edge.
- Fixture copy in `src/ui/lab/state.rs`: the Claude gateway key is "used by cx lend
  (shared claude access)" (was "claude lends", which read as a typo).
- Fixture fix in `src/ui/lab/state.rs`: the Claude composer's middle row was one cell
  wider than its border, which showed as a stray `│` in every concept's peek.
