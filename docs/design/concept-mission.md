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
   session pips, a down node says `seen 3h`. Unknown is `?`, never green. The former
   device strip repeated all of this and is gone from the hub (phones keep device rows,
   because their schematic is keycaps only).
4. **Tints, not boxes.** Panels are surfaces (background tints). Each zone opens with a
   spaced small-caps label followed by a faint rule (`▍N E T W O R K ──────`), with one
   blank row above it when the height allows. Boxes are used only for the dashed "new
   session" ghost tile. Without tints (16 colours, NO_COLOR) tiles are separated by a
   faint `│` gutter.
5. **Focus is a colour bar, in every tier.** The focused zone's label gets an accent bar
   `▍`; its selected item gets a left bar (`▍`/`▌`, ASCII `>`) and a bold accent name,
   plus `accent.bg` where tints exist. 16 colours and NO_COLOR use the same bar, not a
   reversed block. Unfocused zones keep a muted bar at their selection.
6. **Every action flows into the next.** A letter lands you on the device with its
   sessions under the home row; a slot key attaches straight away (no preview chain).
   Detach (`Ctrl+\`) returns to the tile you left, still selected, so `⏎` re-attaches.
7. **One address per session.** A session is always shown as its device keycap plus its
   slot keycap (`i` `a`) on hub tiles, device tiles, phone rows, the peek filmstrip, the
   palette and help. That is exactly what you type from the hub. There are no hub digits.

## 2. Screens

| View | What it shows |
|---|---|
| Hub | NETWORK (header with mesh, colour and line-style legend; the star schematic) and SESSIONS (whole rows of equal thumbnails ranked by attention, then the ghost tile "new on <selected device>" with its real keys `i c`, `i x`, `i n`, only as tall as its content). Session rows are sized first; spare height makes the tiles a little taller and then goes to the map, so no zone has a void. Phone (< 70 columns): keycap schematic, device rows (two rows each when the height allows), a two-row "needs you" strip, and a thumb footer. |
| Device (`letter`) | Header with identity, evidence, mesh squares, cpu chart and gauges; session thumbnails keyed `a s d g h j k l`, only as tall as their screens; the ghost tile "new on <device>" (`c` claude, `x` codex, `n` shell), content-sized, in the same row; under them LINKS: the same star, one hop from this device, every chip with its policy word and rtt, a blank row above the footer. |
| Session peek (`␣`) | A meta line (address, state, provider, folder, uptime, clients, then the activity trend right after it), the screen 1:1 on a deeper surface only as tall as the screen, and the OTHERS filmstrip. Hub letters work in peek (`t a` attaches t's slot a; no inline typing, so letters are free) and `Tab` steps through the other peeks. `⏎` attaches, `⌫` stops, Esc goes back one level, `` ` `` to the hub. Phones drop the folder/uptime detail. |
| Files (`letter f`) | Tree with a size micro-bar and age, git letters; inline syntax preview at ≥100 cols, where a line cut at the edge ends in an accent `›`. The footer teaches `j k` there (and `␣ preview` only below 100 cols). |
| Palette (`/`) | Jump to anything: devices, sessions and actions, each with the keys that reach it directly (so the palette teaches the shortcuts). |
| Keys / secrets | Masked secret cards keyed `a s d g` (a slot key opens that card's editor, so adding the Cloudflare key is `K a`, paste, `⏎`); missing ones get a `⏎ add` or `⏎ connect` button. Crumb `cx › keys`. |
| Onboarding | Viewer pill alone on an empty orbit, the wrapped one-sentence explanation, a `⏎ Connect Cloudflare` button, three plain numbered steps (`1.`, not keycaps: they are not keys), a breathing row, then `j` join code and `esc` plain SSH. |
| Join | Half-block QR (dark on light, always), the short code as keycaps, the one-liner wrapped over up to three rows (readable to retype where clipboard escapes fail) and an expiry bar. |
| Edge editor | Crumb `cx › t ⇄ i link`. The schematic with the edited link drawn heavy (`━┃┏┓`) at full mesh hue and the rest faded; a phone without room for the map gets the link alone (`t ━━━ ⇄ key ━━━ i`). The sheet has one segmented control per direction: the opener's outbound lane first with `k p o`, the way back with `K P O`, so no arrow keys are needed. A key applies at once, `u` undoes; the wrapped line states the live effect, and the rtt history follows without a gap. |
| Help | Crumb `cx › help`. A drawn keyboard per layer (hub: device keys with status and rtt when the keycaps are wide enough, `e link`, `/ find`; in a device: `ses 1..8`, verbs), keycap width chosen so the staggered rows fit (80 columns included), a legend of device names and a key legend, both on fixed column grids. Phone: a list. |

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
- **Lines** are box-drawing strokes with rounded elbows (`─ │ ╭ ╮ ╰ ╯`, ASCII `- | +`),
  coloured by network (mesh hue per mesh, `lan` teal, direct ssh `info`, down `err`).
  **Line style is the access policy**, readable without colour: solid when key access is
  allowed, dashed `┄ ┆` when only a password is, dotted `┈ ┊` when nothing is (or the
  link is down). A trunk shared by two links takes the brighter, more solid style.
- **No crossings by construction.** Links leave the pill from its top edge (targets
  above) or bottom edge (targets below), farthest target first, so elbows nest; a target
  on the pill's row gets a straight line; children leave their parent on a short trunk.
- **One chip per link** on its own last straight run, which is also its tap target. It
  says the policy and the rtt in the richest form that fits: `⇄ key 14ms`, `key 0.6ms`
  (two-way and equal), `→ pass 1.2ms`, `⇄ key`, `⇄`; down links say `✕ down`. Every chip
  shows its word whenever the run allows it, not only on the selected link.
- **Motion** only where something moves: a link with an active transfer carries one bright
  3-cell `━━━` packet on the unbroken line, on straight cells clear of the chip, heading
  for the receiving end. A live build advances it by `caps.frame` (2-4 Hz, synchronized
  output) only while the transfer runs; there are no gaps in a still frame.
- **Selection** puts the bar and tint on the node and lights its links plus its route back
  to the root (`t → c → b`). With the sessions zone focused, the schematic marks the
  selected session's device, so the two zones point at each other.
- **Node cards**: status dot, identity keycap, bold name, mesh squares; with height a
  second row with the cpu sparkline (M+) or cpu % (S), temperature and session pips.
  Label richness is chosen per frame (two-row, name, keycap only); before falling back to
  keycaps the names shorten to 10 and 8 characters and the pill gives up its name first.
- **Mesh hulls**: neighbours in one column on one side that share a mesh sit on a faint
  tint of the mesh hue (truecolor/256), so membership reads as a region.
- Peers in one column (mesh peer links) are not drawn on the hub; the edge editor keeps
  both ends of an edited peer link on one side and draws it as a bracket outside the
  column. They are listed in the device view.
- Overflow per column becomes a `+N more` (`+N` on phones) target that opens the palette.
- Touch layouts (< 70 columns): node, pill and `+N` targets are at least 3 rows by 7
  columns.

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
| mesh hues | mesh 1, mesh 2 | `#c49cf0`, `#7fd8e8` | `#7048c0`, `#0a8090` | 177, 116 | magenta, brightcyan | `*` |
| `id.0..7` | device keycaps | spec hues | spec hues | spec | rotated 8 | reversed bold letter |

Session screens are mostly authored for dark terminals. On a light theme, dark screen
backgrounds are stretched to full saturation and then mixed 16 % into white (a diff row
becomes about `#d9ffe4` / `#ffdcdc`, like GitHub light), and pale inks are deepened 42 %
toward black (`tiles::adapt`). In 16 colours, named screen colours stay themselves, rgb
inks map by hue (a nearest-distance match turned muted reds grey), and coloured
backgrounds become ink of their hue, so `+` rows stay green and `-` rows red without a
loud block (`tiles::degrade`).

Glyphs, with ASCII fallbacks: status `◉ ● ◐ ✕ ◌` / `@ * ~ x ?`; session state `◆ ● ○ ·` /
`! * o -`; policy `⇄ → ← ⊘` / `<> -> <- x`; line styles `─ ┄ ┈` / `- . .`; mesh squares
`▄` and session pips `▖` / `*` and `#` (block elements, which kitty, foot, ghostty and
alacritty draw as geometry, so they keep their size in DejaVu or any fallback font: the
former `◆` badge shrank to a dot there); keycaps are coloured cells in every tier
(`REVERSED|BOLD` without colour); link lines become `- | +`; `⎿` is `L` and `✻` is `*`;
eighth-block charts become `_ : = #`; the minimap is omitted in ASCII and NO_COLOR (the
text tail carries the tile); the QR is drawn with `▀▄█` even in NO_COLOR and dropped
only in ASCII, where the code and one-liner remain. Fixture text is transliterated in
ASCII (`asciify`), so the lint stays at 0 warnings.

## 5. Layout per size

Width classes as in the spec: XS < 60, S 60-99, M 100-159, L ≥ 160. Touch layouts are
everything under 70 columns (40x20 and 60x30 phones).

| | touch: 40x20, 60x30 | S 80x24 | M 120x40 | L 200x60 |
|---|---|---|---|---|
| Top bar | `cx`, crumb (earlier device crumbs as keycaps), `● 6/7 ▲1` | + transfer % and `?` | + transfer name and rate | + the alert headline (`▲ squirtle-jetson unreachable`), placed before the transfer |
| Schematic | keycap nodes with policy chips | names (≤ 10), chips with rtt | two-row node cards, hulls, chips with rtt | same, more air |
| Below it | device rows (two rows: name, status, rtt; cpu trend, %, temp, pips) and a two-row "needs you" strip | — | — | — |
| Sessions | (in device rows and the strip) | 3 per row, 2 rows | 3 per row, 2 rows + ghost | 4 per row, taller tiles |
| Device view footer | `esc back`, `f files`, `⌫ stop`, `e links` (tiles attach on tap) | 6 chips | 8 chips | same |
| Hub footer | `/ find`, `+ invite`, `K keys`, `? help` (two-row targets over a spacer row) | 6 chips | 8 chips | same |

One blank row separates NETWORK's header from the map and the map from SESSIONS at every
size; at ≥ 30 rows another one sits above the footer. Session tiles share one width and
one 2-column gutter; rows are equal height. Thumbnails show the last non-blank lines of
the screen and drop shared indentation when narrower than the screen; the agent's own
reversed key hints (`y Yes`) are underlined in thumbnails so they never look like cx
keycaps. A cut never shows two ellipses. Under 20x6 the frame shows only `cx`.

## 6. Interaction model

Hub keys are non-letters plus `e`, so every other letter can be a device (a real build
never assigns `e` as a hotkey). Inside a device, letters are verbs and slots; the home row
holds the slots (`a s d g h j k l`; `f` stays files). Any list inside a screen uses the
same home-row slots (secrets cards are `a s d g`). Peek has no inline typing, so hub
letters work there unchanged. Uppercase letters are free on the hub, so the two global
panels use them.

| Key | Hub | Device | Peek | Mouse / touch |
|---|---|---|---|---|
| device letter | open that device | — | as on the hub | tap node, row or chip |
| `a s d g h j k l` | — | attach that session | (after a letter) attach | tap thumbnail |
| `c` / `x` / `n` | — | new claude / codex / shell here | — | tap ghost-tile option |
| `e` + letter | edit the link from this device to that one | edit the link to that peer | — | tap the link chip |
| `f` | — | files | — | tap `f files` |
| `⌫` | — | stop the selected session (inline `y` / `n`) | stop this one | tap `⌫ stop` |
| `⏎` | open selection (device or session) | attach selection | attach | tap selection |
| `␣` | peek selected session | peek | — | tap `␣ peek` |
| `Tab` | next zone (network, sessions) | next zone | next peek | tap a zone |
| `↑↓←→` | move spatially | move | — | — |
| `/` | find anything | find | — | tap `/ find` |
| `+` / `K` / `?` | invite / keys / help | | | footer chips |
| `Esc` | — | back one level | back to the device | tap the crumb or `esc back` |
| `` ` `` | — | to the hub from any depth | to the hub | tap `cx` |
| `Ctrl+\` | — | detach from an attached session | | `⏏ detach` in the tmux status |
| `k p o` / `K P O`, `u` | edge editor: outbound / inbound key, password, off, applied at once; undo | | | tap segment |
| `y` / `Y` | copy (code, path, secret) / copy the alternate (command) | | | tap the code or line |

`/` always searches what you are looking at. Files move with `j k` (and arrows), `l`/`h`
expand and collapse. Copying uses OSC 52; inside tmux with `set-clipboard off` (checked
once with `tmux show -gv set-clipboard`) or on a terminal that cannot confirm it, the toast
says "copied, if your terminal allows clipboard access" instead of a plain "copied".

Decisions where reviewers disagreed (recorded, not asked):
- **Launch focus stays on the network**, as the brief's flow says (move to a node and
  Enter, or press its letter). The approval that needs you is `Tab ⏎` (its tile ranks
  first) or its address `t a`, then the agent's `y`: three keys either way. Attention
  verbs answering approvals from the hub were left out: hub letters are devices (`n` is nas).
- **`⌫` is stop** behind an inline confirm, so a reflexive "back" costs nothing.
- In the device layer `j` and `k` are slots, not vim motion; arrows still move.
- `Ctrl+\` replaces SIGQUIT inside attached shells (spec 8.1); on ISO layouts it may need
  AltGr, which is why the tmux status chip exists. `` ` `` is a dead key on some ISO
  layouts, so a double `Esc` (two presses within 400 ms) also jumps to the hub from any
  depth, and `cx` is always tappable.

### Keystrokes for the top tasks (from the hub)

| Task | Keys | Count | Path |
|---|---|---|---|
| Open a session on another device | `i` `a` | 2 | tap tile (1) |
| Answer the approval that needs you | `Tab` `⏎` `y` or `t` `a` `y` | 3 | tap the strip, then Yes (2) |
| Re-attach after detaching | `⏎` | 1 | tap the tile |
| Start a new claude session on innovation | `i` `c` | 2 | tap `i c claude` on the ghost tile (1) |
| Stop a session | `i` `⌫` `y` (slot a selected) | 3 | tap `⌫ stop`, tap `y` |
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
| Attach a session | high | slot `a s d …` | 1 (2 from hub) | home row, strong fingers | most frequent action, best keys; one address everywhere |
| Open / re-attach selection | high | `⏎` | 1 | right pinky, large key | universal |
| Detach | high | `Ctrl+\` | 1 chord | left pinky + right pinky | the only key cx steals inside sessions; also a tap chip |
| Answer what needs you | high | `Tab ⏎` | 2 | left pinky, right pinky | the tile ranks first; no letter collides with devices |
| Peek | med | `␣` | 1 | thumb | cheapest key for a look without commitment |
| Next peek | med | `Tab` | 1 | left pinky | triage without leaving peek |
| New claude / codex / shell | med | `c` / `x` / `n` | 1 (2 from hub) | left middle/ring, right index | mnemonic; `c x` adjacent |
| Switch zone | med | `Tab` | 1 | left pinky | two zones on the hub, so one press toggles |
| Find anything | med | `/` | 1 | right pinky | one meaning: search what you see |
| Files of a device | med | `f` | 1 | left index, home row | mnemonic, home row |
| Back / hub | med | `Esc` / `` ` `` | 1 | left pinky | one level vs. all the way; crumbs are tappable |
| Stop a session | low | `⌫` then `y` | 2 | right pinky reach, then right index | destructive, so a far key plus confirm |
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
- **ASCII**: every glyph has a one-cell substitute (section 4); lines are `- | +`; the QR
  is dropped (code and one-liner remain). The capture lint reports 0 non-ASCII cells.
- **Ambiguous-width terminals** (Blink with CJK fonts, `ambiwidth=double`): status and
  policy glyphs, eighth-block charts, `▍ ▄ ▖`, `•` masks, `°` and box drawing are all
  East-Asian-Ambiguous. A real build probes once at startup (print `●` and `─`, read the
  cursor with `CSI 6n`) and, when either is two columns wide, renders with the whole ASCII
  glyph set (`unicode = false`: charts `_ : = #`, masks `*`, `°` as `C`, bars `>`, lines
  `- | +`) while keeping colour. That is the existing ASCII tier, which the lint keeps at 0
  non-ASCII cells. The lab cannot emulate double-width ambiguous cells, so it is not
  captured.
- **Optional enhancement**: kitty/sixel images could show a real downscaled screenshot in
  session tiles; the cell tail is the shipped fallback and is what the captures show.

Captures: `tests/lab.sh mission` (dark/light truecolor, dark-16, ascii) plus
`CX_LAB_VARIANTS=dark-256,dark-nocolor,light-16` and a fallback-font check with
`CX_LAB_FONT="DejaVu Sans Mono"` (status glyphs shrink, `✕` becomes `X`, still legible).

## 8. Integration notes

- Rendering never dispatches; every target is recorded as `LabTarget` with the action the
  real UI would run (`device:<id>`, `session:<dev>:<id>`, `edge:<id>`, `new:<dev>:<provider>`,
  `policy:<from>:<to>:<value>`, `palette:<scope>`).
- The schematic needs per-device cpu history and per-edge rtt history (already in the
  spec's additive data, section 14) and the session screen tail the preview already has.
- The transfer pulse is driven by a frame counter; a real build would redraw the
  schematic at a low rate (2-4 Hz) only while a transfer runs and the hub is visible.
- `map::draw(cx, buf, band, root, depth, sel)` serves the hub (viewer, 3 hops), the
  device view (that device, 1 hop) and the edge editor. The left half is the right half
  mirrored (`Plan::sx`), so there is one router and one node painter.
- The edge editor id names the opener first (`innovation-tranquility` from `i e t`); the
  real UI resolves either order to the stored edge.
- Fixture fix in `src/ui/lab/state.rs`: the Claude composer's middle row was one cell
  wider than its border, which showed as a stray `│` in every concept's peek.
