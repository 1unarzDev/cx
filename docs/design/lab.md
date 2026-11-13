# Render lab

The render lab draws UI concepts from a pure, synthetic view-model (`LabState`, no App,
IO, tmux or network) into ratatui buffers and turns them into PNGs, so designers can
compare layouts across views, terminal sizes (phone 40x20 through 200x60) and terminal
tiers (truecolor dark/light, 16 colors, ASCII/NO_COLOR) without running cx. A concept is
a `Concept` impl in `src/ui/lab/concepts/<name>/mod.rs`, registered with one `mod` line
and one entry in `concepts::all()` (`src/ui/lab/concepts/mod.rs`). It receives a
`LabView` (Hub, DeviceFocus, SessionPeek, Files, FilePreview, Palette, Secrets,
Onboarding, JoinCode, EdgeEditor, Help), the fixture, `LabCaps` (color level, unicode,
theme, focus, selection, hover, frame) and an area. Use `caps.tok(Tok::..)` for colors
and `caps.glyph(unicode, ascii)` for symbols. The capture then clamps colors to the
tier and lints leaks (non-ASCII cells in ASCII mode, colors above the tier), which show
as `!` in contact-sheet captions. Fixtures are `small` (3 devices), `typical` (7 devices,
2 meshes, 5 sessions), `large` (25 devices, 40 sessions), `nebula-1` .. `nebula-25` (that
many devices over 1-3 meshes, mixed up, down and never-seen, some behind a relay) and
`mirror` (a network that is its own mirror image). `CX_LAB_MOTION=<device>:<thousandths>`
captures the hub map part-way through a transition from that device to the selection. `baseline` roughly copies
the v0.1.95 layout and is the bar to beat. `glyphs` is a renderer test card.

```sh
tests/lab.sh baseline                         # every view x 5 sizes x 5 variants (dark/light truecolor, dark/light 16, ascii)
tests/lab.sh mydesign hub,device:innovation   # only some views
CX_LAB_SIZES=40x20,60x30 CX_LAB_VARIANTS=dark-truecolor,ascii \
  CX_LAB_FIXTURES=typical,large tests/lab.sh mydesign hub
CX_LAB_FOCUS=sessions CX_LAB_SELECTED=2 CX_LAB_HOVER=30,8 tests/lab.sh mydesign session
```

The script prints `local-evidence/ui-lab/<concept>/round-N/`, a new numbered folder on
every run. It contains `contact.png` (rows are views, columns are variants, at 80x24),
`contact-<variant>.png` (rows are views, columns are sizes) and `frames/` with one
`<view>__<WxH>__<variant>[__<fixture>].{png,txt,json}` per frame. A frame also gets
`.targets.png` when the concept implements `targets()`. View specs: `hub`,
`device[:id]`, `session[:id]`, `files`, `file-preview`, `palette[:query]`, `secrets`,
`onboarding`, `join`, `edge[:from-to]`, `help`. Variants are dash-joined from `dark`,
`light`, `truecolor`, `256`, `16`, `nocolor`, `ascii` (for example `light-256` or
`ascii-16`). `cargo test --locked` renders every concept across all views, fixtures and
some degenerate sizes, so a concept that panics fails the suite. PNGs are rendered by
`tests/lab_png.py` (run by `CX_LAB_PYTHON`, default `python3`, which needs Pillow), which uses JetBrains Mono (`CX_LAB_FONT`, `CX_LAB_FONT_PX`) with
fontconfig fallback and draws box, block and sextant glyphs as geometry, the way
kitty, foot and ghostty do.
