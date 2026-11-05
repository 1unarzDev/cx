# Concept: mission control

Code: `src/ui/lab/concepts/mission/` (`mod.rs` render context and shared helpers,
`map.rs` the network schematic, `tiles.rs` device tiles and session thumbnails,
`views.rs` one function per screen). Render: `tests/lab.sh mission`.

A calm, dense-but-airy dashboard: btop refined by a Swiss designer. The start screen is
an instrument panel. A network schematic sits on top, a strip of device tiles with
micro-charts sits under it, and the sessions fill the rest as large live thumbnails you
recognise by how they actually look. Each screen is one grammar: **a device letter, then
a verb or a session slot**.

## 1. Principles

1. **The schematic is the hub.** You do not pick from a list of devices; you look at the
   network and press the letter printed on the node. Every node, edge chip, tile and
   chip on screen is also a click/tap target that maps to the same action as its key.
2. **Recognise, don't read.** A session tile shows the real last informative lines of its
   screen (agent status, the approval prompt, the failing test) in the screen's own
   colours, beside a braille minimap of the whole screen (diff blocks, log colours,
   indentation). Two Claude sessions look different because their screens do.
3. **Evidence on the surface.** Every device shows how its status is known: rtt or the
   age of the last sighting, cpu history, temperature, session pips. Unknown is `?`,
   never green.
4. **Tints, not boxes.** Panels are surfaces (background tints) with spaced small-caps
   labels (`N E T W O R K`). Boxes are used only for the "new session" ghost tile and for
   the viewer pill when tints are unavailable.
5. **Focus is a colour bar plus a tint.** The focused zone's label gets an accent bar
   `▍` and accent colour; its selected item gets `accent.bg` plus a left `▌`. Unfocused
   zones keep a dim tint at their selection, so you see where you return to.
6. **Every action flows into the next.** A letter lands you on the device with its
   sessions under the home row; a slot key attaches straight away (no preview chain).
   Detach (`Ctrl+\`) returns to the tile you left.

## 2. Screens

| View | What it shows |
|---|---|
| Hub | Schematic (top ~42 %), device tiles strip, session thumbnails ranked by attention (needs you, working, running, idle). Phone: compact schematic plus one row per device. |
| Device (`letter`) | Header with identity, evidence, mesh badges, cpu chart and gauges; session thumbnails keyed `a s d g h j k l`; a ghost tile "new on <device>" with `c` claude, `x` codex, `n` shell; links column (peer, policy arrow, network, rtt trend). |
| Session peek (`␣`) | The screen 1:1 on a deeper surface, a meta line (slot, state, provider, folder, uptime, clients, activity trend) and a filmstrip of the other sessions. `⏎` attaches, `i` types inline. |
| Files (`letter f`) | Tree with a size micro-bar and age, git letters; inline syntax preview at ≥100 cols. |
| Palette (`/`) | Jump to anything: devices, sessions and actions, each with the keys that reach it directly (so the palette teaches the shortcuts). |
| Keys / secrets | Masked secret cards (source, age, what uses them); missing ones get a `⏎ add` or `⏎ connect` button. |
| Onboarding | Viewer pill alone on an empty orbit, a `⏎ Connect Cloudflare` button and three numbered steps. |
| Join | Half-block QR (dark on light, always), the short code as keycaps, the one-liner and an expiry bar. |
| Edge editor | The schematic with only the edited link lit, over a sheet with one segmented control per direction (`k` key, `p` password, `o` off) and a consequence line. |
| Help | A drawn keyboard with each key labelled by role, per layer (hub, device). Phone: a list. |

## 3. The schematic

A left-to-right flow by hop count from this device. The viewer is a tinted capsule on
the left (half-block caps make it read as a pill). Devices it reaches over a routed
link form column 1, grouped by mesh; devices reached through those form column 2,
ordered by parent to avoid crossings; at most three columns. Jump edges are not drawn
(the route is visible through the hop). Links are cubic Bézier S-curves on a braille
canvas (2x4 dots per cell) with horizontal tangents, so the fan out of the viewer reads
like a schematic, not a TUI tree. Links between devices in the same column arc out to
the right of their labels.

- **Colour** is the network: mesh hue per mesh, `lan` teal, direct ssh `info`, down links
  `err` with a `✕` chip at the midpoint.
- **Flow** is policy: brighter "packets" are drawn along each direction access is allowed
  (`key` or `password`). A one-way link has packets going one way; a link with both
  directions off is drawn sparse with no packets. Frames advance the packet phase
  (`caps.frame`), so the static capture already shows direction and a live build
  animates it. Links with an active transfer have denser packets.
- **Nodes** are status dot, identity keycap (the hotkey in the device hue), bold name, then
  (when there is height) a second row with evidence (rtt or age), mesh diamonds and a cpu
  sparkline. Label richness is chosen per frame: two-row with evidence, one-row with
  evidence, name only, keycap only, whichever fits with ≥8 columns between columns.
- **Selection** tints the node (or reverses its name without tints) and lights its links;
  at ≥100 cols the lit links show their policy chip (`⇄ key`, `→ key`, `⊘ off`).
- A faint dot grid (every 4 cols, 2 rows) gives the band depth in truecolor/256 only.
- Overflow per column becomes a `+N more` target that opens the palette on devices.

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
backgrounds become pale tints of their hue (22 % toward white) and pale inks are
deepened (42 % toward black), so diffs stay green/red and legible (`tiles::adapt`).

Glyphs, with ASCII fallbacks: status `◉ ● ◐ ✕ ◌` / `@ * ~ x ?`; session state `◆ ● ○ ·` /
`! * o -`; policy `⇄ → ← ⊘` / `<> -> <- x`; keycaps are coloured cells in every tier
(`REVERSED|BOLD` without colour); braille links become slope strokes `- | / \ .`;
eighth-block charts become `_ : = #`; the minimap is omitted in ASCII and NO_COLOR (the
text tail carries the tile); the QR is drawn with `▀▄█` even in NO_COLOR and dropped
only in ASCII, where the code and one-liner remain. Fixture text is transliterated in
ASCII (`asciify`), so the lint stays at 0 warnings.

## 5. Layout per size

Width classes as in the spec: XS < 60, S 60-99, M 100-159, L ≥ 160.

| | XS 40x20, 60x30 phone | S 80x24 | M 120x40 | L 200x60 |
|---|---|---|---|---|
| Top bar | `cx`, crumb, `● 6/7 ▲1` | + transfer % and `?` | + transfer name and rate | + newest notice text |
| Schematic | keycap-only nodes, 1-row pill | names, 3-row pill, chips hidden | two-row nodes with rtt and cpu trend, policy chips on lit links | same, more air (gaps capped at 34 cols) |
| Devices | one row per device: keycap, name, status, evidence, cpu trend, session pips | 4 tiles + `4/7 ›` | all 7 tiles (16 cols) | all 7 tiles (≥22 cols) |
| Sessions | via the device letter | 3 thumbnails + `3 of 5 sessions › more` | 3x2 balanced grid | 4 per row, live tail + minimap |
| Device view | rows (2 lines per session) + provider chips | thumbnails + ghost tile | + links column | same |
| Footer | 3 chips | 5 chips | 7 chips incl. `+ invite`, `? keys` | same |

Tiles balance their grid so the last row is never a lone tile (`cols = ceil(n / rows)`).
Under 20x6 the frame shows only `cx`.

## 6. Interaction model

Hub keys are non-letters, so every letter can be a device. Inside a device, letters are
verbs and slots. The home row holds the slots (`a s d g h j k l`; `f` stays files).

| Key | Hub | Device | Mouse / touch |
|---|---|---|---|
| device letter | open that device | — | tap node, tile or row |
| `a s d g h j k l` | — | attach that session | tap thumbnail |
| `c` / `x` / `n` | — | new claude / codex / shell here | tap ghost-tile option |
| `f` / `e` | — | files / links of this device | tap footer chip / link row |
| `⏎` | open selection (device or session) | attach selection | tap selection |
| `␣` | peek selected session | peek | tap `␣ peek` chip |
| `↑↓←→` | move spatially in the schematic, or in a zone | move | — |
| `Tab` | next zone (network, devices, sessions) | next zone | tap a zone |
| `/` | palette (devices, sessions, actions) | palette | tap `/ find` |
| `+` | invite a device (join code) | — | tap `+ invite` |
| `?` | keyboard map | keyboard map | tap `?` |
| `Esc` | — | back to hub | tap `cx` or crumb |
| `Ctrl+\` | — | detach from an attached session | tap `⏏ detach` in the tmux status |
| `k` / `p` / `o` | edge editor: key / password / off for the lane under the cursor | | tap segment |

Tradeoff (recorded, not asked): in the device layer `j` and `k` are slots, not vim
motion; arrows still move. Slots on the home row are the more frequent action.

### Keystrokes for the top tasks (from the hub)

| Task | Keys | Count | Path |
|---|---|---|---|
| Open a session on another device | `i` `a` | 2 | tap node, tap tile |
| Start a new claude session on innovation | `i` `c` | 2 | tap node, tap `c claude` |
| Edit an edge policy (innovation → tranquility off) | `i` `e` `t` `↓` `o` `⏎` | 6 | tap the link chip on the schematic, tap `o off`, tap apply |
| Send a join code | `+` `c` | 2 | tap `+ invite`, tap `c copy code` (or show the QR) |
| Add a Cloudflare key | `/` `key` `⏎` `⏎` paste `⏎` | 5 + paste | tap `⏎ connect` on the onboarding or keys card |
| Browse and preview a file | `i` `f` `↓…` (preview follows the cursor at ≥100 cols; `⏎` full) | 2 + moves | tap node, tap `f files`, tap file |

### Ergonomics table

| Action | Frequency | Keys | Count | Key positions / fingers | Rationale |
|---|---|---|---|---|---|
| Go to a device | high | its letter | 1 | wherever the letter is; letters are the device's initial | mnemonic beats position; shown on every node |
| Attach a session | high | slot `a s d …` | 1 (2 from hub) | home row, strong fingers | most frequent action gets the best keys |
| Open selection | high | `⏎` | 1 | right pinky, large key | universal |
| Detach | high | `Ctrl+\` | 1 chord | left pinky + right pinky | the only key cx steals inside sessions (spec 8.1); also a tap chip |
| Peek | med | `␣` | 1 | thumb | cheapest key for a look-without-commitment |
| New claude / codex / shell | med | `c` / `x` / `n` | 1 (2 from hub) | left index/middle, right index | mnemonic; `c x` adjacent |
| Switch zone | med | `Tab` | 1 | left pinky | no chord |
| Find anything | med | `/` | 1 | right pinky | vim/web convention |
| Files of a device | med | `f` | 1 | left index, home row | mnemonic, home row |
| Edit a link | low | `e` then peer letter | 2 | left middle | mnemonic "edges" |
| Policy | low | `k p o` | 1 | right hand | mnemonic key / password / off |
| Invite | low | `+` | 1 chord | shift + right pinky | rare; also a footer chip |
| Help | low | `?` | 1 chord | shift + right pinky | convention |

## 7. Degradation

- **256 colours**: identical layout; blends resolve to the nearest xterm index.
- **16 colours**: no tints. Tiles and zones separate by spacing and labels; selection is a
  reversed name plus `▸`; the viewer pill becomes a rounded box; charts keep the heat
  ramp in green/yellow/red; minimap cells keep ink colours but no backgrounds.
- **NO_COLOR**: attributes only. Keycaps are reversed bold letters; selection is reverse;
  thumbnails show the text tail (dim/bold kept) without the minimap.
- **ASCII**: every glyph has a one-cell substitute (section 4); braille links become slope
  strokes; the QR is dropped (code and one-liner remain). The capture lint reports 0
  non-ASCII cells.
- **Optional enhancement**: kitty/sixel images could replace the braille minimap with a
  real downscaled screenshot of the session; the braille minimap is the shipped fallback
  and is what the captures show.

## 8. Integration notes

- Rendering never dispatches; every target is recorded as `LabTarget` with the action the
  real UI would run (`device:<id>`, `session:<dev>:<id>`, `edge:<id>`, `new:<dev>:<provider>`,
  `policy:<from>:<to>:<value>`, `palette:<scope>`).
- The schematic needs per-device cpu history and per-edge rtt history (already in the
  spec's additive data, section 14) and the session screen tail the preview already has.
- The packet animation is driven by a frame counter; a real build would redraw the
  schematic at a low rate (2-4 Hz) only while the hub is visible.
