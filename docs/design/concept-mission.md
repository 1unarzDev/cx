# Concept: mission control

Code: `src/ui/lab/concepts/mission/` (`mod.rs` render context and shared helpers,
`orbit.rs` the network orbit and its braille canvas, `hub.rs` the start screen (vitals
strip, ticker, phone tree), `tiles.rs` device rows and session thumbnails, `views.rs`
every other screen). Render: `tests/lab.sh mission`.

The judges' winner, with their agreed grafts (2026-11-10): the hub is an orbit (this
device in the middle inside a faint dotted halo, peers on rings by hop distance, curved links
whose hue is the network and whose stroke is the policy, mesh members marked with their
mesh's swatch), under it
the selected device's vitals strip, one row of live session thumbnails (what needs you
first) and a one-line alert ticker that pairs each problem with the keys that fix it.
Hotkey letters are underlined inside node names; a session is full screen under a
one-row breadcrumb with a tappable `ctrl+\ detach` chip; phones get a mini orbit over an
indented device tree. Each screen is one grammar: **a device letter, then a verb or a
session slot**.

## 1. Principles

1. **The orbit is the hub.** You do not pick from a list of devices; you look at the
   network and press the letter underlined in the node's name. Every node, edge chip, tile and
   chip on screen is also a click/tap target that maps to the same action as its key.
2. **Recognise, don't read.** A session tile shows the real last informative lines of its
   screen (agent status, the approval prompt, the failing test) in the screen's own
   colours, in every colour tier. Two Claude sessions look different because their
   screens do.
3. **Evidence on the surface, said once.** Every device shows how its status is known:
   the rtt rides on its link chip, the node card carries cpu history, temperature and
   its sessions' slot letters in their state colours (`a s`: how many run there and the
   key that attaches each), a down node says `seen 3h`. Unknown is `?`, never green.
   Under the map the vitals strip names the selected device with its route from here and the
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
| Hub | `▍NETWORK` (accent while focused) with the stroke legend and the mesh swatches (members up) on its rule (the rule only when at least 12 cells remain); the orbit (section 3) under it, as tall as the rows left, its drawing one row under the band's top, and the rows it leaves at its foot go to the thumbnails (up to what their screens hold; a second row of tiles with the rest of the sessions and the ghost when each row still keeps ten); the selected device's vitals strip (no left bar: column 0 is the focus bar only; M+: two rows, name with route and per-hop rtt `t → c → b  0.6 + 2.4 ms`, os and account; cpu sparkline and %, mem gauge, temperature, uptime, meshes; its keys `b a attach  b c claude  b f files  e b link` on the right; S: one row); SESSIONS `needs you first`: one row of live thumbnails ranked by attention (the ghost "new on <selected>" tile fills a free cell; the full grid is in each device; `5 sessions ›` opens the palette); the ticker (`◆ fix flaky test needs you  t a attach   ▲ squirtle-jetson unreachable via ssh-overlay 3h  s r retry`, a tappable `+N` chip when more wait); the footer follows the focused zone. Phone (< 70 columns): NETWORK with the legend, a keycap mini orbit, the device tree (section 3), up to two ticker rows (tinted, the whole row is the tap target) and a wrapping footer. |
| Device (`letter`) | Header with identity, evidence, mesh squares, cpu chart and gauges (four rows, one blank row before SESSIONS); session thumbnails keyed `a s d g h l`, only as tall as their screen tails and at most two hub tiles wide (a lone session reads as a tile beside its ghost, not a banner); the ghost tile "new on <device>" (`c` claude, `x` codex, `n` shell), content-sized, right beside them; under them LINKS: the same star from this device with plain names (no letter stands out: in a device, letters are slots and verbs), then (at M+, when rows are left) one block per link: name, network and policy on the first row of an rtt area chart that grows into the rows the star leaves (up to four, six at L), rtt and loss at the right; a link closed both ways draws no chart and says `closed both ways` with the last value seen, muted. The star takes at most ten rows (two-row labels with air) and sits under the zone label. Phones (touch, under 70 columns) draw no star: one plain row per link on fixed columns sized to the longest name (name, policy, rtt or `closed · last 41ms`; the loss goes first when rows are short). Footer `⏎ a attach` (Enter and the slot keys on one chip), `r retry` when the device is down or late, then `f files  e link  ⌫ stop  esc back  ` hub  - last  / find` (peek from 100 columns); phones lead with `ret attach  esc back`, `del stop` last. |
| Session (`⏎`, a slot key) | Full screen: the pane owns every row but the top one, a breadcrumb `cx › innovation › i a refactor transport  ● working` (the session's address, device keycap then slot keycap, before its title exactly as on its tile) whose segments are taps (hub, device) and whose right end is the `⏏ ctrl+\ detach` chip (ASCII `ctrl+\ detach`); at 120+ columns one muted live fact sits before it (`up 1h · 1 client`). On phones (touch, under 70 columns) the chip is ` ctrl+\ ` without `⏏` (phone fonts draw it as a wide emoji), so `cx › i a title ●` keeps its room (phones drop the device name). |
| Session peek (`␣`, `CX_LAB_FOCUS=peek`) | A meta line (address, state, provider, folder, uptime, clients, activity trend), the screen 1:1 on a deeper surface, and at M+ the OTHERS filmstrip. Hub letters work in peek (`t a` attaches t's slot a); `Tab` steps through the other peeks; `⏎` attaches, `⌫` stops, Esc back, `` ` `` hub. |
| Files (`letter f`) | Tree with a size micro-bar and age, git letters; under it, when rows are left, the selected entry's path, size, age and git state and its send targets as device letters (five rows: a SELECTED zone, one fact per row; two to four rows: the path, then size, age and `s send to t c b` on one row; only devices a transfer can reach over links open in that direction are listed, those reached through a relay in their hue but not bold; tree guides are `:` in ASCII, since `|` is the focus bar's continuation). Inline syntax preview at ≥100 cols, its surface as tall as the file, where a line cut at the edge ends in an accent `›`. The footer teaches `j k move  l h fold  s send to`; phones `ret open  esc back  s send  spc preview  / find  ` hub` (wrapped). `s` (send to) dims the files and raises a bottom sheet with the orbit seen from this device: press or tap the destination's letter (capture: `CX_LAB_FOCUS=dialog tests/lab.sh mission files`). The transfer then shows on the hub: its link heavy and lit, the rate on its chip, name and percentage in the top bar. |
| Palette (`/`) | Jump to anything: devices, sessions and actions (new claude / codex / shell, files, link), each with the keys that reach it directly, drawn as keycaps (the row's device letter in its hue), so the palette teaches the shortcuts by sight; action rows have no bullet, the key column is their mark. Rows cut by the height cap end in a muted `+N more ↓`, never silently. The selected row is the bar, a bold accent name and `accent.bg` where tints exist. While it is open only the sheet is live: the backdrop's targets are dropped and a tap beside the sheet closes it. `/` is the one search everywhere; inside files or a preview it opens pre-filtered to that scope as the first section. Sized to its results, at most 60 % of the height, with a breathing row, then a row of `⏎ go  ↑↓ move  esc close` chips (`up/dn` on phones and in ASCII); on phones a full sheet on a cleared backdrop, descriptions cut with an ellipsis before the key column. |
| Keys / secrets | Masked secret cards keyed `a s d g` (a slot key opens that card's editor, so adding the missing Tailscale key is `K g`, paste, `⏎`); missing ones get a solid accent button with a background-coloured label that names the key acting on it: `⏎ add` on the selected card, its slot key on the others (`g add`). Names are cut against the room really left (the button only on its row). The status line under the cards is cut at a `·` boundary, never mid-word. The zone label and cards sit on the same column as every other screen's zones, with one breathing row under the label so its bar and the selected card's bar stay two marks. Crumb `cx › keys`. |
| Onboarding | Viewer pill alone on an empty orbit (the same `◉ tranquility` capsule as the hub's root), the wrapped one-sentence explanation, a `⏎ Connect Cloudflare` button, three plain numbered steps (`1.`, not keycaps: they are not keys), a breathing row, then `j` join code and `esc` plain SSH (the footer says the same words). |
| Join | Half-block QR (dark on light, always, with the standard four-module quiet zone wherever it fits, two only when the alternative is no QR) and the text column centred as one group; the short code as large mono text on one tinted surface, three rows tall where there is room (`K X 7   4 Q M`; not keycaps: its letters are not cx keys), the one-liner wrapped over up to three rows (readable to retype where clipboard escapes fail) and an expiry bar. Footer `y code  Y command  esc done  R new code` (shifted: a new code revokes one that may already be shared). |
| Edge editor | Crumb `cx › t ⇄ i link`. The orbit right under the crumb, with the edited link drawn heavy (three dots thick) at full hue and everything else faded to faint: other links, their chips, mesh swatches, the other nodes' dots, the session slot letters on every label (here `a` is a policy key) and the other names (muted, plain, no hotkey hue: here `a p o` are policy keys) and the root's capsule tint; no halo. The sheet below has one segmented control per direction, labelled by its direction (`t→i`): the opener's outbound lane first with `a p o` (key, password, off), the way back with `A P O`, so no arrow keys are needed; `j k` move the lane bar as in every list. Each press applies at once, `u` undoes; one line says so (`Each press applies at once; u undoes.`, never "A key", which reads as the `A` binding) and how many sessions cross the link, then the rtt history (an area chart up to three rows) and, when four or more rows remain, the live thumbnails of the crossing sessions (their address as plain muted text: here `a` is a policy key) with the LINK figures at the right (now, min / max, mean, jitter, loss, network). The sheet takes the rows the map leaves at its foot. Footer `a p o  t→i key/pass/off   A P O  i→t   j k lane   u undo   esc done` (lane first; phones drop the verbs, which the segmented controls name). |
| Help | Crumb `cx › help`. Centred vertically. `+claude`, `+codex` and `+shell` all take their provider tint; the global keys sit on a grid whose column count divides them evenly (no orphan row). A drawn keyboard per layer (hub: device keys with status and rtt when the keycaps are wide enough, `e link`, `/ find`; in a device: `attach` on the slot keys, `down` / `up` on `j k`, verbs), a number-row stub with `` ` hub`` and `- last` (the round trip) where it costs no global key, `j k` `down` / `up` on the hub keyboard too, keycap width chosen so the staggered rows fit, a legend of device names and a key legend (including `- last session`), both on fixed column grids. Below 28 rows the device layer is one line of chips (`a s d attach  c x n new  f files  e link  ⌫ stop`). Phone: a list centred vertically, `then` as plain muted text in its own column, keycaps holding only real keys (device letters in their hue), descriptions on a fixed column; `del` (ASCII `bksp`) and `ctrl+\` as everywhere. |

## 3. The orbit

One renderer (`orbit::draw`) serves the hub (root = this device, three hops), the device
view's LINKS (root = that device, one ring, letters as plain text), the send-to picker
(letters live) and the edge editor (the edited link heavy, everything else faded,
letters plain because `a p o` are policy keys there).

- **Rings by hop.** A spanning tree from the root over routed links (jump links are drawn
  through their relay). First-ring subtrees share the circle by size; children divide
  their parent's sector on the next ring out (at most 50° each). The largest subtree
  faces right, where rows are long; equal ones sit on the diagonals. Ring radii are
  fractions of the band (first ring wide 0.5 / tall 0.78, keeping clear of the pill (by
  at least 11 columns at S, so every link keeps a readable run of its stroke) and,
  at M+, leaving a chip's room between the halo and the label), capped past a calm size
  (radius at most 11.5 rows, so the first ring stays within about 30 columns and 9 rows
  of the root): a big screen gives the rows to the thumbnails instead of spreading a few
  nodes thin. The finished drawing is centred in the band, not the root.
- **The device star** (a device's LINKS, one ring): the root sits at the band's centre
  in the same capsule and halo (no halo under 80 columns, so the links keep their
  length), its peers beside it, left and right where rows are long, top first on each
  side. Its links leave the capsule's end facing their peer and arrive level with it, so
  each chip rides on its peer's row; a chip that cannot sit clear of the labels is
  dropped (the rows under the star say `closed both ways`).
- **The root** is a capsule (`◉ tranquility`: an accent tint; a solid accent row in 16
  colours; reversed in NO_COLOR; in ASCII its padding cells become `(` `)`, so
  `(@ tranquility)`, and a selection adds `[ ]` outside it) inside a dotted halo in
  faint ink that rings its evidence row too and yields to links (no bead within a cell of
  a stroke, so a sparse password link never reads as halo; no halo in ASCII, where `'`
  beads beside `-` `/` strokes read as noise and the `( )` capsule marks the root), with
  its own evidence centred beneath
  (`you ▅▅█▅ 18% 47° a s`). The same capsule marks the root of a device star and the
  viewer on onboarding. `◉` marks the root whenever it is online.
- **Nodes**: status dot, name with the hotkey letter underlined in the device hue (a
  keycap in front when the letter is not in the name), and with height a second row:
  cpu sparkline, cpu %, temperature (L), session slot letters in their state colours
  (hub only: elsewhere `a s` are not those sessions' keys); a down device says `seen
  3h`. Each mesh the device is in adds its swatch (`▄` in the mesh hue, ASCII its number,
  `1 2`) on the label's outer side, the same swatch as the legend. Left-hand nodes read toward the root (`innovation ●`).
  Richness is chosen per frame (two-row, name 16/12/9, keycap only); a label that would
  overlap slides along its ring first, and anything still overlapping becomes `+N more`
  (opens the palette). Phones draw keycaps only (the tree carries the names).
- **Links** are quadratic curves with a gentle bow, drawn with braille dots (2x4 per
  cell; in ASCII one cell per column where the line runs sideways (per row where it runs up or down), its character from the next cell: `-` / `|` where it stays, `\` or `/` where it steps, so a link is one continuous run, never doubled). **Hue is network**: the mesh
  hue (violet, blue), teal for lan, neutral for direct ssh, red when down. **Stroke is
  policy**: a continuous chain where a key is allowed, sparse dots where only a password
  is, dash-dot when down, faint beads when closed both ways, plus an arrowhead
  (`→ ↘ ↓ …`, ASCII two cells `->` / `<-` by its horizontal sense, since `>` `^` `v`
  mean other things) two thirds along a one-way link pointing the way access goes.
  ASCII: `-`, `.`, `-.-`.
- **Meshes** are their hue: links inside a mesh are drawn in it and members carry its
  swatch, which the legend names with members up (`▄ cloudflare-warp 4/4`). Orbit arcs
  around members were dropped (round 2): a mesh whose members are spread around the root
  needs an arc over most of the map, which read as a broken link and buried the canvas.
- **Chips** at a link's middle: M the rtt (`14ms`, or a transfer's `↓38M/s`), L the
  policy word too (`key 14ms`, `pass 1.2ms`); `✕ down` and `⊘ off` at every size, except
  that a bare `✕` (S and keycap maps) is dropped when an end already shows `✕`. A chip
  the labels crowd out of the middle slides along its curve (a running transfer's chip
  is placed first, then the lit links). Each chip, or a 3x1 spot on the link, is its tap
  target (desktop only: on phones the tree rows are the targets). In ASCII an arrowhead
  keeps one clear cell each side (`<-` beside a `.` would read as `-.-`), and the capsule
  two.
- **Labels win.** Node labels (plus two columns each side), the root's evidence row and
  chips are placed before the strokes are flushed and their cells, with a clear column
  each side, are masked. So no
  stroke or bead ever restyles a letter or touches `a s`, in any tier.
- **Selection**: a tinted label with bracket corners `╭ ╮ ╰ ╯` (`▕ ▏` for one row, ASCII
  `[ ]`), bold accent name; its links and its route back to the root lit; its peer links
  (not in the tree) appear only while it is selected, bowing outward.
- **Phone tree** (under the mini orbit): every device depth first, the root first, then
  each relay followed by the devices reached through it (`├─ c creation`, `│ ├─ b
  blastoise-odroid`), the branch in the hue of the link to its parent and dotted (`├┄`)
  where only a password is allowed or the link is down; keycap, name, status, rtt or last
  sighting, session pips; with room a second row (cpu sparkline, %, temperature). Each
  row is the device's tap target.
- Touch layouts (< 70 columns): node and pill targets are at least 3 rows by 7 columns,
  and `Cx::unclash` repeats until no two targets overlap.

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
`▍ ▌` / `>` on the first row and `|` below (and nothing else is `>` or `|` in ASCII: tile
gutters are blank, tree guides `:`); status `◉ ● ◐ ✕ ◌` / `@ o ~ x ?`;
session state `◆ ● ○ ·` / `! o - .` (`!` means needs you and nothing else; `*` stays
the agents' own bullet); alerts `▲` / `^` (and nothing else is `^`); a transfer `⇣` / `dl`; one-way arrowheads `→ ↘ …` / `->` `<-`; policy `⇄ → ← ⊘` / `<> -> <- off`; line
styles `─ ═ ┈` / `- = .`; mesh squares `▄` / the mesh's number (`1`, `2`, repeated in
the legend, so membership reads without colour); the separator `·` / `-`; a cut `…` /
`..` (also inside fixture text); crumbs `›` / `/` (the sessions overflow `5 sessions ›` drops it in ASCII); a symlink `→` / `*`; an agent's own prompt bar inside a thumbnail `▌` / `:` (so `|` stays cx's focus bar); folds `▾ ▸` / `- +`; a line cut at
the preview edge `›` / `$`; keycaps are coloured cells in every tier
(`REVERSED|BOLD` without colour); link lines become `- | +`; `⎿` is `L` and `✻` is `*`;
charts become `_ : = #` and sparklines `_ = #` (`.` and `-` are link strokes; `#` is used by nothing else); sheet and palette frames are `+--+` / `|`; the minimap is omitted in ASCII
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
| Orbit | keycap mini orbit | names (9-16), chips only on down and closed links | two-row nodes, rtt chips | `key 14ms` chips, more air |
| Below it | device tree (one or two rows per device), up to two ticker rows | legend, one-row vitals | legend, two-row vitals | same |
| Sessions | (pips in the tree, needs-you in the ticker) | one row of 3 tiles, 5 tall | one row of 3, up to 9 tall | one row of 4, up to what the screens hold; two rows (the rest and the ghost) when the map leaves room for each to keep ten |
| Device view footer | `ret attach`, `esc back`, `f files`, `e link`, `` ` hub``, `del stop` (two rows, top first) | 7 chips | 7 chips | same |
| Hub footer | `ret open` / `ret attach` first, then `- last`, `/ find`, `+ invite`, `K keys`, `? help` (two rows, top first) | by zone, up to 8 chips | by zone | same |

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
(`del stop`) is always last.

## 6. Interaction model

Hub keys are non-letters plus `e`, `j` and `k`, so every other letter can be a device: a
real build never assigns `e` (link) and assigns `h` `j` `k` `l` last, only past 21
devices, so on the hub `j k` move through the sessions row and the phone tree as in every
list and a vim hand pressing `h` or `l` never opens a device (a device whose initial is
reserved takes the next letter of its name that is neither reserved nor a slot:
laptop-guest is `p`; the large fixture follows that order). Inside a device, letters are verbs and slots; the home row
holds the slots (`a s d g h l`; `f` stays files, `j k` stay motion as in files and every
list, so a vim hand never attaches by accident). More sessions than slots are reached
through `/`. Any list inside a screen uses the
same home-row slots (secrets cards are `a s d g`). Peek has no inline typing, so hub
letters work there unchanged. `K` (keys), `+` (invite) and `?` (help) are global outside text input and attached
panes: they are uppercase or symbols, so they never clash with slots, verbs or devices.

| Key | Hub | Device | Peek | Mouse / touch |
|---|---|---|---|---|
| device letter | open that device | — | as on the hub | tap node, row or chip |
| `a s d g h l` | — | attach that session | (after a letter) attach | tap thumbnail |
| `c` / `x` / `n` | — | new claude / codex / shell here, attached at once | — | tap ghost-tile option |
| `e` + letter | edit the link from this device to that one | edit the link to that peer | — | tap the link chip |
| `f` | — | files | — | tap `f files` |
| `s` (in files) | — | send the file: the orbit becomes the picker, a device letter sends | — | tap the node |
| `⌫` | stop the selected session tile (sessions zone; inline `y` / `n`) | stop the selected session; a slot key retargets (`⌫ s y`) | stop this one | tap `⌫ stop` |
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
- **`⌫` is stop** behind an inline confirm, so a reflexive "back" costs nothing.
- In the device layer `j` and `k` are motion, never slots (six slots `a s d g h l`).
- `Ctrl+\` replaces SIGQUIT inside attached shells (spec 8.1) and **detaches at once**
  (0 ms: no wait for a second press). On ISO layouts it may need AltGr, which is why the
  tap chip exists. Its collisions (nvim's `Ctrl-\ Ctrl-n`, emacs `toggle-input-method`)
  are accepted (user decision); a literal `Ctrl+\` is sent by the palette action "send
  Ctrl+\" (it re-attaches the last session and forwards one). Also accepted: in an
  outer tmux, vim-tmux-navigator binds `C-\` (`select-pane -l`) in the root table, so
  the key never reaches cx there. Integration: probe once at startup inside tmux
  (`tmux list-keys -T root 'C-\'`); when bound, the breadcrumb chip says `⏏ detach` (the
  tap still works) and a toast names the fallback detach key, a configurable chord
  (default `Ctrl+]`, telnet's escape) that the help screen then shows instead.
- `` ` `` is a dead key on some ISO layouts, so a double `Esc` (two presses within 400 ms)
  also jumps to the hub from any cx screen **outside an attached pane** (Claude Code and
  Codex own `Esc Esc` there: rewind and backtrack); the first `Esc` acts immediately (back
  one level), the second continues to the hub. `cx` is always tappable.
- **New sessions attach at once** (`i c` lands in the new claude session), consistent with
  principle 6, so every new-session task costs two keys.
- In the edge editor `j k` stay motion (they move the lane bar); the key policy is `a`
  (`A` for the way back), so a vim hand pressing `k` never grants managed-key access.
- **Chips at S**: below 100 columns the map keeps names before chips (only closed links
  get one); the reviewers who asked for one chip form everywhere and those who asked for
  fuller names at 80 columns could not both be served at that width. The vitals strip and
  the device view carry the rtt there.
- **Hub-level answers** (`y`/`n` on the focused needs-you tile) stay out: `y` and `n`
  can be device letters (`n` is nas). The approval is `t a y`, the keys the ticker prints.
- **Numbered 1-9 attach from the hub** (one judge's graft) was left out: the ticker
  already prints the two-key address of what needs you, and digits would be a second
  address for the same session (one address per session, principle 7).
- **Session view**: the attached pane is the session view (`⏎` attaches directly); the
  breadcrumb sits on top so the detach chip is where the eye and a thumb look first.

### Keystrokes for the top tasks (from the hub)

| Task | Keys | Count | Path |
|---|---|---|---|
| Open a session on another device | `i` `a` | 2 | tap tile (1) |
| Answer the approval that needs you | `t` `a` `y` (the ticker prints `t a`) or `Tab` `⏎` `y` | 3 | tap the ticker row, then Yes (2) |
| Retry an unreachable device | `s` `r` (printed on the ticker) | 2 | tap `s r retry` (1) |
| Re-attach after detaching | `⏎` (tile still selected) or `-` (from anywhere) | 1 | tap the tile or `- last` |
| Start a new claude session on innovation | `i` `c` | 2 | tap `i c claude` on the ghost tile (1) |
| Stop a session | `Tab` `⌫` `y` on the hub, `i` `⌫` `y` (selected), `i` `⌫` `s` `y` (another slot) | 3-4 | tap `⌫ stop`, tap `y` |
| Send a file to creation | `i` `f` `j…` `s` `c` | 4 + moves | `s send to`, tap the node |
| Edit innovation → tranquility to off | `e` `i` `O` (the inbound lane, from the hub) or `i` `e` `t` `o` (the outbound lane first) | 3 | tap the link chip, tap `O off` (2) |
| Make tranquility → innovation key-only | `e` `i` `a` | 3 | tap the chip, tap `a key` (2) |
| Edit tranquility ⇄ innovation from the hub | `e` `i` then `a p o` / `A P O` | 3 | tap the chip (1) |
| Send a join code | `+` `y` | 2 | tap `+ invite`, tap the code |
| Add the missing Tailscale key | `K` `g` paste `⏎` | 3 + paste | tap `⏎ add`, paste, `⏎` |
| Browse and preview a file | `i` `f` `j…` (inline preview at ≥ 100 columns; `spc` below) or by name `i` `f` `/` `t` `r` `⏎` | 2 + moves, or 6 | tap node, `f files`, tap file |
| Back to the hub from a file preview | `` ` `` (or a double `Esc`) | 1 (2) | tap `cx` |

### Ergonomics table

| Action | Frequency | Keys | Count | Key positions / fingers | Rationale |
|---|---|---|---|---|---|
| Go to a device | high | its letter | 1 | wherever the letter is; letters are the device's initial | mnemonic beats position; printed on every node, row and chip |
| Attach a session | high | slot `a s d g h l` | 1 (2 from hub) | home row, strong fingers | most frequent action, best keys; one address everywhere; `j k` left to motion |
| Open / re-attach selection | high | `⏎` | 1 | right pinky, large key | universal |
| Detach | high | `Ctrl+\` | 1 chord | left pinky + right pinky | the only key cx steals inside sessions; instant; a literal one via the palette; also a tap chip |
| Re-attach last | high | `-` | 1 | right pinky, top row, unshifted | the round trip with detach; works from every screen outside a pane |
| Answer what needs you | high | the ticker's `t a` | 2 | its letter, then the home-row slot | the fix is printed next to the problem; `Tab ⏎` also works (the tile ranks first) |
| Retry a device | low | letter then `r` | 2 | its letter, left index top row | the ticker prints it; `r` is free in the device layer |
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
| Policy | low | `a p o` / `A P O`, `u` | 1 | left pinky home, right pinky / ring top row; Shift for the way back | applied at once with undo; no arrow to pick a lane; `j k` stay motion |
| Copy | low | `y` / `Y` | 1 | right index | vim "yank"; never collides with `c` claude |
| Invite | low | `+` | 1 chord | shift + right pinky | rare; also a footer chip |
| Keys and secrets | low | `K` | 1 chord | shift + right middle | rare; global (uppercase never clashes) |
| Help | low | `?` | 1 chord | shift + right pinky | convention |

## 7. Degradation

- **256 colours**: identical layout; blends resolve to the nearest xterm index.
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
hub` (a selected outer node: lit route, peer links with chips, vitals strip).

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
