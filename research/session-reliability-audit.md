# Session reliability audit

Reviewed 2026-10-19 against upstream v0.1.66, in isolated worktrees. The original
checkout was already paused in a rebase with conflicts; that checkout and its
uncommitted changes were preserved.

## Confirmed defects and corrections

- Failed device refreshes retained selectable shell rows indefinitely. A failing
  regression reproduced this; failed/invalid session responses now clear rows,
  reset freshness and persist the cleared cache. Background reconciliation runs
  regardless of which panel is open. Explicitly down devices remain excluded.
- Retained tmux panes were listed even after their shells exited. Discovery now
  rejects `pane_dead`; only fixture-owned servers were used to reproduce this.
- Container inventory silently discarded inspection failures while claiming a
  complete scan. Discovery now reports completeness, failed inspections and time.
  Absence removes scopes only after complete observation; explicit stop, restart,
  engine, user or authorization changes invalidate a scope even on partial scans.
  Every saved container session is revalidated, preventing it from reappearing on
  the next Sessions refresh. Docker objects are not deleted by cache cleanup.
- Devcontainer workspace identity disappeared when its host configuration was
  missing; explicit configuration labels also lost to conventional paths. Labels
  survive reconnect and explicit configurations take precedence.
- Switching an agent's project within its process retained launch-time directory
  assumptions. Exact native metadata now takes precedence, with PID/start and
  writer/tree identity checks. Missing metadata clears transient names/tasks.
- Device removal could shift array indices beneath queued replies, causing a
  panic or a reply to update another device. Production replies now carry stable
  device IDs and execution targets, remap after reorder, and are ignored after
  removal or address edits. Index-keyed caches and source clipboards are cleared
  together. Late mutation replies release the serialized queue before rejection.
- Cache pruning could silently move selection to another session. A shared
  selection reconciler now preserves identity across all relevant refreshes and
  refreshes/closes the selected task dialog with the session.
- Launch-folder paths must use the existing opaque path encoding, not lossy UTF-8.
  Startup now opens Files on the invocation device, preserving non-UTF8 paths.
- SQLite's startup file and a reused deadline could compromise metadata-only
  reads and fresh rename lookup. The optional SQLite reader suppresses startup
  configuration and uses separate bounded budgets.

## Design and convention findings

The UI controller remains large and mixes launch policy, async state and panel
navigation; rendering and menu modules provide useful seams but should absorb
future additions rather than grow parallel key/provider arrays. Selection
reconciliation is now one helper instead of a Sessions-only special case.

Process discovery still has several bounded `/proc` walks and start-time parsers
across sessions, containers and metadata. They have different namespace/privacy
requirements, so combining them blindly would weaken checks. A future small
process-observation module could centralize identity and bounded traversal while
keeping provider-specific parsing separate.

Session schema additions are optional and retain framed protocol version 1.
Older helpers omit task/activity fields and must be updated for the new evidence.
No transcript scraping, generic pane-title guessing or folder/recency matching was added.
New supported Codex launches set a native thread-name title template per launch.
That signal selects among exact owned writer IDs only when every candidate has
an explicit, unique and lossless name; ambiguity clears the directory.
User-defined names remain authoritative. Native adapters are version sensitive;
unknown identities and unsupported schemas deliberately omit optional metadata.

Activity is evidence, not a claim about model reasoning. Shell foreground commands
and known Claude status drive `running`/`waiting`/`idle`. Recent terminal output
provides `output` for agents and containers, expiring after two seconds; the viewer
also expires stale observations. This cannot identify a silent model/network wait
or silent inner-container command. Codex `open` child edges remain literal states.

## Validation boundaries

Real local tmux/PTYS, synthetic exact provider records and owned network-none
Docker fixtures exercise discovery, stop/restart reconciliation and native input.
Actual Codex 0.163.0 launch-directory detection and idle in-TUI resume switching
passed: selecting another project and “Use session directory” updates both name
and folder without changing terminal identity or requesting inference during the
switch. Old writers remain open, so separate processes alone are insufficient.
Per-launch lifecycle hooks were investigated with native trust review but defer
until a turn; no hook is shipped. The controlled native title bridge resolves
the tested workflow. Other client versions still need acceptance testing.

Blastoise and Squirtle failed bounded authenticated SSH probes on this date. Their
physical reconnect and robot environment behavior remain unvalidated. No local
installation, fleet helper deployment, tmux reset, provider configuration, SSH
trust, WARP or live networking was changed.

The user narrowed Claude's contrast report to the small descriptions in consecutive
question choices in Fish/foot, and selected an opaque Claude background. An actual
Claude 2.1.303 AskUserQuestion widget, fed exclusively synthetic loopback responses
with private configuration, reproduced low description contrast in foot at 78%
background opacity. Two successive questions restored default backgrounds
correctly; there was no background leak. Before/after foot screenshots of the
same screen confirmed improved description readability with solid default-cell
backgrounds. Text colors, explicit backgrounds and choice highlights remain
unchanged. Light/dark backgrounds follow the host Claude theme; a native attach
fixture verifies both and restoration on shell reattachment. External sessions,
terminal configuration and provider configuration are untouched. Compositor
opacity can still affect the whole window independently of terminal cell colors.
