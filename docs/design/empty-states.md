# Empty and sparse states

Audit of every screen where cx has little or nothing to show, drawn from the lab
fixtures `bare` (no sessions anywhere, an empty folder, no secrets), `ended` (creation's
work has ended), `solo` (only this device), `nomesh` (plain SSH links) and `typical`
(an unreachable device, an empty search), at 40x20, 80x24, 120x40 and 190x50, dark and
light:

```sh
CX_LAB_SIZES=40x20,80x24,120x40,190x50 CX_LAB_VARIANTS=dark-truecolor,light-truecolor \
  CX_LAB_FIXTURES=bare,ended,solo,nomesh,typical \
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
in the space the tiles would take. Top to bottom, each part only when its data exists:

1. **Launch row**: `c claude   x codex   n shell`, each provider word in its colour, its
   key as a keycap. On the hub each key carries the device letter first (`t c`). An
   agent the device's helper did not report is left out (shell always stays); before
   the helper has answered, all three show.
2. **Resume** (the newest session that ended while cx watched): `a ↺ claude · merge
   lidar maps · ~/ros_ws/src/mapping · 40m ago`. The key resumes the agent's latest
   conversation in that folder (`claude --continue`, `codex resume --last`), or opens a
   shell there.
3. **Recent folders** (folders of ended sessions, then Files locations on that device,
   newest first, at most five): `s ~/cx`, `d ~/datasets/lidar`. The key opens the
   provider chooser in that folder, so a new agent in a recent project is two keys.
4. **Capabilities** are the launch row itself: an agent the helper did not report is
   not offered (a separate "runs claude · codex" line repeated it, and the header
   already carries cpu, memory, temperature and gpu).
5. **Only this device** (hub): `+ invite a device` closes the launch row.

Resume and the folders take the session slot keys (`a s d g ...`): on a device with no
sessions those keys are free, and they are the same keys a started session will take.

The other states:

- **Hub, no sessions**: the launch panel for the selected device in place of the tile
  grid, headed `nothing running · start something on tranquility`.
- **Unreachable device**: the panel says `can't start anything: no route · last seen 3h`
  with `r retry`; recent folders and resume still show (as memory, without keys that
  would fail).
- **Empty Files folder**: under `No items`, two quiet hints, `a create` and
  `' other device`, both keys yazi already binds.
- **Keys with none**: one line saying what keys are for ("Secrets reach new sessions on
  this device as environment variables, and connect meshes"), then `h new secret`.
  The slot chip leaves the footer while there are no cards.
- **Empty search**: `no matches for “zzqx”` and `esc close`.
- **Tile keys**: a pool, home row first, that avoids every key the device screen binds:
  `a s d g`, then `q u i o y z v m`, then `1`-`9` (21 keys). A session keeps its key
  until it ends; new sessions take the lowest free key. Past 21 sessions on one device
  the last tile is a `+N` chip whose key (`/`) opens the session picker.

## As built

- `views::mission::views::launch_panel` draws the panel (device view with no
  sessions, the hub's empty grid and its spare cell). Its rows tap as their keys
  (`new:<d>:<provider>`, `launch:<d>:<key>`, `invite`).
- Data, no new process per frame: `LabDevice::caps` from the helper's capability reply
  (whenever cx last asked, for a launch; opening a device does not ask, because a capability reply also turns on external-terminal discovery), `LabDevice::recent` and
  `LabState::ended` from `App::launch_items`: sessions that left a fresh listing while
  cx ran (`App::ended`, memory only: the session list cache on disk is unchanged), pinned
  sessions and the last Files folder.
- Keys: `screens::slot_keys` keeps each session's key in `App::slot_memo` until the
  session leaves the listing; `screens::assign_slots` is the pure rule. The device and
  hub tiles are drawn in key order, and the hub ranks keyless sessions last, so every
  tile on screen has a key (`lab::key_tests`).
- Not built: "last-used mode" on the launch row (cx does not record a mode yet), ROS
  workspace and container detection on the panel (no cheap source today).
