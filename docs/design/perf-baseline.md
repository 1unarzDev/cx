# Performance baseline (cx 0.1.95, release)

Harness: `tests/perf_harness.py`. Raw results, with three repeats and a per-metric aggregate:
`local-evidence/perf/baseline.json`.

Reproduce:

```sh
cargo build --release --locked
uv run --with pyte python tests/perf_harness.py target/release/cx --repeat 3 --out local-evidence/perf/baseline.json
```

Recorded 2026-11-04 on branch `revamp/perf-harness` at `b0eb0eb`, which is the same code as
`origin/main` v0.1.95. Host: Linux 7.2.9, 20 CPUs, tmux 3.8, Python 3.13. The PTY is
120x40 and a pyte screen models the terminal. Each scenario runs in its own temporary
HOME/XDG_STATE_HOME/TMUX_TMPDIR. All three runs passed cleanup: no surviving process
carried the private HOME, `cx orphans --json` reported no stray process among the harness
pids, and no stale handoffs were left.

**How to read the numbers.** *Settled* is the time from the write until the last output
chunk, with the expected change on screen and no further output for 60 ms. *Visible* is the
time until the output chunk that first made the expected change appear. *Bursts* are output
chunks separated by more than 3 ms; each one is roughly one frame. Each value below is the
median across the 3 runs of that run's p50 or p95. The runs agreed closely: most spreads are
under 1 ms.

## Files preview (5,000-line fixtures; 30 presses of each key)

| Metric | Markdown p50 / p95 ms | Source (.rs) p50 / p95 ms | Bytes per press (md / rs) |
|---|---|---|---|
| Enter -> preview visible | 111.7 | 104.6 | 4412 / 4391 |
| `j` settled | 3.3 / 4.6 | 5.4 / 6.6 | 1369 / 3745 |
| PageDown settled | 3.3 / 5.3 | 5.4 / 7.1 | 1582 / 4171 |
| Wheel down settled | 5.4 / 7.3 | 8.1 / 9.9 | 1150 / 3777 |
| `j` at 30 Hz for 30 presses: settle after the last key | 0 | 0 | 30 frames, 40 KB / 113 KB |
| End: last fixture line reachable | line 685 of 5000 | line 849 of 5000 | |

The preview loads only a 32 KiB prefix (`file_preview.rs` `TEXT`). End stops near line
700-850, and the rest of a 5,000-line file cannot be reached. Scroll latency is low because
only that prefix is wrapped on each frame. A streaming preview must be measured again with
this harness.

## Directory listing (20,000 entries, local)

| Metric | Value |
|---|---|
| Enter -> first rows visible | 106.6 ms (first page: 1,000 rows) |
| Enter -> final count stable | 510 ms |
| Final count shown | **10,000 of 20,000** (cap notice shown) |
| `j` in the loaded 10k list, p50 / p95 | 16.0 / 17.4 ms (303 bytes) |

Loading stops at 10,000 entries. Entries arrive in readdir order and are sorted within what
has loaded, so the top rows are not the lexicographically first entries of the directory
(for example, `entry-18972.txt` heads the list).

## Sessions (local managed shell; 5 cycles)

| Metric | p50 / p95 ms |
|---|---|
| `cx new` (CLI, shell session) | 184 |
| Enter -> focused embedded pane | 0.8 / 1.0 |
| Keystroke echo in the embedded pane | **100.9 / 201.4** |
| Detach (`Ctrl+]`) from pane -> list | 1.2 / 1.6 |
| Ctrl+O expand -> native tmux prompt | 83.5 / 85.2 |
| Detach from native -> cx UI visible | 1.7 / 1.8 |
| Detach from native -> cx UI settled | 269.1 / 270.0 (9 frames, about 9.7 KB) |

Echo in the embedded pane comes in 100 ms steps, which points to polled capture rather than
event-driven output. The detach key is the `DETACH_KEY` constant in the harness, so the
planned move to Ctrl+\ is a one-line change.

## Pointer motion with no buttons (40 events, 40 ms apart)

All tested views request any-event mouse reporting (1003). The Sessions idle baseline is
250 B and 10 frames per 1.6 s.

| Region | Sessions bytes / frames per event | Files bytes / frames per event |
|---|---|---|
| Along the already-selected row (nothing changes) | **25 B / 1.0** | 0 / 0 |
| Blank row under the tab bar | 9.4 B / 0.33 (matches the idle baseline) | 0 / 0 |
| Across list rows (hover selects) | 184.6 B / 1.0 | 0.6 B / 0.03 |

In Sessions, every motion event over a row returns "handled" and causes a full redraw,
even when the selection is unchanged. The diff is empty, but ratatui still writes about
25 bytes per frame. The target for a motion event that changes nothing is 0 frames.

## Idle (5 s, after a 1 s settle)

| View | Viewer CPU | Process CPU (viewer, descendants and private tmux) | Bytes/s | Frames |
|---|---|---|---|---|
| Files | 0.00 s | 0.00 s | 5 | 1 |
| Sessions (with live preview) | 0.03 s | 0.06 s | 180 | 34 (about every 150 ms) |

## Regressions to watch, and the targets for revamp work

- Embedded pane echo p50 should drop below about 16 ms.
- Same-row pointer motion should produce 0 frames.
- Idle Sessions should write about 0 B/s when nothing changes.
- The 20k listing should either load in full or state the cap honestly, and should sort
  across all entries.
- Preview scrolling must stay under about 10 ms p95 once the whole file is reachable.

Files load-time benchmark (directory open, previews, scrolling, helper protocol, local and SSH): `docs/design/files-bench.md`.
