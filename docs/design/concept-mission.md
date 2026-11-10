# Concept: mission control

Code: `src/ui/lab/concepts/mission/` (`mod.rs` render context and shared helpers,
`orbit.rs` the network diagram and its box-drawing lines, `hub.rs` the start screen (vitals
strip, ticker, phone tree), `tiles.rs` device rows and session thumbnails, `views.rs`
every other screen). Render: `tests/lab.sh mission`.

The judges' winner, with their agreed grafts (2026-11-10): the hub is the network around
this device (it sits in the middle in its capsule, its peers in a column either side,
devices reached through a relay in the next column out; thin continuous box-drawing
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
   hub (`b a attach  b c new claude  b f files  e b link`). Phones show the
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
| Hub | `▍NETWORK` (accent while focused) with one quiet legend line on its rule (the meshes named in their hue with members up and `lan` in teal; the line grammar is in help) (the rule only when at least 12 cells remain); the orbit (section 3) under it, as tall as the rows left, its drawing one row under the band's top, and the rows it leaves at its foot go to the thumbnails (up to what their screens hold; a second row of tiles with the rest of the sessions and the ghost when each row still keeps ten); the selected device's vitals strip (no left bar: column 0 is the focus bar only; M+: two rows, name with route and per-hop rtt `t → c → b  0.6 + 2.4 ms`, os and account; cpu sparkline and %, mem gauge, temperature, uptime, meshes; its keys `b a attach  b c claude  b f files  e b link` on the right, each address drawn as keycaps with the device letter in its hue and the verb neutral; S: one row); SESSIONS `needs you first`: one row of live thumbnails ranked by attention (the ghost "new on <selected>" tile fills a free cell; the full grid is in each device; `+2 more ›` opens the palette, shown only when tiles are hidden); the ticker (`▲ squirtle-jetson unreachable via ssh-overlay 3h  s r retry`, a tappable `+N` chip when more wait; the first-ranked session is the first tile right above it, so its `◆ … needs you  t a attach` item is not repeated there; addresses as on the vitals strip); the footer follows the focused zone. Phone (< 70 columns): NETWORK with the legend, the device tree (section 3; a keycap mini map only crowded at that size), up to two ticker rows (tinted, the whole row is the tap target) and a wrapping footer. |
| Device (`letter`) | Header with identity, evidence, mesh squares, cpu chart and gauges (four rows, one blank row before SESSIONS); session thumbnails keyed `a s d g`, only as tall as their screen tails and at most two hub tiles wide (a lone session reads as a tile beside its ghost, not a banner); the ghost tile "new on <device>" (`c` claude, `x` codex, `n` shell), content-sized, right beside them; below 90 columns the ghost is the one-row `new  c claude  x codex  n shell` under the tiles, so a lone session keeps the full width; under them LINKS: the same star from this device with plain names (no letter stands out: in a device, letters are slots and verbs), then (at M+, when rows are left) one block per link: name, network and policy on the first row of an rtt area chart (sharing the rows the screen leaves, up to eight each, less a breathing row above the footer, a closed link one row; from 0 on one scale shared by every link and named on the zone's rule, `rtt 0–20ms`, so a calm 10-17 ms link looks calm; a faint body, lighter on light themes, under a full-hue top cell; a column whose top lands on a cell boundary gets a thin `▔` cap over a body cell, so a dip reads as a lower value, never a full-hue notch) that runs to the stats column less a two-column gutter, rtt and loss at the right; the header's cpu chart stops at 72 columns and the metrics block hugs the right edge; a link closed both ways draws no chart and says `closed both ways` with the last value seen, muted. The star takes at most ten rows (two-row labels with air) and sits under the zone label. Phones give the rows the entries, ghost row and links leave to the selected session as a thumbnail (up to its screen tail), so a prompt is recognisable. Phones (touch, under 70 columns) draw no star: one plain row per link on fixed columns sized to the longest name (name, policy, rtt or `last 41ms` when closed, since the policy column already says `off`; the loss goes first when rows are short). `⌫` asks inline on the selected tile's last row (`stop refactor transport?  ⌫ again to stop, any key keeps it`, warn-tinted; footer `⌫ stop it  esc keep`; capture `CX_LAB_FOCUS=stop tests/lab.sh mission device:innovation`): a second `⌫` stops, every other key keeps it. Footer `⏎ a attach` (Enter and the slot keys on one chip), `r retry` when the device is down or late, then `f files  e link  ⌫ stop  esc back  ` hub  - last  / find` (peek from 100 columns); phones lead with `ret attach  esc back`, `del stop` last. |
| Session (`⏎`, a slot key) | Full screen: the pane owns every row but the top one, a breadcrumb `cx › innovation › i a refactor transport  ● working` (the session's address, device keycap then slot keycap, before its title exactly as on its tile) whose segments are taps (hub, device) and whose right end is the `⏏ ctrl+\ detach` chip (ASCII `ctrl+\ detach`); the pane reflows to the screen's width (words wrap at spaces, an agent's drawn composer box is redrawn at that width with its right edge, before the ASCII substitution so ASCII keeps `+--+` at the pane width with blank sides; continuation rows hang at the line's content column, after its `⎿` / `+` / `●` marker, keeping the first row's tint), as the agent would draw it in a pane that size; at 120+ columns one muted live fact sits before it (`up 1h · 1 client`). On phones (touch, under 70 columns) the chip is ` ctrl+\ ` without `⏏` (phone fonts draw it as a wide emoji), so `cx › i a title ●` keeps its room (phones drop the device name). |
| Session peek (`␣`, `CX_LAB_FOCUS=peek`) | A meta line (address, state, provider, folder, uptime, clients, activity trend), the screen 1:1 on a deeper surface, and at M+ the OTHERS filmstrip. Hub letters work in peek (`t a` attaches t's slot a); `Tab` steps through the other peeks; `⏎` attaches, `⌫` stops, Esc back, `` ` `` hub. |
| Files (`letter f`) | Tree with a size micro-bar and age, git letters; under it, when rows are left, the selected entry's path, size, age and git state and its send targets as device letters (five rows: a SELECTED zone, one fact per row, and with room for them its send targets one per row with their route's rtt and relay, `b blastoise-odroid  via creation 17ms`; two to four rows: the path, then size, age and `s send to t c b` on one row; only devices a transfer can reach over links open in that direction are listed, those reached through a relay in their hue but not bold; tree guides are blank indentation in ASCII, since `|` and `:` are the selection bars; untracked is `U`; a tree cut by the height ends in a muted `+N more ↓`, and phones keep the spacer row above their wrapped footer). Inline syntax preview at ≥100 cols, its surface as tall as the file, where a line cut at the edge ends in an accent `›`. The footer teaches `j k move  l h fold  s send to` (80 columns: `⏎ open  j k move  l h fold  s send  spc preview  / find  esc back`); phones `ret open  esc back  s send  spc preview  / find  ` hub` (wrapped). `s` (send to) dims the files and raises a bottom sheet with the orbit seen from this device: press or tap the destination's letter (capture: `CX_LAB_FOCUS=dialog tests/lab.sh mission files`). The transfer then shows on the hub: its link heavy and lit, the rate on its chip, name and percentage in the top bar. |
| Palette (`/`) | Jump to anything: devices, sessions and actions (new claude / codex / shell, files, link), each with the keys that reach it directly, drawn as keycaps (the row's device letter in its hue), so the palette teaches the shortcuts by sight; action rows have no bullet, the key column is their mark. Rows cut by the height cap end in a muted `+N more ↓`, never silently. The selected row is the bar, a bold accent name and `accent.bg` where tints exist. The backdrop is dimmed to one level for every ink (names, strokes, tile text, bold or screen-authored colours). While it is open only the sheet is live: the backdrop's targets are dropped and a tap beside the sheet closes it. `/` is the one search everywhere; inside files or a preview it opens pre-filtered to that scope as the first section. Sized to its results, at most 75 % of the height (60 % would cut results with backdrop rows free), a breathing row between sections when every row still fits, a breathing row, then a row of `⏎ go  ↑↓ move  esc close` chips (`up/dn` on phones and in ASCII); on phones a full sheet on a cleared backdrop with those chips on the screen's last row like every phone footer (the sheet ends a row above them, so they never sit in its border), descriptions cut with an ellipsis before the key column. |
| Keys / secrets | Masked secret cards keyed `a s d g` (a slot key opens that card's editor, so adding the missing Tailscale key is `K g`, paste, `⏎`); missing ones get a solid accent button with a background-coloured label that names the key acting on it: `⏎ add` on the selected card, its slot key on the others (`g add`; in 16 colours a cyan cell with black text like the root capsule, never the gray keycap). Under the cards `h new secret` (the next free slot) adds any other secret by name; the footer says `h new`. Cards run the full content width like every zone. Names are cut against the room really left (the button only on its row). The status line under the cards is cut at a `·` boundary, never mid-word. The zone label and cards sit on the same column as every other screen's zones, with one breathing row under the label so its bar and the selected card's bar stay two marks. Crumb `cx › keys`. |
| Onboarding | Viewer pill alone on an empty orbit (the same `◉ tranquility` capsule as the hub's root), the wrapped one-sentence explanation, a `⏎ Connect Cloudflare` button, three plain numbered steps (`1.`, not keycaps: they are not keys), a breathing row, then `+` join code (the global invite and join key; `j` stays motion) and `esc` plain SSH (the footer says `esc plain ssh`: chip labels are never cut to a first word, a phone footer wraps instead). Each step's note sits in a side column only when it fits whole; below about 90 columns it wraps on muted rows under its step, and the button's note moves under the button. |
| Join | Half-block QR (dark on light, always: in 16 colours on a dark theme colour 15 and 0; on a light theme, where bright white is often a gray, the terminal's own background and black ink with the NO_COLOR glyphs; with the standard four-module quiet zone wherever it fits, two only when the alternative is no QR) and the text column centred as one group (with no QR, as in ASCII and on phones, the text column centres on its own, as wide as the one-liner, with a faint `QR on a wider screen` in Unicode); at M+ the code is drawn in three-row half-block letters (a 3x5 pixel font of square half-block pixels) on a five-row tint, below that as spaced mono text on a tinted surface (`K X 7   4 Q M`; not keycaps: its letters are not cx keys), the one-liner broken after a space or `/`, never inside the code, over up to three rows (readable to retype where clipboard escapes fail) and an expiry bar. Footer `y code  Y command  esc done  R new code` (shifted: a new code revokes one that may already be shared). |
| Edge editor | Crumb `cx › t ⇄ i link`. The network diagram right under the crumb, with the edited link at full hue (the same thin line: one weight everywhere) and everything else faded to faint: other lines, the other nodes' dots and names (muted, plain, no underline: here `a p o` are policy keys) and the root's capsule tint. Phones draw no map: the link alone between its two keycaps (`t ── ⇄ key ── i`), so the sheet keeps its rows. The sheet below has one segmented control per direction, labelled by its direction (`t→i`): the opener's outbound lane first with `a p o` (key, password, off), the way back with `A P O`, so no arrow keys are needed; `j k` move the lane bar as in every list. Each press applies at once, `u` undoes; one line says so (`Each press applies at once; u undoes.`, never "A key", which reads as the `A` binding) and how many sessions cross the link, then the rtt history (an area chart, from 0) and, when four or more rows remain, the live thumbnails of the crossing sessions (their address as plain muted text: here `a` is a policy key) with the LINK figures at the right (now, min / max, mean, jitter, loss, network). The sheet takes the rows the map leaves at its foot. Every segment has one shape, keycap, a blank cell, the word (` p  password`), the selected one on its policy tint, so unselected keys still read as pressable (phones say `pass`); ASCII keeps a blank cell after the `>` lane bar (`> t->i`); the explanatory line is muted text. Footer `a p o  t→i key/pass/off   A P O  i→t   j k lane   u undo   esc done` (lane first; phones drop the verbs, which the segmented controls name). |
| Help | Crumb `cx › help`. No drawn keyboards (user feedback, 2026-11-10: two keyboard maps looked odd): a calm list for the view it was opened from (`ON THE HUB`), centred, one column at every size. Seven keys grouped by intent (`j k` previous / next device; one keycap width per column, so the caps line up), the intent word muted in a left column (`move  j k  tab  ⏎`, `look  spc  -`, `act  e  ⌫`), a breathing row between groups; then the device-letter grammar said once: the first four device keycaps in their hue (`…` when there are more) and `press a device's letter to jump; then a s d g attach a slot, c x n new claude / codex / shell` (keys bold, the rest muted; beside the keycaps from 56 columns, under them on phones); then, desktop only, the map's line grammar as its own quiet `lines` row (`── key  ┄┄ password  → one way  ⊘ closed`); then one row of global keys as footer-style chips (`/ find  ` hub  + invite  K secrets  ctrl+\ detach`; `esc` is on the footer, where it closes help), wrapping on phones. Breathing rows give way first on short screens, then the zone label. Phones and ASCII say `ret`, `del` / `bksp`. |

## 3. The network diagram

One renderer (`orbit::draw`) serves the hub (root = this device, three hops), the device
view's LINKS (root = that device, one hop, letters as plain text), the send-to picker
(letters live) and the edge editor (the edited link at full hue, everything else faded,
letters plain because `a p o` are policy keys there). It aims for a quiet network
diagram: continuous thin lines of one weight, crisp labels, air between everything,
nothing on the lines but the lines.

- **Columns by hop.** A spanning tree from the root over routed links (jump links are
  drawn through their relay). The root's subtrees split between its right and left so
  both sides hold about as many rows (the right takes a tie), mesh members kept
  together. Every device is one row, with a clear row between labels whenever the map
  fits with it (every named richness is tried with that air before any without it, bare
  keycaps last); a relay sits on the middle row of its children, so two peers sit
  symmetrically about it. The columns of one hop are aligned (right-hand labels start on
  one column, left-hand ones end on one) and as wide as their widest label, with a calm
  gap between columns (6 / 8 / 10 / 12 by size). The drawing is centred in the band (not
  the root) one row under its top; the hub keeps a blank row under it. Labels that would
  leave the band, and anything hanging from them, become `+N more` (opens the palette).
  Devices with no routed link sit in the first column with no line.
- **Lines** are box-drawing characters: a stub leaves the root (or relay) on its row, a
  trunk runs down the column gap with rounded corners `╭ ╮ ╰ ╯` and junctions
  `├ ┤ ┬ ┴ ┼`, and a branch enters each label on its row. Every line end keeps one clear
  cell from a label, one more where a selection's brackets may sit. ASCII: `-`, `|` and
  `+` at every corner. One weight everywhere; unlit lines at 55 %, the selection's route
  back to the root (and its own children) at 95 %. **Hue is network**: the mesh hue
  (violet, blue), teal for lan, neutral for direct ssh; a cell shared by links of
  different networks is neutral. **Stroke is policy, used sparingly**: solid where a key
  is allowed, dashed (`┄ ┆`; ASCII an unbroken `=`) where only a password is, a
  faint neutral line where nothing crosses (30 %; ASCII `.` and `:`), with `⊘` at the
  device's end when it is closed both ways (a down device's `✕` says the rest). A one-way
  link has one small arrowhead (`→ ←`, ASCII `>` `<`) at the end access reaches: beside
  the device, or where its branch leaves the trunk. Nothing else rides on a line.
- **No figures on the map.** rtt, transfer rates and session slots are said once, for the
  selection, in the vitals strip (and in the device and edge views). Each link keeps a
  3x1 tap target on its branch (desktop; phones tap the tree rows).
- **Nodes** have one treatment everywhere: the status dot on the side the line arrives
  (`● innovation`, left-hand `laptop-guest ◐`), then the name with its hotkey letter
  underlined in the accent (a keycap in front when the letter is not in the name). No
  background except the selection. A down device is its one red `✕` and a muted name.
  The viewer seen in another device's star is a plain peer (`●`); `◉` marks the map's
  root only. Richness is chosen per frame (name 16/12/9, keycap only).
- **The root** is a capsule (`◉ tranquility`: an accent tint; a solid accent row in 16
  colours; reversed in NO_COLOR; in ASCII its padding cells are `(` `)`, which become
  `[` `]` when it is selected: `[@ tranquility]`, never a double wrap). The same capsule
  marks the viewer on onboarding.
- **Selection**: the label on the accent tint (a dim tint without focus) with a bold
  accent name; without tints, `[ ]` brackets one cell outside it. Its route back to the
  root is lit.
- **Phone tree** (the phone hub has no map): every device depth first, the root first,
  then each relay followed by the devices reached through it (`├─ c creation`,
  `│ ├─ b blastoise-odroid`; ASCII `+-`, `` `- `` and `|` guides), the branch in the hue
  of the link to its parent, dashed (`├┄`) where only a password is allowed, faint where
  the link is down; keycap, name, status, rtt or last sighting, session pips; with room
  a second row (cpu sparkline, %, temperature). Each row is the device's tap target.
  Rows the tree leaves go to a SESSIONS list (two-row session rows, what needs you
  first) above the ticker. The send-to picker keeps a keycap map on phones (its letters
  are the choice).
- **Lan without colour**: lan is a hue, so in NO_COLOR and ASCII the legend drops it.
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
| pill tint | root capsule | accent 26 % | accent 18 % | nearest | solid accent row, black text | reversed; ASCII `(` `)` padding |
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

Glyphs, with ASCII fallbacks, each substitute meaning one thing: selection and focus
`▍ ▌` / `>` on the first row and `|` below in the focused zone, `:` on every row in an
unfocused one (and nothing else is `>`, `|` or `:` in ASCII: tile gutters, tree guides,
sheet and ghost-tile sides and an agent's own prompt bar are blank); status `◉ ● ◐ ✕ ◌` / `@ o ~ x ?`;
session state `◆ ● ○ ·` / `! o - .` (`!` means needs you and nothing else; `*` stays
the agents' own bullet); alerts `▲` / `^` (and nothing else is `^`); a transfer `⇣` / `dl`; one-way arrowheads `→ ↘ …` / `->` `<-`; policy `⇄ → ← ⊘` / `<> -> <- off`; line
styles `─ ═ ┈` / `- = .`; mesh squares `▄` / the mesh's number (`1`, `2`, repeated in
the legend, so membership reads without colour); the separator `·` / `-`; a cut `…` /
`..` (also inside fixture text); crumbs `›` / `/` (the sessions overflow `+2 more ›` drops it in ASCII); a symlink `→` / `*`; an agent's own prompt bar inside a thumbnail `▌` / blank (so `|` and `:` stay cx's selection bars); folds `▾ ▸` / `- +`; a line cut at
the preview edge `›` / `$`; keycaps are coloured cells in every tier
(`REVERSED|BOLD` without colour); link lines become `- | +`; `⎿` is `L` and `✻` is `*`;
charts and sparklines become `_ = #` (`.` and `-` are link strokes; `#` is used by nothing else); sheet and palette frames are `+--+` with blank sides; the minimap is omitted in ASCII
and NO_COLOR; the QR is drawn with `▀▄█` even in NO_COLOR and dropped in ASCII and where it cannot fit above the code (40x20: the code and one-liner carry it);
Fixture text is transliterated in ASCII (`asciify`), so the lint stays at 0 warnings.
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
| Sessions | (pips in the tree, needs-you in the ticker) | one row of 3 tiles, 5 tall | one row of 3, up to 9 tall | one row of 4, up to what the screens hold; two rows (the rest and the ghost) when the map leaves room for each to keep ten |
| Device view footer | `ret attach`, `esc back`, `f files`, `e link`, `` ` hub``, `del stop` (two rows, top first) | 7 chips | 7 chips | same |
| Hub footer | `ret open` / `ret attach` first, then `- last`, `/ find`, `+ invite`, `K secrets`, `? help` (two rows, top first) | by zone, up to 8 chips | by zone | same |

The orbit takes the rows the thumbnails do not need; the ticker sits right above the
footer (it is the last line you read before choosing a key). Session tiles share one width and
one 2-column gutter; rows are equal height. Thumbnails show the last non-blank lines of
the screen and drop shared indentation when narrower than the screen; the agent's own
reversed key hints (`y Yes`) are drawn in flat thumbnail ink (no reverse, underline or
bold), so they never look like cx keycaps. A cut never shows two ellipses. Under 20x6
the frame shows only `cx`. Phones (and ASCII) label keys with words where symbols may be
missing from the font: `ret`, `del` (ASCII `bksp`); `spc` everywhere (the `␣` symbol
renders as a bare underscore). A phone footer leads with its primary verb, then
`esc back`, and every chip has a word. When it wraps, rows fill in priority order from
the top (the spacer row above the footer takes the first chips) and a destructive verb
(`del stop`) is always last. Chips keep two columns between them at every width (a wrapped phone footer wraps one chip earlier rather than closing the gap).

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
| Help | Crumb `cx › help`. No drawn keyboards (user feedback, 2026-11-10: two keyboard maps looked odd): a calm list for the view it was opened from (`ON THE HUB`), centred, one column at every size. Seven keys grouped by intent (`j k` previous / next device; one keycap width per column, so the caps line up), the intent word muted in a left column (`move  j k  tab  ⏎`, `look  spc  -`, `act  e  ⌫`), a breathing row between groups; then the device-letter grammar said once: the first four device keycaps in their hue (`…` when there are more) and `press a device's letter to jump; then a s d g attach a slot, c x n new claude / codex / shell` (keys bold, the rest muted; beside the keycaps from 56 columns, under them on phones); then, desktop only, the map's line grammar as its own quiet `lines` row (`── key  ┄┄ password  → one way  ⊘ closed`); then one row of global keys as footer-style chips (`/ find  ` hub  + invite  K secrets  ctrl+\ detach`; `esc` is on the footer, where it closes help), wrapping on phones. Breathing rows give way first on short screens, then the zone label. Phones and ASCII say `ret`, `del` / `bksp`. |

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
  view keep names on a phone. `orbit::tree_rows` gives the phone tree the same spanning
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
