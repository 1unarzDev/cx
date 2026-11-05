# Files load-time benchmark (cx 0.1.95 baseline)

Harness: `tests/files_bench.py`. Comparator: `tests/files_bench_compare.py`. Raw results:
`local-evidence/perf/files-baseline.json` (local and loopback SSH, 5 runs) and
`local-evidence/perf/files-baseline-rtt20.json` (remote only, 20 ms added RTT, 3 runs). A
generated table sits next to each file (`.md`).

## Reproduce

```sh
cargo build --release --locked
P=/path/to/python-with-pyte-and-pillow
# Full baseline: local and loopback sshd, 5 runs (about 24 minutes on this host)
TMPDIR=/var/tmp $P tests/files_bench.py target/release/cx --remote --runs 5 \
    --label v0.1.95-baseline --out local-evidence/perf/files-baseline.json
# Remote over a 20 ms round-trip relay
TMPDIR=/var/tmp $P tests/files_bench.py target/release/cx --remote --sides remote --rtt 20 --runs 3 \
    --only startup,helper,dirs,previews --files src_2k,txt_200k,src_200k \
    --label v0.1.95-rtt20 --out local-evidence/perf/files-baseline-rtt20.json
# Gate a candidate build against the baseline
$P tests/files_bench.py path/to/candidate/cx --remote --runs 5 --label candidate --out /var/tmp/candidate.json
$P tests/files_bench_compare.py local-evidence/perf/files-baseline.json /var/tmp/candidate.json --gate
```

`TMPDIR=/var/tmp` was needed only because the per-user `/tmp` quota on this host was exhausted
by other build trees during the run. The harness works with any writable TMPDIR.

Other options: `--only startup,dirs,deep,previews,scroll,helper`, `--sizes 100,1000,20000,100000`,
`--files KEY,...`, `--sides local,remote`, and `--oplog FILE`. `--oplog` is a diagnostic option
that puts a framing proxy in front of the remote helper. The proxy logs each operation's name,
request and response sizes, and helper-side time, but never paths or payloads. Do not use
`--oplog` for numbers you intend to compare.

Recorded 2026-11-04/05 on `revamp/files-bench` at `fd494d1` (src unchanged from `revamp/base`),
binary `cx 0.1.95` sha256 `de74eb41ab82…`. Host: Linux 7.2.9, 20 CPUs, Python 3.14, 120x40
pyte PTY. **The host was heavily loaded by other workers**: the load average ran from 33 to
110 during the baseline. Medians are usable. The p95 values are inflated by scheduler noise, so
compare p95 only between runs made under similar load (each JSON records `loadavg_before` and
`loadavg_after`).

## Method

- **Real PTY, as the user feels it.** Each viewer is `cx restart <handoff>`, opened in Files on
  the chosen device with the chosen row selected. Every time runs from the `write()` of the key
  to the arrival of the PTY output chunk that first made a pyte screen predicate true. The
  predicates are: rows `b-NNNNNN.txt` visible; the selected `▸` row moved; the `▸ N items` title
  at its final value with no `· loading`; the fixture's first line inside the preview box; at
  least 200 half-block cells for raster previews. No sleep is used to detect a paint.
- **Directories.** Each size has `big/` and `sibling/` (N empty files each) under one parent.
  *cold* is the first Enter in a fresh viewer process. Then `h` goes back to the parent, *sibling*
  is Enter on the sibling (also uncached), and *warm* is a second visit to `big/` served from the
  viewer's browser cache. *Usable* sends `j` the moment the first rows appear and records when
  the selection visibly moves. *Full count* is reached when the title shows
  `min(N, 10000)` items and loading has finished. Deep tree: 24 nested levels with 10 files
  each, timed per Enter hop and per `h` hop.
- **Previews.** One fresh viewer per file. Before each cold open, the file's pages are dropped
  with `posix_fadvise(DONTNEED)`, and `fincore` confirmed this for 110 of 110 opens. Then `G`
  checks whether the last line is reachable. Fixtures: `.rs` source at 2 KB, 200 KB, 5 MB and
  50 MB; `txt_200k` (the same bytes as `src_200k` under a `.txt` name, which isolates syntax
  highlighting); a 2 MB markdown file; a 1920x1080 PNG and JPEG; a 3-page PDF; a 1 MB binary;
  and a 20 KB `.rs` file inside `ünïcødé ✓ 目录/файл — ✓ 文件.rs`.
- **Scrolling.** 30 presses each of `j`, PageDown and SGR wheel-down in the 5 MB source and the
  2 MB markdown file. Key-to-paint is the first chunk that changes the preview box content.
- **Startup.** Enter is pressed as soon as the first listing paints. That is what a user does who
  opens cx and goes straight to a file. Every other remote measurement waits 2.5 s after startup
  first, so that it measures file work rather than queueing (see bottleneck 5).
- **Helper microbenchmark.** Drives `cx helper` directly with `CX1 <len>\n<json>` frames. Locally
  it talks to a subprocess on stdin; remotely it goes over the same loopback sshd. It records
  the time to the first response, a minimal request (List of an empty dir), Info, the first List
  page, 1000-entry continuation pages until 10,000 entries (what the UI does today), a full
  listing with 4000-entry pages, and Preview cold and warm. It also records bytes per entry.
  `fs floor` is Python `scandir` + sort + `lstat` of one page on the same directory, a lower bound
  for the helper's filesystem work. Local UI requests are dispatched in-process (no
  serialization), so the local helper number is an upper bound for the UI's helper share.
- **Remote fixture.** A private sshd on 127.0.0.1 accepts only a temporary key. A PATH-wrapped
  `ssh` replaces cx's `-F` config with a private one and rewrites only the fixed
  `exec ~/.local/bin/cx helper` command to `exec env HOME=<private> … <bench binary> helper`.
  It refuses every other remote command and counts the refusals by cx subcommand name (one
  `update` and one other attempt per viewer start). The real `~/.ssh`, the installed
  `~/.local/bin/cx`, agents and the user's tmux are never touched. `--rtt MS` inserts a
  ProxyCommand relay that delays each direction by MS/2.
- **Cleanup.** Every process gets a private `CX_FILES_BENCH=<tag>` in its environment. Cleanup
  kills only tagged processes and verifies that none survive: 200 viewers, 0 remaining, 0 stray
  orphans, 0 stale handoffs.

**Cache caveat.** Without root, the dentry and page caches cannot be dropped for directories, so
directory "cold" means *cx-cold, OS-warm*. File previews are OS-cold for the file's data pages.

## Headline baseline (medians; p95 in the JSON)

| What | Local | Remote (loopback) | Remote (+20 ms RTT) | Target |
|---|---|---|---|---|
| Dir 100: Enter -> first rows | 103 ms | 102 ms | 102 ms | < 50 ms local, < RTT + 30 ms remote |
| Dir 20k: Enter -> first rows | 151 ms | 113 ms | 108 ms | same |
| Dir 100k: Enter -> first rows | 232 ms | 239 ms | 113 ms | same |
| Dir 20k: Enter -> full count | 557 ms, **10,612 of 20,000** | 497 ms | 519 ms | all entries |
| Dir 100k: Enter -> full count | 820 ms, **10,604 of 100,000** | 862 ms | 580 ms | all entries |
| Dir revisit (warm) 100 / 20k: first rows | 2.4 / 57 ms | 2.2 / 24 ms | 3 / 61 ms | < 16 ms |
| Deep tree: Enter per level / `h` per level | 104 / 2.2 ms | 104 / 2.0 ms | - | < 50 ms |
| Preview 2 KB `.rs`: first paint | 102 ms | 101 ms | 101 ms | < 50 ms |
| Preview 200 KB `.rs` / same bytes `.txt` | 211 / 105 ms | 205 / 103 ms | 206 / 103 ms | < 50 ms |
| Preview 5 MB / 50 MB `.rs` | 207 / 209 ms | 207 / 308 ms | - | < 50 ms |
| Preview 2 MB markdown | 133 ms | 129 ms | - | < 50 ms |
| Preview PNG / JPEG / PDF | 118 / 225 / 1523 ms (PDF fails 2 of 5) | 116 / 214 / 2031 ms | - | < 50 ms (raster: first paint < 100 ms) |
| `G` reaches the last line of a > 32 KiB file | **no** (any size) | no | no | yes |
| 2 KB preview straight after startup | 204 ms | 304 ms (p95 1000 ms) | 302 ms | < 50 ms / RTT + 30 |
| Scroll 5 MB source j / PageDown / wheel, p50 / p95 | 11.7 / 30.9, 10.5 / 27.9, 9.9 / 24.3 ms | 17 / 30, 17 / 39, 20 / 38 ms | - | p95 < 10 ms |
| Scroll 2 MB markdown j / PageDown / wheel, p50 / p95 | 4.5 / 20.2, 6.2 / 38.1, 8.1 / 35.7 ms | 11 / 21, 13 / 23, 17 / 43 ms | - | p95 < 10 ms |
| `j` in a loaded 10k list, p50 / p95 | 59 / 219 ms (20k dir) | 25 / 109 ms | 21 / 35 ms | < 10 ms |

Helper microbenchmark (no UI):

| | Local | Remote loopback | Remote +20 ms |
|---|---|---|---|
| Spawn/connect -> first response | 6.8 ms | 118 ms (SSH handshake) | 224 ms |
| Minimal request | 0.03 ms | 0.1 ms | 20.5 ms (= 1 RTT) |
| First List page, 20k dir (2,612 entries, 669 KB) | 83 ms (best 14) | 17 ms | 45 ms |
| First List page, 100k dir | 113 ms (best 48) | 76 ms | 91 ms |
| UI paging pattern to 10k entries (9 requests) | 177 ms (best ~56) | 49 ms | 238 ms (≈ 9 RTT + work) |
| Full listing, 100k with 4000-entry pages (39 requests) | 1280 ms | 782 ms | 1248 ms |
| Bytes per entry | 256 | 256 | 256 |
| Preview request, any text size (32 KiB cap) | 0.2–2.8 ms | 0.4–0.7 ms | 22–29 ms |
| Preview PNG / JPEG / PDF | 112 / 84 / 3051 ms | 40 / 53 / 1652 ms | - |
| fs floor 20k / 100k: scandir+sort (lstat of one page) | 51 / 267 ms (13 ms) | | |

The local helper numbers are noisier than the remote ones under this host load. The smoke run at
lower load measured 23 ms for the 20k first page and 56 ms for the 10k paging pattern.

## Where the time goes

The microbenchmark makes the split clear. A request for a text preview costs the helper under
3 ms, and a 100-entry List costs under 1 ms. The user nonetheless waits about 100 ms for
either. Almost all of the user-visible time is in the viewer's event loop and rendering, not in
the helper or the transport.

1. **The event loop sleeps up to 100 ms before it handles a reply (helper ~1 ms, UI ~100 ms).**
   `src/ui/runtime.rs:1409` blocks in `event::poll(wait)`, and `wait` defaults to
   `Duration::from_millis(100)` (`src/ui/notifications.rs:96-111`). Worker replies arrive on
   `result_rx` but do not wake the poll. They are drained only after it returns
   (`src/ui/runtime.rs:1136`). Evidence: a 100-entry listing, a 2 KB file, a binary, a `.txt` of
   any size and every deep-tree hop all land at 101–105 ms, with or without 20 ms of RTT,
   because the reply is ready long before the tick. This one cost puts every local and remote
   "first rows" and "first paint" over the 50 ms target. A wakeup on reply (one channel for
   input and replies, or a self-pipe) would remove it.
2. **Directory paging round trips and the 10,000-entry cap (helper + transport, made worse by the
   tick).** `src/ui/apply/files.rs:249-266` requests 1000-entry continuation pages one at a time
   and stops at 10,000 entries. A 20k or 100k directory therefore never shows all its entries:
   10,612 and 10,604 are shown. Reaching that takes 9 serial requests. That is 0.5–0.9 s in the
   UI against 56–180 ms of helper time, and 9 RTTs over a real link. In addition:
   - Each page triggers a full re-sort of all loaded entries (`src/ui/apply/files.rs:215`), even
     though the helper already returns them in the UI's order (`"order"` field,
     `src/files.rs:196`).
   - The first page carries only about 2,600 entries, not the 4,000 that `FIRST_PAGE_LIMIT`
     allows (`src/files.rs:184`). At 256 bytes per entry, `LIST_BYTES` = 768 KiB
     (`src/files.rs:187`) is reached first. Each entry repeats the full encoded path and an
     identity JSON string (`src/files.rs:364-375`).
   - Sending the parent once and only the names would cut the bytes per entry roughly fourfold.
     A larger first page and window plus a total count (which the helper already returns) would
     then cover the visible rows in one RTT.
3. **Rendering and selection are O(n) in the loaded list (UI render).**
   `browser_entries()` clones and lowercases every entry on every call
   (`src/ui/files_view.rs:4-13`). It is called several times per frame (`src/ui/files_view.rs:194`
   and `:225`). The list builds a `Line` and a `Row` for every entry, not just the visible window
   (`src/ui/files_view.rs:242-330`). Evidence:
   - `j` in a loaded 10k list costs 21–59 ms median (p95 up to 219 ms under load), against
     2–8 ms at 100–1000 entries.
   - A warm revisit of a 20k or 100k dir takes 24–82 ms just to paint cached rows, against
     2–7 ms for small dirs.
   - First rows of the 100k dir cost about 230 ms. The helper's own first page is 48–113 ms of
     that, from the read_dir + sort of 100k names (`src/files.rs:250-285`, fs floor 64–267 ms).

   A windowed list (being built by another worker) addresses the render half. The helper's
   100k cost is the single-pass snapshot and is inherent to sorted output. The way out is
   streaming the first visible window before the sort completes, or accepting it as a 100k-only
   cost.
4. **Syntax highlighting runs on the reply path (UI CPU, about 100 ms per 32 KiB of code).**
   `RichPreview::from_value` runs on the worker before the reply is delivered
   (`src/ui/runtime.rs:790`). For `kind == "code"` it calls `syntax_preview::highlight`
   (`src/ui.rs:188`), which highlights the whole 32 KiB prefix with syntect fancy-regex under a
   500 ms budget (`src/syntax_preview.rs:31-33`). The controlled pair shows the cost:
   `src_200k.rs` takes 205–211 ms and the same bytes as `txt_200k.txt` take 103–105 ms. The
   20 KB Unicode `.rs` file takes 306 ms. Markdown line building adds about 30 ms (133 ms). The
   fix is to paint plain text first and apply highlighting afterwards, or to highlight only the
   visible lines.
5. **Head-of-line blocking on the single per-host metadata channel (transport/queue).** Remote
   file requests share one SSH channel and one UI worker endpoint per target with Sessions,
   Containers and the provider checks (`src/transport.rs:236-249`; endpoint key at
   `src/ui/runtime.rs:693-726`; local file reads get their own `<local-files-read>` endpoint,
   remote reads do not). The `--oplog` diagnostic shows the startup burst: Info at 100–260 ms
   and **Containers at 600–1090 ms** of helper time on this host, then the Preview (0.2 ms of
   helper work) served about 640 ms after Enter. The UI result: a 2 KB remote preview straight
   after startup takes 304 ms median, p95 1.0 s, and one smoke run reached 6.4 s. The same burst
   repeats every 30 s (`src/ui/runtime.rs:1479-1489`, `refresh_containers`), so a remote file
   open can stall for up to about 1 s at any time. A separate file-read channel per host, like
   the existing `session-preview:` and `capabilities:` split, would remove this.
6. **Previews are a fixed 32 KiB prefix (helper contract).** `TEXT` = 32 KiB
   (`src/file_preview.rs:12`). `G` reaches the end only for the 2 KB file. For 200 KB, 5 MB and
   50 MB files and the 2 MB markdown file, the end is unreachable at any size. Open cost is
   therefore flat in file size (a 50 MB file opens as fast as a 200 KB one). Any streaming
   design must keep that property: first paint must stay independent of file size.
7. **Raster previews: decode, re-encode and wire size (helper + transport + UI decode).**
   - PNG and JPEG cost 40–112 ms in the helper: decode, resize and PNG re-encode
     (`src/file_preview.rs:44-80`). They put 217–343 KB of base64 on the wire, and the viewer
     decodes the PNG again (`src/ui.rs:137-160`). First paint is 116–225 ms.
   - PDF shells out to `pdftoppm -scale-to 1280 -png` (`src/file_preview.rs:303-320`), which took
     1.5–2.6 s for this photo-like page on the loaded host. The 3 s converter deadline
     (`src/file_preview.rs:409`) then often fails it: 4 of 5 local helper requests returned
     "PDF preview unavailable", and 3 of 10 UI opens timed out at 60 s without a raster paint.
   - The fix is to render at the terminal's pixel size, not at 1280 px, and to send the
     half-block cell grid (or a smaller raster) instead of a PNG.
8. **Scrolling re-wraps the whole loaded preview every frame (UI wrap).**
   - Every frame rebuilds the display lines and recomputes the wrapped height of the whole
     32 KiB prefix (`src/ui/preview.rs:802-806`, `preview_max_scroll` at `:634`). It then renders
     a `Paragraph` scrolled from the top (`src/ui/preview.rs:866`).
   - Markdown files with links render the whole paragraph a second time into a scratch buffer for
     hit-testing (`src/ui/preview.rs:836-855`).
   - Result: key-to-paint p50 is 4–20 ms and p95 is 20–43 ms against the 10 ms target. At lower
     host load the smoke run measured 15–18 ms p95.
   - This stays bounded only because of the 32 KiB cap. A streaming preview without a wrap cache
     (being built by another worker) would make it grow with file size.
9. **SSH connection setup (transport).** Connecting costs 118 ms on loopback and 224 ms with
   20 ms of RTT, before the first remote response. The viewer pays it at startup, so it shows in
   "viewer ready" (211 ms vs 79 ms locally) and not in per-file times. Each viewer start also
   attempts `cx update --automatic` over a separate SSH connection, which the fixture refuses.

Transport per request is small. A minimal request costs 0.1 ms on loopback and exactly 1 RTT at
+20 ms. The 669 KB first page of a 20k dir takes 17 ms remote vs about 14 ms local at best. At a
20 ms RTT the paging pattern costs 238 ms of round trips for 10k entries. So "first rows < 1 RTT +
30 ms" is achievable once the tick (1) and the per-page serialization (2) are gone. Today the 100
ms tick hides the RTT completely: remote first rows are about 102–113 ms at both 0 and 20 ms RTT.

## Targets and gaps

| Metric (JSON name pattern) | Target | Baseline (median) | Main blockers |
|---|---|---|---|
| `local.dir.*.cold.first_rows_ms` | < 50 ms | 103–232 ms | 1, 3 |
| `remote.dir.*.cold.first_rows_ms` | < RTT + 30 ms | 102–239 ms (RTT ≈ 0 / 20) | 1, 2, 5 |
| `*.dir.*.full_count_ms` and `final_count` | all entries; < 300 ms for 20k locally | 10.6k of 20k/100k in 0.5–0.9 s | 2, 3 |
| `*.dir.*.warm.first_rows_ms` | < 16 ms | 2–82 ms | 3 |
| `local.preview.*.first_paint_ms` (text) | < 50 ms | 102–307 ms | 1, 4 |
| `remote.preview.*.first_paint_ms` (text) | < RTT + 30 ms | 101–308 ms; 304 ms after startup | 1, 4, 5 |
| `*.preview.*.end_reachable` | 1 (yes) | 0 for every file > 32 KiB | 6 |
| `*.scroll.*.key_to_paint_ms` p95 | < 10 ms | 20–43 ms | 8 |
| `*.dir.*.j_in_list_ms` p95 | < 10 ms | 35–219 ms with 10k loaded | 3 |
| raster `first_paint_ms` | < 100 ms; PDF never fails | 116–225 ms; PDF fails 40–80 % under load | 7 |

## Comparing runs

`tests/files_bench_compare.py BASE NEW [--threshold 10] [--min-ms 2] [--filter REGEX] [--all]
[--gate]` prints a markdown table of regressions, improvements and new or missing metrics.
- A timing metric regresses when its median moves the wrong way by more than 10 % *and* more than
  2 ms. When both sides have at least 10 samples (scroll and `j` metrics), the same test applies
  to p95.
- Counts and reachability (`better: higher`) regress when they drop.
- A metric whose every attempt failed regresses.

`--gate` exits 1 on any regression. Compare only runs with the same `--remote`/`--rtt`
settings (the comparator warns otherwise) and similar host load. On a loaded host, run 5 or more
times and gate on medians, or use `--filter` to scope the comparison to the metrics an
integration targets.

## Not measured, or measured with caveats

- **OS-cold directories.** Dropping the dentry and page caches needs root, so directories are
  OS-warm. File previews are OS-cold via `fadvise`, confirmed by `fincore`.
- **A slow filesystem path** (NFS, FUSE, spinning disk). Simulating one needs root or FUSE.
  `--rtt` models link latency for the remote path only. It adds no bandwidth limit or jitter.
- **Terminal paint time.** pyte measures when bytes arrive at the PTY, not when the GPU or
  terminal emulator draws them. Kitty, sixel and iTerm image protocols are not exercised:
  rasters render as half-blocks under `TERM=xterm-256color`.
- **A real remote host.** The remote helper runs on the same machine, the same architecture and
  the same filesystem as the viewer, so remote helper CPU competes with the viewer.
- **Container-scoped browsing** (`ContainerFiles`) and the transfer and destination browsers are
  not covered.
- **Host load.** The load average was 33–110 from other workers during the baseline. p95 values
  and local helper medians are inflated. The smoke run at lower load is noted in the text but
  is not a recorded baseline.
