# Empty and sparse states

Audit of every screen where cx has little or nothing to show, drawn from the lab
fixtures `bare` (no sessions anywhere, an empty folder, no secrets), `ended` (creation's
work has ended), `quiet` (everything has ended), `solo` (only this device), `nomesh` (plain SSH links) and `typical`
(an unreachable device, an empty search), at 40x20, 80x24, 120x40 and 190x50, dark and
light:

```sh
CX_LAB_SIZES=40x20,80x24,120x40,190x50 CX_LAB_VARIANTS=dark-truecolor,light-truecolor \
  CX_LAB_FIXTURES=bare,ended,quiet,solo,nomesh,typical \
  tests/lab.sh empty "hub,device:creation,device:squirtle-jetson,files,secrets,palette:zzqx"
```

The rule for all of them: an empty area is a chance to offer the next step, using data
cx already holds. Nothing is invented: a row is drawn only when its fact is known, and
every row is a key and a tap.

## Audit (v0.3.1)

| State | What it draws | What is wrong |
|---|---|---|
| Device with no sessions (creation), 80x24 and up | A dashed `╭╌╌╮` box filling the sessions zone, `new on c creation` and three stacked `c claude / x codex / n shell` rows, centred in up to 20 blank rows | The box is the loudest thing on the screen and holds three words. Rows of blank space inside a frame read as "broken", not "calm". The title repeats the device the header already names. No hierarchy: the three options are the only content, set flush left in a wide frame. |
| Device with no sessions, 40x20 | `new  c claude  x codex  n shell` on one row | Fine as a row; nothing says what the device can run or what happened here before. |
| Hub, no device has sessions | The same dashed box under SESSIONS, with `t c claude` addresses | Half the hub is an empty frame. The vitals strip right above it already offers `t then f files`; the box repeats the device name. |
| Single device (solo) | Map is one node in an orbit; the dashed box takes the bottom 12-40 rows | At 190x50 the box is 35 rows of nothing. Nothing points at how to add a second device (`+ invite`) except the footer. |
| No meshes | The map draws plain links; the device header says `no mesh · direct ssh` | Reads well. No change. |
| Unreachable device (squirtle-jetson) | Header says `✕ no route · last seen 3h` twice (evidence line and metrics line), then the dashed box with `c x n`, which cannot work | The launch options are offered on a device that cannot start anything; the reason is repeated instead of the fix (`r retry`) being next to it. |
| Never had a session vs all ended | Identical: both show the empty dashed box | cx forgets what just ended. The most useful thing on an idle device (resume what you were doing, in that folder) is missing. |
| Empty Files folder | yazi's `No items` centred in the current column | Correct yazi behaviour, but a dead end: no hint that `a` creates or that `'` switches device. |
| Keys with no secrets | `h new secret` and one line about Cloudflare, then 20 blank rows; the footer offers `a s d g set` for cards that do not exist | The footer's slot keys do nothing. Nothing says what a secret is for (sessions' env vars, mesh tokens). |
| Empty search (`/zzqx`) | The palette frame with the query and an empty body | No "no matches" line, no way out except Esc (which the footer does say). |
| Hub tile keys | Only the first four sessions on a device (`a s d g`) have a key; a fifth tile shows a blank keycap; `+3 more ›` has no key | Not every visible tile can be reached by a key (reported by the user). |

## Redesign

One component replaces the dashed card: the **launch panel**, drawn without a frame,
in the space the tiles would take. Every row has one grammar: a quiet label column
(`start`, `or on`, `add`, `resume`, `recent`), the key, then what it does. Phones (under
56 columns) drop the label column on every row, so no row puts its label on the other
side. Top to bottom, each part only when its data exists:

1. **Launch row**: `start  c claude   x codex   n shell`, each provider word in its
   colour, its key as a keycap. It is the same component (word, colours, label column)
   as the row under a device's tiles. On the hub the device is said once, in the grammar
   of the vitals strip: `start on  t tranquility then  c claude  x codex  n shell`; the
   keys after `then` are drawn quiet (muted, not the accent), so only the first-press
   keys look primary and `c` under it (`or on  c creation`) is not read as claude. An
   agent the device's helper did not report is left out (shell always stays); before
   the helper has answered, all three show.
2. **Only this device**: `add  + invite a device`, a group of its own (a blank row
   before it), on the hub and on the device screen (the footer's `+ invite` gives way).
3. **Resume** (the newest session that ended while cx watched): `resume  a claude  merge
   lidar maps · ~/ros_ws/src/mapping · 40m ago` (a narrow row drops the age, then the
   folder, then cuts the title). The key resumes the agent's latest conversation in
   that folder (`claude --continue`, `codex resume --last`), or opens a shell there.
4. **Recent folders** (folders of ended sessions, then Files locations on that device,
   newest first, at most five): `recent  s ~/cx`, `d ~/datasets/lidar`. The key opens
   the provider chooser in that folder, so a new agent in a recent project is two keys.

"Nothing running" is said on the SESSIONS rule (` SESSIONS  nothing running ───`, where
the hub's tiles say `needs you first`), not on a row of its own. The panel sits at the
top of its zone, right under the rule. A blank row separates its groups where they all
fit: always on the hub, and on a device screen from 120x40 up (so two devices at one
size keep one rhythm). Resume and the folders take the session slot keys (`a s d g
...`): on a device with no sessions those keys are free, and they are the same keys a
started session will take.

The other states:

- **Hub, no sessions**: the map sits near the top as in the approved reference, the
  vitals strip and the panel right under it; the rows left over stay below, where tiles
  will go. The panel adds `or on  i innovation  c creation ...` (every other reachable
  device, a letter away, names cut to a common length, at least four letters, so no
  key ever shows without its device) and the newest session that ended on up to two
  reachable devices (`b a shell  ros core · ~/ros_ws · 1s ago`, the device letter first
  as on a tile). Fixture `quiet`. Phones: under the device tree, the same panel
  (`on t then c claude  x codex`, resumes), the tree's rows going single-height to make
  room; the `or on` row is left to the tree right above it.
- **Only this device** (`solo`): the hub draws this device at the middle of the welcome
  page's quiet orbits, as tall as the panel leaves (up to 15 rows); the device screen
  has `LINKS  none yet` with the same picture under its panel.
- **Device screen links**: the LINKS star is pinned under its rule (as the sessions are
  under theirs); the rows it does not need stay below it.
- **Unreachable device**: the header says `✕ unreachable`; the reason is said once,
  under SESSIONS, muted with only the mark red: `✕ can't start anything · no route ·
  last seen 3h   r retry` (on a phone the reason alone; the footer's `r retry` gives way
  to it). What it last ran and its folders show as a quiet history: `earlier  shell
  record run-0412 · ~/bags · 3h ago`, `recent  ~/ros_ws`, muted, with no key column at
  all (not options with missing keys). Fixture `ended`, squirtle-jetson.
- **Empty Files folder**: under yazi's `No items` (its own place, the column's first
  row), `a create   ' other device`, keys in which-key's colour, both keys yazi already
  binds; a narrow column (phones) stacks them, one centred row each.
- **Keys with none**: one muted line, `new sessions here get the ones you expose  e  as
  env vars` (the key a keycap), under `h new secret`. Not connected, Cloudflare's own
  card already says `⏎ connect`, so no second (yellow) line repeats it. The slot chip
  leaves the footer while there are no cards.
- **Empty search**: `no matches for “zzqx”` and `esc close`, one breathing row between.
- **Tile keys**: a pool, home row first, that avoids every key the device screen binds:
  `a s d g`, then `q u i o y z v m`, then `1`-`9` (21 keys). A session keeps its key
  until it ends; new sessions take the lowest free key. Past 21 sessions on one device
  the last tile is a `+N` chip whose key (`/`) opens the session picker; while it shows,
  the footer's `/ find` (the same palette) gives way to it, so `/` is labelled once. On
  a device's own screen a tile shows only its slot key (`a refactor transport`), the key
  pressed there; the hub keeps the device letter first (`i a`).

## As built

- `views::mission::views::launch_panel` draws the panel (device view with no
  sessions, the hub's empty grid, the phone hub under its tree, and the hub's spare cell
  when one row of tiles leaves one; with two rows the last row's tiles widen instead);
  `launch_keys` is the launch row both it and `new_row` (under a device's tiles) draw. Its rows tap as their keys
  (`new:<d>:<provider>`, `launch:<d>:<key>`, `invite`).
- Data, no new process per frame: `LabDevice::caps` from the helper's capability reply
  (whenever cx last asked, for a launch; opening a device does not ask, because a capability reply also turns on external-terminal discovery), `LabDevice::recent` and
  `LabState::ended` from `App::launch_items`: sessions that left a fresh listing while
  cx ran (`App::ended`, memory only: the session list cache on disk is unchanged), pinned
  sessions and the last Files folder.
- Keys: `screens::slot_keys` keeps each session's key in `App::slot_memo` until the
  session leaves the listing; `screens::assign_slots` is the pure rule. The device and
  hub tiles are drawn in key order, and the hub ranks keyless sessions last, so every
  tile on screen has a key. `lab::key_tests` checks that every tile, launch row, `+N`
  chip and invite row prints its key, once per screen, in every fixture and size;
  `nav_tests::every_launch_panel_keycap_does_what_its_tap_does` presses each printed key
  (the device letter first on the hub) and compares the result with a tap.
- Not built: "last-used mode" on the launch row (cx does not record a mode yet), ROS
  workspace and container detection on the panel (no cheap source today).
