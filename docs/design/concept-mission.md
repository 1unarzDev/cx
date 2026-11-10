# Concept: mission control

Code: `src/ui/lab/concepts/mission/` (`mod.rs` render context and shared helpers,
`map.rs` the network schematic and its line router, `tiles.rs` device tiles and session thumbnails,
`views.rs` one function per screen). Render: `tests/lab.sh mission`.

A calm, dense-but-airy dashboard: btop refined by a Swiss designer. The start screen is
an instrument panel. A network schematic drawn like a transit map sits on top, a strip
of device tiles with micro-charts sits under it, and the sessions fill the rest as live
thumbnails you recognise by how they actually look. Each screen is one grammar: **a device letter, then
a verb or a session slot**.

## 1. Principles

1. **The schematic is the hub.** You do not pick from a list of devices; you look at the
   network and press the letter printed on the node. Every node, edge chip, tile and
   chip on screen is also a click/tap target that maps to the same action as its key.
2. **Recognise, don't read.** A session tile shows the real last informative lines of its
   screen (agent status, the approval prompt, the failing test) in the screen's own
   colours, in every colour tier. Two Claude sessions look different because their
   screens do.
3. **Evidence on the surface.** Every device shows how its status is known: rtt or the
   age of the last sighting, cpu history, temperature, session pips. Unknown is `?`,
   never green.
4. **Tints, not boxes.** Panels are surfaces (background tints). Each zone opens with a
   spaced small-caps label followed by a faint rule (`▍N E T W O R K ──────`), with one
   blank row above it when the height allows. Boxes are used only for the dashed "new
   session" ghost tile. Without tints (16 colours, NO_COLOR) tiles are separated by a
   faint `│` gutter.
5. **Focus is a colour bar plus a tint.** The focused zone's label gets an accent bar
   `▍` and accent colour; its selected item gets `accent.bg` plus a left `▌`. Unfocused
   zones keep a dim tint at their selection, so you see where you return to.
6. **Every action flows into the next.** A letter lands you on the device with its
   sessions under the home row; a slot key attaches straight away (no preview chain).
   Detach (`Ctrl+\`) returns to the tile you left, still selected, so `⏎` re-attaches.
7. **One address per session.** A session is always shown as its device keycap plus its
   slot keycap (`i` `a`) on hub tiles, device tiles, phone rows, the peek filmstrip, the
   palette and help. That is exactly what you type from the hub. There are no hub digits.

## 2. Screens

| View | What it shows |
|---|---|
| Hub | Schematic (about 40 % of the height, capped at what its tallest column needs), the device tiles strip, then session thumbnails on one uniform grid ranked by attention; a free cell is the "new session" ghost tile. Phone: compact schematic, one row per device, and a "needs you" strip above the footer. |
| Device (`letter`) | Header with identity (as wide as it needs), evidence, mesh badges, cpu chart and gauges; session thumbnails keyed `a s d g h j k l`, only as tall as their screens; the ghost tile "new on <device>" (`c` claude, `x` codex, `n` shell) sits in the same row, narrow, so one session keeps its screen nearly 1:1; under them LINKS: the same schematic, one hop from this device, with every policy chip shown. |
| Session peek (`␣`) | The screen 1:1 on a deeper surface, a meta line (address, state, provider, folder, uptime, clients, activity trend) and a filmstrip of the other sessions, each with its address keys. `⏎` attaches, `⌫` stops. There is no inline typing: Esc stays free for the agent (Claude and Codex use it to interrupt) and `⏎` attaching costs one key. |
| Files (`letter f`) | Tree with a size micro-bar and age, git letters; inline syntax preview at ≥100 cols. |
| Palette (`/`) | Jump to anything: devices, sessions and actions, each with the keys that reach it directly (so the palette teaches the shortcuts). |
| Keys / secrets | Masked secret cards (source, age, what uses them); missing ones get a `⏎ add` or `⏎ connect` button. |
| Onboarding | Viewer pill alone on an empty orbit, a `⏎ Connect Cloudflare` button, three numbered steps, then `j` join code and `esc` plain SSH, one per row. The top bar shows no network status (there is none yet). |
| Join | Half-block QR (dark on light, always), the short code as keycaps, the one-liner and an expiry bar. |
| Edge editor | Crumb `cx › t ⇄ i link`. The schematic with the edited link lit and the rest dimmed but visible, over a sheet (ending a row above the footer) with one segmented control per direction (`k` key, `p` password, `o` off). A key press applies at once, `u` undoes; the wrapped consequence line states the live effect (open sessions crossing the link). Nothing describes a change nobody made. |
| Help | A drawn keyboard per layer (hub: device keys carry status and rtt; in a device: slots, verbs), a legend with full device names, and the keys that mean the same everywhere. Phone: a list. |

## 3. The schematic

A left-to-right transit map by hop count from the root (this device on the hub, the
focused device in the device view). The root is a one-row tinted pill (half-block caps;
`( … )` without tints). Devices it reaches over a routed link form column 1, grouped by
mesh; devices reached through those form column 2, ordered by parent; at most three
columns. Jump edges are not drawn (the route is visible through the hop).

- **Lines** are solid box-drawing strokes with rounded elbows (`─ │ ╭ ╮ ╰ ╯`, ASCII
  `- | +`), one router for every tier. Braille curves were dropped: in real fonts they
  read as dotted stair-steps, and they collapsed into slope noise in ASCII.
- **No crossings by construction.** Links leave the pill from its top edge (targets
  above) or bottom edge (targets below); the farthest target bends first and the nearest
  bends last, so the elbows nest. A node's children leave on a short trunk to the right of
  its column's widest label, again farthest first. A target on the pill's row gets a
  straight line.
- **One chip per link**, centred on its own last straight run (no other link uses that
  run, so chips never overlap a different edge): `⇄ → ← ⊘ ✕` (glyph) on resting links,
  `⇄ key`, `→ pwd`, `⊘ off` on lit ones. Every chip is the link's tap target.
- **Colour** is the network: mesh hue per mesh, `lan` teal, direct ssh `info`, down links
  `err`. Lit links are full strength; resting links 78 % (90 % on light); in the edge
  editor the others drop to 38 % but stay visible.
- **Motion** only where something moves: a link with an active transfer carries heavy
  `━` pulses in the transfer direction, advanced by `caps.frame` (2-4 Hz in a live build).
- **Selection** tints the node (reverses or underlines its name without tints) and lights
  its links plus its route back to the root (`t → c → b`), answering "how do I reach it".
- **Mesh hulls**: neighbours in one column that share a mesh sit on a faint tint of the
  mesh hue (truecolor/256, not in keycap-only mode), so mesh vs non-mesh reads at a glance.
- Peers in one column (mesh peer links) are not drawn on the hub; the edge editor draws
  the edited one as a bracket right of the column. They are listed in the device view.
- **Nodes**: status dot, identity keycap, bold name; with height a second row with
  evidence (rtt or age), mesh diamonds and a cpu sparkline. Label richness is chosen per
  frame (two-row, one-row, name, keycap only) so that ≥ 9 columns remain between columns
  for a chip; gaps grow up to 26 columns and the drawing is centred.
- Overflow per column becomes a `+N more` (`+N` on phones) target that opens the palette.
- Phones: node and pill hit boxes include the row above and below.

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
`! * o -`; policy `⇄ → ← ⊘` / `<> -> <- x`; keycaps are coloured cells in every tier
(`REVERSED|BOLD` without colour); link lines become `- | +`; `⎿` is `L` and `✻` is `*`;
eighth-block charts become `_ : = #`; the minimap is omitted in ASCII and NO_COLOR (the
text tail carries the tile); the QR is drawn with `▀▄█` even in NO_COLOR and dropped
only in ASCII, where the code and one-liner remain. Fixture text is transliterated in
ASCII (`asciify`), so the lint stays at 0 warnings.

## 5. Layout per size

Width classes as in the spec: XS < 60, S 60-99, M 100-159, L ≥ 160.

| | XS 40x20, 60x30 phone | S 80x24 | M 120x40 | L 200x60 |
|---|---|---|---|---|
| Top bar | `cx`, crumb, `● 6/7 ▲1` | + transfer % and `?` | + transfer name and rate | + newest notice text |
| Schematic | keycap-only nodes, glyph chips | names, chips on lit links | two-row nodes with rtt and cpu trend, hulls | same, more air |
| Devices | one row per device: keycap, name, status, evidence, cpu trend, session pips | 4 tiles + `4/7 ›` | all 7 tiles (16 cols) | all 7 tiles (≥ 22 cols) |
| Sessions | "needs you" strip (address, title) above the footer | 3 per row | 3 per row, 2 rows + ghost | 4 per row |
| Device view | rows (2 lines per session), provider buttons, keycap fan | tiles + ghost + links fan | same | same |
| Footer | `/ find`, `+ add`, `K keys`, `? help` (two-row tap targets over a spacer row) | 6 chips | 8 chips | same |

Session tiles share one width and one 2-column gutter per grid; rows are equal height.
Thumbnails show the last non-blank lines of the screen (blank lines go first when the
tile is short) and drop the indentation those lines share when the tile is narrower than
the screen. A cut never shows two ellipses (`fit` drops a trailing `…` first).
Under 20x6 the frame shows only `cx`.

## 6. Interaction model

Hub keys are non-letters, so every letter can be a device. Inside a device, letters are
verbs and slots; the home row holds the slots (`a s d g h j k l`; `f` stays files). Any
list inside a screen uses the same home-row slots (secrets cards are `a s d g`). To hop
to another device from inside one: `esc` then its letter, or tap it in the links fan.
Uppercase letters are free on the hub, so the two global panels use them.

| Key | Hub | Device | Mouse / touch |
|---|---|---|---|
| device letter | open that device | — | tap node, tile or row |
| `a s d g h j k l` | — | attach that session | tap thumbnail |
| `c` / `x` / `n` | — | new claude / codex / shell here | tap ghost-tile option |
| `f` / `e` | — | files / links of this device (`e` then a peer letter opens that link) | tap footer chip / link chip |
| `⌫` | — | stop the selected session (inline `y` / `n` confirm) | tap `⌫ stop` |
| `⏎` | open selection (device or session) | attach selection | tap selection |
| `␣` | peek selected session | peek | tap `␣ peek` chip |
| `↑↓←→` | move spatially in the schematic, or in a zone | move | — |
| `Tab` | next zone (network, devices, sessions) | next zone | tap a zone |
| `/` | find anything (palette) | find | tap `/ find` |
| `+` | invite a device (join code) | — | tap `+ invite` |
| `K` | keys and secrets | — | tap `K keys` |
| `?` | keyboard map | keyboard map | tap `?` |
| `Esc` | — | back one level | tap the crumb |
| `` ` `` | — | straight back to the hub from any depth | tap `cx` |
| `Ctrl+\` | — | detach from an attached session | tap `⏏ detach` in the tmux status |
| `k` / `p` / `o`, `u` | edge editor: key / password / off for the focused direction, applied at once; undo | | tap segment |
| `y` / `Y` | copy (code, path, secret) / copy the alternate (command) | | tap the code or line |

`/` always searches what you are looking at: from the hub that is everything (devices,
sessions, actions); in files it filters the tree; in a preview it finds in the file.
Files move with `j k` (and arrows), `l`/`h` expand and collapse.

Tradeoffs (recorded, not asked): in the device layer `j` and `k` are slots, not vim
motion; arrows still move. `Ctrl+\` replaces SIGQUIT inside attached shells (spec 8.1);
on ISO layouts it may need AltGr, which is why the tmux status chip exists.

### Keystrokes for the top tasks (from the hub)

| Task | Keys | Count | Path |
|---|---|---|---|
| Open a session on another device | `i` `a` | 2 | tap tile (1) |
| Answer the approval that needs you | `t` `a` (shown on the tile and the phone strip) | 2 | tap the strip row (1) |
| Re-attach after detaching | `⏎` | 1 | tap the tile |
| Start a new claude session on innovation | `i` `c` | 2 | tap node, tap `c claude` |
| Stop a session | `i` `a`-select then `⌫` `y`, or `⌫` `y` in peek | 3-4 | tap `⌫ stop`, tap `y` |
| Edit an edge policy (innovation → tranquility off) | `i` `e` `t` `o` | 4 | tap the link chip, tap `o off` (2) |
| Send a join code | `+` `y` | 2 | tap `+ invite`, tap the code |
| Add a Cloudflare key | `K` `a` paste `⏎` | 2 + paste | tap `⏎ connect` on the onboarding or keys card |
| Browse and preview a file | `i` `f` `/` `tr` `⏎` | 5 | tap node, `f files`, tap file |
| Back to the hub from a file preview | `` ` `` | 1 | tap `cx` |

### Ergonomics table

| Action | Frequency | Keys | Count | Key positions / fingers | Rationale |
|---|---|---|---|---|---|
| Go to a device | high | its letter | 1 | wherever the letter is; letters are the device's initial | mnemonic beats position; printed on every node, tile and row |
| Attach a session | high | slot `a s d …` | 1 (2 from hub) | home row, strong fingers | most frequent action, best keys; one address everywhere |
| Open / re-attach selection | high | `⏎` | 1 | right pinky, large key | universal |
| Detach | high | `Ctrl+\` | 1 chord | left pinky + right pinky | the only key cx steals inside sessions; also a tap chip |
| Peek | med | `␣` | 1 | thumb | cheapest key for a look without commitment |
| New claude / codex / shell | med | `c` / `x` / `n` | 1 (2 from hub) | left middle/ring, right index | mnemonic; `c x` adjacent |
| Switch zone | med | `Tab` | 1 | left pinky | no chord |
| Find anything | med | `/` | 1 | right pinky | vim/web convention; one meaning: search what you see |
| Files of a device | med | `f` | 1 | left index, home row | mnemonic, home row |
| Back / hub | med | `Esc` / `` ` `` | 1 | left pinky | one level vs. all the way; crumbs are tappable |
| Stop a session | low | `⌫` then `y` | 2 | right pinky reach, then left index | destructive, so a far key plus confirm |
| Edit a link | low | `e` then peer letter | 2 | left middle | mnemonic "edges" |
| Policy | low | `k p o`, `u` | 1 | right hand; `u` right index | mnemonic key / password / off; applied at once with undo |
| Copy | low | `y` / `Y` | 1 | right index | vim "yank"; never collides with `c` claude |
| Invite | low | `+` | 1 chord | shift + right pinky | rare; "add"; also a footer chip |
| Keys and secrets | low | `K` | 1 chord | shift + right middle | rare; uppercase is free on the hub |
| Help | low | `?` | 1 chord | shift + right pinky | convention |

## 7. Degradation

- **256 colours**: identical layout; blends resolve to the nearest xterm index.
- **16 colours**: no tints. Tiles separate by a faint `│` gutter and zones by their rule;
  selection is a reversed name (underlined when the zone is unfocused); the pill becomes
  `( … )` in accent; charts keep the heat ramp; screen inks keep their hue (section 4).
- **NO_COLOR**: attributes only. Keycaps are reversed bold letters; selection is reverse;
  thumbnails keep dim/bold.
- **ASCII**: every glyph has a one-cell substitute (section 4); lines are `- | +`; the QR
  is dropped (code and one-liner remain). The capture lint reports 0 non-ASCII cells.
- **Ambiguous-width terminals** (Blink with CJK fonts, `ambiwidth=double`): `● ◆ ○ ▲ ◉ ◐
  ✕ …` are East-Asian-Ambiguous. A real build probes once at startup (print `●`, read the
  cursor with `CSI 6n`) and switches to the ASCII glyph set for status and badges when it
  is two columns wide; box-drawing lines are narrow in practice. The lab renderer cannot
  emulate double-width ambiguous cells, so this is not captured.
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
  device view (that device, 1 hop) and the edge editor.
- Fixture fix in `src/ui/lab/state.rs`: the Claude composer's middle row was one cell
  wider than its border, which showed as a stray `│` in every concept's peek.
